use std::{
    collections::{HashMap, HashSet},
    future::{self, Future},
    io,
    path::{Path, PathBuf},
};

use futures::{Stream, StreamExt, stream};
use gamepalate_core::{
    catalog::{Catalog, Freshness, TagFrequencies, compute_idf},
    config::Config,
    currency::Currency,
    ids::AppId,
    incremental::{KnownApps, changed_apps},
    region::Region,
    renewal::{Answers, Band, Renewal},
    starter::StarterCatalog,
    store_item::StoreItem,
    timestamp::Timestamp,
};
pub use gamepalate_steam::items::BATCH_SIZE;
use gamepalate_steam::{
    apps::get_app_list,
    archive::{Archive, ArchivedUpdate, PendingUpdate},
    chain::{ChainError, chain, rotate},
    items::{archived_asked, archived_batches, get_items},
    tags::get_tags,
    transport::{Get, SteamError, Transport},
};
use gamepalate_store::{
    catalog::{CatalogError, write_catalog},
    config::{ConfigError, load_config},
};
use reqwest::Client;

use crate::{
    account::steam_api_key,
    lock::{LockError, UpdateLock},
    log::failed,
    new_version::releases_url,
    starter::first_part,
};

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
    Renewing {
        apps: usize,
        requests: usize,
    },
    /// The requests of the first part, which asks `apps` appids. The
    /// `StoreData` after it count only the requests of the rest.
    FirstPart {
        done: usize,
        total: usize,
        apps: usize,
    },
    StoreData {
        done: usize,
        total: usize,
    },
    Writing,
}

#[derive(Debug)]
pub struct Updated {
    pub full: bool,
    pub games: usize,
    pub apps: usize,
    pub asked: usize,
    pub prices_at: Timestamp,
    pub renewal: Option<Renewal>,
    pub rotate_error: Option<ChainError>,
    /// The Catalog has only the first part, and the update is still pending.
    pub provisional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArchiveState {
    pub pending: bool,
    pub updated_at: Option<Timestamp>,
    pub renewal_due: bool,
    pub provisional: bool,
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

    #[error("another update is running")]
    Locked,

    #[error("could not lock the data folder for the update")]
    Lock(#[source] io::Error),

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
    next_expiry: Option<Timestamp>,
    stopped: bool,
}

struct FirstPart {
    apps: Vec<AppId>,
    tags: Option<TagFrequencies>,
    stop: bool,
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
    logged_update(data_dir, steam, config, false, NO_FIRST_PART, progress).await
}

pub async fn renew_catalog<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    logged_update(data_dir, steam, config, true, NO_FIRST_PART, progress).await
}

const NO_FIRST_PART: Option<future::Ready<Option<StarterCatalog>>> = None;

/// With `first_part` the update stops after the first part and leaves the
/// rest to a resume. Only a new full update awaits the future.
async fn logged_update<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    renewal: bool,
    first_part: Option<impl Future<Output = Option<StarterCatalog>>>,
    progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    let _lock = UpdateLock::take(data_dir).map_err(|error| match error {
        LockError::Taken => UpdateError::Locked,
        LockError::Io(source) => UpdateError::Lock(source),
    })?;
    tracing::info!("update started");
    let updated = update_with(data_dir, steam, config, renewal, first_part, progress)
        .await
        .inspect_err(failed("update stopped"))?;
    if updated.provisional {
        tracing::info!("first part done, {} games", updated.games);
        return Ok(updated);
    }
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
    first_part: Option<impl Future<Output = Option<StarterCatalog>>>,
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
    let update_full = update.full();
    if update.resumed_responses() > 0 {
        progress(Progress::Resumed {
            started_at: now,
            responses: update.resumed_responses(),
        });
    }

    let stop = first_part.is_some();
    if let Some(first_part) = first_part
        && update_full
        && !update.resumed()
        && update.first().is_none()
        && let Some(starter) = first_part.await
    {
        update
            .set_first(&starter.apps, starter.tags.as_ref())
            .map_err(UpdateError::Start)?;
    }
    let first = update.first().map(|apps| FirstPart {
        apps,
        tags: update.first_tags(),
        stop,
    });

