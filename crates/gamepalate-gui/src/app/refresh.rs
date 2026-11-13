use std::{path::Path, sync::Arc, time::Instant};

use gamepalate_app::{
    Gamepalate, PendingUpdate, Region, Timestamp,
    refresh::{Plan, Step, refresh, startup_plan},
    update::{archive_state, wall_clock},
};
use iced::{
    Center, Element, Task,
    futures::{
        SinkExt,
        channel::mpsc,
        future::{AbortHandle, Abortable, Aborted},
    },
    stream, task,
    widget::{button, container, progress_bar, row, space, text, tooltip},
};

use super::{App, BAR_ICON_SCALE, Message, Start, bar_icon, frame::CLOSE, with_dot, with_tooltip};
use crate::{
    font, format,
    progress::{self, Stage, Tracker},
    screen::Screen,
    style,
};

const REFRESH: char = '\u{E5D5}';
const CANCEL: char = '\u{E5CD}';

#[derive(Debug, Default)]
pub(super) enum Refresh {
    #[default]
    Idle,
    Running(Refreshing, task::Handle),
    Failed(String),
}

#[derive(Debug)]
pub(super) enum Refreshing {
    Catalog(Tracker),
    Signals,
    Renewal(Tracker),
    Opening,
}

async fn refresh_in_two(
    dir: &Path,
    plan: Plan,
    output: &mut mpsc::Sender<Message>,
) -> Result<Option<Arc<Gamepalate>>, String> {
    let mut events = output.clone();
    let mut progress = move |step| {
        let _ = events.try_send(Message::Refreshing(step));
    };
    let mut latest = None;
    let mut renewal = plan.renewal_after(None);

    let prices = plan.prices_only();
    if !prices.is_nothing() {
        let refreshed = refresh(dir, prices, &mut progress)
            .await
            .map_err(|error| progress::refresh_error_text(&error))?;
        renewal = plan.renewal_after(refreshed.prices.as_ref());
        latest = refreshed.gamepalate.map(Arc::new);
        if let Some(gamepalate) = latest.as_ref().filter(|_| !renewal.is_nothing()) {
            let _ = output.send(Message::Entered(Arc::clone(gamepalate))).await;
        }
    }

    if !renewal.is_nothing() {
        let refreshed = refresh(dir, renewal, &mut progress)
            .await
            .map_err(|error| progress::refresh_error_text(&error))?;
        if let Some(gamepalate) = refreshed.gamepalate {
            latest = Some(Arc::new(gamepalate));
        }
    }

    Ok(latest)
}

struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) fn reopens_the_screen(start: Start) -> bool {
    start == Start::Button
}

impl Refreshing {
    fn advance(&mut self, step: Step, now: Instant) {
        match step {
            Step::Catalog(progress) => {
                if !matches!(self, Refreshing::Catalog(_)) {
                    *self = Refreshing::Catalog(Tracker::default());
                }
                if let Refreshing::Catalog(tracker) = self {
                    tracker.advance(progress, now);
                }
            }
            Step::Signals => *self = Refreshing::Signals,
            Step::Renewal(progress) => {
                if !matches!(self, Refreshing::Renewal(_)) {
                    *self = Refreshing::Renewal(Tracker::default());
                }
                if let Refreshing::Renewal(tracker) = self {
                    tracker.advance(progress, now);
                }
            }
            Step::Opening => *self = Refreshing::Opening,
        }
    }

    fn can_cancel(&self) -> bool {
        matches!(
            self,
            Refreshing::Catalog(tracker) | Refreshing::Renewal(tracker)
                if tracker.stage() != Stage::Writing
        )
    }
}

