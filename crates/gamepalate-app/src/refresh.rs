use std::path::Path;

use gamepalate_core::{config::Config, timestamp::Timestamp};
use gamepalate_steam::transport::{Get, Transport};

use crate::{
    account::steam_api_key,
    app::{Gamepalate, OpenError},
    log::failed,
    ready::{ReadyError, check_ready_with},
    update::{
        ArchiveState, Progress, UpdateError, Updated, expired_apps, renew_catalog, update_catalog,
        update_config, wall_clock,
    },
};

const FRESH_PRICES_MS: i64 = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Catalog(Progress),
    Signals,
    Renewal(Progress),
    Opening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub prices: bool,
    pub renewal: bool,
}

impl Plan {
    pub const NOTHING: Plan = Plan {
        prices: false,
        renewal: false,
    };
    pub const EVERYTHING: Plan = Plan {
        prices: true,
        renewal: true,
    };

    pub fn is_nothing(self) -> bool {
        self == Plan::NOTHING
    }
}

pub fn startup_plan(state: &ArchiveState, now: Timestamp) -> Plan {
    let Some(updated_at) = state.updated_at else {
        return Plan::NOTHING;
    };

    Plan {
        prices: state.pending || now.ms_since(updated_at) > FRESH_PRICES_MS,
        renewal: state.expired_apps > 0,
    }
}

#[derive(Debug)]
pub struct Refreshed {
    pub gamepalate: Gamepalate,
    pub prices: Option<Updated>,
    pub renewal: Option<Updated>,
    pub signals_error: Option<ReadyError>,
    pub renewal_error: Option<UpdateError>,
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("another update is running")]
    Locked,

    #[error(transparent)]
    Update(UpdateError),

    #[error(transparent)]
    Open(#[from] OpenError),
}

impl From<UpdateError> for RefreshError {
    fn from(error: UpdateError) -> RefreshError {
        match error {
            UpdateError::Locked => RefreshError::Locked,
            error => RefreshError::Update(error),
        }
    }
}

#[derive(Debug, Default)]
struct Outcome {
    prices: Option<Updated>,
    renewal: Option<Updated>,
    signals_error: Option<ReadyError>,
    renewal_error: Option<UpdateError>,
}

pub async fn refresh(
    data_dir: &Path,
    plan: Plan,
    mut progress: impl FnMut(Step),
) -> Result<Refreshed, RefreshError> {
    let config = update_config(data_dir)?;
    let transport = Transport::new(steam_api_key()).map_err(UpdateError::from)?;
    let outcome = refresh_with(
        data_dir,
        &transport,
        &config,
        plan,
        wall_clock(),
        &mut progress,
    )
    .await?;
    progress(Step::Opening);
    let gamepalate = Gamepalate::open(Some(data_dir.to_path_buf()))?;

    Ok(Refreshed {
        gamepalate,
        prices: outcome.prices,
        renewal: outcome.renewal,
        signals_error: outcome.signals_error,
        renewal_error: outcome.renewal_error,
    })
}

async fn refresh_with<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    plan: Plan,
    now: Timestamp,
    progress: &mut impl FnMut(Step),
) -> Result<Outcome, UpdateError> {
    let mut outcome = Outcome::default();
    if plan.prices {
        let updated = update_catalog(data_dir, steam, config, |step| {
            progress(Step::Catalog(step))
        })
        .await?;
        progress(Step::Signals);
        outcome.signals_error = check_ready_with(data_dir, steam, true, now)
            .await
            .inspect_err(failed("could not read the Signals"))
            .err();
        outcome.prices = Some(updated);
    }

    let renews_already = outcome
        .prices
        .as_ref()
        .is_some_and(|updated| updated.full || updated.renewal.is_some());
    if plan.renewal && !renews_already {
        match renew_expired(data_dir, steam, config, now, progress).await {
            Ok(renewal) => outcome.renewal = renewal,
            Err(error) if outcome.prices.is_some() => outcome.renewal_error = Some(error),
            Err(error) => return Err(error),
        }
    }

    Ok(outcome)
}

