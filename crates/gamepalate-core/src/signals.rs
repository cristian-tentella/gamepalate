use crate::{ids::AppId, timestamp::Timestamp};

#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub read_at: Timestamp,
    pub library: Vec<LibraryGame>,
    pub recent: Vec<RecentGame>,
    pub wishlist: Vec<AppId>,
    pub followed: Vec<AppId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalsReading {
    pub read_at: Timestamp,
    pub library: Vec<LibraryGame>,
    pub recent: Vec<RecentGame>,
    pub wishlist: Option<Vec<AppId>>,
    pub followed: Option<Vec<AppId>>,
    pub wishlist_read_at: Option<Timestamp>,
    pub followed_read_at: Option<Timestamp>,
}

impl SignalsReading {
    pub fn into_signals(self) -> Signals {
        Signals {
            read_at: self.read_at,
            library: self.library,
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
