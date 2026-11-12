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
    Later,
    NotForMe,
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
// A guess, not a measure: long enough for a sale or a mood to change.
pub const LATER_DAYS: i64 = 30;

impl Vote {
    /// A Later whose days are over: the game is back, as if it had no vote.
    pub fn has_lapsed(&self, now: Timestamp) -> bool {
        if self.kind != VoteKind::Later {
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
        .filter(|vote| vote.kind != VoteKind::Later)
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
            VoteKind::Later,
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
        cast_on(&mut votes, VoteKind::Later, true);

        cast_on(&mut votes, VoteKind::NotForMe, false);

        assert!(votes[&AppId(620)].bet);
    }

    fn cast_at(votes: &mut Votes, id: u32, at: Option<i64>) {
        cast(
            votes,
            AppId(id),
            VoteKind::WouldTry,
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

    #[test]
    fn a_later_does_not_count_among_the_votes_of_the_last_day() {
        let now = 10 * DAY_MS;
        let mut votes = Votes::new();
        cast_at(&mut votes, 1, Some(now - 1));
        cast(
            &mut votes,
            AppId(2),
            VoteKind::Later,
            "2026-11-07".to_string(),
            false,
            Some(Timestamp(now - 1)),
        );

        assert_eq!(cast_in_last_day(&votes, Timestamp(now)), 1);
    }

    fn vote(kind: VoteKind, date: &str, at: Option<i64>) -> Vote {
        Vote {
            kind,
            date: date.to_string(),
            at: at.map(Timestamp),
            bet: false,
        }
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
            VoteKind::Later,
            "2026-11-11".to_string(),
            false,
            Some(Timestamp(day)),
        );
        let back = Timestamp(day + LATER_DAYS * DAY_MS);
        assert!(standing(&votes, AppId(620), back).is_none());

        cast(
            &mut votes,
            AppId(620),
            VoteKind::Later,
            "2026-12-11".to_string(),
            false,
            Some(back),
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
            VoteKind::Later,
            "2026-11-11".to_string(),
            false,
            Some(Timestamp(day)),
        );

        cast(
            &mut votes,
            AppId(620),
            VoteKind::NotForMe,
            "2026-11-11".to_string(),
            false,
            Some(Timestamp(day + 1)),
        );

        let far = Timestamp(day + 2 * LATER_DAYS * DAY_MS);
        assert_eq!(
            standing(&votes, AppId(620), far).map(|vote| vote.kind),
            Some(VoteKind::NotForMe)
        );
    }
}
