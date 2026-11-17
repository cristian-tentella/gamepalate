use std::{
    io,
    path::{Path, PathBuf},
};

use gamepalate_core::{
    ids::{AppId, SteamId},
    signals::LibraryGame,
    timestamp::Timestamp,
};
use keyvalues_parser::Value;

const ACCOUNT_ID_BASE: u64 = 76_561_197_960_265_728;
const AVATAR_CACHE: [&str; 2] = ["config", "avatarcache"];
const APPS: [&str; 4] = ["Software", "Valve", "Steam", "apps"];

#[derive(Debug, thiserror::Error)]
pub enum SteamClientError {
    #[error("could not read {}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("{} is not a file the Steam app wrote: {reason}", .path.display())]
    Shape { path: PathBuf, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAccount {
    pub steam_id: SteamId,
    pub persona_name: Option<String>,
    pub most_recent: bool,
    pub avatar: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    account: ClientAccount,
    timestamp: i64,
}

pub fn steam_client_present() -> bool {
    steamlocate::locate_all().is_ok_and(|installs| !installs.is_empty())
}

pub fn login_users_file() -> Option<PathBuf> {
    first_install().map(|steam| login_users_path(&steam))
}

pub fn local_config_shown() -> Option<PathBuf> {
    first_install().map(|steam| {
        steam
            .join("userdata")
            .join("\u{2026}")
            .join("config")
            .join("localconfig.vdf")
    })
}

pub fn steam_accounts() -> Vec<ClientAccount> {
    let Ok(installs) = steamlocate::locate_all() else {
        return Vec::new();
    };
    accounts_in(installs.iter().map(|steam| steam.path().to_path_buf()))
}

fn first_install() -> Option<PathBuf> {
    steamlocate::locate_all()
        .ok()?
        .first()
        .map(|steam| steam.path().to_path_buf())
}

fn login_users_path(steam: &Path) -> PathBuf {
    steam.join("config").join("loginusers.vdf")
}

fn accounts_in(installs: impl Iterator<Item = PathBuf>) -> Vec<ClientAccount> {
    let mut entries: Vec<Entry> = installs
        .flat_map(|steam| {
            let path = login_users_path(&steam);
            let entries = read_entries(&path).unwrap_or_else(|error| {
                log::warn!("could not read the Steam accounts: {error}");
                Vec::new()
            });
            entries.into_iter().map(move |mut entry| {
                entry.account.avatar = avatar_file(&steam, entry.account.steam_id);
                entry
            })
        })
        .collect();
    sort_entries(&mut entries);
    entries.dedup_by_key(|entry| entry.account.steam_id.0);
    entries.into_iter().map(|entry| entry.account).collect()
}

fn avatar_file(steam: &Path, steam_id: SteamId) -> Option<PathBuf> {
    let path = AVATAR_CACHE
        .iter()
        .fold(steam.to_path_buf(), |path, part| path.join(part))
        .join(format!("{}.png", steam_id.0));
    path.is_file().then_some(path)
}

fn read_entries(path: &Path) -> Result<Vec<Entry>, SteamClientError> {
    let bytes = std::fs::read(path).map_err(|source| SteamClientError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_accounts(&String::from_utf8_lossy(&bytes)).map_err(|reason| SteamClientError::Shape {
        path: path.to_path_buf(),
        reason,
    })
}

fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.account
            .most_recent
            .cmp(&a.account.most_recent)
            .then(b.timestamp.cmp(&a.timestamp))
            .then(a.account.steam_id.0.cmp(&b.account.steam_id.0))
    });
}

fn parse_accounts(text: &str) -> Result<Vec<Entry>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let vdf = keyvalues_parser::parse(text).map_err(|error| error.to_string())?;
    let Some(users) = vdf.value.get_obj() else {
        return Ok(Vec::new());
    };
    let mut entries: Vec<Entry> = users
        .iter()
        .filter_map(|(id, values)| {
            let steam_id = SteamId(id.parse().ok()?);
            let user = values.first()?;
            let text = |key| child(user, key).and_then(Value::get_str);
            Some(Entry {
                account: ClientAccount {
                    steam_id,
                    persona_name: text("PersonaName")
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string),
                    most_recent: text("MostRecent") == Some("1"),
                    avatar: None,
                },
                timestamp: text("Timestamp")
                    .and_then(|seconds| seconds.parse().ok())
                    .unwrap_or(0),
            })
        })
        .collect();
    sort_entries(&mut entries);
    Ok(entries)
}

