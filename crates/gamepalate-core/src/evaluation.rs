use std::collections::{BTreeMap, BTreeSet, HashMap};

use rayon::prelude::*;

use crate::{
    affinity::{
        ReferenceIndex, ReferenceMatch, affinity_values, closest_references, references,
        references_similarity,
    },
    catalog::Catalog,
    effective_hours::effective_hours,
    families::Families,
    filters::FilterContext,
    game_vector::GameVector,
    ids::AppId,
    off_catalog::OffCatalogGames,
    ranking::{RankedGame, ranking_order, relevance, sort_ranking},
    search::SearchInputs,
    signals::Signals,
    tag_vector::{CompactTagVector, TagIndex, dense_dot, norm},
    taste::{Taste, family_taste, game_name, signals_taste},
    turns::in_turns,
};

const REFERENCES: usize = 3;
// One more than the ranking uses, so the left out game can be skipped
const NEAREST: usize = REFERENCES + 1;
const GUARDRAIL_SIZE: usize = 60;
const TOP: usize = 100;
const TOP_WIDE: usize = 1000;

#[derive(Debug, Clone)]
pub struct PreparedTaste {
    pub taste: Taste,
    pub nearest: Vec<Vec<ReferenceMatch>>,
}

impl PreparedTaste {
    pub fn contains(&self, id: AppId) -> bool {
        self.taste.games.iter().any(|game| game.id == id)
    }
}

// A family with its normal relevances and ranking, computed once.
#[derive(Debug, Clone)]
pub struct PreparedFamily {
    pub taste: PreparedTaste,
    pub relevances: Vec<f64>,
    pub ranking: Vec<RankedGame>,
}

