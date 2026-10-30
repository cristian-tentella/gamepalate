use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use gamepalate_core::{
    currency::Currency,
    filters::TagNameError,
    game_vector::{GameVector, game_vectors},
    ids::AppId,
    search::{
        Recommendation, RecommendedGame, SearchError, SearchInputs, SearchOptions, recommend,
    },
    taste::game_name,
    timestamp::Timestamp,
    votes::{VoteKind, Votes, cast},
};
use gamepalate_steam::{
    images::{ImageDownloader, ImageError, header_url},
    transport::{SteamError, Transport},
};
use gamepalate_store::{
    data_files::{DataFiles, DataFilesError, load_data_files},
    votes::{VotesError, save_votes},
};

use crate::{
    data_dir,
    details::{GameDetails, StoreDetails},
    images::{ImageCache, LoadImageError},
};

#[derive(Debug)]
pub struct Gamepalate {
    data_dir: PathBuf,
    files: DataFiles,
    votes: Mutex<Votes>,
    vectors: Vec<GameVector>,
    images: Arc<ImageCache>,
    store: Arc<StoreDetails>,
}

impl Gamepalate {
    pub fn open(data_dir: Option<PathBuf>) -> Result<Gamepalate, OpenError> {
        let dir = data_dir
            .or_else(data_dir::resolve)
            .ok_or(OpenError::DataDirNotFound)?;
        let files = load_data_files(&dir)?;
        let vectors = game_vectors(&files.catalog);
        let images = ImageCache::new(
            dirs::cache_dir().map(|dir| dir.join("gamepalate").join("cache").join("images")),
            ImageDownloader::new()?,
        );

        let store = StoreDetails::new(
            Transport::new(None)?.with_interval(Duration::ZERO),
            files.config.region,
        );

        Ok(Gamepalate {
            data_dir: dir,
            votes: Mutex::new(files.votes.clone()),
            files,
            vectors,
            images: Arc::new(images),
            store: Arc::new(store),
        })
    }

    pub fn recommend(&self, options: &SearchOptions) -> Result<Recommendation, RecommendError> {
        let votes = self.lock_votes();
        recommend(
            &SearchInputs {
                votes: &votes,
                ..self.files.search_inputs(&self.vectors)
            },
            options,
            Timestamp(jiff::Timestamp::now().as_millisecond()),
        )
        .map_err(RecommendError)
    }

    pub fn recommend_after(
        &self,
        options: &SearchOptions,
        seen: &[AppId],
    ) -> Result<Recommendation, RecommendError> {
        let count = options.limit.unwrap_or(self.files.config.results.count);
        let wider = SearchOptions {
            limit: Some(count + seen.len()),
            ..options.clone()
        };
        let mut recommendation = self.recommend(&wider)?;
        recommendation.games = unseen(recommendation.games, seen, count);

        Ok(recommendation)
    }

    pub fn vote(&self, id: AppId, kind: VoteKind, bet: bool) -> Result<(), VoteError> {
        let mut votes = self.lock_votes();
        let date = jiff::Zoned::now().date().to_string();
        let name = |id| {
            let name = game_name(id, &self.files.catalog, &self.files.off_catalog);
            (name != format!("#{}", id.0)).then_some(name)
        };
        *votes = saved_vote(
            &self.data_dir.join("votes.toml"),
            &votes,
            id,
            kind,
            date,
            bet,
            name,
        )?;
        Ok(())
    }

    fn lock_votes(&self) -> MutexGuard<'_, Votes> {
        self.votes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub async fn image(&self, url: &str) -> Result<Vec<u8>, LoadImageError> {
        self.images.image(url).await
    }

    pub async fn details(&self, id: AppId) -> Result<Option<GameDetails>, SteamError> {
        self.store.details(id).await
    }

    pub fn prune_images(&self) {
        self.images.prune();
    }

    pub fn header_url(&self, id: AppId) -> String {
        header_url(id)
    }

    pub fn bet_every(&self) -> usize {
        self.files.config.results.bet_every
    }

    pub fn signals_read_at(&self) -> Timestamp {
        self.files.signals.read_at
    }

    pub fn prices_updated_at(&self) -> Timestamp {
        self.files.catalog.freshness.prices_updated_at
    }

    pub fn currency(&self) -> &Currency {
        &self.files.catalog.currency
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

#[derive(Debug, thiserror::Error)]
#[error("could not save the vote in {}", path.display())]
pub struct VoteError {
    path: PathBuf,
    #[source]
    source: VotesError,
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

fn saved_vote(
    path: &Path,
    votes: &Votes,
    id: AppId,
    kind: VoteKind,
    date: String,
    bet: bool,
    name: impl Fn(AppId) -> Option<String>,
) -> Result<Votes, VoteError> {
    let mut changed = votes.clone();
    cast(&mut changed, id, kind, date, bet);
    save_votes(path, &changed, name).map_err(|source| VoteError {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(changed)
}

fn unseen(games: Vec<RecommendedGame>, seen: &[AppId], count: usize) -> Vec<RecommendedGame> {
    games
        .into_iter()
        .filter(|game| !seen.contains(&game.id))
        .take(count)
        .collect()
}

#[cfg(test)]
mod tests {
    use gamepalate_core::game::{Price, Reviews};

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

    fn vote_in(path: &Path, votes: &Votes) -> Result<Votes, VoteError> {
        saved_vote(
            path,
            votes,
            AppId(620),
            VoteKind::WouldTry,
            "2026-10-30".to_string(),
            true,
            |_| Some("Portal 2".to_string()),
        )
    }

    #[test]
    fn a_vote_is_saved_in_the_votes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("votes.toml");

        let votes = vote_in(&path, &Votes::new()).unwrap();

        assert_eq!(votes.len(), 1);
        assert_eq!(gamepalate_store::votes::load_votes(&path).unwrap(), votes);
        assert!(std::fs::read_to_string(&path).unwrap().contains(
            "620 = { vote = \"would_try\", date = \"2026-10-30\", bet = true } # Portal 2"
        ));
    }

    #[test]
    fn a_vote_that_cannot_be_saved_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("votes.toml");

        let error = vote_in(&path, &Votes::new()).err().unwrap();

        assert_eq!(
            error.to_string(),
            format!("could not save the vote in {}", path.display())
        );
    }

    #[test]
    fn skips_the_games_already_seen() {
        let games = (1..=5).map(game).collect();

        let ids: Vec<AppId> = unseen(games, &[AppId(1), AppId(3)], 2)
            .iter()
            .map(|game| game.id)
            .collect();

        assert_eq!(ids, vec![AppId(2), AppId(4)]);
    }

    fn game(id: u32) -> RecommendedGame {
        RecommendedGame {
            position: id as usize,
            id: AppId(id),
            name: format!("Game {id}"),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            store_url: String::new(),
            from_family: None,
            affinity: 1.0,
            top_tags: Vec::new(),
            similar_to: None,
            similar_to_multiplier: None,
            quality: 1.0,
            reviews: Reviews {
                total: 100,
                positive: 90,
            },
            in_wishlist: false,
            relevance: 1.0,
            bet: false,
        }
    }
}
