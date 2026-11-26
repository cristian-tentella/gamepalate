use std::{cmp::Ordering, collections::HashMap};

use crate::{
    filters::{FilterContext, FilterOptions},
    game::{Game, Reviews},
    ids::AppId,
    timestamp::Timestamp,
    votes::{VoteKind, standing},
};

/// A filter on a kind of game that is not a number: no limit, only those games, or none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Choice {
    #[default]
    Any,
    Only,
    #[cfg_attr(feature = "serde", serde(rename = "hide"))]
    Without,
}

impl Choice {
    /// Whether a game that is (`holds`) or is not of the kind passes.
    pub fn allows(self, holds: bool) -> bool {
        match self {
            Choice::Any => true,
            Choice::Only => holds,
            Choice::Without => !holds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum BrowseOrder {
    #[default]
    Reviews,
    Name,
    Discount,
    Price,
    Rating,
    Ends,
}

impl BrowseOrder {
    /// The direction a first click on the column gives: the most first for reviews, discount and
    /// rating, the least first for the others.
    pub fn descending_first(self) -> bool {
        matches!(
            self,
            BrowseOrder::Reviews | BrowseOrder::Discount | BrowseOrder::Rating
        )
    }
}

/// What to list of the catalog, and in which order. `filters.without_voted` does not count here:
/// the `Choice`s say what to do with each vote.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct BrowseOptions {
    pub filters: FilterOptions,
    pub order: BrowseOrder,
    pub descending: bool,
    /// The games you have: owned, borrowed or played elsewhere.
    pub library: Choice,
    /// The wishlist and the followed games.
    pub wished: Choice,
    pub would_try: Choice,
    pub saved: Choice,
    pub not_for_me: Choice,
    pub later: Choice,
}

impl Default for BrowseOptions {
    fn default() -> BrowseOptions {
        BrowseOptions {
            filters: FilterOptions {
                only_discounted: true,
                ..FilterOptions::default()
            },
            order: BrowseOrder::Reviews,
            descending: BrowseOrder::Reviews.descending_first(),
            library: Choice::Without,
            wished: Choice::Any,
            would_try: Choice::Any,
            saved: Choice::Any,
            not_for_me: Choice::Without,
            later: Choice::Any,
        }
    }
}

impl BrowseOptions {
    /// Whether a game with these marks passes the `Choice`s.
    pub fn allows(&self, mark: Mark) -> bool {
        self.library.allows(mark.owned)
            && self.wished.allows(mark.wished)
            && self
                .would_try
                .allows(mark.vote.is_some_and(VoteKind::is_yes))
            && self.saved.allows(mark.saved)
            && self
                .not_for_me
                .allows(mark.vote.is_some_and(VoteKind::is_no))
            && self.later.allows(mark.vote.is_some_and(VoteKind::lapses))
    }

    /// Whether a game with no marks at all can pass: no `Choice` asks for one.
    pub fn allows_unmarked(&self) -> bool {
        self.allows(Mark::default())
    }
}

/// What the Library, the wishlist and the votes say about a game. A game with none has the
/// default, and is not in `marks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mark {
    pub owned: bool,
    pub wished: bool,
    /// The vote that still counts: a Later that is over is none.
    pub vote: Option<VoteKind>,
    pub saved: bool,
}

/// The marks of the few games that have any, so a game of the catalog costs one lookup.
pub fn marks(context: &FilterContext, now: Timestamp) -> HashMap<AppId, Mark> {
    let mut marks: HashMap<AppId, Mark> = HashMap::new();
    for &id in &context.owned {
        marks.entry(id).or_default().owned = true;
    }
    for &id in &context.wished_or_followed {
        marks.entry(id).or_default().wished = true;
    }
    for &id in context.votes.keys() {
        if let Some(vote) = standing(context.votes, id, now) {
            let mark = marks.entry(id).or_default();
            mark.vote = Some(vote.kind);
            mark.saved = vote.saved;
        }
    }
    marks
}

/// A game of the catalog put in order: where it is (`position` in `Catalog::games`) and the key
/// the order gives it, so putting many games in order never goes back to the games.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    pub position: u32,
    id: AppId,
    reviews: u32,
    key: u128,
}

