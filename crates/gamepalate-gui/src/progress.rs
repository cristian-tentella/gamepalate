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
}

impl Tracker {
    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn advance(&mut self, progress: Progress, now: Instant) {
        match progress {
            Progress::Resumed { .. } => {}
            Progress::ReadingApps => self.stage = Stage::Reading,
            Progress::StoreData { done, total } => {
                self.stage = Stage::StoreData { done, total };
                if done > 0 {
                    self.batches.push_back(now);
                    if self.batches.len() > RECENT_BATCHES {
                        self.batches.pop_front();
                    }
                }
            }
            Progress::Writing => self.stage = Stage::Writing,
        }
    }

    pub fn done_of(&self) -> (usize, usize) {
        match self.stage {
            Stage::StoreData { done, total } => (done, total),
            Stage::Writing => (1, 1),
            Stage::Starting | Stage::Reading => (0, 1),
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
                SteamError::SteamDown { .. } => "Steam is not responding".to_string(),
                source => error_chain(source),
            };
            format!(
                "Update stopped: {why}. Run it again before {} to resume.",
                format::day_time(*resume_before)
            )
        }
        UpdateError::StartOver { .. } => {
            "Steam answered in an unexpected way: the next update starts over".to_string()
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
                "Update stopped: Steam is not responding. Run it again before {} to resume.",
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
            "Steam answered in an unexpected way: the next update starts over"
        );
    }
}