fn refreshing_text(refreshing: &Refreshing) -> &'static str {
    match refreshing {
        Refreshing::Catalog(tracker) => match tracker.stage() {
            Stage::Starting | Stage::Reading => "Reading the list of Steam games…",
            Stage::StoreData { .. } => "Updating prices",
            Stage::Writing => "Writing the catalog…",
        },
        Refreshing::Signals => "Reading your Signals…",
        Refreshing::Renewal(tracker) => match tracker.stage() {
            Stage::Starting | Stage::Reading => "Renewing tags and reviews…",
            Stage::StoreData { .. } => "Renewing tags and reviews",
            Stage::Writing => "Writing the catalog…",
        },
        Refreshing::Opening => "Loading…",
    }
}

fn refreshing_count(refreshing: &Refreshing) -> Option<String> {
    let (Refreshing::Catalog(tracker) | Refreshing::Renewal(tracker)) = refreshing else {
        return None;
    };
    let Stage::StoreData { done, total } = tracker.stage() else {
        return None;
    };
    let count = format!(
        "{} of {}",
        format::count(done as u32),
        format::count(total as u32)
    );

    Some(match tracker.time_left() {
        Some(left) => format!("{count} · {}", format::time_left(left)),
        None => count,
    })
}

pub(super) fn refresh_tooltip(
    pending: Option<PendingUpdate>,
    renewal_due: bool,
    startup_error: Option<&str>,
) -> String {
    if let Some(error) = startup_error {
        return error.to_string();
    }
    match pending {
        Some(pending) => format!(
            "Resume the update started on {}",
            format::day_time(pending.started_at)
        ),
        None if renewal_due => "Update prices and Signals, then renew tags and reviews".to_string(),
        None => "Update prices and Signals".to_string(),
    }
}

fn refresh_failed<'a>(error: &str) -> Element<'a, Message> {
    row![
        tooltip(
            text("Update stopped")
                .size(font::NOTE_SIZE)
                .style(text::danger),
            container(text(error.to_string()).size(font::NOTE_SIZE))
                .padding([6, 10])
                .max_width(420)
                .style(style::popup),
            tooltip::Position::Bottom,
        ),
        button(
            text(CLOSE)
                .font(font::ICONS)
                .size(font::icon(9.0) * BAR_ICON_SCALE)
        )
        .padding(0)
        .style(style::link)
        .on_press(Message::DismissRefreshError),
    ]
    .spacing(8)
    .align_y(Center)
    .into()
}

pub(super) fn refresh_button<'a>(
    refresh: &Refresh,
    idle_tip: String,
    times: String,
    dot: bool,
) -> Element<'a, Message> {
    let (icon, message, tip) = match refresh {
        Refresh::Running(refreshing, ..) if refreshing.can_cancel() => (
            CANCEL,
            Some(Message::CancelRefresh),
            "Pause the update".to_string(),
        ),
        Refresh::Running(..) => (REFRESH, None, idle_tip),
        Refresh::Idle | Refresh::Failed(_) => (REFRESH, Some(Message::Refresh), idle_tip),
    };
    let dot = dot && !matches!(refresh, Refresh::Running(..));

    with_tooltip(
        with_dot(bar_icon(icon, message), dot),
        two_lines(&tip, &times),
    )
}

/// The dot on the refresh button: an update to resume, or one that stopped at startup. A
/// renewal that is due needs none, since it starts by itself at startup.
pub(super) fn refresh_dot(pending: Option<PendingUpdate>, startup_error: Option<&str>) -> bool {
    pending.is_some() || startup_error.is_some()
}

fn two_lines(first: &str, second: &str) -> String {
    format!("{first}\n{second}")
}

pub(super) fn data_times(gamepalate: &Gamepalate) -> String {
    let now = Timestamp(jiff::Timestamp::now().as_millisecond());
    times_text(
        format::when(gamepalate.signals_read_at(), now),
        format::when(gamepalate.prices_updated_at(), now),
        gamepalate.prices_region(),
    )
}

fn times_text(signals: String, prices: String, other_region: Option<Region>) -> String {
    format!("Signals {signals} · {}", prices_text(prices, other_region))
}

fn prices_text(when: String, other_region: Option<Region>) -> String {
    match other_region {
        Some(region) => format!("Prices {when} · {region}"),
        None => format!("Prices {when}"),
    }
}

