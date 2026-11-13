use std::fmt;

use gamepalate_app::{LibraryGameView, names::folded};

const FILTERS: [Filter; 7] = [
    Filter::All,
    Filter::Owned,
    Filter::NotOwned,
    Filter::Wishlist,
    Filter::Followed,
    Filter::Elsewhere,
    Filter::Corrected,
];

pub(super) const SORTS: [Sort; 3] = [Sort::TasteWeight, Sort::Hours, Sort::Name];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    All,
    Owned,
    NotOwned,
    Wishlist,
    Followed,
    Elsewhere,
    Corrected,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sort {
    TasteWeight,
    Hours,
    Name,
}

impl Filter {
    fn keeps(self, game: &LibraryGameView) -> bool {
        match self {
            Filter::All => true,
            Filter::Owned => game.owned_hours.is_some(),
            Filter::NotOwned => game.borrowed,
            Filter::Wishlist => game.wishlist,
            Filter::Followed => game.followed,
            Filter::Elsewhere => game.elsewhere,
            Filter::Corrected => {
                game.correction.multiplier.is_some() || game.correction.hidden || game.held_out
            }
        }
    }

    fn hides_when_empty(self) -> bool {
        matches!(
            self,
            Filter::NotOwned | Filter::Elsewhere | Filter::Corrected
        )
    }
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Sort::TasteWeight => "Taste weight",
            Sort::Hours => "Hours",
            Sort::Name => "Name",
        })
    }
}

/// A filter with its count, as the "Show" menu lists it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Choice {
    pub filter: Filter,
    count: usize,
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} · {}", filter_label(self.filter), self.count)
    }
}

pub(super) fn choice(games: &[LibraryGameView], filter: Filter) -> Choice {
    Choice {
        filter,
        count: count(games, filter),
    }
}

/// The filters the "Show" menu offers: the ones that would come up empty are left out.
pub(super) fn choices(games: &[LibraryGameView]) -> Vec<Choice> {
    FILTERS
        .into_iter()
        .filter(|&filter| shows_filter(filter, games))
        .map(|filter| choice(games, filter))
        .collect()
}

pub(super) fn shows_filter(filter: Filter, games: &[LibraryGameView]) -> bool {
    !filter.hides_when_empty() || count(games, filter) > 0
}

pub(super) fn filter_label(filter: Filter) -> &'static str {
    match filter {
        Filter::All => "All",
        Filter::Owned => "Owned",
        Filter::NotOwned => "Not owned",
        Filter::Wishlist => "Wishlist",
        Filter::Followed => "Followed",
        Filter::Elsewhere => "Elsewhere",
        Filter::Corrected => "Corrected",
    }
}

pub(super) fn count(games: &[LibraryGameView], filter: Filter) -> usize {
    games.iter().filter(|game| filter.keeps(game)).count()
}

pub(super) fn shown(
    games: &[LibraryGameView],
    filter: Filter,
    sort: Sort,
    search: &str,
) -> Vec<usize> {
    let search = folded(search.trim());
    let mut shown: Vec<usize> = (0..games.len())
        .filter(|&index| {
            let game = &games[index];
            if search.is_empty() {
                filter.keeps(game)
            } else {
                folded(&game.name).contains(&search)
            }
        })
        .collect();
    let name = |index: &usize| folded(&games[*index].name);

    match sort {
        Sort::TasteWeight => shown.sort_by(|a, b| {
            let (a_game, b_game) = (&games[*a], &games[*b]);
            b_game
                .counts
                .cmp(&a_game.counts)
                .then(if a_game.counts {
                    b_game.share.total_cmp(&a_game.share)
                } else {
                    b_game.hours.total_cmp(&a_game.hours)
                })
                .then_with(|| name(a).cmp(&name(b)))
        }),
        Sort::Hours => shown.sort_by(|a, b| {
            games[*b]
                .hours
                .total_cmp(&games[*a].hours)
                .then_with(|| name(a).cmp(&name(b)))
        }),
        Sort::Name => shown.sort_by_cached_key(name),
    }

    shown
}

