use std::path::Path;

use gamepalate_core::{
    families::{Families, Family},
    ids::AppId,
};
use gamepalate_store::{
    families::FamiliesError,
    families_edit::{FamiliesEditor, FamiliesSnapshot},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FamilyChange {
    AddGame { family: String, game: AppId },
    RemoveGame { family: String, game: AppId },
    Rename { from: String, to: String },
    Delete { family: String },
    HoldOut { game: AppId },
    Release { game: AppId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamiliesUndo {
    pub(crate) snapshot: FamiliesSnapshot,
    pub(crate) discover_family: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyGame {
    pub id: AppId,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FamilyChangeError {
    #[error("the name of a family needs at least one letter")]
    NameWithoutLetters,

    #[error("there is already a family named \"{0}\"")]
    NameTaken(String),

    #[error("no family named \"{0}\"")]
    NotFound(String),

    #[error("{game} is already in {family}")]
    AlreadyIn { game: String, family: String },

    #[error("families.toml is not valid, fix it by hand")]
    Invalid(#[source] FamiliesError),

    #[error("could not save families.toml")]
    Save(#[source] FamiliesError),
}

#[derive(Debug)]
pub(crate) struct ChangedFamilies {
    pub families: Families,
    pub snapshot: FamiliesSnapshot,
    pub renamed: Vec<(String, Option<String>)>,
}

pub(crate) fn changed_families(
    path: &Path,
    families: &Families,
    change: &FamilyChange,
    name: impl Fn(AppId) -> Option<String>,
) -> Result<ChangedFamilies, FamilyChangeError> {
    let comment = |game| name(game).unwrap_or_default();
    let mut renamed = Vec::new();
    let mut editor = FamiliesEditor::open(path).map_err(open_error)?;

    match change {
        FamilyChange::AddGame { family, game } => {
            let family = match find(families, family) {
                Ok(found) if found.games.contains(game) => {
                    return Err(FamilyChangeError::AlreadyIn {
                        game: name(*game).unwrap_or_else(|| format!("#{}", game.0)),
                        family: found.name.clone(),
                    });
                }
                Ok(found) => found.name.clone(),
                Err(_) => checked_name(families, family, None)?,
            };
            editor.add_game(&family, *game, &comment(*game));
            editor.release(*game);
        }
        FamilyChange::RemoveGame { family, game } => {
            let family = find(families, family)?;
            renamed.extend(remove(&mut editor, family, *game));
        }
        FamilyChange::Rename { from, to } => {
            let family = find(families, from)?;
            let to = checked_name(families, to, Some(&family.name))?;
            editor.rename(&family.name, &to);
            renamed.push((family.name.clone(), Some(to)));
        }
        FamilyChange::Delete { family } => {
            let family = find(families, family)?;
            editor.delete(&family.name);
            renamed.push((family.name.clone(), None));
        }
        FamilyChange::HoldOut { game } => {
            if !families.held_out.contains(game) {
                editor.hold_out(*game, &comment(*game));
            }
            for family in &families.families {
                renamed.extend(remove(&mut editor, family, *game));
            }
        }
        FamilyChange::Release { game } => {
            editor.release(*game);
        }
    }

    let (families, snapshot) = editor.save().map_err(FamilyChangeError::Save)?;
    Ok(ChangedFamilies {
        families,
        snapshot,
        renamed,
    })
}

fn remove(
    editor: &mut FamiliesEditor,
    family: &Family,
    game: AppId,
) -> Option<(String, Option<String>)> {
    if !family.games.contains(&game) {
        return None;
    }
    if family.games.iter().all(|&other| other == game) {
        editor.delete(&family.name);
        return Some((family.name.clone(), None));
    }
    editor.remove_game(&family.name, game);
    None
}

fn find<'a>(families: &'a Families, name: &str) -> Result<&'a Family, FamilyChangeError> {
    families
        .families
        .iter()
        .find(|family| family.name == name)
        .ok_or_else(|| FamilyChangeError::NotFound(name.to_string()))
}

fn checked_name(
    families: &Families,
    name: &str,
    except: Option<&str>,
) -> Result<String, FamilyChangeError> {
    let name = name.trim();
    if !name.chars().any(char::is_alphabetic) {
        return Err(FamilyChangeError::NameWithoutLetters);
    }
    let folded = name.to_lowercase();
    match families
        .families
        .iter()
        .find(|family| Some(family.name.as_str()) != except && family.name.to_lowercase() == folded)
    {
        Some(taken) => Err(FamilyChangeError::NameTaken(taken.name.clone())),
        None => Ok(name.to_string()),
    }
}

fn open_error(error: FamiliesError) -> FamilyChangeError {
    match error {
        FamiliesError::Io(_) | FamiliesError::Write(_) => FamilyChangeError::Save(error),
        FamiliesError::Toml(_) | FamiliesError::Edit(_) | FamiliesError::Family { .. } => {
            FamilyChangeError::Invalid(error)
        }
    }
}

pub(crate) fn family_games(
    families: &Families,
    family: &str,
    name: impl Fn(AppId) -> String,
) -> Vec<FamilyGame> {
    families
        .families
        .iter()
        .find(|found| found.name == family)
        .map(|found| {
            found
                .games
                .iter()
                .map(|&id| FamilyGame { id, name: name(id) })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn renamed_choice(
    choice: &Option<String>,
    renamed: &[(String, Option<String>)],
) -> Option<String> {
    renamed
        .iter()
        .fold(choice.clone(), |choice, (from, to)| match choice {
            Some(name) if name == *from => to.clone(),
            other => other,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamepalate_store::families::load_families;

    const FILE: &str = "\
[families.cozy]
games = [
    1, # One
    2, # Two
]

[families.mysteries]
games = [
    3, # Three
]

[held_out]
games = [
    4, # Four
]
";

    struct Folder {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }

    fn folder(text: &str) -> Folder {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");
        std::fs::write(&path, text).unwrap();
        Folder { _dir: dir, path }
    }

    fn change(folder: &Folder, change: FamilyChange) -> Result<ChangedFamilies, FamilyChangeError> {
        let families = load_families(&folder.path).unwrap();
        changed_families(&folder.path, &families, &change, |id| {
            (id.0 != 9).then(|| format!("Game {}", id.0))
        })
    }

    fn names(families: &Families) -> Vec<&str> {
        families
            .families
            .iter()
            .map(|family| family.name.as_str())
            .collect()
    }

    #[test]
    fn a_game_added_to_a_new_family_creates_it() {
        let folder = folder(FILE);
        let changed = change(
            &folder,
            FamilyChange::AddGame {
                family: "  slow puzzles ".to_string(),
                game: AppId(5),
            },
        )
        .unwrap();

        assert_eq!(
            names(&changed.families),
            ["cozy", "mysteries", "slow puzzles"]
        );
        assert_eq!(changed.families.families[2].games, [AppId(5)]);
        let text = std::fs::read_to_string(&folder.path).unwrap();
        assert!(text.contains("    5, # Game 5\n"));
    }

    #[test]
    fn a_name_needs_a_letter_and_must_be_new_whatever_the_case() {
        let folder = folder(FILE);
        let add = |family: &str| {
            change(
                &folder,
                FamilyChange::AddGame {
                    family: family.to_string(),
                    game: AppId(5),
                },
            )
        };

        assert!(matches!(
            add("42"),
            Err(FamilyChangeError::NameWithoutLetters)
        ));
        assert!(matches!(add("Cozy"), Err(FamilyChangeError::NameTaken(name)) if name == "cozy"));
        assert_eq!(std::fs::read_to_string(&folder.path).unwrap(), FILE);
    }

    #[test]
    fn a_game_is_not_added_twice() {
        let folder = folder(FILE);
        let result = change(
            &folder,
            FamilyChange::AddGame {
                family: "cozy".to_string(),
                game: AppId(1),
            },
        );
        assert!(matches!(
            result,
            Err(FamilyChangeError::AlreadyIn { game, family }) if game == "Game 1" && family == "cozy"
        ));
    }

    #[test]
    fn a_held_out_game_put_in_a_family_is_no_longer_held_out() {
        let folder = folder(FILE);
        let changed = change(
            &folder,
            FamilyChange::AddGame {
                family: "cozy".to_string(),
                game: AppId(4),
            },
        )
        .unwrap();

        assert_eq!(
            changed.families.families[0].games,
            [AppId(1), AppId(2), AppId(4)]
        );
        assert!(changed.families.held_out.is_empty());
    }

    #[test]
    fn holding_out_a_game_takes_it_out_of_its_families() {
        let folder = folder(FILE);
        let changed = change(&folder, FamilyChange::HoldOut { game: AppId(3) }).unwrap();

        assert_eq!(names(&changed.families), ["cozy"]);
        assert_eq!(changed.families.held_out, [AppId(4), AppId(3)]);
        assert_eq!(changed.renamed, [("mysteries".to_string(), None)]);
    }

    #[test]
    fn removing_the_last_game_deletes_the_family() {
        let folder = folder(FILE);
        let changed = change(
            &folder,
            FamilyChange::RemoveGame {
                family: "cozy".to_string(),
                game: AppId(1),
            },
        )
        .unwrap();
        assert_eq!(changed.families.families[0].games, [AppId(2)]);
        assert!(changed.renamed.is_empty());

        let changed = change(
            &folder,
            FamilyChange::RemoveGame {
                family: "mysteries".to_string(),
                game: AppId(3),
            },
        )
        .unwrap();
        assert_eq!(names(&changed.families), ["cozy"]);
        assert_eq!(changed.renamed, [("mysteries".to_string(), None)]);
    }

    #[test]
    fn a_family_can_change_only_the_case_of_its_name() {
        let folder = folder(FILE);
        let changed = change(
            &folder,
            FamilyChange::Rename {
                from: "cozy".to_string(),
                to: "Cozy".to_string(),
            },
        )
        .unwrap();

        assert_eq!(names(&changed.families), ["Cozy", "mysteries"]);
        assert_eq!(
            changed.renamed,
            [("cozy".to_string(), Some("Cozy".to_string()))]
        );
    }

    #[test]
    fn a_missing_family_is_named() {
        let folder = folder(FILE);
        let result = change(
            &folder,
            FamilyChange::Delete {
                family: "roguelike".to_string(),
            },
        );
        assert!(matches!(result, Err(FamilyChangeError::NotFound(name)) if name == "roguelike"));
    }

    #[test]
    fn a_game_without_a_name_is_written_without_a_comment() {
        let folder = folder(FILE);
        change(&folder, FamilyChange::HoldOut { game: AppId(9) }).unwrap();
        let text = std::fs::read_to_string(&folder.path).unwrap();
        assert!(text.ends_with("    4, # Four\n    9,\n]\n"));
    }

    #[test]
    fn an_invalid_file_is_left_alone() {
        let folder = folder("[families.cozy]\ngame = [1]\n");
        let result = changed_families(
            &folder.path,
            &Families::default(),
            &FamilyChange::Release { game: AppId(1) },
            |_| None,
        );
        assert!(matches!(result, Err(FamilyChangeError::Invalid(_))));
    }

    #[test]
    fn the_family_chosen_in_discover_follows_its_name() {
        let renamed = [
            ("cozy".to_string(), Some("calm".to_string())),
            ("mysteries".to_string(), None),
        ];
        assert_eq!(
            renamed_choice(&Some("cozy".to_string()), &renamed),
            Some("calm".to_string())
        );
        assert_eq!(
            renamed_choice(&Some("mysteries".to_string()), &renamed),
            None
        );
        assert_eq!(
            renamed_choice(&Some("roguelike".to_string()), &renamed),
            Some("roguelike".to_string())
        );
        assert_eq!(renamed_choice(&None, &renamed), None);
    }
}
