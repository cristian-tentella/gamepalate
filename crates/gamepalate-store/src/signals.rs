use std::path::{Path, PathBuf};

use gamepalate_core::{
    ids::AppId,
    signals::{LibraryGame, RecentGame, Signals, SignalsReading},
    timestamp::Timestamp,
};
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomically;

#[derive(Debug, thiserror::Error)]
pub enum SignalsError {
    #[error("signals not found at {}", .0.display())]
    NotFound(PathBuf),

    #[error("could not read the signals")]
    Io(#[from] std::io::Error),

    #[error("the signals file is not valid")]
    Json(#[from] serde_json::Error),

    #[error("could not save the signals to {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalsFile {
    read_at: i64,
    library: Vec<LibraryEntry>,
    recent: Vec<RecentEntry>,
    wishlist: Option<Vec<u32>>,
    followed: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wishlist_read_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    followed_read_at: Option<i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LibraryEntry {
    appid: u32,
    total_hours: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecentEntry {
    appid: u32,
    recent_hours: f64,
    total_hours: f64,
}

pub fn load_signals(path: &Path) -> Result<Signals, SignalsError> {
    Ok(load_reading(path)?.into_signals())
}

pub fn load_reading(path: &Path) -> Result<SignalsReading, SignalsError> {
    if !path.is_file() {
        return Err(SignalsError::NotFound(path.to_path_buf()));
    }

    let text = std::fs::read_to_string(path)?;

    parse_reading(&text)
}

pub fn save_reading(path: &Path, reading: &SignalsReading) -> Result<(), SignalsError> {
    let text = serde_json::to_string(&signals_file(reading))?;

    write_atomically(path, &text).map_err(|source| SignalsError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_reading(text: &str) -> Result<SignalsReading, SignalsError> {
    let file: SignalsFile = serde_json::from_str(text)?;

    Ok(SignalsReading {
        read_at: Timestamp(file.read_at),
        library: file
            .library
            .into_iter()
            .map(|entry| LibraryGame {
                id: AppId(entry.appid),
                total_hours: entry.total_hours,
            })
            .collect(),
        recent: file
            .recent
            .into_iter()
            .map(|entry| RecentGame {
                id: AppId(entry.appid),
                total_hours: entry.total_hours,
                recent_hours: entry.recent_hours,
            })
            .collect(),
        wishlist: app_ids(file.wishlist),
        followed: app_ids(file.followed),
        wishlist_read_at: file.wishlist_read_at.map(Timestamp),
        followed_read_at: file.followed_read_at.map(Timestamp),
    })
}

fn signals_file(reading: &SignalsReading) -> SignalsFile {
    SignalsFile {
        read_at: reading.read_at.0,
        library: reading
            .library
            .iter()
            .map(|game| LibraryEntry {
                appid: game.id.0,
                total_hours: game.total_hours,
            })
            .collect(),
        recent: reading
            .recent
            .iter()
            .map(|game| RecentEntry {
                appid: game.id.0,
                recent_hours: game.recent_hours,
                total_hours: game.total_hours,
            })
            .collect(),
        wishlist: raw_ids(reading.wishlist.as_deref()),
        followed: raw_ids(reading.followed.as_deref()),
        wishlist_read_at: reading.wishlist_read_at.map(|at| at.0),
        followed_read_at: reading.followed_read_at.map(|at| at.0),
    }
}

fn app_ids(ids: Option<Vec<u32>>) -> Option<Vec<AppId>> {
    ids.map(|ids| ids.into_iter().map(AppId).collect())
}

fn raw_ids(ids: Option<&[AppId]>) -> Option<Vec<u32>> {
    ids.map(|ids| ids.iter().map(|id| id.0).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reading_reads_every_field() {
        let reading = parse_reading(
            r#"{
                "readAt": 1759835069734,
                "library": [{ "appid": 10, "totalHours": 12.5 }],
                "recent": [{ "appid": 10, "recentHours": 1.5, "totalHours": 12.5 }],
                "wishlist": [400],
                "followed": [500, 600],
                "wishlistReadAt": 1759835069734,
                "followedReadAt": 1759700000000
            }"#,
        )
        .unwrap();

        assert_eq!(
            reading,
            SignalsReading {
                read_at: Timestamp(1_759_835_069_734),
                library: vec![LibraryGame {
                    id: AppId(10),
                    total_hours: 12.5,
                }],
                recent: vec![RecentGame {
                    id: AppId(10),
                    total_hours: 12.5,
                    recent_hours: 1.5,
                }],
                wishlist: Some(vec![AppId(400)]),
                followed: Some(vec![AppId(500), AppId(600)]),
                wishlist_read_at: Some(Timestamp(1_759_835_069_734)),
                followed_read_at: Some(Timestamp(1_759_700_000_000)),
            }
        );
    }

    #[test]
    fn null_lists_are_empty_in_the_signals() {
        let signals = parse_reading(
            r#"{ "readAt": 1, "library": [], "recent": [], "wishlist": null, "followed": null }"#,
        )
        .unwrap()
        .into_signals();

        assert!(signals.wishlist.is_empty());
        assert!(signals.followed.is_empty());
    }

    #[test]
    fn parse_reading_rejects_an_unknown_field() {
        let result = parse_reading(
            r#"{ "readAt": 1, "library": [], "recent": [], "wishlist": [], "followed": [], "extra": 1 }"#,
        );

        assert!(matches!(result, Err(SignalsError::Json(_))));
    }

    #[test]
    fn parse_reading_rejects_minutes_instead_of_hours() {
        let result = parse_reading(
            r#"{ "readAt": 1, "library": [{ "appid": 10, "totalMinutes": 60 }], "recent": [], "wishlist": [], "followed": [] }"#,
        );

        assert!(matches!(result, Err(SignalsError::Json(_))));
    }

    #[test]
    fn a_saved_reading_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signals.json");
        let reading = SignalsReading {
            read_at: Timestamp(2_000),
            library: vec![LibraryGame {
                id: AppId(10),
                total_hours: 1.5,
            }],
            recent: vec![RecentGame {
                id: AppId(10),
                total_hours: 1.5,
                recent_hours: 0.25,
            }],
            wishlist: Some(vec![AppId(400)]),
            followed: None,
            wishlist_read_at: Some(Timestamp(2_000)),
            followed_read_at: None,
        };

        save_reading(&path, &reading).unwrap();

        assert_eq!(load_reading(&path).unwrap(), reading);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""followed":null"#));
        assert!(!text.contains("followedReadAt"));
        assert!(!dir.path().join("signals.json.tmp").exists());
    }

    #[test]
    fn saving_replaces_the_previous_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signals.json");
        std::fs::write(&path, "old").unwrap();
        let reading = SignalsReading {
            read_at: Timestamp(3_000),
            library: vec![],
            recent: vec![],
            wishlist: Some(vec![]),
            followed: Some(vec![AppId(500)]),
            wishlist_read_at: Some(Timestamp(3_000)),
            followed_read_at: Some(Timestamp(1_000)),
        };

        save_reading(&path, &reading).unwrap();

        assert_eq!(load_reading(&path).unwrap(), reading);
    }

    #[test]
    fn load_signals_reports_a_missing_file() {
        let result = load_signals(Path::new("does-not-exist/signals.json"));
        assert!(matches!(result, Err(SignalsError::NotFound(_))));
    }
}