    let asking = match update.renewal() {
        true => Asking::Renewal(config.renewal_requests as usize * BATCH_SIZE),
        false => Asking::Changed,
    };
    let result = build_catalog(
        &update.archiving(steam),
        now,
        &earlier,
        config.region,
        asking,
        first,
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
    if built.stopped {
        return Ok(Updated {
            full: update_full,
            games: built.catalog.games().len(),
            apps: built.apps,
            asked: built.asked,
            prices_at: now,
            renewal: None,
            rotate_error: None,
            provisional: true,
        });
    }
    if let Some(renewal) = &built.renewal {
        update.set_renewed_apps(renewal.renewed_apps());
    }
    if let Some(at) = built.next_expiry {
        update.set_next_expiry(at);
    }
    update
        .complete(wall_clock())
        .map_err(UpdateError::Complete)?;
    let rotate_error = rotate(&archive, config.kept_updates).await.err();

    Ok(Updated {
        full: update_full,
        games: built.catalog.games().len(),
        apps: built.apps,
        asked: built.asked,
        prices_at: now,
        renewal: built.renewal,
        rotate_error,
        provisional: false,
    })
}

/// The download of the first run. With the starter catalog of the release
/// it stops after the first part, and a resume does the rest.
pub async fn download_catalog(
    data_dir: &Path,
    progress: impl FnMut(Progress),
) -> Result<Updated, UpdateError> {
    let config = update_config(data_dir)?;
    let transport = Transport::new(steam_api_key())?;
    let client = Client::new();
    let releases_url = releases_url();
    let first_part = first_part(&transport, &client, &releases_url, config.steam_id);

    logged_update(
        data_dir,
        &transport,
        &config,
        false,
        Some(first_part),
        progress,
    )
    .await
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

pub fn renewal_due(data_dir: &Path, region: Region, now: Timestamp) -> Result<bool, UpdateError> {
    let completed = archive(data_dir)
        .completed()
        .map_err(UpdateError::Archive)?;
    let Some(latest) = completed.first() else {
        return Ok(false);
    };
    if latest.region() != region {
        return Ok(false);
    }

    Ok(latest.next_expiry().is_none_or(|at| now > at))
}

pub fn archive_state(data_dir: &Path, now: Timestamp) -> Result<ArchiveState, UpdateError> {
    let config = update_config(data_dir)?;
    let updated_at = archive(data_dir)
        .completed()
        .map_err(UpdateError::Archive)?
        .first()
        .map(ArchivedUpdate::started_at);

    Ok(ArchiveState {
        pending: pending_update(data_dir, &config).is_some(),
        updated_at,
        renewal_due: renewal_due(data_dir, config.region, now)?,
        provisional: archive(data_dir)
            .provisional()
            .map_err(UpdateError::Archive)?,
    })
}

pub async fn rebuild_catalog(data_dir: &Path) -> Result<Rebuilt, UpdateError> {
    let chain = chain(&archive(data_dir)).await?;
    let Some((latest, earlier)) = chain.split_first() else {
        return Err(UpdateError::NoCompletedUpdate);
    };

    let batches = archived_asked(latest)?.collect::<Result<_, _>>()?;
    let built = build_catalog(
        latest,
        latest.started_at(),
        earlier,
        latest.region(),
        Asking::Archived(batches),
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

/// Which apps a catalog build asks Steam for.
enum Asking {
    /// The apps whose price changed since the update before.
    Changed,
    /// The changed apps plus expired ones, up to this many apps in all.
    Renewal(usize),
    /// Exactly these batches: what an archived update really asked.
    Archived(Vec<Vec<AppId>>),
}

async fn build_catalog<S: Get>(
    steam: &S,
    now: Timestamp,
    earlier: &[ArchivedUpdate],
    region: Region,
    asking: Asking,
    first: Option<FirstPart>,
    progress: &mut impl FnMut(Progress),
) -> Result<BuiltCatalog, SteamError> {
    progress(Progress::ReadingApps);
    let tags = get_tags(steam).await?;
    let apps = get_app_list(steam).await?;
    let ids: Vec<AppId> = apps.iter().map(|app| app.id).collect();
    let current = apps.iter().map(|app| (app.id, app.price_change_number));
    let (renewal, asked, archived) = match asking {
        Asking::Archived(batches) => (None, batches.concat(), Some(batches)),
        asking => {
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
            match asking {
                Asking::Renewal(max_apps) => {
                    let renewal = answers(earlier)?.choose(&ids, &changed, now, max_apps);
                    let asked = renewal.asked.clone();
                    (Some(renewal), asked, None)
                }
                _ => (None, changed, None),
            }
        }
    };
    let (asked, first_apps) = match &first {
        Some(first) => put_first(&first.apps, asked),
        None => (asked, 0),
    };
    let stopped = first_apps > 0 && first.as_ref().is_some_and(|first| first.stop);
    let (first_part, rest) = asked.split_at(first_apps);
    let (first_batches, rest_batches): (Vec<&[AppId]>, Vec<&[AppId]>) = match &archived {
        Some(batches) => (Vec::new(), batches.iter().map(Vec::as_slice).collect()),
        None => (
            first_part.chunks(BATCH_SIZE).collect(),
            rest.chunks(BATCH_SIZE).collect(),
        ),
    };

    let mut items: Vec<StoreItem> = Vec::new();
    if first_apps > 0 {
        let total = first_batches.len();
        let apps = first_apps;
        progress(Progress::FirstPart {
            done: 0,
            total,
            apps,
        });
        let mut stream = batches(steam, first_batches, region.as_str());
        let mut done = 0;
        while let Some(batch) = stream.next().await {
            items.extend(batch?);
            done += 1;
            progress(Progress::FirstPart { done, total, apps });
        }
    }
    let asked = if stopped {
        first_part
    } else {
        let total = rest_batches.len();
        if let Some(renewal) = &renewal {
            progress(Progress::Renewing {
                apps: renewal.renewed_apps(),
                requests: total,
            });
        }
        progress(Progress::StoreData { done: 0, total });
        let mut stream = batches(steam, rest_batches, region.as_str());
        let mut done = 0;
        while let Some(batch) = stream.next().await {
            items.extend(batch?);
            done += 1;
            progress(Progress::StoreData { done, total });
        }
        &asked[..]
    };
    progress(Progress::Writing);

    let mut prices = Vec::new();
    let mut known = KnownApps::new(ids.iter().copied(), now);
    let mut answers = Answers::default();
    prices.extend(formatted_prices(&items));
    answers.add(now, asked, &items);
    known.add(asked, items);
    let mut tags_updated_at = now;
    for update in earlier {
        for batch in archived_batches(update)? {
            let batch = batch?;
            prices.extend(formatted_prices(&batch.items));
            answers.add(update.started_at(), &batch.asked, &batch.items);
            if known.add(&batch.asked, batch.items) > 0 {
                tags_updated_at = update.started_at();
            }
        }
    }
    let currency = Currency::learn(prices.iter().map(|(cents, text)| (*cents, text.as_str())))
        .unwrap_or_else(Currency::italian);

    let games = known.into_games();
    let idf = match first.and_then(|first| first.tags).filter(|_| stopped) {
        Some(tags) => tags.idf_for(&games),
        None if stopped => {
            tracing::info!("no tags in the starter catalog, weighing them on the first part");
            compute_idf(&games)
        }
        None => compute_idf(&games),
    };
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
        next_expiry: answers.next_expiry(&ids, now),
        stopped,
    })
}

/// The apps of the first part that are asked anyway, in their order, then
/// the others in theirs; and how many the first part has.
fn put_first(first: &[AppId], asked: Vec<AppId>) -> (Vec<AppId>, usize) {
    let listed: HashSet<AppId> = asked.iter().copied().collect();
    let mut taken = HashSet::new();
    let mut ordered: Vec<AppId> = first
        .iter()
        .copied()
        .filter(|app| listed.contains(app) && taken.insert(*app))
        .collect();
    let first_apps = ordered.len();
    ordered.extend(asked.into_iter().filter(|app| !taken.contains(app)));
    (ordered, first_apps)
}

fn batches<'a, S: Get>(
    steam: &'a S,
    batches: Vec<&'a [AppId]>,
    country_code: &'a str,
) -> impl Stream<Item = Result<Vec<StoreItem>, SteamError>> + 'a {
    stream::iter(batches)
        .map(move |batch| get_items(steam, batch, LANGUAGE, country_code))
        .buffer_unordered(CONCURRENCY)
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

