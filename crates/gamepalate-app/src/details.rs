use std::{
    collections::HashMap,
    fmt,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard},
};

use futures::{FutureExt, future::join};
use gamepalate_core::{ids::AppId, region::Region};
use gamepalate_steam::{
    description::{Block, get_description},
    details::{ItemDetails, get_item_details, get_store_categories},
    reviews::{ReviewPage, ReviewQuery, get_reviews},
    transport::{Get, SteamError, Transport},
};

const LANGUAGE: &str = "english";
const DESCRIPTION: &str = "the store description";
const REVIEWS: &str = "the reviews";

#[derive(Debug, Clone, PartialEq)]
pub struct GameDetails {
    pub store: ItemDetails,
    pub modes: Vec<String>,
}

pub struct StoreDetails<S = Transport> {
    transport: S,
    region: Region,
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
    pub fn new(transport: S, region: Region) -> StoreDetails<S> {
        StoreDetails {
            transport,
            region,
            categories: Mutex::new(None),
        }
    }

    pub async fn details(&self, id: AppId) -> Result<Option<GameDetails>, SteamError> {
        let (names, store) = join(
            self.categories(),
            get_item_details(&self.transport, id, LANGUAGE, self.region.as_str()),
        )
        .await;
        let store = store?;

        Ok(store.map(|store| game_details(store, &names)))
    }

    pub async fn description(&self, id: AppId) -> Result<Option<Vec<Block>>, SteamError> {
        AssertUnwindSafe(get_description(&self.transport, id, LANGUAGE))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| {
                Err(SteamError::Shape {
                    endpoint: DESCRIPTION.to_string(),
                    reason: "the reader stopped".to_string(),
                })
            })
    }

    pub async fn reviews(
        &self,
        id: AppId,
        query: &ReviewQuery,
        cursor: Option<&str>,
    ) -> Result<ReviewPage, SteamError> {
        AssertUnwindSafe(get_reviews(&self.transport, id, query, cursor))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| {
                Err(SteamError::Shape {
                    endpoint: REVIEWS.to_string(),
                    reason: "the reader stopped".to_string(),
                })
            })
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
        params: Mutex<Vec<String>>,
    }

    impl Get for FakeSteam {
        fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> impl Future<Output = Result<Value, SteamError>> + Send {
            self.asked.lock().unwrap().push(endpoint.to_string());
            self.params
                .lock()
                .unwrap()
                .extend(params.iter().map(|(_, value)| value.to_string()));
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("one reply for every request");

            std::future::ready(reply)
        }
    }

    #[derive(Default)]
    struct SlowSteam {
        asked: Mutex<Vec<String>>,
    }

    impl Get for SlowSteam {
        fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> impl Future<Output = Result<Value, SteamError>> + Send {
            self.asked.lock().unwrap().push(endpoint.to_string());
            std::future::pending()
        }
    }

    fn store(replies: Vec<Result<Value, SteamError>>) -> StoreDetails<FakeSteam> {
        store_in(replies, Region::ITALY)
    }

    fn store_in(
        replies: Vec<Result<Value, SteamError>>,
        region: Region,
    ) -> StoreDetails<FakeSteam> {
        StoreDetails::new(
            FakeSteam {
                replies: Mutex::new(replies.into()),
                asked: Mutex::new(Vec::new()),
                params: Mutex::new(Vec::new()),
            },
            region,
        )
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
    async fn a_description_that_breaks_the_reader_is_an_error() {
        let store = store(Vec::new());

        assert!(matches!(
            store.description(AppId(620)).await,
            Err(SteamError::Shape { .. })
        ));
    }

    #[tokio::test]
    async fn reads_the_reviews_with_the_same_transport() {
        let store = store(vec![Ok(json!({ "response": {
            "reviews": [{
                "recommendationid": "1",
                "author": { "playtime_forever": 120, "playtime_at_review": 60 },
                "language": "italian",
                "review": "Bello",
                "timestamp_created": 1_700_000_000,
                "voted_up": true,
                "votes_up": 3,
            }],
            "cursor": "next",
            "total_matching": 1,
        } }))]);

        let page = store
            .reviews(AppId(620), &ReviewQuery::default(), None)
            .await
            .unwrap();

        assert_eq!(page.reviews.len(), 1);
        assert_eq!(page.next.as_deref(), Some("next"));
        assert_eq!(
            *store.transport.asked.lock().unwrap(),
            ["IUserReviewsService/GetAppReviews/v1"]
        );
    }

    #[tokio::test]
    async fn reviews_that_break_the_reader_are_an_error() {
        let store = store(Vec::new());

        assert!(matches!(
            store
                .reviews(AppId(620), &ReviewQuery::default(), None)
                .await,
            Err(SteamError::Shape { .. })
        ));
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

    #[test]
    fn asks_the_categories_and_the_details_together() {
        let store = StoreDetails::new(SlowSteam::default(), Region::ITALY);
        let mut details = std::pin::pin!(store.details(AppId(620)));

        let poll = details
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()));

        assert!(poll.is_pending());
        assert_eq!(*store.transport.asked.lock().unwrap(), [CATEGORIES, ITEMS]);
    }

    #[tokio::test]
    async fn asks_the_details_in_the_region_it_was_given() {
        let store = store_in(vec![categories(), item()], Region::new("US").unwrap());

        store.details(AppId(620)).await.unwrap();

        let params = store.transport.params.lock().unwrap();
        let input: Value = serde_json::from_str(params.last().unwrap()).unwrap();
        assert_eq!(input["context"]["country_code"], "US");
    }

    #[tokio::test]
    async fn a_failed_request_is_an_error() {
        let store = store(vec![categories(), failure()]);

        assert!(store.details(AppId(620)).await.is_err());
    }
}