pub(super) fn empty_text(no_games: bool, filter: Filter, search: &str) -> String {
    let search = search.trim();
    if no_games {
        "No games in your Signals yet".to_string()
    } else if !search.is_empty() {
        format!("No game matches \"{search}\"")
    } else {
        match filter {
            Filter::All | Filter::Owned => "No games in your library".to_string(),
            Filter::NotOwned => "No games played without owning them".to_string(),
            Filter::Wishlist => "No games in your wishlist".to_string(),
            Filter::Followed => "No followed games".to_string(),
            Filter::Elsewhere => "No games played elsewhere".to_string(),
            Filter::Corrected => "No corrected games".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use gamepalate_app::GameCorrection;

    #[test]
    fn taste_weight_puts_the_games_that_do_not_count_last_by_hours() {
        let games = games();

        let shown = shown(&games, Filter::All, Sort::TasteWeight, "");

        assert_eq!(ids(&games, &shown), [2, 1, 4, 5, 3, 6]);
    }

    #[test]
    fn hours_and_name_sort_every_game_together() {
        let games = games();

        let by_hours = shown(&games, Filter::All, Sort::Hours, "");
        let by_name = shown(&games, Filter::All, Sort::Name, "");

        assert_eq!(ids(&games, &by_hours), [2, 1, 4, 5, 3, 6]);
        assert_eq!(ids(&games, &by_name), [6, 3, 2, 5, 1, 4]);
    }

    #[test]
    fn a_filter_keeps_one_signal() {
        let games = games();
        let kept = |filter| ids(&games, &shown(&games, filter, Sort::Name, ""));

        assert_eq!(kept(Filter::Owned), [6, 3, 2, 1]);
        assert_eq!(kept(Filter::Wishlist), [4]);
        assert_eq!(kept(Filter::Followed), [5]);
        assert_eq!(count(&games, Filter::All), 6);
        assert_eq!(count(&games, Filter::Owned), 4);
    }

    #[test]
    fn the_search_ignores_case_accents_and_the_filter() {
        let games = games();
        let found = |search| ids(&games, &shown(&games, Filter::Wishlist, Sort::Name, search));

        assert_eq!(found("okami"), [5]);
        assert_eq!(found("  HADES "), [2]);
        assert_eq!(found("e"), [6, 3, 2, 1, 4]);
        assert!(found("xyz").is_empty());
    }

    #[test]
    fn not_owned_keeps_the_games_played_without_owning_them() {
        let mut games = games();
        games.push(LibraryGameView {
            borrowed: true,
            hours: 40.0,
            ..game(7, "Baldur's Gate 3")
        });

        assert_eq!(
            ids(&games, &shown(&games, Filter::NotOwned, Sort::Name, "")),
            [7]
        );
        assert_eq!(count(&games, Filter::Owned), 4);
        assert_eq!(count(&games, Filter::All), 7);
        assert_eq!(
            empty_text(false, Filter::NotOwned, ""),
            "No games played without owning them"
        );
    }

    #[test]
    fn corrected_keeps_the_games_with_a_correction_or_held_out() {
        let mut games = games();
        games.push(LibraryGameView {
            correction: GameCorrection {
                multiplier: Some(0.5),
                hidden: false,
            },
            ..owned(7, "Valheim", 266.0, 0.1)
        });
        games.push(LibraryGameView {
            correction: GameCorrection {
                multiplier: None,
                hidden: true,
            },
            ..game(8, "Splitgate")
        });
        games.push(LibraryGameView {
            held_out: true,
            ..owned(9, "Factorio", 90.0, 0.1)
        });
        games.push(LibraryGameView {
            families: vec!["Strategy".to_string()],
            ..owned(10, "Civilization VI", 70.0, 0.1)
        });

        assert_eq!(
            ids(&games, &shown(&games, Filter::Corrected, Sort::Name, "")),
            [9, 8, 7]
        );
        assert_eq!(count(&games, Filter::Corrected), 3);
        assert_eq!(count(&games[..6], Filter::Corrected), 0);
        assert!(Filter::Corrected.hides_when_empty());
        assert!(!Filter::Followed.hides_when_empty());
        assert_eq!(
            empty_text(false, Filter::Corrected, ""),
            "No corrected games"
        );
    }

    #[test]
    fn the_elsewhere_filter_shows_with_the_first_game_played_elsewhere() {
        let mut games = games();
        assert!(!shows_filter(Filter::Elsewhere, &games));
        assert!(shows_filter(Filter::Followed, &games));

        games.push(elsewhere(7, "It Takes Two"));
        assert!(shows_filter(Filter::Elsewhere, &games));
        assert_eq!(
            ids(&games, &shown(&games, Filter::Elsewhere, Sort::Name, "")),
            [7]
        );
        assert_eq!(filter_label(Filter::Elsewhere), "Elsewhere");

        assert_eq!(
            choices(&games)
                .into_iter()
                .map(|choice| choice.to_string())
                .collect::<Vec<_>>(),
            [
                "All · 7",
                "Owned · 4",
                "Wishlist · 1",
                "Followed · 1",
                "Elsewhere · 1"
            ]
        );

        games.pop();
        assert!(!shows_filter(Filter::Elsewhere, &games));
        assert_eq!(
            empty_text(false, Filter::Elsewhere, ""),
            "No games played elsewhere"
        );
    }

    #[test]
    fn an_empty_grid_says_why() {
        assert_eq!(
            empty_text(true, Filter::All, ""),
            "No games in your Signals yet"
        );
        assert_eq!(
            empty_text(false, Filter::All, " xyz "),
            "No game matches \"xyz\""
        );
        assert_eq!(
            empty_text(false, Filter::Wishlist, ""),
            "No games in your wishlist"
        );
    }
}
