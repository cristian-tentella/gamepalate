use std::path::{Path, PathBuf};

use gamepalate_core::{
    ids::AppId,
    signals::{LibraryGame, RecentGame, Signals},
    timestamp::Timestamp,
};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum SignalsError {
    #[error("signals not found at {}", .0.display())]
    NotFound(PathBuf),

    #[error("could not read the signals")]
    Io(#[from] std::io::Error),

    #[error("the signals file is not valid")]
    Json(#[from] serde_json::Error),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalsFile {
    read_at: i64,
    library: Vec<LibraryEntry>,
    recent: Vec<RecentEntry>,
    wishlist: Option<Vec<u32>>,
    followed: Option<Vec<u32>>,
    #[allow(dead_code)]
    wishlist_read_at: Option<i64>,
    #[allow(dead_code)]
    followed_read_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LibraryEntry {
    appid: u32,
    total_hours: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecentEntry {
    appid: u32,
    total_hours: f64,
    recent_hours: f64,
}

pub fn load_signals(path: &Path) -> Result<Signals, SignalsError> {
    if !path.is_file() {
        return Err(SignalsError::NotFound(path.to_path_buf()));
    }

    let text = std::fs::read_to_string(path)?;

    parse_signals(&text)
}

fn parse_signals(text: &str) -> Result<Signals, SignalsError> {
    let file: SignalsFile = serde_json::from_str(text)?;

    Ok(Signals {
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
    })
}

fn app_ids(ids: Option<Vec<u32>>) -> Vec<AppId> {
    ids.unwrap_or_default().into_iter().map(AppId).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_signals_reads_every_field() {
        let signals = parse_signals(
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
            signals,
            Signals {
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
                wishlist: vec![AppId(400)],
                followed: vec![AppId(500), AppId(600)],
            }
        );
    }

    #[test]
    fn parse_signals_treats_null_lists_as_empty() {
        let signals = parse_signals(
            r#"{ "readAt": 1, "library": [], "recent": [], "wishlist": null, "followed": null }"#,
        )
        .unwrap();

        assert!(signals.wishlist.is_empty());
        assert!(signals.followed.is_empty());
    }

    #[test]
    fn parse_signals_rejects_an_unknown_field() {
        let result = parse_signals(
            r#"{ "readAt": 1, "library": [], "recent": [], "wishlist": [], "followed": [], "extra": 1 }"#,
        );

        assert!(matches!(result, Err(SignalsError::Json(_))));
    }

    #[test]
    fn parse_signals_rejects_minutes_instead_of_hours() {
        let result = parse_signals(
            r#"{ "readAt": 1, "library": [{ "appid": 10, "totalMinutes": 60 }], "recent": [], "wishlist": [], "followed": [] }"#,
        );

        assert!(matches!(result, Err(SignalsError::Json(_))));
    }

    #[test]
    fn load_signals_reports_a_missing_file() {
        let result = load_signals(Path::new("does-not-exist/signals.json"));
        assert!(matches!(result, Err(SignalsError::NotFound(_))));
    }
}
