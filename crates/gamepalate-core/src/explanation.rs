use crate::{
    affinity::ReferenceMatch,
    catalog::Catalog,
    game::Game,
    tag_vector::{TagVector, tf_idf_vector},
};

pub fn top_tags(game: &Game, profile: &TagVector, catalog: &Catalog) -> Vec<String> {
    let vector = tf_idf_vector(&game.tags, &catalog.idf);
    let mut contributions: Vec<_> = vector
        .iter()
        .map(|(&id, &value)| (id, value * profile.get(&id).copied().unwrap_or(0.0)))
        .filter(|&(_, contribution)| contribution > 0.0)
        .collect();
    contributions.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    contributions
        .iter()
        .take(3)
        .map(|(id, _)| match catalog.tags.get(id) {
            Some(name) => name.italian.clone(),
            None => format!("#{}", id.0),
        })
        .collect()
}

pub fn similar_to(references: &[ReferenceMatch]) -> Option<&ReferenceMatch> {
    references
        .iter()
        .map(|reference| (reference, reference.weight * reference.similarity))
        .filter(|&(_, strength)| strength > 0.0)
        .min_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.cmp(&b.0.id)))
        .map(|(reference, _)| reference)
}
