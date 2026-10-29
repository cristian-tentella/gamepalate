use std::path::Path;

use anyhow::Context;
use gamepalate_core::{config::Config, signals::Signals, timestamp::Timestamp};
use gamepalate_steam::{
    signals::{FailedList, OptionalList, read_signals},
    transport::{Get, SteamError, Transport},
};
use gamepalate_store::signals::{load_reading, save_reading};

use crate::{format::date_time, update::wall_clock};

pub fn from_steam(data_dir: &Path, config: &Config) -> anyhow::Result<Signals> {
    let transport = Transport::new(std::env::var("STEAM_API_KEY").ok())?;
    let runtime = tokio::runtime::Runtime::new()?;
    let (signals, warnings) = runtime.block_on(read(data_dir, &transport, config, wall_clock()))?;
    for warning in warnings {
        eprintln!("warn: {warning}");
    }
    Ok(signals)
}

async fn read<S: Get>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    now: Timestamp,
) -> anyhow::Result<(Signals, Vec<String>)> {
    let steam_id = config
        .steam_id
        .context("steam_id is missing from the config")?;
    let path = data_dir.join("signals.json");
    let saved = load_reading(&path).ok();
    let had_saved = saved.is_some();

    let read = read_signals(steam, steam_id, saved, now)
        .await
        .map_err(|error| {
            let down = matches!(error, SteamError::SteamDown { .. });
            let error = anyhow::Error::new(error);
            if down && !had_saved {
                error.context(format!(
                    "could not read the signals from Steam, and {} has no reading to use instead",
                    path.display()
                ))
            } else {
                error.context("could not read the signals from Steam")
            }
        })?;

    let mut warnings = Vec::new();
    match read.saved_from {
        Some(read_at) => warnings.push(format!(
            "Steam is not responding: using the signals read on {}.",
            date_time(read_at)
        )),
        None => save_reading(&path, &read.reading)
            .with_context(|| format!("could not save the signals to {}", path.display()))?,
    }
    warnings.extend(read.failed.iter().map(failed_warning));

    Ok((read.reading.into_signals(), warnings))
}

fn failed_warning(failed: &FailedList) -> String {
    let (name, one, it) = match failed.list {
        OptionalList::Wishlist => ("Wishlist", "the one", "it"),
        OptionalList::Followed => ("Followed games", "the ones", "them"),
    };
    match failed.saved_from {
        Some(read_at) => format!(
            "{name} unavailable: using {one} read on {}.",
            date_time(read_at)
        ),
        None => format!("{name} unavailable: going on without {it}."),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_core::ids::{AppId, SteamId};
    use serde_json::{Value, json};

    use super::*;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    struct FakeSteam {
        down: bool,
        wishlist_down: bool,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            let down = self.down || self.wishlist_down && endpoint.contains("Wishlist");
            if down {
                return Err(SteamError::SteamDown {
                    endpoint: endpoint.to_string(),
                    reason: "timed out".to_string(),
                    source: None,
                });
            }
            Ok(match endpoint {
                "IPlayerService/GetOwnedGames/v1" => {
                    json!({ "response": { "games": [{ "appid": 10, "playtime_forever": 120 }] } })
                }
                "IStoreService/GetGamesFollowed/v1" => json!({ "response": { "appids": [500] } }),
                _ => json!({ "response": {} }),
            })
        }
    }

    fn config() -> Config {
        Config {
            steam_id: Some(SteamId(76_561_198_000_000_000)),
            ..Config::default()
        }
    }

    fn steam() -> FakeSteam {
        FakeSteam {
            down: false,
            wishlist_down: false,
        }
    }

    const SAVED: &str = r#"{"readAt":1793000000000,"library":[{"appid":1,"totalHours":3.0}],"recent":[],"wishlist":[400],"followed":null}"#;

    #[tokio::test]
    async fn a_reading_from_steam_is_saved_without_warnings() {
        let dir = tempfile::tempdir().unwrap();

        let (signals, warnings) = read(dir.path(), &steam(), &config(), NOW).await.unwrap();

        assert_eq!(signals.library[0].id, AppId(10));
        assert_eq!(signals.followed, [AppId(500)]);
        assert!(warnings.is_empty());
        let saved = load_reading(&dir.path().join("signals.json")).unwrap();
        assert_eq!(saved.read_at, NOW);
        assert_eq!(saved.followed_read_at, Some(NOW));
    }

    #[tokio::test]
    async fn steam_down_uses_the_saved_signals_and_says_when_they_were_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signals.json");
        fs::write(&path, SAVED).unwrap();
        let steam = FakeSteam {
            down: true,
            ..steam()
        };

        let (signals, warnings) = read(dir.path(), &steam, &config(), NOW).await.unwrap();

        assert_eq!(signals.library[0].id, AppId(1));
        assert_eq!(
            warnings,
            [format!(
                "Steam is not responding: using the signals read on {}.",
                date_time(Timestamp(1_793_000_000_000))
            )]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), SAVED);
    }

    #[tokio::test]
    async fn steam_down_without_saved_signals_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("signals.json"), "not json").unwrap();
        let steam = FakeSteam {
            down: true,
            ..steam()
        };

        let error = read(dir.path(), &steam, &config(), NOW).await.unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("has no reading to use instead"));
        assert!(message.contains("Steam is not responding"));
    }

    #[tokio::test]
    async fn a_failed_wishlist_says_which_one_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signals.json");
        let steam = FakeSteam {
            wishlist_down: true,
            ..steam()
        };

        let (_, without_saved) = read(dir.path(), &steam, &config(), NOW).await.unwrap();
        fs::write(&path, SAVED).unwrap();
        let (signals, with_saved) = read(dir.path(), &steam, &config(), NOW).await.unwrap();

        assert_eq!(
            without_saved,
            ["Wishlist unavailable: going on without it."]
        );
        assert_eq!(signals.wishlist, [AppId(400)]);
        assert_eq!(
            with_saved,
            [format!(
                "Wishlist unavailable: using the one read on {}.",
                date_time(Timestamp(1_793_000_000_000))
            )]
        );
        let saved = load_reading(&path).unwrap();
        assert_eq!(saved.read_at, NOW);
        assert_eq!(saved.wishlist_read_at, Some(Timestamp(1_793_000_000_000)));
    }

    #[test]
    fn a_failed_followed_list_is_plural() {
        let failed = FailedList {
            list: OptionalList::Followed,
            saved_from: None,
        };

        assert_eq!(
            failed_warning(&failed),
            "Followed games unavailable: going on without them."
        );
    }
}
