use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use gamepalate_core::{
    catalog::Catalog,
    config::{Config, RelevanceConfig},
    effective_hours::effective_hours,
    evaluation::{GroupSummary, evaluate},
    filters::{FilterContext, FilterOptions, active_filters, apply_filters},
    game_vector::{GameVector, game_vectors},
    off_catalog::OffCatalogGames,
    opinion::{opinion_model, opinions},
    ranking::ranking,
    search::{Recommendation, SearchInputs, SearchOptions, SortBy, recommend},
    taste::{Taste, family_taste, game_name, signals_taste},
    timestamp::Timestamp,
    votes::{VoteKind, Votes},
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
    print_opinions(&votes, &catalog, &off_catalog);

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

    let filter_context = FilterContext::new(&signals, &families, &votes, &corrections);
    print_filters(&catalog, &filter_context, &config);

    let inputs = SearchInputs {
        catalog: &catalog,
        game_vectors: &vectors,
        signals: &signals,
        votes: &votes,
        corrections: &corrections,
        families: &families,
        off_catalog: &off_catalog,
        config: &config,
    };
    print_recommendations(&inputs);
    print_evaluation(&inputs);

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

fn print_opinions(votes: &Votes, catalog: &Catalog, off_catalog: &OffCatalogGames) {
    const RUNS: u32 = 10;

    println!("\n== opinions");

    let start = Instant::now();
    let Some(model) = opinion_model(votes, catalog) else {
        println!("not enough votes");
        return;
    };
    println!("training: {:?}", start.elapsed());
    println!("used tags: {}", model.tags.len());

    let mut times = Vec::new();
    let mut all = Vec::new();
    for _ in 0..RUNS {
        let start = Instant::now();
        all = opinions(&model, catalog);
        times.push(start.elapsed());
    }
    println!("opinions best of {RUNS}: {:?}", times.iter().min().unwrap());
    println!("opinions mean: {:?}", times.iter().sum::<Duration>() / RUNS);

    let mut unvoted: Vec<_> = catalog
        .games()
        .iter()
        .zip(&all)
        .filter(|(game, _)| !votes.contains_key(&game.id))
        .collect();
    unvoted.sort_by(|a, b| b.1.total_cmp(a.1));

    println!("highest:");
    for (game, opinion) in unvoted.iter().take(10) {
        println!(
            "{opinion:>8.2}  {}",
            game_name(game.id, catalog, off_catalog)
        );
    }
    println!("lowest:");
    for (game, opinion) in unvoted.iter().rev().take(10) {
        println!(
            "{opinion:>8.2}  {}",
            game_name(game.id, catalog, off_catalog)
        );
    }
}

fn now() -> Timestamp {
    Timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64),
    )
}

fn print_filters(catalog: &Catalog, context: &FilterContext, config: &Config) {
    let now = now();
    let cases = [
        ("default", FilterOptions::default()),
        (
            "free",
            FilterOptions {
                max_price_cents: Some(0),
                min_reviews: Some(0),
                ..FilterOptions::default()
            },
        ),
        (
            "roguelike, discount at least 50%",
            FilterOptions {
                tags: vec!["Roguelike".to_string()],
                min_discount_percent: Some(50),
                ..FilterOptions::default()
            },
        ),
        (
            "souls-like, at most 10 reviews",
            FilterOptions {
                tags: vec!["Souls-like".to_string()],
                max_reviews: Some(10),
                ..FilterOptions::default()
            },
        ),
        (
            "unknown tag",
            FilterOptions {
                tags: vec!["Not a tag".to_string()],
                ..FilterOptions::default()
            },
        ),
    ];

    println!("\n== filters");
    for (name, options) in cases {
        let filters = match active_filters(&options, config, catalog) {
            Ok(filters) => filters,
            Err(error) => {
                println!("{name}: {error:?}");
                continue;
            }
        };

        let start = Instant::now();
        let outcome = apply_filters(catalog, &filters, context, now);
        let elapsed = start.elapsed();

        let blame = outcome.blame.map_or_else(
            || "none".to_string(),
            |blame| format!("{} ({} games)", blame.filter, blame.discarded),
        );
        println!(
            "{name}: {} candidates, blame {blame}, {elapsed:?}",
            outcome.candidates.len()
        );
    }
}

