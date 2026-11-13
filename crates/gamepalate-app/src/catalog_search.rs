use std::cmp::Reverse;

use gamepalate_core::{catalog::Catalog, ids::AppId};

use crate::names::folded;

pub const MIN_SEARCH_CHARS: usize = 3;
pub const MAX_MATCHES: usize = 8;

#[derive(Debug)]
pub(crate) struct NameIndex {
    entries: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    id: AppId,
    name: Box<str>,
}

impl NameIndex {
    pub(crate) fn new(catalog: &Catalog) -> NameIndex {
        let mut games: Vec<_> = catalog.games().iter().collect();
        games.sort_by_key(|game| (Reverse(game.reviews.total), game.id));

        NameIndex {
            entries: games
                .into_iter()
                .map(|game| Entry {
                    id: game.id,
                    name: folded(game.name.trim()).into_boxed_str(),
                })
                .collect(),
        }
    }

    /// The games of the whole catalog for `text`: the game of an appid or of a store link when
    /// there is one in the catalog, else the ones with that name.
    pub(crate) fn find(&self, text: &str, catalog: &Catalog) -> Vec<AppId> {
        match appid_in(text) {
            Some(id) => catalog.game(id).map(|game| game.id).into_iter().collect(),
            None => self.search(text),
        }
    }

    fn search(&self, text: &str) -> Vec<AppId> {
        let text = folded(text.trim());
        if text.chars().count() < MIN_SEARCH_CHARS {
            return Vec::new();
        }

        let mut equal = Vec::new();
        let mut starting = Vec::new();
        let mut containing = Vec::new();
        for entry in &self.entries {
            if equal.len() == MAX_MATCHES {
                break;
            }
            if !entry.name.contains(&text) {
                continue;
            }
            let group = if *entry.name == *text {
                &mut equal
            } else if entry.name.starts_with(&text) {
                &mut starting
            } else {
                &mut containing
            };
            if group.len() < MAX_MATCHES {
                group.push(entry.id);
            }
        }

        equal
            .into_iter()
            .chain(starting)
            .chain(containing)
            .take(MAX_MATCHES)
            .collect()
    }
}

