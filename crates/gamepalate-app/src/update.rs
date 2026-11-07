use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use futures::StreamExt;
use gamepalate_core::{
    catalog::{Catalog, Freshness, compute_idf},
    config::Config,
    currency::Currency,
    ids::AppId,
    incremental::{KnownApps, changed_apps},
    region::Region,
    renewal::{Answers, Band, Renewal},
    store_item::StoreItem,
    timestamp::Timestamp,
};
use gamepalate_steam::{
    apps::get_app_list,
    archive::{Archive, ArchivedUpdate, PendingUpdate},
    chain::{ChainError, chain, rotate},
    items::{BATCH_SIZE, archived_batches, batch_count, item_batches},
    tags::get_tags,
    transport::{Get, SteamError, Transport},
};
use gamepalate_store::{
    catalog::{CatalogError, write_catalog},
    config::{ConfigError, load_config},
};

use crate::{account::steam_api_key, log::failed};

const CONCURRENCY: usize = 4;
const HOUR_MS: i64 = 3_600_000;
pub const LANGUAGE: &str = "english";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    Resumed {
        started_at: Timestamp,
        responses: usize,
    },
    ReadingApps,
    StoreData {
        done: usize,
        total: usize,
    },
    Writing,
}

#[derive(Debug)]
pub struct Updated {
    pub games: usize,
    pub apps: usize,
    pub asked: usize,
    pub prices_at: Timestamp,
    pub renewal: Option<Renewal>,
    pub rotate_error: Option<ChainError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rebuilt {
    pub dir: PathBuf,
    pub games: usize,
    pub apps: usize,
    pub prices_at: Timestamp,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("could not start a new update in the archive")]
    Start(#[source] io::Error),

    #[error("could not read the archive")]
    Archive(#[source] io::Error),

    #[error("the archive has no completed update: run update first")]
    NoCompletedUpdate,

    #[error(
        "app {} has no response in the archive: remove the archive folder and run update again",
        .app.0
    )]
    Uncovered { app: AppId },

    #[error("the Catalog was not changed: run update again to resume")]
    Stopped {
        resume_before: Timestamp,
        #[source]
        source: SteamError,
    },

    #[error(
        "the Catalog was not changed, and the next update will start over{}",
        not_marked_note(.not_marked)
    )]
    StartOver {
        #[source]
        source: SteamError,
        not_marked: Option<io::Error>,
    },

    #[error(transparent)]
    Steam(#[from] SteamError),

    #[error(transparent)]
    Catalog(#[from] CatalogError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("could not mark the update as completed in the archive")]
    Complete(#[source] io::Error),
}

impl From<ChainError> for UpdateError {
    fn from(error: ChainError) -> UpdateError {
        match error {
            ChainError::Archive(source) => UpdateError::Archive(source),
            ChainError::Steam(source) => UpdateError::Steam(source),
            ChainError::Uncovered { app } => UpdateError::Uncovered { app },
        }
    }
}

struct BuiltCatalog {
    catalog: Catalog,
    apps: usize,
    asked: usize,
    price_change_numbers: HashMap<AppId, u64>,
    renewal: Option<Renewal>,
}

pub fn archive(data_dir: &Path) -> Archive {
    Archive::new(data_dir.join("archive"))
}

pub fn wall_clock() -> Timestamp {
    Timestamp(jiff::Timestamp::now().as_millisecond())
}

pub async fn update_catalog<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    logged_update(data_dir, steam, config, false, progress).await
}

pub async fn renew_catalog<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    logged_update(data_dir, steam, config, true, progress).await
}

async fn logged_update<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    renewal: bool,
    progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    tracing::info!("update started");
    let updated = update_with(data_dir, steam, config, renewal, progress)
        .await
        .inspect_err(failed("update stopped"))?;
    if let Some(error) = &updated.rotate_error {
        failed("could not remove the old updates")(error);
    }
    if let Some(renewal) = &updated.renewal {
        log_renewal(renewal);
    }
    tracing::info!("update done, {} games", updated.games);

    Ok(updated)
}

fn log_renewal(renewal: &Renewal) {
    let bands: Vec<String> = renewal
        .renewed
        .iter()
        .map(|(band, apps)| format!("{} {apps}", band_name(*band)))
        .collect();
    tracing::info!(
        "renewed {} apps ({}), {} still expired",
        renewal.renewed_apps(),
        bands.join(", "),
        renewal.still_expired
    );
}

