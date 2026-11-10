use std::{cmp::Reverse, collections::HashSet};

use crate::{families::Family, ids::AppId, ranking::RankedGame};

// Measured: with half the places the signals lose more than the families gain
const SIGNALS_PERCENT: u32 = 80;
// A family made by hand asks for more room, at some cost for the signals
const SIGNALS_PERCENT_WITH_USER_FAMILIES: u32 = 60;
// A family made by hand is a statement, an automatic one a guess
const USER_FAMILY_WEIGHT: u32 = 2;
const AUTOMATIC_FAMILY_WEIGHT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Signals,
    UserFamily,
    AutomaticFamily,
}

impl ListKind {
    pub fn of_family(family: &Family) -> ListKind {
        match family.automatic {
            Some(_) => ListKind::AutomaticFamily,
            None => ListKind::UserFamily,
        }
    }

    fn family_weight(self) -> u32 {
        match self {
            ListKind::Signals => 0,
            ListKind::UserFamily => USER_FAMILY_WEIGHT,
            ListKind::AutomaticFamily => AUTOMATIC_FAMILY_WEIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Pick<'a> {
    pub game: &'a RankedGame,
    pub from: usize,
    pub bet: bool,
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

pub fn turn_shares(kinds: &[ListKind]) -> Vec<u32> {
    if kinds.len() == 1 {
        return vec![1];
    }

    let signals_percent = if kinds.contains(&ListKind::UserFamily) {
        SIGNALS_PERCENT_WITH_USER_FAMILIES
    } else {
        SIGNALS_PERCENT
    };
    let family_weights: u32 = kinds.iter().map(|kind| kind.family_weight()).sum();
    let shares: Vec<u32> = kinds
        .iter()
        .map(|kind| match kind {
            ListKind::Signals => signals_percent * family_weights,
            family => (100 - signals_percent) * family.family_weight(),
        })
        .collect();

    let divisor = shares.iter().copied().fold(0, gcd).max(1);
    shares.into_iter().map(|share| share / divisor).collect()
}

// A weighted round robin: one full round, as long as the sum of the shares.
pub fn turn_order(shares: &[u32]) -> Vec<usize> {
    let total: u32 = shares.iter().sum();
    let mut credits = vec![0_i64; shares.len()];

    (0..total)
        .map(|_| {
            for (credit, &share) in credits.iter_mut().zip(shares) {
                *credit += i64::from(share);
            }
            let from = (0..credits.len())
                .max_by_key(|&i| (credits[i], Reverse(i)))
                .expect("a round has at least one list");
            credits[from] -= i64::from(total);
            from
        })
        .collect()
}

// The turns go one at a time, so that a game placed anywhere, even as a bet,
// is skipped by every list without using up a turn. Skipping it later or
// leaving it out before gives the same places, and a page can go on from
// where the last one stopped.
#[derive(Debug, Clone)]
pub struct Turns<'a> {
    lists: &'a [Vec<RankedGame>],
    order: Vec<usize>,
    turn: usize,
    cursors: Vec<usize>,
}

impl<'a> Turns<'a> {
    pub fn new(lists: &'a [Vec<RankedGame>], shares: &[u32], turn: usize) -> Turns<'a> {
        // Without a vote model there are no bet lists
        let order = if lists.is_empty() {
            Vec::new()
        } else {
            assert_eq!(lists.len(), shares.len(), "one share per list");
            turn_order(shares)
        };

        Turns {
            lists,
            turn: turn.checked_rem(order.len()).unwrap_or(0),
            order,
            cursors: vec![0; lists.len()],
        }
    }

    // The place in turn_order of the next turn
    pub fn turn(&self) -> usize {
        self.turn
    }

    pub fn next(&mut self, seen: &HashSet<AppId>) -> Option<Pick<'a>> {
        // A whole round of empty turns: every list is finished
        for _ in 0..self.order.len() {
            let from = self.order[self.turn];
            self.turn = (self.turn + 1) % self.order.len();

            let list = &self.lists[from];
            let cursor = &mut self.cursors[from];
            while *cursor < list.len() && seen.contains(&list[*cursor].id) {
                *cursor += 1;
            }
            if let Some(game) = list.get(*cursor) {
                *cursor += 1;
                return Some(Pick {
                    game,
                    from,
                    bet: false,
                });
            }
        }
        None
    }
}

pub fn in_turns<'a>(lists: &'a [Vec<RankedGame>], shares: &[u32]) -> Vec<Pick<'a>> {
    let mut turns = Turns::new(lists, shares, 0);
    let mut seen = HashSet::new();

    std::iter::from_fn(|| {
        let pick = turns.next(&seen)?;
        seen.insert(pick.game.id);
        Some(pick)
    })
    .collect()
}

