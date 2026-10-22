use std::collections::HashMap;

use crate::{
    affinity::{ReferenceMatch, affinities},
    config::RelevanceConfig,
    game_vector::GameVector,
    ids::AppId,
    taste::Taste,
    votes::Votes,
};

#[derive(Debug, Clone)]
pub struct RankedGame {
    pub id: AppId,
    pub relevance: f64,
    pub references: Vec<ReferenceMatch>,
    pub affinity: f64,
}

pub fn relevance(affinity: f64, quality: f64, quality_weight: f64) -> f64 {
    affinity * (1.0 - quality_weight + quality_weight * quality)
}

pub fn sort_ranking(ranked_games: &mut [RankedGame]) {
    ranked_games.sort_unstable_by(|a, b| b.relevance.total_cmp(&a.relevance).then(a.id.cmp(&b.id)));
}

pub fn ranking(
    game_vectors: &[GameVector],
    taste: &Taste,
    relevance_config: &RelevanceConfig,
) -> Option<Vec<RankedGame>> {
    let affinities = affinities(game_vectors, taste)?;
    let mut ranked_games: Vec<RankedGame> = affinities
        .into_iter()
        .zip(game_vectors)
        .map(|(affinity, game)| RankedGame {
            id: game.id,
            affinity: affinity.affinity,
            relevance: relevance(
                affinity.affinity,
                game.quality,
                relevance_config.quality_weight,
            ),
            references: affinity.references,
        })
        .collect();
    sort_ranking(&mut ranked_games);
    Some(ranked_games)
}

pub fn with_votes(
    ranked_games: &[RankedGame],
    opinions: &HashMap<AppId, f64>,
    votes_weight: f64,
) -> Vec<RankedGame> {
    let mut voted: Vec<RankedGame> = ranked_games
        .iter()
        .map(|ranked_game| {
            let opinion = opinions.get(&ranked_game.id).copied().unwrap_or(0.0);
            let mut voted_game = ranked_game.clone();
            voted_game.relevance *= (votes_weight * opinion).exp();
            voted_game
        })
        .collect();
    sort_ranking(&mut voted);
    voted
}

pub fn bets(
    ranked_games: &[RankedGame],
    opinions: &HashMap<AppId, f64>,
    votes: &Votes,
    votes_weight: f64,
) -> Vec<RankedGame> {
    ranked_games
        .iter()
        .filter(|ranked_game| !votes.contains_key(&ranked_game.id))
        .filter_map(|ranked_game| {
            let opinion = *opinions.get(&ranked_game.id)?;
            if opinion >= 0.0 {
                return None;
            }
            let mut bet = ranked_game.clone();
            bet.relevance *= (votes_weight * opinion).exp();
            Some(bet)
        })
        .collect()
}
