use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{AppId, TagId};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Corrections {
    pub game_multipliers: BTreeMap<AppId, f64>,
    pub tag_multipliers: BTreeMap<TagId, f64>,
    pub banned_tags: BTreeMap<TagId, Ban>,
    pub hidden_games: BTreeSet<AppId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ban {
    Everywhere,
    TopTags(u32),
}