pub fn steam_client_file(steam_id: SteamId) -> Option<PathBuf> {
    let installs = steamlocate::locate_all().ok()?;
    newest_file(
        installs
            .iter()
            .filter_map(|steam| local_config_path(steam.path(), steam_id)),
    )
}

fn newest_file(paths: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    paths
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

pub fn read_played_hours(path: &Path) -> Result<Vec<LibraryGame>, SteamClientError> {
    let bytes = std::fs::read(path).map_err(|source| SteamClientError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_played_hours(&String::from_utf8_lossy(&bytes)).map_err(|reason| SteamClientError::Shape {
        path: path.to_path_buf(),
        reason,
    })
}

pub fn read_persona_name(path: &Path) -> Result<Option<String>, SteamClientError> {
    let bytes = std::fs::read(path).map_err(|source| SteamClientError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_persona_name(&String::from_utf8_lossy(&bytes)).map_err(|reason| SteamClientError::Shape {
        path: path.to_path_buf(),
        reason,
    })
}

fn local_config_path(steam: &Path, steam_id: SteamId) -> Option<PathBuf> {
    let account_id = steam_id.0.checked_sub(ACCOUNT_ID_BASE)?;
    Some(
        steam
            .join("userdata")
            .join(account_id.to_string())
            .join("config")
            .join("localconfig.vdf"),
    )
}

fn parse_persona_name(text: &str) -> Result<Option<String>, String> {
    let vdf = keyvalues_parser::parse(text).map_err(|error| error.to_string())?;
    Ok(child(&vdf.value, "friends")
        .and_then(|friends| child(friends, "PersonaName"))
        .and_then(Value::get_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string))
}

fn parse_played_hours(text: &str) -> Result<Vec<LibraryGame>, String> {
    let vdf = keyvalues_parser::parse(text).map_err(|error| error.to_string())?;
    let Some(apps) = APPS
        .iter()
        .try_fold(&vdf.value, |value, key| child(value, key))
        .and_then(Value::get_obj)
    else {
        return Ok(Vec::new());
    };

    Ok(apps
        .iter()
        .filter_map(|(appid, values)| {
            let id = AppId(appid.parse().ok()?);
            let minutes: u64 = child(values.first()?, "Playtime")?
                .get_str()?
                .parse()
                .ok()?;
            let last_played = child(values.first()?, "LastPlayed")
                .and_then(Value::get_str)
                .and_then(|seconds| seconds.parse::<i64>().ok())
                .filter(|seconds| *seconds > 0)
                .map(|seconds| Timestamp(seconds * 1000));
            (minutes > 0).then(|| LibraryGame {
                id,
                total_hours: minutes as f64 / 60.0,
                last_played,
            })
        })
        .collect())
}

fn child<'a>(value: &'a Value<'a>, key: &str) -> Option<&'a Value<'a>> {
    value
        .get_obj()?
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .and_then(|(_, values)| values.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_CONFIG: &str = r#"
"UserLocalConfigStore"
{
	"friends"
	{
		"PersonaName"		"someone \"quoted\""
	}
	"Software"
	{
		"valve"
		{
			"Steam"
			{
				"apps"
				{
					"1086940"
					{
						"LastPlayed"		"1771705394"
						"Playtime"		"8866"
						"cloud"
						{
							"last_sync_state"		"synchronized"
						}
					}
					"218"
					{
						"LastPlayed"		"1689629625"
					}
					"440"
					{
						"Playtime"		"0"
					}
					"not-an-app"
					{
						"Playtime"		"30"
					}
				}
				"InstallDir"		"C:\\Program Files (x86)\\Steam"
			}
		}
	}
}
"#;

    #[test]
    fn reads_the_hours_of_every_played_app() {
        assert_eq!(
            parse_played_hours(LOCAL_CONFIG).unwrap(),
            [LibraryGame {
                id: AppId(1_086_940),
                total_hours: 8866.0 / 60.0,
                last_played: Some(Timestamp(1_771_705_394_000)),
            }]
        );
    }

    #[test]
    fn an_app_without_a_last_played_date_has_none() {
        let text = r#"
"UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" {
    "70" { "Playtime" "90" }
    "80" { "Playtime" "30" "LastPlayed" "0" }
} } } } }"#;

        let games = parse_played_hours(text).unwrap();

        assert_eq!(games.len(), 2);
        assert!(games.iter().all(|game| game.last_played.is_none()));
    }

    #[test]
    fn a_file_without_apps_has_no_hours() {
        let text = r#""UserLocalConfigStore" { "friends" { "PersonaName" "someone" } }"#;

        assert_eq!(parse_played_hours(text).unwrap(), []);
    }

    #[test]
    fn a_broken_file_is_a_shape_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("localconfig.vdf");
        std::fs::write(&path, r#""UserLocalConfigStore" { "Software" "#).unwrap();

        let error = read_played_hours(&path).unwrap_err();

        assert!(matches!(error, SteamClientError::Shape { .. }));
        assert!(error.to_string().contains("localconfig.vdf"));
    }

    #[test]
    fn a_missing_file_is_an_io_error() {
        let error = read_played_hours(Path::new("does-not-exist/localconfig.vdf")).unwrap_err();

        assert!(matches!(error, SteamClientError::Io { .. }));
    }

    #[test]
    fn with_two_installs_the_newest_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.vdf");
        let new = dir.path().join("new.vdf");
        std::fs::write(&old, "").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        std::fs::write(&new, "").unwrap();
        let missing = dir.path().join("missing.vdf");

        assert_eq!(
            newest_file([old, missing.clone(), new.clone()].into_iter()),
            Some(new)
        );
        assert_eq!(newest_file([missing].into_iter()), None);
    }

    #[test]
    fn reads_the_persona_name() {
        assert_eq!(
            parse_persona_name(LOCAL_CONFIG).unwrap().as_deref(),
            Some("someone \"quoted\"")
        );
    }

    #[test]
    fn no_persona_name_or_an_empty_one_is_none() {
        let without = r#""UserLocalConfigStore" { "friends" { "Other" "x" } }"#;
        let empty = r#""UserLocalConfigStore" { "friends" { "PersonaName" "  " } }"#;
        let no_friends = r#""UserLocalConfigStore" { "Software" { } }"#;

        assert_eq!(parse_persona_name(without).unwrap(), None);
        assert_eq!(parse_persona_name(empty).unwrap(), None);
        assert_eq!(parse_persona_name(no_friends).unwrap(), None);
    }

    #[test]
    fn the_persona_name_is_read_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("localconfig.vdf");
        std::fs::write(&path, LOCAL_CONFIG).unwrap();

        assert_eq!(
            read_persona_name(&path).unwrap().as_deref(),
            Some("someone \"quoted\"")
        );
        std::fs::write(&path, r#""UserLocalConfigStore" { "friends" "#).unwrap();
        assert!(matches!(
            read_persona_name(&path).unwrap_err(),
            SteamClientError::Shape { .. }
        ));
        assert!(matches!(
            read_persona_name(&dir.path().join("missing.vdf")).unwrap_err(),
            SteamClientError::Io { .. }
        ));
    }

    const LOGIN_USERS: &str = r#"
