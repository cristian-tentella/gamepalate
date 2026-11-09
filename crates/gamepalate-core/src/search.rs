use std::{cmp::Reverse, collections::HashMap};

use crate::{
    catalog::{Catalog, Freshness},
    config::Config,
    corrections::Corrections,
    creators::{LikedCreators, liked_creators},
    effective_hours::hours_by_source,
    explanation::{similar_to, top_tags},
    families::{Families, Family},
    filters::{Blame, FilterContext, FilterOptions, TagNameError, active_filters, apply_filters},
    game::{Price, Reviews},
    game_vector::GameVector,
    ids::AppId,
    off_catalog::OffCatalogGames,
    opinion::{opinion_model, opinions},
    quality::quality,
    ranking::{RankedGame, bets, ranking, with_votes},
    signals::Signals,
    taste::{Taste, family_taste, game_name, signals_taste},
    timestamp::Timestamp,
    turns::{ListKind, Pick, in_turns, turn_shares, with_bets},
    votes::Votes,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum SortBy {
    #[default]
    Relevance,
    Discount,
    Price,
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct SearchOptions {
    pub family: Option<String>,
    pub signals_only: bool,
    pub only_bets: bool,
    pub sort: SortBy,
    pub limit: Option<usize>,
    pub filters: FilterOptions,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchInputs<'a> {
    pub catalog: &'a Catalog,
    pub game_vectors: &'a [GameVector],
    pub signals: &'a Signals,
    pub votes: &'a Votes,
    pub corrections: &'a Corrections,
    pub families: &'a Families,
    pub off_catalog: &'a OffCatalogGames,
    pub config: &'a Config,
}

impl SearchInputs<'_> {
    pub fn liked_creators(&self) -> LikedCreators {
        liked_creators(
            self.signals,
            &self.corrections.played_elsewhere,
            &self.config.signals,
            self.catalog,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchError {
    FamilyNotFound { name: String, families: Vec<String> },
    TagName(TagNameError),
    EmptySignalsProfile,
}

impl From<TagNameError> for SearchError {
    fn from(error: TagNameError) -> SearchError {
        SearchError::TagName(error)
    }
}

#[derive(Debug, Clone)]
pub struct SearchTaste {
    pub family: Option<String>,
    pub kind: ListKind,
    pub taste: Taste,
}

#[derive(Debug, Clone)]
pub struct RecommendedGame {
    pub position: usize,
    pub id: AppId,
    pub name: String,
    pub price: Price,
    pub store_url: String,
    pub from_family: Option<String>,
    pub affinity: f64,
    pub top_tags: Vec<String>,
    pub similar_to: Option<String>,
    pub similar_to_id: Option<AppId>,
    pub similar_to_multiplier: Option<f64>,
    pub quality: f64,
    pub reviews: Reviews,
    pub in_wishlist: bool,
    pub relevance: f64,
    pub bet: bool,
}

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub games: Vec<RecommendedGame>,
    pub family: Option<String>,
    pub saved_signals_from: Timestamp,
    pub freshness: Freshness,
    pub stale_prices: bool,
    pub empty_because: Option<Blame>,
    pub without_vote_model: bool,
}

pub fn find_family<'f>(families: &'f Families, name: &str) -> Result<&'f Family, SearchError> {
    let lowercase_name = name.to_lowercase();

    match families
        .families
        .iter()
        .find(|family| family.name == name)
        .or_else(|| {
            families
                .families
                .iter()
                .find(|family| family.name.to_lowercase() == lowercase_name)
        }) {
        Some(family) => Ok(family),
        None => Err(SearchError::FamilyNotFound {
            name: name.into(),
            families: families
                .families
                .iter()
                .map(|family| family.name.clone())
                .collect(),
        }),
    }
}

pub fn search_tastes(
    inputs: &SearchInputs,
    family: Option<&str>,
) -> Result<Vec<SearchTaste>, SearchError> {
    let taste_of = |family: &Family| SearchTaste {
        family: Some(family.name.clone()),
        kind: ListKind::of_family(family),
        taste: family_taste(
            family,
            inputs.catalog,
            inputs.off_catalog,
            &inputs.corrections.tag_multipliers,
        ),
    };

    match family {
        None => {
            let hours = hours_by_source(
                inputs.signals,
                inputs.votes,
                &inputs.corrections.played_elsewhere,
                &inputs.config.signals,
            );
            let taste = signals_taste(
                &hours,
                inputs.catalog,
                inputs.off_catalog,
                inputs.corrections,
                &inputs.config.signals,
            );

            if !taste.has_profile() {
                return Err(SearchError::EmptySignalsProfile);
            }

            let mut search_tastes = vec![SearchTaste {
                family: None,
                kind: ListKind::Signals,
                taste,
            }];

            // A family without a profile would leave an empty turn
            search_tastes.extend(
                inputs
                    .families
                    .families
                    .iter()
                    .map(taste_of)
                    .filter(|search_taste| search_taste.taste.has_profile()),
            );

            Ok(search_tastes)
        }
        Some(family_name) => Ok(vec![taste_of(find_family(inputs.families, family_name)?)]),
    }
}

pub fn needs_opinion(config: &Config, options: &SearchOptions) -> bool {
    options.only_bets || config.results.bet_every > 0 || config.relevance.votes_weight > 0.0
}

pub fn sort_picks(picks: &mut [Pick], sort: SortBy, catalog: &Catalog, now: Timestamp) {
    let price = |pick: &Pick| catalog.game(pick.game.id).map(|game| game.price.as_of(now));

    match sort {
        SortBy::Relevance => {}
        SortBy::Discount => picks.sort_by_key(|pick| {
            Reverse(
                price(pick)
                    .and_then(|price| price.discount)
                    .map_or(0, |discount| discount.percent),
            )
        }),
        SortBy::Price => {
            picks.sort_by_key(|pick| price(pick).map_or(u32::MAX, |price| price.final_cents()))
        }
    }
}

pub fn recommended_game(
    pick: &Pick,
    position: usize,
    tastes: &[SearchTaste],
    inputs: &SearchInputs,
    now: Timestamp,
) -> RecommendedGame {
    let id = pick.game.id;
    let game = inputs
        .catalog
        .game(id)
        .expect("ranked games are in the catalog");
    let taste = &tastes[pick.from];
    let similar = similar_to(&pick.game.references);

    RecommendedGame {
        position,
        id,
        name: game.name.clone(),
        price: game.price.as_of(now),
        store_url: format!("https://store.steampowered.com/app/{}/", id.0),
        from_family: taste.family.clone(),
        affinity: pick.game.affinity,
        top_tags: top_tags(game, &taste.taste.profile(), inputs.catalog),
        similar_to: similar
            .map(|reference| game_name(reference.id, inputs.catalog, inputs.off_catalog)),
        similar_to_id: similar.map(|reference| reference.id),
        // Only signals use game multipliers
        similar_to_multiplier: similar
            .filter(|_| taste.family.is_none())
            .and_then(|reference| inputs.corrections.game_multipliers.get(&reference.id))
            .copied(),
        quality: quality(&game.reviews),
        reviews: game.reviews,
        in_wishlist: inputs.signals.wishlist.contains(&id),
        relevance: pick.game.relevance,
        bet: pick.bet,
    }
}

fn merged_lists<'a>(
    voted: &'a [Vec<RankedGame>],
    bet_lists: &'a [Vec<RankedGame>],
    shares: &[u32],
    only_bets: bool,
    bet_every: usize,
) -> Vec<Pick<'a>> {
    if only_bets {
        in_turns(bet_lists, shares)
            .into_iter()
            .map(|pick| Pick { bet: true, ..pick })
            .collect()
    } else if bet_every > 0 {
        with_bets(
            in_turns(voted, shares),
            in_turns(bet_lists, shares),
            bet_every,
        )
    } else {
        in_turns(voted, shares)
    }
}

