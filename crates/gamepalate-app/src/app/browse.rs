use std::sync::Arc;

use gamepalate_core::{
    browse::{BrowseOptions, BrowseOrder, Listed, Mark, marks},
    filters::{FilterContext, chosen_filters, first_failed},
    game::{Price, Reviews},
    ids::AppId,
    search::SearchInputs,
    timestamp::Timestamp,
};

use super::{Gamepalate, RecommendError};
use crate::{
    catalog_search::NameIndex,
    library::{GameStanding, Standings},
    log::failed,
};

/// A game of the catalog in the list of Browse. It carries numbers and the name, not the text
/// to show: the window makes that for the rows it draws.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowseRow {
    pub id: AppId,
    pub name: Arc<str>,
    pub standing: Option<GameStanding>,
    pub starred: bool,
    /// Without a discount that is already over.
    pub price: Price,
    pub reviews: Reviews,
}

/// The games of the catalog that pass the filters, the three-state choices and the name, in the
/// order of `options`. Everything that does not depend on the game is worked out once.
fn browse(
    inputs: &SearchInputs,
    names: &NameIndex,
    options: &BrowseOptions,
    text: &str,
    now: Timestamp,
) -> Result<Vec<BrowseRow>, RecommendError> {
    let catalog = inputs.catalog;
    let filters =
        chosen_filters(&options.filters, catalog).map_err(|error| RecommendError(error.into()))?;
    let context = FilterContext::new(
        inputs.signals,
        inputs.families,
        inputs.votes,
        inputs.corrections,
        &inputs.config.signals,
    );
    let marks = marks(&context, now);
    let unmarked_passes = options.allows_unmarked();
    let query = NameIndex::query(text);
    let ranks = (options.order == BrowseOrder::Name).then(|| names.name_ranks());
    let shown = names.shown_names(catalog);

    let mut listed: Vec<(Listed, Option<Mark>)> = Vec::new();
    for (position, game) in catalog.games().iter().enumerate() {
        if first_failed(game, &filters, &context, now).is_some() {
            continue;
        }
        let mark = marks.get(&game.id).copied();
        if !mark.map_or(unmarked_passes, |mark| options.allows(mark)) {
            continue;
        }
        if query
            .as_ref()
            .is_some_and(|query| !names.matches(position, query))
        {
            continue;
        }
        let rank = ranks.map_or(0, |ranks| ranks[position]);
        listed.push((
            Listed::new(position, game, rank, options.order, options.descending, now),
            mark,
        ));
    }
    listed.sort_unstable_by(|(a, _), (b, _)| a.compare(b));

    // A game without a mark is not in the Library and has no vote that counts, so it has no
    // standing to look for
    let standings = Standings::new(inputs);
    Ok(listed
        .into_iter()
        .map(|(listed, mark)| {
            let game = &catalog.games()[listed.position as usize];
            BrowseRow {
                id: game.id,
                name: Arc::clone(&shown[listed.position as usize]),
                standing: mark.and_then(|_| standings.of(game.id, now)),
                starred: mark.is_some_and(|mark| mark.starred),
                price: game.price.as_of(now),
                reviews: game.reviews,
            }
        })
        .collect())
}

