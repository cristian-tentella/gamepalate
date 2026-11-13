use std::{collections::BTreeSet, path::Path};

use gamepalate_core::{filters::owned_games, ids::AppId, taste::game_name};

use crate::{
    corrections_edit::{CorrectionsEditor, EntryValue, Section},
    data_files::{DataFiles, DataFilesError},
    votes::save_votes,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownConverted {
    pub votes: usize,
    pub played_elsewhere: usize,
}

// Known is gone. It was used as "I played it", so a game not in the library becomes
// played elsewhere, and an owned one only loses the vote.
// The corrections are saved before the votes, so a stop in between finds the same games owned.
pub(crate) fn convert_known(
    dir: &Path,
    known: &BTreeSet<AppId>,
    files: &mut DataFiles,
) -> Result<Option<KnownConverted>, DataFilesError> {
    if known.is_empty() {
        return Ok(None);
    }

    let name = |id: AppId| {
        let name = game_name(id, &files.catalog, &files.off_catalog);
        (name != format!("#{}", id.0)).then_some(name)
    };
    let owned = owned_games(&files.signals, &files.corrections);
    let played_elsewhere: Vec<AppId> = known
        .iter()
        .copied()
        .filter(|id| !owned.contains(id))
        .collect();

    if !played_elsewhere.is_empty() {
        let mut editor = CorrectionsEditor::open(&dir.join("corrections.toml"))?;
        for &id in &played_elsewhere {
            editor.set(
                Section::PlayedElsewhere,
                id.0,
                EntryValue::Bool(true),
                name(id).as_deref(),
            );
        }
        files.corrections = editor.save()?;
    }
    save_votes(&dir.join("votes.toml"), &files.votes, name)?;

    Ok(Some(KnownConverted {
        votes: known.len(),
        played_elsewhere: played_elsewhere.len(),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gamepalate_core::{
        catalog::{Catalog, Freshness},
        config::Config,
        corrections::Corrections,
        currency::Currency,
        families::Families,
        off_catalog::{OffCatalogGame, OffCatalogGames},
        region::Region,
        signals::{LibraryGame, Signals},
        timestamp::Timestamp,
    };

    use super::*;
    use crate::{corrections::load_corrections, votes::load_votes};

    const CORRECTIONS: &str = "# my corrections

[hidden_games]
# hidden by hand
40 = true
";

    const CONVERTED: &str = "
[played_elsewhere]
40 = true
50 = true # Firewatch
";

    const VOTES: &str = r#"
[votes]
10 = { vote = "known", date = "2026-10-07" }
20 = { vote = "known", date = "2026-10-07" }
30 = { vote = "known", date = "2026-10-07" }
40 = { vote = "known", date = "2026-10-07" }
50 = { vote = "known", date = "2026-10-07", bet = true }
60 = { vote = "would_try", date = "2026-10-08" }
"#;

    fn library(id: u32) -> LibraryGame {
        LibraryGame {
            id: AppId(id),
            total_hours: 3.0,
            last_played: None,
        }
    }

    // 10 owned, 20 borrowed, 30 played elsewhere, 40 already hidden, 50 none of these.
    fn written(votes: &str, corrections: &str) -> (tempfile::TempDir, BTreeSet<AppId>, DataFiles) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("votes.toml"), votes).unwrap();
        std::fs::write(dir.path().join("corrections.toml"), corrections).unwrap();
        let read = load_votes(&dir.path().join("votes.toml")).unwrap();
        let mut corrections = load_corrections(&dir.path().join("corrections.toml")).unwrap();
        corrections.played_elsewhere.insert(AppId(30));
        let files = DataFiles {
            config: Config::default(),
            catalog: Catalog::new(
                Vec::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                Freshness {
                    prices_updated_at: Timestamp(0),
                    tags_updated_at: Timestamp(0),
                    region: Region::ITALY,
                },
                Currency::italian(),
            ),
            corrections,
            families: Families::default(),
            signals: Signals {
                read_at: Timestamp(0),
                library: vec![library(10)],
                borrowed: vec![library(20)],
                recent: Vec::new(),
                wishlist: vec![AppId(50)],
                followed: Vec::new(),
            },
            votes: read.votes,
            off_catalog: OffCatalogGames::from([(
                AppId(50),
                OffCatalogGame {
                    tags: Vec::new(),
                    name: Some("Firewatch".to_string()),
                },
            )]),
            known_converted: None,
        };
        (dir, read.known, files)
    }

    fn text(dir: &tempfile::TempDir, file: &str) -> String {
        std::fs::read_to_string(dir.path().join(file)).unwrap()
    }

    #[test]
    fn an_owned_game_loses_the_vote_and_any_other_is_played_elsewhere() {
        let (dir, known, mut files) = written(VOTES, CORRECTIONS);

        let converted = convert_known(dir.path(), &known, &mut files).unwrap();

        assert_eq!(
            converted,
            Some(KnownConverted {
                votes: 5,
                played_elsewhere: 2
            })
        );
        assert_eq!(
            files.corrections.played_elsewhere,
            BTreeSet::from([AppId(40), AppId(50)])
        );
        assert_eq!(files.corrections.hidden_games, BTreeSet::from([AppId(40)]));
        let read = load_votes(&dir.path().join("votes.toml")).unwrap();
        assert!(read.known.is_empty());
        assert_eq!(read.votes.keys().copied().collect::<Vec<_>>(), [AppId(60)]);
        assert_eq!(read.votes, files.votes);
    }

    #[test]
    fn the_comments_of_the_corrections_stay() {
        let (dir, known, mut files) = written(VOTES, CORRECTIONS);

        convert_known(dir.path(), &known, &mut files).unwrap();

        assert_eq!(
            text(&dir, "corrections.toml"),
            format!("{CORRECTIONS}{CONVERTED}")
        );
    }

    #[test]
    fn a_second_start_converts_nothing() {
        let (dir, known, mut files) = written(VOTES, CORRECTIONS);
        convert_known(dir.path(), &known, &mut files).unwrap();
        let votes = text(&dir, "votes.toml");
        let corrections = text(&dir, "corrections.toml");

        let (_, known, mut files) = written(&votes, &corrections);
        assert!(known.is_empty());
        std::fs::remove_file(dir.path().join("votes.toml")).unwrap();

        assert_eq!(convert_known(dir.path(), &known, &mut files).unwrap(), None);
        assert!(!dir.path().join("votes.toml").exists());
        assert_eq!(text(&dir, "corrections.toml"), corrections);
    }

    #[test]
    fn a_stop_between_the_two_saves_is_picked_up_at_the_next_start() {
        let saved = format!("{CORRECTIONS}{CONVERTED}");
        let (dir, known, mut files) = written(VOTES, &saved);

        let converted = convert_known(dir.path(), &known, &mut files).unwrap();

        assert_eq!(
            converted,
            Some(KnownConverted {
                votes: 5,
                played_elsewhere: 0
            })
        );
        assert_eq!(text(&dir, "corrections.toml"), saved);
        assert!(!text(&dir, "votes.toml").contains("known"));
    }

    #[test]
    fn without_a_corrections_file_one_is_made() {
        let (dir, known, mut files) = written(VOTES, "");
        std::fs::remove_file(dir.path().join("corrections.toml")).unwrap();
        files.corrections = Corrections {
            played_elsewhere: BTreeSet::from([AppId(30)]),
            ..Corrections::default()
        };

        convert_known(dir.path(), &known, &mut files).unwrap();

        assert_eq!(
            files.corrections.played_elsewhere,
            BTreeSet::from([AppId(40), AppId(50)])
        );
        assert_eq!(
            load_corrections(&dir.path().join("corrections.toml"))
                .unwrap()
                .played_elsewhere,
            files.corrections.played_elsewhere
        );
    }
}
