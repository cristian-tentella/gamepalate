use std::{path::Path, sync::OnceLock};

use gamepalate_core::{ids::SteamId, timestamp::Timestamp};
use gamepalate_steam::transport::{Get, SteamError, Transport};
use gamepalate_store::{
    config::{ConfigError, save_steam_id},
    signals::SignalsError,
};

use crate::{
    signals::{ReadSignalsError, read_signals, save_signals},
    update::wall_clock,
};

const KEY_ENV_VAR: &str = "STEAM_API_KEY";
const KEY_SERVICE: &str = "gamepalate";
const KEY_USER: &str = "steam-api-key";
const PROFILES: &str = "steamcommunity.com/profiles/";
const VANITY: &str = "steamcommunity.com/id/";

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum SteamIdError {
    #[error("a Steam ID has 17 digits")]
    Invalid,

    #[error("the link has the custom name of the profile, not the Steam ID")]
    CustomName,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("could not open the Windows Credential Manager: {0}")]
    Store(String),

    #[error("could not use the Windows Credential Manager")]
    Entry(#[source] keyring_core::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error(transparent)]
    Read(#[from] ReadSignalsError),

    #[error(transparent)]
    Steam(#[from] SteamError),

    #[error("Steam is not responding: the signals could not be read")]
    SteamDown,

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Key(#[from] KeyError),

    #[error(transparent)]
    Signals(#[from] SignalsError),
}

pub trait KeyStore: Sync {
    fn load(&self) -> Result<Option<String>, KeyError>;
    fn save(&self, key: &str) -> Result<(), KeyError>;
}

pub struct CredentialManager;

impl ConnectError {
    pub fn steam_error(&self) -> Option<&SteamError> {
        match self {
            ConnectError::Read(error) => Some(error.steam_error()),
            ConnectError::Steam(error) => Some(error),
            _ => None,
        }
    }
}

impl CredentialManager {
    fn entry() -> Result<keyring_core::Entry, KeyError> {
        static STORE: OnceLock<Result<(), String>> = OnceLock::new();
        let store = STORE.get_or_init(|| {
            windows_native_keyring_store::Store::new()
                .map(|store| keyring_core::set_default_store(store))
                .map_err(|error| error.to_string())
        });
        if let Err(reason) = store {
            return Err(KeyError::Store(reason.clone()));
        }
        keyring_core::Entry::new(KEY_SERVICE, KEY_USER).map_err(KeyError::Entry)
    }
}

impl KeyStore for CredentialManager {
    fn load(&self) -> Result<Option<String>, KeyError> {
        match CredentialManager::entry()?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(KeyError::Entry(error)),
        }
    }

    fn save(&self, key: &str) -> Result<(), KeyError> {
        CredentialManager::entry()?
            .set_password(key)
            .map_err(KeyError::Entry)
    }
}

pub fn parse_steam_id(text: &str) -> Result<SteamId, SteamIdError> {
    let text = text.trim().trim_end_matches('/');
    let text = ["https://", "http://"]
        .iter()
        .find_map(|scheme| text.strip_prefix(scheme))
        .unwrap_or(text);
    let text = text.strip_prefix("www.").unwrap_or(text);
    if text.starts_with(VANITY) {
        return Err(SteamIdError::CustomName);
    }
    let digits = text.strip_prefix(PROFILES).unwrap_or(text);

    if digits.len() == 17 && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(SteamId(digits.parse().expect("17 digits fit in a u64")))
    } else {
        Err(SteamIdError::Invalid)
    }
}

pub fn steam_api_key() -> Option<String> {
    api_key(std::env::var(KEY_ENV_VAR).ok(), &CredentialManager)
}

pub async fn connect(
    data_dir: &Path,
    steam_id: SteamId,
    key: Option<String>,
) -> Result<(), ConnectError> {
    let transport = Transport::new(key.clone().or_else(steam_api_key))?;
    connect_with(
        data_dir,
        &transport,
        steam_id,
        key.as_deref(),
        &CredentialManager,
        wall_clock(),
    )
    .await
}

fn api_key(env: Option<String>, store: &dyn KeyStore) -> Option<String> {
    env.filter(|key| !key.trim().is_empty())
        .or_else(|| store.load().ok().flatten())
}

async fn connect_with<S: Get>(
    data_dir: &Path,
    steam: &S,
    steam_id: SteamId,
    key: Option<&str>,
    keys: &dyn KeyStore,
    now: Timestamp,
) -> Result<(), ConnectError> {
    let read = read_signals(data_dir, steam, steam_id, now).await?;
    if !read.fresh {
        return Err(ConnectError::SteamDown);
    }

    save_steam_id(&data_dir.join("config.toml"), steam_id)?;
    if let Some(key) = key {
        keys.save(key)?;
    }
    save_signals(data_dir, &read)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Mutex};

    use gamepalate_store::{config::load_config, signals::load_reading};
    use serde_json::{Value, json};

    use super::*;

    const ID: SteamId = SteamId(76_561_198_000_000_000);
    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    #[derive(Default)]
    struct FakeKeys {
        key: Mutex<Option<String>>,
    }

    impl KeyStore for FakeKeys {
        fn load(&self) -> Result<Option<String>, KeyError> {
            Ok(self.key.lock().unwrap().clone())
        }

        fn save(&self, key: &str) -> Result<(), KeyError> {
            *self.key.lock().unwrap() = Some(key.to_string());
            Ok(())
        }
    }

    enum Answer {
        Library,
        Hidden,
        Rejected,
        Down,
    }

    struct FakeSteam(Answer);

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            match (&self.0, endpoint) {
                (Answer::Down, _) => Err(SteamError::SteamDown {
                    endpoint: endpoint.to_string(),
                    reason: "timed out".to_string(),
                    source: None,
                }),
                (Answer::Rejected, "IPlayerService/GetOwnedGames/v1") => {
                    Err(SteamError::KeyRejected {
                        endpoint: endpoint.to_string(),
                        status: http::StatusCode::FORBIDDEN,
                    })
                }
                (Answer::Hidden, "IPlayerService/GetOwnedGames/v1") => {
                    Ok(json!({ "response": {} }))
                }
                (_, "IPlayerService/GetOwnedGames/v1") => Ok(json!({ "response": {
                    "games": [{ "appid": 10, "playtime_forever": 120 }]
                } })),
                _ => Ok(json!({ "response": {} })),
            }
        }
    }

    async fn connect_to(dir: &Path, answer: Answer, keys: &FakeKeys) -> Result<(), ConnectError> {
        connect_with(dir, &FakeSteam(answer), ID, Some("KEY"), keys, NOW).await
    }

    #[test]
    fn seventeen_digits_are_a_steam_id() {
        assert_eq!(parse_steam_id("76561198000000000"), Ok(ID));
        assert_eq!(parse_steam_id("  76561198000000000 "), Ok(ID));
    }

    #[test]
    fn the_profile_link_gives_the_steam_id() {
        for link in [
            "https://steamcommunity.com/profiles/76561198000000000/",
            "http://www.steamcommunity.com/profiles/76561198000000000",
            "steamcommunity.com/profiles/76561198000000000",
        ] {
            assert_eq!(parse_steam_id(link), Ok(ID), "{link}");
        }
    }

    #[test]
    fn other_texts_are_not_a_steam_id() {
        for text in [
            "7656119800000000",
            "7656119800000000a",
            "",
            "steamcommunity.com/profiles/",
        ] {
            assert_eq!(parse_steam_id(text), Err(SteamIdError::Invalid), "{text}");
        }
        assert_eq!(
            parse_steam_id("https://steamcommunity.com/id/someone/"),
            Err(SteamIdError::CustomName)
        );
    }

    #[test]
    fn the_environment_wins_over_the_saved_key() {
        let keys = FakeKeys::default();
        keys.save("SAVED").unwrap();

        assert_eq!(
            api_key(Some("ENV".to_string()), &keys).as_deref(),
            Some("ENV")
        );
        assert_eq!(
            api_key(Some(" ".to_string()), &keys).as_deref(),
            Some("SAVED")
        );
        assert_eq!(api_key(None, &keys).as_deref(), Some("SAVED"));
        assert_eq!(api_key(None, &FakeKeys::default()), None);
    }

    #[tokio::test]
    async fn connecting_saves_the_steam_id_the_key_and_the_signals() {
        let dir = tempfile::tempdir().unwrap();
        let keys = FakeKeys::default();

        connect_to(dir.path(), Answer::Library, &keys)
            .await
            .unwrap();

        assert_eq!(
            load_config(&dir.path().join("config.toml"))
                .unwrap()
                .steam_id,
            Some(ID)
        );
        assert_eq!(keys.load().unwrap().as_deref(), Some("KEY"));
        assert_eq!(
            load_reading(&dir.path().join("signals.json"))
                .unwrap()
                .read_at,
            NOW
        );
    }

    #[tokio::test]
    async fn without_a_new_key_the_saved_one_stays() {
        let dir = tempfile::tempdir().unwrap();
        let keys = FakeKeys::default();
        keys.save("SAVED").unwrap();

        connect_with(
            dir.path(),
            &FakeSteam(Answer::Library),
            ID,
            None,
            &keys,
            NOW,
        )
        .await
        .unwrap();

        assert_eq!(keys.load().unwrap().as_deref(), Some("SAVED"));
    }

    #[tokio::test]
    async fn a_failed_connection_writes_nothing() {
        for answer in [Answer::Hidden, Answer::Rejected, Answer::Down] {
            let dir = tempfile::tempdir().unwrap();
            let keys = FakeKeys::default();

            let error = connect_to(dir.path(), answer, &keys).await.unwrap_err();

            assert!(error.steam_error().is_some(), "{error:?}");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
            assert_eq!(keys.load().unwrap(), None);
        }
    }

    #[tokio::test]
    async fn the_errors_are_recognized() {
        let dir = tempfile::tempdir().unwrap();
        let keys = FakeKeys::default();

        let hidden = connect_to(dir.path(), Answer::Hidden, &keys)
            .await
            .unwrap_err();
        let rejected = connect_to(dir.path(), Answer::Rejected, &keys)
            .await
            .unwrap_err();

        assert!(matches!(
            hidden.steam_error(),
            Some(SteamError::LibraryHidden { .. })
        ));
        assert!(matches!(
            rejected.steam_error(),
            Some(SteamError::KeyRejected { .. })
        ));
    }

    #[tokio::test]
    async fn steam_down_with_saved_signals_is_not_a_connection() {
        let dir = tempfile::tempdir().unwrap();
        let signals = dir.path().join("signals.json");
        fs::write(
            &signals,
            r#"{"readAt":1793000000000,"library":[{"appid":1,"totalHours":3.0}],"recent":[],"wishlist":[],"followed":null}"#,
        )
        .unwrap();
        let keys = FakeKeys::default();

        let error = connect_to(dir.path(), Answer::Down, &keys)
            .await
            .unwrap_err();

        assert!(matches!(error, ConnectError::SteamDown));
        assert!(!dir.path().join("config.toml").exists());
        assert_eq!(keys.load().unwrap(), None);
    }
}