fn band_name(band: Band) -> &'static str {
    match band {
        Band::Month => "month",
        Band::Quarter => "quarter",
        Band::Year => "year",
        Band::Older => "older",
    }
}

async fn update_with<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    renewal: bool,
    mut progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    let archive = archive(data_dir);
    let mut update = archive
        .start_update(
            wall_clock(),
            config.prices_max_age_hours,
            config.region,
            renewal,
        )
        .map_err(UpdateError::Start)?;
    let earlier = if update.full() {
        Vec::new()
    } else {
        chain(&archive).await?
    };
    let now = update.started_at();
    if update.resumed_responses() > 0 {
        progress(Progress::Resumed {
            started_at: now,
            responses: update.resumed_responses(),
        });
    }

    let max_renewed = update
        .renewal()
        .then(|| config.renewal_requests as usize * BATCH_SIZE);
    let result = build_catalog(
        &update.archiving(steam),
        now,
        &earlier,
        config.region,
        max_renewed,
        &mut progress,
    )
    .await;
    let built = match result {
        Ok(built) => built,
        Err(source @ SteamError::Shape { .. }) => {
            return Err(UpdateError::StartOver {
                source,
                not_marked: update.fail(wall_clock()).err(),
            });
        }
        Err(source) => {
            return Err(UpdateError::Stopped {
                resume_before: Timestamp(now.0 + i64::from(config.prices_max_age_hours) * HOUR_MS),
                source,
            });
        }
    };
    built.write(data_dir)?;
    if let Some(renewal) = &built.renewal {
        update.set_renewed_apps(renewal.renewed_apps());
    }
    update
        .complete(wall_clock())
        .map_err(UpdateError::Complete)?;
    let rotate_error = rotate(&archive, config.kept_updates).await.err();

    Ok(Updated {
        games: built.catalog.games().len(),
        apps: built.apps,
        asked: built.asked,
        prices_at: now,
        renewal: built.renewal,
        rotate_error,
    })
}

pub async fn download_catalog(
    data_dir: &Path,
    progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    let config = update_config(data_dir)?;
    let transport = Transport::new(steam_api_key())?;

    update_catalog(data_dir, &transport, &config, progress).await
}

pub(crate) fn update_config(data_dir: &Path) -> Result<Config, UpdateError> {
    match load_config(&data_dir.join("config.toml")) {
        Err(ConfigError::NotFound(_)) => Ok(Config::default()),
        config => Ok(config?),
    }
}

pub fn pending_update(data_dir: &Path, config: &Config) -> Option<PendingUpdate> {
    archive(data_dir)
        .pending_update(wall_clock(), config.prices_max_age_hours, config.region)
        .ok()
        .flatten()
}

pub async fn rebuild_catalog(data_dir: &Path) -> Result<Rebuilt, UpdateError> {
    let chain = chain(&archive(data_dir)).await?;
    let Some((latest, earlier)) = chain.split_first() else {
        return Err(UpdateError::NoCompletedUpdate);
    };

    let built = build_catalog(
        latest,
        latest.started_at(),
        earlier,
        latest.region(),
        None,
        &mut |_| {},
    )
    .await?;
    built.write(data_dir)?;

    Ok(Rebuilt {
        dir: latest.dir().to_path_buf(),
        games: built.catalog.games().len(),
        apps: built.apps,
        prices_at: latest.started_at(),
    })
}

fn not_marked_note(not_marked: &Option<io::Error>) -> String {
    match not_marked {
        Some(error) => format!("; could not mark this one as failed in the archive: {error}"),
        None => String::new(),
    }
}

