use std::{io::ErrorKind, path::Path};

use gamepalate_core::{
    families::{Families, Family},
    ids::AppId,
};
use serde::{Deserialize, Deserializer, de::Error};

#[derive(Debug, thiserror::Error)]
pub enum FamiliesError {
    #[error("could not read the families")]
    Io(#[from] std::io::Error),

    #[error("the families file is not valid")]
    Toml(#[from] toml::de::Error),

    #[error("the families file is not valid")]
    Edit(#[from] toml_edit::TomlError),

    #[error("could not save the families")]
    Write(#[source] std::io::Error),

    #[error("family {name:?} is not valid")]
    Family {
        name: String,
        #[source]
        source: toml::de::Error,
    },
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct FamiliesFile {
    // toml::Table keeps the file order (preserve_order feature).
    families: toml::Table,
    held_out: GamesSection,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct GamesSection {
    games: Vec<u32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct FamilySection {
    games: Vec<u32>,
    #[serde(deserialize_with = "date")]
    automatic: Option<String>,
}

fn date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let text = String::deserialize(deserializer)?;
    let valid = text.parse::<toml::value::Datetime>().is_ok_and(|datetime| {
        datetime.date.is_some() && datetime.time.is_none() && datetime.offset.is_none()
    });
    if !valid {
        return Err(D::Error::custom(format!(
            "automatic must be a date such as \"2026-11-10\", not {text:?}"
        )));
    }
    Ok(Some(text))
}

pub fn load_families(path: &Path) -> Result<Families, FamiliesError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_families(&text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Families::default()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn parse_families(text: &str) -> Result<Families, FamiliesError> {
    let file: FamiliesFile = toml::from_str(text)?;

    let families = file
        .families
        .into_iter()
        .map(|(name, value)| match value.try_into::<FamilySection>() {
            Ok(section) => Ok(Family {
                name,
                games: app_ids(section.games),
                automatic: section.automatic,
            }),
            Err(source) => Err(FamiliesError::Family { name, source }),
        })
        .collect::<Result<_, _>>()?;

    Ok(Families {
        families,
        held_out: app_ids(file.held_out.games),
    })
}

fn app_ids(ids: Vec<u32>) -> Vec<AppId> {
    ids.into_iter().map(AppId).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_families_keeps_the_order_of_the_file() {
        let families = parse_families(
            "[families.zeta]
            games = [30, 10]
            [families.alpha]
            games = [20]
            [held_out]
            games = [40]",
        )
        .unwrap();

        assert_eq!(
            families,
            Families {
                families: vec![
                    Family {
                        name: "zeta".to_string(),
                        games: vec![AppId(30), AppId(10)],
                        automatic: None,
                    },
                    Family {
                        name: "alpha".to_string(),
                        games: vec![AppId(20)],
                        automatic: None,
                    },
                ],
                held_out: vec![AppId(40)],
            }
        );
    }

    #[test]
    fn parse_families_without_games_gives_empty_lists() {
        let families = parse_families("[families.empty]\n[held_out]").unwrap();
        assert!(families.families[0].games.is_empty());
        assert!(families.held_out.is_empty());
    }

    #[test]
    fn parse_families_rejects_an_unknown_section() {
        let result = parse_families("[kept_aside]\ngames = [1]");
        assert!(matches!(result, Err(FamiliesError::Toml(_))));
    }

    #[test]
    fn parse_families_names_the_family_with_an_unknown_key() {
        let result = parse_families("[families.mysteries]\ngame = [1]");
        assert!(matches!(result, Err(FamiliesError::Family { name, .. }) if name == "mysteries"));
    }

    #[test]
    fn parse_families_rejects_a_game_that_is_not_an_appid() {
        let result = parse_families("[families.mysteries]\ngames = [-1]");
        assert!(matches!(result, Err(FamiliesError::Family { .. })));
    }

    #[test]
    fn parse_families_reads_the_day_an_automatic_family_was_made() {
        let families = parse_families(
            "[families.mysteries]
            games = [1]
            automatic = \"2026-11-10\"
            [families.cozy]
            games = [2]",
        )
        .unwrap();

        assert_eq!(
            families.families[0].automatic.as_deref(),
            Some("2026-11-10")
        );
        assert_eq!(families.families[1].automatic, None);
    }

    #[test]
    fn parse_families_rejects_an_automatic_key_that_is_not_a_date() {
        for value in [
            "\"10 Nov\"",
            "\"2026-13-01\"",
            "\"2026-11-10T10:00:00\"",
            "true",
        ] {
            let result = parse_families(&format!(
                "[families.mysteries]\ngames = [1]\nautomatic = {value}"
            ));
            assert!(
                matches!(result, Err(FamiliesError::Family { ref name, .. }) if name == "mysteries"),
                "{value} was accepted"
            );
        }
    }

    #[test]
    fn parse_families_keeps_held_out_without_the_automatic_key() {
        let result = parse_families("[held_out]\ngames = [1]\nautomatic = \"2026-11-10\"");
        assert!(matches!(result, Err(FamiliesError::Toml(_))));
    }

    #[test]
    fn load_families_without_a_file_is_empty() {
        let families = load_families(Path::new("does-not-exist/families.toml")).unwrap();
        assert_eq!(families, Families::default());
    }
}
