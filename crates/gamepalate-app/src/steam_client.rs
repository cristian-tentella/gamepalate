use std::path::{Path, PathBuf};

pub use gamepalate_steam::client::{
    ClientAccount, local_config_shown, login_users_file, steam_client_present,
};
use gamepalate_steam::client::{
    SteamClientError, read_played_hours, steam_accounts, steam_client_file,
};
use gamepalate_store::{
    config::{
        ConfigError, load_config, load_steam_client_accounts, save_steam_client_accounts,
        save_steam_client_hours,
    },
    signals::{SignalsError, load_reading, save_reading},
};

use crate::{log::failed, signals::not_owned};

const CONFIG_FILE: &str = "config.toml";
const SIGNALS_FILE: &str = "signals.json";

#[derive(Debug, thiserror::Error)]
pub enum SteamClientAnswerError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Signals(#[from] SignalsError),

    #[error(transparent)]
    SteamClient(#[from] SteamClientError),
}

pub fn steam_client_question(data_dir: &Path) -> Option<PathBuf> {
    let config = load_config(&data_dir.join(CONFIG_FILE)).ok()?;
    if config.signals.steam_client_hours.is_some() {
        return None;
    }
    steam_client_file(config.steam_id?)
}

pub fn steam_client_file_of(data_dir: &Path) -> Option<PathBuf> {
    steam_client_file(load_config(&data_dir.join(CONFIG_FILE)).ok()?.steam_id?)
}

pub fn answer_steam_client(data_dir: &Path, read: bool) -> Result<(), SteamClientAnswerError> {
    let file = read.then(|| steam_client_file_of(data_dir)).flatten();
    answer_with(data_dir, read, file.as_deref())
        .inspect_err(failed("could not save the Steam app answer"))
}

pub fn steam_accounts_answer(data_dir: &Path) -> Option<bool> {
    load_steam_client_accounts(&data_dir.join(CONFIG_FILE))
}

pub fn answer_steam_accounts(data_dir: &Path, read: bool) -> Result<(), ConfigError> {
    save_steam_client_accounts(&data_dir.join(CONFIG_FILE), read)
        .inspect_err(failed("could not save the Steam accounts answer"))
}

pub(crate) fn accounts_if_allowed(
    data_dir: &Path,
    read: impl FnOnce() -> Vec<ClientAccount>,
) -> Vec<ClientAccount> {
    if steam_accounts_answer(data_dir) == Some(true) {
        read()
    } else {
        Vec::new()
    }
}

pub(crate) fn client_accounts(data_dir: &Path) -> Vec<ClientAccount> {
    accounts_if_allowed(data_dir, steam_accounts)
}

fn answer_with(
    data_dir: &Path,
    read: bool,
    file: Option<&Path>,
) -> Result<(), SteamClientAnswerError> {
    let borrowed = match file {
        Some(file) if read => Some(read_played_hours(file)?),
        _ => None,
    };
    save_steam_client_hours(&data_dir.join(CONFIG_FILE), read)?;

    let path = data_dir.join(SIGNALS_FILE);
    let Ok(mut reading) = load_reading(&path) else {
        return Ok(());
    };
    reading.borrowed = borrowed
        .map(|played| not_owned(played, &reading.library))
        .unwrap_or_default();
    save_reading(&path, &reading)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_core::{
        ids::{AppId, SteamId},
        signals::LibraryGame,
    };

    use super::*;

    const CONFIG: &str = "steam_id = \"76561198000000000\"\n";
    const SIGNALS: &str = r#"{"readAt":1,"library":[{"appid":10,"totalHours":2.0}],"recent":[],"wishlist":null,"followed":null}"#;
    const LOCAL_CONFIG: &str = r#""UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" {
        "10" { "Playtime" "600" }
        "70" { "Playtime" "90" }
    } } } } }"#;

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), CONFIG).unwrap();
        fs::write(dir.path().join(SIGNALS_FILE), SIGNALS).unwrap();
        fs::write(dir.path().join("localconfig.vdf"), LOCAL_CONFIG).unwrap();
        dir
    }

    fn answer(dir: &Path) -> Option<bool> {
        load_config(&dir.join(CONFIG_FILE))
            .unwrap()
            .signals
            .steam_client_hours
    }

    #[test]
    fn a_yes_saves_the_answer_and_reads_the_games_not_owned() {
        let dir = folder();
        let file = dir.path().join("localconfig.vdf");

        answer_with(dir.path(), true, Some(&file)).unwrap();

        assert_eq!(answer(dir.path()), Some(true));
        assert_eq!(
            load_reading(&dir.path().join(SIGNALS_FILE))
                .unwrap()
                .borrowed,
            [LibraryGame {
                id: AppId(70),
                total_hours: 1.5,
                last_played: None,
            }]
        );
    }

    #[test]
    fn a_no_after_a_yes_drops_the_games() {
        let dir = folder();
        let file = dir.path().join("localconfig.vdf");
        answer_with(dir.path(), true, Some(&file)).unwrap();

        answer_with(dir.path(), false, Some(&file)).unwrap();

        assert_eq!(answer(dir.path()), Some(false));
        assert!(
            load_reading(&dir.path().join(SIGNALS_FILE))
                .unwrap()
                .borrowed
                .is_empty()
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_saves_nothing() {
        let dir = folder();
        let missing = dir.path().join("missing.vdf");

        let error = answer_with(dir.path(), true, Some(&missing)).unwrap_err();

        assert!(matches!(error, SteamClientAnswerError::SteamClient(_)));
        assert_eq!(answer(dir.path()), None);
    }

    fn account() -> ClientAccount {
        ClientAccount {
            steam_id: SteamId(76_561_198_000_000_000),
            persona_name: None,
            most_recent: true,
            avatar: None,
        }
    }

    #[test]
    fn the_accounts_answer_is_saved_and_read_back() {
        let dir = folder();
        assert_eq!(steam_accounts_answer(dir.path()), None);

        answer_steam_accounts(dir.path(), true).unwrap();

        assert_eq!(steam_accounts_answer(dir.path()), Some(true));
        assert_eq!(answer(dir.path()), None);
        answer_steam_accounts(dir.path(), false).unwrap();
        assert_eq!(steam_accounts_answer(dir.path()), Some(false));
    }

    #[test]
    fn the_accounts_answer_is_saved_before_the_steam_id() {
        let dir = tempfile::tempdir().unwrap();

        answer_steam_accounts(dir.path(), true).unwrap();

        assert_eq!(steam_accounts_answer(dir.path()), Some(true));
    }

    #[test]
    fn without_the_answer_or_with_a_no_the_accounts_are_not_read() {
        let dir = folder();
        let read = || vec![account()];

        assert!(accounts_if_allowed(dir.path(), read).is_empty());
        answer_steam_accounts(dir.path(), false).unwrap();
        assert!(accounts_if_allowed(dir.path(), read).is_empty());
        let nothing = tempfile::tempdir().unwrap();
        assert!(accounts_if_allowed(nothing.path(), read).is_empty());
    }

    #[test]
    fn with_a_yes_the_accounts_are_read() {
        let dir = folder();
        answer_steam_accounts(dir.path(), true).unwrap();

        assert_eq!(
            accounts_if_allowed(dir.path(), || vec![account()]),
            [account()]
        );
    }

    #[test]
    fn an_answer_before_the_signals_only_saves_the_config() {
        let dir = folder();
        fs::remove_file(dir.path().join(SIGNALS_FILE)).unwrap();

        answer_with(dir.path(), false, None).unwrap();

        assert_eq!(answer(dir.path()), Some(false));
        assert!(!dir.path().join(SIGNALS_FILE).exists());
    }
}
