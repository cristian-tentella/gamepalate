use std::path::Path;

use gamepalate_core::{profile::profile, timestamp::Timestamp};
use gamepalate_steam::transport::{Get, SteamError, Transport};
use gamepalate_store::{
    config::load_config,
    data_files::{DataFilesError, load_data_files},
    signals::SignalsError,
};

use crate::{
    account::steam_api_key,
    signals::{
        OffCatalogError, OffCatalogInputs, ReadSignalsError, off_catalog, read_signals,
        save_signals,
    },
    update::wall_clock,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadyCheck {
    pub games: usize,
    pub library: usize,
    pub min_hours: f64,
    pub empty_profile: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadyError {
    #[error(transparent)]
    Files(#[from] DataFilesError),

    #[error("steam_id is missing from the config")]
    NoSteamId,

    #[error(transparent)]
    Steam(#[from] SteamError),

    #[error(transparent)]
    Read(#[from] ReadSignalsError),

    #[error(transparent)]
    Signals(#[from] SignalsError),

    #[error(transparent)]
    OffCatalog(#[from] OffCatalogError),
}

pub async fn check_ready(
    data_dir: &Path,
    read_signals_again: bool,
) -> Result<ReadyCheck, ReadyError> {
    let transport = Transport::new(steam_api_key())?;
    check_ready_with(data_dir, &transport, read_signals_again, wall_clock()).await
}

async fn check_ready_with<S: Get>(
    data_dir: &Path,
    steam: &S,
    read_signals_again: bool,
    now: Timestamp,
) -> Result<ReadyCheck, ReadyError> {
    if read_signals_again {
        let steam_id = load_config(&data_dir.join("config.toml"))
            .map_err(DataFilesError::from)?
            .steam_id
            .ok_or(ReadyError::NoSteamId)?;
        let read = read_signals(data_dir, steam, steam_id, now).await?;
        save_signals(data_dir, &read)?;
    }

    let mut files = load_data_files(data_dir)?;
    files.off_catalog = off_catalog(
        data_dir,
        steam,
        OffCatalogInputs {
            signals: &files.signals,
            votes: &files.votes,
            families: &files.families,
            catalog: &files.catalog,
            configs: &[&files.config],
        },
        now,
    )
    .await?;

    Ok(ReadyCheck {
        games: files.catalog.games().len(),
        library: files.signals.library.len(),
        min_hours: files.config.signals.min_hours,
        empty_profile: profile(&files.search_inputs(&[]), 1).is_none(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_core::config::Config;
    use gamepalate_store::off_catalog::load_off_catalog_cache;
    use serde_json::{Value, json};

    use super::*;
    use crate::update::update_catalog;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    struct FakeSteam {
        owned: Vec<(u32, u64)>,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            Ok(match endpoint {
                "IStoreService/GetTagList/v1" => json!({ "response": { "tags": [
                    { "tagid": 19, "name": "Action" },
                    { "tagid": 20, "name": "Puzzle" },
                ] } }),
                "IStoreService/GetAppList/v1" => json!({ "response": { "apps": (10..=13)
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
        let tag = if (11..=13).contains(&appid) { 20 } else { 19 };
        json!({
            "id": appid,
            "success": 1,
            "name": format!("Game {appid}"),
            "type": 0,
            "release": { "steam_release_date": 1_600_000_000 },
            "reviews": { "summary_filtered": {
                "review_count": 1000,
                "percent_positive": 90,
                "review_score": 8,
            } },
            "tags": [{ "tagid": tag, "weight": 100 }],
            "best_purchase_option": { "packageid": appid, "final_price_in_cents": "999" },
        })
    }

    async fn ready_folder(owned: &[(u32, u64)]) -> (tempfile::TempDir, FakeSteam) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "steam_id = \"76561198000000000\"\n",
        )
        .unwrap();
        let steam = FakeSteam {
            owned: owned.to_vec(),
        };
        update_catalog(dir.path(), &steam, &Config::default(), |_| {})
            .await
            .unwrap();
        (dir, steam)
    }

    #[tokio::test]
    async fn a_game_above_the_threshold_makes_a_profile() {
        let (dir, steam) = ready_folder(&[(10, 180)]).await;

        let check = check_ready_with(dir.path(), &steam, true, NOW)
            .await
            .unwrap();

        assert_eq!(
            check,
            ReadyCheck {
                games: 4,
                library: 1,
                min_hours: 2.0,
                empty_profile: false,
            }
        );
    }

    #[tokio::test]
    async fn games_below_the_threshold_make_an_empty_profile() {
        let (dir, steam) = ready_folder(&[(10, 60)]).await;

        let check = check_ready_with(dir.path(), &steam, true, NOW)
            .await
            .unwrap();

        assert!(check.empty_profile);
    }

    #[tokio::test]
    async fn a_game_outside_the_catalog_gets_its_tags() {
        let (dir, steam) = ready_folder(&[(30, 180)]).await;

        let check = check_ready_with(dir.path(), &steam, true, NOW)
            .await
            .unwrap();

        assert!(!check.empty_profile);
        let cache = load_off_catalog_cache(&dir.path().join("off-catalog-tags.json"));
        assert!(cache.contains_key(&gamepalate_core::ids::AppId(30)));
    }

    #[tokio::test]
    async fn without_reading_again_it_uses_the_saved_signals() {
        let (dir, steam) = ready_folder(&[(10, 180)]).await;
        check_ready_with(dir.path(), &steam, true, NOW)
            .await
            .unwrap();
        let quiet = FakeSteam { owned: Vec::new() };

        let check = check_ready_with(dir.path(), &quiet, false, NOW)
            .await
            .unwrap();

        assert!(!check.empty_profile);
    }
}
