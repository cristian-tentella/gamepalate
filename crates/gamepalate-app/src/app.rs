use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use gamepalate_core::{
    catalog::Catalog,
    currency::Currency,
    filters::TagNameError,
    game::{Price, Reviews},
    game_vector::{GameVector, game_vectors},
    ids::AppId,
    off_catalog::OffCatalogGames,
    search::{
        Recommendation, RecommendedGame, SearchError, SearchInputs, SearchOptions, recommend,
    },
    taste::game_name,
    timestamp::Timestamp,
    votes::{Vote, VoteKind, Votes, cast},
};
use gamepalate_steam::{
    images::{ImageDownloader, ImageError, header_url},
    transport::{SteamError, Transport},
};
use gamepalate_store::{
    data_files::{DataFiles, DataFilesError, load_data_files},
    gui::{GuiSettings, load_gui_settings, save_gui_settings},
    votes::{VotesError, save_votes},
};

use crate::{
    data_dir,
    details::{GameDetails, StoreDetails},
    images::{ImageCache, LoadImageError},
};

pub(crate) const GUI_FILE: &str = "gui.toml";

#[derive(Debug)]
pub struct Gamepalate {
    data_dir: PathBuf,
    files: DataFiles,
    votes: Mutex<Votes>,
    gui_saving: Mutex<()>,
    vectors: Vec<GameVector>,
    images: Arc<ImageCache>,
    store: Arc<StoreDetails>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VotedGame {
    pub id: AppId,
    pub name: String,
    pub vote: Vote,
    pub price: Option<Price>,
    pub reviews: Option<Reviews>,
    pub store_url: String,
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
            gui_saving: Mutex::new(()),
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

    pub fn voted_games(&self) -> Vec<VotedGame> {
        voted_games(
            &self.lock_votes(),
            &self.files.catalog,
            &self.files.off_catalog,
            Timestamp(jiff::Timestamp::now().as_millisecond()),
        )
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

    pub fn gui_settings(&self) -> GuiSettings {
        load_gui_settings(&self.data_dir.join(GUI_FILE))
    }

    pub fn change_gui_settings(
        &self,
        change: impl FnOnce(&mut GuiSettings),
    ) -> std::io::Result<()> {
        let _saving = self
            .gui_saving
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change_settings(&self.data_dir.join(GUI_FILE), change)
    }

    pub fn family_names(&self) -> Vec<String> {
        self.files
            .families
            .families
            .iter()
            .map(|family| family.name.clone())
            .collect()
    }

    pub fn tag_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .files
            .catalog
            .tags
            .values()
            .map(|tag| tag.english.clone())
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup();
        names
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

pub(crate) fn change_settings(
    path: &Path,
    change: impl FnOnce(&mut GuiSettings),
) -> std::io::Result<()> {
    let mut settings = load_gui_settings(path);
    change(&mut settings);
    save_gui_settings(path, &settings)
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

fn voted_games(
    votes: &Votes,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
    now: Timestamp,
) -> Vec<VotedGame> {
    let mut games: Vec<VotedGame> = votes
        .iter()
        .map(|(&id, vote)| {
            let game = catalog.game(id);
            VotedGame {
                id,
                name: game_name(id, catalog, off_catalog),
                vote: vote.clone(),
                price: game.map(|game| game.price.as_of(now)),
                reviews: game.map(|game| game.reviews),
                store_url: format!("https://store.steampowered.com/app/{}/", id.0),
            }
        })
        .collect();
    games.sort_by_cached_key(|game| (Reverse(game.vote.date.clone()), game.name.to_lowercase()));
    games
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
    use std::collections::BTreeMap;

    use gamepalate_core::{
        catalog::Freshness,
        game::{Discount, Game},
        off_catalog::OffCatalogGame,
        region::Region,
    };

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
    fn the_voted_games_come_newest_first_with_todays_price() {
        let mut votes = Votes::new();
        cast(
            &mut votes,
            AppId(1),
            VoteKind::Maybe,
            "2026-10-01".to_string(),
            false,
        );
        cast(
            &mut votes,
            AppId(2),
            VoteKind::WouldTry,
            "2026-10-30".to_string(),
            true,
        );
        let mut catalog_game = Game {
            id: AppId(1),
            name: "Subnautica".to_string(),
            price: Price {
                original_cents: 2999,
                discount: Some(Discount {
                    percent: 75,
                    final_cents: 749,
                    end: Some(Timestamp(10)),
                }),
            },
            reviews: Reviews {
                total: 100,
                positive: 95,
            },
            is_early_access: false,
            tags: Vec::new(),
        };
        let catalog = catalog(vec![catalog_game.clone()]);

        let games = voted_games(&votes, &catalog, &OffCatalogGames::new(), Timestamp(20));

        assert_eq!(games.len(), 2);
        assert_eq!(games[0].id, AppId(2));
        assert_eq!(games[0].name, "#2");
        assert!(games[0].vote.bet);
        assert_eq!(games[0].price, None);
        assert_eq!(games[1].name, "Subnautica");
        assert_eq!(games[1].vote.kind, VoteKind::Maybe);
        catalog_game.price.discount = None;
        assert_eq!(games[1].price, Some(catalog_game.price));
        assert_eq!(games[1].reviews, Some(catalog_game.reviews));
        assert_eq!(games[1].store_url, "https://store.steampowered.com/app/1/");
    }

    #[test]
    fn a_voted_game_out_of_the_catalog_keeps_its_name() {
        let mut votes = Votes::new();
        cast(
            &mut votes,
            AppId(72850),
            VoteKind::Known,
            "2026-10-01".to_string(),
            false,
        );
        let off_catalog = OffCatalogGames::from([(
            AppId(72850),
            OffCatalogGame {
                tags: Vec::new(),
                name: Some("Skyrim".to_string()),
            },
        )]);

        let games = voted_games(&votes, &catalog(Vec::new()), &off_catalog, Timestamp(0));

        assert_eq!(games[0].name, "Skyrim");
        assert_eq!(games[0].price, None);
    }

    fn catalog(games: Vec<Game>) -> Catalog {
        Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
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