pub fn recommend(
    inputs: &SearchInputs,
    options: &SearchOptions,
    now: Timestamp,
) -> Result<Recommendation, SearchError> {
    let filters = active_filters(&options.filters, inputs.config, inputs.catalog)?;
    let mut tastes = search_tastes(inputs, options.family.as_deref())?;
    if options.signals_only && options.family.is_none() {
        tastes.truncate(1);
    }
    let filter_context = FilterContext::new(
        inputs.signals,
        inputs.families,
        inputs.votes,
        inputs.corrections,
    );
    let filter_outcome = apply_filters(inputs.catalog, &filters, &filter_context, now);

    let rankings: Vec<Vec<RankedGame>> = tastes
        .iter()
        .map(|search_taste| {
            let mut ranked = ranking(
                inputs.game_vectors,
                &search_taste.taste,
                &inputs.config.relevance,
            )
            .unwrap_or_default();
            ranked.retain(|game| filter_outcome.candidates.contains(&game.id));

            ranked
        })
        .collect();

    let opinion_needed = needs_opinion(inputs.config, options);
    let model = if opinion_needed {
        opinion_model(inputs.votes, inputs.catalog, &inputs.liked_creators())
    } else {
        None
    };
    let opinion_by_id: Option<HashMap<AppId, f64>> = model.map(|model| {
        inputs
            .catalog
            .games()
            .iter()
            .map(|game| game.id)
            .zip(opinions(&model, inputs.catalog))
            .collect()
    });

    let votes_weight = inputs.config.relevance.votes_weight;
    let (voted, bet_lists): (Vec<Vec<RankedGame>>, Vec<Vec<RankedGame>>) = match &opinion_by_id {
        Some(opinion_by_id) => rankings
            .iter()
            .map(|ranked| {
                (
                    with_votes(ranked, opinion_by_id, votes_weight),
                    bets(ranked, opinion_by_id, inputs.votes, votes_weight),
                )
            })
            .unzip(),
        None => (rankings, Vec::new()),
    };

    let bet_every = if opinion_by_id.is_some() {
        inputs.config.results.bet_every
    } else {
        0
    };
    let kinds: Vec<ListKind> = tastes
        .iter()
        .map(|search_taste| search_taste.kind)
        .collect();
    let mut picks = merged_lists(
        &voted,
        &bet_lists,
        &turn_shares(&kinds),
        options.only_bets,
        bet_every,
    );

    sort_picks(&mut picks, options.sort, inputs.catalog, now);
    picks.truncate(options.limit.unwrap_or(inputs.config.results.count));

    let games: Vec<RecommendedGame> = picks
        .iter()
        .enumerate()
        .map(|(i, pick)| recommended_game(pick, i + 1, &tastes, inputs, now))
        .collect();

    Ok(Recommendation {
        empty_because: if games.is_empty() {
            filter_outcome.blame
        } else {
            None
        },
        games,
        family: options.family.clone(),
        saved_signals_from: inputs.signals.read_at,
        freshness: inputs.catalog.freshness,
        stale_prices: inputs.catalog.freshness.prices_are_stale(
            now,
            inputs.config.prices_max_age_hours,
            inputs.config.region,
        ),
        without_vote_model: opinion_needed && opinion_by_id.is_none(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        currency::Currency,
        families::Family,
        game::{Game, GameTag},
        game_vector::game_vectors,
        ids::TagId,
        region::Region,
        signals::LibraryGame,
    };

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    fn game(id: u32, tag: u32) -> Game {
        Game {
            id: AppId(id),
            name: format!("game {id}"),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: Reviews {
                total: 5000,
                positive: 4500,
            },
            is_early_access: false,
            tags: vec![GameTag {
                id: TagId(tag),
                weight: 1.0,
            }],
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn recommendation_with(
        region: Region,
        options: &SearchOptions,
        automatic: Option<&str>,
    ) -> Recommendation {
        let catalog = Catalog::new(
            vec![
                game(1, 1),
                game(2, 1),
                game(3, 2),
                game(4, 2),
                game(5, 1),
                game(6, 2),
            ],
            BTreeMap::new(),
            BTreeMap::from([(TagId(1), 1.0), (TagId(2), 1.0)]),
            Freshness {
                prices_updated_at: NOW,
                tags_updated_at: NOW,
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let vectors = game_vectors(&catalog);
        let signals = Signals {
            read_at: NOW,
            library: vec![LibraryGame {
                id: AppId(1),
                total_hours: 20.0,
                last_played: None,
            }],
            borrowed: vec![],
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        };
        let config = Config {
            region,
            ..Config::default()
        };
        let inputs = SearchInputs {
            catalog: &catalog,
            game_vectors: &vectors,
            signals: &signals,
            votes: &Votes::new(),
            corrections: &Corrections::default(),
            families: &Families {
                families: vec![Family {
                    name: "Puzzle".to_string(),
                    games: vec![AppId(3)],
                    automatic: automatic.map(str::to_string),
                }],
                held_out: Vec::new(),
            },
            off_catalog: &OffCatalogGames::new(),
            config: &config,
        };

        recommend(&inputs, options, NOW).unwrap()
    }

    fn recommendation(region: Region, options: &SearchOptions) -> Recommendation {
        recommendation_with(region, options, None)
    }

    fn stale_prices(region: Region) -> bool {
        recommendation(region, &SearchOptions::default()).stale_prices
    }

    fn families_in(recommendation: Recommendation) -> Vec<Option<String>> {
        recommendation
            .games
            .into_iter()
            .map(|game| game.from_family)
            .collect()
    }

    fn families_of(options: &SearchOptions) -> Vec<Option<String>> {
        families_in(recommendation(Region::ITALY, options))
    }

    fn ranked(id: u32) -> RankedGame {
        RankedGame {
            id: AppId(id),
            relevance: 0.0,
            references: Vec::new(),
            affinity: 0.0,
        }
    }

    #[test]
    fn all_tastes_mix_the_signals_and_the_families() {
        let puzzle = || Some("Puzzle".to_string());

        assert_eq!(
            families_of(&SearchOptions::default()),
            [None, puzzle(), None, puzzle()]
        );
    }

    #[test]
    fn an_automatic_family_gets_a_smaller_share() {
        let recommendation =
            recommendation_with(Region::ITALY, &SearchOptions::default(), Some("2026-11-10"));

        assert_eq!(
            families_in(recommendation),
            [None, None, Some("Puzzle".to_string()), None]
        );
    }

    #[test]
    fn the_bets_take_turns_with_the_same_shares_as_the_list() {
        let shares = turn_shares(&[ListKind::Signals, ListKind::AutomaticFamily]);
        let voted = [
            (1..=10).map(ranked).collect(),
            (11..=20).map(ranked).collect(),
        ];
        let bet_lists = [
            (21..=30).map(ranked).collect(),
            (31..=40).map(ranked).collect(),
        ];
        let bets_from = |picks: Vec<Pick>| -> Vec<usize> {
            picks
                .iter()
                .filter(|pick| pick.bet)
                .map(|pick| pick.from)
                .take(5)
                .collect()
        };

        let only_bets = merged_lists(&voted, &bet_lists, &shares, true, 0);
        let mixed = merged_lists(&voted, &bet_lists, &shares, false, 2);
        let list: Vec<usize> = merged_lists(&voted, &bet_lists, &shares, false, 0)
            .iter()
            .map(|pick| pick.from)
            .take(5)
            .collect();

        assert_eq!(list, [0, 0, 1, 0, 0]);
        assert_eq!(bets_from(only_bets), [0, 0, 1, 0, 0]);
        assert_eq!(bets_from(mixed), [0, 0, 1, 0, 0]);
    }

    #[test]
    fn signals_only_leaves_the_families_out() {
        let options = SearchOptions {
            signals_only: true,
            ..SearchOptions::default()
        };

        let families = families_of(&options);

        assert!(!families.is_empty());
        assert!(families.iter().all(Option::is_none));
    }

    #[test]
    fn prices_of_another_region_than_the_config_are_stale() {
        assert!(stale_prices(Region::new("US").unwrap()));
        assert!(!stale_prices(Region::ITALY));
    }
}
