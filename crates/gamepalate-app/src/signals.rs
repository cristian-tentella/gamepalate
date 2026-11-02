use std::{
    io,
    path::{Path, PathBuf},
};

use gamepalate_core::{
    catalog::Catalog,
    config::Config,
    effective_hours::effective_hours,
    families::Families,
    ids::SteamId,
    off_catalog::{OffCatalogGames, off_catalog_games, stale_off_catalog, wanted_off_catalog},
    signals::{LibraryGame, Signals, SignalsReading},
    timestamp::Timestamp,
    votes::Votes,
};
use gamepalate_steam::{
    client::{read_played_hours, steam_client_file},
    off_catalog::renew_off_catalog,
    signals::{self as steam_signals, FailedList},
    transport::{Get, SteamError},
};
use gamepalate_store::{
    off_catalog::{load_off_catalog_cache, save_off_catalog_cache},
    signals::{SignalsError, load_reading, save_reading},
};

use crate::update::LANGUAGE;

const SIGNALS_FILE: &str = "signals.json";
const OFF_CATALOG_FILE: &str = "off-catalog-tags.json";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalsWarning {
    SteamDown { read_at: Timestamp },
    ListFailed(FailedList),
    SteamClientFailed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SteamClientHours {
    Off,
    Found(PathBuf),
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadSignals {
    pub reading: SignalsReading,
    pub warnings: Vec<SignalsWarning>,
    pub(crate) fresh: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadSignalsError {
    #[error(
        "could not read the signals from Steam, and {} has no reading to use instead",
        .path.display()
    )]
    NothingSaved {
        path: PathBuf,
        #[source]
        source: SteamError,
    },

    #[error("could not read the signals from Steam")]
    Steam(#[source] SteamError),
}

#[derive(Debug, thiserror::Error)]
pub enum OffCatalogError {
    #[error("could not read the tags of the games outside the Catalog from Steam")]
    Steam(#[source] SteamError),

    #[error("could not save the tags to {}", .path.display())]
    Save {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub struct OffCatalogInputs<'a> {
    pub signals: &'a Signals,
    pub votes: &'a Votes,
    pub families: &'a Families,
    pub catalog: &'a Catalog,
    pub config: &'a Config,
}

impl ReadSignals {
    pub fn signals(&self) -> Signals {
        self.reading.clone().into_signals()
    }
}

impl ReadSignalsError {
    pub fn steam_error(&self) -> &SteamError {
        match self {
            ReadSignalsError::NothingSaved { source, .. } | ReadSignalsError::Steam(source) => {
                source
            }
        }
    }
}

pub fn steam_client_hours(config: &Config, steam_id: SteamId) -> SteamClientHours {
    if config.signals.steam_client_hours != Some(true) {
        return SteamClientHours::Off;
    }
    steam_client_file(steam_id).map_or(SteamClientHours::Missing, SteamClientHours::Found)
}

pub async fn read_signals<S: Get>(
    data_dir: &Path,
    steam: &S,
    steam_id: SteamId,
    client: &SteamClientHours,
    now: Timestamp,
) -> Result<ReadSignals, ReadSignalsError> {
    let path = data_dir.join(SIGNALS_FILE);
    let saved = load_reading(&path).ok();
    let had_saved = saved.is_some();
    let saved_borrowed = saved
        .as_ref()
        .map(|saved| saved.borrowed.clone())
        .unwrap_or_default();

    let read = steam_signals::read_signals(steam, steam_id, saved, now)
        .await
        .map_err(|source| {
            if matches!(source, SteamError::SteamDown { .. }) && !had_saved {
                ReadSignalsError::NothingSaved { path, source }
            } else {
                ReadSignalsError::Steam(source)
            }
        })?;

    let mut warnings: Vec<SignalsWarning> = read
        .saved_from
        .map(|read_at| SignalsWarning::SteamDown { read_at })
        .into_iter()
        .collect();
    warnings.extend(read.failed.into_iter().map(SignalsWarning::ListFailed));

    let fresh = read.saved_from.is_none();
    let mut reading = read.reading;
    if fresh {
        let played = match client {
            SteamClientHours::Off => Ok(Vec::new()),
            SteamClientHours::Found(path) => read_played_hours(path).map_err(|_| ()),
            SteamClientHours::Missing => Err(()),
        };
        reading.borrowed = match played {
            Ok(played) => not_owned(played, &reading.library),
            Err(()) => {
                warnings.push(SignalsWarning::SteamClientFailed);
                saved_borrowed
            }
        };
    }

    Ok(ReadSignals {
        reading,
        warnings,
        fresh,
    })
}

pub(crate) fn not_owned(played: Vec<LibraryGame>, library: &[LibraryGame]) -> Vec<LibraryGame> {
    played
        .into_iter()
        .filter(|game| !library.iter().any(|owned| owned.id == game.id))
        .collect()
}

pub fn save_signals(data_dir: &Path, read: &ReadSignals) -> Result<(), SignalsError> {
    if !read.fresh {
        return Ok(());
    }
    save_reading(&data_dir.join(SIGNALS_FILE), &read.reading)
}

pub async fn off_catalog<S: Get>(
    data_dir: &Path,
    steam: &S,
    inputs: OffCatalogInputs<'_>,
    now: Timestamp,
) -> Result<OffCatalogGames, OffCatalogError> {
    let hours = effective_hours(inputs.signals, inputs.votes, &inputs.config.signals);
    let wanted = wanted_off_catalog(&hours, inputs.families, inputs.catalog);
    let max_age_days = inputs.config.tags_max_age_days;
    let region = inputs.config.region;
    let path = data_dir.join(OFF_CATALOG_FILE);
    let mut cache = load_off_catalog_cache(&path);
    let stale = stale_off_catalog(&cache, &wanted, now, max_age_days);

    let renewed = renew_off_catalog(steam, &mut cache, &stale, now, LANGUAGE, region.as_str())
        .await
        .map_err(OffCatalogError::Steam)?;
    if renewed {
        save_off_catalog_cache(&path, &cache)
            .map_err(|source| OffCatalogError::Save { path, source })?;
    }

    Ok(off_catalog_games(cache))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, sync::Mutex};

    use gamepalate_core::{
        catalog::Freshness, currency::Currency, families::Family, ids::AppId, region::Region,
    };
    use gamepalate_steam::signals::OptionalList;
    use serde_json::{Value, json};

    use super::*;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const STEAM_ID: SteamId = SteamId(76_561_198_000_000_000);

    struct FakeSteam {
        down: bool,
        wishlist_down: bool,
        hidden: bool,
        asked_items: Mutex<Vec<Value>>,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            if endpoint == "IStoreBrowseService/GetItems/v1" {
                let input: Value = serde_json::from_str(params[0].1).unwrap();
                self.asked_items.lock().unwrap().push(input["ids"].clone());
                let items: Vec<Value> = input["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| {
                        json!({
                            "id": id["appid"],
                            "success": 1,
                            "type": 0,
                            "name": format!("Game {}", id["appid"]),
                            "tags": [{ "tagid": 19, "weight": 100 }],
                        })
                    })
                    .collect();
                return Ok(json!({ "response": { "store_items": items } }));
            }
            let down = self.down || self.wishlist_down && endpoint.contains("Wishlist");
            if down {
                return Err(SteamError::SteamDown {
                    endpoint: endpoint.to_string(),
                    reason: "timed out".to_string(),
                    source: None,
                });
            }
            Ok(match endpoint {
                "IPlayerService/GetOwnedGames/v1" if self.hidden => json!({ "response": {} }),
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
            steam_id: Some(STEAM_ID),
            ..Config::default()
        }
    }

    fn steam() -> FakeSteam {
        FakeSteam {
            down: false,
            wishlist_down: false,
            hidden: false,
            asked_items: Mutex::new(Vec::new()),
        }
    }

    fn empty_catalog() -> Catalog {
        Catalog::new(
            Vec::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: NOW,
                tags_updated_at: NOW,
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn signals() -> Signals {
        Signals {
            read_at: NOW,
            library: vec![gamepalate_core::signals::LibraryGame {
                id: AppId(10),
                total_hours: 0.5,
            }],
            borrowed: vec![],
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        }
    }

    fn families() -> Families {
        Families {
            families: vec![Family {
                name: "Roguelike".to_string(),
                games: vec![AppId(20)],
            }],
            held_out: Vec::new(),
        }
    }

    async fn off_catalog_now(dir: &Path, steam: &FakeSteam) -> OffCatalogGames {
        let (signals, votes, families, catalog, config) = (
            signals(),
            Votes::default(),
            families(),
            empty_catalog(),
            config(),
        );
        let inputs = OffCatalogInputs {
            signals: &signals,
            votes: &votes,
            families: &families,
            catalog: &catalog,
            config: &config,
        };
        off_catalog(dir, steam, inputs, NOW).await.unwrap()
    }

    async fn read_and_save(dir: &Path, steam: &FakeSteam) -> ReadSignals {
        read_and_save_with(dir, steam, &SteamClientHours::Off).await
    }

    async fn read_and_save_with(
        dir: &Path,
        steam: &FakeSteam,
        client: &SteamClientHours,
    ) -> ReadSignals {
        let read = read_signals(dir, steam, STEAM_ID, client, NOW)
            .await
            .unwrap();
        save_signals(dir, &read).unwrap();
        read
    }

    #[tokio::test]
    async fn games_outside_the_catalog_are_asked_and_cached() {
        let dir = tempfile::tempdir().unwrap();
        let steam = steam();

        let games = off_catalog_now(dir.path(), &steam).await;

        assert_eq!(
            *steam.asked_items.lock().unwrap(),
            [json!([{ "appid": 10 }, { "appid": 20 }])]
        );
        assert_eq!(games[&AppId(10)].name.as_deref(), Some("Game 10"));
        assert_eq!(games[&AppId(20)].tags.len(), 1);
        let cache = load_off_catalog_cache(&dir.path().join(OFF_CATALOG_FILE));
        assert_eq!(cache[&AppId(20)].fetched_at, NOW);
    }

    #[tokio::test]
    async fn a_fresh_cache_asks_nothing_and_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(OFF_CATALOG_FILE);
        let text = format!(
            r#"{{"10":{{"fetchedAt":{},"tags":[[19,1.0]],"name":"Cached"}},"20":{{"fetchedAt":{},"tags":[]}}}}"#,
            NOW.0 - 86_400_000,
            NOW.0 - 1,
        );
        fs::write(&path, &text).unwrap();
        let steam = steam();

        let games = off_catalog_now(dir.path(), &steam).await;

        assert!(steam.asked_items.lock().unwrap().is_empty());
        assert_eq!(games[&AppId(10)].name.as_deref(), Some("Cached"));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }

    const SAVED: &str = r#"{"readAt":1793000000000,"library":[{"appid":1,"totalHours":3.0}],"recent":[],"wishlist":[400],"followed":null}"#;

    #[tokio::test]
    async fn a_reading_from_steam_is_saved_without_warnings() {
        let dir = tempfile::tempdir().unwrap();

        let read = read_and_save(dir.path(), &steam()).await;

        let signals = read.signals();
        assert_eq!(signals.library[0].id, AppId(10));
        assert_eq!(signals.followed, [AppId(500)]);
        assert!(read.warnings.is_empty());
        let saved = load_reading(&dir.path().join(SIGNALS_FILE)).unwrap();
        assert_eq!(saved.read_at, NOW);
        assert_eq!(saved.followed_read_at, Some(NOW));
    }

    #[tokio::test]
    async fn the_reading_needs_no_config_and_saves_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SIGNALS_FILE);

        let read = read_signals(dir.path(), &steam(), STEAM_ID, &SteamClientHours::Off, NOW)
            .await
            .unwrap();

        assert_eq!(read.signals().library[0].id, AppId(10));
        assert!(!path.exists());
        save_signals(dir.path(), &read).unwrap();
        assert_eq!(load_reading(&path).unwrap(), read.reading);
    }

    #[tokio::test]
    async fn steam_down_uses_the_saved_signals_and_says_when_they_were_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SIGNALS_FILE);
        fs::write(&path, SAVED).unwrap();
        let steam = FakeSteam {
            down: true,
            ..steam()
        };

        let read = read_and_save(dir.path(), &steam).await;

        assert_eq!(read.signals().library[0].id, AppId(1));
        assert_eq!(
            read.warnings,
            [SignalsWarning::SteamDown {
                read_at: Timestamp(1_793_000_000_000)
            }]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), SAVED);
    }

    #[tokio::test]
    async fn steam_down_without_saved_signals_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(SIGNALS_FILE), "not json").unwrap();
        let steam = FakeSteam {
            down: true,
            ..steam()
        };

        let error = read_signals(dir.path(), &steam, STEAM_ID, &SteamClientHours::Off, NOW)
            .await
            .unwrap_err();

        assert!(matches!(error, ReadSignalsError::NothingSaved { .. }));
        assert!(error.to_string().contains("has no reading to use instead"));
        assert!(matches!(error.steam_error(), SteamError::SteamDown { .. }));
    }

    #[tokio::test]
    async fn a_hidden_library_is_recognized() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(SIGNALS_FILE), SAVED).unwrap();
        let steam = FakeSteam {
            hidden: true,
            ..steam()
        };

        let error = read_signals(dir.path(), &steam, STEAM_ID, &SteamClientHours::Off, NOW)
            .await
            .unwrap_err();

        assert!(matches!(
            error.steam_error(),
            SteamError::LibraryHidden { .. }
        ));
    }

    #[tokio::test]
    async fn a_failed_wishlist_says_which_one_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SIGNALS_FILE);
        let steam = FakeSteam {
            wishlist_down: true,
            ..steam()
        };

        let without_saved = read_and_save(dir.path(), &steam).await;
        fs::write(&path, SAVED).unwrap();
        let with_saved = read_and_save(dir.path(), &steam).await;

        let failed = |saved_from| {
            [SignalsWarning::ListFailed(FailedList {
                list: OptionalList::Wishlist,
                saved_from,
            })]
        };
        assert_eq!(without_saved.warnings, failed(None));
        assert_eq!(with_saved.signals().wishlist, [AppId(400)]);
        assert_eq!(
            with_saved.warnings,
            failed(Some(Timestamp(1_793_000_000_000)))
        );
        let saved = load_reading(&path).unwrap();
        assert_eq!(saved.read_at, NOW);
        assert_eq!(saved.wishlist_read_at, Some(Timestamp(1_793_000_000_000)));
    }

    const LOCAL_CONFIG: &str = r#""UserLocalConfigStore" { "Software" { "Valve" { "Steam" { "apps" {
        "10" { "Playtime" "600" }
        "70" { "Playtime" "90" }
    } } } } }"#;

    #[tokio::test]
    async fn the_hours_from_the_steam_app_add_the_games_not_owned() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("localconfig.vdf");
        fs::write(&file, LOCAL_CONFIG).unwrap();

        let read = read_and_save_with(dir.path(), &steam(), &SteamClientHours::Found(file)).await;

        assert!(read.warnings.is_empty());
        let saved = load_reading(&dir.path().join(SIGNALS_FILE)).unwrap();
        assert_eq!(
            saved.borrowed,
            [LibraryGame {
                id: AppId(70),
                total_hours: 1.5,
            }]
        );
        assert_eq!(saved.library[0].total_hours, 2.0);
    }

    #[tokio::test]
    async fn without_the_answer_the_borrowed_games_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(SIGNALS_FILE),
            r#"{"readAt":1,"library":[],"borrowed":[{"appid":70,"totalHours":1.5}],"recent":[],"wishlist":null,"followed":null}"#,
        )
        .unwrap();

        let read = read_and_save(dir.path(), &steam()).await;

        assert!(read.reading.borrowed.is_empty());
    }

    #[tokio::test]
    async fn a_steam_app_file_that_fails_keeps_the_saved_games() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(SIGNALS_FILE),
            r#"{"readAt":1,"library":[],"borrowed":[{"appid":70,"totalHours":1.5}],"recent":[],"wishlist":null,"followed":null}"#,
        )
        .unwrap();
        let broken = dir.path().join("missing.vdf");

        for client in [SteamClientHours::Found(broken), SteamClientHours::Missing] {
            let read = read_and_save_with(dir.path(), &steam(), &client).await;

            assert_eq!(read.warnings, [SignalsWarning::SteamClientFailed]);
            assert_eq!(read.reading.borrowed[0].id, AppId(70));
        }
    }

    #[test]
    fn the_steam_app_is_read_only_after_a_yes() {
        for answer in [None, Some(false)] {
            let mut config = config();
            config.signals.steam_client_hours = answer;

            assert_eq!(steam_client_hours(&config, STEAM_ID), SteamClientHours::Off);
        }
    }
}
