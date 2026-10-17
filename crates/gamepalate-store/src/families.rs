use std::{io::ErrorKind, path::Path};

use gamepalate_core::{
    families::{Families, Family},
    ids::AppId,
};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum FamiliesError {
    #[error("could not read the families")]
    Io(#[from] std::io::Error),

    #[error("the families file is not valid")]
    Toml(#[from] toml::de::Error),

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

pub fn load_families(path: &Path) -> Result<Families, FamiliesError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_families(&text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Families::default()),
        Err(error) => Err(error.into()),
    }
}

fn parse_families(text: &str) -> Result<Families, FamiliesError> {
    let file: FamiliesFile = toml::from_str(text)?;

    let families = file
        .families
        .into_iter()
        .map(|(name, value)| match value.try_into::<GamesSection>() {
            Ok(section) => Ok(Family {
                name,
                games: app_ids(section.games),
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
                    },
                    Family {
                        name: "alpha".to_string(),
                        games: vec![AppId(20)],
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
    fn load_families_without_a_file_is_empty() {
        let families = load_families(Path::new("does-not-exist/families.toml")).unwrap();
        assert_eq!(families, Families::default());
    }
}