fn print_recommendations(inputs: &SearchInputs) {
    let mut cases = vec![
        ("default", SearchOptions::default()),
        (
            "only bets",
            SearchOptions {
                only_bets: true,
                ..SearchOptions::default()
            },
        ),
        (
            "by discount",
            SearchOptions {
                sort: SortBy::Discount,
                ..SearchOptions::default()
            },
        ),
        (
            "impossible filters",
            SearchOptions {
                filters: FilterOptions {
                    max_price_cents: Some(0),
                    min_reviews: Some(1_000_000),
                    ..FilterOptions::default()
                },
                ..SearchOptions::default()
            },
        ),
        (
            "unknown family",
            SearchOptions {
                family: Some("not a family".to_string()),
                ..SearchOptions::default()
            },
        ),
    ];
    if let Some(family) = inputs.families.families.first() {
        cases.push((
            "first family",
            SearchOptions {
                family: Some(family.name.to_lowercase()),
                ..SearchOptions::default()
            },
        ));
    }

    for (name, options) in cases {
        println!(
            "
== recommendation: {name}"
        );
        let start = Instant::now();
        match recommend(inputs, &options, now()) {
            Ok(recommendation) => {
                println!("time: {:?}", start.elapsed());
                print_recommendation(&recommendation);
            }
            Err(error) => println!("error: {error:?}"),
        }
    }
}

fn print_recommendation(recommendation: &Recommendation) {
    if recommendation.stale_prices {
        println!("prices are stale: {:?}", recommendation.freshness);
    }
    if recommendation.without_vote_model {
        println!("not enough votes for the opinion");
    }
    if recommendation.games.is_empty() {
        match &recommendation.empty_because {
            Some(blame) => println!(
                "no games, blame {} ({} games)",
                blame.filter, blame.discarded
            ),
            None => println!("no games"),
        }
        return;
    }

    for game in &recommendation.games {
        let price = match game.price.discount {
            Some(discount) => format!(
                "{:.2} € (-{}%)",
                f64::from(game.price.final_cents()) / 100.0,
                discount.percent
            ),
            None => format!("{:.2} €", f64::from(game.price.final_cents()) / 100.0),
        };
        let from = game.from_family.as_deref().unwrap_or("signals");
        let bet = if game.bet { "  [bet]" } else { "" };
        println!(
            "{:>3}. {}  {price}  from {from}, relevance {:.2}, affinity {:.2}{bet}",
            game.position, game.name, game.relevance, game.affinity
        );

        if let Some(similar) = &game.similar_to {
            match game.similar_to_multiplier {
                Some(multiplier) => println!("     similar to {similar} (x{multiplier})"),
                None => println!("     similar to {similar}"),
            }
        }
        if !game.top_tags.is_empty() {
            println!("     tags: {}", game.top_tags.join(", "));
        }
    }
}

fn print_evaluation(inputs: &SearchInputs) {
    println!("\n== evaluation");
    let start = Instant::now();
    let evaluation = evaluate(inputs);
    println!("time: {:?}", start.elapsed());
    println!("catalog: {} games", evaluation.catalog_size);

    print_group("kept aside", &evaluation.kept_aside);
    print_group("guardrail", &evaluation.guardrail);
    print_group("families", &evaluation.families);
}

fn print_group(name: &str, group: &GroupSummary) {
    let average = |value: Option<f64>| value.map_or("-".to_string(), |value| format!("{value:.1}"));
    println!("\n-- {name}: {} games", group.games.len());
    println!(
        "top 100: {}, top 1000: {}, median: {}, geometric mean: {}, without position: {}",
        group.in_top,
        group.in_top_wide,
        average(group.median),
        average(group.geometric_mean),
        group.without_position
    );

    for game in &group.games {
        let position = game
            .position
            .map_or("-".to_string(), |position| position.to_string());
        let mut details = Vec::new();
        if let Some(hours) = game.effective_hours {
            details.push(format!("{hours:.1} h"));
        }
        if !game.families.is_empty() {
            details.push(game.families.join(", "));
        }
        let details = if details.is_empty() {
            String::new()
        } else {
            format!("  ({})", details.join(", "))
        };
        println!("{position:>7}  {}{details}", game.name);
    }
}
