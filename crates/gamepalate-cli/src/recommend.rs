use std::path::Path;

use anyhow::bail;
use clap::{Args, ValueEnum};
use comfy_table::{CellAlignment, ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};
use gamepalate_core::{
    catalog::Freshness,
    currency::Currency,
    filters::FilterOptions,
    game::Reviews,
    game_vector::game_vectors,
    region::Region,
    search::{RecommendedGame, SearchError, SearchOptions, SortBy, recommend},
    timestamp::Timestamp,
};
use gamepalate_store::data_files::DataFiles;

use crate::{
    format::{date, date_time, decimal, integer, multiplier, parse_price},
    names::{check_families, tag_name_error_text},
};

const NAME_WIDTH: usize = 60;

#[derive(Args)]
pub struct RecommendArgs {
    /// Use signals.json and the tags cache as they are, without asking Steam
    #[arg(long)]
    pub offline: bool,

    /// Only games with at least this many reviews
    #[arg(long)]
    min_reviews: Option<u32>,

    /// Only games with at most this many reviews
    #[arg(long)]
    max_reviews: Option<u32>,

    /// Only games with at least this percentage of positive reviews
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    min_positive: Option<u8>,

    /// Minimum price in the store's currency, such as 9.99 or 9,99
    #[arg(long, value_parser = parse_price)]
    min_price: Option<u32>,

    /// Maximum price in the store's currency, such as 9.99 or 9,99
    #[arg(long, value_parser = parse_price)]
    max_price: Option<u32>,

    /// Only discounted games
    #[arg(long)]
    only_discounted: bool,

    /// Only games discounted by at least this percentage
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    min_discount: Option<u8>,

    /// Include early access games, overriding config.toml
    #[arg(long, conflicts_with = "no_early_access")]
    early_access: bool,

    /// Exclude early access games, overriding config.toml
    #[arg(long)]
    no_early_access: bool,

    /// Only games with this tag; repeat it for more tags
    #[arg(long = "tag", value_name = "TAG")]
    tags: Vec<String>,

    /// Exclude games with this tag; repeat it for more tags
    #[arg(long = "without", value_name = "TAG")]
    without_tags: Vec<String>,

    /// Order of the list
    #[arg(long, value_enum, default_value_t = Sort::Relevance)]
    sort: Sort,

    /// Maximum number of games
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    limit: Option<u32>,

    /// Exclude the games you have voted
    #[arg(long)]
    without_voted: bool,

    /// Only bets
    #[arg(long)]
    only_bets: bool,

    /// Recommend only from this family
    #[arg(long)]
    family: Option<String>,