impl App {
    pub(super) fn update_refresh(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => self.run_refresh(Plan::EVERYTHING, Start::Button),
            Message::ArchiveRead(start, result) => match result {
                Ok(state) => {
                    self.renewal_due = state.renewal_due;
                    match start {
                        Start::Startup => {
                            self.run_refresh(startup_plan(&state, wall_clock()), Start::Startup)
                        }
                        Start::Button => Task::none(),
                    }
                }
                Err(error) => {
                    if start == Start::Startup {
                        self.startup_error = Some(error);
                    }
                    Task::none()
                }
            },
            Message::Refreshing(step) => {
                if let Refresh::Running(refreshing, ..) = &mut self.refresh {
                    refreshing.advance(step, Instant::now());
                }
                Task::none()
            }
            Message::Entered(gamepalate) => {
                if let Refresh::Running(refreshing, ..) = &mut self.refresh {
                    *refreshing = Refreshing::Renewal(Tracker::default());
                }
                self.enter(gamepalate)
            }
            Message::Refreshed(start, result) => {
                self.refresh = Refresh::Idle;
                let read = self.read_archive(Start::Button);
                let gamepalate = match result {
                    Ok(Some(gamepalate)) => gamepalate,
                    Ok(None) => {
                        self.find_pending();
                        return read;
                    }
                    Err(error) => {
                        match start {
                            Start::Button => self.refresh = Refresh::Failed(error),
                            Start::Startup => self.startup_error = Some(error),
                        }
                        self.find_pending();
                        return read;
                    }
                };
                if !reopens_the_screen(start) {
                    let entered = self.enter(gamepalate);
                    return Task::batch([read, entered]);
                }
                let placed = self.game_page_uses(&gamepalate);
                self.gamepalate = Some(gamepalate);
                self.find_pending();
                let open = match self.page() {
                    Some(page) => self.open(page),
                    None => Task::none(),
                };
                Task::batch([open, read, placed])
            }
            Message::CancelRefresh => {
                if let Refresh::Running(refreshing, handle) = &self.refresh
                    && refreshing.can_cancel()
                {
                    handle.abort();
                    self.refresh = Refresh::Idle;
                    self.find_pending();
                    return self.read_archive(Start::Button);
                }
                Task::none()
            }
            Message::DismissRefreshError => {
                if matches!(self.refresh, Refresh::Failed(_)) {
                    self.refresh = Refresh::Idle;
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    pub(super) fn status(&self) -> Element<'_, Message> {
        let refreshing = match &self.refresh {
            Refresh::Idle => return space().into(),
            Refresh::Failed(error) => return refresh_failed(error),
            Refresh::Running(refreshing, ..) => refreshing,
        };
        let bar = match refreshing {
            Refreshing::Catalog(tracker) | Refreshing::Renewal(tracker)
                if matches!(tracker.stage(), Stage::StoreData { .. } | Stage::Writing) =>
            {
                Some(
                    progress_bar(0.0..=1.0, tracker.fraction())
                        .girth(4)
                        .length(160)
                        .style(style::meter),
                )
            }
            _ => None,
        };

        row![text(refreshing_text(refreshing)).size(font::NOTE_SIZE)]
            .push(bar)
            .push(
                refreshing_count(refreshing)
                    .map(|count| text(count).size(font::NOTE_SIZE).style(style::muted)),
            )
            .spacing(10)
            .align_y(Center)
            .into()
    }

    pub(super) fn run_refresh(&mut self, plan: Plan, start: Start) -> Task<Message> {
        let Some(gamepalate) = &self.gamepalate else {
            return Task::none();
        };
        if plan.is_nothing() || matches!(self.refresh, Refresh::Running(..)) {
            return Task::none();
        }
        let dir = gamepalate.data_dir().to_path_buf();

        let (task, handle) = Task::run(
            stream::channel(64, async move |mut output: mpsc::Sender<Message>| {
                let (abort, registration) = AbortHandle::new_pair();
                let _abort_on_drop = AbortOnDrop(abort);
                let mut events = output.clone();
                let runtime = tokio::runtime::Handle::current();
                let blocking = tokio::task::spawn_blocking(move || {
                    runtime.block_on(Abortable::new(
                        refresh_in_two(&dir, plan, &mut events),
                        registration,
                    ))
                });
                let result = match blocking.await {
                    Ok(Ok(result)) => result,
                    Ok(Err(Aborted)) => return,
                    Err(error) => Err(format!("The update stopped unexpectedly: {error}")),
                };
                let _ = output.send(Message::Refreshed(start, result)).await;
            }),
            |message| message,
        )
        .abortable();
        let refreshing = if plan.prices {
            Refreshing::Catalog(Tracker::default())
        } else {
            Refreshing::Renewal(Tracker::default())
        };
        self.startup_error = None;
        self.refresh = Refresh::Running(refreshing, handle);

        task
    }

    pub(super) fn read_archive(&self, start: Start) -> Task<Message> {
        let Some(gamepalate) = &self.gamepalate else {
            return Task::none();
        };
        let dir = gamepalate.data_dir().to_path_buf();

        Task::perform(
            async move {
                archive_state(&dir, wall_clock())
                    .map_err(|error| progress::update_error_text(&error))
            },
            move |result| Message::ArchiveRead(start, result),
        )
    }

    fn enter(&mut self, gamepalate: Arc<Gamepalate>) -> Task<Message> {
        if let Screen::Discover(discover) = &mut self.screen {
            discover.use_gamepalate(&gamepalate);
        }
        let placed = self.game_page_uses(&gamepalate);
        self.gamepalate = Some(gamepalate);
        self.find_pending();
        placed
    }

    pub(super) fn find_pending(&mut self) {
        self.pending = self
            .gamepalate
            .as_ref()
            .and_then(|gamepalate| gamepalate.pending_update());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn store_data(done: usize, total: usize) -> Step {
        Step::Catalog(gamepalate_app::update::Progress::StoreData { done, total })
    }

    #[test]
    fn the_bar_says_each_step_of_the_update() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let now = Instant::now();
        assert_eq!(
            refreshing_text(&refreshing),
            "Reading the list of Steam games…"
        );
        assert_eq!(refreshing_count(&refreshing), None);

        refreshing.advance(store_data(312, 757), now);
        assert_eq!(refreshing_text(&refreshing), "Updating prices");
        assert_eq!(refreshing_count(&refreshing).as_deref(), Some("312 of 757"));

        refreshing.advance(Step::Signals, now);
        assert_eq!(refreshing_text(&refreshing), "Reading your Signals…");
        assert_eq!(refreshing_count(&refreshing), None);

        refreshing.advance(store_data(1, 2), now);
        refreshing.advance(Step::Opening, now);
        assert_eq!(refreshing_text(&refreshing), "Loading…");
    }

    #[test]
    fn after_ten_requests_the_bar_says_the_time_left() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let start = Instant::now();
        for done in 1..=10 {
            refreshing.advance(
                store_data(done, 1_210),
                start + Duration::from_secs(done as u64),
            );
        }

        assert_eq!(
            refreshing_count(&refreshing).as_deref(),
            Some("10 of 1,210 · About 20 minutes left")
        );
    }

    #[test]
    fn the_update_can_be_stopped_until_it_writes() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let now = Instant::now();
        assert!(refreshing.can_cancel());

        refreshing.advance(store_data(312, 757), now);
        assert!(refreshing.can_cancel());

        refreshing.advance(
            Step::Catalog(gamepalate_app::update::Progress::Writing),
            now,
        );
        assert!(!refreshing.can_cancel());

        refreshing.advance(Step::Signals, now);
        assert!(!refreshing.can_cancel());
    }