impl Listed {
    /// `name_rank` is where the name of the game falls among all the names: only
    /// `BrowseOrder::Name` looks at it. `descending` turns the main criterion; a game without
    /// the end of a discount still goes after the others in `Ends`, in both directions.
    pub fn new(
        position: usize,
        game: &Game,
        name_rank: u32,
        order: BrowseOrder,
        descending: bool,
        now: Timestamp,
    ) -> Listed {
        let price = game.price.as_of(now);
        let key = match order {
            BrowseOrder::Reviews => Some(u128::from(game.reviews.total)),
            BrowseOrder::Name => Some(u128::from(name_rank)),
            BrowseOrder::Discount => Some(u128::from(
                price.discount.map_or(0, |discount| discount.percent),
            )),
            BrowseOrder::Price => Some(u128::from(price.final_cents())),
            BrowseOrder::Rating => Some(share_key(game.reviews)),
            // Shifted by one, so that turning it never gives the key of a game without an end
            BrowseOrder::Ends => price
                .discount
                .and_then(|discount| discount.end)
                .map(|end| (i128::from(end.0) - i128::from(i64::MIN)) as u128 + 1),
        };
        Listed {
            position: position as u32,
            id: game.id,
            reviews: game.reviews.total,
            key: match key {
                Some(key) if descending => !key,
                Some(key) => key,
                None => u128::MAX,
            },
        }
    }

    /// The order of the list: by the key, then more reviews, then the lower appid, so it is
    /// always the same.
    pub fn compare(&self, other: &Listed) -> Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| other.reviews.cmp(&self.reviews))
            .then_with(|| self.id.cmp(&other.id))
    }
}

