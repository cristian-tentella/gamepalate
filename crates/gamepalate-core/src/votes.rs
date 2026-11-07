use std::collections::BTreeMap;

use crate::{ids::AppId, timestamp::Timestamp};

pub type Votes = BTreeMap<AppId, Vote>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    pub kind: VoteKind,
    pub date: String,
    pub at: Option<Timestamp>,
    pub bet: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VoteKind {
    WouldTry,
    Maybe,
    NotForMe,
    Known,
}

pub fn cast(
    votes: &mut Votes,
    id: AppId,
    kind: VoteKind,
    date: String,
    bet: bool,
    at: Option<Timestamp>,
) {
    let bet = bet || votes.get(&id).is_some_and(|vote| vote.bet);
    votes.insert(
        id,
        Vote {
            kind,
            date,
            at,
            bet,
        },
    );
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Votes cast in the 24 hours before `now`. Votes saved without a time do not count.
pub fn cast_in_last_day(votes: &Votes, now: Timestamp) -> usize {
    votes
        .values()
        .filter(|vote| {
            vote.at
                .is_some_and(|at| (0..DAY_MS).contains(&now.ms_since(at)))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast_on(votes: &mut Votes, kind: VoteKind, bet: bool) {
        cast(votes, AppId(620), kind, "2026-10-30".to_string(), bet, None);
    }

    #[test]
    fn a_new_vote_replaces_the_old_one() {
        let mut votes = Votes::new();
        cast(
            &mut votes,
            AppId(620),
            VoteKind::Maybe,
            "2026-10-29".to_string(),
            false,
            None,
        );

        cast_on(&mut votes, VoteKind::WouldTry, false);

        assert_eq!(
            votes[&AppId(620)],
            Vote {
                kind: VoteKind::WouldTry,
                date: "2026-10-30".to_string(),
                at: None,
                bet: false,
            }
        );
    }

    #[test]
    fn a_bet_stays_a_bet() {
        let mut votes = Votes::new();
        cast_on(&mut votes, VoteKind::Maybe, true);

        cast_on(&mut votes, VoteKind::NotForMe, false);

        assert!(votes[&AppId(620)].bet);
    }

    fn cast_at(votes: &mut Votes, id: u32, at: Option<i64>) {
        cast(
            votes,
            AppId(id),
            VoteKind::Maybe,
            "2026-11-07".to_string(),
            false,
            at.map(Timestamp),
        );
    }

    #[test]
    fn only_the_votes_of_the_last_24_hours_count() {
        let now = 10 * DAY_MS;
        let mut votes = Votes::new();
        cast_at(&mut votes, 1, Some(now - 1));
        cast_at(&mut votes, 2, Some(now - DAY_MS + 1));
        cast_at(&mut votes, 3, Some(now - DAY_MS));
        cast_at(&mut votes, 4, Some(now + 1));
        cast_at(&mut votes, 5, None);

        assert_eq!(cast_in_last_day(&votes, Timestamp(now)), 2);
    }
}