    fn renewal_data(done: usize, total: usize) -> Step {
        Step::Renewal(gamepalate_app::update::Progress::StoreData { done, total })
    }

    #[test]
    fn the_tooltip_offers_to_resume_a_pending_update() {
        let started_at = Timestamp(1_793_369_460_000);

        assert_eq!(
            refresh_tooltip(None, false, None),
            "Update prices and Signals"
        );
        assert_eq!(
            refresh_tooltip(
                Some(PendingUpdate {
                    started_at,
                    responses: 3,
                }),
                true,
                None
            ),
            format!(
                "Resume the update started on {}",
                format::day_time(started_at)
            )
        );
    }

    #[test]
    fn the_tooltip_says_when_tags_and_reviews_are_renewed_after_the_prices() {
        assert_eq!(
            refresh_tooltip(None, true, None),
            "Update prices and Signals, then renew tags and reviews"
        );
    }

    #[test]
    fn the_tooltip_says_what_stopped_the_update_at_startup() {
        let error =
            "Steam is not responding, so the update stopped. Update again before 7 Nov to resume.";

        assert_eq!(refresh_tooltip(None, true, Some(error)), error);
    }

    #[test]
    fn the_renewal_has_its_own_words_in_the_bar() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let now = Instant::now();
        refreshing.advance(store_data(2, 2), now);
        refreshing.advance(Step::Signals, now);

