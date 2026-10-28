use std::{collections::HashMap, path::Path};

use futures::StreamExt;
use gamepalate_core::{
    catalog::{Catalog, Freshness, compute_idf},
    game::Game,
    ids::AppId,
    timestamp::Timestamp,
};
use gamepalate_steam::{
    apps::get_app_list,
    items::{batch_count, item_batches},
    tags::get_tags,
    transport::Transport,
};
use gamepalate_store::catalog::write_catalog;

use crate::format::{date_time, integer};

const CONCURRENCY: usize = 4;
const LANGUAGE: &str = "english";
const COUNTRY_CODE: &str = "IT";

pub fn run(data_dir: &Path) -> anyhow::Result<()> {
    let now = Timestamp(jiff::Timestamp::now().as_millisecond());
    let transport = Transport::new(std::env::var("STEAM_API_KEY").ok())?;
    let runtime = tokio::runtime::Runtime::new()?;

    let tags = runtime.block_on(get_tags(&transport))?;
    let apps = runtime.block_on(get_app_list(&transport))?;
    let ids: Vec<AppId> = apps.iter().map(|app| app.id).collect();

    let total = batch_count(ids.len());
    let mut games: Vec<Game> = Vec::new();
    runtime.block_on(async {
        let mut stream = item_batches(&transport, &ids, LANGUAGE, COUNTRY_CODE, CONCURRENCY);
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
    let catalog = Catalog::new(games, tags, idf, freshness);
    let price_change_numbers: HashMap<AppId, u64> = apps
        .iter()
        .map(|app| (app.id, app.price_change_number))
        .collect();

    write_catalog(
        &data_dir.join("catalog.sqlite"),
        &catalog,
        &price_change_numbers,
    )?;
    println!(
        "Catalog updated: {} games out of {} apps.",
        integer(catalog.games().len() as u64),
        integer(ids.len() as u64)
    );
    println!("Prices as of {}.", date_time(now));
    Ok(())
}