/// The appid in a text that is only digits (`10`) or has `/app/<digits>` in it, like the
/// link of a game in the store or in the community.
fn appid_in(text: &str) -> Option<AppId> {
    let text = text.trim();
    let digits = if text.bytes().all(|byte| byte.is_ascii_digit()) {
        text
    } else {
        let (_, after) = text.split_once("/app/")?;
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        &after[..end]
    };
    digits.parse().ok().filter(|&id| id != 0).map(AppId)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Instant};

    use gamepalate_core::{
        catalog::Freshness,
        currency::Currency,
        game::{Game, Price, Reviews},
        region::Region,
        timestamp::Timestamp,
    };

    use super::*;

    fn game(id: u32, name: &str, reviews: u32) -> Game {
        Game {
            id: AppId(id),
            name: name.to_string(),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: Reviews {
                total: reviews,
                positive: reviews,
            },
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

    fn search(games: Vec<Game>, text: &str) -> Vec<u32> {
        NameIndex::new(&catalog(games))
            .search(text)
            .into_iter()
            .map(|id| id.0)
            .collect()
    }

    #[test]
    fn case_and_accents_are_ignored() {
        let games = vec![game(1, "Pokémon Snap", 10), game(2, "Portal", 10)];

        assert_eq!(search(games.clone(), "POKEMON"), [1]);
        assert_eq!(search(games.clone(), "pokémon"), [1]);
        assert_eq!(search(games, "  portal  "), [2]);
    }

    #[test]
    fn under_three_characters_nothing_is_found() {
        let games = vec![game(1, "It Takes Two", 10)];

        assert!(search(games.clone(), "it").is_empty());
        assert!(search(games.clone(), " it ").is_empty());
        assert!(search(games.clone(), "").is_empty());
        assert_eq!(search(games, "it t"), [1]);
    }

    #[test]
    fn at_most_twenty_games_come_back() {
        let games = (1..=50)
            .map(|id| game(id, &format!("Puzzle {id}"), id))
            .collect();

        let found = search(games, "puzzle");

        assert_eq!(found.len(), MAX_MATCHES);
        assert_eq!(found[0], 50);
    }

    #[test]
    fn an_equal_name_comes_before_the_names_that_start_with_it() {
        let games = vec![game(1, "Hades II", 90_000), game(2, "Hades", 200)];

        assert_eq!(search(games, "hades"), [2, 1]);
    }

    #[test]
    fn a_name_that_starts_with_the_text_comes_before_one_that_contains_it() {
        let games = vec![
            game(1, "Escape the Portal Room", 90_000),
            game(2, "Portal 2", 100),
            game(3, "Portal", 50),
            game(4, "Aperture Portal Test", 80_000),
        ];

        assert_eq!(search(games, "portal"), [3, 2, 1, 4]);
    }

    #[test]
    fn games_with_the_same_kind_of_match_come_by_reviews() {
        let games = vec![
            game(1, "Portal Knights", 10),
            game(2, "Portal Reloaded", 500),
            game(3, "Portal Stories", 500),
        ];

        assert_eq!(search(games, "portal"), [2, 3, 1]);
    }

    #[test]
    fn the_equal_names_beyond_twenty_are_cut_while_a_start_stays_out() {
        let mut games: Vec<Game> = (1..=25).map(|id| game(id, "Hades", 100 + id)).collect();
        games.push(game(100, "Hades II", 1_000_000));

        let found = search(games, "hades");

        assert_eq!(found.len(), MAX_MATCHES);
        assert!(found.iter().all(|&id| id != 100));
    }

    fn find(games: Vec<Game>, text: &str) -> Vec<u32> {
        let catalog = catalog(games);
        NameIndex::new(&catalog)
            .find(text, &catalog)
            .into_iter()
            .map(|id| id.0)
            .collect()
    }

    #[test]
    fn find_does_not_leave_out_any_game() {
        let games = vec![game(1, "Portal", 10), game(2, "Portal 2", 20)];

        assert_eq!(find(games, "portal"), [1, 2]);
    }

    #[test]
    fn an_appid_gives_that_game_even_when_it_is_short() {
        let games = vec![game(10, "Counter-Strike", 10), game(1145360, "Hades", 10)];

        assert_eq!(find(games.clone(), "10"), [10]);
        assert_eq!(find(games.clone(), " 1145360 "), [1145360]);
        assert!(find(games, "999").is_empty());
    }

    #[test]
    fn a_store_link_gives_its_game() {
        let games = vec![game(1145360, "Hades", 10), game(10, "Counter-Strike", 10)];

        for link in [
            "https://store.steampowered.com/app/1145360/Hades/",
            "store.steampowered.com/app/1145360",
            "https://steamcommunity.com/app/1145360/discussions/",
            "http://store.steampowered.com/app/1145360?l=italian",
        ] {
            assert_eq!(find(games.clone(), link), [1145360], "{link}");
        }
    }

    #[test]
    fn an_appid_out_of_the_catalog_gives_nothing() {
        let games = vec![game(1145360, "Hades", 10)];

        assert!(find(games.clone(), "store.steampowered.com/app/620/Portal_2/").is_empty());
        assert!(find(games, "620").is_empty());
    }

    #[test]
    fn a_link_without_an_appid_is_a_name() {
        let games = vec![game(1, "Steam Cleaner", 10)];

        assert!(find(games.clone(), "store.steampowered.com/app/").is_empty());
        assert_eq!(find(games, "steam clean"), [1]);
    }

    #[test]
    fn a_new_catalog_gives_a_new_index() {
        let before = NameIndex::new(&catalog(vec![game(1, "Portal", 10)]));
        let after = NameIndex::new(&catalog(vec![game(2, "Portal 2", 10)]));

        assert_eq!(before.search("portal"), [AppId(1)]);
        assert_eq!(after.search("portal"), [AppId(2)]);
    }

    #[test]
    #[ignore = "measures the cost of a search, run with --release --nocapture"]
    fn measure_a_search_on_a_catalog_of_110_thousand_games() {
        let words = [
            "Dark", "Portal", "Quest", "Hades", "Tower", "Space", "Farm", "Craft", "Night",
            "Legend", "Rogue", "Puzzle", "Ocean", "Knight", "Cyber", "Zombie",
        ];
        let games: Vec<Game> = (1..=110_000u32)
            .map(|id| {
                let a = words[(id as usize * 7) % words.len()];
                let b = words[(id as usize * 13 + 5) % words.len()];
                game(
                    id,
                    &format!("{a} of the {b} {id}"),
                    id.wrapping_mul(2_654_435_761) % 100_000,
                )
            })
            .collect();

        let built = Instant::now();
        let index = NameIndex::new(&catalog(games));
        let build = built.elapsed();
        let bytes: usize = index.entries.len() * std::mem::size_of::<Entry>()
            + index
                .entries
                .iter()
                .map(|entry| entry.name.len())
                .sum::<usize>();

        for text in ["por", "portal", "dark of the", "hades", "zzz", "the 1"] {
            let started = Instant::now();
            let runs = 50;
            let mut found = 0;
            for _ in 0..runs {
                found = index.search(text).len();
            }
            println!(
                "{text:?}: {found} games, {:?} per search",
                started.elapsed() / runs
            );
        }
        println!(
            "built in {build:?}, {} entries, {} KB",
            index.entries.len(),
            bytes / 1024
        );
    }
}
