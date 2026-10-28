use std::collections::BTreeMap;

use crate::{
    game::Game,
    ids::{AppId, TagId},
    timestamp::Timestamp,
};

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Catalog {
    games: Vec<Game>,
    pub tags: BTreeMap<TagId, TagName>,
    pub idf: BTreeMap<TagId, f64>,
    pub freshness: Freshness,
}

impl Catalog {
    pub fn new(
        mut games: Vec<Game>,
        tags: BTreeMap<TagId, TagName>,
        idf: BTreeMap<TagId, f64>,
        freshness: Freshness,
    ) -> Catalog {
        games.sort_unstable_by_key(|game| game.id);
        Catalog {
            games,
            tags,
            idf,
            freshness,
        }
    }

    pub fn games(&self) -> &[Game] {
        &self.games
    }

    pub fn game(&self, id: AppId) -> Option<&Game> {
        self.games
            .binary_search_by_key(&id, |game| game.id)
            .ok()
            .map(|i| &self.games[i])
    }
}

pub fn compute_idf(games: &[Game]) -> BTreeMap<TagId, f64> {
    let mut document_frequency: BTreeMap<TagId, u32> = BTreeMap::new();
    for tag in games.iter().flat_map(|game| &game.tags) {
        if tag.weight > 0.0 {
            *document_frequency.entry(tag.id).or_default() += 1;
        }
    }

    let total = games.len() as f64;
    document_frequency
        .into_iter()
        .map(|(id, df)| (id, (total / (1.0 + f64::from(df))).ln().max(0.0)))
        .collect()
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TagName {
    pub english: String,
    pub italian: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Freshness {
    pub prices_updated_at: Timestamp,
    pub tags_updated_at: Timestamp,
}

impl Freshness {
    pub fn prices_are_stale(&self, now: Timestamp, max_age_hours: u32) -> bool {
        now.ms_since(self.prices_updated_at) > i64::from(max_age_hours) * 3_600_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameTag, Price, Reviews};

    const HOUR_MS: i64 = 3_600_000;
    const UPDATED_AT: Timestamp = Timestamp(1_700_000_000_000);

    fn freshness() -> Freshness {
        Freshness {
            prices_updated_at: UPDATED_AT,
            tags_updated_at: UPDATED_AT,
        }
    }

    fn hours_after_update(hours: i64) -> Timestamp {
        Timestamp(UPDATED_AT.0 + hours * HOUR_MS)
    }

    fn game_with_tags(id: u32, tags: &[(u32, f64)]) -> Game {
        Game {
            id: AppId(id),
            name: String::new(),
            price: Price {
                original_cents: 1000,
                discount: None,
            },
            reviews: Reviews {
                total: 0,
                positive: 0,
            },
            is_early_access: false,
            tags: tags
                .iter()
                .map(|&(id, weight)| GameTag {
                    id: TagId(id),
                    weight,
                })
                .collect(),
        }
    }

    #[test]
    fn compute_idf_follows_the_smoothed_formula() {
        let games = [
            game_with_tags(1, &[(2, 0.5), (5, 0.5)]),
            game_with_tags(2, &[(2, 1.0)]),
            game_with_tags(3, &[]),
            game_with_tags(4, &[]),
        ];

        let idf = compute_idf(&games);

        assert_eq!(idf[&TagId(5)], 2.0_f64.ln());
        assert_eq!(idf[&TagId(2)], (4.0_f64 / 3.0).ln());
    }

    #[test]
    fn compute_idf_is_never_negative() {
        let games = [game_with_tags(1, &[(2, 1.0)])];

        assert_eq!(compute_idf(&games)[&TagId(2)], 0.0);
    }

    #[test]
    fn compute_idf_ignores_tags_with_zero_weight() {
        let games = [game_with_tags(1, &[(2, 1.0), (5, 0.0)])];

        assert!(!compute_idf(&games).contains_key(&TagId(5)));
    }

    #[test]
    fn prices_are_stale_when_older_than_max_age() {
        let now = Timestamp(hours_after_update(24).0 + 1);
        assert!(freshness().prices_are_stale(now, 24));
    }

    #[test]
    fn prices_are_not_stale_when_newer_than_max_age() {
        assert!(!freshness().prices_are_stale(hours_after_update(23), 24));
    }

    #[test]
    fn prices_are_not_stale_at_exactly_max_age() {
        assert!(!freshness().prices_are_stale(hours_after_update(24), 24));
    }
}
