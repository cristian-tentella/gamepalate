use std::collections::HashSet;

use crate::{ids::AppId, ranking::RankedGame};

#[derive(Debug, Clone, Copy)]
pub struct Pick<'a> {
    pub game: &'a RankedGame,
    pub from: usize,
    pub bet: bool,
}

pub fn turn_order(k: usize) -> Vec<usize> {
    if k == 1 {
        return vec![0];
    }
    (1..k).flat_map(|i| [0, i]).collect()
}

pub fn in_turns<'a>(lists: &'a [Vec<RankedGame>]) -> Vec<Pick<'a>> {
    if lists.is_empty() {
        return Vec::new();
    }

    let order = turn_order(lists.len());
    let mut cursors = vec![0; lists.len()];
    let mut seen = HashSet::new();
    let mut picks = Vec::new();
    let mut empty_turns = 0;

    for &from in order.iter().cycle() {
        let list = &lists[from];
        let cursor = &mut cursors[from];
        while *cursor < list.len() && seen.contains(&list[*cursor].id) {
            *cursor += 1;
        }

        match list.get(*cursor) {
            Some(game) => {
                seen.insert(game.id);
                picks.push(Pick {
                    game,
                    from,
                    bet: false,
                });
                *cursor += 1;
                empty_turns = 0;
            }
            None => {
                empty_turns += 1;
                if empty_turns == order.len() {
                    break;
                }
            }
        }
    }

    picks
}

fn next_unseen<'a>(
    picks: &[Pick<'a>],
    cursor: &mut usize,
    seen: &HashSet<AppId>,
) -> Option<Pick<'a>> {
    while let Some(&pick) = picks.get(*cursor) {
        *cursor += 1;
        if !seen.contains(&pick.game.id) {
            return Some(pick);
        }
    }
    None
}

pub fn with_bets<'a>(list: Vec<Pick<'a>>, bets: Vec<Pick<'a>>, bet_every: usize) -> Vec<Pick<'a>> {
    let mut list_cursor = 0;
    let mut bets_cursor = 0;
    let mut seen = HashSet::new();
    let mut placed = Vec::new();

    loop {
        let bet_turn = (placed.len() + 1) % bet_every == 0;
        let bet = if bet_turn {
            next_unseen(&bets, &mut bets_cursor, &seen)
        } else {
            None
        };

        let pick = match bet {
            Some(bet) => Some(Pick { bet: true, ..bet }),
            None => next_unseen(&list, &mut list_cursor, &seen).or_else(|| {
                next_unseen(&bets, &mut bets_cursor, &seen).map(|bet| Pick { bet: true, ..bet })
            }),
        };

        let Some(pick) = pick else {
            break;
        };
        seen.insert(pick.game.id);
        placed.push(pick);
    }

    placed
}
