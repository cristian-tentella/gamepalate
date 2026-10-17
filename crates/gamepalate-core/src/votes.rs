use std::collections::BTreeMap;

use crate::ids::AppId;

pub type Votes = BTreeMap<AppId, Vote>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    pub kind: VoteKind,
    pub date: String,
    pub bet: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VoteKind {
    WouldTry,
    Maybe,
    NotForMe,
    Known,
}