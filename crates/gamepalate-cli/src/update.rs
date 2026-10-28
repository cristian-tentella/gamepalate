use std::{collections::HashMap, path::Path};

use anyhow::Context;
use futures::StreamExt;
use gamepalate_core::{
    catalog::{Catalog, Freshness, compute_idf},
    config::Config,
    ids::AppId,
    incremental::{KnownApps, changed_apps},
    store_item::StoreItem,
    timestamp::Timestamp,
};
use gamepalate_steam::{
    apps::get_app_list,
    archive::{Archive, ArchivedUpdate, UpdateFolder},
    items::{archived_batches, batch_count, item_batches},
    tags::get_tags,
    transport::{Get, SteamError, Transport},
};
use gamepalate_store::{
    catalog::write_catalog,
    config::{ConfigError, load_config},
};

use crate::format::{date_time, integer};

const CONCURRENCY: usize = 4;
const LANGUAGE: &str = "english";
const COUNTRY_CODE: &str = "IT";

pub struct BuiltCatalog {
    pub catalog: Catalog,
    pub apps: usize,
    pub asked: usize,
    price_change_numbers: HashMap<AppId, u64>,
}

pub fn archive(data_dir: &Path) -> Archive {
    Archive::new(data_dir.join("archive"))
}

pub fn run(data_dir: &Path) -> anyhow::Result<()> {
    let config = match load_config(&data_dir.join("config.toml")) {
        Err(ConfigError::NotFound(_)) => Config::default(),
        config => config?,
    };
    let transport = Transport::new(std::env::var("STEAM_API_KEY").ok())?;
    update(data_dir, &transport, &config)
}

fn update<S: Get + Sync>(data_dir: &Path, steam: &S, config: &Config) -> anyhow::Result<()> {
    let resume_within_hours = config.prices_max_age_hours;
    let archive = archive(data_dir);
    let update = archive
        .start_update(wall_clock(), resume_within_hours, config.tags_max_age_days)
        .context("could not start a new update in the archive")?;
    let earlier = if update.full() {
        Vec::new()
    } else {
        let chain = archive.chain().context("could not read the archive")?;
        check_base(&chain, update.dir())?;
        chain
    };
    let now = update.started_at();
    if update.resumed_responses() > 0 {
        println!(
            "Resuming the update started {}: {} responses already archived.",
            date_time(now),
            integer(update.resumed_responses() as u64)
        );
    }

    let result = build_catalog(&update.archiving(steam), now, &earlier);
    let built = match result {
        Ok(built) => built,
        Err(error) => return Err(stopped(update, error, resume_within_hours)),
    };
    built.write(data_dir)?;
    update
        .complete(wall_clock())
        .context("could not mark the update as completed in the archive")?;
    if let Err(error) = archive.rotate(config.kept_updates) {
        eprintln!("warn: could not remove the old updates from the archive: {error}");
    }

    println!(
        "Catalog updated: {} games out of {} apps (store data downloaded for {} apps).",
        integer(built.catalog.games().len() as u64),
        integer(built.apps as u64),
        integer(built.asked as u64)
    );
    println!("Prices as of {}.", date_time(now));
    Ok(())
}

fn stopped(update: UpdateFolder, error: anyhow::Error, resume_within_hours: u32) -> anyhow::Error {
    if matches!(
        error.downcast_ref::<SteamError>(),
        Some(SteamError::Shape { .. })
    ) {
        let note = "the Catalog was not changed, and the next update will start over";
        return match update.fail(wall_clock()) {
            Ok(()) => error.context(note),
            Err(fail_error) => error.context(format!(
                "{note}; could not mark this one as failed in the archive: {fail_error}"
            )),
        };
    }

    let deadline = Timestamp(update.started_at().0 + i64::from(resume_within_hours) * 3_600_000);
    error.context(format!(
        "the Catalog was not changed: run update again before {} to resume",
        date_time(deadline)
    ))
}

pub fn check_base(chain: &[ArchivedUpdate], update: &Path) -> anyhow::Result<()> {
    match chain.last() {
        Some(base) if base.full() => Ok(()),
        _ => anyhow::bail!(
            "the archive no longer has the full update that {} builds on: remove that folder and run update again",
            update.display()
        ),
    }
}

fn wall_clock() -> Timestamp {
    Timestamp(jiff::Timestamp::now().as_millisecond())
}

