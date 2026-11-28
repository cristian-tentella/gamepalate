use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{
    Gamepalate, PendingUpdate, Region, Timestamp,
    refresh::{Plan, Step, refresh, startup_plan},
    update::{ArchiveState, archive_state, wall_clock},
};
use iced::{
    Center, Color, Element, Radians, Task,
    advanced::graphics::text::{cosmic_text, font_system, to_attributes},
    futures::{
        SinkExt,
        channel::mpsc,
        future::{AbortHandle, Abortable, Aborted},
    },
    stream, task,
    widget::{button, column, container, image, progress_bar, row, space, text, tooltip},
};

use super::{
    App, BAR_ICON_SCALE, Message, Start, badged, bar_button, bar_icon, frame::CLOSE, icon_text,
    with_dot, with_tooltip,
};
use crate::{
    font, format,
    progress::{self, Stage, Tracker},
    screen::Screen,
    style,
};

const REFRESH: char = '\u{E5D5}';
const NEVER_UPDATED: &str =
    "Your catalog doesn't update on its own. Download it from Steam, 15-25 minutes.";
const CANCEL: char = '\u{E5CD}';
const SECOND_PART: &str = "Downloading the rest of the catalog";
const UNTIL_THEN: &str = "Until then Browse and search show the popular games only";
const CLICK_TO_PAUSE: &str = "Click to pause";
pub(super) const SPIN_TICK: Duration = Duration::from_millis(50);
const SPIN_TURN: Duration = Duration::from_millis(1500);

/// ⟳ drawn once as a picture, since only pictures turn in iced.
#[derive(Debug)]
pub(super) struct Spinner {
    color: Color,
    scale: f32,
    handle: image::Handle,
    side: f32,
}

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

/// ⟳ opens the page again on the new catalog, except at the end of the first download: Discover
/// keeps the game shown and its queue.
pub(super) fn reopens_the_screen(start: Start, provisional: bool) -> bool {
    start == Start::Button && !provisional
}

/// A catalog from someone else never updates by itself. The one of the first part has no
/// completed update either, but its rest downloads at every start.
fn never_updated(state: &ArchiveState) -> bool {
    state.updated_at.is_none() && !state.provisional
}

