use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Context;
use gamepalate_core::{
    catalog::Catalog,
    config::RelevanceConfig,
    effective_hours::effective_hours,
    game_vector::{GameVector, game_vectors},
    off_catalog::OffCatalogGames,
    ranking::ranking,
    taste::{Taste, family_taste, game_name, signals_taste},
    votes::VoteKind,
};
use gamepalate_store::{
    catalog::load_catalog, config::load_config, corrections::load_corrections,
    families::load_families, off_catalog::load_off_catalog_games, signals::load_signals,
    votes::load_votes,
};

fn main() -> anyhow::Result<()> {
    let data_dir = match env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env::var("APPDATA").context("usage: playground <data folder>")?)
            .join("gamepalate"),
    };
    println!("data folder: {}", data_dir.display());

    let config = load_config(&data_dir.join("config.toml"))?;
    println!("\n== config\n{config:#?}");

    let catalog = load_catalog(&data_dir.join("catalog.sqlite"))?;
    print_catalog(&catalog);
    time_game_vectors(&catalog);
    let vectors = game_vectors(&catalog);

    let signals = load_signals(&data_dir.join("signals.json"))?;
    println!("\n== signals");
    println!("read at: {:?}", signals.read_at);
    println!("library: {}", signals.library.len());
    println!("recent: {}", signals.recent.len());
    println!("wishlist: {}", signals.wishlist.len());
    println!("followed: {}", signals.followed.len());

    let off_catalog = load_off_catalog_games(&data_dir.join("off-catalog-tags.json"));
    let without_tags = off_catalog
        .values()
        .filter(|game| game.tags.is_empty())
        .count();
    println!("\n== off-catalog tags");
    println!("games: {}", off_catalog.len());
    println!("games without tags: {without_tags}");

    let votes = load_votes(&data_dir.join("votes.toml"))?;
    let count = |kind: VoteKind| votes.values().filter(|vote| vote.kind == kind).count();
    println!("\n== votes");
    println!("votes: {}", votes.len());
    println!("would try: {}", count(VoteKind::WouldTry));
    println!("maybe: {}", count(VoteKind::Maybe));
    println!("not for me: {}", count(VoteKind::NotForMe));
    println!("known: {}", count(VoteKind::Known));
    println!("bets: {}", votes.values().filter(|vote| vote.bet).count());

    let hours = effective_hours(&signals, &votes, &config.signals);
    let mut by_hours: Vec<_> = hours.iter().collect();
    by_hours.sort_by(|a, b| b.1.total_cmp(a.1));
    println!("\n== effective hours");
    println!("games: {}", hours.len());
    for &(&id, hours) in by_hours.iter().take(10) {
        println!("{hours:>8.1}  {}", game_name(id, &catalog, &off_catalog));
    }

    let corrections = load_corrections(&data_dir.join("corrections.toml"))?;
    println!("\n== corrections");
    println!("banned tags: {:?}", corrections.banned_tags);
    println!("tag multipliers: {:?}", corrections.tag_multipliers);
    println!("hidden games: {}", corrections.hidden_games.len());
    println!("game multipliers: {:?}", corrections.game_multipliers);

    let families = load_families(&data_dir.join("families.toml"))?;
    let problems = families.problems(&catalog, &off_catalog);
    println!("\n== families");
    for family in &families.families {
        println!("{}: {} games", family.name, family.games.len());
    }
    println!("held out: {}", families.held_out.len());
    println!("problems: {problems:?}");

    let taste = signals_taste(
        &hours,
        &catalog,
        &off_catalog,
        &corrections,
        &config.signals,
    );
    print_taste("signals", &taste, &catalog);
    print_ranking(
        "signals",
        &vectors,
        &taste,
        &config.relevance,
        &catalog,
        &off_catalog,
    );
    for family in &families.families {
        let taste = family_taste(family, &catalog, &off_catalog, &corrections.tag_multipliers);
        print_taste(&family.name, &taste, &catalog);
        print_ranking(
            &family.name,
            &vectors,
            &taste,
            &config.relevance,
            &catalog,
            &off_catalog,
        );
    }

    Ok(())
}

fn print_taste(name: &str, taste: &Taste, catalog: &Catalog) {
    let mut profile: Vec<_> = taste.profile().into_iter().collect();
    profile.sort_by(|a, b| b.1.total_cmp(&a.1));

    println!(
        "
== {name} taste"
    );
    println!("games: {}", taste.games.len());
    for (id, value) in profile.iter().take(10) {
        let tag = catalog
            .tags
            .get(id)
            .map_or_else(|| format!("#{}", id.0), |tag| tag.english.clone());
        println!("{value:>8.2}  {tag}");
    }
}

fn print_ranking(
    name: &str,
    vectors: &[GameVector],
    taste: &Taste,
    relevance_config: &RelevanceConfig,
    catalog: &Catalog,
    off_catalog: &OffCatalogGames,
) {
    println!(
        "
== {name} ranking"
    );

    let start = Instant::now();
    let Some(ranking) = ranking(vectors, taste, relevance_config) else {
        println!("no profile");
        return;
    };
    println!("time: {:?}", start.elapsed());

    for game in ranking.iter().take(20) {
        println!(
            "{:>8.2}  {}",
            game.relevance,
            game_name(game.id, catalog, off_catalog)
        );
    }
}

fn print_catalog(catalog: &Catalog) {
    let games = catalog.games();

    let tag_rows: usize = games.iter().map(|game| game.tags.len()).sum();
    let discounted = games
        .iter()
        .filter(|game| game.price.discount.is_some())
        .count();

    println!("\n== catalog");
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
}

fn time_game_vectors(catalog: &Catalog) {
    const RUNS: u32 = 10;

    let mut times = Vec::new();
    let mut count = 0;
    for _ in 0..RUNS {
        let start = Instant::now();
        count = game_vectors(catalog).len();
        times.push(start.elapsed());
    }

    println!("\n== game vectors");
    println!("games: {count}");
    println!("best of {RUNS}: {:?}", times.iter().min().unwrap());
    println!("mean: {:?}", times.iter().sum::<Duration>() / RUNS);
}
