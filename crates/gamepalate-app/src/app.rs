use std::{
    cmp::Reverse,
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use gamepalate_core::{
    best_match::best_matches,
    catalog::Catalog,
    corrections::Corrections,
    currency::Currency,
    families::Families,
    filters::{FilterOptions, TagNameError},
    game::{Price, Reviews},
    game_vector::{GameVector, game_vectors},
    ids::AppId,
    off_catalog::OffCatalogGames,
    region::Region,
    search::{
        Recommendation, RecommendedGame, SearchError, SearchInputs, SearchOptions, recommend,
    },
    taste::game_name,
    timestamp::Timestamp,
    votes::{Vote, VoteKind, Votes, cast, cast_in_last_day},
};
use gamepalate_steam::{
    archive::PendingUpdate,
    description::Block,
    images::{ImageDownloader, ImageError, header_url},
    transport::{SteamError, Transport},
};
use gamepalate_store::{
    data_files::{DataFiles, DataFilesError, load_data_files},
    families_edit::restore_families,
    gui::{GuiSettings, load_gui_settings, save_gui_settings},
    votes::{VotesError, save_votes},
};

use crate::{
    capsules::{Capsule, CapsuleFinder, capsule},
    catalog_search::{CatalogMatch, NameIndex},
    data_dir,
    details::{GameDetails, StoreDetails},
    families::{
        CreatedFamilies, FamiliesChanged, FamiliesRemade, FamiliesUndo, FamilyChange,
        FamilyChangeError, FamilyGame, RemadeFamilies, RemakePreview, automatic_family_names,
        changed_families, families_on_open, family_games, new_played_games, open_error,
        remake_automatic_families, remake_preview, renamed_choice,
    },
    images::{DecodedImage, ImageCache, ImageKind, LoadImageError, MAX_CACHE_BYTES, cache_dir},
    library::{
        GameCorrection, GameCorrectionError, LibraryView, PlayedElsewhereError,
        changed_played_elsewhere, corrected_game, game_correction, library_ids, library_view,
    },
    log::failed,
    profile::{ProfileView, profile_view},
    settings::{self, Settings},
    steam_client::steam_client_file_of,
    tags::{GameTagsView, game_tags_view},
    update,
};

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
        let mut files =
            load_data_files(&dir).inspect_err(failed("could not read the data files"))?;
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
        let now = jiff::Zoned::now();
        let date = now.date().to_string();
        let at = Timestamp(now.timestamp().as_millisecond());
        let name = |id| {
            let name = game_name(id, &self.files.catalog, &self.files.off_catalog);
            (name != format!("#{}", id.0)).then_some(name)
        };
        *votes = saved_vote(
            &self.data_dir.join("votes.toml"),
            &votes,
            id,
            Vote {
                kind,
                date,
                at: Some(at),
                bet,
            },
            name,
        )
        .inspect_err(failed("could not save the vote"))?;
        Ok(())
    }

    pub fn votes_in_last_day(&self) -> usize {
        cast_in_last_day(
            &self.lock_votes(),
            Timestamp(jiff::Timestamp::now().as_millisecond()),
        )
    }

    pub fn voted_games(&self) -> Vec<VotedGame> {
        voted_games(
            &self.lock_votes(),
            &self.files.catalog,
            &self.files.off_catalog,
            Timestamp(jiff::Timestamp::now().as_millisecond()),
        )
    }

    pub fn best_matches(&self) -> HashMap<AppId, f64> {
        self.with_inputs(best_matches)
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

    pub async fn image(&self, url: &str, kind: ImageKind) -> Result<Vec<u8>, LoadImageError> {
        self.images.image(url, kind).await
    }

    pub async fn image_without_cache(&self, url: &str) -> Result<Vec<u8>, LoadImageError> {
        self.images.download(url).await
    }

    pub async fn extra_image(&self, url: &str) -> Result<DecodedImage, LoadImageError> {
        self.images.extra(url).await
    }

    pub async fn capsule(&self, id: AppId) -> Result<Capsule, LoadImageError> {
        capsule(id, &self.images, &self.capsules).await
    }

    pub async fn details(&self, id: AppId) -> Result<Option<GameDetails>, SteamError> {
        self.store
            .details(id)
            .await
            .inspect_err(failed("could not read the store details"))
    }

    pub async fn description(&self, id: AppId) -> Result<Option<Vec<Block>>, SteamError> {
        self.store
            .description(id)
            .await
            .inspect_err(failed("could not read the store description"))
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
            .inspect_err(failed("could not save gui.toml"))
    }

    pub fn profile_view(&self) -> ProfileView {
        self.with_inputs(profile_view)
    }

    pub fn game_tags(&self, id: AppId) -> Option<GameTagsView> {
        self.with_inputs(|inputs| game_tags_view(id, inputs))
    }

    pub fn library_view(&self) -> LibraryView {
        self.with_inputs(library_view)
    }

    pub fn correct_game(
        &self,
        id: AppId,
        correction: GameCorrection,
    ) -> Result<GameCorrection, GameCorrectionError> {
        let votes = self.lock_votes();
        let mut corrections = self.lock_corrections();
        let families = self.lock_families();
        let before = game_correction(id, &corrections);
        *corrections = corrected_game(
            &self.data_dir.join("corrections.toml"),
            id,
            correction,
            &SearchInputs {
                votes: &votes,
                corrections: &corrections,
                families: &families,
                ..self.files.search_inputs(&self.vectors)
            },
        )
        .inspect_err(failed("could not save the correction"))?;
        Ok(before)
    }

    pub fn set_played_elsewhere(
        &self,
        id: AppId,
        played: bool,
    ) -> Result<bool, PlayedElsewhereError> {
        let mut corrections = self.lock_corrections();
        let before = corrections.played_elsewhere.contains(&id);
        let name = game_name(id, &self.files.catalog, &self.files.off_catalog);
        *corrections = changed_played_elsewhere(
            &self.data_dir.join("corrections.toml"),
            id,
            played,
            &corrections,
            (name != format!("#{}", id.0)).then_some(name.as_str()),
        )
        .inspect_err(failed("could not save the game played elsewhere"))?;
        Ok(before)
    }

    pub fn search_catalog(&self, text: &str) -> Vec<CatalogMatch> {
        let in_library = self.with_inputs(library_ids);
        self.names
            .search(text, &in_library)
            .into_iter()
            .filter_map(|id| self.files.catalog.game(id))
            .map(|game| CatalogMatch {
                id: game.id,
                name: game.name.clone(),
            })
            .collect()
    }

    pub fn change_families(
        &self,
        change: &FamilyChange,
    ) -> Result<FamiliesChanged, FamilyChangeError> {
        let mut families = self.lock_families();
        let changed = changed_families(
            &self.data_dir.join(FAMILIES_FILE),
            &families,
            change,
            |id| {
                let name = game_name(id, &self.files.catalog, &self.files.off_catalog);
                (name != format!("#{}", id.0)).then_some(name)
            },
        )
        .inspect_err(failed("could not save the families"))?;
        *families = changed.families;

        Ok(FamiliesChanged {
            undo: FamiliesUndo {
                snapshot: changed.snapshot,
                discover_family: self.follow_renamed(&changed.renamed),
                families_played: None,
            },
            gone: changed
                .renamed
                .into_iter()
                .filter(|(_, to)| to.is_none())
                .map(|(from, _)| from)
                .collect(),
        })
    }

    pub fn remake_preview(&self) -> RemakePreview {
        self.with_inputs(remake_preview)
    }

    // Only the automatic families change; the undo puts back the file as it was.
    pub fn remake_families(&self) -> Result<FamiliesRemade, FamilyChangeError> {
        let votes = self.lock_votes();
        let corrections = self.lock_corrections();
        let mut families = self.lock_families();
        let outcome = remake_automatic_families(
            &self.data_dir.join(FAMILIES_FILE),
            &SearchInputs {
                votes: &votes,
                corrections: &corrections,
                families: &families,
                ..self.files.search_inputs(&self.vectors)
            },
            &jiff::Zoned::now().date().to_string(),
        )
        .map_err(open_error)
        .inspect_err(failed("could not remake the families"))?;

        let RemadeFamilies::Remade(remade) = outcome else {
            return Ok(FamiliesRemade {
                outcome,
                undo: None,
            });
        };
        *families = remade.families.clone();
        let undo = FamiliesUndo {
            snapshot: remade.snapshot.clone(),
            discover_family: self.follow_renamed(&remade.renamed()),
            families_played: self.remember_played(&remade.played),
        };
        Ok(FamiliesRemade {
            outcome: RemadeFamilies::Remade(remade),
            undo: Some(undo),
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

    pub fn undo_families(&self, undo: &FamiliesUndo) -> Result<(), FamilyChangeError> {
        let mut families = self.lock_families();
        *families = restore_families(&self.data_dir.join(FAMILIES_FILE), &undo.snapshot)
            .map_err(FamilyChangeError::Save)
            .inspect_err(failed("could not undo the families"))?;
        if let Some(family) = &undo.discover_family {
            let _ = self.change_gui_settings(|settings| settings.search.family = family.clone());
        }
        if let Some(played) = &undo.families_played {
            let _ = self.change_gui_settings(|settings| settings.families_played = played.clone());
        }
        Ok(())
    }

    // The families made from the played games when this session opened the data, if any.
    pub fn created_families(&self) -> Option<&CreatedFamilies> {
        self.created_families.as_ref()
    }

    // How many played games are new since the families were made, once there are enough.
    pub fn new_played_games(&self) -> Option<usize> {
        let seen = self.gui_settings().families_played?;
        self.with_inputs(|inputs| new_played_games(inputs, Some(&seen)))
    }

    pub fn family_games(&self, family: &str) -> Vec<FamilyGame> {
        family_games(&self.lock_families(), family, |id| {
            game_name(id, &self.files.catalog, &self.files.off_catalog)
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn family_names(&self) -> Vec<String> {
        self.lock_families()
            .families
            .iter()
            .map(|family| family.name.clone())
            .collect()
    }

    pub fn automatic_family_names(&self) -> Vec<String> {
        automatic_family_names(&self.lock_families())
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

    pub fn image_cache_bytes(&self) -> u64 {
        self.images.bytes()
    }

    pub fn clear_image_cache(&self) {
        self.images.clear();
    }

    pub fn settings(&self) -> Settings {
        Settings {
            steam_id: self.files.config.steam_id,
            key: settings::key_source(),
            region: self.files.config.region,
            data_dir: self.data_dir.clone(),
            image_cache_max_bytes: MAX_CACHE_BYTES,
            steam_client_hours: self.files.config.signals.steam_client_hours,
            steam_client_file: steam_client_file_of(&self.data_dir),
            catalog_games: self.files.catalog.games().len(),
            tags_updated_at: self.files.catalog.freshness.tags_updated_at,
            config: self.files.config,
        }
    }

    pub fn header_url(&self, id: AppId) -> String {
        header_url(id)
    }

    pub fn bet_every(&self) -> usize {
        self.files.config.results.bet_every
    }

    pub fn config_filters(&self) -> FilterOptions {
        let config = &self.files.config;
        FilterOptions {
            min_reviews: Some(config.relevance.min_reviews),
            min_positive_percent: Some(config.results.min_positive_percent),
            without_early_access: Some(!config.results.early_access),
            ..FilterOptions::default()
        }
    }

    pub fn signals_read_at(&self) -> Timestamp {
        self.files.signals.read_at
    }

    pub fn prices_updated_at(&self) -> Timestamp {
        self.files.catalog.freshness.prices_updated_at
    }

    pub fn pending_update(&self) -> Option<PendingUpdate> {
        update::pending_update(&self.data_dir, &self.files.config)
    }

    pub fn prices_region(&self) -> Option<Region> {
        let region = self.files.catalog.freshness.region;
        (region != self.files.config.region).then_some(region)
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
    vote: Vote,
    name: impl Fn(AppId) -> Option<String>,
) -> Result<Votes, VoteError> {
    let mut changed = votes.clone();
    cast(&mut changed, id, vote.kind, vote.date, vote.bet, vote.at);
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
            Vote {
                kind: VoteKind::WouldTry,
                date: "2026-10-30".to_string(),
                at: Some(Timestamp(1_762_500_000_000)),
                bet: true,
            },
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
            "620 = { vote = \"would_try\", date = \"2026-10-30\", at = 1762500000000, bet = true } # Portal 2"
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
            None,
        );
        cast(
            &mut votes,
            AppId(2),
            VoteKind::WouldTry,
            "2026-10-30".to_string(),
            true,
            None,
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
            developers: Vec::new(),
            franchises: Vec::new(),
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
            None,
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
            similar_to_id: None,
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
