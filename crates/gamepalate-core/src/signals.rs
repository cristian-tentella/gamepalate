use std::collections::BTreeSet;

use crate::{ids::AppId, timestamp::Timestamp};

#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub read_at: Timestamp,
    pub library: Vec<LibraryGame>,
    pub borrowed: Vec<LibraryGame>,
    pub recent: Vec<RecentGame>,
    pub wishlist: Vec<AppId>,
    pub followed: Vec<AppId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalsReading {
    pub read_at: Timestamp,
    pub library: Vec<LibraryGame>,
    pub borrowed: Vec<LibraryGame>,
    pub recent: Vec<RecentGame>,
    pub wishlist: Option<Vec<AppId>>,
    pub followed: Option<Vec<AppId>>,
    pub wishlist_read_at: Option<Timestamp>,
    pub followed_read_at: Option<Timestamp>,
}

impl Signals {
    pub fn ids(&self) -> Vec<AppId> {
        self.played()
            .chain(self.recent.iter().map(|game| game.id))
            .chain(self.wishlist.iter().copied())
            .chain(self.followed.iter().copied())
            .collect::<BTreeSet<AppId>>()
            .into_iter()
            .collect()
    }

    pub fn played(&self) -> impl Iterator<Item = AppId> + '_ {
        self.library
            .iter()
            .chain(&self.borrowed)
            .map(|game| game.id)
    }
}

impl SignalsReading {
    pub fn into_signals(self) -> Signals {
        Signals {
            read_at: self.read_at,
            library: self.library,
            borrowed: self.borrowed,
            recent: self.recent,
            wishlist: self.wishlist.unwrap_or_default(),
            followed: self.followed.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LibraryGame {
    pub id: AppId,
    pub total_hours: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecentGame {
    pub id: AppId,
    pub total_hours: f64,
    pub recent_hours: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ids_of_every_signal_come_once_in_order() {
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![LibraryGame {
                id: AppId(3),
                total_hours: 1.0,
            }],
            borrowed: vec![LibraryGame {
                id: AppId(7),
                total_hours: 4.0,
            }],
            recent: vec![RecentGame {
                id: AppId(9),
                total_hours: 2.0,
                recent_hours: 1.0,
            }],
            wishlist: vec![AppId(5), AppId(1)],
            followed: vec![AppId(3)],
        };

        assert_eq!(
            signals.ids(),
            [AppId(1), AppId(3), AppId(5), AppId(7), AppId(9)]
        );
    }
}