"users"
{
	"76561198000000002"
	{
		"AccountName"		"secret_login_two"
		"PersonaName"		"Second"
		"RememberPassword"		"1"
		"MostRecent"		"0"
		"Timestamp"		"1700000000"
	}
	"76561198000000001"
	{
		"AccountName"		"secret_login_one"
		"PersonaName"		"First"
		"MostRecent"		"1"
		"Timestamp"		"1600000000"
	}
	"76561198000000003"
	{
		"AccountName"		"secret_login_three"
		"PersonaName"		"Third"
		"MostRecent"		"0"
		"Timestamp"		"1800000000"
	}
}
"#;

    fn ids(entries: &[Entry]) -> Vec<u64> {
        entries
            .iter()
            .map(|entry| entry.account.steam_id.0)
            .collect()
    }

    fn install(login_users: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(
            dir.path().join("config").join("loginusers.vdf"),
            login_users,
        )
        .unwrap();
        dir
    }

    #[test]
    fn reads_one_account() {
        let text = r#""users" { "76561198000000001" { "AccountName" "x" "PersonaName" "Player" "MostRecent" "1" "Timestamp" "5" } }"#;

        let entries = parse_accounts(text).unwrap();

        assert_eq!(
            entries,
            [Entry {
                account: ClientAccount {
                    steam_id: SteamId(76_561_198_000_000_001),
                    persona_name: Some("Player".to_string()),
                    most_recent: true,
                    avatar: None,
                },
                timestamp: 5,
            }]
        );
    }

    #[test]
    fn the_most_recent_account_comes_first_then_the_newest() {
        let entries = parse_accounts(LOGIN_USERS).unwrap();

        assert_eq!(
            ids(&entries),
            [
                76_561_198_000_000_001,
                76_561_198_000_000_003,
                76_561_198_000_000_002
            ]
        );
    }

    #[test]
    fn without_a_most_recent_the_timestamp_decides() {
        let text = LOGIN_USERS.replace("\"MostRecent\"\t\t\"1\"", "\"MostRecent\"\t\t\"0\"");

        let entries = parse_accounts(&text).unwrap();

        assert!(entries.iter().all(|entry| !entry.account.most_recent));
        assert_eq!(
            ids(&entries),
            [
                76_561_198_000_000_003,
                76_561_198_000_000_002,
                76_561_198_000_000_001
            ]
        );
    }

    #[test]
    fn the_steam_id_breaks_a_tie() {
        let text = r#""users" { "76561198000000009" { "Timestamp" "1" } "76561198000000004" { "Timestamp" "1" } }"#;

        assert_eq!(
            ids(&parse_accounts(text).unwrap()),
            [76_561_198_000_000_004, 76_561_198_000_000_009]
        );
    }

    #[test]
    fn an_account_without_a_persona_name_has_none() {
        let text = r#""users" { "76561198000000001" { "AccountName" "x" "PersonaName" " " } "76561198000000002" { "MostRecent" "1" } }"#;

        let entries = parse_accounts(text).unwrap();

        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry.account.persona_name.is_none())
        );
    }

    #[test]
    fn an_empty_file_has_no_accounts() {
        assert_eq!(parse_accounts("").unwrap(), []);
        assert_eq!(parse_accounts("  \n").unwrap(), []);
        assert_eq!(parse_accounts(r#""users" { }"#).unwrap(), []);
    }

    #[test]
    fn a_broken_file_is_an_error() {
        assert!(parse_accounts(r#""users" { "76561198000000001" "#).is_err());
    }

    #[test]
    fn the_login_name_is_not_kept() {
        let entries = parse_accounts(LOGIN_USERS).unwrap();

        assert!(!format!("{entries:?}").contains("secret_login"));
    }

    #[test]
    fn a_repeated_steam_id_in_two_installs_counts_once() {
        let first = install(LOGIN_USERS);
        let second = install(
            r#""users" { "76561198000000001" { "PersonaName" "Other name" "MostRecent" "1" "Timestamp" "9" } "76561198000000005" { "Timestamp" "1" } }"#,
        );

        let accounts =
            accounts_in([first.path().to_path_buf(), second.path().to_path_buf()].into_iter());

        let ids: Vec<u64> = accounts.iter().map(|account| account.steam_id.0).collect();
        assert_eq!(
            ids,
            [
                76_561_198_000_000_001,
                76_561_198_000_000_003,
                76_561_198_000_000_002,
                76_561_198_000_000_005
            ]
        );
    }

    #[test]
    fn a_missing_or_broken_file_gives_no_accounts() {
        let empty = tempfile::tempdir().unwrap();
        let broken = install(r#""users" { "7656"#);

        assert_eq!(
            accounts_in([empty.path().to_path_buf(), broken.path().to_path_buf()].into_iter()),
            []
        );
    }

    #[test]
    fn the_avatar_is_the_cached_picture_when_there_is_one() {
        let steam = install(LOGIN_USERS);
        let cache = steam.path().join("config").join("avatarcache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("76561198000000001.png"), "").unwrap();

        let accounts = accounts_in([steam.path().to_path_buf()].into_iter());

        assert_eq!(
            accounts[0].avatar.as_deref(),
            Some(cache.join("76561198000000001.png").as_path())
        );
        assert!(accounts[1..].iter().all(|account| account.avatar.is_none()));
    }

    #[test]
    fn the_login_users_file_is_in_the_config_folder() {
        assert_eq!(
            login_users_path(Path::new("steam")),
            Path::new("steam").join("config").join("loginusers.vdf")
        );
    }

    #[test]
    fn the_file_is_in_the_folder_of_the_account() {
        let path = local_config_path(Path::new("steam"), SteamId(76_561_198_000_000_000));

        assert_eq!(
            path,
            Some(
                Path::new("steam")
                    .join("userdata")
                    .join("39734272")
                    .join("config")
                    .join("localconfig.vdf")
            )
        );
        assert_eq!(local_config_path(Path::new("steam"), SteamId(5)), None);
    }
}
