use std::{path::PathBuf, sync::Arc};

use gamepalate_core::{
    filters::TagNameError,
    game_vector::{GameVector, game_vectors},
    ids::AppId,
    search::{Recommendation, RecommendedGame, SearchError, SearchOptions, recommend},
    timestamp::Timestamp,
};
use gamepalate_steam::images::{ImageDownloader, ImageError, header_url};
use gamepalate_store::data_files::{DataFiles, DataFilesError, load_data_files};

use crate::{
    data_dir,
    images::{ImageCache, LoadImageError},
};

#[derive(Debug, Clone)]
pub struct Gamepalate {
    data_dir: PathBuf,
    files: DataFiles,
    vectors: Vec<GameVector>,
    images: Arc<ImageCache>,
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

        Ok(Gamepalate {
            data_dir: dir,
            files,
            vectors,
            images: Arc::new(images),
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

    pub async fn image(&self, url: &str) -> Result<Vec<u8>, LoadImageError> {
        self.images.image(url).await
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
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("could not find the user data folder: set GAMEPALATE_DATA_DIR")]
    DataDirNotFound,

    #[error(transparent)]
    DataFiles(#[from] DataFilesError),

    #[error(transparent)]
    Images(#[from] ImageError),
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
