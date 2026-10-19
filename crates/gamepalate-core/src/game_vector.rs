use rayon::prelude::*;

use crate::{
    catalog::Catalog,
    ids::AppId,
    quality::quality,
    tag_vector::{TagVector, norm, tf_idf_vector},
};

#[derive(Debug, Clone)]
pub struct GameVector {
    pub id: AppId,
    pub tag_vector: TagVector,
    pub norm: f64,
    pub quality: f64,
}

pub fn game_vectors(catalog: &Catalog) -> Vec<GameVector> {
    catalog
        .games()
        .par_iter()
        .map(|game| {
            let tag_vector = tf_idf_vector(&game.tags, &catalog.idf);
            let norm = norm(&tag_vector);

            GameVector {
                id: game.id,
                tag_vector,
                norm,
                quality: quality(&game.reviews),
            }
        })
        .collect()
}
