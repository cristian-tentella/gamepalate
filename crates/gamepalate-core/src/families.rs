use crate::ids::AppId;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Families {
    pub families: Vec<Family>,
    pub held_out: Vec<AppId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub name: String,
    pub games: Vec<AppId>,
}
