use std::collections::BTreeMap;

use crate::ids::AppId;

pub type Votes = BTreeMap<AppId, Vote>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    pub kind: VoteKind,
    pub date: String,
    pub bet: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VoteKind {
    WouldTry,
    Maybe,
    NotForMe,
    Known,
}

pub fn cast(votes: &mut Votes, id: AppId, kind: VoteKind, date: String, bet: bool) {
    let bet = bet || votes.get(&id).is_some_and(|vote| vote.bet);
    votes.insert(id, Vote { kind, date, bet });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast_on(votes: &mut Votes, kind: VoteKind, bet: bool) {
        cast(votes, AppId(620), kind, "2026-10-30".to_string(), bet);
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
        );

        cast_on(&mut votes, VoteKind::WouldTry, false);

        assert_eq!(
            votes[&AppId(620)],
            Vote {
                kind: VoteKind::WouldTry,
                date: "2026-10-30".to_string(),
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
}
