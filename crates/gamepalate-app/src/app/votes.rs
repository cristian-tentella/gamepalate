use std::{
    cmp::Reverse,
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use gamepalate_core::{
    best_match::best_matches,
    catalog::Catalog,
    filters::{FilterContext, FilterOptions, chosen_filters, first_failed, owned_games},
    game::{Price, Reviews},
    ids::AppId,
    off_catalog::OffCatalogGames,
    search::SearchInputs,
    taste::game_name,
    timestamp::Timestamp,
    votes::{Vote, VoteKind, Votes, cast, cast_in_last_day},
};
use gamepalate_store::votes::{VotesError, remove_vote, save_votes};

use super::{Gamepalate, RecommendError};
use crate::log::failed;

#[derive(Debug, Clone, PartialEq)]
pub struct VotedGame {
    pub id: AppId,
    pub name: String,
    pub vote: Vote,
    pub price: Option<Price>,
    pub reviews: Option<Reviews>,
    pub store_url: String,
    /// In the library, borrowed, played recently or played elsewhere.
    pub owned: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("could not save the vote in {}", path.display())]
pub struct VoteError {
    path: PathBuf,
    #[source]
    source: VotesError,
}

fn saved_vote(
    path: &Path,
    votes: &Votes,
    id: AppId,
    vote: Vote,
    name: impl Fn(AppId) -> Option<String>,
) -> Result<Votes, VoteError> {
    let mut changed = votes.clone();
    cast(&mut changed, id, vote);
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
    owned: &BTreeSet<AppId>,
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
                owned: owned.contains(&id),
            }
        })
        .collect();
    games.sort_by_cached_key(|game| (Reverse(game.vote.date.clone()), game.name.to_lowercase()));
    games
}

/// The voted games the chosen filters leave in. Without filters, all of them; with one, only
/// the games of the Catalog, since the others have no price, reviews or tags to judge.
fn votes_passing(
    inputs: &SearchInputs,
    options: &FilterOptions,
    now: Timestamp,
) -> Result<BTreeSet<AppId>, RecommendError> {
    let filters =
        chosen_filters(options, inputs.catalog).map_err(|error| RecommendError(error.into()))?;
    if filters.is_empty() {
        return Ok(inputs.votes.keys().copied().collect());
    }
    let context = FilterContext::new(
        inputs.signals,
        inputs.families,
        inputs.votes,
        inputs.corrections,
    );
    Ok(inputs
        .votes
        .keys()
        .copied()
        .filter(|&id| {
            inputs
                .catalog
                .game(id)
                .is_some_and(|game| first_failed(game, &filters, &context, now).is_none())
        })
        .collect())
}

impl Gamepalate {
    /// `searched` marks a vote given to a game looked up by name rather than recommended.
    pub fn vote(
        &self,
        id: AppId,
        kind: VoteKind,
        genre_ok: bool,
        bet: bool,
        searched: bool,
    ) -> Result<(), VoteError> {
        let mut votes = self.write_votes();
        let now = jiff::Zoned::now();
        let date = now.date().to_string();
        let at = Timestamp(now.timestamp().as_millisecond());
        *votes = saved_vote(
            &self.data_dir.join("votes.toml"),
            &votes,
            id,
            Vote {
                kind,
                date,
                at: Some(at),
                bet,
                genre_ok,
                searched,
            },
            |id| self.saved_name(id),
        )
        .inspect_err(failed("could not save the vote"))?;
        Ok(())
    }

    /// Takes the vote off the game, which goes back to having none. Without a vote, nothing changes.
    pub fn remove_vote(&self, id: AppId) -> Result<(), VoteError> {
        let mut votes = self.write_votes();
        let path = self.data_dir.join("votes.toml");
        *votes = remove_vote(&path, &votes, id, |id| self.saved_name(id))
            .map_err(|source| VoteError { path, source })
            .inspect_err(failed("could not remove the vote"))?;
        Ok(())
    }

