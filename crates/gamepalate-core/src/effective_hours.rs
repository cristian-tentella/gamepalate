use std::collections::BTreeMap;

use crate::{
    config::SignalsConfig,
    ids::AppId,
    signals::Signals,
    votes::{VoteKind, Votes},
};

pub fn effective_hours(
    signals: &Signals,
    votes: &Votes,
    signals_config: &SignalsConfig,
) -> BTreeMap<AppId, f64> {
    let mut hours: BTreeMap<AppId, f64> = signals
        .library
        .iter()
        .map(|g| (g.id, g.total_hours))
        .collect();

    fn keep_max_hours(hours: &mut BTreeMap<AppId, f64>, game_id: AppId, new_hours: f64) {
        hours
            .entry(game_id)
            .and_modify(|h| *h = h.max(new_hours))
            .or_insert(new_hours);
    }

    for g in &signals.recent {
        hours.insert(
            g.id,
            g.total_hours - g.recent_hours
                + g.recent_hours * signals_config.recent_hours_multiplier,
        );
    }

    for &id in &signals.followed {
        keep_max_hours(&mut hours, id, signals_config.followed_hours);
    }

    for &id in &signals.wishlist {
        keep_max_hours(&mut hours, id, signals_config.wishlist_hours);
    }

    for (&id, _) in votes
        .iter()
        .filter(|(_, vote)| vote.kind == VoteKind::WouldTry)
    {
        keep_max_hours(&mut hours, id, signals_config.would_try_hours);
    }

    hours
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        signals::{LibraryGame, RecentGame},
        timestamp::Timestamp,
        votes::Vote,
    };

    fn signals() -> Signals {
        Signals {
            read_at: Timestamp(0),
            library: vec![
                LibraryGame {
                    id: AppId(1),
                    total_hours: 100.0,
                },
                LibraryGame {
                    id: AppId(2),
                    total_hours: 1.0,
                },
                LibraryGame {
                    id: AppId(3),
                    total_hours: 50.0,
                },
            ],
            recent: vec![RecentGame {
                id: AppId(1),
                total_hours: 100.0,
                recent_hours: 5.0,
            }],
            wishlist: vec![AppId(3), AppId(4)],
            followed: vec![AppId(2)],
        }
    }

    fn vote(kind: VoteKind) -> Vote {
        Vote {
            kind,
            date: "2026-10-01".to_string(),
            bet: false,
        }
    }

    #[test]
    fn effective_hours_count_recent_hours_three_times() {
        let hours = effective_hours(&signals(), &Votes::new(), &SignalsConfig::default());
        assert_eq!(hours[&AppId(1)], 110.0);
    }

    #[test]
    fn effective_hours_keep_the_highest_value_of_every_source() {
        let hours = effective_hours(&signals(), &Votes::new(), &SignalsConfig::default());
        assert_eq!(hours[&AppId(2)], 3.0);
        assert_eq!(hours[&AppId(3)], 50.0);
        assert_eq!(hours[&AppId(4)], 5.0);
    }

    #[test]
    fn effective_hours_add_only_would_try_votes() {
        let votes = Votes::from([
            (AppId(10), vote(VoteKind::WouldTry)),
            (AppId(20), vote(VoteKind::Maybe)),
        ]);
        let hours = effective_hours(&signals(), &votes, &SignalsConfig::default());
        assert_eq!(hours[&AppId(10)], 5.0);
        assert!(!hours.contains_key(&AppId(20)));
    }
}
