use std::collections::HashMap;

use gamepalate_core::{
    ids::AppId,
    off_catalog::{CachedGame, OffCatalogCache, OffCatalogGame},
    timestamp::Timestamp,
};

use crate::{
    items::{BATCH_SIZE, get_items},
    transport::{Get, SteamError},
};

pub async fn renew_off_catalog<S: Get>(
    steam: &S,
    cache: &mut OffCatalogCache,
    stale: &[AppId],
    now: Timestamp,
    language: &str,
    country_code: &str,
) -> Result<bool, SteamError> {
    let mut renewed = false;

    for batch in stale.chunks(BATCH_SIZE) {
        let items = match get_items(steam, batch, language, country_code).await {
            Ok(items) => items,
            Err(error @ SteamError::SteamDown { .. }) => {
                let games = if batch.len() == 1 { "game" } else { "games" };
                log::warn!(
                    "{error}: the tags of {} {games} outside the Catalog stay as they were",
                    batch.len()
                );
                continue;
            }
            Err(error) => return Err(error),
        };

        let mut found: HashMap<AppId, _> = items.into_iter().map(|item| (item.id, item)).collect();
        for id in batch {
            let game = found.remove(id).map_or(
                OffCatalogGame {
                    tags: Vec::new(),
                    name: None,
                },
                |item| item.into_off_catalog_game(),
            );
            cache.insert(
                *id,
                CachedGame {
                    fetched_at: now,
                    game,
                },
            );
        }
        renewed = true;
    }

    Ok(renewed)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use gamepalate_core::{game::GameTag, ids::TagId};
    use serde_json::{Value, json};

    use super::*;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    struct FakeSteam {
        failure: Option<fn(&str) -> SteamError>,
        asked: Mutex<Vec<usize>>,
    }

    impl FakeSteam {
        fn new() -> FakeSteam {
            FakeSteam {
                failure: None,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn failing(failure: fn(&str) -> SteamError) -> FakeSteam {
            FakeSteam {
                failure: Some(failure),
                ..FakeSteam::new()
            }
        }
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            let input: Value = serde_json::from_str(params[0].1).unwrap();
            let ids: Vec<u64> = input["ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id["appid"].as_u64().unwrap())
                .collect();
            self.asked.lock().unwrap().push(ids.len());
            if let Some(failure) = self.failure {
                return Err(failure(endpoint));
            }
            let items: Vec<Value> = ids
                .iter()
                .filter(|&&id| id != 440)
                .map(|&id| {
                    json!({
                        "id": id,
                        "success": 1,
                        "type": 0,
                        "name": format!("Game {id}"),
                        "tags": [{ "tagid": 19, "weight": 300 }, { "tagid": 492, "weight": 100 }],
                    })
                })
                .collect();
            Ok(json!({ "response": { "store_items": items } }))
        }
    }

    fn down(endpoint: &str) -> SteamError {
        SteamError::SteamDown {
            endpoint: endpoint.to_string(),
            reason: "timed out".to_string(),
            source: None,
        }
    }

    fn shape(endpoint: &str) -> SteamError {
        SteamError::Shape {
            endpoint: endpoint.to_string(),
            reason: "changed".to_string(),
        }
    }

    fn old_entry() -> CachedGame {
        CachedGame {
            fetched_at: Timestamp(1),
            game: OffCatalogGame {
                tags: Vec::new(),
                name: Some("Old".to_string()),
            },
        }
    }

    #[tokio::test]
    async fn every_asked_game_is_written_with_now() {
        let mut cache = OffCatalogCache::from([(AppId(730), old_entry())]);

        let renewed = renew_off_catalog(
            &FakeSteam::new(),
            &mut cache,
            &[AppId(730), AppId(440)],
            NOW,
            "english",
            "IT",
        )
        .await
        .unwrap();

        assert!(renewed);
        assert_eq!(
            cache[&AppId(730)],
            CachedGame {
                fetched_at: NOW,
                game: OffCatalogGame {
                    tags: vec![
                        GameTag {
                            id: TagId(19),
                            weight: 0.75,
                        },
                        GameTag {
                            id: TagId(492),
                            weight: 0.25,
                        },
                    ],
                    name: Some("Game 730".to_string()),
                },
            }
        );
        assert_eq!(
            cache[&AppId(440)],
            CachedGame {
                fetched_at: NOW,
                game: OffCatalogGame {
                    tags: Vec::new(),
                    name: None,
                },
            }
        );
    }

    #[tokio::test]
    async fn games_are_asked_in_batches() {
        let steam = FakeSteam::new();
        let stale: Vec<AppId> = (1..=BATCH_SIZE as u32 + 1).map(AppId).collect();
        let mut cache = OffCatalogCache::new();

        renew_off_catalog(&steam, &mut cache, &stale, NOW, "english", "IT")
            .await
            .unwrap();

        assert_eq!(*steam.asked.lock().unwrap(), [BATCH_SIZE, 1]);
        assert_eq!(cache.len(), BATCH_SIZE + 1);
    }

    #[tokio::test]
    async fn nothing_to_ask_renews_nothing() {
        let mut cache = OffCatalogCache::new();

        let renewed = renew_off_catalog(&FakeSteam::new(), &mut cache, &[], NOW, "english", "IT")
            .await
            .unwrap();

        assert!(!renewed);
    }

    #[tokio::test]
    async fn steam_down_skips_the_batch_and_keeps_the_old_entries() {
        let mut cache = OffCatalogCache::from([(AppId(730), old_entry())]);

        let renewed = renew_off_catalog(
            &FakeSteam::failing(down),
            &mut cache,
            &[AppId(730), AppId(570)],
            NOW,
            "english",
            "IT",
        )
        .await
        .unwrap();

        assert!(!renewed);
        assert_eq!(cache, OffCatalogCache::from([(AppId(730), old_entry())]));
    }

    #[tokio::test]
    async fn other_errors_stop_everything() {
        let mut cache = OffCatalogCache::new();

        let result = renew_off_catalog(
            &FakeSteam::failing(shape),
            &mut cache,
            &[AppId(730)],
            NOW,
            "english",
            "IT",
        )
        .await;

        assert!(matches!(result, Err(SteamError::Shape { .. })));
    }
}