        refreshing.advance(
            Step::Renewal(gamepalate_app::update::Progress::ReadingApps),
            now,
        );
        assert_eq!(refreshing_text(&refreshing), "Renewing tags and reviews…");
        assert!(refreshing.can_cancel());

        refreshing.advance(renewal_data(40, 150), now);
        assert_eq!(refreshing_text(&refreshing), "Renewing tags and reviews");
        assert_eq!(refreshing_count(&refreshing).as_deref(), Some("40 of 150"));

        refreshing.advance(
            Step::Renewal(gamepalate_app::update::Progress::Writing),
            now,
        );
        assert_eq!(refreshing_text(&refreshing), "Writing the catalog…");
        assert!(!refreshing.can_cancel());
    }

    #[test]
    fn the_renewal_starts_its_count_from_zero() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let now = Instant::now();
        refreshing.advance(store_data(757, 757), now);

        refreshing.advance(renewal_data(3, 150), now);

        assert!(matches!(refreshing, Refreshing::Renewal(_)));
        assert_eq!(refreshing_count(&refreshing).as_deref(), Some("3 of 150"));
    }

    #[test]
    fn only_the_button_opens_the_screen_again() {
        assert!(reopens_the_screen(Start::Button));
        assert!(!reopens_the_screen(Start::Startup));
    }

    #[test]
    fn the_tooltip_says_the_time_of_the_data_on_a_second_line() {
        let times = times_text("today 10:32".to_string(), "today 10:30".to_string(), None);

        assert_eq!(
            two_lines(&refresh_tooltip(None, false, None), &times),
            "Update prices and Signals\nSignals today 10:32 · Prices today 10:30"
        );
        assert_eq!(
            two_lines("Pause the update", &times),
            "Pause the update\nSignals today 10:32 · Prices today 10:30"
        );
        assert_eq!(
            times_text(
                "today 10:32".to_string(),
                "28 Oct".to_string(),
                Some(Region::ITALY)
            ),
            "Signals today 10:32 · Prices 28 Oct · IT"
        );
    }

    #[test]
    fn the_dot_shows_an_update_to_resume_or_an_error_at_startup() {
        let pending = PendingUpdate {
            started_at: Timestamp(1_793_369_460_000),
            responses: 3,
        };

        assert!(!refresh_dot(None, None));
        assert!(refresh_dot(Some(pending), None));
        assert!(refresh_dot(None, Some("Steam is not responding")));
    }

    #[test]
    fn prices_of_another_region_say_their_region() {
        assert_eq!(prices_text("28 Oct".to_string(), None), "Prices 28 Oct");
        assert_eq!(
            prices_text("28 Oct".to_string(), Some(Region::ITALY)),
            "Prices 28 Oct · IT"
        );
    }
}