async fn build_catalog<S: Get>(
    steam: &S,
    now: Timestamp,
    earlier: &[ArchivedUpdate],
    region: Region,
    max_renewed: Option<usize>,
    progress: &mut impl FnMut(Progress),
) -> Result<BuiltCatalog, SteamError> {
    progress(Progress::ReadingApps);
    let tags = get_tags(steam).await?;
    let apps = get_app_list(steam).await?;
    let ids: Vec<AppId> = apps.iter().map(|app| app.id).collect();
    let current = apps.iter().map(|app| (app.id, app.price_change_number));
    let changed = match earlier.first() {
        None => ids.clone(),
        Some(previous) => {
            let previous: HashMap<AppId, u64> = get_app_list(previous)
                .await?
                .into_iter()
                .map(|app| (app.id, app.price_change_number))
                .collect();
            changed_apps(current, &previous)
        }
    };
    let renewal = match max_renewed {
        Some(max_apps) => Some(answers(earlier)?.choose(&ids, &changed, now, max_apps)),
        None => None,
    };
    let asked = match &renewal {
        Some(renewal) => renewal.asked.clone(),
        None => changed,
    };

    let total = batch_count(asked.len());
    progress(Progress::StoreData { done: 0, total });
    let mut items: Vec<StoreItem> = Vec::new();
    let mut stream = item_batches(steam, &asked, LANGUAGE, region.as_str(), CONCURRENCY);
    let mut done = 0;
    while let Some(batch) = stream.next().await {
        items.extend(batch?);
        done += 1;
        progress(Progress::StoreData { done, total });
    }
    progress(Progress::Writing);

    let mut prices = Vec::new();
    let mut known = KnownApps::new(ids.iter().copied(), now);
    prices.extend(formatted_prices(&items));
    known.add(&asked, items);
    let mut tags_updated_at = now;
    for update in earlier {
        for batch in archived_batches(update)? {
            let batch = batch?;
            prices.extend(formatted_prices(&batch.items));
            if known.add(&batch.asked, batch.items) > 0 {
                tags_updated_at = update.started_at();
            }
        }
    }
    let currency = Currency::learn(prices.iter().map(|(cents, text)| (*cents, text.as_str())))
        .unwrap_or_else(Currency::italian);

    let games = known.into_games();
    let idf = compute_idf(&games);
    let freshness = Freshness {
        prices_updated_at: now,
        tags_updated_at,
        region,
    };
    Ok(BuiltCatalog {
        catalog: Catalog::new(games, tags, idf, freshness, currency),
        apps: ids.len(),
        asked: asked.len(),
        price_change_numbers: apps
            .iter()
            .map(|app| (app.id, app.price_change_number))
            .collect(),
        renewal,
    })
}

fn answers(earlier: &[ArchivedUpdate]) -> Result<Answers, SteamError> {
    let mut answers = Answers::default();
    for update in earlier {
        for batch in archived_batches(update)? {
            let batch = batch?;
            answers.add(update.started_at(), &batch.asked, &batch.items);
        }
    }
    Ok(answers)
}

fn formatted_prices(items: &[StoreItem]) -> impl Iterator<Item = (u32, String)> + '_ {
    items
        .iter()
        .filter_map(|item| Some((item.price?.final_cents, item.formatted_price.clone()?)))
}

impl BuiltCatalog {
    fn write(&self, data_dir: &Path) -> Result<(), CatalogError> {
        write_catalog(
            &data_dir.join("catalog.sqlite"),
            &self.catalog,
            &self.price_change_numbers,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Mutex};

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
        formatted_price: Option<&'static str>,
        asked_items: Mutex<Vec<u32>>,
        countries: Mutex<Vec<String>>,
    }

    impl FakeSteam {
        fn new(apps: &[(u32, u64)]) -> FakeSteam {
            FakeSteam {
                apps: apps.to_vec(),
                failure: None,
                formatted_price: None,
                asked_items: Mutex::new(Vec::new()),
                countries: Mutex::new(Vec::new()),
            }
        }

        fn failing(apps: &[(u32, u64)], failure: Failure) -> FakeSteam {
            FakeSteam {
                failure: Some(failure),
                ..FakeSteam::new(apps)
            }
        }

