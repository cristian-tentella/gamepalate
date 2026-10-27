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
