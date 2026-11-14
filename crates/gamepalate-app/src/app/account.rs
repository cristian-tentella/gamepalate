use super::Gamepalate;
use crate::steam_account::{SteamAccount, steam_account};

impl Gamepalate {
    pub async fn steam_account(&self) -> Option<SteamAccount> {
        let steam_id = self.files.config.steam_id?;
        Some(steam_account(steam_id, &self.images).await)
    }
}
