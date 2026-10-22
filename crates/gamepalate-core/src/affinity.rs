use std::collections::HashMap;

use rayon::prelude::*;

use crate::{
    game_vector::GameVector,
    ids::{AppId, TagId},
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

// For each tag, the references that have it, with their value for that tag.
#[derive(Debug, Clone)]
pub struct ReferenceIndex<'a> {
    references: &'a [Reference],
    postings: HashMap<TagId, Vec<(usize, f64)>>,
}

impl<'a> ReferenceIndex<'a> {
    pub fn new(references: &'a [Reference]) -> ReferenceIndex<'a> {
        let mut postings: HashMap<TagId, Vec<(usize, f64)>> = HashMap::new();
        for (i, reference) in references.iter().enumerate() {
            for (&tag, &value) in &reference.tag_vector {
                postings.entry(tag).or_default().push((i, value));
            }
        }

        ReferenceIndex {
            references,
            postings,
        }
    }

    pub fn len(&self) -> usize {
        self.references.len()
    }

    pub fn is_empty(&self) -> bool {
        self.references.is_empty()
    }
}

// `dots` is scratch space, one slot per reference, all zero between calls.
pub fn closest_references(
    game_vector: &GameVector,
    index: &ReferenceIndex,
    dots: &mut [f64],
) -> Vec<ReferenceMatch> {
    if game_vector.norm == 0.0 {
        return Vec::new();
    }

    for (tag, &value) in &game_vector.tag_vector {
        if let Some(posting) = index.postings.get(tag) {
            for &(i, reference_value) in posting {
                dots[i] += value * reference_value;
            }
        }
    }

    let mut closest_three: Vec<ReferenceMatch> = Vec::new();

    for (reference, dot) in index.references.iter().zip(dots.iter_mut()) {
        let similarity = *dot / (game_vector.norm * reference.norm);
        *dot = 0.0;

        if similarity == 0.0 || reference.id == game_vector.id {
            continue;
        }

        let beats_third = closest_three.len() < 3 || {
            let third = &closest_three[2];
            similarity > third.similarity
                || (similarity == third.similarity && reference.id < third.id)
        };
        if !beats_third {
            continue;
        }

        closest_three.push(ReferenceMatch {
            id: reference.id,
            weight: reference.weight,
            similarity,
        });
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
    let index = ReferenceIndex::new(&references);

    let halves: Vec<(f64, f64, Vec<ReferenceMatch>)> = game_vectors
        .par_iter()
        .map_init(
            || vec![0.0; index.len()],
            |dots, game_vector| {
                let reference_matches = closest_references(game_vector, &index, dots);
                (
                    profile_similarity(game_vector, &profile, profile_norm),
                    references_similarity(&reference_matches),
                    reference_matches,
                )
            },
        )
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
