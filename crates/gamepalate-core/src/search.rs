use std::{cmp::Reverse, collections::HashMap};

use crate::{
    catalog::{Catalog, Freshness},
    config::Config,
    corrections::Corrections,
    effective_hours::effective_hours,
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
    turns::{Pick, in_turns, with_bets},
    votes::Votes,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Relevance,
    Discount,
    Price,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub family: Option<String>,
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
        taste: family_taste(
            family,
            inputs.catalog,
            inputs.off_catalog,
            &inputs.corrections.tag_multipliers,
        ),
    };

    match family {
        None => {
            let hours = effective_hours(inputs.signals, inputs.votes, &inputs.config.signals);
            let taste = signals_taste(
                &hours,
                inputs.catalog,
                inputs.off_catalog,
                inputs.corrections,
                &inputs.config.signals,
            );

            if taste.profile().is_empty() {
                return Err(SearchError::EmptySignalsProfile);
            }

            let mut search_tastes = vec![SearchTaste {
                family: None,
                taste,
            }];

            search_tastes.extend(inputs.families.families.iter().map(taste_of));

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

pub fn recommend(
    inputs: &SearchInputs,
    options: &SearchOptions,
    now: Timestamp,
) -> Result<Recommendation, SearchError> {
    let filters = active_filters(&options.filters, inputs.config, inputs.catalog)?;
    let tastes = search_tastes(inputs, options.family.as_deref())?;
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
        opinion_model(inputs.votes, inputs.catalog)
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

    let bet_every = inputs.config.results.bet_every;
    let mut picks = if options.only_bets {
        in_turns(&bet_lists)
            .into_iter()
            .map(|pick| Pick { bet: true, ..pick })
            .collect()
    } else if opinion_by_id.is_some() && bet_every > 0 {
        with_bets(in_turns(&voted), in_turns(&bet_lists), bet_every)
    } else {
        in_turns(&voted)
    };

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
        stale_prices: inputs
            .catalog
            .freshness
            .prices_are_stale(now, inputs.config.prices_max_age_hours),
        without_vote_model: opinion_needed && opinion_by_id.is_none(),
    })
}
