use crate::{ids::AppId, timestamp::Timestamp};

#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub read_at: Timestamp,
    pub library: Vec<LibraryGame>,
    pub recent: Vec<RecentGame>,
    pub wishlist: Vec<AppId>,
    pub followed: Vec<AppId>,
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