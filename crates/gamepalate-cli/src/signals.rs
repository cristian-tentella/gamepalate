use std::path::Path;

use anyhow::Context;
use gamepalate_core::{
    catalog::Catalog,
    config::Config,
    effective_hours::effective_hours,
    families::Families,
    off_catalog::{OffCatalogGames, off_catalog_games, stale_off_catalog, wanted_off_catalog},
    signals::Signals,
    timestamp::Timestamp,
    votes::Votes,
};
use gamepalate_steam::{
    off_catalog::renew_off_catalog,
    signals::{FailedList, OptionalList, read_signals},
    transport::{Get, SteamError, Transport},
};
use gamepalate_store::{
    data_files::DataFiles,
    off_catalog::{load_off_catalog_cache, save_off_catalog_cache},
    signals::{load_reading, save_reading},
};

use crate::{
    format::date_time,
    update::{COUNTRY_CODE, LANGUAGE, wall_clock},
};

pub fn from_steam(data_dir: &Path, files: &mut DataFiles) -> anyhow::Result<()> {
    let transport = Transport::new(std::env::var("STEAM_API_KEY").ok())?;
    let runtime = tokio::runtime::Runtime::new()?;
    let now = wall_clock();

    let (signals, warnings) = runtime.block_on(read(data_dir, &transport, &files.config, now))?;
    for warning in warnings {
        eprintln!("warn: {warning}");
    }
    files.signals = signals;

    files.off_catalog = runtime.block_on(off_catalog(
        data_dir,
        &transport,
        OffCatalogInputs {
            signals: &files.signals,
            votes: &files.votes,
            families: &files.families,
            catalog: &files.catalog,
            config: &files.config,
        },
        now,
    ))?;
    Ok(())
}

struct OffCatalogInputs<'a> {
    signals: &'a Signals,
    votes: &'a Votes,
    families: &'a Families,
    catalog: &'a Catalog,
    config: &'a Config,
}

async fn off_catalog<S: Get>(
    data_dir: &Path,
    steam: &S,
    inputs: OffCatalogInputs<'_>,
    now: Timestamp,
) -> anyhow::Result<OffCatalogGames> {
    let config = inputs.config;
    let hours = effective_hours(inputs.signals, inputs.votes, &config.signals);
    let wanted = wanted_off_catalog(
        &hours,
        inputs.families,
        inputs.catalog,
        config.signals.min_hours,
    );
    let path = data_dir.join("off-catalog-tags.json");
    let mut cache = load_off_catalog_cache(&path);
    let stale = stale_off_catalog(&cache, &wanted, now, config.tags_max_age_days);

    let renewed = renew_off_catalog(steam, &mut cache, &stale, now, LANGUAGE, COUNTRY_CODE)
        .await
        .context("could not read the tags of the games outside the Catalog from Steam")?;
    if renewed {
        save_off_catalog_cache(&path, &cache)
            .with_context(|| format!("could not save the tags to {}", path.display()))?;
    }

    Ok(off_catalog_games(cache))
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
    use std::{collections::BTreeMap, fs, sync::Mutex};

    use gamepalate_core::{
        catalog::Freshness,
        families::Family,
        ids::{AppId, SteamId},
    };
    use serde_json::{Value, json};

    use super::*;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    struct FakeSteam {
        down: bool,
        wishlist_down: bool,
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
            },
        )
    }

    fn signals() -> Signals {
        Signals {
            read_at: NOW,
            library: vec![gamepalate_core::signals::LibraryGame {
                id: AppId(10),
                total_hours: 2.0,
            }],
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
        let cache = load_off_catalog_cache(&dir.path().join("off-catalog-tags.json"));
        assert_eq!(cache[&AppId(20)].fetched_at, NOW);
    }

    #[tokio::test]
    async fn a_fresh_cache_asks_nothing_and_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("off-catalog-tags.json");
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
