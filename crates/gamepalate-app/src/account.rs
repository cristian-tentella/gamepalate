use std::{path::Path, sync::OnceLock};

use gamepalate_core::{ids::SteamId, timestamp::Timestamp};
use gamepalate_steam::{
    signals::resolve_vanity_url,
    transport::{Get, SteamError, Transport},
};
use gamepalate_store::{
    config::{ConfigError, load_config, save_steam_id},
    signals::SignalsError,
};

use crate::{
    log::failed,
    signals::{ReadSignalsError, SteamClientHours, read_signals, save_signals, steam_client_hours},
    update::wall_clock,
};

pub(crate) const KEY_ENV_VAR: &str = "STEAM_API_KEY";
const KEY_SERVICE: &str = "gamepalate";
const KEY_USER: &str = "steam-api-key";

#[cfg(windows)]
const KEY_STORE_NAME: &str = "the Windows Credential Manager";
#[cfg(not(windows))]
const KEY_STORE_NAME: &str = "the system keyring";

#[cfg(windows)]
fn open_system_store() -> Result<(), String> {
    windows_native_keyring_store::Store::new()
        .map(|store| keyring_core::set_default_store(store))
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn open_system_store() -> Result<(), String> {
    dbus_secret_service_keyring_store::Store::new()
        .map(|store| keyring_core::set_default_store(store))
        .map_err(|error| error.to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_system_store() -> Result<(), String> {
    Err("this system has no supported keyring".to_string())
}
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
    #[error("could not open {name}: {0}", name = KEY_STORE_NAME)]
    Store(String),

    #[error("could not use {name}", name = KEY_STORE_NAME)]
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

    #[error("Steam has no profile with the custom name {0}")]
    NoProfile(String),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Key(#[from] KeyError),

    #[error(transparent)]
    Signals(#[from] SignalsError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Profile {
    Id(SteamId),
    CustomName(String),
}

pub trait KeyStore: Sync {
    fn load(&self) -> Result<Option<String>, KeyError>;
    fn save(&self, key: &str) -> Result<(), KeyError>;
}

pub struct SystemKeyring;

impl ConnectError {
    pub fn steam_error(&self) -> Option<&SteamError> {
        match self {
            ConnectError::Read(error) => Some(error.steam_error()),
            ConnectError::Steam(error) => Some(error),
            _ => None,
        }
    }
}

impl SystemKeyring {
    fn entry() -> Result<keyring_core::Entry, KeyError> {
        static STORE: OnceLock<Result<(), String>> = OnceLock::new();
        let store = STORE.get_or_init(open_system_store);
        if let Err(reason) = store {
            return Err(KeyError::Store(reason.clone()));
        }
        keyring_core::Entry::new(KEY_SERVICE, KEY_USER).map_err(KeyError::Entry)
    }
}

impl KeyStore for SystemKeyring {
    fn load(&self) -> Result<Option<String>, KeyError> {
        match SystemKeyring::entry()?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(KeyError::Entry(error)),
        }
    }

    fn save(&self, key: &str) -> Result<(), KeyError> {
        SystemKeyring::entry()?
            .set_password(key)
            .map_err(KeyError::Entry)
    }
}

pub fn parse_steam_id(text: &str) -> Result<SteamId, SteamIdError> {
    let text = profile_path(text);
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

pub fn parse_profile(text: &str) -> Result<Profile, SteamIdError> {
    match custom_name(text) {
        Some(name) => Ok(Profile::CustomName(name.to_string())),
        None => parse_steam_id(text).map(Profile::Id),
    }
}

fn custom_name(text: &str) -> Option<&str> {
    let name = profile_path(text).strip_prefix(VANITY)?;
    let name = name.split(['/', '?', '#']).next()?;
    (!name.is_empty()).then_some(name)
}

fn profile_path(text: &str) -> &str {
    let text = text.trim().trim_end_matches('/');
    let text = ["https://", "http://"]
        .iter()
        .find_map(|scheme| text.strip_prefix(scheme))
        .unwrap_or(text);
    text.strip_prefix("www.").unwrap_or(text)
}

pub fn steam_api_key() -> Option<String> {
    api_key(std::env::var(KEY_ENV_VAR).ok(), &SystemKeyring)
}

pub async fn connect(
    data_dir: &Path,
    profile: Profile,
    key: Option<String>,
) -> Result<(), ConnectError> {
    let transport = Transport::new(key.clone().or_else(steam_api_key))?;
    connect_with(
        data_dir,
        &transport,
        profile,
        key.as_deref(),
        &SystemKeyring,
        wall_clock(),
    )
    .await
    .inspect_err(failed("could not connect to Steam"))
}

fn api_key(env: Option<String>, store: &dyn KeyStore) -> Option<String> {
    env.filter(|key| !key.trim().is_empty())
        .or_else(|| store.load().ok().flatten())
}

async fn connect_with<S: Get>(
    data_dir: &Path,
    steam: &S,
    profile: Profile,
    key: Option<&str>,
    keys: &dyn KeyStore,
    now: Timestamp,
) -> Result<(), ConnectError> {
    let steam_id = match profile {
        Profile::Id(steam_id) => steam_id,
        Profile::CustomName(name) => resolve_vanity_url(steam, &name)
            .await?
            .ok_or(ConnectError::NoProfile(name))?,
    };
    let client = load_config(&data_dir.join("config.toml"))
        .map_or(SteamClientHours::Off, |config| {
            steam_client_hours(&config, steam_id)
        });
    let read = read_signals(data_dir, steam, steam_id, &client, now).await?;
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

    use gamepalate_store::signals::load_reading;
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
            params: &[(&str, &str)],
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
                (_, "ISteamUser/ResolveVanityURL/v1") => {
                    Ok(if params.contains(&("vanityurl", "someone")) {
                        json!({ "response": { "steamid": "76561198000000000", "success": 1 } })
                    } else {
                        json!({ "response": { "success": 42, "message": "No match" } })
                    })
                }
                (_, "IPlayerService/GetOwnedGames/v1") => Ok(json!({ "response": {
                    "games": [{ "appid": 10, "playtime_forever": 120 }]
                } })),
                _ => Ok(json!({ "response": {} })),
            }
        }
    }

    async fn connect_to(dir: &Path, answer: Answer, keys: &FakeKeys) -> Result<(), ConnectError> {
        connect_with(
            dir,
            &FakeSteam(answer),
            Profile::Id(ID),
            Some("KEY"),
            keys,
            NOW,
        )
        .await
    }

    #[test]
    fn the_custom_name_comes_from_the_profile_link() {
        assert_eq!(
            custom_name("https://steamcommunity.com/id/someone/"),
            Some("someone")
        );
        assert_eq!(
            custom_name("www.steamcommunity.com/id/someone/games?tab=all"),
            Some("someone")
        );
        assert_eq!(custom_name("76561198000000000"), None);
        assert_eq!(
            custom_name("https://steamcommunity.com/profiles/76561198000000000"),
            None
        );
    }

    #[test]
    fn a_profile_is_a_custom_name_or_a_steam_id() {
        assert_eq!(
            parse_profile("https://steamcommunity.com/id/someone/"),
            Ok(Profile::CustomName("someone".to_string()))
        );
        assert_eq!(
            parse_profile("steamcommunity.com/profiles/76561198000000000"),
            Ok(Profile::Id(ID))
        );
        assert_eq!(parse_profile("123"), Err(SteamIdError::Invalid));
    }

    #[tokio::test]
    async fn a_custom_name_is_saved_as_its_steam_id() {
        let dir = tempfile::tempdir().unwrap();

        connect_with(
            dir.path(),
            &FakeSteam(Answer::Library),
            Profile::CustomName("someone".to_string()),
            Some("KEY"),
            &FakeKeys::default(),
            NOW,
        )
        .await
        .unwrap();

        assert_eq!(
            load_config(&dir.path().join("config.toml"))
                .unwrap()
                .steam_id,
            Some(ID)
        );
    }

    #[tokio::test]
    async fn an_unknown_custom_name_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();

        let error = connect_with(
            dir.path(),
            &FakeSteam(Answer::Library),
            Profile::CustomName("nobody".to_string()),
            Some("KEY"),
            &FakeKeys::default(),
            NOW,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, ConnectError::NoProfile(name) if name == "nobody"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
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
            Profile::Id(ID),
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
