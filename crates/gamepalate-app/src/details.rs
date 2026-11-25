use std::{
    collections::HashMap,
    fmt,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use futures::{FutureExt, future::join3};
use gamepalate_core::{ids::AppId, region::Region};
use gamepalate_steam::{
    deck::{DeckCheck, ProtonDb, get_deck_report, get_protondb},
    description::{Block, get_description},
    details::{ItemDetails, get_item_details, get_store_categories},
    reviews::{ReviewPage, ReviewQuery, get_reviews},
    transport::{Get, SteamError, Transport},
};

use crate::log::failed;

const LANGUAGE: &str = "english";
const DESCRIPTION: &str = "the store description";
const REVIEWS: &str = "the reviews";
const EXTRAS_DEADLINE: Duration = Duration::from_secs(3);
const DETAILS: &str = "the store details";
/// The page waits this long, not the minutes the transport would retry while Steam is down.
const DETAILS_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq)]
pub struct GameDetails {
    pub store: ItemDetails,
    pub modes: Vec<String>,
    pub deck: Vec<DeckCheck>,
    pub protondb: Option<ProtonDb>,
}

pub struct StoreDetails<S = Transport, P = S> {
    transport: S,
    protondb: P,
    region: Region,
    categories: Mutex<Option<Arc<HashMap<u32, String>>>>,
}

impl<S, P> fmt::Debug for StoreDetails<S, P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreDetails")
            .finish_non_exhaustive()
    }
}

impl<S: Get + Sync, P: Get + Sync> StoreDetails<S, P> {
    pub fn new(transport: S, protondb: P, region: Region) -> StoreDetails<S, P> {
        StoreDetails {
            transport,
            protondb,
            region,
            categories: Mutex::new(None),
        }
    }

