use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use gamepalate_core::ids::AppId;
use gamepalate_steam::{
    details::{ItemDetails, get_item_details, get_store_categories},
    transport::{Get, SteamError, Transport},
};

const LANGUAGE: &str = "english";
const COUNTRY_CODE: &str = "IT";

#[derive(Debug, Clone, PartialEq)]
pub struct GameDetails {
    pub store: ItemDetails,
    pub modes: Vec<String>,
}

pub struct StoreDetails<S = Transport> {
    transport: S,
    categories: Mutex<Option<Arc<HashMap<u32, String>>>>,
}

impl<S> fmt::Debug for StoreDetails<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreDetails")
            .finish_non_exhaustive()
    }
}

impl<S: Get + Sync> StoreDetails<S> {
    pub fn new(transport: S) -> StoreDetails<S> {
        StoreDetails {
            transport,
            categories: Mutex::new(None),
        }
    }

    pub async fn details(&self, id: AppId) -> Result<Option<GameDetails>, SteamError> {
        let names = self.categories().await;
        let store = get_item_details(&self.transport, id, LANGUAGE, COUNTRY_CODE).await?;

        Ok(store.map(|store| game_details(store, &names)))
    }

    async fn categories(&self) -> Arc<HashMap<u32, String>> {
        let known = self.lock().clone();
        if let Some(names) = known {
            return names;
        }

        match get_store_categories(&self.transport, LANGUAGE).await {
            Ok(names) => {
                let names = Arc::new(names);
                *self.lock() = Some(Arc::clone(&names));
                names
            }
            Err(_) => Arc::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Arc<HashMap<u32, String>>>> {
        self.categories
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn game_details(store: ItemDetails, names: &HashMap<u32, String>) -> GameDetails {
    let modes = store
        .player_categories
        .iter()
        .filter_map(|id| names.get(id).cloned())
        .collect();

    GameDetails { store, modes }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use serde_json::{Value, json};

    use super::*;

    const CATEGORIES: &str = "IStoreBrowseService/GetStoreCategories/v1";
    const ITEMS: &str = "IStoreBrowseService/GetItems/v1";

    struct FakeSteam {
        replies: Mutex<VecDeque<Result<Value, SteamError>>>,
        asked: Mutex<Vec<String>>,
    }

    impl Get for FakeSteam {
        fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> impl Future<Output = Result<Value, SteamError>> + Send {
            self.asked.lock().unwrap().push(endpoint.to_string());
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("one reply for every request");

            std::future::ready(reply)
        }
    }

    fn store(replies: Vec<Result<Value, SteamError>>) -> StoreDetails<FakeSteam> {
        StoreDetails::new(FakeSteam {
            replies: Mutex::new(replies.into()),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn categories() -> Result<Value, SteamError> {
        Ok(json!({ "response": { "categories": [
            { "categoryid": 2, "display_name": "Single-player" },
            { "categoryid": 9, "display_name": "Co-op" },
        ] } }))
    }

    fn item() -> Result<Value, SteamError> {
        Ok(json!({ "response": { "store_items": [{
            "success": 1,
            "categories": { "supported_player_categoryids": [2, 9, 99] },
        }] } }))
    }

    fn failure() -> Result<Value, SteamError> {
        Err(SteamError::Shape {
            endpoint: CATEGORIES.to_string(),
            reason: "down".to_string(),
        })
    }

    #[tokio::test]
    async fn asks_for_the_categories_once() {
        let store = store(vec![categories(), item(), item()]);

        let first = store.details(AppId(620)).await.unwrap().unwrap();
        store.details(AppId(620)).await.unwrap();

        assert_eq!(first.modes, ["Single-player", "Co-op"]);
        assert_eq!(
            *store.transport.asked.lock().unwrap(),
            [CATEGORIES, ITEMS, ITEMS]
        );
    }

    #[tokio::test]
    async fn without_categories_the_modes_are_empty_and_asked_again() {
        let store = store(vec![failure(), item(), categories(), item()]);

        let first = store.details(AppId(620)).await.unwrap().unwrap();
        let second = store.details(AppId(620)).await.unwrap().unwrap();

        assert!(first.modes.is_empty());
        assert_eq!(second.modes, ["Single-player", "Co-op"]);
    }

    #[tokio::test]
    async fn a_game_without_a_store_page_has_no_details() {
        let store = store(vec![
            categories(),
            Ok(json!({ "response": { "store_items": [{ "success": 15 }] } })),
        ]);

        assert_eq!(store.details(AppId(3)).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_failed_request_is_an_error() {
        let store = store(vec![categories(), failure()]);

        assert!(store.details(AppId(620)).await.is_err());
    }
}
