use crate::{
    affinity::{ReferenceMatch, affinities},
    config::RelevanceConfig,
    game_vector::GameVector,
    ids::AppId,
    taste::Taste,
};

#[derive(Debug, Clone)]
pub struct RankedGame {
    pub id: AppId,
    pub relevance: f64,
    pub references: Vec<ReferenceMatch>,
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
