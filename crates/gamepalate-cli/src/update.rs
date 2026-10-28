use std::{collections::HashMap, path::Path};

use anyhow::Context;
use futures::StreamExt;
use gamepalate_core::{
    catalog::{Catalog, Freshness, compute_idf},
    config::Config,
    game::Game,
    ids::AppId,
    timestamp::Timestamp,
};
use gamepalate_steam::{
    apps::get_app_list,
    archive::{Archive, UpdateFolder},
    items::{batch_count, item_batches},
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
    price_change_numbers: HashMap<AppId, u64>,
}

pub fn archive(data_dir: &Path) -> Archive {
    Archive::new(data_dir.join("archive"))
}

pub fn run(data_dir: &Path) -> anyhow::Result<()> {
    let resume_within_hours = match load_config(&data_dir.join("config.toml")) {
        Err(ConfigError::NotFound(_)) => Config::default(),
        config => config?,
    }
    .prices_max_age_hours;
    let transport = Transport::new(std::env::var("STEAM_API_KEY").ok())?;
    let update = archive(data_dir)
        .start_update(wall_clock(), resume_within_hours)
        .context("could not start a new update in the archive")?;
    let now = update.started_at();
    if update.resumed_responses() > 0 {
        println!(
            "Resuming the update started {}: {} responses already archived.",
            date_time(now),
            integer(update.resumed_responses() as u64)
        );
    }

    let result = build_catalog(&update.archiving(&transport), now);
    let built = match result {
        Ok(built) => built,
        Err(error) => return Err(stopped(update, error, resume_within_hours)),
    };
    built.write(data_dir)?;
    update
        .complete(wall_clock())
        .context("could not mark the update as completed in the archive")?;

    println!(
        "Catalog updated: {} games out of {} apps.",
        integer(built.catalog.games().len() as u64),
        integer(built.apps as u64)
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

fn wall_clock() -> Timestamp {
    Timestamp(jiff::Timestamp::now().as_millisecond())
}

pub fn build_catalog<S: Get>(steam: &S, now: Timestamp) -> anyhow::Result<BuiltCatalog> {
    let runtime = tokio::runtime::Runtime::new()?;

    let tags = runtime.block_on(get_tags(steam))?;
    let apps = runtime.block_on(get_app_list(steam))?;
    let ids: Vec<AppId> = apps.iter().map(|app| app.id).collect();

    let total = batch_count(ids.len());
    let mut games: Vec<Game> = Vec::new();
    runtime.block_on(async {
        let mut stream = item_batches(steam, &ids, LANGUAGE, COUNTRY_CODE, CONCURRENCY);
        let mut done = 0;
        while let Some(items) = stream.next().await {
            games.extend(items?.into_iter().filter_map(|item| item.into_game(now)));
            done += 1;
            eprint!("\rStore data: {done} of {total} requests");
        }
        eprintln!();
        anyhow::Ok(())
    })?;

    let idf = compute_idf(&games);
    let freshness = Freshness {
        prices_updated_at: now,
        tags_updated_at: now,
    };
    Ok(BuiltCatalog {
        catalog: Catalog::new(games, tags, idf, freshness),
        apps: ids.len(),
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
