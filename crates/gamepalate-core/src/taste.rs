use std::collections::{BTreeMap, BTreeSet};

use crate::{
    catalog::Catalog,
    config::SignalsConfig,
    corrections::Corrections,
    families::Family,
    game::GameTag,
    ids::{AppId, TagId},
    off_catalog::OffCatalogGames,
    tag_vector::{TagVector, add_weighted, apply_multipliers, tf_idf_vector},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Taste {
    pub games: Vec<TasteGame>,
}

impl Taste {
    pub fn profile(&self) -> TagVector {
        let mut tag_vector = TagVector::new();

        for game in &self.games {
            add_weighted(&mut tag_vector, &game.tag_vector, game.weight);
        }

        tag_vector
    }

    pub fn without(&self, id: AppId) -> Taste {
        Taste {
            games: self
                .games
                .iter()
                .filter(|game| game.id != id)
                .cloned()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TasteGame {
    pub id: AppId,
    pub weight: f64,
    pub tag_vector: TagVector,
}

pub fn game_tags<'a>(
    id: AppId,
    catalog: &'a Catalog,
    off_catalog: &'a OffCatalogGames,
) -> Option<&'a [GameTag]> {
    catalog
        .game(id)
        .map(|game| game.tags.as_slice())
        .or_else(|| off_catalog.get(&id).map(|game| game.tags.as_slice()))
}

pub fn game_name(id: AppId, catalog: &Catalog, off_catalog: &OffCatalogGames) -> String {
    catalog
        .game(id)
        .map(|game| game.name.clone())
        .or_else(|| off_catalog.get(&id).and_then(|game| game.name.clone()))
        .unwrap_or_else(|| format!("#{}", id.0))
}

pub fn signals_taste(
    effective_hours: &BTreeMap<AppId, f64>,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
    corrections: &Corrections,
    signals_config: &SignalsConfig,
) -> Taste {
    let mut taste = Taste { games: Vec::new() };

    for (&id, &hours) in effective_hours {
        if hours < signals_config.min_hours {
            continue;
        }

        let Some(tags) = game_tags(id, catalog, off_catalog).filter(|tags| !tags.is_empty()) else {
            continue;
        };

        let weight = f64::min(hours, signals_config.max_hours).ln_1p()
            * corrections
                .game_multipliers
                .get(&id)
                .copied()
                .unwrap_or(1.0);

        if weight == 0.0 {
            continue;
        }

        let tf_idf_vector = tf_idf_vector(tags, &catalog.idf);
        let tag_vector = apply_multipliers(&tf_idf_vector, &corrections.tag_multipliers);

        taste.games.push(TasteGame {
            id,
            weight,
            tag_vector,
        })
    }

    taste
}

pub fn family_taste(
    family: &Family,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
    tag_multipliers: &BTreeMap<TagId, f64>,
) -> Taste {
    let mut taste = Taste { games: Vec::new() };
    let mut seen_ids: BTreeSet<AppId> = BTreeSet::new();

    for &id in &family.games {
        if !seen_ids.insert(id) {
            continue;
        }

        let Some(tags) = game_tags(id, catalog, off_catalog).filter(|tags| !tags.is_empty()) else {
            continue;
        };

        let tf_idf_vector = tf_idf_vector(tags, &catalog.idf);
        let tag_vector = apply_multipliers(&tf_idf_vector, tag_multipliers);

        taste.games.push(TasteGame {
            id,
            weight: 1.0,
            tag_vector,
        })
    }

    taste
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::Freshness,
        game::{Game, Price, Reviews},
        ids::TagId,
        off_catalog::OffCatalogGame,
        timestamp::Timestamp,
    };

    fn tags(pairs: &[(u32, f64)]) -> Vec<GameTag> {
        pairs
            .iter()
            .map(|&(id, weight)| GameTag {
                id: TagId(id),
                weight,
            })
            .collect()
    }

    fn vector(pairs: &[(u32, f64)]) -> TagVector {
        pairs
            .iter()
            .map(|&(id, value)| (TagId(id), value))
            .collect()
    }

    // Games 2 and 20 have no tags. Tag 1 has idf 2, tag 2 has idf 1.
    fn catalog() -> Catalog {
        let game = |id: u32, game_tags: &[(u32, f64)]| Game {
            id: AppId(id),
            name: format!("game {id}"),
            price: Price {
                original_cents: 0,
                discount: None,
            },
            reviews: Reviews {
                total: 0,
                positive: 0,
            },
            is_early_access: false,
            tags: tags(game_tags),
        };
        Catalog::new(
            vec![
                game(1, &[(1, 1.0)]),
                game(2, &[]),
                game(3, &[(2, 1.0)]),
                game(4, &[(1, 1.0)]),
            ],
            BTreeMap::new(),
            BTreeMap::from([(TagId(1), 2.0), (TagId(2), 1.0)]),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
            },
        )
    }

    fn off_catalog() -> OffCatalogGames {
        BTreeMap::from([
            (
                AppId(10),
                OffCatalogGame {
                    tags: tags(&[(1, 0.5)]),
                    name: Some("off game".to_string()),
                },
            ),
            (
                AppId(20),
                OffCatalogGame {
                    tags: Vec::new(),
                    name: None,
                },
            ),
        ])
    }

    fn corrections() -> Corrections {
        Corrections {
            game_multipliers: BTreeMap::from([(AppId(3), 0.0)]),
            tag_multipliers: BTreeMap::from([(TagId(1), 0.5)]),
            ..Corrections::default()
        }
    }

    fn ids(taste: &Taste) -> Vec<AppId> {
        taste.games.iter().map(|game| game.id).collect()
    }

    #[test]
    fn signals_taste_keeps_only_the_games_that_count() {
        let hours = BTreeMap::from([
            (AppId(1), 100.0),
            (AppId(2), 50.0),
            (AppId(3), 10.0),
            (AppId(4), 1.0),
            (AppId(10), 1000.0),
            (AppId(20), 10.0),
            (AppId(99), 10.0),
        ]);
        let taste = signals_taste(
            &hours,
            &catalog(),
            &off_catalog(),
            &corrections(),
            &SignalsConfig::default(),
        );

        assert_eq!(ids(&taste), [AppId(1), AppId(10)]);
    }

    #[test]
    fn signals_taste_caps_the_hours_and_applies_the_tag_multipliers() {
        let hours = BTreeMap::from([(AppId(1), 100.0), (AppId(10), 1000.0)]);
        let taste = signals_taste(
            &hours,
            &catalog(),
            &off_catalog(),
            &corrections(),
            &SignalsConfig::default(),
        );

        assert_eq!(
            taste.games,
            [
                TasteGame {
                    id: AppId(1),
                    weight: 100.0_f64.ln_1p(),
                    tag_vector: vector(&[(1, 1.0)]),
                },
                TasteGame {
                    id: AppId(10),
                    weight: 200.0_f64.ln_1p(),
                    tag_vector: vector(&[(1, 0.5)]),
                },
            ]
        );
    }

    #[test]
    fn family_taste_skips_duplicates_and_games_without_tags() {
        let family = Family {
            name: "mysteries".to_string(),
            games: [3, 1, 3, 2, 99, 10, 20].map(AppId).to_vec(),
        };
        let taste = family_taste(
            &family,
            &catalog(),
            &off_catalog(),
            &corrections().tag_multipliers,
        );

        assert_eq!(ids(&taste), [AppId(3), AppId(1), AppId(10)]);
        assert!(taste.games.iter().all(|game| game.weight == 1.0));
    }

    #[test]
    fn profile_sums_the_weighted_vectors() {
        let taste = Taste {
            games: vec![
                TasteGame {
                    id: AppId(1),
                    weight: 2.0,
                    tag_vector: vector(&[(1, 1.0)]),
                },
                TasteGame {
                    id: AppId(2),
                    weight: 1.0,
                    tag_vector: vector(&[(1, 0.5), (2, 1.0)]),
                },
            ],
        };

        assert_eq!(taste.profile(), vector(&[(1, 2.5), (2, 1.0)]));
    }

    #[test]
    fn game_name_falls_back_to_the_cache_and_then_to_the_appid() {
        let catalog = catalog();
        let off_catalog = off_catalog();
        assert_eq!(game_name(AppId(1), &catalog, &off_catalog), "game 1");
        assert_eq!(game_name(AppId(10), &catalog, &off_catalog), "off game");
        assert_eq!(game_name(AppId(20), &catalog, &off_catalog), "#20");
    }
}
