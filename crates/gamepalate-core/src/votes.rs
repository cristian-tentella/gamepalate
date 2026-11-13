use std::collections::BTreeMap;

use crate::{ids::AppId, timestamp::Timestamp};

pub type Votes = BTreeMap<AppId, Vote>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    pub kind: VoteKind,
    pub date: String,
    pub at: Option<Timestamp>,
    pub bet: bool,
    pub genre_ok: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VoteKind {
    WouldTry,
    Later,
    NotForMe,
}

/// What each vote means. Every part of gamepalate that treats votes differently asks here.
impl VoteKind {
    /// A Would try: the game counts in the taste, like a game played.
    pub fn is_yes(self) -> bool {
        self == VoteKind::WouldTry
    }

    /// A Not for me: the Filter of the same name keeps the game out, and only a no can have `genre_ok`.
    pub fn is_no(self) -> bool {
        self == VoteKind::NotForMe
    }

    /// What the vote teaches the Parere: 1 for a yes, 0 for a no. A Later only passes a game, so
    /// it teaches nothing, and it is not counted among the votes of the day or the Bets.
    pub fn label(self) -> Option<f64> {
        match self {
            VoteKind::WouldTry => Some(1.0),
            VoteKind::NotForMe => Some(0.0),
            VoteKind::Later => None,
        }
    }

    /// The vote keeps the game out of Discover only for `LATER_DAYS`; the others keep it out for good.
    pub fn lapses(self) -> bool {
        self == VoteKind::Later
    }
}

