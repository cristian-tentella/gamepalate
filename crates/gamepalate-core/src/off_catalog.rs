use std::collections::BTreeMap;

use crate::{game::GameTag, ids::AppId};

pub type OffCatalogGames = BTreeMap<AppId, OffCatalogGame>;

#[derive(Debug, Clone, PartialEq)]
pub struct OffCatalogGame {
    pub tags: Vec<GameTag>,
    pub name: Option<String>,
}