async fn renew_expired<S: Get + Sync>(
    data_dir: &Path,
    steam: &S,
    config: &Config,
    now: Timestamp,
    progress: &mut impl FnMut(Step),
) -> Result<Option<Updated>, UpdateError> {
    let expired = expired_apps(data_dir, config.region, now)
        .await
        .inspect_err(failed("could not read the expired apps"))?;
    if expired == 0 {
        return Ok(None);
    }

    renew_catalog(data_dir, steam, config, |step| {
        progress(Step::Renewal(step))
    })
    .await
    .map(Some)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_steam::transport::SteamError;
    use gamepalate_store::{catalog::load_catalog, signals::load_reading};
    use serde_json::{Value, json};

    use super::*;
    use crate::lock::UpdateLock;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const HOUR_MS: i64 = 3_600_000;
    const ITEMS: &str = "IStoreBrowseService";

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

    async fn run_plan(
        dir: &Path,
        steam: &FakeSteam,
        config: &Config,
        plan: Plan,
        now: Timestamp,
    ) -> (Vec<Step>, Result<Outcome, UpdateError>) {
        let mut steps = Vec::new();
        let result =
            refresh_with(dir, steam, config, plan, now, &mut |step| steps.push(step)).await;
        (steps, result)
    }

    async fn run(
        dir: &Path,
        steam: &FakeSteam,
    ) -> (Vec<Step>, Result<Option<ReadyError>, UpdateError>) {
        let (steps, result) = run_plan(dir, steam, &Config::default(), Plan::EVERYTHING, NOW).await;
        (steps, result.map(|outcome| outcome.signals_error))
    }

    fn renew_only() -> Plan {
        Plan {
            prices: false,
            renewal: true,
        }
    }

    fn game_ids(dir: &Path) -> Vec<u32> {
        load_catalog(&dir.join("catalog.sqlite"))
            .unwrap()
            .games()
            .iter()
            .map(|game| game.id.0)
            .collect()
    }

    fn updates(dir: &Path) -> usize {
        fs::read_dir(dir.join("archive")).unwrap().count()
    }

    fn backdate(dir: &Path, number: &str, days: i64) {
        let path = dir.join("archive").join(number).join("update.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let started: jiff::Timestamp = manifest["startedAt"].as_str().unwrap().parse().unwrap();
        let earlier = started.as_millisecond() - days * 24 * HOUR_MS;
        manifest["startedAt"] = json!(
            jiff::Timestamp::from_millisecond(earlier)
                .unwrap()
                .to_string()
        );
        fs::write(path, manifest.to_string()).unwrap();
    }

    fn many(count: u32) -> Vec<u32> {
        (1..=count).collect()
    }

    async fn expired_folder(apps: &[u32]) -> tempfile::TempDir {
        let dir = folder();
        run_plan(
            dir.path(),
            &steam(apps, &[], None),
            &Config::default(),
            Plan::EVERYTHING,
            NOW,
        )
        .await
        .1
        .unwrap();
        backdate(dir.path(), "000001", 31);
        dir
    }

    fn renewals(steps: &[Step]) -> usize {
        steps
            .iter()
            .filter(|step| matches!(step, Step::Renewal(_)))
            .count()
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

        let (steps, result) = run(dir.path(), &steam(&[10], &[(10, 180)], Some(ITEMS))).await;

        assert!(matches!(result, Err(UpdateError::Stopped { .. })));
        assert!(!steps.contains(&Step::Signals));
        assert!(!dir.path().join("signals.json").exists());
    }

    #[tokio::test]
    async fn the_first_update_is_full_and_renews_nothing() {
        let dir = folder();

        let (steps, result) = run_plan(
            dir.path(),
            &steam(&[10, 11], &[], None),
            &Config::default(),
            Plan::EVERYTHING,
            wall_clock(),
        )
        .await;

        let outcome = result.unwrap();
        assert!(outcome.prices.unwrap().full);
        assert!(outcome.renewal.is_none());
        assert_eq!(renewals(&steps), 0);
        assert_eq!(updates(dir.path()), 1);
    }

    #[tokio::test]
    async fn the_renewal_follows_the_prices_and_the_signals_are_read_once() {
        let dir = expired_folder(&[10, 11]).await;
        let now = wall_clock();

        let (steps, result) = run_plan(
            dir.path(),
            &steam(&[10, 11], &[(10, 180)], None),
            &Config::default(),
            Plan::EVERYTHING,
            now,
        )
        .await;

        let outcome = result.unwrap();
        assert!(outcome.prices.is_some());
        assert_eq!(outcome.renewal.unwrap().renewal.unwrap().renewed_apps(), 2);
        assert!(outcome.renewal_error.is_none());
        assert_eq!(updates(dir.path()), 3);
        let signals = steps.iter().position(|step| *step == Step::Signals);
        let first_renewal = steps
            .iter()
            .position(|step| matches!(step, Step::Renewal(_)));
        assert_eq!(
            steps.iter().filter(|step| **step == Step::Signals).count(),
            1
        );
        assert!(signals.unwrap() < first_renewal.unwrap());
        assert_eq!(steps.last(), Some(&Step::Renewal(Progress::Writing)));
        let reading = load_reading(&dir.path().join("signals.json")).unwrap();
        assert_eq!(reading.read_at, now);
    }

    #[tokio::test]
    async fn nothing_expired_means_no_renewal() {
        let dir = folder();
        let steam = steam(&[10, 11], &[], None);
        let config = Config::default();
        run_plan(dir.path(), &steam, &config, Plan::EVERYTHING, NOW)
            .await
            .1
            .unwrap();

        let (steps, result) =
            run_plan(dir.path(), &steam, &config, Plan::EVERYTHING, wall_clock()).await;

        let outcome = result.unwrap();
        assert!(outcome.prices.is_some());
        assert!(outcome.renewal.is_none());
        assert_eq!(renewals(&steps), 0);
        assert_eq!(updates(dir.path()), 2);
    }

    #[tokio::test]
    async fn the_plan_can_leave_out_the_renewal() {
        let dir = expired_folder(&[10, 11]).await;

        let (steps, result) = run_plan(
            dir.path(),
            &steam(&[10, 11], &[], None),
            &Config::default(),
            Plan {
                prices: true,
                renewal: false,
            },
            wall_clock(),
        )
        .await;

        assert!(result.unwrap().renewal.is_none());
        assert_eq!(renewals(&steps), 0);
        assert_eq!(updates(dir.path()), 2);
    }

    #[tokio::test]
    async fn the_plan_can_renew_without_the_prices_or_the_signals() {
        let dir = expired_folder(&[10, 11]).await;
        fs::remove_file(dir.path().join("signals.json")).ok();

        let (steps, result) = run_plan(
            dir.path(),
            &steam(&[10, 11], &[(10, 180)], None),
            &Config::default(),
            renew_only(),
            wall_clock(),
        )
        .await;

        let outcome = result.unwrap();
        assert!(outcome.prices.is_none());
        assert!(outcome.renewal.is_some());
        assert!(steps.iter().all(|step| matches!(step, Step::Renewal(_))));
        assert!(!dir.path().join("signals.json").exists());
        assert_eq!(updates(dir.path()), 2);
    }

    #[tokio::test]
    async fn an_empty_plan_does_nothing() {
        let dir = expired_folder(&[10]).await;

        let (steps, result) = run_plan(
            dir.path(),
            &steam(&[10], &[], None),
            &Config::default(),
            Plan::NOTHING,
            wall_clock(),
        )
        .await;

        let outcome = result.unwrap();
        assert!(outcome.prices.is_none() && outcome.renewal.is_none());
        assert!(steps.is_empty());
        assert_eq!(updates(dir.path()), 1);
    }

    #[tokio::test]
    async fn a_renewal_that_stops_keeps_the_new_prices() {
        let dir = expired_folder(&[10, 11]).await;

        let (_, result) = run_plan(
            dir.path(),
            &steam(&[10, 11], &[], Some(ITEMS)),
            &Config::default(),
            Plan::EVERYTHING,
            wall_clock(),
        )
        .await;

        let outcome = result.unwrap();
        assert!(outcome.prices.is_some());
        assert!(outcome.renewal.is_none());
        assert!(matches!(
            outcome.renewal_error,
            Some(UpdateError::Stopped { .. })
        ));
    }

    #[tokio::test]
    async fn a_renewal_alone_that_stops_is_an_error() {
        let dir = expired_folder(&[10, 11]).await;

        let (_, result) = run_plan(
            dir.path(),
            &steam(&[10, 11], &[], Some(ITEMS)),
            &Config::default(),
            renew_only(),
            wall_clock(),
        )
        .await;

        assert!(matches!(result, Err(UpdateError::Stopped { .. })));
    }

    #[tokio::test]
    async fn a_pending_renewal_is_resumed_by_the_prices_and_not_followed_by_another() {
        let dir = expired_folder(&many(600)).await;
        let config = Config {
            renewal_requests: 1,
            ..Config::default()
        };
        run_plan(
            dir.path(),
            &steam(&many(600), &[], Some(ITEMS)),
            &config,
            renew_only(),
            wall_clock(),
        )
        .await
        .1
        .unwrap_err();
        assert_eq!(updates(dir.path()), 2);

        let (steps, result) = run_plan(
            dir.path(),
            &steam(&many(600), &[], None),
            &config,
            Plan::EVERYTHING,
            wall_clock(),
        )
        .await;

        let outcome = result.unwrap();
        assert!(outcome.prices.unwrap().renewal.is_some());
        assert!(outcome.renewal.is_none());
        assert_eq!(renewals(&steps), 0);
        assert_eq!(updates(dir.path()), 2);
    }

    #[tokio::test]
    async fn a_taken_lock_stops_the_refresh_before_anything_else() {
        let dir = expired_folder(&[10]).await;
        let _held = UpdateLock::take(dir.path()).unwrap();

        for plan in [Plan::EVERYTHING, renew_only()] {
            let (steps, result) = run_plan(
                dir.path(),
                &steam(&[10], &[], None),
                &Config::default(),
                plan,
                wall_clock(),
            )
            .await;

            let error = RefreshError::from(result.unwrap_err());
            assert!(matches!(error, RefreshError::Locked));
            assert!(!steps.contains(&Step::Signals));
            assert_eq!(updates(dir.path()), 1);
        }
    }

    #[test]
    fn other_update_errors_stay_update_errors() {
        let error = RefreshError::from(UpdateError::NoCompletedUpdate);

        assert!(matches!(
            error,
            RefreshError::Update(UpdateError::NoCompletedUpdate)
        ));
    }

    fn state(pending: bool, updated_minutes_ago: Option<i64>, expired_apps: usize) -> ArchiveState {
        ArchiveState {
            pending,
            updated_at: updated_minutes_ago.map(|minutes| Timestamp(NOW.0 - minutes * 60_000)),
            expired_apps,
        }
    }

    fn plan(prices: bool, renewal: bool) -> Plan {
        Plan { prices, renewal }
    }

    #[test]
    fn the_startup_does_nothing_before_the_first_update() {
        assert!(startup_plan(&state(false, None, 0), NOW).is_nothing());
        assert!(startup_plan(&state(true, None, 5), NOW).is_nothing());
    }

    #[test]
    fn the_startup_resumes_a_pending_update_even_when_the_prices_are_recent() {
        assert_eq!(
            startup_plan(&state(true, Some(5), 0), NOW),
            plan(true, false)
        );
        assert_eq!(
            startup_plan(&state(true, Some(5), 9), NOW),
            plan(true, true)
        );
    }

    #[test]
    fn the_startup_updates_the_prices_after_an_hour() {
        assert_eq!(startup_plan(&state(false, Some(30), 0), NOW), Plan::NOTHING);
        assert_eq!(startup_plan(&state(false, Some(60), 0), NOW), Plan::NOTHING);
        assert_eq!(
            startup_plan(&state(false, Some(61), 0), NOW),
            plan(true, false)
        );
    }

    #[test]
    fn the_startup_renews_when_an_app_is_expired() {
        assert_eq!(
            startup_plan(&state(false, Some(30), 1), NOW),
            plan(false, true)
        );
        assert_eq!(
            startup_plan(&state(false, Some(600), 1), NOW),
            Plan::EVERYTHING
        );
    }
}