    use gamepalate_core::{catalog::idf_from, ids::TagId};
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
        slow_first_batch: bool,
        batches: Mutex<Vec<Batch>>,
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Batch {
        Asked(u32),
        Answered(u32),
    }

    impl FakeSteam {
        fn new(apps: &[(u32, u64)]) -> FakeSteam {
            FakeSteam {
                apps: apps.to_vec(),
                failure: None,
                formatted_price: None,
                asked_items: Mutex::new(Vec::new()),
                countries: Mutex::new(Vec::new()),
                slow_first_batch: false,
                batches: Mutex::new(Vec::new()),
            }
        }

        fn out_of_order(apps: &[(u32, u64)]) -> FakeSteam {
            FakeSteam {
                slow_first_batch: true,
                ..FakeSteam::new(apps)
            }
        }

        fn batches(&self) -> Vec<Batch> {
            self.batches.lock().unwrap().clone()
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
                    if self.slow_first_batch {
                        self.batches.lock().unwrap().push(Batch::Asked(ids[0]));
                        let wait = if earlier_batches == 0 { 10 } else { 1 };
                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                        self.batches.lock().unwrap().push(Batch::Answered(ids[0]));
                    }
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
            "tags": if appid.is_multiple_of(3) {
                json!([{ "tagid": 19, "weight": 100 }, { "tagid": 21, "weight": 50 }])
            } else {
                json!([{ "tagid": 19, "weight": 100 }])
            },
            "basic_info": {
                "developers": [{ "name": format!("Studio {appid}") }],
                "franchises": [{ "name": "Saga" }],
            },
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
    async fn rebuild_after_a_renewal_gives_the_catalog_of_the_renewal() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        backdate(data.path(), "000001", 31);
        renew(
            data.path(),
            &FakeSteam::new(&[(1, 10), (2, 21), (3, 30)]),
            &Config::default(),
        )
        .await
        .unwrap();
        assert_eq!(manifests(data.path())[1]["renewal"], true);
        let catalog = data.path().join("catalog.sqlite");
        let renewed = data.path().join("renewed.sqlite");
        fs::rename(&catalog, &renewed).unwrap();

        rebuild_catalog(data.path()).await.unwrap();

        assert_eq!(game_ids(data.path()), [1, 2, 3]);
        assert_eq!(
            load_catalog(&catalog).unwrap().games(),
            load_catalog(&renewed).unwrap().games()
        );
    }

