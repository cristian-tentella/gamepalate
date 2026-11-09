use std::{collections::BTreeSet, path::Path};

use gamepalate_core::{
    automatic_families::{automatic_families, grouped_games},
    effective_hours::hours_by_source,
    families::{Families, Family},
    ids::AppId,
    search::SearchInputs,
    taste::{Taste, signals_taste},
};
use gamepalate_store::{
    data_files::DataFiles,
    families::FamiliesError,
    families_edit::{FamiliesEditor, FamiliesSnapshot},
};

use crate::log::failed;

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
    pub(crate) families_played: Option<Option<Vec<AppId>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamiliesChanged {
    pub undo: FamiliesUndo,
    pub gone: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamiliesRemade {
    pub outcome: RemadeFamilies,
    pub undo: Option<FamiliesUndo>,
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

pub(crate) fn open_error(error: FamiliesError) -> FamilyChangeError {
    match error {
        FamiliesError::Io(_) | FamiliesError::Write(_) => FamilyChangeError::Save(error),
        FamiliesError::Toml(_) | FamiliesError::Edit(_) | FamiliesError::Family { .. } => {
            FamilyChangeError::Invalid(error)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomaticFamilies {
    Created(CreatedFamilies),
    FileExists,
    Off,
    TooFewGames,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedFamilies {
    pub families: Families,
    // The played games of that day, to tell later how many are new.
    pub played: Vec<AppId>,
}

// Writes families.toml from the played games, only where there is none yet.
pub fn create_automatic_families(
    path: &Path,
    inputs: &SearchInputs,
    date: &str,
) -> Result<AutomaticFamilies, FamiliesError> {
    if path.exists() {
        return Ok(AutomaticFamilies::FileExists);
    }
    if !inputs.config.families.automatic {
        return Ok(AutomaticFamilies::Off);
    }

    let taste = played_taste(inputs, &[]);
    let mut made = automatic_families(&taste, inputs.signals, inputs.catalog);
    made.truncate(MAX_FAMILIES);
    if made.is_empty() {
        return Ok(AutomaticFamilies::TooFewGames);
    }

    let mut editor = FamiliesEditor::open(path)?;
    add_heading(&mut editor, date);
    add_automatic(&mut editor, &made, inputs, date);
    let (families, _) = editor.save()?;

    Ok(AutomaticFamilies::Created(CreatedFamilies {
        families,
        played: grouped_games(&taste, inputs.signals, inputs.catalog),
    }))
}

fn played_taste(inputs: &SearchInputs, held_out: &[AppId]) -> Taste {
    let config = &inputs.config.signals;
    let hours = hours_by_source(
        inputs.signals,
        inputs.votes,
        &inputs.corrections.played_elsewhere,
        config,
    );
    let mut taste = signals_taste(
        &hours,
        inputs.catalog,
        inputs.off_catalog,
        inputs.corrections,
        config,
    );
    taste.games.retain(|game| !held_out.contains(&game.id));
    taste
}

// Only a new file gets the heading: one that is there already keeps its own.
fn add_heading(editor: &mut FamiliesEditor, date: &str) {
    editor.add_heading(&format!(
        "Made from the games you play on {date}. Change them as you like."
    ));
}

fn add_automatic(editor: &mut FamiliesEditor, made: &[Family], inputs: &SearchInputs, date: &str) {
    for family in made {
        for &game in &family.games {
            let name = inputs
                .catalog
                .game(game)
                .map(|game| game.name.as_str())
                .unwrap_or_default();
            editor.add_game(&family.name, game, name);
        }
        editor.set_automatic(&family.name, date);
    }
}

// How many played games the families have not seen before the remake is worth a hint.
const NEW_GAMES_HINT: usize = 10;
// Measured on UCSD: with the shares of the turns, more than 4 families change nothing.
pub const MAX_FAMILIES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemadeFamilies {
    Remade(Remade),
    NoNewFamilies,
    AlreadyMany,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remade {
    pub removed: Vec<Family>,
    pub added: Vec<Family>,
    pub families: Families,
    // The played games of that day, to tell later how many are new.
    pub played: Vec<AppId>,
    pub(crate) snapshot: FamiliesSnapshot,
}

impl Remade {
    // The removed families whose name is not back, for the one chosen in Discover.
    pub(crate) fn renamed(&self) -> Vec<(String, Option<String>)> {
        self.removed
            .iter()
            .filter(|old| {
                self.families
                    .families
                    .iter()
                    .all(|new| new.name != old.name)
            })
            .map(|old| (old.name.clone(), None))
            .collect()
    }
}

// What a remake would do, to ask before writing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemakePreview {
    Changes(RemakeChanges),
    NoNewFamilies,
    AlreadyMany,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemakeChanges {
    pub kept: Vec<String>,
    pub made_again: Vec<String>,
    pub removed: Vec<String>,
    pub added: Vec<String>,
}

struct Planned<'a> {
    removed: Vec<&'a Family>,
    added: Vec<Family>,
    taste: Taste,
    unchanged: Option<RemadeFamilies>,
}

// Replaces the automatic families with new ones for the tastes the user's own don't cover.
fn planned<'a>(inputs: &SearchInputs<'a>, date: &str) -> Planned<'a> {
    let families = inputs.families;
    let (removed, own): (Vec<&Family>, Vec<&Family>) = families
        .families
        .iter()
        .partition(|family| family.automatic.is_some());

    let taste = played_taste(inputs, &families.held_out);
    let in_own: BTreeSet<AppId> = own
        .iter()
        .flat_map(|family| family.games.iter().copied())
        .collect();
    let mut taken: BTreeSet<String> = own
        .iter()
        .map(|family| family.name.to_lowercase())
        .collect();
    let added: Vec<Family> = automatic_families(&taste, inputs.signals, inputs.catalog)
        .into_iter()
        .filter(|family| !covered(family, &in_own))
        .take(MAX_FAMILIES.saturating_sub(own.len()))
        .map(|family| Family {
            name: free_name(&family.name, &mut taken),
            automatic: Some(date.to_string()),
            ..family
        })
        .collect();

    let unchanged = (removed.is_empty() && added.is_empty()).then_some(
        if !own.is_empty() && own.len() >= MAX_FAMILIES {
            RemadeFamilies::AlreadyMany
        } else {
            RemadeFamilies::NoNewFamilies
        },
    );

    Planned {
        removed,
        added,
        taste,
        unchanged,
    }
}

pub fn remake_preview(inputs: &SearchInputs) -> RemakePreview {
    let planned = planned(inputs, "");
    match planned.unchanged {
        Some(RemadeFamilies::AlreadyMany) => return RemakePreview::AlreadyMany,
        Some(_) => return RemakePreview::NoNewFamilies,
        None => {}
    }

    let added: Vec<String> = planned
        .added
        .iter()
        .map(|family| family.name.clone())
        .collect();
    let (made_again, removed): (Vec<String>, Vec<String>) = planned
        .removed
        .iter()
        .map(|family| family.name.clone())
        .partition(|name| added.contains(name));

    RemakePreview::Changes(RemakeChanges {
        kept: inputs
            .families
            .families
            .iter()
            .filter(|family| family.automatic.is_none())
            .map(|family| family.name.clone())
            .collect(),
        added: added
            .into_iter()
            .filter(|name| !made_again.contains(name))
            .collect(),
        made_again,
        removed,
    })
}

pub fn remake_automatic_families(
    path: &Path,
    inputs: &SearchInputs,
    date: &str,
) -> Result<RemadeFamilies, FamiliesError> {
    let Planned {
        removed,
        added,
        taste,
        unchanged,
    } = planned(inputs, date);
    if let Some(unchanged) = unchanged {
        return Ok(unchanged);
    }

    let mut editor = FamiliesEditor::open(path)?;
    add_heading(&mut editor, date);
    editor.delete_automatic();
    add_automatic(&mut editor, &added, inputs, date);
    let (families, snapshot) = editor.save()?;

    Ok(RemadeFamilies::Remade(Remade {
        removed: removed.into_iter().cloned().collect(),
        added,
        families,
        played: grouped_games(&taste, inputs.signals, inputs.catalog),
        snapshot,
    }))
}

// The played games of today that the families did not see, once there are enough of them.
// Without the list of that day there is no way to tell.
pub(crate) fn new_played_games(inputs: &SearchInputs, seen: Option<&[AppId]>) -> Option<usize> {
    let seen: BTreeSet<AppId> = seen?.iter().copied().collect();
    let taste = played_taste(inputs, &inputs.families.held_out);
    let new = grouped_games(&taste, inputs.signals, inputs.catalog)
        .into_iter()
        .filter(|game| !seen.contains(game))
        .count();

    (new >= NEW_GAMES_HINT).then_some(new)
}

// A group is covered when at least half of its games are in the user's own families.
fn covered(family: &Family, in_own: &BTreeSet<AppId>) -> bool {
    let inside = family
        .games
        .iter()
        .filter(|game| in_own.contains(game))
        .count();
    2 * inside >= family.games.len()
}

fn free_name(name: &str, taken: &mut BTreeSet<String>) -> String {
    let mut free = name.to_string();
    let mut number = 2;
    while !taken.insert(free.to_lowercase()) {
        free = format!("{name} {number}");
        number += 1;
    }
    free
}

// On opening, the families just made replace the empty ones that were read.
pub(crate) fn families_on_open(
    path: &Path,
    files: &mut DataFiles,
    date: &str,
) -> Option<CreatedFamilies> {
    let outcome = create_automatic_families(path, &files.search_inputs(&[]), date)
        .inspect_err(failed("could not create the families"))
        .ok()?;
    let AutomaticFamilies::Created(created) = outcome else {
        return None;
    };
    files.families = created.families.clone();
    Some(created)
}

pub(crate) fn automatic_family_names(families: &Families) -> Vec<String> {
    families
        .families
        .iter()
        .filter(|family| family.automatic.is_some())
        .map(|family| family.name.clone())
        .collect()
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

    mod automatic {
        use std::collections::BTreeMap;

        use gamepalate_core::{
            catalog::{Catalog, Freshness, TagName},
            config::Config,
            corrections::Corrections,
            currency::Currency,
            game::{Game, GameTag, Price, Reviews},
            ids::TagId,
            region::Region,
            signals::{LibraryGame, Signals},
            timestamp::Timestamp,
            votes::Votes,
        };
        use gamepalate_store::{data_files::DataFiles, families::load_families};

        use super::super::*;

        const DATE: &str = "2026-11-10";
        const LATER: &str = "2026-12-01";

        // Game g01, g02, … is of genre g, with tags 2g − 1 and 2g, and has 10g + its number hours.
        fn files(played: &[u32]) -> DataFiles {
            let genres = played.iter().map(|id| id / 100).max().unwrap_or(0);
            let tag_names: Vec<(u32, String)> = (1..=2 * genres)
                .map(|id| (id, format!("Tag {id}")))
                .collect();
            let games = played
                .iter()
                .map(|&id| Game {
                    id: AppId(id),
                    name: format!("Game {id}"),
                    price: Price {
                        original_cents: 0,
                        discount: None,
                    },
                    reviews: Reviews {
                        total: 0,
                        positive: 0,
                    },
                    is_early_access: false,
                    tags: [2 * (id / 100) - 1, 2 * (id / 100)]
                        .iter()
                        .map(|&tag| GameTag {
                            id: TagId(tag),
                            weight: 1.0,
                        })
                        .collect(),
                    developers: Vec::new(),
                    franchises: Vec::new(),
                })
                .collect();
            let catalog = Catalog::new(
                games,
                tag_names
                    .iter()
                    .map(|(id, name)| {
                        (
                            TagId(*id),
                            TagName {
                                english: name.clone(),
                                italian: name.clone(),
                            },
                        )
                    })
                    .collect(),
                tag_names.iter().map(|&(id, _)| (TagId(id), 1.0)).collect(),
                Freshness {
                    prices_updated_at: Timestamp(0),
                    tags_updated_at: Timestamp(0),
                    region: Region::ITALY,
                },
                Currency::italian(),
            );

            DataFiles {
                config: Config::default(),
                catalog,
                corrections: Corrections::default(),
                families: Families::default(),
                signals: Signals {
                    read_at: Timestamp(0),
                    library: played
                        .iter()
                        .map(|&id| LibraryGame {
                            id: AppId(id),
                            total_hours: f64::from(id / 100 * 10 + id % 100),
                            last_played: None,
                        })
                        .collect(),
                    borrowed: Vec::new(),
                    recent: Vec::new(),
                    wishlist: Vec::new(),
                    followed: Vec::new(),
                },
                votes: Votes::new(),
                off_catalog: BTreeMap::new(),
            }
        }

        fn ten_games() -> DataFiles {
            genres(2, 5)
        }

        fn genres(genres: u32, games: u32) -> DataFiles {
            let played: Vec<u32> = (1..=genres)
                .flat_map(|genre| (1..=games).map(move |game| genre * 100 + game))
                .collect();
            files(&played)
        }

        fn written(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");
            if !text.is_empty() {
                std::fs::write(&path, text).unwrap();
            }
            (dir, path)
        }

        fn remake(path: &Path, files: &mut DataFiles) -> RemadeFamilies {
            files.families = load_families(path).unwrap();
            remake_automatic_families(path, &files.search_inputs(&[]), LATER).unwrap()
        }

        fn remade_ok(path: &Path, files: &mut DataFiles) -> Remade {
            match remake(path, files) {
                RemadeFamilies::Remade(remade) => remade,
                other => panic!("not remade: {other:?}"),
            }
        }

        fn family_names(families: &[Family]) -> Vec<&str> {
            families.iter().map(|family| family.name.as_str()).collect()
        }

        fn created(path: &Path, files: &DataFiles) -> AutomaticFamilies {
            create_automatic_families(path, &files.search_inputs(&[]), DATE).unwrap()
        }

        fn sorted(games: &[AppId]) -> Vec<u32> {
            let mut games: Vec<u32> = games.iter().map(|game| game.0).collect();
            games.sort();
            games
        }

        #[test]
        fn a_family_stops_being_automatic_when_the_user_changes_it() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");
            let AutomaticFamilies::Created(created) = created(&path, &ten_games()) else {
                panic!("no families made");
            };
            let first = created.families.families[0].name.clone();
            let second = created.families.families[1].name.clone();

            let changed = changed_families(
                &path,
                &created.families,
                &FamilyChange::Rename {
                    from: first,
                    to: "mine".to_string(),
                },
                |_| None,
            )
            .unwrap();

            assert_eq!(automatic_family_names(&changed.families), [second]);
        }

        #[test]
        fn without_a_file_the_families_are_written_with_their_day() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");
            let files = ten_games();

            let AutomaticFamilies::Created(created) = created(&path, &files) else {
                panic!("no families made");
            };

            let read = load_families(&path).unwrap();
            assert_eq!(read, created.families);
            assert_eq!(read.families.len(), 2);
            let groups: Vec<Vec<u32>> = read
                .families
                .iter()
                .map(|family| sorted(&family.games))
                .collect();
            assert!(groups.contains(&vec![101, 102, 103, 104, 105]));
            assert!(groups.contains(&vec![201, 202, 203, 204, 205]));
            assert!(
                read.families
                    .iter()
                    .all(|family| family.automatic.as_deref() == Some(DATE))
            );
            assert_eq!(sorted(&created.played), sorted(&files.signals.ids()));

            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.starts_with(&format!(
                "{}# Made from the games you play on {DATE}. Change them as you like.\n\n[families.",
                gamepalate_store::families_edit::NEW_FILE
            )));
            assert!(text.contains("    105, # Game 105\n"));
        }

        #[test]
        fn a_file_even_empty_is_left_as_it_is() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");
            std::fs::write(&path, "").unwrap();

            assert_eq!(created(&path, &ten_games()), AutomaticFamilies::FileExists);
            assert_eq!(std::fs::read(&path).unwrap(), b"");
        }

        #[test]
        fn fewer_than_six_played_games_make_no_file() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");

            let files = files(&[101, 102, 103, 201, 202]);
            assert_eq!(created(&path, &files), AutomaticFamilies::TooFewGames);
            assert!(!path.exists());
        }

        #[test]
        fn the_switch_off_makes_no_file() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");
            let mut files = ten_games();
            files.config.families.automatic = false;

            assert_eq!(created(&path, &files), AutomaticFamilies::Off);
            assert!(!path.exists());
        }

        #[test]
        fn the_families_are_there_at_once_and_made_only_once() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");
            let mut files = ten_games();

            let created = families_on_open(&path, &mut files, DATE).unwrap();
            assert_eq!(files.families, created.families);
            assert_eq!(files.families.families.len(), 2);

            for family in &created.families.families {
                let families = load_families(&path).unwrap();
                changed_families(
                    &path,
                    &families,
                    &FamilyChange::Delete {
                        family: family.name.clone(),
                    },
                    |_| None,
                )
                .unwrap();
            }
            let mut files = ten_games();
            assert_eq!(families_on_open(&path, &mut files, DATE), None);
            assert_eq!(files.families, Families::default());
            assert_eq!(load_families(&path).unwrap(), Families::default());
        }

        #[test]
        fn the_made_families_change_like_the_others() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");
            let AutomaticFamilies::Created(created) = created(&path, &ten_games()) else {
                panic!("no families made");
            };
            let first = created.families.families[0].clone();
            let second = created.families.families[1].clone();
            let change = |change: FamilyChange| {
                let families = load_families(&path).unwrap();
                changed_families(&path, &families, &change, |id| {
                    Some(format!("Game {}", id.0))
                })
                .unwrap()
                .families
            };

            let families = change(FamilyChange::RemoveGame {
                family: first.name.clone(),
                game: first.games[0],
            });
            assert_eq!(families.families[0].games, first.games[1..]);
            assert_eq!(families.families[0].automatic, None);
            assert_eq!(families.families[1].automatic.as_deref(), Some(DATE));

            let families = change(FamilyChange::Rename {
                from: second.name.clone(),
                to: "Brawlers".to_string(),
            });
            assert_eq!(families.families[1].name, "Brawlers");
            assert_eq!(families.families[1].automatic, None);

            let families = change(FamilyChange::Delete {
                family: "Brawlers".to_string(),
            });
            assert_eq!(families.families.len(), 1);

            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("Made from the games you play on 2026-11-10."));
            for game in &first.games[1..] {
                assert!(text.contains(&format!("    {}, # Game {}\n", game.0, game.0)));
            }
        }

        const MIXED: &str = "\
# Mine
[families.mine]
games = [
    999, # Not played
]

