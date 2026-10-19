use rayon::prelude::*;

use crate::{
    game_vector::GameVector,
    ids::AppId,
    tag_vector::{TagVector, dot, norm},
    taste::Taste,
};

#[derive(Debug, Clone)]
pub struct Reference {
    pub id: AppId,
    pub weight: f64,
    pub tag_vector: TagVector,
    pub norm: f64,
}

#[derive(Debug, Clone)]
pub struct ReferenceMatch {
    pub id: AppId,
    pub weight: f64,
    pub similarity: f64,
}

#[derive(Debug, Clone)]
pub struct GameAffinity {
    pub id: AppId,
    pub affinity: f64,
    pub references: Vec<ReferenceMatch>,
}

pub fn references(taste: &Taste) -> Vec<Reference> {
    taste
        .games
        .iter()
        .filter(|game| game.weight > 0.0 && !game.tag_vector.is_empty())
        .map(|game| Reference {
            id: game.id,
            weight: game.weight,
            tag_vector: game.tag_vector.clone(),
            norm: norm(&game.tag_vector),
        })
        .collect()
}

pub fn profile_similarity(game_vector: &GameVector, profile: &TagVector, profile_norm: f64) -> f64 {
    if game_vector.norm == 0.0 {
        return 0.0;
    }

    dot(&game_vector.tag_vector, profile) / (profile_norm * game_vector.norm)
}

pub fn closest_references(
    game_vector: &GameVector,
    references: &[Reference],
) -> Vec<ReferenceMatch> {
    if game_vector.norm == 0.0 {
        return Vec::new();
    }

    let mut closest_three: Vec<ReferenceMatch> = Vec::new();

    for reference in references {
        if reference.id == game_vector.id {
            continue;
        }

        let similarity = dot(&game_vector.tag_vector, &reference.tag_vector)
            / (game_vector.norm * reference.norm);

        if similarity == 0.0 {
            continue;
        }

        let reference_match = ReferenceMatch {
            id: reference.id,
            weight: reference.weight,
            similarity,
        };

        closest_three.push(reference_match);
        closest_three.sort_by(|a, b| {
            a.similarity
                .total_cmp(&b.similarity)
                .reverse()
                .then(a.id.cmp(&b.id))
        });
        closest_three.truncate(3);
    }

    closest_three
}

pub fn references_similarity(reference_matches: &[ReferenceMatch]) -> f64 {
    if reference_matches.is_empty() {
        return 0.0;
    }

    let mut total_weight = 0.0;
    let mut total_weighted_similarity = 0.0;

    for reference_match in reference_matches {
        total_weight += reference_match.weight;
        total_weighted_similarity += reference_match.weight * reference_match.similarity;
    }

    total_weighted_similarity / total_weight
}

pub fn scale(values: &[f64]) -> f64 {
    let sum = values.iter().sum::<f64>();
    if sum == 0.0 {
        return 0.0;
    }
    values.len() as f64 / sum
}

pub fn affinities(game_vectors: &[GameVector], taste: &Taste) -> Option<Vec<GameAffinity>> {
    let profile = taste.profile();
    let profile_norm = norm(&profile);
    if profile_norm == 0.0 {
        return None;
    }
    let references = references(taste);

    let halves: Vec<(f64, f64, Vec<ReferenceMatch>)> = game_vectors
        .par_iter()
        .map(|game_vector| {
            let reference_matches = closest_references(game_vector, &references);
            (
                profile_similarity(game_vector, &profile, profile_norm),
                references_similarity(&reference_matches),
                reference_matches,
            )
        })
        .collect();

    let profile_halves: Vec<f64> = halves.iter().map(|half| half.0).collect();
    let references_halves: Vec<f64> = halves.iter().map(|half| half.1).collect();
    let profile_half_scale = scale(&profile_halves);
    let references_half_scale = scale(&references_halves);

    let affinities = game_vectors
        .iter()
        .zip(halves)
        .map(
            |(game_vector, (profile_value, references_value, references))| {
                let affinity = 0.5 * profile_value * profile_half_scale
                    + 0.5 * references_value * references_half_scale;

                GameAffinity {
                    id: game_vector.id,
                    affinity,
                    references,
                }
            },
        )
        .collect();

    Some(affinities)
}
