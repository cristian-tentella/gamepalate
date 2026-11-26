use std::path::Path;

use gamepalate_core::{
    catalog::Catalog, config::Config, corrections::Corrections, families::Families,
    game_vector::GameVector, lists::Lists, off_catalog::OffCatalogGames, search::SearchInputs,
    signals::Signals, votes::Votes,
};

use crate::{
    catalog::{CatalogError, load_catalog},
    config::{ConfigError, load_config},
    corrections::{CorrectionsError, load_corrections},
    families::{FamiliesError, load_families},
    known::{KnownConverted, convert_known},
    lists::{ListsError, load_lists},
    off_catalog::load_off_catalog_games,
    signals::{SignalsError, load_signals},
    votes::{ReadVotes, VotesError, load_votes},
};

#[derive(Debug, thiserror::Error)]
pub enum DataFilesError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Catalog(#[from] CatalogError),

    #[error(transparent)]
    Corrections(#[from] CorrectionsError),

    #[error(transparent)]
    Families(#[from] FamiliesError),

    #[error(transparent)]
    Signals(#[from] SignalsError),

    #[error(transparent)]
    Votes(#[from] VotesError),

    #[error(transparent)]
    Lists(#[from] ListsError),

    #[error("the vote for {appid} has list {list}, which is not in lists.toml")]
    UnknownList { appid: u32, list: u32 },
}

#[derive(Debug, Clone)]
pub struct DataFiles {
    pub config: Config,
    pub catalog: Catalog,
    pub corrections: Corrections,
    pub families: Families,
    pub signals: Signals,
    pub votes: Votes,
    pub lists: Lists,
    pub off_catalog: OffCatalogGames,
    pub known_converted: Option<KnownConverted>,
}

pub fn load_data_files(dir: &Path) -> Result<DataFiles, DataFilesError> {
    let config = load_config(&dir.join("config.toml"))?;
    let signals = load_signals(&dir.join("signals.json"))?;

    load_data_files_with(dir, config, signals)
}

pub fn load_data_files_with(
    dir: &Path,
    config: Config,
    signals: Signals,
) -> Result<DataFiles, DataFilesError> {
    let catalog = load_catalog(&dir.join("catalog.sqlite"))?;
    let corrections = load_corrections(&dir.join("corrections.toml"))?;
    let families = load_families(&dir.join("families.toml"))?;
    let ReadVotes { votes, known } = load_votes(&dir.join("votes.toml"))?;
    let lists = load_lists(&dir.join("lists.toml"))?;
    check_lists(&votes, &lists)?;
    let off_catalog = load_off_catalog_games(&dir.join("off-catalog-tags.json"));

    let mut files = DataFiles {
        config,
        catalog,
        corrections,
        families,
        signals,
        votes,
        lists,
        off_catalog,
        known_converted: None,
    };
    files.known_converted = convert_known(dir, &known, &mut files)?;
    Ok(files)
}

// An id that lists.toml lacks would leave the game in a list without a name.
fn check_lists(votes: &Votes, lists: &Lists) -> Result<(), DataFilesError> {
    for (id, vote) in votes {
        if let Some(list) = vote.lists.iter().find(|list| !lists.contains_key(list)) {
            return Err(DataFilesError::UnknownList {
                appid: id.0,
                list: list.0,
            });
        }
    }
    Ok(())
}

impl DataFiles {
    pub fn search_inputs<'a>(&'a self, game_vectors: &'a [GameVector]) -> SearchInputs<'a> {
        SearchInputs {
            catalog: &self.catalog,
            game_vectors,
            signals: &self.signals,
            votes: &self.votes,
            corrections: &self.corrections,
            families: &self.families,
            off_catalog: &self.off_catalog,
            config: &self.config,
        }
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_core::{
        ids::AppId,
        lists::ListId,
        votes::{Vote, VoteKind},
    };

    use super::*;

    fn saved_in(lists: &[u32]) -> Votes {
        Votes::from([(
            AppId(620),
            Vote {
                saved: true,
                lists: lists.iter().copied().map(ListId).collect(),
                ..Vote::new(VoteKind::WouldTry, "2026-11-25")
            },
        )])
    }

    #[test]
    fn every_list_of_a_vote_is_in_the_lists_file() {
        let lists = Lists::from([(ListId(2), "Co-op".to_string())]);

        assert!(check_lists(&saved_in(&[2]), &lists).is_ok());
        let error = check_lists(&saved_in(&[2, 9]), &lists).unwrap_err();
        assert!(matches!(
            error,
            DataFilesError::UnknownList {
                appid: 620,
                list: 9
            }
        ));
        assert_eq!(
            error.to_string(),
            "the vote for 620 has list 9, which is not in lists.toml"
        );
    }
}