    /// Open the Store page of every game found in the browser
    #[arg(long)]
    open: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Sort {
    Relevance,
    Discount,
    Price,
}

impl From<Sort> for SortBy {
    fn from(sort: Sort) -> Self {
        match sort {
            Sort::Relevance => SortBy::Relevance,
            Sort::Discount => SortBy::Discount,
            Sort::Price => SortBy::Price,
        }
    }
}

impl RecommendArgs {
    fn search_options(&self) -> SearchOptions {
        let without_early_access = if self.early_access {
            Some(false)
        } else if self.no_early_access {
            Some(true)
        } else {
            None
        };
        SearchOptions {
            filters: FilterOptions {
                min_reviews: self.min_reviews,
                max_reviews: self.max_reviews,
                min_positive_percent: self.min_positive,
                min_price_cents: self.min_price,
                max_price_cents: self.max_price,
                only_discounted: self.only_discounted,
                min_discount_percent: self.min_discount,
                without_early_access,
                without_voted: self.without_voted,
                tags: self.tags.clone(),
                without_tags: self.without_tags.clone(),
            },
            only_bets: self.only_bets,
            family: self.family.clone(),
            sort: self.sort.into(),
            limit: self.limit.map(|limit| limit as usize),
            ..SearchOptions::default()
        }
    }
}

pub fn run(args: &RecommendArgs, files: &DataFiles, data_dir: &Path) -> anyhow::Result<()> {
    let vectors = game_vectors(&files.catalog);
    let inputs = files.search_inputs(&vectors);

    check_families(files, &inputs, data_dir)?;

    let now = Timestamp(jiff::Timestamp::now().as_millisecond());
    let recommendation = match recommend(&inputs, &args.search_options(), now) {
        Ok(recommendation) => recommendation,
        Err(error) => bail!(search_error_text(&error)),
    };

    if recommendation.stale_prices {
        eprintln!(
            "{}",
            stale_prices_text(&recommendation.freshness, files.config.region)
        );
    }
    let no_bets = "No bets: they need at least 10 would_try votes and 10 not_for_me votes.";
    if args.only_bets && recommendation.without_vote_model {
        println!("{no_bets}");
        return Ok(());
    }
    if recommendation.without_vote_model {
        eprintln!("{no_bets}");
    }

    if recommendation.games.is_empty() {
        match &recommendation.empty_because {
            Some(blame) => println!(
                "No game passes the filters. The strictest is \"{}\", which discarded {} candidates.",
                blame.filter,
                integer(blame.discarded as u64)
            ),
            None => println!("No games to recommend."),
        }
        return Ok(());
    }

    let one_family = recommendation.family.is_some();
    let mut header = vec!["#", "Game", "Price", "Discount", "Discount ends", "Reviews"];
    let mut right_aligned = vec![0, 2, 3, 5];
    if one_family {
        header.extend(["Affinity", "Relevance"]);
        right_aligned.extend([6, 7]);
    } else {
        header.push("From");
    }
    header.extend(["Shared tags", "Similar to"]);

    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(header);
    for game in &recommendation.games {
        table.add_row(row(game, one_family, &files.catalog.currency));
    }
    for column in right_aligned {
        if let Some(column) = table.column_mut(column) {
            column.set_cell_alignment(CellAlignment::Right);
        }
    }
    println!("{table}");
    if one_family {
        println!("Affinity: 1 = average catalog game.");
    }

    if args.open {
        for game in &recommendation.games {
            if let Err(error) = open::that(&game.store_url) {
                eprintln!("Could not open {}: {error}", game.store_url);
            }
        }
    }

    Ok(())
}

fn stale_prices_text(freshness: &Freshness, region: Region) -> String {
    if freshness.region != region {
        return format!(
            "The catalog has prices for {}, the config says {region}: run update.",
            freshness.region
        );
    }
    format!(
        "Prices last updated on {}: they may be out of date.",
        date_time(freshness.prices_updated_at)
    )
}

fn row(game: &RecommendedGame, one_family: bool, currency: &Currency) -> Vec<String> {
    let discount = game.price.discount;
    let mut cells = vec![
        game.position.to_string(),
        game_cell(game),
        currency.format(game.price.final_cents()),
        discount.map_or("-".to_string(), |discount| {
            format!("-{}%", discount.percent)
        }),
        discount
            .and_then(|discount| discount.end)
            .map_or("-".to_string(), date),
        reviews_cell(game.reviews),
    ];
    if one_family {
        cells.extend([decimal(game.affinity), decimal(game.relevance)]);
    } else {
        cells.push(
            game.from_family
                .clone()
                .unwrap_or_else(|| "Signals".to_string()),
        );
    }
    cells.extend([game.top_tags.join(", "), similar_cell(game)]);
    cells
}

fn game_cell(game: &RecommendedGame) -> String {
    let mut cell = if game.name.chars().count() > NAME_WIDTH {
        let short: String = game.name.chars().take(NAME_WIDTH - 1).collect();
        format!("{short}…")
    } else {
        game.name.clone()
    };
    if game.in_wishlist {
        cell.push_str(" [wishlist]");
    }
    if game.bet {
        cell.push_str(" [bet]");
    }
    cell
}

fn reviews_cell(reviews: Reviews) -> String {
    if reviews.total == 0 {
        return "-".to_string();
    }
    let percent = (f64::from(reviews.positive) * 100.0 / f64::from(reviews.total)).round();
    format!("{percent}% ({})", integer(u64::from(reviews.total)))
}

fn similar_cell(game: &RecommendedGame) -> String {
    match (&game.similar_to, game.similar_to_multiplier) {
        (Some(name), Some(value)) => format!("{name} {}", multiplier(value)),
        (Some(name), None) => name.clone(),
        (None, _) => "-".to_string(),
    }
}

fn search_error_text(error: &SearchError) -> String {
    match error {
        SearchError::FamilyNotFound { name, families } => {
            format!(
                "no family named \"{name}\": the families are {}",
                families.join(", ")
            )
        }
        SearchError::TagName(error) => tag_name_error_text(error),
        SearchError::EmptySignalsProfile => {
            "the signals give an empty profile: no game above the hours threshold has tags"
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPDATED_AT: Timestamp = Timestamp(1_793_638_800_000);

    fn italian_freshness() -> Freshness {
        Freshness {
            prices_updated_at: UPDATED_AT,
            tags_updated_at: UPDATED_AT,
            region: Region::ITALY,
        }
    }

    #[test]
    fn a_catalog_of_another_region_says_to_run_update() {
        assert_eq!(
            stale_prices_text(&italian_freshness(), Region::new("US").unwrap()),
            "The catalog has prices for IT, the config says US: run update."
        );
    }

    #[test]
    fn old_prices_of_the_same_region_say_when_they_were_updated() {
        assert!(
            stale_prices_text(&italian_freshness(), Region::ITALY)
                .starts_with("Prices last updated on ")
        );
    }
}
