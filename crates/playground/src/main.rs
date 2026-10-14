use std::{env, path::PathBuf};

use anyhow::Context;
use gamepalate_store::catalog::load_catalog;

fn main() -> anyhow::Result<()> {
    let path: PathBuf = env::args()
        .nth(1)
        .context("usage: playground <path to catalogo.sqlite>")?
        .into();

    let catalog = load_catalog(&path)?;
    let games = catalog.games();

    let tag_rows: usize = games.iter().map(|game| game.tags.len()).sum();
    let discounted = games
        .iter()
        .filter(|game| game.price.discount.is_some())
        .count();

    println!("games: {}", games.len());
    println!("game tags: {tag_rows}");
    println!("discounted games: {discounted}");
    println!("tag names: {}", catalog.tags.len());
    println!("tags with idf: {}", catalog.idf.len());
    println!("freshness: {:?}", catalog.freshness);

    for game in games.iter().step_by(games.len().div_ceil(5).max(1)) {
        println!(
            "{:?} {:?}: {:?}, {:?}, early access {}, {} tags",
            game.id,
            game.name,
            game.price,
            game.reviews,
            game.is_early_access,
            game.tags.len(),
        );
    }

    Ok(())
}
