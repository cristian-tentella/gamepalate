use std::{collections::BTreeMap, path::Path};

use gamepalate_core::{
    game::GameTag,
    ids::{AppId, TagId},
    off_catalog::{OffCatalogGame, OffCatalogGames},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OffCatalogEntry {
    #[allow(dead_code)]
    fetched_at: i64,
    tags: Vec<(u32, f64)>,
    name: Option<String>,
}

pub fn load_off_catalog_games(path: &Path) -> OffCatalogGames {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_off_catalog_games(&text).ok())
        .unwrap_or_default()
}

fn parse_off_catalog_games(text: &str) -> Result<OffCatalogGames, serde_json::Error> {
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
            (AppId(appid), game)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_off_catalog_games_reads_tags_and_name() {
        let games = parse_off_catalog_games(
            r#"{
                "730": { "fetchedAt": 1, "tags": [[1663, 0.75], [1774, 0.25]], "name": "Counter-Strike 2" },
                "440": { "fetchedAt": 1, "tags": [] }
            }"#,
        )
        .unwrap();

        assert_eq!(
            games[&AppId(730)],
            OffCatalogGame {
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
            }
        );
        assert_eq!(
            games[&AppId(440)],
            OffCatalogGame {
                tags: Vec::new(),
                name: None,
            }
        );
    }

    #[test]
    fn load_off_catalog_games_without_a_file_is_empty() {
        let games = load_off_catalog_games(Path::new("does-not-exist/off-catalog-tags.json"));
        assert!(games.is_empty());
    }

    #[test]
    fn parse_off_catalog_games_rejects_invalid_json() {
        assert!(parse_off_catalog_games("{ not json").is_err());
    }
}