/// Saves `vote` in place of the old one. A bet stays a bet; `genre_ok` holds only on a Not for me.
pub fn cast(votes: &mut Votes, id: AppId, vote: Vote) {
    let bet = vote.bet || votes.get(&id).is_some_and(|old| old.bet);
    let genre_ok = vote.genre_ok && vote.kind.is_no();
    votes.insert(
        id,
        Vote {
            bet,
            genre_ok,
            ..vote
        },
    );
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
// A guess, not a measure: long enough for a sale or a mood to change.
pub const LATER_DAYS: i64 = 30;

impl Vote {
    /// A vote cast on `date`, without a time, a bet or `genre_ok`.
    pub fn new(kind: VoteKind, date: &str) -> Vote {
        Vote {
            kind,
            date: date.to_string(),
            at: None,
            bet: false,
            genre_ok: false,
        }
    }

    /// A Later whose days are over: the game is back, as if it had no vote.
    pub fn has_lapsed(&self, now: Timestamp) -> bool {
        if !self.kind.lapses() {
            return false;
        }
        let since = self.at.or_else(|| day_start(&self.date));
        since.is_some_and(|since| now.ms_since(since) >= LATER_DAYS * DAY_MS)
    }
}

/// The vote that still keeps the game out, if any.
pub fn standing(votes: &Votes, id: AppId, now: Timestamp) -> Option<&Vote> {
    votes.get(&id).filter(|vote| !vote.has_lapsed(now))
}

// A vote saved without a time has only its day: its midnight in UTC is close enough for 30 days.
fn day_start(date: &str) -> Option<Timestamp> {
    let mut parts = date.splitn(3, '-').map(str::parse::<i64>);
    let (Some(Ok(year)), Some(Ok(month)), Some(Ok(day))) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    Some(Timestamp((era * 146_097 + day_of_era - 719_468) * DAY_MS))
}

/// Votes cast in the 24 hours before `now`. Votes saved without a time do not count,
/// and neither do the Laters, which only pass a game.
pub fn cast_in_last_day(votes: &Votes, now: Timestamp) -> usize {
    votes
        .values()
        .filter(|vote| vote.kind.label().is_some())
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
        cast(
            votes,
            AppId(620),
            Vote {
                bet,
                ..vote(kind, "2026-10-30", None)
            },
        );
    }

    fn not_this_one(votes: &mut Votes) {
        cast(
            votes,
            AppId(620),
            Vote {
                genre_ok: true,
                ..vote(VoteKind::NotForMe, "2026-10-30", None)
            },
        );
    }

    #[test]
    fn a_new_vote_replaces_the_old_one() {
        let mut votes = Votes::new();
        cast(
            &mut votes,
            AppId(620),
            vote(VoteKind::Later, "2026-10-29", None),
        );

        cast_on(&mut votes, VoteKind::WouldTry, false);

        assert_eq!(
            votes[&AppId(620)],
            vote(VoteKind::WouldTry, "2026-10-30", None)
        );
    }

    #[test]
    fn a_bet_stays_a_bet() {
        let mut votes = Votes::new();
        cast_on(&mut votes, VoteKind::Later, true);

        cast_on(&mut votes, VoteKind::NotForMe, false);

        assert!(votes[&AppId(620)].bet);
    }

    #[test]
    fn not_this_one_is_a_not_for_me_with_the_genre_ok() {
        let mut votes = Votes::new();

        not_this_one(&mut votes);

        assert_eq!(votes[&AppId(620)].kind, VoteKind::NotForMe);
        assert!(votes[&AppId(620)].genre_ok);
    }

    #[test]
    fn the_genre_ok_is_lost_when_the_vote_changes() {
        for kind in [VoteKind::WouldTry, VoteKind::Later, VoteKind::NotForMe] {
            let mut votes = Votes::new();
            not_this_one(&mut votes);

            cast_on(&mut votes, kind, false);

            assert!(!votes[&AppId(620)].genre_ok);
        }
    }

    #[test]
    fn only_a_not_for_me_keeps_the_genre_ok() {
        let mut votes = Votes::new();

        cast(
            &mut votes,
            AppId(620),
            Vote {
                genre_ok: true,
                ..vote(VoteKind::WouldTry, "2026-10-30", None)
            },
        );

        assert!(!votes[&AppId(620)].genre_ok);
    }

    fn cast_at(votes: &mut Votes, id: u32, at: Option<i64>) {
        cast(votes, AppId(id), vote(VoteKind::WouldTry, "2026-11-07", at));
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

    #[test]
    fn a_later_does_not_count_among_the_votes_of_the_last_day() {
        let now = 10 * DAY_MS;
        let mut votes = Votes::new();
        cast_at(&mut votes, 1, Some(now - 1));
        cast(
            &mut votes,
            AppId(2),
            vote(VoteKind::Later, "2026-11-07", Some(now - 1)),
        );

        assert_eq!(cast_in_last_day(&votes, Timestamp(now)), 1);
    }

    fn vote(kind: VoteKind, date: &str, at: Option<i64>) -> Vote {
        Vote {
            at: at.map(Timestamp),
            ..Vote::new(kind, date)
        }
    }

    #[test]
    fn only_a_would_try_and_a_not_for_me_teach_the_opinion() {
        assert_eq!(VoteKind::WouldTry.label(), Some(1.0));
        assert_eq!(VoteKind::NotForMe.label(), Some(0.0));
        assert_eq!(VoteKind::Later.label(), None);
    }

    #[test]
    fn a_later_lapses_30_days_after_it_was_cast() {
        let at = 1_794_404_475_152;
        let later = vote(VoteKind::Later, "2026-11-11", Some(at));

        assert!(!later.has_lapsed(Timestamp(at + LATER_DAYS * DAY_MS - 1)));
        assert!(later.has_lapsed(Timestamp(at + LATER_DAYS * DAY_MS)));
    }

    #[test]
    fn a_later_without_a_time_lapses_30_days_after_its_day() {
        let later = vote(VoteKind::Later, "2026-11-11", None);
        let day = day_start("2026-11-11").unwrap();

        assert_eq!(day, Timestamp(1_794_355_200_000));
        assert!(!later.has_lapsed(Timestamp(day.0 + LATER_DAYS * DAY_MS - 1)));
        assert!(later.has_lapsed(Timestamp(day.0 + LATER_DAYS * DAY_MS)));
    }

    #[test]
    fn the_start_of_a_day_is_its_midnight() {
        assert_eq!(day_start("1970-01-01"), Some(Timestamp(0)));
        assert_eq!(day_start("2024-03-01"), Some(Timestamp(1_709_251_200_000)));
        assert_eq!(day_start("2026-13-01"), None);
        assert_eq!(day_start("2026-11"), None);
    }

    #[test]
    fn only_a_later_lapses() {
        let long_after = Timestamp(i64::MAX / 2);
        for kind in [VoteKind::WouldTry, VoteKind::NotForMe] {
            assert!(!vote(kind, "2020-01-01", Some(0)).has_lapsed(long_after));
        }
    }

    #[test]
    fn another_later_starts_the_days_again() {
        let day = 1_794_404_475_152;
        let mut votes = Votes::new();
        cast(
            &mut votes,
            AppId(620),
            vote(VoteKind::Later, "2026-11-11", Some(day)),
        );
        let back = Timestamp(day + LATER_DAYS * DAY_MS);
        assert!(standing(&votes, AppId(620), back).is_none());

        cast(
            &mut votes,
            AppId(620),
            vote(VoteKind::Later, "2026-12-11", Some(back.0)),
        );

        assert!(standing(&votes, AppId(620), back).is_some());
        assert!(standing(&votes, AppId(620), Timestamp(back.0 + LATER_DAYS * DAY_MS)).is_none());
    }

    #[test]
    fn a_vote_replaces_a_later() {
        let day = 1_794_404_475_152;
        let mut votes = Votes::new();
        cast(
            &mut votes,
            AppId(620),
            vote(VoteKind::Later, "2026-11-11", Some(day)),
        );

        cast(
            &mut votes,
            AppId(620),
            vote(VoteKind::NotForMe, "2026-11-11", Some(day + 1)),
        );

        let far = Timestamp(day + 2 * LATER_DAYS * DAY_MS);
        assert_eq!(
            standing(&votes, AppId(620), far).map(|vote| vote.kind),
            Some(VoteKind::NotForMe)
        );
    }
}