#[derive(Debug, Clone)]
pub struct EvaluatedGame {
    pub id: AppId,
    pub name: String,
    pub position: Option<usize>,
    pub effective_hours: Option<f64>,
    pub families: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GroupSummary {
    pub games: Vec<EvaluatedGame>,
    pub in_top: usize,
    pub in_top_wide: usize,
    pub median: Option<f64>,
    pub geometric_mean: Option<f64>,
    pub without_position: usize,
}

#[derive(Debug, Clone)]
pub struct Evaluation {
    pub kept_aside: GroupSummary,
    pub guardrail: GroupSummary,
    pub families: GroupSummary,
    pub catalog_size: usize,
}

// The game vectors with compact tags, so the profile half is fast.
#[derive(Debug, Clone)]
pub struct CompactVectors {
    pub tag_index: TagIndex,
    pub tag_vectors: Vec<CompactTagVector>,
}

impl CompactVectors {
    pub fn new(game_vectors: &[GameVector], catalog: &Catalog) -> CompactVectors {
        let tag_index = TagIndex::new(&catalog.idf);
        let tag_vectors = game_vectors
            .par_iter()
            .map(|game_vector| tag_index.compact(&game_vector.tag_vector))
            .collect();

        CompactVectors {
            tag_index,
            tag_vectors,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EvaluationInputs<'a> {
    pub catalog: &'a Catalog,
    pub game_vectors: &'a [GameVector],
    pub compact: CompactVectors,
    pub signals: PreparedTaste,
    pub families: Vec<PreparedFamily>,
    pub competitors: Vec<usize>,
    pub quality_weight: f64,
    pub min_reviews: u32,
}

pub fn prepare_taste(game_vectors: &[GameVector], taste: Taste) -> PreparedTaste {
    let references = references(&taste);
    let index = ReferenceIndex::new(&references);

    let nearest = game_vectors
        .par_iter()
        .map_init(
            || vec![0.0; index.len()],
            |dots, game_vector| closest_references(game_vector, &index, dots, NEAREST),
        )
        .collect();

    PreparedTaste { taste, nearest }
}

pub fn relevances_without(
    prepared: &PreparedTaste,
    game_vectors: &[GameVector],
    compact: &CompactVectors,
    without: Option<AppId>,
    quality_weight: f64,
) -> Option<Vec<f64>> {
    let profile = match without {
        Some(id) => prepared.taste.without(id).profile(),
        None => prepared.taste.profile(),
    };
    let profile_norm = norm(&profile);
    if profile_norm == 0.0 {
        return None;
    }

    // Same as profile_similarity
    let dense_profile = compact.tag_index.dense(&profile);
    let profile_halves: Vec<f64> = game_vectors
        .iter()
        .zip(&compact.tag_vectors)
        .map(|(game_vector, tag_vector)| {
            if game_vector.norm == 0.0 {
                return 0.0;
            }
            dense_dot(tag_vector, &dense_profile) / (profile_norm * game_vector.norm)
        })
        .collect();
    let references_halves: Vec<f64> = prepared
        .nearest
        .iter()
        .map(|nearest| {
            references_similarity(
                nearest
                    .iter()
                    .filter(|reference_match| Some(reference_match.id) != without)
                    .take(REFERENCES),
            )
        })
        .collect();

    let relevances = affinity_values(&profile_halves, &references_halves)
        .into_iter()
        .zip(game_vectors)
        .map(|(affinity, game_vector)| relevance(affinity, game_vector.quality, quality_weight))
        .collect();

    Some(relevances)
}

pub fn competitors(catalog: &Catalog, context: &FilterContext, min_reviews: u32) -> Vec<usize> {
    catalog
        .games()
        .iter()
        .enumerate()
        .filter(|(_, game)| {
            game.reviews.total >= min_reviews
                && !context.owned.contains(&game.id)
                && !context.family_games.contains(&game.id)
        })
        .map(|(i, _)| i)
        .collect()
}

// Positions need only the relevance, so references and affinity stay empty.
fn ranked_game(game_vectors: &[GameVector], relevances: &[f64], i: usize) -> RankedGame {
    RankedGame {
        id: game_vectors[i].id,
        relevance: relevances[i],
        references: Vec::new(),
        affinity: 0.0,
    }
}

pub fn competitor_ranking(
    game_vectors: &[GameVector],
    relevances: &[f64],
    competitors: &[usize],
    x: Option<usize>,
) -> Vec<RankedGame> {
    let x = x.filter(|x| competitors.binary_search(x).is_err());
    let mut ranked: Vec<RankedGame> = competitors
        .iter()
        .copied()
        .chain(x)
        .map(|i| ranked_game(game_vectors, relevances, i))
        .collect();
    sort_ranking(&mut ranked);
    ranked
}

pub fn position(x: AppId, inputs: &EvaluationInputs) -> Option<usize> {
    // Game vectors follow the catalog, sorted by appid
    let x_index = inputs
        .game_vectors
        .binary_search_by_key(&x, |game_vector| game_vector.id)
        .ok()?;
    if inputs.catalog.games()[x_index].reviews.total < inputs.min_reviews {
        return None;
    }

    let ranking_without_x = |taste: &PreparedTaste| {
        relevances_without(
            taste,
            inputs.game_vectors,
            &inputs.compact,
            Some(x),
            inputs.quality_weight,
        )
        .map(|relevances| {
            competitor_ranking(
                inputs.game_vectors,
                &relevances,
                &inputs.competitors,
                Some(x_index),
            )
        })
    };

    let mut lists = vec![ranking_without_x(&inputs.signals)?];
    for family in &inputs.families {
        if family.taste.contains(x) {
            lists.extend(ranking_without_x(&family.taste));
            continue;
        }

        let mut ranking = family.ranking.clone();
        if inputs.competitors.binary_search(&x_index).is_err() {
            let game = ranked_game(inputs.game_vectors, &family.relevances, x_index);
            let at = ranking.partition_point(|other| ranking_order(other, &game).is_lt());
            ranking.insert(at, game);
        }
        lists.push(ranking);
    }

    in_turns(&lists)
        .iter()
        .position(|pick| pick.game.id == x)
        .map(|i| i + 1)
}

fn evaluated_game(id: AppId, catalog: &Catalog, off_catalog: &OffCatalogGames) -> EvaluatedGame {
    EvaluatedGame {
        id,
        name: game_name(id, catalog, off_catalog),
        position: None,
        effective_hours: None,
        families: Vec::new(),
    }
}

pub fn kept_aside_group(
    families: &Families,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
) -> Vec<EvaluatedGame> {
    let mut seen = BTreeSet::new();
    families
        .held_out
        .iter()
        .filter(|&&id| seen.insert(id))
        .map(|&id| evaluated_game(id, catalog, off_catalog))
        .collect()
}

pub fn guardrail_group(
    families: &Families,
    signals: &Signals,
    effective_hours: &BTreeMap<AppId, f64>,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
) -> Vec<EvaluatedGame> {
    let family_games: BTreeSet<AppId> = families
        .families
        .iter()
        .flat_map(|family| family.games.iter().copied())
        .collect();

    let mut games: Vec<(AppId, f64)> = signals
        .library
        .iter()
        .map(|game| game.id)
        .filter(|id| {
            catalog.game(*id).is_some()
                && !family_games.contains(id)
                && !families.held_out.contains(id)
        })
        .map(|id| (id, effective_hours.get(&id).copied().unwrap_or(0.0)))
        .collect();
    games.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    games
        .into_iter()
        .take(GUARDRAIL_SIZE)
        .map(|(id, hours)| EvaluatedGame {
            effective_hours: Some(hours),
            ..evaluated_game(id, catalog, off_catalog)
        })
        .collect()
}

pub fn families_group(
    families: &Families,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
) -> Vec<EvaluatedGame> {
    let mut games: Vec<EvaluatedGame> = Vec::new();

    for family in &families.families {
        for &id in &family.games {
            match games.iter_mut().find(|game| game.id == id) {
                Some(game) => {
                    if !game.families.contains(&family.name) {
                        game.families.push(family.name.clone());
                    }
                }
                None => games.push(EvaluatedGame {
                    families: vec![family.name.clone()],
                    ..evaluated_game(id, catalog, off_catalog)
                }),
            }
        }
    }

    games
}

pub fn median(positions: &[usize]) -> Option<f64> {
    if positions.is_empty() {
        return None;
    }

    let mut sorted = positions.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;

    if sorted.len().is_multiple_of(2) {
        Some((sorted[middle - 1] + sorted[middle]) as f64 / 2.0)
    } else {
        Some(sorted[middle] as f64)
    }
}

pub fn geometric_mean(positions: &[usize]) -> Option<f64> {
    if positions.is_empty() {
        return None;
    }

    let sum_of_logs: f64 = positions.iter().map(|&p| (p as f64).ln()).sum();
    Some((sum_of_logs / positions.len() as f64).exp())
}

pub fn summarize(games: Vec<EvaluatedGame>) -> GroupSummary {
    let positions: Vec<usize> = games.iter().filter_map(|game| game.position).collect();

    GroupSummary {
        in_top: positions.iter().filter(|&&p| p <= TOP).count(),
        in_top_wide: positions.iter().filter(|&&p| p <= TOP_WIDE).count(),
        median: median(&positions),
        geometric_mean: geometric_mean(&positions),
        without_position: games.len() - positions.len(),
        games,
    }
}

// Ignores hidden games, votes, banned tags and the opinion: it measures tastes, not the list.
pub fn evaluate(inputs: &SearchInputs) -> Evaluation {
    let config = inputs.config;
    let quality_weight = config.relevance.quality_weight;
    let hours = effective_hours(inputs.signals, inputs.votes, &config.signals);
    let taste = signals_taste(
        &hours,
        inputs.catalog,
        inputs.off_catalog,
        inputs.corrections,
        &config.signals,
    );
    let context = FilterContext::new(
        inputs.signals,
        inputs.families,
        inputs.votes,
        inputs.corrections,
    );
    let competitor_indices = competitors(inputs.catalog, &context, config.relevance.min_reviews);

    let compact = CompactVectors::new(inputs.game_vectors, inputs.catalog);
    let families = inputs
        .families
        .families
        .iter()
        .filter_map(|family| {
            let taste = family_taste(
                family,
                inputs.catalog,
                inputs.off_catalog,
                &inputs.corrections.tag_multipliers,
            );
            let taste = prepare_taste(inputs.game_vectors, taste);
            let relevances =
                relevances_without(&taste, inputs.game_vectors, &compact, None, quality_weight)?;
            let ranking =
                competitor_ranking(inputs.game_vectors, &relevances, &competitor_indices, None);

            Some(PreparedFamily {
                taste,
                relevances,
                ranking,
            })
        })
        .collect();

    let evaluation_inputs = EvaluationInputs {
        catalog: inputs.catalog,
        game_vectors: inputs.game_vectors,
        compact,
        signals: prepare_taste(inputs.game_vectors, taste),
        families,
        competitors: competitor_indices,
        quality_weight,
        min_reviews: config.relevance.min_reviews,
    };

    let mut groups = [
        kept_aside_group(inputs.families, inputs.catalog, inputs.off_catalog),
        guardrail_group(
            inputs.families,
            inputs.signals,
            &hours,
            inputs.catalog,
            inputs.off_catalog,
        ),
        families_group(inputs.families, inputs.catalog, inputs.off_catalog),
    ];

    // The same game can be in more than one group, but its position is computed once
    let ids: BTreeSet<AppId> = groups.iter().flatten().map(|game| game.id).collect();
    let ids: Vec<AppId> = ids.into_iter().collect();
    let positions: HashMap<AppId, Option<usize>> = ids
        .par_iter()
        .map(|&id| (id, position(id, &evaluation_inputs)))
        .collect();
    for game in groups.iter_mut().flatten() {
        game.position = positions[&game.id];
    }

    let [kept_aside, guardrail, families] = groups.map(summarize);
    Evaluation {
        kept_aside,
        guardrail,
        families,
        catalog_size: inputs.catalog.games().len(),
    }
}
