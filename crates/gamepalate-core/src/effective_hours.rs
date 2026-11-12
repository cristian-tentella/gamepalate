use std::collections::{BTreeMap, BTreeSet};

use crate::{
    config::SignalsConfig,
    ids::AppId,
    signals::Signals,
    votes::{VoteKind, Votes},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HoursSource {
    Played,
    Followed,
    Wishlist,
    WouldTry,
    Elsewhere,
}

pub fn effective_hours(
    signals: &Signals,
    votes: &Votes,
    played_elsewhere: &BTreeSet<AppId>,
    signals_config: &SignalsConfig,
) -> BTreeMap<AppId, f64> {
    hours_by_source(signals, votes, played_elsewhere, signals_config)
        .into_iter()
        .map(|(id, (hours, _))| (id, hours))
        .collect()
}

pub fn hours_by_source(
    signals: &Signals,
    votes: &Votes,
    played_elsewhere: &BTreeSet<AppId>,
    signals_config: &SignalsConfig,
) -> BTreeMap<AppId, (f64, HoursSource)> {
    let mut hours: BTreeMap<AppId, (f64, HoursSource)> = signals
        .library
        .iter()
        .map(|g| (g.id, (g.total_hours, HoursSource::Played)))
        .collect();

    for g in &signals.borrowed {
        hours
            .entry(g.id)
            .or_insert((g.total_hours, HoursSource::Played));
    }

    fn keep_max_hours(
        hours: &mut BTreeMap<AppId, (f64, HoursSource)>,
        game_id: AppId,
        new_hours: f64,
        source: HoursSource,
    ) {
        hours
            .entry(game_id)
            .and_modify(|h| {
                if new_hours > h.0 {
                    *h = (new_hours, source);
                }
            })
            .or_insert((new_hours, source));
    }

    for g in &signals.recent {
        hours.insert(
            g.id,
            (
                g.total_hours - g.recent_hours
                    + g.recent_hours * signals_config.recent_hours_multiplier,
                HoursSource::Played,
            ),
        );
    }

    for &id in &signals.followed {
        keep_max_hours(
            &mut hours,
            id,
            signals_config.followed_hours,
            HoursSource::Followed,
        );
    }

    for &id in &signals.wishlist {
        keep_max_hours(
            &mut hours,
            id,
            signals_config.wishlist_hours,
            HoursSource::Wishlist,
        );
    }

    for (&id, _) in votes
        .iter()
        .filter(|(_, vote)| vote.kind == VoteKind::WouldTry)
    {
        keep_max_hours(
            &mut hours,
            id,
            signals_config.would_try_hours,
            HoursSource::WouldTry,
        );
    }

    for &id in played_elsewhere {
        keep_max_hours(
            &mut hours,
            id,
            signals_config.elsewhere_hours,
            HoursSource::Elsewhere,
        );
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
                    last_played: None,
                },
                LibraryGame {
                    id: AppId(2),
                    total_hours: 1.0,
                    last_played: None,
                },
                LibraryGame {
                    id: AppId(3),
                    total_hours: 50.0,
                    last_played: None,
                },
            ],
            borrowed: vec![],
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
            at: None,
        }
    }

    #[test]
    fn effective_hours_count_recent_hours_three_times() {
        let hours = effective_hours(
            &signals(),
            &Votes::new(),
            &BTreeSet::new(),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(1)], 110.0);
    }

    #[test]
    fn effective_hours_keep_the_highest_value_of_every_source() {
        let hours = effective_hours(
            &signals(),
            &Votes::new(),
            &BTreeSet::new(),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(2)], 3.0);
        assert_eq!(hours[&AppId(3)], 50.0);
        assert_eq!(hours[&AppId(4)], 5.0);
    }

    #[test]
    fn effective_hours_add_only_would_try_votes() {
        let votes = Votes::from([
            (AppId(10), vote(VoteKind::WouldTry)),
            (AppId(20), vote(VoteKind::Later)),
        ]);
        let hours = effective_hours(
            &signals(),
            &votes,
            &BTreeSet::new(),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(10)], 5.0);
        assert!(!hours.contains_key(&AppId(20)));
    }

    #[test]
    fn every_game_keeps_the_source_of_its_hours() {
        let votes = Votes::from([(AppId(2), vote(VoteKind::WouldTry))]);
        let hours = hours_by_source(
            &signals(),
            &votes,
            &BTreeSet::new(),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(1)], (110.0, HoursSource::Played));
        assert_eq!(hours[&AppId(2)], (5.0, HoursSource::WouldTry));
        assert_eq!(hours[&AppId(3)], (50.0, HoursSource::Played));
        assert_eq!(hours[&AppId(4)], (5.0, HoursSource::Wishlist));
    }

    #[test]
    fn a_tie_keeps_the_first_source() {
        let signals = Signals {
            followed: vec![AppId(4)],
            ..signals()
        };
        let config = SignalsConfig {
            followed_hours: 5.0,
            ..SignalsConfig::default()
        };
        let hours = hours_by_source(&signals, &Votes::new(), &BTreeSet::new(), &config);
        assert_eq!(hours[&AppId(4)], (5.0, HoursSource::Followed));
    }

    #[test]
    fn a_borrowed_game_counts_its_hours_unless_owned_or_recent() {
        let signals = Signals {
            borrowed: vec![
                LibraryGame {
                    id: AppId(1),
                    total_hours: 7.0,
                    last_played: None,
                },
                LibraryGame {
                    id: AppId(3),
                    total_hours: 80.0,
                    last_played: None,
                },
                LibraryGame {
                    id: AppId(8),
                    total_hours: 40.0,
                    last_played: None,
                },
            ],
            library: vec![LibraryGame {
                id: AppId(3),
                total_hours: 50.0,
                last_played: None,
            }],
            ..signals()
        };
        let hours = hours_by_source(
            &signals,
            &Votes::new(),
            &BTreeSet::new(),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(8)], (40.0, HoursSource::Played));
        assert_eq!(hours[&AppId(3)], (50.0, HoursSource::Played));
        assert_eq!(hours[&AppId(1)], (110.0, HoursSource::Played));
    }

    fn elsewhere(ids: &[u32]) -> BTreeSet<AppId> {
        ids.iter().map(|&id| AppId(id)).collect()
    }

    #[test]
    fn a_game_played_elsewhere_counts_the_fixed_hours() {
        let hours = hours_by_source(
            &signals(),
            &Votes::new(),
            &elsewhere(&[9]),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(9)], (20.0, HoursSource::Elsewhere));
    }

    #[test]
    fn more_real_hours_win_over_played_elsewhere() {
        let hours = hours_by_source(
            &signals(),
            &Votes::new(),
            &elsewhere(&[1]),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(1)], (110.0, HoursSource::Played));
    }

    #[test]
    fn an_owned_game_with_no_hours_played_elsewhere_counts_as_elsewhere() {
        let signals = Signals {
            library: vec![LibraryGame {
                id: AppId(7),
                total_hours: 0.0,
                last_played: None,
            }],
            ..signals()
        };
        let hours = hours_by_source(
            &signals,
            &Votes::new(),
            &elsewhere(&[7]),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(7)], (20.0, HoursSource::Elsewhere));
    }

    #[test]
    fn the_highest_of_wishlist_and_played_elsewhere_wins() {
        let config = SignalsConfig {
            elsewhere_hours: 3.0,
            ..SignalsConfig::default()
        };
        let hours = hours_by_source(&signals(), &Votes::new(), &elsewhere(&[4]), &config);
        assert_eq!(hours[&AppId(4)], (5.0, HoursSource::Wishlist));
        let hours = hours_by_source(
            &signals(),
            &Votes::new(),
            &elsewhere(&[4]),
            &SignalsConfig::default(),
        );
        assert_eq!(hours[&AppId(4)], (20.0, HoursSource::Elsewhere));
    }
}