/// The share of positive reviews as a number that is equal for equal shares and different for
/// different ones (two shares of at most 2^32 reviews are more than 2^-64 apart). A game without
/// reviews is at 0.
fn share_key(reviews: Reviews) -> u128 {
    match reviews.total {
        0 => 0,
        total => (u128::from(reviews.positive) << 64) / u128::from(total),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::SignalsConfig,
        corrections::Corrections,
        families::Families,
        game::{Discount, Price, Reviews},
        signals::{LibraryGame, RecentGame, Signals},
        votes::{Vote, Votes},
    };

    const NOW: Timestamp = Timestamp(1_000_000_000_000);
    const DAY_MS: i64 = 86_400_000;

    fn game(id: u32, original: u32, discount: Option<(u8, u32, Option<i64>)>) -> Game {
        Game {
            id: AppId(id),
            name: format!("Game {id}"),
            price: Price {
                original_cents: original,
                discount: discount.map(|(percent, final_cents, end)| Discount {
                    percent,
                    final_cents,
                    end: end.map(|end| Timestamp(NOW.0 + end)),
                }),
            },
            reviews: Reviews {
                total: 100,
                positive: 80,
            },
            is_early_access: false,
            tags: Vec::new(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn with_reviews(mut game: Game, total: u32, positive: u32) -> Game {
        game.reviews = Reviews { total, positive };
        game
    }

    fn listed(position: usize, game: &Game, order: BrowseOrder, descending: bool) -> Listed {
        Listed::new(position, game, game.id.0, order, descending, NOW)
    }

    fn sorted(order: BrowseOrder, descending: bool, games: &[Game]) -> Vec<u32> {
        let mut listed: Vec<Listed> = games
            .iter()
            .enumerate()
            .map(|(position, game)| listed(position, game, order, descending))
            .collect();
        listed.sort_unstable_by(Listed::compare);
        listed
            .iter()
            .map(|listed| games[listed.position as usize].id.0)
            .collect()
    }

    #[test]
    fn the_defaults_leave_out_the_library_and_the_not_for_me_and_ask_for_a_sale() {
        let options = BrowseOptions::default();

        assert!(options.filters.only_discounted);
        assert_eq!(options.order, BrowseOrder::Reviews);
        assert!(options.descending);
        assert_eq!(options.library, Choice::Without);
        assert_eq!(options.not_for_me, Choice::Without);
        for choice in [
            options.wished,
            options.would_try,
            options.saved,
            options.later,
        ] {
            assert_eq!(choice, Choice::Any);
        }
    }

    #[test]
    fn the_first_click_goes_the_natural_way() {
        for order in [
            BrowseOrder::Reviews,
            BrowseOrder::Discount,
            BrowseOrder::Rating,
        ] {
            assert!(order.descending_first(), "{order:?}");
        }
        for order in [BrowseOrder::Name, BrowseOrder::Price, BrowseOrder::Ends] {
            assert!(!order.descending_first(), "{order:?}");
        }
    }

    fn mark(vote: Option<VoteKind>) -> Mark {
        Mark {
            vote,
            ..Mark::default()
        }
    }

    fn only(change: impl FnOnce(&mut BrowseOptions)) -> BrowseOptions {
        let mut options = BrowseOptions {
            library: Choice::Any,
            not_for_me: Choice::Any,
            ..BrowseOptions::default()
        };
        change(&mut options);
        options
    }

    #[test]
    fn a_choice_looks_at_each_vote() {
        let would_try = mark(Some(VoteKind::WouldTry));
        let later = mark(Some(VoteKind::Later));
        let not_for_me = mark(Some(VoteKind::NotForMe));
        let saved = Mark {
            saved: true,
            ..would_try
        };

        let options = only(|options| options.would_try = Choice::Only);
        assert!(options.allows(would_try));
        assert!(options.allows(saved));
        assert!(!options.allows(later));
        assert!(!options.allows(not_for_me));
        assert!(!options.allows(Mark::default()));

        let options = only(|options| options.later = Choice::Only);
        assert!(options.allows(later));
        assert!(!options.allows(would_try));

        let options = only(|options| options.not_for_me = Choice::Only);
        assert!(options.allows(not_for_me));
        assert!(!options.allows(later));

        let options = only(|options| options.saved = Choice::Only);
        assert!(options.allows(saved));
        assert!(!options.allows(would_try));

        let options = only(|options| options.saved = Choice::Without);
        assert!(!options.allows(saved));
        assert!(options.allows(would_try));
        assert!(options.allows(Mark::default()));
    }

    #[test]
    fn a_choice_looks_at_the_library_and_the_wishlist() {
        let owned = Mark {
            owned: true,
            ..Mark::default()
        };
        let wished = Mark {
            wished: true,
            ..Mark::default()
        };

        let options = only(|options| options.library = Choice::Only);
        assert!(options.allows(owned));
        assert!(!options.allows(wished));

        let options = only(|options| options.library = Choice::Without);
        assert!(!options.allows(owned));
        assert!(options.allows(wished));

        let options = only(|options| options.wished = Choice::Only);
        assert!(options.allows(wished));
        assert!(!options.allows(owned));
    }

    #[test]
    fn two_votes_asked_as_only_leave_nothing() {
        let options = only(|options| {
            options.would_try = Choice::Only;
            options.not_for_me = Choice::Only;
        });

        for kind in [VoteKind::WouldTry, VoteKind::NotForMe, VoteKind::Later] {
            assert!(!options.allows(mark(Some(kind))));
        }
        assert!(!options.allows_unmarked());
    }

    #[test]
    fn a_game_without_marks_passes_unless_a_choice_asks_for_one() {
        assert!(BrowseOptions::default().allows_unmarked());
        assert!(!only(|options| options.later = Choice::Only).allows_unmarked());
        assert!(only(|options| options.later = Choice::Without).allows_unmarked());
    }

    fn vote(kind: VoteKind, at: Option<i64>, saved: bool) -> Vote {
        Vote {
            at: at.map(Timestamp),
            saved,
            ..Vote::new(kind, "2026-11-01")
        }
    }

    #[test]
    fn the_marks_come_from_the_library_the_wishlist_and_the_votes_that_still_count() {
        let signals = Signals {
            read_at: NOW,
            library: vec![LibraryGame {
                id: AppId(1),
                total_hours: 5.0,
                last_played: None,
            }],
            borrowed: Vec::new(),
            recent: vec![RecentGame {
                id: AppId(2),
                total_hours: 1.0,
                recent_hours: 1.0,
            }],
            wishlist: vec![AppId(3)],
            followed: vec![AppId(4)],
        };
        let corrections = Corrections {
            played_elsewhere: [AppId(5)].into(),
            ..Corrections::default()
        };
        let mut votes = Votes::new();
        votes.insert(AppId(6), vote(VoteKind::WouldTry, None, true));
        votes.insert(AppId(7), vote(VoteKind::NotForMe, None, false));
        votes.insert(AppId(8), vote(VoteKind::Later, Some(NOW.0 - DAY_MS), false));
        votes.insert(
            AppId(9),
            vote(VoteKind::Later, Some(NOW.0 - 8 * DAY_MS), false),
        );
        votes.insert(AppId(3), vote(VoteKind::WouldTry, None, false));
        let families = Families::default();
        let context = FilterContext::new(
            &signals,
            &families,
            &votes,
            &corrections,
            &SignalsConfig::default(),
        );

        let marks = marks(&context, NOW);

        for id in [1, 2, 5] {
            assert!(marks[&AppId(id)].owned, "{id}");
        }
        assert!(marks[&AppId(3)].wished);
        assert_eq!(marks[&AppId(3)].vote, Some(VoteKind::WouldTry));
        assert!(marks[&AppId(4)].wished);
        assert_eq!(marks[&AppId(6)].vote, Some(VoteKind::WouldTry));
        assert!(marks[&AppId(6)].saved);
        assert_eq!(marks[&AppId(7)].vote, Some(VoteKind::NotForMe));
        assert_eq!(marks[&AppId(8)].vote, Some(VoteKind::Later));
        assert!(!marks.contains_key(&AppId(9)), "a Later that is over");
        assert_eq!(marks.len(), 8);
    }

    #[test]
    fn reviews_go_by_the_number_and_then_by_the_appid() {
        let games = [
            with_reviews(game(3, 100, None), 50, 50),
            with_reviews(game(1, 100, None), 900, 10),
            with_reviews(game(2, 100, None), 50, 0),
        ];

        assert_eq!(sorted(BrowseOrder::Reviews, true, &games), [1, 2, 3]);
        assert_eq!(sorted(BrowseOrder::Reviews, false, &games), [2, 3, 1]);
    }

    #[test]
    fn the_name_goes_by_the_rank_and_the_direction_turns_it() {
        let games = [
            game(30, 100, None),
            game(10, 100, None),
            game(20, 100, None),
        ];
        let ids = |descending| {
            let mut ranked: Vec<Listed> = games
                .iter()
                .zip([2, 0, 1])
                .enumerate()
                .map(|(position, (game, rank))| {
                    Listed::new(position, game, rank, BrowseOrder::Name, descending, NOW)
                })
                .collect();
            ranked.sort_unstable_by(Listed::compare);
            ranked
                .iter()
                .map(|listed| games[listed.position as usize].id.0)
                .collect::<Vec<_>>()
        };

        assert_eq!(ids(false), [10, 20, 30]);
        assert_eq!(ids(true), [30, 20, 10]);
    }

    #[test]
    fn the_discount_goes_by_the_percent_and_a_game_without_one_is_at_zero() {
        let games = [
            game(1, 1000, None),
            game(2, 1000, Some((50, 500, None))),
            game(3, 1000, Some((10, 900, None))),
            game(4, 1000, Some((50, 500, None))),
        ];

        assert_eq!(sorted(BrowseOrder::Discount, true, &games), [2, 4, 3, 1]);
        assert_eq!(sorted(BrowseOrder::Discount, false, &games), [1, 3, 2, 4]);
    }

    #[test]
    fn the_price_is_the_final_one() {
        let games = [
            game(1, 2000, Some((75, 500, None))),
            game(2, 1000, None),
            game(3, 700, None),
        ];

        assert_eq!(sorted(BrowseOrder::Price, false, &games), [1, 3, 2]);
        assert_eq!(sorted(BrowseOrder::Price, true, &games), [2, 3, 1]);
    }

    #[test]
    fn a_discount_that_is_over_counts_for_neither_the_percent_nor_the_end() {
        let over = game(1, 1000, Some((90, 100, Some(-1))));
        let running = game(2, 1000, Some((10, 900, Some(DAY_MS))));
        let games = [over, running];

        assert_eq!(sorted(BrowseOrder::Discount, true, &games), [2, 1]);
        assert_eq!(sorted(BrowseOrder::Price, false, &games), [2, 1]);
        assert_eq!(sorted(BrowseOrder::Ends, false, &games), [2, 1]);
        assert_eq!(sorted(BrowseOrder::Ends, true, &games), [2, 1]);
    }

    #[test]
    fn the_rating_is_the_share_of_positive_reviews_and_then_the_reviews() {
        let games = [
            with_reviews(game(1, 100, None), 4, 2),
            with_reviews(game(2, 100, None), 100, 50),
            with_reviews(game(3, 100, None), 10, 9),
            with_reviews(game(4, 100, None), 0, 0),
            with_reviews(game(5, 100, None), 1, 0),
        ];

        assert_eq!(sorted(BrowseOrder::Rating, true, &games), [3, 2, 1, 5, 4]);
        assert_eq!(sorted(BrowseOrder::Rating, false, &games), [5, 4, 2, 1, 3]);
    }

    #[test]
    fn the_rating_is_exact_with_the_biggest_counts() {
        // Two shares that differ by 5e-20, with the better one on fewer reviews
        let games = [
            with_reviews(game(1, 100, None), 4_294_967_293, 2_147_483_647),
            with_reviews(game(2, 100, None), 4_294_967_291, 2_147_483_646),
        ];

        assert_eq!(sorted(BrowseOrder::Rating, true, &games), [2, 1]);
        assert_eq!(sorted(BrowseOrder::Rating, false, &games), [1, 2]);
    }

    #[test]
    fn the_games_that_end_soon_come_first_and_the_ones_without_an_end_come_last_either_way() {
        let games = [
            game(1, 1000, None),
            game(2, 1000, Some((50, 500, Some(5 * DAY_MS)))),
            game(3, 1000, Some((50, 500, None))),
            game(4, 1000, Some((50, 500, Some(DAY_MS)))),
            game(5, 1000, Some((50, 500, Some(DAY_MS)))),
        ];

        assert_eq!(sorted(BrowseOrder::Ends, false, &games), [4, 5, 2, 1, 3]);
        assert_eq!(sorted(BrowseOrder::Ends, true, &games), [2, 4, 5, 1, 3]);
    }

    #[test]
    fn what_is_equal_goes_by_reviews_in_both_directions() {
        let games = [
            with_reviews(game(1, 500, None), 10, 10),
            with_reviews(game(2, 500, None), 90, 90),
            with_reviews(game(3, 500, None), 90, 90),
        ];

        assert_eq!(sorted(BrowseOrder::Price, false, &games), [2, 3, 1]);
        assert_eq!(sorted(BrowseOrder::Price, true, &games), [2, 3, 1]);
    }
}