impl Gamepalate {
    /// The games of the whole catalog that pass `options` and have `name` in their name, in the
    /// order of `options`. An empty `name` is no limit; there is no minimum of characters.
    pub fn browse(
        &self,
        options: &BrowseOptions,
        name: &str,
    ) -> Result<Vec<BrowseRow>, RecommendError> {
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());
        self.with_inputs(|inputs| browse(inputs, &self.names, options, name, now))
            .inspect_err(failed("could not browse the catalog"))
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Instant};

    use gamepalate_core::{
        browse::Choice,
        catalog::{Catalog, Freshness, TagName},
        config::Config,
        corrections::Corrections,
        currency::Currency,
        families::Families,
        filters::FilterOptions,
        game::{Discount, Game, GameTag},
        ids::TagId,
        off_catalog::OffCatalogGames,
        region::Region,
        signals::{LibraryGame, Signals},
        votes::{Vote, VoteKind, Votes},
    };

    use super::*;

    const NOW: Timestamp = Timestamp(1_800_000_000_000);
    const DAY_MS: i64 = 86_400_000;

    fn game(id: u32, name: &str, original: u32, sale: Option<(u8, Option<i64>)>) -> Game {
        Game {
            id: AppId(id),
            name: name.to_string(),
            price: Price {
                original_cents: original,
                discount: sale.map(|(percent, end)| Discount {
                    percent,
                    final_cents: original * u32::from(100 - percent) / 100,
                    end: end.map(|end| Timestamp(NOW.0 + end)),
                }),
            },
            reviews: Reviews {
                total: 100,
                positive: 90,
            },
            is_early_access: false,
            tags: Vec::new(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn with(mut game: Game, total: u32, positive: u32, tags: &[u32]) -> Game {
        game.reviews = Reviews { total, positive };
        game.tags = tags
            .iter()
            .map(|&id| GameTag {
                id: TagId(id),
                weight: 1.0,
            })
            .collect();
        game
    }

    fn catalog(games: Vec<Game>) -> Catalog {
        let tag = |name: &str| TagName {
            english: name.to_string(),
            italian: name.to_string(),
        };
        Catalog::new(
            games,
            BTreeMap::from([(TagId(1), tag("Roguelike")), (TagId(2), tag("Horror"))]),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: NOW,
                tags_updated_at: NOW,
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    struct Fixture {
        catalog: Catalog,
        signals: Signals,
        votes: Votes,
        corrections: Corrections,
        families: Families,
        off_catalog: OffCatalogGames,
        config: Config,
    }

    impl Fixture {
        fn new(
            catalog: Catalog,
            signals: Signals,
            votes: Votes,
            corrections: Corrections,
        ) -> Fixture {
            Fixture {
                catalog,
                signals,
                votes,
                corrections,
                families: Families::default(),
                off_catalog: OffCatalogGames::new(),
                config: Config::default(),
            }
        }

        fn inputs(&self) -> SearchInputs<'_> {
            SearchInputs {
                catalog: &self.catalog,
                game_vectors: &[],
                signals: &self.signals,
                votes: &self.votes,
                corrections: &self.corrections,
                families: &self.families,
                off_catalog: &self.off_catalog,
                config: &self.config,
            }
        }

        fn browse(&self, options: &BrowseOptions, text: &str) -> Vec<BrowseRow> {
            let names = NameIndex::new(&self.catalog);
            browse(&self.inputs(), &names, options, text, NOW).unwrap()
        }

        fn ids(&self, options: &BrowseOptions, text: &str) -> Vec<u32> {
            self.browse(options, text)
                .iter()
                .map(|row| row.id.0)
                .collect()
        }
    }

    fn signals(library: &[(u32, f64)], wishlist: &[u32]) -> Signals {
        Signals {
            read_at: NOW,
            library: library
                .iter()
                .map(|&(id, total_hours)| LibraryGame {
                    id: AppId(id),
                    total_hours,
                    last_played: None,
                })
                .collect(),
            borrowed: Vec::new(),
            recent: Vec::new(),
            wishlist: wishlist.iter().map(|&id| AppId(id)).collect(),
            followed: Vec::new(),
        }
    }

    fn vote(kind: VoteKind, at: i64, starred: bool) -> Vote {
        Vote {
            at: Some(Timestamp(NOW.0 + at)),
            starred,
            ..Vote::new(kind, "2026-11-17")
        }
    }

    /// Six games: 1 is in the Library, 2 in the wishlist, 3 has a star, 4 a Not for me, 5 is
    /// played elsewhere and 6 has a Later. Four are on sale.
    fn fixture() -> Fixture {
        let games = vec![
            with(
                game(1, "Alpha", 1000, Some((50, Some(2 * DAY_MS)))),
                1000,
                900,
                &[],
            ),
            with(game(2, "Ábzu", 2000, None), 500, 450, &[]),
            with(game(3, "Abc", 1500, Some((20, None))), 100, 50, &[1]),
            Game {
                is_early_access: true,
                ..with(
                    game(4, "Delta", 3000, Some((75, Some(DAY_MS)))),
                    20,
                    10,
                    &[2],
                )
            },
            with(game(5, "Echo", 500, None), 5, 5, &[]),
            with(
                game(6, "Foxtrot", 800, Some((10, Some(5 * DAY_MS)))),
                300,
                270,
                &[1, 2],
            ),
        ];
        let mut votes = Votes::new();
        votes.insert(AppId(3), vote(VoteKind::WouldTry, -DAY_MS, true));
        votes.insert(AppId(4), vote(VoteKind::NotForMe, -DAY_MS, false));
        votes.insert(AppId(6), vote(VoteKind::Later, -DAY_MS, false));
        Fixture::new(
            catalog(games),
            signals(&[(1, 10.0)], &[2]),
            votes,
            Corrections {
                played_elsewhere: [AppId(5)].into(),
                ..Corrections::default()
            },
        )
    }

    fn everything() -> BrowseOptions {
        BrowseOptions {
            filters: FilterOptions::default(),
            library: Choice::Any,
            not_for_me: Choice::Any,
            ..BrowseOptions::default()
        }
    }

    #[test]
    fn the_defaults_list_the_games_on_sale_without_the_library_and_the_not_for_me() {
        // On sale: 1, 3, 4, 6. Out: 1 (Library), 4 (Not for me). By reviews: 6 then 3
        assert_eq!(fixture().ids(&BrowseOptions::default(), ""), [6, 3]);
    }

    #[test]
    fn without_the_sale_the_whole_catalog_comes_by_reviews() {
        let options = BrowseOptions {
            filters: FilterOptions::default(),
            ..BrowseOptions::default()
        };

        assert_eq!(fixture().ids(&options, ""), [2, 6, 3]);
        assert_eq!(fixture().ids(&everything(), ""), [1, 2, 6, 3, 4, 5]);
    }

    #[test]
    fn the_three_states_pick_the_games_of_each_kind() {
        let fixture = fixture();

        let only = |change: fn(&mut BrowseOptions)| {
            let mut options = everything();
            change(&mut options);
            fixture.ids(&options, "")
        };
        assert_eq!(only(|o| o.library = Choice::Only), [1, 5]);
        assert_eq!(only(|o| o.wished = Choice::Only), [2]);
        assert_eq!(only(|o| o.would_try = Choice::Only), [3]);
        assert_eq!(only(|o| o.starred = Choice::Only), [3]);
        assert_eq!(only(|o| o.not_for_me = Choice::Only), [4]);
        assert_eq!(only(|o| o.later = Choice::Only), [6]);
        assert_eq!(only(|o| o.starred = Choice::Without), [1, 2, 6, 4, 5]);
        assert_eq!(
            only(|o| {
                o.would_try = Choice::Only;
                o.not_for_me = Choice::Only;
            }),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn a_later_that_is_over_is_not_a_later() {
        let mut fixture = fixture();
        fixture
            .votes
            .insert(AppId(6), vote(VoteKind::Later, -8 * DAY_MS, false));
        let options = BrowseOptions {
            later: Choice::Only,
            ..everything()
        };

        assert!(fixture.ids(&options, "").is_empty());
        assert_eq!(fixture.browse(&everything(), "")[2].standing, None);
    }

    #[test]
    fn each_game_says_where_it_stands() {
        let rows = fixture().browse(&everything(), "");
        let standing = |id: u32| {
            rows.iter()
                .find(|row| row.id.0 == id)
                .map(|row| (row.standing, row.starred))
                .unwrap()
        };

        assert!(matches!(
            standing(1),
            (Some(GameStanding::InLibrary { hours }), false) if hours == 10.0
        ));
        assert_eq!(standing(2), (None, false));
        assert_eq!(standing(3), (Some(GameStanding::WouldTry), true));
        assert_eq!(standing(4), (Some(GameStanding::NotForMe), false));
        assert_eq!(standing(5), (Some(GameStanding::PlayedElsewhere), false));
        assert_eq!(
            standing(6),
            (Some(GameStanding::Later { days_left: 6 }), false)
        );
    }

    #[test]
    fn a_row_has_the_price_of_now() {
        let mut fixture = fixture();
        fixture.catalog = catalog(vec![game(1, "Alpha", 1000, Some((50, Some(-1))))]);
        fixture.signals = signals(&[], &[]);
        let row = &fixture.browse(&everything(), "")[0];

        assert_eq!(row.price.discount, None);
        assert_eq!(row.price.final_cents(), 1000);
        // And so a sale that is over is no sale
        assert!(fixture.ids(&BrowseOptions::default(), "").is_empty());
    }

    #[test]
    fn the_numbers_and_the_tags_limit_the_list() {
        let fixture = fixture();
        let with_filters = |change: fn(&mut FilterOptions)| {
            let mut options = everything();
            change(&mut options.filters);
            fixture.ids(&options, "")
        };

        assert_eq!(with_filters(|f| f.min_reviews = Some(100)), [1, 2, 6, 3]);
        assert_eq!(with_filters(|f| f.max_reviews = Some(100)), [3, 4, 5]);
        assert_eq!(
            with_filters(|f| f.min_positive_percent = Some(90)),
            [1, 2, 6, 5]
        );
        assert_eq!(
            with_filters(|f| f.without_early_access = Some(true)),
            [1, 2, 6, 3, 5]
        );
        assert_eq!(
            with_filters(|f| f.max_price_cents = Some(1000)),
            [1, 6, 4, 5]
        );
        assert_eq!(with_filters(|f| f.min_discount_percent = Some(50)), [1, 4]);
        assert_eq!(
            with_filters(|f| f.tags = vec!["roguelike".to_string()]),
            [6, 3]
        );
        assert_eq!(
            with_filters(|f| f.without_tags = vec!["HORROR".to_string()]),
            [1, 2, 3, 5]
        );
    }

    #[test]
    fn an_unknown_tag_is_an_error() {
        let fixture = fixture();
        let names = NameIndex::new(&fixture.catalog);
        let mut options = everything();
        options.filters.tags = vec!["no such tag".to_string()];

        let error = browse(&fixture.inputs(), &names, &options, "", NOW).unwrap_err();

        assert_eq!(error.to_string(), "no tag named \"no such tag\"");
    }

    #[test]
    fn the_name_keeps_the_games_that_have_it_in_any_case_and_accent() {
        let fixture = fixture();

        assert_eq!(fixture.ids(&everything(), "ABZU"), [2]);
        assert_eq!(fixture.ids(&everything(), "  ábzu "), [2]);
        // No minimum of characters: one letter is a name
        assert_eq!(fixture.ids(&everything(), "a"), [1, 2, 3, 4]);
        assert_eq!(fixture.ids(&everything(), "x"), [6]);
        assert!(fixture.ids(&everything(), "zzz").is_empty());
        // The name goes with the other limits
        assert_eq!(fixture.ids(&BrowseOptions::default(), "a"), [3]);
    }

    #[test]
    fn the_name_order_puts_abzu_between_abc_and_the_other_abz() {
        let mut fixture = fixture();
        fixture.catalog = catalog(vec![
            game(10, "Abzz", 100, None),
            game(20, "Ábzu", 100, None),
            game(30, "Abc", 100, None),
            game(40, "abc", 100, None),
        ]);
        fixture.signals = signals(&[], &[]);
        fixture.votes = Votes::new();
        fixture.corrections = Corrections::default();
        let ordered = |descending| BrowseOptions {
            order: BrowseOrder::Name,
            descending,
            ..everything()
        };

        // The same name goes by the appid
        assert_eq!(fixture.ids(&ordered(false), ""), [30, 40, 20, 10]);
        assert_eq!(fixture.ids(&ordered(true), ""), [10, 20, 40, 30]);
    }

    #[test]
    fn each_order_puts_the_list_in_order() {
        let fixture = fixture();
        let ordered = |order, descending| {
            fixture.ids(
                &BrowseOptions {
                    order,
                    descending,
                    ..everything()
                },
                "",
            )
        };

        assert_eq!(ordered(BrowseOrder::Reviews, true), [1, 2, 6, 3, 4, 5]);
        assert_eq!(ordered(BrowseOrder::Discount, true), [4, 1, 3, 6, 2, 5]);
        assert_eq!(ordered(BrowseOrder::Price, false), [1, 5, 6, 4, 3, 2]);
        assert_eq!(ordered(BrowseOrder::Rating, true), [5, 1, 2, 6, 3, 4]);
        // Ends: 4 in a day, 1 in two, 6 in five; 3 has no end and the others no sale
        assert_eq!(ordered(BrowseOrder::Ends, false), [4, 1, 6, 2, 3, 5]);
        assert_eq!(ordered(BrowseOrder::Ends, true), [6, 1, 4, 2, 3, 5]);
    }

    fn big_catalog(count: u32) -> Catalog {
        let words = [
            "Dark", "Portal", "Quest", "Hades", "Tower", "Space", "Farm", "Craft", "Night",
            "Legend", "Rogue", "Puzzle", "Ocean", "Knight", "Cyber", "Zombie",
        ];
        let games = (1..=count)
            .map(|id| {
                let a = words[(id as usize * 7) % words.len()];
                let b = words[(id as usize * 13 + 5) % words.len()];
                let total = id.wrapping_mul(2_654_435_761) % 5_000;
                let sale = (id % 16 == 0)
                    .then_some((10 + (id % 8) as u8 * 10, Some(i64::from(id % 20) * DAY_MS)));
                Game {
                    reviews: Reviews {
                        total,
                        positive: total * (50 + id % 50) / 100,
                    },
                    tags: vec![GameTag {
                        id: TagId(1 + id % 2),
                        weight: 1.0,
                    }],
                    ..game(id, &format!("{a} of the {b} {id}"), 500 + id % 4_000, sale)
                }
            })
            .collect();
        catalog(games)
    }

    #[test]
    fn measure_a_list_of_the_whole_catalog() {
        let catalog = big_catalog(120_000);
        let votes: Votes = (1..=400u32)
            .map(|id| {
                let kind =
                    [VoteKind::WouldTry, VoteKind::NotForMe, VoteKind::Later][id as usize % 3];
                let starred = id % 7 == 0 && kind == VoteKind::WouldTry;
                (
                    AppId(id * 290),
                    vote(kind, -i64::from(id) * DAY_MS / 40, starred),
                )
            })
            .collect();
        let library: Vec<(u32, f64)> = (1..=300).map(|id| (id * 397, f64::from(id % 40))).collect();
        let fixture = Fixture::new(
            catalog,
            signals(&library, &[5, 55, 555]),
            votes,
            Corrections::default(),
        );
        let names = NameIndex::new(&fixture.catalog);
        let inputs = fixture.inputs();

        let orders = [
            BrowseOrder::Reviews,
            BrowseOrder::Name,
            BrowseOrder::Discount,
            BrowseOrder::Price,
            BrowseOrder::Rating,
            BrowseOrder::Ends,
        ];
        // The first list by name makes the ranks of the names, once
        let started = Instant::now();
        let by_name = BrowseOptions {
            order: BrowseOrder::Name,
            ..BrowseOptions::default()
        };
        browse(&inputs, &names, &by_name, "", NOW).unwrap();
        println!("the first list by name: {:?}", started.elapsed());

        let mut slowest = std::time::Duration::ZERO;
        for (label, on_sale, text) in [
            ("on sale", true, ""),
            ("whole catalog", false, ""),
            ("whole catalog, name \"o\"", false, "o"),
            ("whole catalog, name \"dark of\"", false, "dark of"),
        ] {
            for order in orders {
                let options = BrowseOptions {
                    filters: FilterOptions {
                        only_discounted: on_sale,
                        ..FilterOptions::default()
                    },
                    order,
                    descending: order.descending_first(),
                    ..BrowseOptions::default()
                };
                let runs = 5;
                let started = Instant::now();
                let mut rows = 0;
                for _ in 0..runs {
                    rows = browse(&inputs, &names, &options, text, NOW).unwrap().len();
                }
                let each = started.elapsed() / runs;
                slowest = slowest.max(each);
                println!("{label}, {order:?}: {rows} games, {each:?}");
            }
        }

        let limit = if cfg!(debug_assertions) { 500 } else { 16 };
        assert!(
            slowest.as_millis() < limit,
            "the slowest list took {slowest:?}, the limit is {limit} ms"
        );
    }
}
