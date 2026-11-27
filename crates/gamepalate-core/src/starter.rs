use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

use crate::{
    catalog::{Catalog, TagFrequencies},
    game::Game,
    ids::{AppId, TagId},
};

pub const STARTER_MIN_REVIEWS: u32 = 750;
pub const STARTER_MIN_POSITIVE_PERCENT: u8 = 72;
pub const STARTER_CATALOG_MAX_BYTES: usize = 1_000_000;

const APPS: &str = "[apps]";
const TAGS: &str = "[tags]";
const GAMES: &str = "games";

/// What a release tells a first download: the popular apps to ask first and,
/// from the whole Catalog, the frequencies of the tags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StarterCatalog {
    pub apps: Vec<AppId>,
    pub tags: Option<TagFrequencies>,
}

pub fn starter_apps(catalog: &Catalog) -> Vec<AppId> {
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
    reviews.total >= STARTER_MIN_REVIEWS
        && u64::from(reviews.positive) * 100
            >= u64::from(STARTER_MIN_POSITIVE_PERCENT) * u64::from(reviews.total)
}

pub fn starter_catalog_text(apps: &[AppId], tags: &TagFrequencies, date: &str) -> String {
    let mut text = format!(
        "# gamepalate starter catalog · {date} · {STARTER_MIN_REVIEWS} reviews · {STARTER_MIN_POSITIVE_PERCENT}% positive · {} apps · {} tags\n",
        apps.len(),
        tags.tags.len()
    );
    text.push_str(APPS);
    text.push('\n');
    for app in apps {
        text.push_str(&app.0.to_string());
        text.push('\n');
    }
    text.push_str(TAGS);
    text.push('\n');
    text.push_str(&format!("{GAMES} {}\n", tags.games));
    for (tag, count) in &tags.tags {
        text.push_str(&format!("{} {count}\n", tag.0));
    }
    text
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StarterCatalogError {
    TooLarge { bytes: usize },
    NotText,
    BadLine { line: usize },
    BadSection { line: usize },
    NoGames { line: usize },
    DoubledTag { line: usize },
}

impl fmt::Display for StarterCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StarterCatalogError::TooLarge { bytes } => write!(
                f,
                "the starter catalog is {bytes} bytes, more than {STARTER_CATALOG_MAX_BYTES}"
            ),
            StarterCatalogError::NotText => f.write_str("the starter catalog isn't UTF-8 text"),
            StarterCatalogError::BadLine { line } => {
                write!(f, "line {line} of the starter catalog can't be read")
            }
            StarterCatalogError::BadSection { line } => write!(
                f,
                "line {line} of the starter catalog is out of its sections, which go [apps] then [tags]"
            ),
            StarterCatalogError::NoGames { line } => write!(
                f,
                "line {line} of the starter catalog should be the number of games, more than zero"
            ),
            StarterCatalogError::DoubledTag { line } => {
                write!(f, "line {line} of the starter catalog repeats a tag")
            }
        }
    }
}

impl std::error::Error for StarterCatalogError {}

enum Section {
    None,
    Apps,
    Tags,
}

pub fn parse_starter_catalog(bytes: &[u8]) -> Result<StarterCatalog, StarterCatalogError> {
    if bytes.len() > STARTER_CATALOG_MAX_BYTES {
        return Err(StarterCatalogError::TooLarge { bytes: bytes.len() });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| StarterCatalogError::NotText)?;

    let mut section = Section::None;
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    let mut games = None;
    let mut tags: BTreeMap<TagId, u32> = BTreeMap::new();
    let mut tags_line = 0;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad_line = || StarterCatalogError::BadLine { line: number };
        match (&section, line) {
            (Section::None, APPS) => section = Section::Apps,
            (Section::Apps, TAGS) => {
                section = Section::Tags;
                tags_line = number;
            }
            (Section::None, _) | (_, APPS | TAGS) => {
                return Err(StarterCatalogError::BadSection { line: number });
            }
            _ if line.starts_with('[') => {
                return Err(StarterCatalogError::BadSection { line: number });
            }
            (Section::Apps, _) => {
                let id = line.parse::<u32>().map_err(|_| bad_line())?;
                if seen.insert(id) {
                    apps.push(AppId(id));
                }
            }
            (Section::Tags, _) => {
                let (name, count) = pair(line).ok_or_else(bad_line)?;
                match games {
                    None if name == GAMES => match count.parse::<u32>().map_err(|_| bad_line())? {
                        0 => return Err(StarterCatalogError::NoGames { line: number }),
                        count => games = Some(count),
                    },
                    None => return Err(StarterCatalogError::NoGames { line: number }),
                    Some(_) => {
                        let tag = name.parse::<u32>().map_err(|_| bad_line())?;
                        let count = count.parse::<u32>().map_err(|_| bad_line())?;
                        if tags.insert(TagId(tag), count).is_some() {
                            return Err(StarterCatalogError::DoubledTag { line: number });
                        }
                    }
                }
            }
        }
    }

    let tags = match section {
        Section::Tags => {
            let games = games.ok_or(StarterCatalogError::NoGames { line: tags_line })?;
            Some(TagFrequencies { games, tags })
        }
        _ => None,
    };
    Ok(StarterCatalog { apps, tags })
}

