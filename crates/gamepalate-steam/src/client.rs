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