        fn pricing(apps: &[(u32, u64)], formatted_price: &'static str) -> FakeSteam {
            FakeSteam {
                formatted_price: Some(formatted_price),
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
                    self.countries.lock().unwrap().push(
                        input["context"]["country_code"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                    );
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
                            .map(|id| game(*id, self.formatted_price))
                            .collect::<Vec<_>>() } })),
                    }
                }
                other => panic!("unexpected request to {other}"),
            }
        }
    }

    fn game(appid: u32, formatted_price: Option<&str>) -> Value {
        json!({
            "id": appid,
            "success": 1,
            "name": format!("Game {appid}"),
            "type": 0,
            "release": { "steam_release_date": 1_600_000_000 },
            "tags": [{ "tagid": 19, "weight": 100 }],
            "best_purchase_option": {
                "packageid": appid,
                "final_price_in_cents": "999",
                "formatted_final_price": formatted_price,
            },
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

    async fn run_update(data_dir: &Path, steam: &FakeSteam) -> Result<Updated, UpdateError> {
        update_catalog(data_dir, steam, &Config::default(), |_| {}).await
    }

    async fn events(data_dir: &Path, steam: &FakeSteam) -> Vec<Progress> {
        let mut events = Vec::new();
        let _ = update_catalog(data_dir, steam, &Config::default(), |progress| {
            events.push(progress)
        })
        .await;
        events
    }

    fn apps(count: u32) -> Vec<(u32, u64)> {
        (1..=count).map(|id| (id, 1)).collect()
    }

    #[tokio::test]
    async fn a_full_update_then_an_incremental_one_asks_only_the_changed_apps() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20), (3, 30)]))
            .await
            .unwrap();

        let steam = FakeSteam::new(&[(1, 10), (2, 21), (4, 40)]);
        let updated = run_update(data.path(), &steam).await.unwrap();

        let mut asked = steam.asked_items();
        asked.sort();
        assert_eq!(asked, [2, 4]);
        assert_eq!(game_ids(data.path()), [1, 2, 4]);
        assert_eq!((updated.games, updated.apps, updated.asked), (3, 3, 2));
        let manifests = manifests(data.path());
        assert_eq!(manifests[0]["full"], true);
        assert_eq!(manifests[1]["full"], false);
    }

    #[tokio::test]
    async fn a_full_update_tells_the_list_then_each_batch_then_the_writing() {
        let data = tempfile::tempdir().unwrap();

        let events = events(data.path(), &FakeSteam::new(&apps(600))).await;

        assert_eq!(
            events,
            [
                Progress::ReadingApps,
                Progress::StoreData { done: 0, total: 3 },
                Progress::StoreData { done: 1, total: 3 },
                Progress::StoreData { done: 2, total: 3 },
                Progress::StoreData { done: 3, total: 3 },
                Progress::Writing,
            ]
        );
    }

    #[tokio::test]
    async fn a_resumed_update_tells_how_many_responses_were_archived() {
        let data = tempfile::tempdir().unwrap();
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        run_update(data.path(), &interrupted).await.unwrap_err();
        let archived = fs::read_dir(update_dir(data.path(), "000001"))
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                name.to_string_lossy().ends_with(".json.gz")
            })
            .count();

        let events = events(data.path(), &FakeSteam::new(&apps(600))).await;

        assert!(archived > 0);
        assert!(matches!(
            events[0],
            Progress::Resumed { responses, .. } if responses == archived
        ));
        assert_eq!(events[1], Progress::ReadingApps);
    }

    #[tokio::test]
    async fn the_update_runs_on_another_task() {
        let data = tempfile::tempdir().unwrap();
        let dir = data.path().to_path_buf();
        let steam = FakeSteam::new(&[(1, 10)]);

        let task = tokio::spawn(async move {
            update_catalog(&dir, &steam, &Config::default(), |_| {})
                .await
                .map(|updated| updated.games)
        });

        assert_eq!(task.await.unwrap().unwrap(), 1);
    }

    #[tokio::test]
    async fn the_catalog_learns_the_currency_of_the_store() {
        let data = tempfile::tempdir().unwrap();
        run_update(
            data.path(),
            &FakeSteam::pricing(&[(1, 10), (2, 20)], "$9.99"),
        )
        .await
        .unwrap();

        let catalog = load_catalog(&data.path().join("catalog.sqlite")).unwrap();

        assert_eq!(catalog.freshness.region, Region::ITALY);
        assert_eq!(
            catalog.currency,
            Currency {
                prefix: "$".to_string(),
                suffix: String::new(),
                decimal_separator: Some('.'),
                group_separator: None,
            }
        );
    }

    #[tokio::test]
    async fn an_incremental_update_learns_the_currency_from_the_archive_too() {
        let data = tempfile::tempdir().unwrap();
        run_update(
            data.path(),
            &FakeSteam::pricing(&[(1, 10), (2, 20)], "$9.99"),
        )
        .await
        .unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();

        let catalog = load_catalog(&data.path().join("catalog.sqlite")).unwrap();

        assert_eq!(catalog.currency.prefix, "$");
    }

    #[tokio::test]
    async fn the_update_asks_and_writes_the_region_of_the_config() {
        let data = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(&[(1, 10)]);
        let us = Region::new("US").unwrap();
        let config = Config {
            region: us,
            ..Config::default()
        };

        update_catalog(data.path(), &steam, &config, |_| {})
            .await
            .unwrap();

        assert_eq!(*steam.countries.lock().unwrap(), ["US"]);
        let catalog = load_catalog(&data.path().join("catalog.sqlite")).unwrap();
        assert_eq!(catalog.freshness.region, us);
        assert_eq!(manifests(data.path())[0]["region"], "US");
    }

    #[tokio::test]
    async fn without_formatted_prices_the_currency_is_italian() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)]))
            .await
            .unwrap();

        let catalog = load_catalog(&data.path().join("catalog.sqlite")).unwrap();

        assert_eq!(catalog.currency, Currency::italian());
    }

    #[tokio::test]
    async fn steam_down_keeps_the_old_catalog_and_the_next_update_resumes() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)]))
            .await
            .unwrap();
        let catalog = fs::read(data.path().join("catalog.sqlite")).unwrap();

        let before = wall_clock();
        let error = run_update(data.path(), &FakeSteam::failing(&[(1, 11)], Failure::Down))
            .await
            .unwrap_err();
        let after = wall_clock();

        let day = 24 * HOUR_MS;
        let UpdateError::Stopped { resume_before, .. } = error else {
            panic!("expected a stopped update, got {error:?}");
        };
        assert!((before.0 + day..=after.0 + day).contains(&resume_before.0));
        assert_eq!(
            fs::read(data.path().join("catalog.sqlite")).unwrap(),
            catalog
        );
        assert_eq!(manifests(data.path())[1].get("completedAt"), None);

        let steam = FakeSteam::new(&[(1, 12)]);
        run_update(data.path(), &steam).await.unwrap();
        assert_eq!(steam.asked_items(), [1]);
        assert_eq!(manifests(data.path()).len(), 2);
        assert!(manifests(data.path())[1].get("completedAt").is_some());
    }

    #[tokio::test]
    async fn a_resumed_update_does_not_ask_again_what_it_had() {
        let data = tempfile::tempdir().unwrap();
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        run_update(data.path(), &interrupted).await.unwrap_err();
        let first_batch = interrupted.asked_items()[..260].to_vec();

        let steam = FakeSteam::new(&apps(600));
        run_update(data.path(), &steam).await.unwrap();

        let asked = steam.asked_items();
        assert_eq!(asked.len(), 600 - 260);
        assert!(first_batch.iter().all(|id| !asked.contains(id)));
        assert_eq!(game_ids(data.path()).len(), 600);
    }

    #[tokio::test]
    async fn an_unexpected_shape_marks_the_update_failed_and_the_next_starts_over() {
        let data = tempfile::tempdir().unwrap();

        let error = run_update(data.path(), &FakeSteam::failing(&[(1, 10)], Failure::Shape))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            UpdateError::StartOver {
                not_marked: None,
                ..
            }
        ));
        assert!(
            error
                .to_string()
                .contains("the next update will start over")
        );
        assert!(manifests(data.path())[0].get("failedAt").is_some());
        assert!(!data.path().join("catalog.sqlite").exists());

        let steam = FakeSteam::new(&[(1, 10)]);
        run_update(data.path(), &steam).await.unwrap();
        assert_eq!(steam.asked_items(), [1]);
        assert_eq!(manifests(data.path()).len(), 1);
        assert_eq!(manifests(data.path())[0]["full"], true);
    }

    #[tokio::test]
    async fn an_interrupted_update_is_pending_until_it_completes() {
        let data = tempfile::tempdir().unwrap();
        let config = Config::default();
        assert_eq!(pending_update(data.path(), &config), None);

        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        run_update(data.path(), &interrupted).await.unwrap_err();
        let pending = pending_update(data.path(), &config).unwrap();
        run_update(data.path(), &FakeSteam::new(&apps(600)))
            .await
            .unwrap();

        assert!(pending.responses > 0);
        assert_eq!(pending_update(data.path(), &config), None);
    }

    #[tokio::test]
    async fn rebuild_gives_the_catalog_of_the_last_update() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 11), (3, 30)]))
            .await
            .unwrap();
        let catalog = data.path().join("catalog.sqlite");
        let updated = data.path().join("updated.sqlite");
        fs::rename(&catalog, &updated).unwrap();

        let rebuilt = rebuild_catalog(data.path()).await.unwrap();

        assert_eq!(game_ids(data.path()), [1, 3]);
        assert_eq!(rebuilt.dir, update_dir(data.path(), "000002"));
        assert_eq!((rebuilt.games, rebuilt.apps), (2, 2));
        assert_eq!(
            load_catalog(&catalog).unwrap().games(),
            load_catalog(&updated).unwrap().games()
        );
    }

    #[tokio::test]
    async fn rebuild_without_a_completed_update_says_to_run_update() {
        let data = tempfile::tempdir().unwrap();
        let _ = run_update(data.path(), &FakeSteam::failing(&[(1, 10)], Failure::Down)).await;

        let error = rebuild_catalog(data.path()).await.unwrap_err();

        assert!(error.to_string().contains("run update first"));
    }

    #[tokio::test]
    async fn rebuild_with_an_app_no_update_answers_fails() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 21)]))
            .await
            .unwrap();
        fs::remove_dir_all(update_dir(data.path(), "000001")).unwrap();

        let error = rebuild_catalog(data.path()).await.unwrap_err();

        assert!(matches!(error, UpdateError::Uncovered { app: AppId(1) }));
        assert!(
            error
                .to_string()
                .starts_with("app 1 has no response in the archive")
        );
    }

    #[tokio::test]
    async fn rebuild_needs_only_the_updates_that_answer_the_listed_apps() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 11), (2, 21)]))
            .await
            .unwrap();
        fs::remove_dir_all(update_dir(data.path(), "000001")).unwrap();

        rebuild_catalog(data.path()).await.unwrap();

        assert_eq!(game_ids(data.path()), [1, 2]);
    }

    fn tags_updated_at(data_dir: &Path) -> Timestamp {
        load_catalog(&data_dir.join("catalog.sqlite"))
            .unwrap()
            .freshness
            .tags_updated_at
    }

    fn started_at(data_dir: &Path, number: &str) -> Timestamp {
        let manifest: Value = serde_json::from_slice(
            &fs::read(update_dir(data_dir, number).join("update.json")).unwrap(),
        )
        .unwrap();
        let text = manifest["startedAt"].as_str().unwrap();
        Timestamp(text.parse::<jiff::Timestamp>().unwrap().as_millisecond())
    }

    #[tokio::test]
    async fn the_tags_are_as_old_as_the_oldest_response_in_the_catalog() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 11), (2, 20)]))
            .await
            .unwrap();

        assert_eq!(
            tags_updated_at(data.path()),
            started_at(data.path(), "000001")
        );

        run_update(data.path(), &FakeSteam::new(&[(1, 11), (2, 21)]))
            .await
            .unwrap();

        assert_eq!(
            tags_updated_at(data.path()),
            started_at(data.path(), "000002")
        );
    }

    #[tokio::test]
    async fn an_update_keeps_the_full_one_until_every_app_was_asked_again() {
        let data = tempfile::tempdir().unwrap();
        let config = Config {
            kept_updates: 1,
            ..Config::default()
        };
        for apps in [[(1, 10), (2, 20)], [(1, 11), (2, 20)], [(1, 11), (2, 21)]] {
            update_catalog(data.path(), &FakeSteam::new(&apps), &config, |_| {})
                .await
                .unwrap();
        }

        let mut folders: Vec<_> = fs::read_dir(data.path().join("archive"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        folders.sort();

        assert_eq!(folders, ["000002", "000003"]);
        assert_eq!(game_ids(data.path()), [1, 2]);
    }

    fn backdate(data_dir: &Path, number: &str, days: i64) {
        let path = update_dir(data_dir, number).join("update.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let earlier = started_at(data_dir, number).0 - days * 24 * HOUR_MS;
        manifest["startedAt"] = json!(
            jiff::Timestamp::from_millisecond(earlier)
                .unwrap()
                .to_string()
        );
        fs::write(path, manifest.to_string()).unwrap();
    }

    fn renewing(requests: u32) -> Config {
        Config {
            renewal_requests: requests,
            ..Config::default()
        }
    }

    async fn renew(
        data_dir: &Path,
        steam: &FakeSteam,
        config: &Config,
    ) -> Result<Updated, UpdateError> {
        renew_catalog(data_dir, steam, config, |_| {}).await
    }

    fn sorted(mut ids: Vec<u32>) -> Vec<u32> {
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn an_update_long_after_the_full_one_stays_incremental_and_renews_nothing() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        backdate(data.path(), "000001", 40);

        let steam = FakeSteam::new(&[(1, 10), (2, 21)]);
        let updated = run_update(data.path(), &steam).await.unwrap();

        assert_eq!(steam.asked_items(), [2]);
        assert!(updated.renewal.is_none());
        let manifest = &manifests(data.path())[1];
        assert_eq!(manifest["full"], false);
        assert_eq!(manifest.get("renewal"), None);
    }

    #[tokio::test]
    async fn a_renewal_asks_the_changed_apps_and_the_expired_ones() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        backdate(data.path(), "000001", 31);
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 21)]))
            .await
            .unwrap();

        let steam = FakeSteam::new(&[(1, 10), (2, 21), (3, 30)]);
        let updated = renew(data.path(), &steam, &Config::default())
            .await
            .unwrap();

        assert_eq!(sorted(steam.asked_items()), [1, 3]);
        assert_eq!(game_ids(data.path()), [1, 2, 3]);
        let renewal = updated.renewal.unwrap();
        assert_eq!(renewal.renewed_apps(), 1);
        assert_eq!(renewal.still_expired, 0);
        let manifest = &manifests(data.path())[2];
        assert_eq!(manifest["full"], false);
        assert_eq!(manifest["renewal"], true);
        assert_eq!(manifest["renewedApps"], 1);
    }

    #[tokio::test]
    async fn a_renewal_stops_at_its_requests() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&apps(600)))
            .await
            .unwrap();
        backdate(data.path(), "000001", 31);

        let steam = FakeSteam::new(&apps(600));
        let updated = renew(data.path(), &steam, &renewing(1)).await.unwrap();

        assert_eq!(sorted(steam.asked_items()), (1..=260).collect::<Vec<_>>());
        assert_eq!(updated.asked, 260);
        assert_eq!(updated.renewal.unwrap().still_expired, 340);
        assert_eq!(game_ids(data.path()).len(), 600);
    }

    #[tokio::test]
    async fn a_resumed_renewal_asks_the_same_apps() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&apps(600)))
            .await
            .unwrap();
        backdate(data.path(), "000001", 31);
        let config = renewing(2);
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        renew(data.path(), &interrupted, &config).await.unwrap_err();
        let first_batch = interrupted.asked_items()[..260].to_vec();

        let steam = FakeSteam::new(&apps(600));
        let updated = update_catalog(data.path(), &steam, &config, |_| {})
            .await
            .unwrap();

        let resumed = steam.asked_items();
        assert_eq!(resumed.len(), 260);
        assert!(first_batch.iter().all(|id| !resumed.contains(id)));
        let mut asked = first_batch;
        asked.extend(resumed);
        assert_eq!(sorted(asked), (1..=520).collect::<Vec<_>>());
        assert_eq!(updated.renewal.unwrap().renewed_apps(), 520);
        assert_eq!(manifests(data.path()).len(), 2);
    }

    #[tokio::test]
    async fn the_first_renewal_is_a_full_update() {
        let data = tempfile::tempdir().unwrap();

        let steam = FakeSteam::new(&[(1, 10), (2, 20)]);
        let updated = renew(data.path(), &steam, &Config::default())
            .await
            .unwrap();

        assert_eq!(sorted(steam.asked_items()), [1, 2]);
        assert!(updated.renewal.is_none());
        let manifest = &manifests(data.path())[0];
        assert_eq!(manifest["full"], true);
        assert_eq!(manifest.get("renewal"), None);
    }

    #[tokio::test]
    async fn rebuild_with_a_missing_response_names_the_endpoint() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)]))
            .await
            .unwrap();
        remove_responses(
            &update_dir(data.path(), "000001"),
            "IStoreService_GetAppList",
        );

        let error = rebuild_catalog(data.path()).await.unwrap_err();

        assert!(matches!(
            error,
            UpdateError::Steam(SteamError::NotArchived { .. })
        ));
        assert!(error.to_string().contains("IStoreService/GetAppList/v1"));
    }
}
