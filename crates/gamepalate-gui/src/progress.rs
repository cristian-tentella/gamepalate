use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use gamepalate_app::{
    SteamError,
    refresh::RefreshError,
    setup::error_chain,
    update::{Progress, UpdateError},
};

use crate::format;

const RECENT_BATCHES: usize = 20;
const MIN_BATCHES: usize = 10;
const READING_SHARE: f32 = 0.02;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum Stage {
    #[default]
    Starting,
    Reading,
    StoreData {
        done: usize,
        total: usize,
    },
    Writing,
}

#[derive(Debug, Default)]
pub struct Tracker {
    stage: Stage,
    batches: VecDeque<Instant>,
    archived: usize,
}

impl Tracker {
    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn advance(&mut self, progress: Progress, now: Instant) {
        match progress {
            Progress::Resumed { responses, .. } => self.archived = responses,
            Progress::ReadingApps => self.stage = Stage::Reading,
            Progress::StoreData { done, total } => {
                self.stage = Stage::StoreData { done, total };
                if done > self.archived {
                    self.batches.push_back(now);
                    if self.batches.len() > RECENT_BATCHES {
                        self.batches.pop_front();
                    }
                }
            }
            Progress::Writing => self.stage = Stage::Writing,
        }
    }

    pub fn fraction(&self) -> f32 {
        match self.stage {
            Stage::Starting | Stage::Reading => 0.0,
            Stage::StoreData { done, total } => {
                READING_SHARE + (1.0 - READING_SHARE) * done as f32 / total.max(1) as f32
            }
            Stage::Writing => 1.0,
        }
    }

    pub fn percent(&self) -> Option<String> {
        match self.stage {
            Stage::Starting | Stage::Reading => None,
            Stage::StoreData { .. } | Stage::Writing => {
                Some(format!("{}%", (self.fraction() * 100.0).round()))
            }
        }
    }

    pub fn resuming(&self) -> bool {
        self.archived > 0
            && match self.stage {
                Stage::Starting | Stage::Reading => true,
                Stage::StoreData { done, .. } => done <= self.archived,
                Stage::Writing => false,
            }
    }

    pub fn time_left(&self) -> Option<Duration> {
        let Stage::StoreData { done, total } = self.stage else {
            return None;
        };
        let first = *self.batches.front()?;
        let batches: Vec<Duration> = self
            .batches
            .iter()
            .map(|at| at.duration_since(first))
            .collect();
        time_left(&batches, done, total)
    }
}

fn time_left(batches: &[Duration], done: usize, total: usize) -> Option<Duration> {
    if done < MIN_BATCHES || batches.len() < 2 {
        return None;
    }
    let span = *batches.last()? - batches[0];
    let per_batch = span / (batches.len() as u32 - 1);

    Some(per_batch * total.saturating_sub(done) as u32)
}

pub fn update_error_text(error: &UpdateError) -> String {
    match error {
        UpdateError::Stopped {
            resume_before,
            source,
        } => {
            let why = match source {
                SteamError::SteamDown { .. } => {
                    "Steam is not responding, so the update stopped".to_string()
                }
                source => format!("Update stopped: {}", error_chain(source)),
            };
            format!(
                "{why}. Update again before {} to resume.",
                format::day_time(*resume_before)
            )
        }
        UpdateError::StartOver { .. } => {
            "Steam sent an unexpected answer. The next update starts over.".to_string()
        }
        error => error_chain(error),
    }
}

pub fn refresh_error_text(error: &RefreshError) -> String {
    match error {
        RefreshError::Update(error) => update_error_text(error),
        RefreshError::Open(error) => format!(
            "gamepalate could not open the updated data: {}",
            error_chain(error)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(values: &[u64]) -> Vec<Duration> {
        values
            .iter()
            .map(|value| Duration::from_secs(*value))
            .collect()
    }

    #[test]
    fn there_is_no_estimate_before_ten_batches() {
        assert_eq!(time_left(&seconds(&[0, 1, 2]), 9, 100), None);
        assert_eq!(time_left(&seconds(&[0]), 10, 100), None);
    }

    #[test]
    fn a_steady_pace_gives_the_time_left() {
        let batches = seconds(&[0, 2, 4, 6, 8, 10, 12, 14, 16, 18]);

        assert_eq!(time_left(&batches, 10, 100), Some(Duration::from_secs(180)));
    }

    #[test]
    fn after_a_fast_start_the_estimate_follows_the_last_batches() {
        let mut tracker = Tracker::default();
        let start = Instant::now();
        for done in 1..=30 {
            let at = if done <= 10 {
                start
            } else {
                start + Duration::from_secs(3 * (done - 10))
            };
            tracker.advance(
                Progress::StoreData {
                    done: done as usize,
                    total: 130,
                },
                at,
            );
        }

        assert_eq!(tracker.batches.len(), RECENT_BATCHES);
        assert_eq!(tracker.time_left(), Some(Duration::from_secs(300)));
    }

    #[test]
    fn the_archived_batches_do_not_count_for_the_time_left() {
        let mut tracker = Tracker::default();
        let start = Instant::now();
        tracker.advance(
            Progress::Resumed {
                started_at: gamepalate_app::Timestamp(0),
                responses: 30,
            },
            start,
        );
        assert!(tracker.resuming());
        for done in 1..=45_usize {
            let at = start + Duration::from_secs(2 * done.saturating_sub(30) as u64);
            tracker.advance(Progress::StoreData { done, total: 100 }, at);
            assert_eq!(tracker.resuming(), done <= 30);
        }

        assert_eq!(tracker.time_left(), Some(Duration::from_secs(110)));
    }

    #[test]
    fn the_bar_gives_the_list_of_games_its_share() {
        let mut tracker = Tracker::default();
        let now = Instant::now();
        assert_eq!(tracker.percent(), None);

        tracker.advance(Progress::ReadingApps, now);
        assert_eq!(tracker.fraction(), 0.0);
        assert_eq!(tracker.percent(), None);

        tracker.advance(
            Progress::StoreData {
                done: 0,
                total: 757,
            },
            now,
        );
        assert_eq!(tracker.percent().as_deref(), Some("2%"));

        tracker.advance(
            Progress::StoreData {
                done: 312,
                total: 757,
            },
            now,
        );
        assert_eq!(tracker.percent().as_deref(), Some("42%"));

        tracker.advance(Progress::Writing, now);
        assert_eq!(tracker.fraction(), 1.0);
        assert_eq!(tracker.percent().as_deref(), Some("100%"));
        assert!(!tracker.resuming());
    }

    #[test]
    fn a_stopped_update_says_until_when_it_resumes() {
        let resume_before = gamepalate_app::Timestamp(1_793_369_460_000);
        let stopped = UpdateError::Stopped {
            resume_before,
            source: SteamError::SteamDown {
                endpoint: "IStoreBrowseService/GetItems/v1".to_string(),
                reason: "timed out".to_string(),
                source: None,
            },
        };

        assert_eq!(
            update_error_text(&stopped),
            format!(
                "Steam is not responding, so the update stopped. Update again before {} to resume.",
                format::day_time(resume_before)
            )
        );
    }

    #[test]
    fn an_update_that_starts_over_says_so() {
        let start_over = UpdateError::StartOver {
            source: SteamError::Shape {
                endpoint: "IStoreBrowseService/GetItems/v1".to_string(),
                reason: "unexpected".to_string(),
            },
            not_marked: None,
        };

        assert_eq!(
            update_error_text(&start_over),
            "Steam sent an unexpected answer. The next update starts over."
        );
    }
}
