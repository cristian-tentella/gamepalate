use std::collections::{BTreeMap, BTreeSet};

use crate::{
    catalog::Catalog, families::Families, game::GameTag, ids::AppId, timestamp::Timestamp,
};

const DAY_MS: f64 = 86_400_000.0;

pub type OffCatalogGames = BTreeMap<AppId, OffCatalogGame>;

pub type OffCatalogCache = BTreeMap<AppId, CachedGame>;

#[derive(Debug, Clone, PartialEq)]
pub struct OffCatalogGame {
    pub tags: Vec<GameTag>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedGame {
    pub fetched_at: Timestamp,
    pub game: OffCatalogGame,
}

pub fn wanted_off_catalog(
    effective_hours: &BTreeMap<AppId, f64>,
    families: &Families,
    catalog: &Catalog,
    min_hours: f64,
) -> Vec<AppId> {
    let above_threshold = effective_hours
        .iter()
        .filter(|&(_, &hours)| hours >= min_hours)
        .map(|(&id, _)| id);
    let in_families = families
        .families
        .iter()
        .flat_map(|family| family.games.iter().copied());

    above_threshold
        .chain(in_families)
        .filter(|&id| catalog.game(id).is_none())
        .collect::<BTreeSet<AppId>>()
        .into_iter()
        .collect()
}

pub fn stale_off_catalog(
    cache: &OffCatalogCache,
    wanted: &[AppId],
    now: Timestamp,
    max_age_days: f64,
) -> Vec<AppId> {
    wanted
        .iter()
        .copied()
        .filter(|id| {
            cache.get(id).is_none_or(|cached| {
                now.ms_since(cached.fetched_at) as f64 >= max_age_days * DAY_MS
            })
        })
        .collect()
}

pub fn off_catalog_games(cache: OffCatalogCache) -> OffCatalogGames {
    cache
        .into_iter()
        .map(|(id, cached)| (id, cached.game))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::Freshness,
        families::Family,
        game::{Game, Price, Reviews},
    };

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const DAY: i64 = 86_400_000;

    fn catalog_with(ids: &[u32]) -> Catalog {
        let games = ids
            .iter()
            .map(|&id| Game {
                id: AppId(id),
                name: format!("game {id}"),
                price: Price {
                    original_cents: 999,
                    discount: None,
                },
                reviews: Reviews {
                    total: 0,
                    positive: 0,
                },
                is_early_access: false,
                tags: Vec::new(),
            })
            .collect();
        Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: NOW,
                tags_updated_at: NOW,
            },
        )
    }

    fn cached(fetched_at: Timestamp) -> CachedGame {
        CachedGame {
            fetched_at,
            game: OffCatalogGame {
                tags: Vec::new(),
                name: None,
            },
        }
    }

    #[test]
    fn wants_games_outside_the_catalog_above_the_threshold_or_in_a_family() {
        let hours = BTreeMap::from([
            (AppId(1), 5.0),
            (AppId(2), 5.0),
            (AppId(3), 1.0),
            (AppId(4), 2.0),
        ]);
        let families = Families {
            families: vec![Family {
                name: "Roguelike".to_string(),
                games: vec![AppId(3), AppId(5), AppId(2)],
            }],
            held_out: vec![AppId(6)],
        };

        let wanted = wanted_off_catalog(&hours, &families, &catalog_with(&[1]), 2.0);

        assert_eq!(wanted, [AppId(2), AppId(3), AppId(4), AppId(5)]);
    }

    #[test]
    fn asks_for_games_missing_from_the_cache_or_expired() {
        let cache = OffCatalogCache::from([
            (AppId(1), cached(Timestamp(NOW.0 - 6 * DAY))),
            (AppId(2), cached(Timestamp(NOW.0 - 7 * DAY))),
        ]);

        let stale = stale_off_catalog(&cache, &[AppId(1), AppId(2), AppId(3)], NOW, 7.0);

        assert_eq!(stale, [AppId(2), AppId(3)]);
    }
}