// A bet every bet_every places, counted from the places already given
// before this page.
pub fn with_bets<'a>(
    list: &mut Turns<'a>,
    bets: &mut Turns<'a>,
    bet_every: usize,
    places_before: usize,
    seen: &mut HashSet<AppId>,
    limit: usize,
) -> Vec<Pick<'a>> {
    let mut placed = Vec::new();

    while placed.len() < limit {
        let bet_turn = (places_before + placed.len() + 1).is_multiple_of(bet_every);
        let bet = if bet_turn { bets.next(seen) } else { None };

        let pick = match bet {
            Some(bet) => Some(Pick { bet: true, ..bet }),
            None => list
                .next(seen)
                .or_else(|| bets.next(seen).map(|bet| Pick { bet: true, ..bet })),
        };

        let Some(pick) = pick else {
            break;
        };
        seen.insert(pick.game.id);
        placed.push(pick);
    }

    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    use ListKind::{AutomaticFamily, Signals, UserFamily};

    fn ranked(id: u32) -> RankedGame {
        RankedGame {
            id: AppId(id),
            relevance: 0.0,
            references: Vec::new(),
            affinity: 0.0,
        }
    }

    fn list(ids: impl IntoIterator<Item = u32>) -> Vec<RankedGame> {
        ids.into_iter().map(ranked).collect()
    }

    fn ids(picks: &[Pick]) -> Vec<u32> {
        picks.iter().map(|pick| pick.game.id.0).collect()
    }

    fn froms(picks: &[Pick]) -> Vec<usize> {
        picks.iter().map(|pick| pick.from).collect()
    }

    fn family(automatic: Option<&str>) -> Family {
        Family {
            name: "Puzzle".to_string(),
            games: vec![AppId(1)],
            automatic: automatic.map(str::to_string),
        }
    }

    #[test]
    fn a_single_list_takes_every_place() {
        for kind in [Signals, UserFamily, AutomaticFamily] {
            let shares = turn_shares(&[kind]);
            assert_eq!(turn_order(&shares), [0]);

            let lists = [list([3, 1, 2])];
            assert_eq!(ids(&in_turns(&lists, &shares)), [3, 1, 2]);
        }
    }

    #[test]
    fn the_signals_keep_four_places_in_five_next_to_an_automatic_family() {
        let shares = turn_shares(&[Signals, AutomaticFamily]);

        assert_eq!(turn_order(&shares), [0, 0, 1, 0, 0]);
    }

    #[test]
    fn two_automatic_families_take_one_place_in_ten_each() {
        let shares = turn_shares(&[Signals, AutomaticFamily, AutomaticFamily]);

        assert_eq!(turn_order(&shares), [0, 0, 0, 1, 0, 0, 2, 0, 0, 0]);
    }

    #[test]
    fn the_signals_keep_three_places_in_five_next_to_a_family_of_the_user() {
        let shares = turn_shares(&[Signals, UserFamily]);
        let order = turn_order(&shares);

        assert_eq!(order.len(), 5);
        assert_eq!(order.iter().filter(|&&from| from == 0).count(), 3);
    }

    #[test]
    fn a_family_of_the_user_weighs_twice_an_automatic_one() {
        let kinds = [
            Signals,
            UserFamily,
            AutomaticFamily,
            UserFamily,
            AutomaticFamily,
        ];
        let lists: Vec<Vec<RankedGame>> = (0..5).map(|i| list(i * 1000..i * 1000 + 400)).collect();

        let picks = in_turns(&lists, &turn_shares(&kinds));
        let places = |from: usize| picks[..300].iter().filter(|pick| pick.from == from).count();

        assert_eq!(places(0), 180);
        for (user, automatic) in [(1, 2), (3, 4), (1, 4), (3, 2)] {
            assert!(places(user).abs_diff(2 * places(automatic)) <= 1);
        }
    }

    #[test]
    fn a_changed_automatic_family_weighs_as_one_of_the_user() {
        assert_eq!(
            ListKind::of_family(&family(Some("2026-11-10"))),
            AutomaticFamily
        );
        assert_eq!(ListKind::of_family(&family(None)), UserFamily);
        assert_eq!(
            turn_shares(&[Signals, ListKind::of_family(&family(None))]),
            turn_shares(&[Signals, UserFamily])
        );
    }

    #[test]
    fn a_game_in_two_lists_comes_once_at_the_first_turn_that_finds_it() {
        let lists = [list([1, 2, 3, 4, 5]), list([2, 6])];

        let picks = in_turns(&lists, &turn_shares(&[Signals, AutomaticFamily]));

        assert_eq!(ids(&picks), [1, 2, 6, 3, 4, 5]);
        assert_eq!(froms(&picks), [0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn a_finished_list_leaves_its_turns_and_the_others_go_on() {
        let lists = [list([1]), list([2, 3, 4])];

        let picks = in_turns(&lists, &turn_shares(&[Signals, AutomaticFamily]));

        assert_eq!(ids(&picks), [1, 2, 3, 4]);
    }

    #[test]
    fn turns_from_a_point_of_the_round_give_the_rest_of_the_whole_sequence() {
        let kinds = [Signals, UserFamily, AutomaticFamily];
        let shares = turn_shares(&kinds);
        let lists = [list(1..40), list([3, 40, 41, 5, 42]), list([50, 2, 51])];
        let whole = in_turns(&lists, &shares);

        for shown in 0..=whole.len() {
            let mut turns = Turns::new(&lists, &shares, 0);
            let mut seen = HashSet::new();
            for _ in 0..shown {
                seen.insert(turns.next(&seen).unwrap().game.id);
            }

            let mut rest = Turns::new(&lists, &shares, turns.turn());
            let rest: Vec<Pick> = std::iter::from_fn(|| {
                let pick = rest.next(&seen)?;
                seen.insert(pick.game.id);
                Some(pick)
            })
            .collect();

            assert_eq!(ids(&rest), ids(&whole[shown..]));
            assert_eq!(froms(&rest), froms(&whole[shown..]));
        }
    }

    #[test]
    fn the_same_lists_give_the_same_turns() {
        let kinds = [Signals, UserFamily, AutomaticFamily];
        let lists = [list(1..50), list(30..80), list(60..120)];

        let first = in_turns(&lists, &turn_shares(&kinds));
        let again = in_turns(&lists, &turn_shares(&kinds));

        assert_eq!(ids(&first), ids(&again));
        assert_eq!(froms(&first), froms(&again));
        assert_eq!(first.len(), 119);
    }
}
