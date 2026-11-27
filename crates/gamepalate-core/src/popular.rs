use std::{collections::HashSet, fmt};

use crate::{catalog::Catalog, game::Game, ids::AppId};

pub const POPULAR_MIN_REVIEWS: u32 = 750;
pub const POPULAR_MIN_POSITIVE_PERCENT: u8 = 72;
pub const POPULAR_APPS_MAX_BYTES: usize = 1_000_000;

pub fn popular_apps(catalog: &Catalog) -> Vec<AppId> {
    let mut popular: Vec<&Game> = catalog
        .games()
        .iter()
        .filter(|game| is_popular(game))
        .collect();
    popular.sort_by(|a, b| b.reviews.total.cmp(&a.reviews.total).then(a.id.cmp(&b.id)));
    popular.into_iter().map(|game| game.id).collect()
}

fn is_popular(game: &Game) -> bool {
    let reviews = game.reviews;
    reviews.total >= POPULAR_MIN_REVIEWS
        && u64::from(reviews.positive) * 100
            >= u64::from(POPULAR_MIN_POSITIVE_PERCENT) * u64::from(reviews.total)
}

pub fn popular_apps_text(apps: &[AppId], date: &str) -> String {
    let mut text = format!(
        "# gamepalate popular apps · {date} · {POPULAR_MIN_REVIEWS} reviews · {POPULAR_MIN_POSITIVE_PERCENT}% positive · {} apps\n",
        apps.len()
    );
    for app in apps {
        text.push_str(&app.0.to_string());
        text.push('\n');
    }
    text
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopularAppsError {
    TooLarge { bytes: usize },
    NotText,
    BadLine { line: usize },
}

impl fmt::Display for PopularAppsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PopularAppsError::TooLarge { bytes } => write!(
                f,
                "the list of popular apps is {bytes} bytes, more than {POPULAR_APPS_MAX_BYTES}"
            ),
            PopularAppsError::NotText => f.write_str("the list of popular apps isn't UTF-8 text"),
            PopularAppsError::BadLine { line } => {
                write!(f, "line {line} of the list of popular apps isn't an appid")
            }
        }
    }
}

impl std::error::Error for PopularAppsError {}

pub fn parse_popular_apps(bytes: &[u8]) -> Result<Vec<AppId>, PopularAppsError> {
    if bytes.len() > POPULAR_APPS_MAX_BYTES {
        return Err(PopularAppsError::TooLarge { bytes: bytes.len() });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PopularAppsError::NotText)?;

    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let id = line
            .parse::<u32>()
            .map_err(|_| PopularAppsError::BadLine { line: index + 1 })?;
        if seen.insert(id) {
            apps.push(AppId(id));
        }
    }
    Ok(apps)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        catalog::Freshness,
        currency::Currency,
        game::{Price, Reviews},
        region::Region,
        timestamp::Timestamp,
    };

    fn game(id: u32, total: u32, positive: u32) -> Game {
        Game {
            id: AppId(id),
            name: String::new(),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: Reviews { total, positive },
            is_early_access: false,
            tags: Vec::new(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn catalog(games: Vec<Game>) -> Catalog {
        Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn ids(apps: &[AppId]) -> Vec<u32> {
        apps.iter().map(|app| app.0).collect()
    }

    #[test]
    fn only_the_games_over_both_thresholds_are_popular() {
        let catalog = catalog(vec![
            game(1, 749, 749),
            game(2, 750, 750),
            game(3, 1000, 710),
            game(4, 1000, 720),
            game(5, 0, 0),
            game(6, 750, 540),
            game(7, 750, 539),
        ]);

        assert_eq!(ids(&popular_apps(&catalog)), [4, 2, 6]);
    }

    #[test]
    fn a_game_without_reviews_is_never_popular() {
        let catalog = catalog(vec![game(1, 0, 0)]);

        assert!(popular_apps(&catalog).is_empty());
    }

    #[test]
    fn the_most_reviewed_come_first_and_ties_go_by_appid() {
        let catalog = catalog(vec![
            game(30, 800, 800),
            game(10, 5000, 5000),
            game(20, 800, 800),
            game(5, 900, 900),
        ]);

        assert_eq!(ids(&popular_apps(&catalog)), [10, 5, 20, 30]);
    }

    #[test]
    fn the_text_starts_with_the_comment_line() {
        let text = popular_apps_text(&[AppId(1172470), AppId(730)], "2026-11-27");

        assert_eq!(
            text,
            "# gamepalate popular apps · 2026-11-27 · 750 reviews · 72% positive · 2 apps\n\
             1172470\n\
             730\n"
        );
    }

    #[test]
    fn a_written_list_reads_back_the_same() {
        let apps = [AppId(1172470), AppId(730), AppId(4_000_000_000)];

        let text = popular_apps_text(&apps, "2026-11-27");

        assert_eq!(parse_popular_apps(text.as_bytes()).unwrap(), apps);
    }

    #[test]
    fn comments_and_empty_lines_are_skipped() {
        let text = "# a comment\n\n  730 \r\n#570\n\t\n440\n";

        assert_eq!(
            ids(&parse_popular_apps(text.as_bytes()).unwrap()),
            [730, 440]
        );
    }

    #[test]
    fn a_line_with_a_letter_names_its_number() {
        let text = "# comment\n730\n\n44O\n";

        assert_eq!(
            parse_popular_apps(text.as_bytes()),
            Err(PopularAppsError::BadLine { line: 4 })
        );
    }

    #[test]
    fn an_appid_too_large_for_u32_is_an_error() {
        let text = "730\n4294967296\n";

        assert_eq!(
            parse_popular_apps(text.as_bytes()),
            Err(PopularAppsError::BadLine { line: 2 })
        );
    }

    #[test]
    fn a_file_over_one_megabyte_is_refused() {
        let text = "730\n".repeat(POPULAR_APPS_MAX_BYTES / 4 + 1);

        assert_eq!(
            parse_popular_apps(text.as_bytes()),
            Err(PopularAppsError::TooLarge { bytes: text.len() })
        );
    }

    #[test]
    fn a_doubled_appid_keeps_its_first_place() {
        let text = "730\n440\n730\n570\n440\n";

        assert_eq!(
            ids(&parse_popular_apps(text.as_bytes()).unwrap()),
            [730, 440, 570]
        );
    }

    #[test]
    fn an_empty_file_has_no_apps() {
        assert_eq!(parse_popular_apps(b""), Ok(Vec::new()));
        assert_eq!(parse_popular_apps(b"# only a comment\n\n"), Ok(Vec::new()));
    }

    #[test]
    fn bytes_that_are_not_text_are_an_error() {
        assert_eq!(
            parse_popular_apps(&[0x37, 0xff, 0x0a]),
            Err(PopularAppsError::NotText)
        );
    }
}