fn complete_text(games: usize) -> String {
    format!("Catalog complete, {} games", format::count(games as u32))
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

/// The tip of ⟳ while the rest of the first download comes in: the first line, then the muted
/// ones.
/// The update that downloads the rest of the first download: one is running and the catalog is
/// still the provisional one. Any other update keeps the usual ⟳.
fn second_part_refreshing(provisional: bool, refresh: &Refresh) -> Option<&Refreshing> {
    match refresh {
        Refresh::Running(refreshing, ..) if provisional => Some(refreshing),
        _ => None,
    }
}

fn second_part_lines(refreshing: &Refreshing) -> (&'static str, Vec<String>) {
    let mut muted: Vec<String> = refreshing_count(refreshing).into_iter().collect();
    muted.push(UNTIL_THEN.to_string());
    if refreshing.can_cancel() {
        muted.push(CLICK_TO_PAUSE.to_string());
    }
    (SECOND_PART, muted)
}

fn spin_step(angle: f32) -> f32 {
    let step = std::f32::consts::TAU * SPIN_TICK.as_secs_f32() / SPIN_TURN.as_secs_f32();
    (angle + step) % std::f32::consts::TAU
}

/// The icon as a square picture, its ink centred so that it turns in place. `size` is in pixels
/// of the screen.
fn icon_picture(icon: char, size: f32, color: Color) -> Option<(image::Handle, u32)> {
    let mut system = font_system().write().ok()?;
    let system = system.raw();
    let mut buffer = cosmic_text::Buffer::new(system, cosmic_text::Metrics::new(size, size));
    buffer.set_text(
        system,
        &icon.to_string(),
        &to_attributes(font::ICONS),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(system, false);
    let key = buffer
        .layout_runs()
        .next()?
        .glyphs
        .first()?
        .physical((0.0, 0.0), 1.0)
        .cache_key;
    let glyph = cosmic_text::SwashCache::new().get_image_uncached(system, key)?;
    let cosmic_text::SwashContent::Mask = glyph.content else {
        return None;
    };
    let (width, height) = (glyph.placement.width, glyph.placement.height);
    if width == 0 || height == 0 {
        return None;
    }
    let side = width.max(height);
    let (left, top) = ((side - width) / 2, (side - height) / 2);
    let [red, green, blue, alpha] = color.into_rgba8();
    let mut pixels = vec![0; (side * side * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let ink = glyph.data[(y * width + x) as usize];
            let at = (((y + top) * side + x + left) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&[
                red,
                green,
                blue,
                (u16::from(ink) * u16::from(alpha) / 255) as u8,
            ]);
        }
    }

    Some((image::Handle::from_rgba(side, side, pixels), side))
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

/// The dot on the refresh button: an update to resume, one that stopped at startup, or a catalog
/// that came from someone else and never updates by itself. A renewal that is due needs none,
/// since it starts by itself at startup.
pub(super) fn refresh_dot(
    pending: Option<PendingUpdate>,
    startup_error: Option<&str>,
    never_updated: bool,
) -> bool {
    pending.is_some() || startup_error.is_some() || never_updated
}

pub(super) fn second_line(never_updated: bool, times: String) -> String {
    if never_updated {
        NEVER_UPDATED.to_string()
    } else {
        times
    }
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
                    self.never_updated = never_updated(&state);
                    let notice = self.set_provisional(state.provisional);
                    let run = match start {
                        Start::Startup => {
                            self.run_refresh(startup_plan(&state, wall_clock()), Start::Startup)
                        }
                        Start::Button => Task::none(),
                    };
                    Task::batch([notice, run])
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
                if !reopens_the_screen(start, self.provisional) {
                    let entered = self.enter(gamepalate);
                    return Task::batch([read, entered]);
                }
                let placed = self.new_gamepalate(&gamepalate);
                self.gamepalate = Some(gamepalate);
                self.parked = None;
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
            Message::Spin => {
                self.spin = spin_step(self.spin);
                let color = self.theme.extended_palette().background.base.text;
                let stale = self
                    .spinner
                    .as_ref()
                    .is_none_or(|spinner| spinner.color != color || spinner.scale != self.scale);
                if stale {
                    let size = font::icon(14.0) * BAR_ICON_SCALE * self.scale;
                    self.spinner =
                        icon_picture(REFRESH, size, color).map(|(handle, side)| Spinner {
                            color,
                            scale: self.scale,
                            handle,
                            side: side as f32 / self.scale,
                        });
                }
                Task::none()
            }
            Message::HideToast(id) => {
                self.toasts.hide(id);
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

    /// The rest of the first download is coming in: the bar shows it on ⟳ alone.
    pub(super) fn second_part(&self) -> bool {
        second_part_refreshing(self.provisional, &self.refresh).is_some()
    }

    /// Whether the catalog is the one of the first part. When the whole one takes its place, a
    /// notice says how many games it has.
    fn set_provisional(&mut self, provisional: bool) -> Task<Message> {
        let completed = self.provisional && !provisional;
        self.provisional = provisional;
        self.search.set_provisional(provisional);
        match &self.gamepalate {
            Some(gamepalate) if completed => self.toasts.show_with_icon(
                font::CHECK,
                complete_text(gamepalate.catalog_games()),
                Message::HideToast,
            ),
            _ => Task::none(),
        }
    }

    /// ⟳ turning, with the dot in the primary colour. A click pauses the download as on any
    /// update.
    pub(super) fn second_part_button(&self) -> Option<Element<'_, Message>> {
        let refreshing = second_part_refreshing(self.provisional, &self.refresh)?;
        let icon: Element<'_, Message> = match &self.spinner {
            Some(spinner) => image(spinner.handle.clone())
                .width(spinner.side)
                .height(spinner.side)
                .rotation(Radians(self.spin))
                .into(),
            None => icon_text(REFRESH),
        };
        let message = refreshing.can_cancel().then_some(Message::CancelRefresh);
        let (first, muted) = second_part_lines(refreshing);
        let tip = column![text(first).size(font::NOTE_SIZE)]
            .extend(
                muted
                    .into_iter()
                    .map(|line| text(line).size(font::NOTE_SIZE).style(style::muted).into()),
            )
            .spacing(2);

        Some(
            tooltip(
                badged(bar_button(icon, message), style::primary_badge),
                container(tip).padding([6, 10]).style(style::popup),
                tooltip::Position::Bottom,
            )
            .into(),
        )
    }

    pub(super) fn status(&self) -> Element<'_, Message> {
        if self.second_part() {
            return space().into();
        }
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
        if let Some(parked) = &mut self.parked {
            parked.use_gamepalate(self.gamepalate.as_deref(), &gamepalate);
        }
        let placed = self.new_gamepalate(&gamepalate);
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
    fn only_an_update_of_the_provisional_catalog_is_the_second_part() {
        let (_, handle) = Task::<Message>::none().abortable();
        let running = Refresh::Running(Refreshing::Catalog(Tracker::default()), handle);

        assert!(second_part_refreshing(true, &running).is_some());
        assert!(second_part_refreshing(false, &running).is_none());
        assert!(second_part_refreshing(true, &Refresh::Idle).is_none());
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
            Step::Renewal(gamepalate_app::update::Progress::ReadingApps {
                read: 0,
                first_part: false,
            }),
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
        assert!(reopens_the_screen(Start::Button, false));
        assert!(!reopens_the_screen(Start::Startup, false));
    }

    #[test]
    fn the_end_of_the_first_download_keeps_the_game_shown_in_discover() {
        assert!(!reopens_the_screen(Start::Button, true));
        assert!(!reopens_the_screen(Start::Startup, true));
    }

    #[test]
    fn the_tip_of_the_second_part_says_the_count_after_the_first_request() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        assert_eq!(
            second_part_lines(&refreshing),
            (
                "Downloading the rest of the catalog",
                vec![
                    "Until then Browse and search show the popular games only".to_string(),
                    "Click to pause".to_string(),
                ]
            )
        );

        refreshing.advance(store_data(312, 735), Instant::now());
        assert_eq!(
            second_part_lines(&refreshing),
            (
                "Downloading the rest of the catalog",
                vec![
                    "312 of 735".to_string(),
                    "Until then Browse and search show the popular games only".to_string(),
                    "Click to pause".to_string(),
                ]
            )
        );
    }

    #[test]
    fn the_tip_of_the_second_part_offers_to_pause_only_while_it_can() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        refreshing.advance(
            Step::Catalog(gamepalate_app::update::Progress::Writing),
            Instant::now(),
        );

        let (_, muted) = second_part_lines(&refreshing);
        assert!(!muted.contains(&"Click to pause".to_string()));
    }

    #[test]
    fn the_spin_goes_round_in_a_turn_and_a_half_a_second() {
        let ticks = (SPIN_TURN.as_millis() / SPIN_TICK.as_millis()) as usize;
        let mut angle = 0.0;
        for _ in 0..ticks / 2 {
            angle = spin_step(angle);
        }
        assert!((angle - std::f32::consts::PI).abs() < 1e-3);
        for _ in 0..ticks / 2 {
            angle = spin_step(angle);
        }
        assert!(angle < 1e-3 || (std::f32::consts::TAU - angle) < 1e-3);
    }

    #[test]
    fn the_refresh_icon_becomes_a_square_picture_with_ink_in_it() {
        font_system()
            .write()
            .unwrap()
            .load_font(std::borrow::Cow::Borrowed(font::ICONS_BYTES));

        let (_, side) = icon_picture(REFRESH, 40.0, Color::BLACK).unwrap();

        assert!((20..=40).contains(&side), "{side}");
    }

    #[test]
    fn the_catalog_of_the_first_part_does_not_say_it_never_updates() {
        let state = |provisional| ArchiveState {
            pending: true,
            updated_at: None,
            renewal_due: false,
            provisional,
        };

        assert!(never_updated(&state(false)));
        assert!(!never_updated(&state(true)));
    }

    #[test]
    fn the_notice_says_how_many_games_the_whole_catalog_has() {
        assert_eq!(complete_text(120_300), "Catalog complete, 120,300 games");
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

        assert!(!refresh_dot(None, None, false));
        assert!(refresh_dot(Some(pending), None, false));
        assert!(refresh_dot(None, Some("Steam is not responding"), false));
    }

    #[test]
    fn a_catalog_without_a_completed_update_has_the_dot_and_says_why() {
        let times = "Signals today 10:32 · Prices 13 Nov".to_string();

        assert!(refresh_dot(None, None, true));
        assert_eq!(
            two_lines(
                &refresh_tooltip(None, false, None),
                &second_line(true, times.clone())
            ),
            "Update prices and Signals\nYour catalog doesn't update on its own. Download it from \
            Steam, 15-25 minutes."
        );

        assert!(!refresh_dot(None, None, false));
        assert_eq!(second_line(false, times.clone()), times);
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
