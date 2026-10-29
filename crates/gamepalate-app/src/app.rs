use std::path::PathBuf;

use gamepalate_core::{
    filters::TagNameError,
    game_vector::{GameVector, game_vectors},
    search::{Recommendation, SearchError, SearchOptions, recommend},
    timestamp::Timestamp,
};
use gamepalate_store::data_files::{DataFiles, DataFilesError, load_data_files};

use crate::data_dir;

#[derive(Debug, Clone)]
pub struct Gamepalate {
    data_dir: PathBuf,
    files: DataFiles,
    vectors: Vec<GameVector>,
}

impl Gamepalate {
    pub fn open(data_dir: Option<PathBuf>) -> Result<Gamepalate, OpenError> {
        let dir = data_dir
            .or_else(data_dir::resolve)
            .ok_or(OpenError::DataDirNotFound)?;
        let files = load_data_files(&dir)?;
        let vectors = game_vectors(&files.catalog);

        Ok(Gamepalate {
            data_dir: dir,
            files,
            vectors,
        })
    }

    pub fn recommend(&self, options: &SearchOptions) -> Result<Recommendation, RecommendError> {
        recommend(
            &self.files.search_inputs(&self.vectors),
            options,
            Timestamp(jiff::Timestamp::now().as_millisecond()),
        )
        .map_err(RecommendError)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("could not find the user data folder: set GAMEPALATE_DATA_DIR")]
    DataDirNotFound,

    #[error(transparent)]
    DataFiles(#[from] DataFilesError),
}

#[derive(Debug, thiserror::Error)]
#[error("{}", search_error_text(.0))]
pub struct RecommendError(pub SearchError);

fn search_error_text(error: &SearchError) -> String {
    match error {
        SearchError::FamilyNotFound { name, families } => format!(
            "no family named \"{name}\": the families are {}",
            families.join(", ")
        ),
        SearchError::TagName(TagNameError::NotFound(name)) => format!("no tag named \"{name}\""),
        SearchError::TagName(TagNameError::Ambiguous(name)) => {
            format!("more than one tag matches \"{name}\": write the full name")
        }
        SearchError::EmptySignalsProfile => {
            "the signals give an empty profile: no game above the hours threshold has tags"
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_existing_families_when_one_is_missing() {
        let error = RecommendError(SearchError::FamilyNotFound {
            name: "Puzzle".to_string(),
            families: vec!["Roguelike".to_string(), "Strategy".to_string()],
        });

        assert_eq!(
            error.to_string(),
            "no family named \"Puzzle\": the families are Roguelike, Strategy"
        );
    }

    #[test]
    fn asks_for_the_full_name_of_an_ambiguous_tag() {
        let error = RecommendError(SearchError::TagName(TagNameError::Ambiguous(
            "rogue".to_string(),
        )));

        assert_eq!(
            error.to_string(),
            "more than one tag matches \"rogue\": write the full name"
        );
    }
}