pub fn build_catalog<S: Get>(
    steam: &S,
    now: Timestamp,
    earlier: &[ArchivedUpdate],
) -> anyhow::Result<BuiltCatalog> {
    let runtime = tokio::runtime::Runtime::new()?;

    let tags = runtime.block_on(get_tags(steam))?;
    let apps = runtime.block_on(get_app_list(steam))?;
    let ids: Vec<AppId> = apps.iter().map(|app| app.id).collect();
    let current = apps.iter().map(|app| (app.id, app.price_change_number));
    let asked = match earlier.first() {
        None => ids.clone(),
        Some(previous) => {
            let previous: HashMap<AppId, u64> = runtime
                .block_on(get_app_list(previous))?
                .into_iter()
                .map(|app| (app.id, app.price_change_number))
                .collect();
            changed_apps(current, &previous)
        }
    };

    let total = batch_count(asked.len());
    let mut items: Vec<StoreItem> = Vec::new();
    runtime.block_on(async {
        let mut stream = item_batches(steam, &asked, LANGUAGE, COUNTRY_CODE, CONCURRENCY);
        let mut done = 0;
        while let Some(batch) = stream.next().await {
            items.extend(batch?);
            done += 1;
            eprint!("\rStore data: {done} of {total} requests");
        }
        eprintln!();
        anyhow::Ok(())
    })?;

    let mut known = KnownApps::new(ids.iter().copied(), now);
    known.add(&asked, items);
    for update in earlier {
        for batch in archived_batches(update)? {
            let batch = batch?;
            known.add(&batch.asked, batch.items);
        }
    }

    let games = known.into_games();
    let idf = compute_idf(&games);
    let freshness = Freshness {
        prices_updated_at: now,
        tags_updated_at: earlier.last().map_or(now, ArchivedUpdate::started_at),
    };
    Ok(BuiltCatalog {
        catalog: Catalog::new(games, tags, idf, freshness),
        apps: ids.len(),
        asked: asked.len(),
        price_change_numbers: apps
            .iter()
            .map(|app| (app.id, app.price_change_number))
            .collect(),
    })
}

impl BuiltCatalog {
    pub fn write(&self, data_dir: &Path) -> anyhow::Result<()> {
        write_catalog(
            &data_dir.join("catalog.sqlite"),
            &self.catalog,
            &self.price_change_numbers,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, sync::Mutex};

    use gamepalate_store::catalog::load_catalog;
    use serde_json::{Value, json};

    use super::*;

    const ITEMS: &str = "IStoreBrowseService/GetItems/v1";

    #[derive(Clone, Copy)]
    enum Failure {
        Down,
        DownAfterOneBatch,
        Shape,
    }

    struct FakeSteam {
        apps: Vec<(u32, u64)>,
        failure: Option<Failure>,
        asked_items: Mutex<Vec<u32>>,
    }

    impl FakeSteam {
        fn new(apps: &[(u32, u64)]) -> FakeSteam {
            FakeSteam {
                apps: apps.to_vec(),
                failure: None,
                asked_items: Mutex::new(Vec::new()),
            }
        }

        fn failing(apps: &[(u32, u64)], failure: Failure) -> FakeSteam {
            FakeSteam {
                failure: Some(failure),
                ..FakeSteam::new(apps)
            }
        }

        fn asked_items(&self) -> Vec<u32> {
            self.asked_items.lock().unwrap().clone()
        }
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            match endpoint {
                "IStoreService/GetTagList/v1" => {
                    Ok(json!({ "response": { "tags": [{ "tagid": 19, "name": "Action" }] } }))
                }
                "IStoreService/GetAppList/v1" => Ok(json!({ "response": { "apps": self
                    .apps
                    .iter()
                    .map(|(appid, price_change_number)| json!({
                        "appid": appid,
                        "last_modified": 1,
                        "price_change_number": price_change_number,
                    }))
                    .collect::<Vec<_>>() } })),
                ITEMS => {
                    let input: Value = serde_json::from_str(params[0].1).unwrap();
                    let ids: Vec<u32> = input["ids"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|id| id["appid"].as_u64().unwrap() as u32)
                        .collect();
                    let earlier_batches = {
                        let mut asked = self.asked_items.lock().unwrap();
                        let earlier = asked.len();
                        asked.extend(&ids);
                        earlier
                    };
                    let failure = match self.failure {
                        Some(Failure::DownAfterOneBatch) if earlier_batches == 0 => None,
                        Some(Failure::DownAfterOneBatch) => Some(Failure::Down),
                        failure => failure,
                    };
                    match failure {
                        Some(Failure::Down | Failure::DownAfterOneBatch) => {
                            Err(SteamError::SteamDown {
                                endpoint: ITEMS.to_string(),
                                reason: "timed out".to_string(),
                                source: None,
                            })
                        }
                        Some(Failure::Shape) => {
                            Ok(json!({ "response": { "store_items": "unexpected" } }))
                        }
                        None => Ok(json!({ "response": { "store_items": ids
                            .iter()
                            .map(|id| game(*id))
                            .collect::<Vec<_>>() } })),
                    }
                }
                other => panic!("unexpected request to {other}"),
            }
        }
    }