    fn saved_name(&self, id: AppId) -> Option<String> {
        let name = game_name(id, &self.files.catalog, &self.files.off_catalog);
        (name != format!("#{}", id.0)).then_some(name)
    }

    pub fn votes_in_last_day(&self) -> usize {
        cast_in_last_day(
            &self.lock_votes(),
            Timestamp(jiff::Timestamp::now().as_millisecond()),
        )
    }

    pub fn voted_games(&self) -> Vec<VotedGame> {
        self.with_inputs(|inputs| {
            voted_games(
                inputs.votes,
                inputs.catalog,
                inputs.off_catalog,
                &owned_games(inputs.signals, inputs.corrections),
                Timestamp(jiff::Timestamp::now().as_millisecond()),
            )
        })
    }

    /// The ids of the voted games that pass the filters in `options`, and nothing else: a
    /// missing option is no limit.
    pub fn votes_passing(
        &self,
        options: &FilterOptions,
    ) -> Result<BTreeSet<AppId>, RecommendError> {
        self.with_inputs(|inputs| {
            votes_passing(
                inputs,
                options,
                Timestamp(jiff::Timestamp::now().as_millisecond()),
            )
        })
        .inspect_err(failed("could not filter the votes"))
    }

    pub fn best_matches(&self) -> HashMap<AppId, f64> {
        self.with_inputs(best_matches)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gamepalate_core::{
        catalog::Freshness,
        config::Config,
        corrections::Corrections,
        currency::Currency,
        families::Families,
        game::{Discount, Game},
        off_catalog::OffCatalogGame,
        region::Region,
        signals::{LibraryGame, RecentGame, Signals},
    };

    use super::*;

    fn vote_in(path: &Path, votes: &Votes) -> Result<Votes, VoteError> {
        saved_vote(
            path,
            votes,
            AppId(620),
            Vote {
                at: Some(Timestamp(1_762_500_000_000)),
                bet: true,
                ..Vote::new(VoteKind::WouldTry, "2026-10-30")
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
        assert_eq!(
            gamepalate_store::votes::load_votes(&path).unwrap().votes,
            votes
        );
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
            Vote::new(VoteKind::Later, "2026-10-01"),
        );
        cast(
            &mut votes,
            AppId(2),
            Vote {
                bet: true,
                ..Vote::new(VoteKind::WouldTry, "2026-10-30")
            },
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

        let games = voted_games(
            &votes,
            &catalog,
            &OffCatalogGames::new(),
            &BTreeSet::new(),
            Timestamp(20),
        );

        assert_eq!(games.len(), 2);
        assert_eq!(games[0].id, AppId(2));
        assert_eq!(games[0].name, "#2");
        assert!(games[0].vote.bet);
        assert_eq!(games[0].price, None);
        assert_eq!(games[1].name, "Subnautica");
        assert_eq!(games[1].vote.kind, VoteKind::Later);
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
            Vote::new(VoteKind::NotForMe, "2026-10-01"),
        );
        let off_catalog = OffCatalogGames::from([(
            AppId(72850),
            OffCatalogGame {
                tags: Vec::new(),
                name: Some("Skyrim".to_string()),
            },
        )]);

        let games = voted_games(
            &votes,
            &catalog(Vec::new()),
            &off_catalog,
            &BTreeSet::new(),
            Timestamp(0),
        );

        assert_eq!(games[0].name, "Skyrim");
        assert_eq!(games[0].price, None);
    }

    fn game(id: u32, original_cents: u32, discount: Option<Discount>) -> Game {
        Game {
            id: AppId(id),
            name: format!("game {id}"),
            price: Price {
                original_cents,
                discount,
            },
            reviews: Reviews {
                total: 100,
                positive: 90,
            },
            is_early_access: false,
            tags: Vec::new(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn voted(ids: &[u32]) -> Votes {
        let mut votes = Votes::new();
        for &id in ids {
            cast(
                &mut votes,
                AppId(id),
                Vote::new(VoteKind::WouldTry, "2026-10-01"),
            );
        }
        votes
    }

    fn owned_signals() -> Signals {
        let library = |id| LibraryGame {
            id: AppId(id),
            total_hours: 3.0,
            last_played: None,
        };
        Signals {
            read_at: Timestamp(0),
            library: vec![library(1)],
            borrowed: vec![library(2)],
            recent: vec![RecentGame {
                id: AppId(3),
                total_hours: 1.0,
                recent_hours: 1.0,
            }],
            wishlist: Vec::new(),
            followed: Vec::new(),
        }
    }

    #[test]
    fn a_voted_game_is_owned_when_the_library_lends_or_played_it() {
        let votes = voted(&[1, 2, 3, 4, 5]);
        let corrections = Corrections {
            played_elsewhere: BTreeSet::from([AppId(4)]),
            ..Corrections::default()
        };
        let owned = owned_games(&owned_signals(), &corrections);

        let games = voted_games(
            &votes,
            &catalog(Vec::new()),
            &OffCatalogGames::new(),
            &owned,
            Timestamp(0),
        );

        let owned_ids: Vec<u32> = games
            .iter()
            .filter(|game| game.owned)
            .map(|game| game.id.0)
            .collect();
        assert_eq!(owned_ids.len(), 4);
        assert!(!games.iter().find(|game| game.id == AppId(5)).unwrap().owned);
    }

    fn passing(
        votes: &Votes,
        catalog: &Catalog,
        options: &FilterOptions,
    ) -> Result<BTreeSet<AppId>, RecommendError> {
        let signals = owned_signals();
        let corrections = Corrections::default();
        let families = Families::default();
        let off_catalog = OffCatalogGames::new();
        let config = Config::default();
        let inputs = SearchInputs {
            catalog,
            game_vectors: &[],
            signals: &signals,
            votes,
            corrections: &corrections,
            families: &families,
            off_catalog: &off_catalog,
            config: &config,
        };
        votes_passing(&inputs, options, Timestamp(20))
    }

    #[test]
    fn without_filters_every_vote_passes_even_off_the_catalog() {
        let votes = voted(&[1, 2, 99]);
        let catalog = catalog(vec![game(1, 1000, None), game(2, 3000, None)]);

        let ids = passing(&votes, &catalog, &FilterOptions::default()).unwrap();

        assert_eq!(ids, BTreeSet::from([AppId(1), AppId(2), AppId(99)]));
    }

    #[test]
    fn a_price_limit_keeps_the_cheap_games_and_drops_the_off_catalog_ones() {
        let votes = voted(&[1, 2, 99]);
        let catalog = catalog(vec![game(1, 1000, None), game(2, 3000, None)]);
        let options = FilterOptions {
            max_price_cents: Some(2000),
            ..FilterOptions::default()
        };

        let ids = passing(&votes, &catalog, &options).unwrap();

        assert_eq!(ids, BTreeSet::from([AppId(1)]));
    }

    #[test]
    fn an_ended_discount_is_not_a_discount() {
        let discount = |end| Discount {
            percent: 50,
            final_cents: 500,
            end: Some(Timestamp(end)),
        };
        let votes = voted(&[1, 2]);
        let catalog = catalog(vec![
            game(1, 1000, Some(discount(100))),
            game(2, 1000, Some(discount(10))),
        ]);
        let options = FilterOptions {
            only_discounted: true,
            ..FilterOptions::default()
        };

        let ids = passing(&votes, &catalog, &options).unwrap();

        assert_eq!(ids, BTreeSet::from([AppId(1)]));
    }

    #[test]
    fn an_unknown_tag_names_itself() {
        let options = FilterOptions {
            tags: vec!["Nope".to_string()],
            ..FilterOptions::default()
        };

        let error = passing(&voted(&[1]), &catalog(Vec::new()), &options).unwrap_err();

        assert_eq!(error.to_string(), "no tag named \"Nope\"");
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
}
