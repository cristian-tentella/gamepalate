mod config;
mod families;
mod library;
mod media;
mod votes;

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use gamepalate_core::{
    corrections::Corrections,
    families::Families,
    filters::TagNameError,
    game_vector::{GameVector, game_vectors},
    ids::AppId,
    search::{
        Continuation, Recommendation, SearchError, SearchInputs, SearchOptions, recommend,
        recommend_from,
    },
    timestamp::Timestamp,
    votes::Votes,
};
use gamepalate_steam::{
    images::{ImageDownloader, ImageError},
    transport::{SteamError, Transport},
};
use gamepalate_store::{
    data_files::{DataFiles, DataFilesError, load_data_files},
    gui::{GuiSettings, load_gui_settings, save_gui_settings},
};

use crate::{
    capsules::CapsuleFinder,
    catalog_search::NameIndex,
    data_dir,
    details::StoreDetails,
    families::{CreatedFamilies, families_on_open, renamed_choice},
    images::{ImageCache, cache_dir},
    log::{failed, known_converted},
};
pub use votes::{VoteError, VotedGame};

pub(crate) const GUI_FILE: &str = "gui.toml";
const FAMILIES_FILE: &str = "families.toml";

#[derive(Debug)]
pub struct Gamepalate {
    data_dir: PathBuf,
    files: DataFiles,
    votes: Mutex<Votes>,
    corrections: Mutex<Corrections>,
    families: Mutex<Families>,
    created_families: Option<CreatedFamilies>,
    gui_saving: Mutex<()>,
    vectors: Vec<GameVector>,
    names: NameIndex,
    images: Arc<ImageCache>,
    store: Arc<StoreDetails>,
    capsules: Arc<CapsuleFinder>,
}

impl Gamepalate {
    pub fn open(data_dir: Option<PathBuf>) -> Result<Gamepalate, OpenError> {
        let dir = data_dir
            .or_else(data_dir::resolve)
            .ok_or(OpenError::DataDirNotFound)?;
        let mut files =
            load_data_files(&dir).inspect_err(failed("could not read the data files"))?;
        known_converted(files.known_converted);
        let created_families = families_on_open(
            &dir.join(FAMILIES_FILE),
            &mut files,
            &jiff::Zoned::now().date().to_string(),
        );
        let vectors = game_vectors(&files.catalog);
        let names = NameIndex::new(&files.catalog);
        let images = ImageCache::new(cache_dir(), ImageDownloader::new()?);

        let store = StoreDetails::new(
            Transport::new(None)?.with_interval(Duration::ZERO),
            files.config.region,
        );
        let capsules = CapsuleFinder::new(
            Transport::new(None)?.with_interval(Duration::ZERO),
            files.config.region,
            files
                .signals
                .ids()
                .into_iter()
                .chain(files.corrections.played_elsewhere.iter().copied())
                .collect::<BTreeSet<AppId>>()
                .into_iter()
                .collect(),
        );

        Ok(Gamepalate {
            data_dir: dir,
            votes: Mutex::new(files.votes.clone()),
            corrections: Mutex::new(files.corrections.clone()),
            families: Mutex::new(files.families.clone()),
            created_families,
            gui_saving: Mutex::new(()),
            files,
            vectors,
            names,
            images: Arc::new(images),
            store: Arc::new(store),
            capsules: Arc::new(capsules),
        })
    }

    pub fn recommend(&self, options: &SearchOptions) -> Result<Recommendation, RecommendError> {
        self.with_inputs(|inputs| {
            recommend(
                inputs,
                options,
                Timestamp(jiff::Timestamp::now().as_millisecond()),
            )
        })
        .map_err(RecommendError)
        .inspect_err(failed("could not recommend"))
    }

    pub fn recommend_after(
        &self,
        options: &SearchOptions,
        continuation: &Continuation,
    ) -> Result<Recommendation, RecommendError> {
        self.with_inputs(|inputs| {
            recommend_from(
                inputs,
                options,
                continuation.clone(),
                Timestamp(jiff::Timestamp::now().as_millisecond()),
            )
        })
        .map_err(RecommendError)
        .inspect_err(failed("could not recommend"))
    }

    fn lock_votes(&self) -> MutexGuard<'_, Votes> {
        self.votes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_corrections(&self) -> MutexGuard<'_, Corrections> {
        self.corrections
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_families(&self) -> MutexGuard<'_, Families> {
        self.families
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn with_inputs<R>(&self, f: impl FnOnce(&SearchInputs) -> R) -> R {
        let votes = self.lock_votes();
        let corrections = self.lock_corrections();
        let families = self.lock_families();
        f(&SearchInputs {
            votes: &votes,
            corrections: &corrections,
            families: &families,
            ..self.files.search_inputs(&self.vectors)
        })
    }

    // The played games of the day the families were made, to tell later how many are new.
    fn remember_played(&self, played: &[AppId]) -> Option<Option<Vec<AppId>>> {
        let mut before = None;
        let _ = self.change_gui_settings(|settings| {
            before = Some(settings.families_played.replace(played.to_vec()));
        });
        before
    }

    // The family chosen in Discover follows a rename, or goes back to all when it is gone.
    fn follow_renamed(&self, renamed: &[(String, Option<String>)]) -> Option<Option<String>> {
        let mut discover_family = None;
        if !renamed.is_empty() {
            let _ = self.change_gui_settings(|settings| {
                let choice = renamed_choice(&settings.search.family, renamed);
                if choice != settings.search.family {
                    discover_family = Some(std::mem::replace(&mut settings.search.family, choice));
                }
            });
        }
        discover_family
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("could not find the user data folder: set GAMEPALATE_DATA_DIR")]
    DataDirNotFound,

    #[error(transparent)]
    DataFiles(#[from] DataFilesError),

    #[error(transparent)]
    Images(#[from] ImageError),

    #[error(transparent)]
    Steam(#[from] SteamError),
}

impl OpenError {
    pub fn unreadable_catalog(&self) -> bool {
        matches!(self, OpenError::DataFiles(DataFilesError::Catalog(_)))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
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

pub(crate) fn change_settings(
    path: &Path,
    change: impl FnOnce(&mut GuiSettings),
) -> std::io::Result<()> {
    let mut settings = load_gui_settings(path);
    change(&mut settings);
    save_gui_settings(path, &settings)
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
