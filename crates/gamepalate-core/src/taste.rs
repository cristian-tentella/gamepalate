use std::collections::{BTreeMap, BTreeSet};

use crate::{
    catalog::Catalog,
    config::SignalsConfig,
    corrections::Corrections,
    effective_hours::HoursSource,
    families::Family,
    game::GameTag,
    ids::{AppId, TagId},
    off_catalog::OffCatalogGames,
    tag_vector::{TagVector, add_weighted, apply_multipliers, norm, tf_idf_vector},
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

    // A profile with only zero weights ranks nothing, like an empty one.
    pub fn has_profile(&self) -> bool {
        norm(&self.profile()) > 0.0
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
    hours_by_source: &BTreeMap<AppId, (f64, HoursSource)>,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
    corrections: &Corrections,
    signals_config: &SignalsConfig,
) -> Taste {
    let mut taste = Taste { games: Vec::new() };
    let mut sources: Vec<HoursSource> = Vec::new();

    for (&id, &(hours, source)) in hours_by_source {
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
        });
        sources.push(source);
    }

    cap_wishlist(&mut taste, &sources, signals_config.wishlist_cap);

    taste
}

// With no played games there is nothing to compare the wishlist to, so it stays as it is.
fn cap_wishlist(taste: &mut Taste, sources: &[HoursSource], wishlist_cap: f64) {
    if wishlist_cap == 0.0 {
        return;
    }

    let total = |counts: fn(HoursSource) -> bool| -> f64 {
        taste
            .games
            .iter()
            .zip(sources)
            .filter(|&(_, &source)| counts(source))
            .map(|(game, _)| game.weight)
            .sum()
    };
    let played = total(|source| matches!(source, HoursSource::Played | HoursSource::Elsewhere));
    let wishlist = total(|source| source == HoursSource::Wishlist);

    if played == 0.0 || wishlist <= wishlist_cap * played {
        return;
    }

    let factor = wishlist_cap * played / wishlist;
    for (game, &source) in taste.games.iter_mut().zip(sources) {
        if source == HoursSource::Wishlist {
            game.weight *= factor;
        }
    }
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
        currency::Currency,
        game::{Game, Price, Reviews},
        ids::TagId,
        off_catalog::OffCatalogGame,
        region::Region,
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
                region: Region::ITALY,
            },
            Currency::italian(),
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

    fn played(pairs: &[(u32, f64)]) -> BTreeMap<AppId, (f64, HoursSource)> {
        pairs
            .iter()
            .map(|&(id, hours)| (AppId(id), (hours, HoursSource::Played)))
            .collect()
    }

    fn with_wishlist(
        mut hours: BTreeMap<AppId, (f64, HoursSource)>,
        wishlist: &[(u32, f64)],
    ) -> BTreeMap<AppId, (f64, HoursSource)> {
        hours.extend(
            wishlist
                .iter()
                .map(|&(id, hours)| (AppId(id), (hours, HoursSource::Wishlist))),
        );
        hours
    }

    fn capped_weights(hours: &BTreeMap<AppId, (f64, HoursSource)>, wishlist_cap: f64) -> Vec<f64> {
        let config = SignalsConfig {
            wishlist_cap,
            ..SignalsConfig::default()
        };
        let taste = signals_taste(
            hours,
            &catalog(),
            &off_catalog(),
            &Corrections::default(),
            &config,
        );
        taste.games.iter().map(|game| game.weight).collect()
    }

    fn ids(taste: &Taste) -> Vec<AppId> {
        taste.games.iter().map(|game| game.id).collect()
    }

    #[test]
    fn signals_taste_keeps_only_the_games_that_count() {
        let hours = played(&[
            (1, 100.0),
            (2, 50.0),
            (3, 10.0),
            (4, 1.0),
            (10, 1000.0),
            (20, 10.0),
            (99, 10.0),
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
        let hours = played(&[(1, 100.0), (10, 1000.0)]);
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

    #[test]
    fn signals_taste_changes_when_a_game_is_played_elsewhere() {
        use crate::{
            effective_hours::hours_by_source, signals::Signals, timestamp::Timestamp, votes::Votes,
        };

        let signals = Signals {
            read_at: Timestamp(0),
            library: Vec::new(),
            borrowed: Vec::new(),
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        };
        let config = SignalsConfig::default();
        let taste_of = |played_elsewhere: &BTreeSet<AppId>| {
            let hours = hours_by_source(&signals, &Votes::new(), played_elsewhere, &config);
            signals_taste(&hours, &catalog(), &off_catalog(), &corrections(), &config)
        };

        assert!(ids(&taste_of(&BTreeSet::new())).is_empty());
        assert_eq!(ids(&taste_of(&BTreeSet::from([AppId(4)]))), [AppId(4)]);
    }

    #[test]
    fn a_wishlist_under_the_cap_stays_as_it_is() {
        let hours = with_wishlist(played(&[(1, 100.0)]), &[(4, 5.0)]);

        assert_eq!(
            capped_weights(&hours, 1.0),
            [100.0_f64.ln_1p(), 5.0_f64.ln_1p()]
        );
    }

    #[test]
    fn a_wishlist_over_the_cap_weighs_exactly_the_cap_times_the_played_games() {
        let hours = with_wishlist(played(&[(1, 2.0)]), &[(3, 100.0), (4, 50.0)]);

        let weights = capped_weights(&hours, 1.5);

        let played_weight = 2.0_f64.ln_1p();
        assert_eq!(weights[0], played_weight);
        assert!((weights[1] + weights[2] - 1.5 * played_weight).abs() < 1e-12);
        let ratio = weights[1] / weights[2];
        assert!((ratio - 100.0_f64.ln_1p() / 50.0_f64.ln_1p()).abs() < 1e-12);
    }

    #[test]
    fn a_cap_of_zero_means_no_cap() {
        let hours = with_wishlist(played(&[(1, 2.0)]), &[(3, 100.0), (4, 50.0)]);

        assert_eq!(
            capped_weights(&hours, 0.0),
            [2.0_f64.ln_1p(), 100.0_f64.ln_1p(), 50.0_f64.ln_1p()]
        );
    }

    #[test]
    fn a_wishlist_without_played_games_is_not_capped() {
        let hours = with_wishlist(BTreeMap::new(), &[(3, 100.0), (4, 50.0)]);

        assert_eq!(
            capped_weights(&hours, 1.0),
            [100.0_f64.ln_1p(), 50.0_f64.ln_1p()]
        );
    }

    #[test]
    fn a_wishlist_game_that_is_also_played_counts_as_played() {
        let hours = with_wishlist(played(&[(1, 2.0), (3, 100.0)]), &[(4, 50.0)]);

        assert_eq!(
            capped_weights(&hours, 1.0),
            [2.0_f64.ln_1p(), 100.0_f64.ln_1p(), 50.0_f64.ln_1p()]
        );
    }
}