fn pair(line: &str) -> Option<(&str, &str)> {
    let mut words = line.split_whitespace();
    let pair = (words.next()?, words.next()?);
    words.next().is_none().then_some(pair)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::{Freshness, compute_idf, idf_from, tag_frequencies},
        currency::Currency,
        game::{GameTag, Price, Reviews},
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

    fn frequencies(games: u32, tags: &[(u32, u32)]) -> TagFrequencies {
        TagFrequencies {
            games,
            tags: tags.iter().map(|&(id, count)| (TagId(id), count)).collect(),
        }
    }

    fn parse(text: &str) -> Result<StarterCatalog, StarterCatalogError> {
        parse_starter_catalog(text.as_bytes())
    }

    fn apps_of(text: &str) -> Vec<u32> {
        ids(&parse(text).unwrap().apps)
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

        assert_eq!(ids(&starter_apps(&catalog)), [4, 2, 6]);
    }

    #[test]
    fn a_game_without_reviews_is_never_popular() {
        let catalog = catalog(vec![game(1, 0, 0)]);

        assert!(starter_apps(&catalog).is_empty());
    }

    #[test]
    fn the_most_reviewed_come_first_and_ties_go_by_appid() {
        let catalog = catalog(vec![
            game(30, 800, 800),
            game(10, 5000, 5000),
            game(20, 800, 800),
            game(5, 900, 900),
        ]);

        assert_eq!(ids(&starter_apps(&catalog)), [10, 5, 20, 30]);
    }

    #[test]
    fn the_text_has_the_comment_line_then_the_two_sections() {
        let text = starter_catalog_text(
            &[AppId(1172470), AppId(730)],
            &frequencies(121034, &[(492, 30877), (19, 48211)]),
            "2026-11-27",
        );

        assert_eq!(
            text,
            "# gamepalate starter catalog · 2026-11-27 · 750 reviews · 72% positive · 2 apps · 2 tags\n\
             [apps]\n\
             1172470\n\
             730\n\
             [tags]\n\
             games 121034\n\
             19 48211\n\
             492 30877\n"
        );
    }

    #[test]
    fn a_written_catalog_reads_back_the_same() {
        let apps = [AppId(1172470), AppId(730), AppId(4_000_000_000)];
        let tags = frequencies(121034, &[(19, 48211), (492, 30877), (4_000_000_000, 1)]);

        let text = starter_catalog_text(&apps, &tags, "2026-11-27");

        assert_eq!(
            parse(&text).unwrap(),
            StarterCatalog {
                apps: apps.to_vec(),
                tags: Some(tags),
            }
        );
    }

    #[test]
    fn the_idf_of_the_read_frequencies_is_the_idf_of_the_catalog() {
        let mut games = vec![game(1, 0, 0), game(2, 0, 0), game(3, 0, 0)];
        games[0].tags = vec![GameTag {
            id: TagId(19),
            weight: 1.0,
        }];
        games[1].tags = vec![GameTag {
            id: TagId(492),
            weight: 0.5,
        }];

        let text = starter_catalog_text(&[], &tag_frequencies(&games), "2026-11-27");
        let read = parse(&text).unwrap().tags.unwrap();

        assert_eq!(idf_from(&read), compute_idf(&games));
    }

    #[test]
    fn comments_and_empty_lines_are_skipped() {
        let text =
            "# a comment\n\n[apps]\n  730 \r\n#570\n\t\n440\n[tags]\n# more\n games  12 \n\n19 3\n";

        assert_eq!(
            parse(text).unwrap(),
            StarterCatalog {
                apps: vec![AppId(730), AppId(440)],
                tags: Some(frequencies(12, &[(19, 3)])),
            }
        );
    }

    #[test]
    fn a_catalog_without_tags_is_the_list_alone() {
        assert_eq!(
            parse("# comment\n[apps]\n730\n440\n").unwrap(),
            StarterCatalog {
                apps: vec![AppId(730), AppId(440)],
                tags: None,
            }
        );
    }

    #[test]
    fn a_line_with_a_letter_names_its_number() {
        assert_eq!(
            parse("# comment\n[apps]\n730\n\n44O\n"),
            Err(StarterCatalogError::BadLine { line: 5 })
        );
    }

    #[test]
    fn an_appid_too_large_for_u32_is_an_error() {
        assert_eq!(
            parse("[apps]\n730\n4294967296\n"),
            Err(StarterCatalogError::BadLine { line: 3 })
        );
    }

    #[test]
    fn a_broken_tag_line_names_its_number() {
        for (text, line) in [
            ("[apps]\n730\n[tags]\ngames 10\n19\n", 5),
            ("[apps]\n730\n[tags]\ngames 10\n19 3 4\n", 5),
            ("[apps]\n730\n[tags]\ngames 10\nrpg 3\n", 5),
            ("[apps]\n730\n[tags]\ngames 10\n19 -3\n", 5),
            ("[apps]\n730\n[tags]\ngames ten\n", 4),
        ] {
            assert_eq!(
                parse(text),
                Err(StarterCatalogError::BadLine { line }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_sections_go_apps_then_tags() {
        for (text, line) in [
            ("730\n[apps]\n440\n", 1),
            ("[tags]\ngames 10\n[apps]\n730\n", 1),
            ("[apps]\n730\n[tags]\ngames 10\n[apps]\n440\n", 5),
            ("[apps]\n730\n[apps]\n440\n", 3),
            ("[apps]\n730\n[tags]\ngames 10\n[tags]\n", 5),
            ("[apps]\n730\n[genres]\n", 3),
        ] {
            assert_eq!(
                parse(text),
                Err(StarterCatalogError::BadSection { line }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_tags_start_with_a_number_of_games_over_zero() {
        for (text, line) in [
            ("[apps]\n730\n[tags]\n19 3\ngames 10\n", 4),
            ("[apps]\n730\n[tags]\ngames 0\n19 3\n", 4),
            ("[apps]\n730\n[tags]\n", 3),
            ("[apps]\n730\n[tags]\n# no games\n\n", 3),
        ] {
            assert_eq!(
                parse(text),
                Err(StarterCatalogError::NoGames { line }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_doubled_tag_is_an_error() {
        assert_eq!(
            parse("[apps]\n730\n[tags]\ngames 10\n19 3\n492 2\n19 4\n"),
            Err(StarterCatalogError::DoubledTag { line: 7 })
        );
    }

    #[test]
    fn a_file_over_one_megabyte_is_refused() {
        let text = format!("[apps]\n{}", "730\n".repeat(STARTER_CATALOG_MAX_BYTES / 4));

        assert_eq!(
            parse(&text),
            Err(StarterCatalogError::TooLarge { bytes: text.len() })
        );
    }

    #[test]
    fn a_doubled_appid_keeps_its_first_place() {
        assert_eq!(
            apps_of("[apps]\n730\n440\n730\n570\n440\n"),
            [730, 440, 570]
        );
    }

    #[test]
    fn an_empty_file_has_no_apps() {
        assert_eq!(parse(""), Ok(StarterCatalog::default()));
        assert_eq!(parse("# only a comment\n\n"), Ok(StarterCatalog::default()));
        assert_eq!(parse("[apps]\n"), Ok(StarterCatalog::default()));
    }

    #[test]
    fn bytes_that_are_not_text_are_an_error() {
        assert_eq!(
            parse_starter_catalog(&[0x37, 0xff, 0x0a]),
            Err(StarterCatalogError::NotText)
        );
    }
}
