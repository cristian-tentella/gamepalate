use std::{collections::BTreeMap, path::Path};

use gamepalate_core::{
    game::GameTag,
    ids::{AppId, TagId},
    off_catalog::{
        CachedGame, OffCatalogCache, OffCatalogGame, OffCatalogGames, off_catalog_games,
    },
    timestamp::Timestamp,
};
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomically;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OffCatalogEntry {
    fetched_at: i64,
    tags: Vec<(u32, f64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

pub fn load_off_catalog_games(path: &Path) -> OffCatalogGames {
    off_catalog_games(load_off_catalog_cache(path))
}

pub fn load_off_catalog_cache(path: &Path) -> OffCatalogCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_off_catalog_cache(&text).ok())
        .unwrap_or_default()
}

pub fn save_off_catalog_cache(path: &Path, cache: &OffCatalogCache) -> std::io::Result<()> {
    let file: BTreeMap<u32, OffCatalogEntry> = cache
        .iter()
        .map(|(id, cached)| {
            let entry = OffCatalogEntry {
                fetched_at: cached.fetched_at.0,
                tags: cached
                    .game
                    .tags
                    .iter()
                    .map(|tag| (tag.id.0, tag.weight))
                    .collect(),
                name: cached.game.name.clone(),
            };
            (id.0, entry)
        })
        .collect();
    let text = serde_json::to_string(&file).map_err(std::io::Error::other)?;

    write_atomically(path, &text)
}

fn parse_off_catalog_cache(text: &str) -> Result<OffCatalogCache, serde_json::Error> {
    let file: BTreeMap<u32, OffCatalogEntry> = serde_json::from_str(text)?;

    Ok(file
        .into_iter()
        .map(|(appid, entry)| {
            let game = OffCatalogGame {
                tags: entry
                    .tags
                    .into_iter()
                    .map(|(tagid, weight)| GameTag {
                        id: TagId(tagid),
                        weight,
                    })
                    .collect(),
                name: entry.name,
            };
            let cached = CachedGame {
                fetched_at: Timestamp(entry.fetched_at),
                game,
            };
            (AppId(appid), cached)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_off_catalog_cache_reads_dates_tags_and_name() {
        let cache = parse_off_catalog_cache(
            r#"{
                "730": { "fetchedAt": 1, "tags": [[1663, 0.75], [1774, 0.25]], "name": "Counter-Strike 2" },
                "440": { "fetchedAt": 2, "tags": [] }
            }"#,
        )
        .unwrap();

        assert_eq!(
            cache[&AppId(730)],
            CachedGame {
                fetched_at: Timestamp(1),
                game: OffCatalogGame {
                    tags: vec![
                        GameTag {
                            id: TagId(1663),
                            weight: 0.75,
                        },
                        GameTag {
                            id: TagId(1774),
                            weight: 0.25,
                        },
                    ],
                    name: Some("Counter-Strike 2".to_string()),
                },
            }
        );
        assert_eq!(
            cache[&AppId(440)],
            CachedGame {
                fetched_at: Timestamp(2),
                game: OffCatalogGame {
                    tags: Vec::new(),
                    name: None,
                },
            }
        );
    }

    #[test]
    fn load_off_catalog_games_without_a_file_is_empty() {
        let games = load_off_catalog_games(Path::new("does-not-exist/off-catalog-tags.json"));
        assert!(games.is_empty());
    }

    #[test]
    fn parse_off_catalog_cache_rejects_invalid_json() {
        assert!(parse_off_catalog_cache("{ not json").is_err());
    }

    #[test]
    fn a_saved_cache_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("off-catalog-tags.json");
        let cache = OffCatalogCache::from([
            (
                AppId(730),
                CachedGame {
                    fetched_at: Timestamp(5),
                    game: OffCatalogGame {
                        tags: vec![GameTag {
                            id: TagId(19),
                            weight: 1.0,
                        }],
                        name: Some("Counter-Strike 2".to_string()),
                    },
                },
            ),
            (
                AppId(440),
                CachedGame {
                    fetched_at: Timestamp(6),
                    game: OffCatalogGame {
                        tags: Vec::new(),
                        name: None,
                    },
                },
            ),
        ]);

        save_off_catalog_cache(&path, &cache).unwrap();

        assert_eq!(load_off_catalog_cache(&path), cache);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""440":{"fetchedAt":6,"tags":[]}"#));
    }
}
