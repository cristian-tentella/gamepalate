use std::path::Path;

use gamepalate_core::{
    catalog::Catalog, config::Config, corrections::Corrections, families::Families,
    game_vector::GameVector, off_catalog::OffCatalogGames, search::SearchInputs, signals::Signals,
    votes::Votes,
};

use crate::{
    catalog::{CatalogError, load_catalog},
    config::{ConfigError, load_config},
    corrections::{CorrectionsError, load_corrections},
    families::{FamiliesError, load_families},
    off_catalog::load_off_catalog_games,
    signals::{SignalsError, load_signals},
    votes::{VotesError, load_votes},
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
}

pub struct DataFiles {
    pub config: Config,
    pub catalog: Catalog,
    pub corrections: Corrections,
    pub families: Families,
    pub signals: Signals,
    pub votes: Votes,
    pub off_catalog: OffCatalogGames,
}

pub fn load_data_files(dir: &Path) -> Result<DataFiles, DataFilesError> {
    let config = load_config(&dir.join("config.toml"))?;
    let catalog = load_catalog(&dir.join("catalog.sqlite"))?;
    let corrections = load_corrections(&dir.join("corrections.toml"))?;
    let families = load_families(&dir.join("families.toml"))?;
    let signals = load_signals(&dir.join("signals.json"))?;
    let votes = load_votes(&dir.join("votes.toml"))?;
    let off_catalog = load_off_catalog_games(&dir.join("off-catalog-tags.json"));

    Ok(DataFiles {
        config,
        catalog,
        corrections,
        families,
        signals,
        votes,
        off_catalog,
    })
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
