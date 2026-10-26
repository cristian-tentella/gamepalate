use std::{collections::BTreeMap, io::ErrorKind, path::Path};

use gamepalate_core::{
    corrections::{Ban, Corrections},
    ids::{AppId, TagId},
};
use serde::Deserialize;

use crate::keys::parse_id_key;

#[derive(Debug, thiserror::Error)]
pub enum CorrectionsError {
    #[error("could not read the corrections")]
    Io(#[from] std::io::Error),

    #[error("could not save the corrections")]
    Write(#[source] std::io::Error),

    #[error("the corrections file is not valid")]
    Toml(#[from] toml::de::Error),

    #[error("the corrections file is not valid")]
    Edit(#[from] toml_edit::TomlError),

    #[error("in [{section}], {key:?} is not an id")]
    InvalidKey { section: &'static str, key: String },

    #[error("in [{section}], the value of {key} must be {expected}")]
    InvalidValue {
        section: &'static str,
        key: u32,
        expected: &'static str,
    },

    #[error(
        "game multipliers above 1 for {}: to make a game count, put it in a family",
        join(.0)
    )]
    GameMultipliersAboveOne(Vec<AppId>),
}

fn join(ids: &[AppId]) -> String {
    ids.iter()
        .map(|id| id.0.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct CorrectionsFile {
    banned_tags: BTreeMap<String, BanEntry>,
    tag_multipliers: BTreeMap<String, f64>,
    hidden_games: BTreeMap<String, bool>,
    game_multipliers: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BanEntry {
    Switch(bool),
    TopTags(i64),
}

pub fn load_corrections(path: &Path) -> Result<Corrections, CorrectionsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_corrections(&text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Corrections::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn parse_corrections(text: &str) -> Result<Corrections, CorrectionsError> {
    let file: CorrectionsFile = toml::from_str(text)?;
    let mut corrections = Corrections::default();

    for (key, entry) in file.banned_tags {
        let id = id_key("banned_tags", key)?;
        let ban = match entry {
            BanEntry::Switch(false) => continue,
            BanEntry::Switch(true) => Ban::Everywhere,
            BanEntry::TopTags(n) => match u32::try_from(n) {
                Ok(n) if n > 0 => Ban::TopTags(n),
                _ => {
                    return Err(CorrectionsError::InvalidValue {
                        section: "banned_tags",
                        key: id,
                        expected: "true, false or an integer > 0",
                    });
                }
            },
        };
        corrections.banned_tags.insert(TagId(id), ban);
    }

    for (key, multiplier) in file.tag_multipliers {
        let id = id_key("tag_multipliers", key)?;
        if !(multiplier.is_finite() && multiplier > 0.0) {
            return Err(CorrectionsError::InvalidValue {
                section: "tag_multipliers",
                key: id,
                expected: "a number > 0",
            });
        }
        corrections.tag_multipliers.insert(TagId(id), multiplier);
    }

    for (key, hidden) in file.hidden_games {
        let id = id_key("hidden_games", key)?;
        if hidden {
            corrections.hidden_games.insert(AppId(id));
        }
    }

    let mut above_one = Vec::new();
    for (key, multiplier) in file.game_multipliers {
        let id = id_key("game_multipliers", key)?;
        if !(multiplier.is_finite() && multiplier >= 0.0) {
            return Err(CorrectionsError::InvalidValue {
                section: "game_multipliers",
                key: id,
                expected: "a number >= 0",
            });
        }
        if multiplier > 1.0 {
            above_one.push(AppId(id));
        } else {
            corrections.game_multipliers.insert(AppId(id), multiplier);
        }
    }

    if !above_one.is_empty() {
        above_one.sort();
        return Err(CorrectionsError::GameMultipliersAboveOne(above_one));
    }

    Ok(corrections)
}

fn id_key(section: &'static str, key: String) -> Result<u32, CorrectionsError> {
    parse_id_key(&key).ok_or(CorrectionsError::InvalidKey { section, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_corrections_reads_every_section() {
        let corrections = parse_corrections(
            "[banned_tags]
            10 = true
            20 = 8
            30 = false
            [tag_multipliers]
            40 = 0.5
            50 = 2
            [hidden_games]
            100 = true
            200 = false
            [game_multipliers]
            300 = 0
            400 = 0.25",
        )
        .unwrap();

        assert_eq!(
            corrections.banned_tags,
            BTreeMap::from([(TagId(10), Ban::Everywhere), (TagId(20), Ban::TopTags(8))])
        );
        assert_eq!(
            corrections.tag_multipliers,
            BTreeMap::from([(TagId(40), 0.5), (TagId(50), 2.0)])
        );
        assert_eq!(
            corrections.hidden_games.iter().collect::<Vec<_>>(),
            [&AppId(100)]
        );
        assert_eq!(
            corrections.game_multipliers,
            BTreeMap::from([(AppId(300), 0.0), (AppId(400), 0.25)])
        );
    }

    #[test]
    fn parse_corrections_of_an_empty_file_is_empty() {
        assert_eq!(parse_corrections("").unwrap(), Corrections::default());
    }

    #[test]
    fn parse_corrections_lists_every_game_multiplier_above_one() {
        let result = parse_corrections("[game_multipliers]\n100 = 2\n20 = 1.5\n30 = 1");
        assert!(matches!(
            result,
            Err(CorrectionsError::GameMultipliersAboveOne(ids)) if ids == [AppId(20), AppId(100)]
        ));
    }

    #[test]
    fn parse_corrections_rejects_a_ban_on_zero_tags() {
        let result = parse_corrections("[banned_tags]\n10 = 0");
        assert!(matches!(
            result,
            Err(CorrectionsError::InvalidValue {
                section: "banned_tags",
                key: 10,
                ..
            })
        ));
    }

    #[test]
    fn parse_corrections_rejects_a_tag_multiplier_of_zero() {
        let result = parse_corrections("[tag_multipliers]\n10 = 0");
        assert!(matches!(
            result,
            Err(CorrectionsError::InvalidValue {
                section: "tag_multipliers",
                ..
            })
        ));
    }

    #[test]
    fn parse_corrections_rejects_a_key_that_is_not_an_id() {
        let result = parse_corrections("[hidden_games]\nportal = true");
        assert!(matches!(
            result,
            Err(CorrectionsError::InvalidKey { section: "hidden_games", key }) if key == "portal"
        ));
    }

    #[test]
    fn parse_corrections_rejects_an_unknown_section() {
        let result = parse_corrections("[boosted_tags]\n10 = 2");
        assert!(matches!(result, Err(CorrectionsError::Toml(_))));
    }

    #[test]
    fn load_corrections_without_a_file_is_empty() {
        let corrections = load_corrections(Path::new("does-not-exist/corrections.toml")).unwrap();
        assert_eq!(corrections, Corrections::default());
    }
}