[families.old]
games = [101, 201]
automatic = \"2026-11-10\"

# Also mine
[families.\"Tag 3\"]
games = [998]

[held_out]
games = [
    105, # Game 105
]
";

        #[test]
        fn a_remake_replaces_only_the_automatic_families() {
            let (_dir, path) = written(MIXED);
            let remade = remade_ok(&path, &mut ten_games());

            assert_eq!(family_names(&remade.removed), ["old"]);
            assert_eq!(
                family_names(&remade.families.families),
                ["mine", "Tag 3", "Tag 3 2", "Tag 1"]
            );
            assert_eq!(remade.added, remade.families.families[2..]);
            assert!(
                remade
                    .added
                    .iter()
                    .all(|family| family.automatic.as_deref() == Some(LATER))
            );
            assert_eq!(sorted(&remade.added[1].games), [101, 102, 103, 104]);
            assert_eq!(remade.families.held_out, [AppId(105)]);
            assert!(!remade.played.contains(&AppId(105)));

            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.starts_with(
                "# Mine\n[families.mine]\ngames = [\n    999, # Not played\n]\n\n# Also mine\n[families.\"Tag 3\"]\ngames = [998]\n\n[families."
            ));
            assert!(text.ends_with("\n\n[held_out]\ngames = [\n    105, # Game 105\n]\n"));
            assert!(!text.contains("[families.old]"));
            assert!(text.contains("    205, # Game 205\n"));
        }

        #[test]
        fn a_group_with_half_its_games_in_your_families_is_left_out() {
            let (_dir, path) = written("[families.mine]\ngames = [110, 109, 108, 107]\n");
            let remade = remade_ok(&path, &mut genres(2, 10));
            assert_eq!(family_names(&remade.added), ["Tag 3"]);

            let (_dir, path) = written("[families.mine]\ngames = [110, 109, 108]\n");
            let remade = remade_ok(&path, &mut genres(2, 10));
            assert_eq!(family_names(&remade.added), ["Tag 3", "Tag 1"]);
            assert!(remade.removed.is_empty());
        }

        #[test]
        fn with_your_own_families_there_are_never_more_than_four() {
            let own = |count: u32| -> String {
                (1..=count)
                    .map(|i| {
                        format!(
                            "[families.mine{i}]
games = [{}]
",
                            990 + i
                        )
                    })
                    .collect::<Vec<String>>()
                    .join(
                        "
",
                    )
            };

            let (_dir, path) = written(&own(2));
            let remade = remade_ok(&path, &mut genres(6, 21));
            assert_eq!(remade.added.len(), 2);
            assert_eq!(remade.families.families.len(), 4);
            assert!(
                remade
                    .added
                    .iter()
                    .all(|family| family.games.iter().all(|game| game.0 / 100 != 1))
            );

            let (_dir, path) = written(&own(4));
            assert_eq!(
                remake(&path, &mut genres(6, 21)),
                RemadeFamilies::AlreadyMany
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), own(4));

            let (_dir, path) = written(&own(6));
            assert_eq!(
                remake(&path, &mut genres(6, 21)),
                RemadeFamilies::AlreadyMany
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), own(6));
        }

        #[test]
        fn the_first_creation_makes_at_most_four_families() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("families.toml");

            let AutomaticFamilies::Created(created) = created(&path, &genres(9, 21)) else {
                panic!("no families made");
            };

            assert_eq!(created.families.families.len(), MAX_FAMILIES);
            assert_eq!(load_families(&path).unwrap().families.len(), MAX_FAMILIES);
        }

        #[test]
        fn without_your_own_families_there_are_up_to_four() {
            let (_dir, path) = written(
                "[families.old]
games = [101]
automatic = \"2026-11-10\"
",
            );
            let remade = remade_ok(&path, &mut genres(9, 21));
            assert_eq!(family_names(&remade.removed), ["old"]);
            assert_eq!(remade.added.len(), MAX_FAMILIES);

            let (_dir, path) = written("");
            let remade = remade_ok(&path, &mut genres(16, 16));
            assert_eq!(remade.added.len(), 4);
        }

        #[test]
        fn without_a_file_the_remake_makes_it_even_with_the_switch_off() {
            let (_dir, path) = written("");
            let mut files = ten_games();
            files.config.families.automatic = false;

            let remade = remade_ok(&path, &mut files);

            assert!(remade.removed.is_empty());
            assert_eq!(remade.added.len(), 2);
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.starts_with(&format!(
                "{}# Made from the games you play on {LATER}.",
                gamepalate_store::families_edit::NEW_FILE
            )));
        }

        #[test]
        fn nothing_new_leaves_the_file_as_it_is() {
            let own = "# Mine\n[families.mine]\ngames = [101, 102, 103, 104, 105, 201, 202, 203]\n";
            let (_dir, path) = written(own);
            assert_eq!(
                remake(&path, &mut ten_games()),
                RemadeFamilies::NoNewFamilies
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), own);

            let (_dir, path) = written("");
            assert_eq!(
                remake(&path, &mut files(&[101, 102, 201])),
                RemadeFamilies::NoNewFamilies
            );
            assert!(!path.exists());
        }

        #[test]
        fn the_undo_puts_back_the_file_and_the_family_chosen_in_discover() {
            let (_dir, path) = written(MIXED);
            let remade = remade_ok(&path, &mut ten_games());

            let renamed = remade.renamed();
            assert_eq!(renamed, [("old".to_string(), None)]);
            assert_eq!(renamed_choice(&Some("old".to_string()), &renamed), None);
            assert_eq!(
                renamed_choice(&Some("mine".to_string()), &renamed),
                Some("mine".to_string())
            );

            let families =
                gamepalate_store::families_edit::restore_families(&path, &remade.snapshot).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), MIXED);
            assert_eq!(family_names(&families.families), ["mine", "old", "Tag 3"]);

            remade_ok(&path, &mut ten_games());
            let again = remade_ok(&path, &mut ten_games());
            assert_eq!(again.removed, again.added);
            assert!(again.renamed().is_empty());
        }

        #[test]
        fn the_preview_tells_what_stays_goes_and_comes_and_writes_nothing() {
            let (_dir, path) = written(MIXED);
            let mut files = ten_games();
            files.families = load_families(&path).unwrap();

            assert_eq!(
                remake_preview(&files.search_inputs(&[])),
                RemakePreview::Changes(RemakeChanges {
                    kept: vec!["mine".to_string(), "Tag 3".to_string()],
                    made_again: Vec::new(),
                    removed: vec!["old".to_string()],
                    added: vec!["Tag 3 2".to_string(), "Tag 1".to_string()],
                })
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), MIXED);

            remade_ok(&path, &mut files);
            files.families = load_families(&path).unwrap();
            let RemakePreview::Changes(again) = remake_preview(&files.search_inputs(&[])) else {
                panic!("no changes");
            };
            assert_eq!(again.made_again, ["Tag 3 2", "Tag 1"]);
            assert!(again.removed.is_empty() && again.added.is_empty());
        }

        #[test]
        fn the_preview_says_when_nothing_would_change() {
            let (_dir, path) = written(
                "[families.mine]
games = [101, 102, 103, 104, 105, 201, 202, 203]
",
            );
            let mut files = ten_games();
            files.families = load_families(&path).unwrap();
            assert_eq!(
                remake_preview(&files.search_inputs(&[])),
                RemakePreview::NoNewFamilies
            );
        }

        fn seen(games: &[u32]) -> Vec<AppId> {
            games.iter().map(|&game| AppId(game)).collect()
        }

        #[test]
        fn ten_new_played_games_are_a_hint_and_nine_are_not() {
            let files = genres(2, 10);
            let inputs = files.search_inputs(&[]);
            let first_ten: Vec<u32> = (101..=110).collect();

            assert_eq!(new_played_games(&inputs, Some(&seen(&first_ten))), Some(10));

            let eleven: Vec<u32> = (101..=110).chain([201]).collect();
            assert_eq!(new_played_games(&inputs, Some(&seen(&eleven))), None);
        }

        #[test]
        fn without_the_list_of_the_day_there_is_no_hint() {
            let files = genres(2, 10);

            assert_eq!(new_played_games(&files.search_inputs(&[]), None), None);
            assert_eq!(
                new_played_games(&files.search_inputs(&[]), Some(&[])),
                Some(20)
            );
        }

        #[test]
        fn a_remake_brings_the_hint_down_to_nothing() {
            let (_dir, path) = written("");
            let mut files = genres(2, 10);
            let remade = remade_ok(&path, &mut files);

            assert_eq!(
                new_played_games(&files.search_inputs(&[]), Some(&remade.played)),
                None
            );
        }

        #[test]
        fn a_game_held_out_is_not_a_new_one() {
            let mut files = genres(2, 10);
            files.families.held_out = (101..=110).map(AppId).collect();

            assert_eq!(
                new_played_games(&files.search_inputs(&[]), Some(&[])),
                Some(10)
            );
        }
    }
}