    fn game(appid: u32) -> Value {
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

    fn game_ids(data_dir: &Path) -> Vec<u32> {
        load_catalog(&data_dir.join("catalog.sqlite"))
            .unwrap()
            .games()
            .iter()
            .map(|game| game.id.0)
            .collect()
    }

    fn manifests(data_dir: &Path) -> Vec<Value> {
        let root = data_dir.join("archive");
        let mut names: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
            .iter()
            .map(|name| {
                serde_json::from_slice(&fs::read(root.join(name).join("update.json")).unwrap())
                    .unwrap()
            })
            .collect()
    }

    fn remove_responses(dir: &Path, prefix: &str) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(prefix)
            {
                fs::remove_file(path).unwrap();
            }
        }
    }

    fn update_dir(data_dir: &Path, number: &str) -> PathBuf {
        data_dir.join("archive").join(number)
    }

    fn run_update(data_dir: &Path, steam: &FakeSteam) -> anyhow::Result<()> {
        update(data_dir, steam, &Config::default())
    }

    #[test]
    fn a_full_update_then_an_incremental_one_asks_only_the_changed_apps() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20), (3, 30)])).unwrap();

        let steam = FakeSteam::new(&[(1, 10), (2, 21), (4, 40)]);
        run_update(data.path(), &steam).unwrap();

        let mut asked = steam.asked_items();
        asked.sort();
        assert_eq!(asked, [2, 4]);
        assert_eq!(game_ids(data.path()), [1, 2, 4]);
        let manifests = manifests(data.path());
        assert_eq!(manifests[0]["full"], true);
        assert_eq!(manifests[1]["full"], false);
    }

    #[test]
    fn steam_down_keeps_the_old_catalog_and_the_next_update_resumes() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)])).unwrap();
        let catalog = fs::read(data.path().join("catalog.sqlite")).unwrap();

        let error =
            run_update(data.path(), &FakeSteam::failing(&[(1, 11)], Failure::Down)).unwrap_err();

        assert!(format!("{error:#}").contains("run update again before"));
        assert_eq!(
            fs::read(data.path().join("catalog.sqlite")).unwrap(),
            catalog
        );
        assert_eq!(manifests(data.path())[1].get("completedAt"), None);

        let steam = FakeSteam::new(&[(1, 12)]);
        run_update(data.path(), &steam).unwrap();
        assert_eq!(steam.asked_items(), [1]);
        assert_eq!(manifests(data.path()).len(), 2);
        assert!(manifests(data.path())[1].get("completedAt").is_some());
    }

    #[test]
    fn a_resumed_update_does_not_ask_again_what_it_had() {
        let data = tempfile::tempdir().unwrap();
        let apps: Vec<(u32, u64)> = (1..=600).map(|id| (id, 1)).collect();
        let interrupted = FakeSteam::failing(&apps, Failure::DownAfterOneBatch);
        run_update(data.path(), &interrupted).unwrap_err();
        let first_batch = interrupted.asked_items()[..260].to_vec();

        let steam = FakeSteam::new(&apps);
        run_update(data.path(), &steam).unwrap();

        let asked = steam.asked_items();
        assert_eq!(asked.len(), 600 - 260);
        assert!(first_batch.iter().all(|id| !asked.contains(id)));
        assert_eq!(game_ids(data.path()).len(), 600);
    }

    #[test]
    fn an_unexpected_shape_marks_the_update_failed_and_the_next_starts_over() {
        let data = tempfile::tempdir().unwrap();

        let error =
            run_update(data.path(), &FakeSteam::failing(&[(1, 10)], Failure::Shape)).unwrap_err();

        assert!(format!("{error:#}").contains("the next update will start over"));
        assert!(manifests(data.path())[0].get("failedAt").is_some());
        assert!(!data.path().join("catalog.sqlite").exists());

        let steam = FakeSteam::new(&[(1, 10)]);
        run_update(data.path(), &steam).unwrap();
        assert_eq!(steam.asked_items(), [1]);
        assert_eq!(manifests(data.path()).len(), 1);
        assert_eq!(manifests(data.path())[0]["full"], true);
    }

    #[test]
    fn rebuild_gives_the_catalog_of_the_last_update() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)])).unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 11), (3, 30)])).unwrap();
        let catalog = data.path().join("catalog.sqlite");
        let updated = data.path().join("updated.sqlite");
        fs::rename(&catalog, &updated).unwrap();

        crate::rebuild::run(data.path()).unwrap();

        assert_eq!(game_ids(data.path()), [1, 3]);
        assert_eq!(
            load_catalog(&catalog).unwrap().games(),
            load_catalog(&updated).unwrap().games()
        );
    }

    #[test]
    fn rebuild_without_a_completed_update_says_to_run_update() {
        let data = tempfile::tempdir().unwrap();
        let _ = run_update(data.path(), &FakeSteam::failing(&[(1, 10)], Failure::Down));

        let error = crate::rebuild::run(data.path()).unwrap_err();

        assert!(error.to_string().contains("run update first"));
    }

    #[test]
    fn rebuild_without_the_full_update_of_the_chain_fails() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)])).unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 11)])).unwrap();
        fs::remove_dir_all(update_dir(data.path(), "000001")).unwrap();

        let error = crate::rebuild::run(data.path()).unwrap_err();

        assert!(error.to_string().contains("no longer has the full update"));
    }

    #[test]
    fn rebuild_with_a_missing_response_names_the_endpoint() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)])).unwrap();
        remove_responses(
            &update_dir(data.path(), "000001"),
            "IStoreService_GetAppList",
        );

        let error = crate::rebuild::run(data.path()).unwrap_err();

        assert!(matches!(
            error.downcast_ref::<SteamError>(),
            Some(SteamError::NotArchived { .. })
        ));
        assert!(error.to_string().contains("IStoreService/GetAppList/v1"));
    }
}
