use std::collections::BTreeMap;

use crate::{game::GameTag, ids::TagId};

pub type TagVector = BTreeMap<TagId, f64>;

pub fn tf_idf_vector(tags: &[GameTag], idf: &BTreeMap<TagId, f64>) -> TagVector {
    tags.iter()
        .map(|tag| {
            (
                tag.id,
                tag.weight * idf.get(&tag.id).copied().unwrap_or(0.0),
            )
        })
        .filter(|&(_, value)| value > 0.0)
        .collect()
}

pub fn apply_multipliers(
    tag_vector: &TagVector,
    tag_multipliers: &BTreeMap<TagId, f64>,
) -> TagVector {
    tag_vector
        .iter()
        .map(|(id, value)| (*id, value * tag_multipliers.get(id).copied().unwrap_or(1.0)))
        .collect()
}

pub fn add_weighted(target: &mut TagVector, source: &TagVector, weight: f64) {
    for (&tag, &value) in source {
        *target.entry(tag).or_insert(0.0) += value * weight;
    }
}

pub fn norm(tag_vector: &TagVector) -> f64 {
    tag_vector.values().map(|v| v.powi(2)).sum::<f64>().sqrt()
}

pub fn dot(a: &TagVector, b: &TagVector) -> f64 {
    a.iter()
        .filter_map(|(id, a_value)| b.get(id).map(|b_value| a_value * b_value))
        .sum()
}

// A tag vector with tags numbered from 0, in tag id order.
pub type CompactTagVector = Vec<(usize, f64)>;

// Numbers the catalog tags from 0, so a profile fits in a short array.
#[derive(Debug, Clone)]
pub struct TagIndex {
    tags: Vec<TagId>,
}

impl TagIndex {
    pub fn new(idf: &BTreeMap<TagId, f64>) -> TagIndex {
        TagIndex {
            tags: idf.keys().copied().collect(),
        }
    }

    pub fn compact(&self, tag_vector: &TagVector) -> CompactTagVector {
        tag_vector
            .iter()
            .filter_map(|(tag, &value)| Some((self.tags.binary_search(tag).ok()?, value)))
            .collect()
    }

    pub fn dense(&self, tag_vector: &TagVector) -> Vec<Option<f64>> {
        let mut dense = vec![None; self.tags.len()];
        for (i, value) in self.compact(tag_vector) {
            dense[i] = Some(value);
        }
        dense
    }
}

// Same as `dot`, with the second vector dense.
pub fn dense_dot(a: &[(usize, f64)], b: &[Option<f64>]) -> f64 {
    a.iter()
        .filter_map(|&(i, a_value)| b[i].map(|b_value| a_value * b_value))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(pairs: &[(u32, f64)]) -> TagVector {
        pairs
            .iter()
            .map(|&(id, value)| (TagId(id), value))
            .collect()
    }

    #[test]
    fn tf_idf_vector_drops_tags_without_a_positive_idf() {
        let tags = [(1, 0.5), (2, 0.3), (3, 0.2)].map(|(id, weight)| GameTag {
            id: TagId(id),
            weight,
        });
        let idf = BTreeMap::from([(TagId(1), 2.0), (TagId(2), 0.0)]);

        assert_eq!(tf_idf_vector(&tags, &idf), vector(&[(1, 1.0)]));
    }

    #[test]
    fn apply_multipliers_leaves_tags_without_a_multiplier_alone() {
        let multipliers = BTreeMap::from([(TagId(1), 0.5)]);
        let result = apply_multipliers(&vector(&[(1, 2.0), (2, 3.0)]), &multipliers);
        assert_eq!(result, vector(&[(1, 1.0), (2, 3.0)]));
    }

    #[test]
    fn add_weighted_sums_shared_tags_and_adds_new_ones() {
        let mut target = vector(&[(1, 1.0)]);
        add_weighted(&mut target, &vector(&[(1, 2.0), (2, 1.0)]), 0.5);
        assert_eq!(target, vector(&[(1, 2.0), (2, 0.5)]));
    }
}
