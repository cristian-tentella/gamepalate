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
    count: usize,
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

    let mut closest: Vec<ReferenceMatch> = Vec::new();

    for (reference, dot) in index.references.iter().zip(dots.iter_mut()) {
        let similarity = *dot / (game_vector.norm * reference.norm);
        *dot = 0.0;

        if similarity == 0.0 || reference.id == game_vector.id {
            continue;
        }

        let beats_last = closest.len() < count || {
            let last = &closest[count - 1];
            similarity > last.similarity
                || (similarity == last.similarity && reference.id < last.id)
        };
        if !beats_last {
            continue;
        }

        closest.push(ReferenceMatch {
            id: reference.id,
            weight: reference.weight,
            similarity,
        });
        closest.sort_by(|a, b| {
            a.similarity
                .total_cmp(&b.similarity)
                .reverse()
                .then(a.id.cmp(&b.id))
        });
        closest.truncate(count);
    }

    closest
}

pub fn references_similarity<'a>(
    reference_matches: impl IntoIterator<Item = &'a ReferenceMatch>,
) -> f64 {
    let mut total_weight = 0.0;
    let mut total_weighted_similarity = 0.0;

    for reference_match in reference_matches {
        total_weight += reference_match.weight;
        total_weighted_similarity += reference_match.weight * reference_match.similarity;
    }

    // Reference weights are positive, so zero means no references
    if total_weight == 0.0 {
        return 0.0;
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
                let reference_matches = closest_references(game_vector, &index, dots, 3);
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
    let affinity_values = affinity_values(&profile_halves, &references_halves);

    let affinities = game_vectors
        .iter()
        .zip(halves)
        .zip(affinity_values)
        .map(
            |((game_vector, (_, _, references)), affinity)| GameAffinity {
                id: game_vector.id,
                affinity,
                references,
            },
        )
        .collect();

    Some(affinities)
}

pub fn affinity_values(profile_halves: &[f64], references_halves: &[f64]) -> Vec<f64> {
    let profile_half_scale = scale(profile_halves);
    let references_half_scale = scale(references_halves);

    profile_halves
        .iter()
        .zip(references_halves)
        .map(|(profile_value, references_value)| {
            0.5 * profile_value * profile_half_scale
                + 0.5 * references_value * references_half_scale
        })
        .collect()
}