    #[tokio::test]
    async fn rebuild_after_a_renewal_without_expired_apps() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        renew(
            data.path(),
            &FakeSteam::new(&[(1, 10), (2, 21)]),
            &Config::default(),
        )
        .await
        .unwrap();
        assert_eq!(manifests(data.path())[1]["renewal"], true);
        let catalog = data.path().join("catalog.sqlite");
        let renewed = data.path().join("renewed.sqlite");
        fs::rename(&catalog, &renewed).unwrap();

        rebuild_catalog(data.path()).await.unwrap();

        assert_eq!(
            load_catalog(&catalog).unwrap().games(),
            load_catalog(&renewed).unwrap().games()
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

    #[tokio::test]
    async fn developers_and_franchises_reach_the_catalog_even_from_the_archive() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 11), (2, 20)]))
            .await
            .unwrap();
        let steam = FakeSteam::new(&[(1, 11), (2, 20)]);
        let earlier = archive(data.path()).completed().unwrap();

        let built = build_catalog(
            &steam,
            wall_clock(),
            &earlier,
            Region::ITALY,
            Asking::Changed,
            None,
            &mut |_| {},
        )
        .await
        .unwrap();

        assert!(steam.asked_items().is_empty());
        for id in [1, 2] {
            let game = built.catalog.game(AppId(id)).unwrap();
            assert_eq!(game.developers, [format!("Studio {id}")]);
            assert_eq!(game.franchises, ["Saga"]);
        }
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
        let shift = |text: &Value| {
            let at: jiff::Timestamp = text.as_str().unwrap().parse().unwrap();
            json!(
                jiff::Timestamp::from_millisecond(at.as_millisecond() - days * 24 * HOUR_MS)
                    .unwrap()
                    .to_string()
            )
        };
        manifest["startedAt"] = shift(&manifest["startedAt"]);
        if manifest.get("nextExpiry").is_some() {
            manifest["nextExpiry"] = shift(&manifest["nextExpiry"]);
        }
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

    #[tokio::test]
    async fn a_second_update_does_not_start_while_the_first_holds_the_lock() {
        let data = tempfile::tempdir().unwrap();
        let held = UpdateLock::take(data.path()).unwrap();
        let steam = FakeSteam::new(&[(1, 10)]);

        let error = run_update(data.path(), &steam).await.unwrap_err();
        assert!(matches!(error, UpdateError::Locked));
        let error = renew(data.path(), &steam, &Config::default())
            .await
            .unwrap_err();
        assert!(matches!(error, UpdateError::Locked));
        assert!(steam.asked_items().is_empty());
        assert!(!data.path().join("archive").exists());

        drop(held);
        run_update(data.path(), &steam).await.unwrap();
    }

    #[tokio::test]
    async fn a_failed_update_releases_the_lock() {
        let data = tempfile::tempdir().unwrap();
        let down = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);

        run_update(data.path(), &down).await.unwrap_err();

        assert!(UpdateLock::take(data.path()).is_ok());
    }

    #[tokio::test]
    async fn a_renewal_tells_how_many_apps_and_requests_before_asking() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&apps(600)))
            .await
            .unwrap();
        backdate(data.path(), "000001", 31);
        let mut events = Vec::new();

        renew_catalog(
            data.path(),
            &FakeSteam::new(&apps(600)),
            &renewing(1),
            |progress| events.push(progress),
        )
        .await
        .unwrap();

        assert_eq!(events[0], Progress::ReadingApps);
        assert_eq!(
            events[1],
            Progress::Renewing {
                apps: 260,
                requests: 1
            }
        );
        assert_eq!(events[2], Progress::StoreData { done: 0, total: 1 });
    }

    #[tokio::test]
    async fn the_renewal_is_due_once_an_app_expires() {
        let data = tempfile::tempdir().unwrap();
        let region = Config::default().region;
        assert!(!renewal_due(data.path(), region, wall_clock()).unwrap());
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();
        assert!(!renewal_due(data.path(), region, wall_clock()).unwrap());

        backdate(data.path(), "000001", 31);

        assert!(renewal_due(data.path(), region, wall_clock()).unwrap());
    }

    #[tokio::test]
    async fn every_update_records_when_its_first_app_expires() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)]))
            .await
            .unwrap();
        let first = manifests(data.path())[0]["nextExpiry"].clone();
        backdate(data.path(), "000001", 31);
        run_update(data.path(), &FakeSteam::new(&[(1, 10), (2, 20)]))
            .await
            .unwrap();

        assert!(first.is_string());
        let manifests = manifests(data.path());
        assert!(manifests[1]["nextExpiry"].as_str() < first.as_str());
        assert!(renewal_due(data.path(), Config::default().region, wall_clock()).unwrap());
    }

    #[tokio::test]
    async fn an_update_without_a_next_expiry_makes_the_renewal_due() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)]))
            .await
            .unwrap();
        let path = update_dir(data.path(), "000001").join("update.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest.as_object_mut().unwrap().remove("nextExpiry");
        fs::write(path, manifest.to_string()).unwrap();

        assert!(renewal_due(data.path(), Config::default().region, wall_clock()).unwrap());
    }

    #[tokio::test]
    async fn no_renewal_is_due_in_another_region_than_the_archive_one() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&[(1, 10)]))
            .await
            .unwrap();
        backdate(data.path(), "000001", 31);
        let other = Region::new("US").unwrap();

        assert!(!renewal_due(data.path(), other, wall_clock()).unwrap());
    }

    #[tokio::test]
    async fn the_archive_state_tells_the_last_update_the_pending_one_and_the_renewal() {
        let data = tempfile::tempdir().unwrap();
        let empty = archive_state(data.path(), wall_clock()).unwrap();
        assert_eq!(
            empty,
            ArchiveState {
                pending: false,
                updated_at: None,
                renewal_due: false,
                provisional: false,
            }
        );

        run_update(data.path(), &FakeSteam::new(&apps(600)))
            .await
            .unwrap();
        backdate(data.path(), "000001", 31);
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        renew(data.path(), &interrupted, &Config::default())
            .await
            .unwrap_err();

        let state = archive_state(data.path(), wall_clock()).unwrap();

        assert!(state.pending);
        assert_eq!(state.updated_at, Some(started_at(data.path(), "000001")));
        assert!(state.renewal_due);
    }

    #[tokio::test]
    async fn a_full_update_says_it_was_full() {
        let data = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(&[(1, 10)]);

        assert!(run_update(data.path(), &steam).await.unwrap().full);
        assert!(!run_update(data.path(), &steam).await.unwrap().full);
    }

    fn app_ids(ids: &[u32]) -> Vec<AppId> {
        ids.iter().copied().map(AppId).collect()
    }

    async fn download_with(
        data_dir: &Path,
        steam: &FakeSteam,
        first_part: impl Future<Output = Option<StarterCatalog>>,
        progress: impl FnMut(Progress),
    ) -> Result<Updated, UpdateError> {
        logged_update(
            data_dir,
            steam,
            &Config::default(),
            false,
            Some(first_part),
            progress,
        )
        .await
    }

    fn starter(apps: &[u32], tags: Option<&TagFrequencies>) -> Option<StarterCatalog> {
        Some(StarterCatalog {
            apps: app_ids(apps),
            tags: tags.cloned(),
        })
    }

    async fn download(
        data_dir: &Path,
        steam: &FakeSteam,
        first: Option<&[u32]>,
    ) -> Result<Updated, UpdateError> {
        let first = first.and_then(|apps| starter(apps, None));
        download_with(data_dir, steam, future::ready(first), |_| {}).await
    }

    async fn download_tagged(
        data_dir: &Path,
        steam: &FakeSteam,
        first: &[u32],
        tags: &TagFrequencies,
    ) -> Result<Updated, UpdateError> {
        let first = starter(first, Some(tags));
        download_with(data_dir, steam, future::ready(first), |_| {}).await
    }

    fn whole_catalog_tags() -> TagFrequencies {
        TagFrequencies {
            games: 120_000,
            tags: [(TagId(19), 90_000), (TagId(21), 400), (TagId(492), 30_000)].into(),
        }
    }

    async fn not_asked_again() -> Option<StarterCatalog> {
        panic!("a resumed update must not read the first part again")
    }

    fn range(from: u32, to: u32) -> Vec<u32> {
        (from..=to).collect()
    }

    fn provisional(data_dir: &Path) -> bool {
        archive_state(data_dir, wall_clock()).unwrap().provisional
    }

    #[test]
    fn the_first_part_comes_first_without_doubles_nor_unlisted_apps() {
        let (asked, first_apps) = put_first(&app_ids(&[5, 3, 99, 5, 1]), app_ids(&range(1, 6)));

        assert_eq!(asked, app_ids(&[5, 3, 1, 2, 4, 6]));
        assert_eq!(first_apps, 3);
    }

    #[tokio::test]
    async fn the_first_download_stops_after_the_first_part() {
        let data = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(&apps(12));

        let updated = download(data.path(), &steam, Some(&[12, 3, 99, 6, 9]))
            .await
            .unwrap();

        assert_eq!(steam.asked_items(), [12, 3, 6, 9]);
        assert!(updated.provisional);
        assert_eq!((updated.games, updated.apps, updated.asked), (4, 12, 4));
        assert_eq!(sorted(game_ids(data.path())), [3, 6, 9, 12]);
        let manifest = &manifests(data.path())[0];
        assert_eq!(manifest["first"], json!([12, 3, 99, 6, 9]));
        assert_eq!(manifest.get("completedAt"), None);
        assert!(provisional(data.path()));
        assert!(pending_update(data.path(), &Config::default()).is_some());
        assert!(data.path().join("catalog.bin").exists());
    }

    #[tokio::test]
    async fn the_provisional_catalog_weighs_the_tags_with_the_starter_catalog() {
        let data = tempfile::tempdir().unwrap();
        let tags = whole_catalog_tags();
        download_tagged(
            data.path(),
            &FakeSteam::new(&apps(12)),
            &[3, 6, 9, 1],
            &tags,
        )
        .await
        .unwrap();
        let provisional = load_catalog(&data.path().join("catalog.sqlite")).unwrap();

        run_update(data.path(), &FakeSteam::new(&apps(12)))
            .await
            .unwrap();
        let complete = load_catalog(&data.path().join("catalog.sqlite")).unwrap();

        assert_eq!(provisional.idf, idf_from(&tags));
        assert_ne!(provisional.idf, compute_idf(provisional.games()));
        assert_eq!(
            manifests(data.path())[0]["firstTags"],
            json!({ "games": 120_000, "tags": { "19": 90_000, "21": 400, "492": 30_000 } })
        );
        assert_eq!(complete.idf, compute_idf(complete.games()));
    }

    #[tokio::test]
    async fn an_interrupted_first_part_keeps_the_tags_of_the_starter_catalog() {
        let data = tempfile::tempdir().unwrap();
        let tags = TagFrequencies {
            games: 120_000,
            tags: [(TagId(21), 400)].into(),
        };
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        download_tagged(data.path(), &interrupted, &range(51, 400), &tags)
            .await
            .unwrap_err();

        let steam = FakeSteam::new(&apps(600));
        let updated = download_with(data.path(), &steam, not_asked_again(), |_| {})
            .await
            .unwrap();

        assert!(updated.provisional);
        let provisional = load_catalog(&data.path().join("catalog.sqlite")).unwrap();
        assert_eq!(provisional.idf, tags.idf_for(provisional.games()));
        assert_eq!(provisional.idf[&TagId(19)], 120_000_f64.ln());
        assert_ne!(provisional.idf, compute_idf(provisional.games()));
    }

    #[tokio::test]
    async fn without_tags_the_provisional_catalog_weighs_them_on_its_own_games() {
        let data = tempfile::tempdir().unwrap();
        download(data.path(), &FakeSteam::new(&apps(12)), Some(&[3, 6, 9, 1]))
            .await
            .unwrap();
        let provisional = load_catalog(&data.path().join("catalog.sqlite")).unwrap();

        run_update(data.path(), &FakeSteam::new(&apps(12)))
            .await
            .unwrap();
        let complete = load_catalog(&data.path().join("catalog.sqlite")).unwrap();

        assert_eq!(provisional.idf, compute_idf(provisional.games()));
        assert_eq!(complete.idf, compute_idf(complete.games()));
        assert_ne!(provisional.idf[&TagId(21)], complete.idf[&TagId(21)]);
    }

    #[tokio::test]
    async fn the_resume_asks_only_the_rest_and_completes_the_catalog() {
        let data = tempfile::tempdir().unwrap();
        download(
            data.path(),
            &FakeSteam::new(&apps(600)),
            Some(&range(301, 600)),
        )
        .await
        .unwrap();

        let steam = FakeSteam::new(&apps(600));
        let updated = run_update(data.path(), &steam).await.unwrap();

        assert_eq!(steam.asked_items(), range(1, 300));
        assert!(!updated.provisional);
        assert_eq!((updated.games, updated.asked), (600, 600));
        assert_eq!(game_ids(data.path()).len(), 600);
        let manifests = manifests(data.path());
        assert_eq!(manifests.len(), 1);
        assert!(manifests[0].get("completedAt").is_some());
        assert!(!provisional(data.path()));
        assert_eq!(pending_update(data.path(), &Config::default()), None);
    }

    #[tokio::test]
    async fn the_resume_counts_only_the_requests_of_the_rest() {
        let data = tempfile::tempdir().unwrap();
        let mut first_time = Vec::new();
        download_with(
            data.path(),
            &FakeSteam::new(&apps(600)),
            future::ready(starter(&range(1, 300), None)),
            |progress| first_time.push(progress),
        )
        .await
        .unwrap();

        let events = events(data.path(), &FakeSteam::new(&apps(600))).await;

        let first_part = |done| Progress::FirstPart {
            done,
            total: 2,
            apps: 300,
        };
        assert_eq!(
            first_time,
            [
                Progress::ReadingApps,
                first_part(0),
                first_part(1),
                first_part(2),
                Progress::Writing,
            ]
        );
        assert!(matches!(events[0], Progress::Resumed { .. }));
        assert_eq!(
            events[1..],
            [
                Progress::ReadingApps,
                first_part(0),
                first_part(1),
                first_part(2),
                Progress::StoreData { done: 0, total: 2 },
                Progress::StoreData { done: 1, total: 2 },
                Progress::StoreData { done: 2, total: 2 },
                Progress::Writing,
            ]
        );
    }

    #[tokio::test]
    async fn an_interrupted_first_part_resumes_with_its_order_and_stops_again() {
        let data = tempfile::tempdir().unwrap();
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        download(data.path(), &interrupted, Some(&range(51, 400)))
            .await
            .unwrap_err();
        let first_batch = interrupted.asked_items()[..260].to_vec();
        assert_eq!(first_batch, range(51, 310));

        let steam = FakeSteam::new(&apps(600));
        let updated = download_with(data.path(), &steam, not_asked_again(), |_| {})
            .await
            .unwrap();

        assert_eq!(steam.asked_items(), range(311, 400));
        assert!(updated.provisional);
        assert_eq!(sorted(game_ids(data.path())), range(51, 400));
        assert_eq!(manifests(data.path())[0]["first"], json!(range(51, 400)));
    }

    #[tokio::test]
    async fn a_resume_keeps_the_first_part_even_if_the_list_changed() {
        let data = tempfile::tempdir().unwrap();
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        download(data.path(), &interrupted, Some(&range(51, 400)))
            .await
            .unwrap_err();

        let steam = FakeSteam::new(&apps(600));
        download(data.path(), &steam, Some(&range(1, 600)))
            .await
            .unwrap();

        assert_eq!(steam.asked_items(), range(311, 400));
        assert_eq!(manifests(data.path())[0]["first"], json!(range(51, 400)));
    }

    #[tokio::test]
    async fn a_first_part_already_archived_writes_the_provisional_catalog_at_once() {
        let data = tempfile::tempdir().unwrap();
        download(
            data.path(),
            &FakeSteam::new(&apps(600)),
            Some(&range(1, 300)),
        )
        .await
        .unwrap();
        fs::remove_file(data.path().join("catalog.sqlite")).unwrap();

        let steam = FakeSteam::new(&apps(600));
        let updated = download_with(data.path(), &steam, not_asked_again(), |_| {})
            .await
            .unwrap();

        assert!(steam.asked_items().is_empty());
        assert!(updated.provisional);
        assert_eq!(game_ids(data.path()).len(), 300);
    }

    #[tokio::test(start_paused = true)]
    async fn no_batch_of_the_rest_starts_before_the_first_part_is_done() {
        let data = tempfile::tempdir().unwrap();
        let config = Config::default();
        archive(data.path())
            .start_update(
                wall_clock(),
                config.prices_max_age_hours,
                config.region,
                false,
            )
            .unwrap()
            .set_first(&app_ids(&range(1, 700)), None)
            .unwrap();

        let steam = FakeSteam::out_of_order(&apps(1300));
        run_update(data.path(), &steam).await.unwrap();

        let batches = steam.batches();
        let rest_started = batches
            .iter()
            .position(|batch| *batch == Batch::Asked(701))
            .unwrap();
        for first in [1, 261, 521] {
            let answered = batches
                .iter()
                .position(|batch| *batch == Batch::Answered(first))
                .unwrap();
            assert!(answered < rest_started, "{batches:?}");
        }
        assert!(
            batches
                .iter()
                .position(|batch| *batch == Batch::Answered(261))
                < batches
                    .iter()
                    .position(|batch| *batch == Batch::Answered(1))
        );
    }

    #[tokio::test]
    async fn without_the_list_the_first_download_is_whole() {
        let data = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(&apps(600));

        let updated = download(data.path(), &steam, None).await.unwrap();

        assert_eq!(steam.asked_items().len(), 600);
        assert!(!updated.provisional);
        let manifest = &manifests(data.path())[0];
        assert_eq!(manifest.get("first"), None);
        assert!(manifest.get("completedAt").is_some());
        assert!(!provisional(data.path()));
    }

    #[tokio::test]
    async fn an_update_without_a_first_part_resumes_as_before() {
        let data = tempfile::tempdir().unwrap();
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);
        run_update(data.path(), &interrupted).await.unwrap_err();

        let steam = FakeSteam::new(&apps(600));
        let updated = download_with(data.path(), &steam, not_asked_again(), |_| {})
            .await
            .unwrap();

        assert_eq!(steam.asked_items().len(), 600 - 260);
        assert!(!updated.provisional);
        assert_eq!(manifests(data.path())[0].get("first"), None);
        assert!(manifests(data.path())[0].get("completedAt").is_some());
    }

    #[tokio::test]
    async fn the_update_and_the_renewal_never_read_a_first_part() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&apps(3)))
            .await
            .unwrap();

        let steam = FakeSteam::new(&apps(3));
        let updated = download_with(data.path(), &steam, not_asked_again(), |_| {})
            .await
            .unwrap();

        assert!(!updated.provisional);
        assert_eq!(manifests(data.path())[1].get("first"), None);
    }

    #[tokio::test]
    async fn an_imported_catalog_with_a_stopped_update_is_not_provisional() {
        let data = tempfile::tempdir().unwrap();
        let interrupted = FakeSteam::failing(&apps(600), Failure::DownAfterOneBatch);

        run_update(data.path(), &interrupted).await.unwrap_err();

        let state = archive_state(data.path(), wall_clock()).unwrap();
        assert!(state.pending);
        assert_eq!(state.updated_at, None);
        assert!(!state.provisional);
    }

    #[tokio::test]
    async fn a_first_part_too_old_to_resume_stays_provisional_until_the_whole_catalog() {
        let data = tempfile::tempdir().unwrap();
        download(
            data.path(),
            &FakeSteam::new(&apps(600)),
            Some(&range(1, 300)),
        )
        .await
        .unwrap();
        backdate(data.path(), "000001", 2);

        let state = archive_state(data.path(), wall_clock()).unwrap();
        assert!(!state.pending);
        assert!(state.provisional);

        let steam = FakeSteam::new(&apps(600));
        let updated = run_update(data.path(), &steam).await.unwrap();

        assert_eq!(steam.asked_items().len(), 600);
        assert_eq!(steam.asked_items()[..300], range(1, 300));
        let manifest: Value = serde_json::from_slice(
            &fs::read(update_dir(data.path(), "000002").join("update.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["first"], json!(range(1, 300)));
        assert!(manifest.get("completedAt").is_some());
        assert!(!updated.provisional);
        assert!(!provisional(data.path()));
    }

    #[tokio::test]
    async fn rebuild_after_a_catalog_in_two_parts() {
        let data = tempfile::tempdir().unwrap();
        download(
            data.path(),
            &FakeSteam::new(&apps(600)),
            Some(&range(301, 400)),
        )
        .await
        .unwrap();
        run_update(data.path(), &FakeSteam::new(&apps(600)))
            .await
            .unwrap();
        let catalog = data.path().join("catalog.sqlite");
        let updated = data.path().join("updated.sqlite");
        fs::rename(&catalog, &updated).unwrap();

        rebuild_catalog(data.path()).await.unwrap();

        assert_eq!(
            load_catalog(&catalog).unwrap().games(),
            load_catalog(&updated).unwrap().games()
        );
    }

    #[tokio::test]
    async fn rebuild_after_an_update_with_a_last_batch_that_is_not_full() {
        let data = tempfile::tempdir().unwrap();
        run_update(data.path(), &FakeSteam::new(&apps(600)))
            .await
            .unwrap();

        let rebuilt = rebuild_catalog(data.path()).await.unwrap();

        assert_eq!(rebuilt.games, 600);
    }
}
