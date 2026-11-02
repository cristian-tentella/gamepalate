use std::path::Path;

use gamepalate_core::{config::Config, timestamp::Timestamp};
use gamepalate_steam::transport::{Get, Transport};

use crate::{
    account::steam_api_key,
    app::{Gamepalate, OpenError},
    log::failed,
    ready::{ReadyError, check_ready_with},
    update::{Progress, UpdateError, Updated, update_catalog, update_config, wall_clock},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Catalog(Progress),
    Signals,
    Opening,
}

#[derive(Debug)]
pub struct Refreshed {
    pub gamepalate: Gamepalate,
    pub updated: Updated,
    pub signals_error: Option<ReadyError>,
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error(transparent)]
    Update(#[from] UpdateError),

    #[error(transparent)]
    Open(#[from] OpenError),
}

pub async fn refresh(
    data_dir: &Path,
    mut progress: impl FnMut(Step),
) -> Result<Refreshed, RefreshError> {
    let config = update_config(data_dir)?;
    let transport = Transport::new(steam_api_key()).map_err(UpdateError::from)?;
    let (updated, signals_error) =
        refresh_with(data_dir, &transport, &config, wall_clock(), &mut progress).await?;
    progress(Step::Opening);
    let gamepalate = Gamepalate::open(Some(data_dir.to_path_buf()))?;

    Ok(Refreshed {
        gamepalate,
        updated,
        signals_error,
    })
}

async fn refresh_with<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    now: Timestamp,
    progress: &mut impl FnMut(Step),
) -> Result<(Updated, Option<ReadyError>), UpdateError> {
    let updated = update_catalog(data_dir, steam, config, |step| {
        progress(Step::Catalog(step))
    })
    .await?;
    progress(Step::Signals);
    let signals_error = check_ready_with(data_dir, steam, true, now)
        .await
        .inspect_err(failed("could not read the Signals"))
        .err();

    Ok((updated, signals_error))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_steam::transport::SteamError;
    use gamepalate_store::{catalog::load_catalog, signals::load_reading};
    use serde_json::{Value, json};

    use super::*;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    struct FakeSteam {
        apps: Vec<u32>,
        owned: Vec<(u32, u64)>,
        down: Option<&'static str>,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            if self.down.is_some_and(|down| endpoint.starts_with(down)) {
                return Err(SteamError::SteamDown {
                    endpoint: endpoint.to_string(),
                    reason: "timed out".to_string(),
                    source: None,
                });
            }
            Ok(match endpoint {
                "IStoreService/GetTagList/v1" => {
                    json!({ "response": { "tags": [{ "tagid": 19, "name": "Action" }] } })
                }
                "IStoreService/GetAppList/v1" => json!({ "response": { "apps": self
                    .apps
                    .iter()
                    .map(|appid| json!({ "appid": appid, "last_modified": 1, "price_change_number": 1 }))
                    .collect::<Vec<_>>() } }),
                "IStoreBrowseService/GetItems/v1" => {
                    let input: Value = serde_json::from_str(params[0].1).unwrap();
                    let items: Vec<Value> = input["ids"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|id| game(id["appid"].as_u64().unwrap()))
                        .collect();
                    json!({ "response": { "store_items": items } })
                }
                "IPlayerService/GetOwnedGames/v1" => json!({ "response": { "games": self
                    .owned
                    .iter()
                    .map(|(appid, minutes)| json!({ "appid": appid, "playtime_forever": minutes }))
                    .collect::<Vec<_>>() } }),
                _ => json!({ "response": {} }),
            })
        }
    }

    fn game(appid: u64) -> Value {
        json!({
            "id": appid,
            "success": 1,
            "name": format!("Game {appid}"),
            "type": 0,
            "release": { "steam_release_date": 1_600_000_000 },
            "tags": [{ "tagid": 19, "weight": 100 }],
            "best_purchase_option": { "packageid": appid, "final_price_in_cents": "999" },
        })
    }

    fn steam(apps: &[u32], owned: &[(u32, u64)], down: Option<&'static str>) -> FakeSteam {
        FakeSteam {
            apps: apps.to_vec(),
            owned: owned.to_vec(),
            down,
        }
    }

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "steam_id = \"76561198000000000\"\n",
        )
        .unwrap();
        dir
    }

    async fn run(
        dir: &Path,
        steam: &FakeSteam,
    ) -> (Vec<Step>, Result<Option<ReadyError>, UpdateError>) {
        let mut steps = Vec::new();
        let result = refresh_with(dir, steam, &Config::default(), NOW, &mut |step| {
            steps.push(step)
        })
        .await
        .map(|(_, signals_error)| signals_error);
        (steps, result)
    }

    fn game_ids(dir: &Path) -> Vec<u32> {
        load_catalog(&dir.join("catalog.sqlite"))
            .unwrap()
            .games()
            .iter()
            .map(|game| game.id.0)
            .collect()
    }

    #[tokio::test]
    async fn the_catalog_is_written_then_the_signals_are_read_again() {
        let dir = folder();

        let (steps, result) = run(dir.path(), &steam(&[10, 11], &[(10, 180)], None)).await;

        assert!(result.unwrap().is_none());
        assert_eq!(game_ids(dir.path()), [10, 11]);
        let reading = load_reading(&dir.path().join("signals.json")).unwrap();
        assert_eq!(reading.read_at, NOW);
        assert_eq!(steps.first(), Some(&Step::Catalog(Progress::ReadingApps)));
        assert_eq!(
            &steps[steps.len() - 2..],
            [Step::Catalog(Progress::Writing), Step::Signals]
        );
    }

    #[tokio::test]
    async fn signals_that_fail_do_not_fail_the_update() {
        let dir = folder();

        let (_, result) = run(
            dir.path(),
            &steam(&[10], &[(10, 180)], Some("IPlayerService")),
        )
        .await;

        assert!(result.unwrap().is_some());
        assert_eq!(game_ids(dir.path()), [10]);
    }

    #[tokio::test]
    async fn a_stopped_catalog_does_not_read_the_signals() {
        let dir = folder();

        let (steps, result) = run(
            dir.path(),
            &steam(&[10], &[(10, 180)], Some("IStoreBrowseService")),
        )
        .await;

        assert!(matches!(result, Err(UpdateError::Stopped { .. })));
        assert!(!steps.contains(&Step::Signals));
        assert!(!dir.path().join("signals.json").exists());
    }
}