    pub async fn details(&self, id: AppId) -> Result<Option<GameDetails>, SteamError> {
        let read = join3(
            self.categories(),
            get_item_details(&self.transport, id, LANGUAGE, self.region.as_str()),
            extra(get_protondb(&self.protondb, id), "could not read ProtonDB"),
        );
        let Ok((names, store, protondb)) = tokio::time::timeout(DETAILS_DEADLINE, read).await
        else {
            return Err(SteamError::SteamDown {
                endpoint: DETAILS.to_string(),
                reason: format!("no answer in {}s", DETAILS_DEADLINE.as_secs()),
                source: None,
            });
        };
        let Some(store) = store? else {
            return Ok(None);
        };

        let deck = if store.steam_deck.is_some() {
            extra(
                get_deck_report(&self.transport, id),
                "could not read the Steam Deck report",
            )
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(Some(game_details(store, &names, deck, protondb.flatten())))
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

async fn extra<T>(
    read: impl Future<Output = Result<T, SteamError>>,
    what: &'static str,
) -> Option<T> {
    match tokio::time::timeout(EXTRAS_DEADLINE, read).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            failed(what)(&error);
            None
        }
        Err(_) => {
            tracing::warn!("{what}: no answer in {EXTRAS_DEADLINE:?}");
            None
        }
    }
}

fn game_details(
    store: ItemDetails,
    names: &HashMap<u32, String>,
    deck: Vec<DeckCheck>,
    protondb: Option<ProtonDb>,
) -> GameDetails {
    let modes = store
        .player_categories
        .iter()
        .filter_map(|id| names.get(id).cloned())
        .collect();

    GameDetails {
        store,
        modes,
        deck,
        protondb,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use gamepalate_steam::{
        deck::{CheckResult, ProtonTier},
        details::SteamDeck,
    };
    use reqwest::StatusCode;
    use serde_json::{Value, json};

    use super::*;

    const CATEGORIES: &str = "IStoreBrowseService/GetStoreCategories/v1";
    const ITEMS: &str = "IStoreBrowseService/GetItems/v1";
    const REVIEWS: &str = "IUserReviewsService/GetAppReviews/v1";
    const REPORT: &str =
        "https://store.steampowered.com/saleaction/ajaxgetdeckappcompatibilityreport";
    const PROTONDB: &str = "https://www.protondb.com/api/v1/reports/summaries/620.json";

    type Reply = Result<Value, SteamError>;

    #[derive(Default)]
    struct FakeSteam {
        replies: Mutex<HashMap<String, VecDeque<Reply>>>,
        asked: Mutex<Vec<String>>,
        params: Mutex<Vec<String>>,
    }

    impl FakeSteam {
        fn new(replies: Vec<(&str, Reply)>) -> FakeSteam {
            let fake = FakeSteam::default();
            for (endpoint, reply) in replies {
                fake.replies
                    .lock()
                    .unwrap()
                    .entry(endpoint.to_string())
                    .or_default()
                    .push_back(reply);
            }
            fake
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
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
                .get_mut(endpoint)
                .and_then(VecDeque::pop_front)
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

    fn store(steam: Vec<(&str, Reply)>, protondb: Vec<(&str, Reply)>) -> StoreDetails<FakeSteam> {
        store_in(steam, protondb, Region::ITALY)
    }

    fn store_in(
        steam: Vec<(&str, Reply)>,
        protondb: Vec<(&str, Reply)>,
        region: Region,
    ) -> StoreDetails<FakeSteam> {
        StoreDetails::new(FakeSteam::new(steam), FakeSteam::new(protondb), region)
    }

    fn categories() -> Reply {
        Ok(json!({ "response": { "categories": [
            { "categoryid": 2, "display_name": "Single-player" },
            { "categoryid": 9, "display_name": "Co-op" },
        ] } }))
    }

    fn item() -> Reply {
        Ok(json!({ "response": { "store_items": [{
            "success": 1,
            "categories": { "supported_player_categoryids": [2, 9, 99] },
        }] } }))
    }

    fn verified_item() -> Reply {
        Ok(json!({ "response": { "store_items": [{
            "success": 1,
            "platforms": { "steam_deck_compat_category": 3 },
        }] } }))
    }

    fn report() -> Reply {
        Ok(json!({ "success": 1, "results": { "resolved_items": [{
            "display_type": 4,
            "loc_token": "#SteamDeckVerified_TestResult_DefaultConfigurationIsPerformant",
        }] } }))
    }

    fn gold() -> Reply {
        Ok(json!({ "tier": "gold", "total": 2797 }))
    }

    fn not_found() -> Reply {
        Err(SteamError::Status {
            endpoint: PROTONDB.to_string(),
            status: StatusCode::NOT_FOUND,
        })
    }

    fn failure() -> Reply {
        Err(SteamError::Shape {
            endpoint: CATEGORIES.to_string(),
            reason: "down".to_string(),
        })
    }

    #[tokio::test]
    async fn a_description_that_breaks_the_reader_is_an_error() {
        let store = store(Vec::new(), Vec::new());

        assert!(matches!(
            store.description(AppId(620)).await,
            Err(SteamError::Shape { .. })
        ));
    }

    #[tokio::test]
    async fn reads_the_reviews_with_the_same_transport() {
        let store = store(
            vec![(
                REVIEWS,
                Ok(json!({ "response": {
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
                } })),
            )],
            Vec::new(),
        );

        let page = store
            .reviews(AppId(620), &ReviewQuery::default(), None)
            .await
            .unwrap();

        assert_eq!(page.reviews.len(), 1);
        assert_eq!(page.next.as_deref(), Some("next"));
        assert_eq!(store.transport.asked(), [REVIEWS]);
    }

    #[tokio::test]
    async fn reviews_that_break_the_reader_are_an_error() {
        let store = store(Vec::new(), Vec::new());

        assert!(matches!(
            store
                .reviews(AppId(620), &ReviewQuery::default(), None)
                .await,
            Err(SteamError::Shape { .. })
        ));
    }

    #[tokio::test]
    async fn asks_for_the_categories_once() {
        let store = store(
            vec![(CATEGORIES, categories()), (ITEMS, item()), (ITEMS, item())],
            vec![(PROTONDB, gold()), (PROTONDB, gold())],
        );

        let first = store.details(AppId(620)).await.unwrap().unwrap();
        store.details(AppId(620)).await.unwrap();

        assert_eq!(first.modes, ["Single-player", "Co-op"]);
        assert_eq!(store.transport.asked(), [CATEGORIES, ITEMS, ITEMS]);
    }

    #[tokio::test]
    async fn without_categories_the_modes_are_empty_and_asked_again() {
        let store = store(
            vec![
                (CATEGORIES, failure()),
                (CATEGORIES, categories()),
                (ITEMS, item()),
                (ITEMS, item()),
            ],
            vec![(PROTONDB, gold()), (PROTONDB, gold())],
        );

        let first = store.details(AppId(620)).await.unwrap().unwrap();
        let second = store.details(AppId(620)).await.unwrap().unwrap();

        assert!(first.modes.is_empty());
        assert_eq!(second.modes, ["Single-player", "Co-op"]);
    }

    #[tokio::test]
    async fn a_game_without_a_store_page_has_no_details() {
        let store = store(
            vec![
                (CATEGORIES, categories()),
                (
                    ITEMS,
                    Ok(json!({ "response": { "store_items": [{ "success": 15 }] } })),
                ),
            ],
            vec![(PROTONDB, gold())],
        );

        assert_eq!(store.details(AppId(620)).await.unwrap(), None);
        assert_eq!(store.transport.asked(), [CATEGORIES, ITEMS]);
    }

    #[tokio::test]
    async fn a_verified_game_has_the_report_and_protondb() {
        let store = store(
            vec![
                (CATEGORIES, categories()),
                (ITEMS, verified_item()),
                (REPORT, report()),
            ],
            vec![(PROTONDB, gold())],
        );

        let details = store.details(AppId(620)).await.unwrap().unwrap();

        assert_eq!(
            details.deck,
            [DeckCheck {
                result: CheckResult::Passed,
                text: "Runs well with default settings",
            }]
        );
        assert_eq!(
            details.protondb,
            Some(ProtonDb {
                tier: ProtonTier::Gold,
                reports: 2797,
            })
        );
        assert_eq!(store.transport.asked(), [CATEGORIES, ITEMS, REPORT]);
    }

    #[tokio::test]
    async fn without_a_category_the_report_is_not_asked() {
        let store = store(
            vec![(CATEGORIES, categories()), (ITEMS, item())],
            vec![(PROTONDB, gold())],
        );

        let details = store.details(AppId(620)).await.unwrap().unwrap();

        assert!(details.deck.is_empty());
        assert!(details.protondb.is_some());
        assert_eq!(store.transport.asked(), [CATEGORIES, ITEMS]);
        assert_eq!(store.protondb.asked(), [PROTONDB]);
    }

    #[tokio::test]
    async fn a_failed_report_keeps_the_details() {
        let store = store(
            vec![
                (CATEGORIES, categories()),
                (ITEMS, verified_item()),
                (REPORT, failure()),
            ],
            vec![(PROTONDB, gold())],
        );

        let details = store.details(AppId(620)).await.unwrap().unwrap();

        assert!(details.deck.is_empty());
        assert!(details.protondb.is_some());
        assert_eq!(details.store.steam_deck, Some(SteamDeck::Verified));
    }

    #[tokio::test]
    async fn a_game_without_protondb_reports_has_none() {
        let store = store(
            vec![
                (CATEGORIES, categories()),
                (ITEMS, verified_item()),
                (REPORT, report()),
            ],
            vec![(PROTONDB, not_found())],
        );

        let details = store.details(AppId(620)).await.unwrap().unwrap();

        assert_eq!(details.protondb, None);
        assert_eq!(details.deck.len(), 1);
    }

    #[tokio::test]
    async fn a_failed_protondb_keeps_the_details() {
        let store = store(
            vec![(CATEGORIES, categories()), (ITEMS, item())],
            vec![(PROTONDB, failure())],
        );

        let details = store.details(AppId(620)).await.unwrap().unwrap();

        assert_eq!(details.protondb, None);
        assert_eq!(details.modes, ["Single-player", "Co-op"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_protondb_does_not_hold_the_details() {
        let store = StoreDetails::new(
            FakeSteam::new(vec![(CATEGORIES, categories()), (ITEMS, item())]),
            SlowSteam::default(),
            Region::ITALY,
        );

        let details = store.details(AppId(620)).await.unwrap().unwrap();

        assert_eq!(details.protondb, None);
        assert_eq!(details.modes, ["Single-player", "Co-op"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_store_gives_up_after_the_deadline() {
        let store = StoreDetails::new(SlowSteam::default(), SlowSteam::default(), Region::ITALY);
        let start = tokio::time::Instant::now();

        let error = tokio::time::timeout(DETAILS_DEADLINE * 2, store.details(AppId(620)))
            .await
            .expect("the details give up by themselves")
            .unwrap_err();

        assert_eq!(start.elapsed(), DETAILS_DEADLINE);
        assert!(matches!(error, SteamError::SteamDown { .. }));
    }

    #[tokio::test]
    async fn asks_the_categories_the_details_and_protondb_together() {
        let store = StoreDetails::new(SlowSteam::default(), SlowSteam::default(), Region::ITALY);
        let mut details = std::pin::pin!(store.details(AppId(620)));

        assert!(futures::poll!(details.as_mut()).is_pending());
        assert_eq!(*store.transport.asked.lock().unwrap(), [CATEGORIES, ITEMS]);
        assert_eq!(*store.protondb.asked.lock().unwrap(), [PROTONDB]);
    }

    #[tokio::test]
    async fn asks_the_details_in_the_region_it_was_given() {
        let store = store_in(
            vec![(CATEGORIES, categories()), (ITEMS, item())],
            vec![(PROTONDB, gold())],
            Region::new("US").unwrap(),
        );

        store.details(AppId(620)).await.unwrap();

        let params = store.transport.params.lock().unwrap();
        let input: Value = serde_json::from_str(params.last().unwrap()).unwrap();
        assert_eq!(input["context"]["country_code"], "US");
    }

    #[tokio::test]
    async fn a_failed_request_is_an_error() {
        let store = store(
            vec![(CATEGORIES, categories()), (ITEMS, failure())],
            vec![(PROTONDB, gold())],
        );

        assert!(store.details(AppId(620)).await.is_err());
    }
}
