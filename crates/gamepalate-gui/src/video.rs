mod audio;
mod decode;
mod stream;
mod view;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use iced::Subscription;
use iced::widget::image;

use self::audio::{AudioBuffer, Clock};

pub use self::view::{Surface, probe, spinner};

const FRAME_CAPACITY: usize = 30;
const FRAME_TOLERANCE: f64 = 0.0005;
const IDLE: Duration = Duration::from_millis(5);
const TICK: Duration = Duration::from_millis(250);
const VIEWS_LOGGED_EVERY: Duration = Duration::from_secs(1);
pub const FULL_VOLUME: f32 = 1.0;
/// The volume until the Utente picks one, and after unmuting a trailer turned down to zero.
pub const START_VOLUME: f32 = 0.25;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Dash { manifest_url: String, height: u32 },
}

/// How the trailer starts: `volume` goes from 0 to 1, and 0 counts as muted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub muted: bool,
    pub volume: f32,
    pub rgba: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            muted: false,
            volume: FULL_VOLUME,
            rgba: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Loading,
    Playing,
    Paused,
    Ended,
    Failed(String),
}

pub struct Frame {
    pub epoch: u64,
    pub pts: f64,
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub rgba: Option<image::Handle>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Toggle,
    Scrub(f64),
    Seek,
    Mute,
    Volume(f32),
    Replay,
    Retry,
    Hovered(bool),
    Tick,
}

pub struct Player {
    shared: Arc<Shared>,
    source: Source,
    options: Options,
    scrub: Option<f64>,
    hovered: bool,
    loading_since: Instant,
    views: AtomicU32,
    views_since: Instant,
}

impl Player {
    pub fn new(source: Source, options: Options) -> Player {
        Player {
            shared: start(&source, options),
            source,
            options,
            scrub: None,
            hovered: false,
            loading_since: Instant::now(),
            views: AtomicU32::new(0),
            views_since: Instant::now(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Toggle => match self.state() {
                State::Playing | State::Loading => self.pause(),
                State::Paused => self.play(),
                State::Ended | State::Failed(_) => {}
            },
            Message::Scrub(seconds) => self.scrub = Some(seconds),
            Message::Seek => {
                if let Some(seconds) = self.scrub.take() {
                    self.seek(Duration::from_secs_f64(seconds.max(0.0)));
                }
            }
            Message::Mute => self.switch_sound(),
            Message::Volume(volume) => {
                self.set_volume(volume);
                self.set_muted(self.volume() == 0.0);
            }
            Message::Replay => {
                self.seek(Duration::ZERO);
                self.play();
            }
            Message::Retry => self.retry(),
            Message::Hovered(hovered) => self.hovered = hovered,
            Message::Tick => self.log_views(Instant::now()),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        match self.state() {
            State::Loading | State::Playing => iced::time::every(TICK).map(|_| Message::Tick),
            State::Paused | State::Ended | State::Failed(_) => Subscription::none(),
        }
    }

    fn retry(&mut self) {
        self.shared.stop();
        self.options.muted = self.is_muted();
        self.options.volume = self.volume();
        self.shared = start(&self.source, self.options);
        self.scrub = None;
        self.loading_since = Instant::now();
    }

    /// The mute button: a trailer turned down to zero comes back at [`START_VOLUME`].
    fn switch_sound(&self) {
        if self.is_silent() {
            if self.volume() == 0.0 {
                self.set_volume(START_VOLUME);
            }
            self.set_muted(false);
        } else {
            self.set_muted(true);
        }
    }

    fn log_views(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.views_since);
        if elapsed >= VIEWS_LOGGED_EVERY {
            let views = self.views.swap(0, Ordering::Relaxed);
            tracing::debug!(
                "the trailer view was built {views} times in {:.2} s",
                elapsed.as_secs_f64()
            );
            self.views_since = now;
        }
    }

    fn shows_an_old_frame(&self) -> bool {
        let epoch = self.shared.epoch();
        lock(&self.shared.frames)
            .current
            .as_ref()
            .is_some_and(|frame| frame.epoch != epoch)
    }

    pub fn state(&self) -> State {
        self.shared.state()
    }

    pub fn play(&self) {
        self.shared.playing.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
    }

    pub fn seek(&self, position: Duration) {
        let end = self.duration().unwrap_or(Duration::MAX);
        self.shared.seek(position.min(end).as_secs_f64());
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::SeqCst);
    }

    pub fn is_muted(&self) -> bool {
        self.shared.is_muted()
    }

    pub fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, FULL_VOLUME);
        self.shared.volume.store(volume.to_bits(), Ordering::SeqCst);
    }

    pub fn volume(&self) -> f32 {
        self.shared.volume()
    }

    /// Muted, or turned down to zero.
    pub fn is_silent(&self) -> bool {
        self.is_muted() || self.volume() == 0.0
    }

    pub fn stop(&self) {
        self.shared.stop();
    }

    pub fn position(&self) -> Duration {
        Duration::from_secs_f64(self.shared.clock().at(Instant::now()).max(0.0))
    }

    pub fn duration(&self) -> Option<Duration> {
        self.shared.duration.get().copied()
    }

    pub fn frame(&self) -> Option<Arc<Frame>> {
        self.shared.select(Instant::now())
    }

    pub fn drawn(&self, frame: &Frame) {
        self.shared.drawn(frame.epoch);
    }

    pub fn next_frame_at(&self) -> Option<Instant> {
        self.shared.next_frame_at(Instant::now())
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start(source: &Source, options: Options) -> Arc<Shared> {
    let shared = Arc::new(Shared::new(options));
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => decode::start(Arc::clone(&shared), runtime, source.clone(), options.rgba),
        Err(error) => shared.fail(format!("no async runtime for the trailer: {error}")),
    }

    shared
}

#[derive(Default)]
struct Frames {
    queue: VecDeque<Arc<Frame>>,
    current: Option<Arc<Frame>>,
}

impl Frames {
    fn select(&mut self, epoch: u64, clock: f64) -> bool {
        let mut taken = false;
        while let Some(front) = self.queue.front() {
            let first = self
                .current
                .as_ref()
                .is_none_or(|current| current.epoch != epoch);
            if front.epoch != epoch {
                self.queue.pop_front();
            } else if first || front.pts <= clock + FRAME_TOLERANCE {
                self.current = self.queue.pop_front();
            } else {
                break;
            }
            taken = true;
        }

        taken
    }

    fn next_pts(&self, epoch: u64) -> Option<f64> {
        self.queue
            .iter()
            .find(|frame| frame.epoch == epoch)
            .map(|frame| frame.pts)
    }

    fn has(&self, epoch: u64) -> bool {
        self.queue.iter().any(|frame| frame.epoch == epoch)
    }
}

struct Shared {
    epoch: AtomicU64,
    seek_to: Mutex<f64>,
    playing: AtomicBool,
    muted: AtomicBool,
    volume: AtomicU32,
    stopped: AtomicBool,
    started: AtomicBool,
    drawn: AtomicU64,
    video_done: AtomicU64,
    audio_done: AtomicU64,
    duration: OnceLock<Duration>,
    failure: OnceLock<String>,
    frames: Mutex<Frames>,
    frames_changed: Condvar,
    audio: Mutex<AudioBuffer>,
    clock: Mutex<Clock>,
}

impl Shared {
    fn new(options: Options) -> Shared {
        Shared {
            epoch: AtomicU64::new(0),
            seek_to: Mutex::new(0.0),
            playing: AtomicBool::new(true),
            muted: AtomicBool::new(options.muted),
            volume: AtomicU32::new(options.volume.clamp(0.0, FULL_VOLUME).to_bits()),
            stopped: AtomicBool::new(false),
            started: AtomicBool::new(false),
            drawn: AtomicU64::new(0),
            video_done: AtomicU64::new(0),
            audio_done: AtomicU64::new(0),
            duration: OnceLock::new(),
            failure: OnceLock::new(),
            frames: Mutex::new(Frames::default()),
            frames_changed: Condvar::new(),
            audio: Mutex::new(AudioBuffer::new(0, 0.0)),
            clock: Mutex::new(Clock::stopped(0.0)),
        }
    }

    fn state(&self) -> State {
        if let Some(reason) = self.failure.get() {
            return State::Failed(reason.clone());
        }
        let epoch = self.epoch();
        let mark = epoch + 1;
        if self.video_done.load(Ordering::SeqCst) == mark
            && self.audio_done.load(Ordering::SeqCst) == mark
            && !lock(&self.frames).has(epoch)
        {
            return State::Ended;
        }
        if !self.started.load(Ordering::SeqCst) {
            return State::Loading;
        }
        match self.playing.load(Ordering::SeqCst) {
            true => State::Playing,
            false => State::Paused,
        }
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn is_current(&self, epoch: u64) -> bool {
        self.epoch() == epoch && !self.is_stopped()
    }

    fn is_over(&self) -> bool {
        self.is_stopped() || self.failure.get().is_some()
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    fn is_muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::SeqCst))
    }

    /// What the samples are multiplied by: the square of the volume, closer to how loud it sounds.
    fn gain(&self) -> f32 {
        match self.is_muted() {
            true => 0.0,
            false => self.volume().powi(2),
        }
    }

    fn is_audible(&self, epoch: u64) -> bool {
        self.playing.load(Ordering::SeqCst)
            && !self.is_stopped()
            && self.failure.get().is_none()
            && self.drawn.load(Ordering::SeqCst) == epoch + 1
    }

    fn seek_target(&self) -> f64 {
        *lock(&self.seek_to)
    }

    fn audio(&self) -> MutexGuard<'_, AudioBuffer> {
        lock(&self.audio)
    }

    fn clock(&self) -> MutexGuard<'_, Clock> {
        lock(&self.clock)
    }

    fn fail(&self, reason: String) {
        if self.failure.set(reason.clone()).is_ok() {
            tracing::warn!("the trailer failed: {reason}");
        }
        self.frames_changed.notify_all();
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.frames_changed.notify_all();
    }

    fn seek(&self, to: f64) {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *lock(&self.seek_to) = to;
        self.audio().restart(epoch, to);
        lock(&self.frames).queue.clear();
        self.frames_changed.notify_all();
        *self.clock() = Clock::stopped(to);
    }

    fn drawn(&self, epoch: u64) {
        if epoch == self.epoch() {
            self.drawn.store(epoch + 1, Ordering::SeqCst);
            self.started.store(true, Ordering::SeqCst);
        }
    }

    fn video_finished(&self, epoch: u64) {
        self.video_done.store(epoch + 1, Ordering::SeqCst);
    }

    fn audio_played_out(&self, epoch: u64) {
        self.audio_done.store(epoch + 1, Ordering::SeqCst);
    }

    fn select(&self, now: Instant) -> Option<Arc<Frame>> {
        let epoch = self.epoch();
        // The last frames can come after the last sample: once the audio is over, they are shown.
        let clock = match self.audio_done.load(Ordering::SeqCst) == epoch + 1 {
            true => f64::INFINITY,
            false => self.clock().at(now),
        };
        let mut frames = lock(&self.frames);
        if frames.select(epoch, clock) {
            self.frames_changed.notify_all();
        }

        frames.current.clone()
    }

    fn next_frame_at(&self, now: Instant) -> Option<Instant> {
        if !self.playing.load(Ordering::SeqCst) || self.failure.get().is_some() {
            return None;
        }
        let clock = *self.clock();
        let next = lock(&self.frames).next_pts(self.epoch());
        match next {
            Some(pts) if clock.is_running() => {
                Some(now + Duration::from_secs_f64((pts - clock.at(now)).max(0.0)))
            }
            _ => Some(now + IDLE),
        }
    }

    fn push_frame(&self, frame: Frame) -> bool {
        let epoch = frame.epoch;
        let mut frames = lock(&self.frames);
        while frames.queue.len() >= FRAME_CAPACITY && self.is_current(epoch) {
            frames = self
                .frames_changed
                .wait_timeout(frames, Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        if !self.is_current(epoch) {
            return false;
        }
        frames.queue.push_back(Arc::new(frame));

        true
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(epoch: u64, pts: f64) -> Frame {
        Frame {
            epoch,
            pts,
            width: 2,
            height: 2,
            y: vec![0; 4],
            u: vec![0],
            v: vec![0],
            rgba: None,
        }
    }

    fn queue(frames: &mut Frames, epoch: u64, times: &[f64]) {
        for pts in times {
            frames.queue.push_back(Arc::new(frame(epoch, *pts)));
        }
    }

    fn shown(frames: &Frames) -> Option<(u64, f64)> {
        frames
            .current
            .as_ref()
            .map(|frame| (frame.epoch, frame.pts))
    }

    #[test]
    fn shows_the_last_frame_not_after_the_clock() {
        let mut frames = Frames::default();
        queue(&mut frames, 0, &[0.0, 0.1, 0.2, 0.3]);

        frames.select(0, 0.25);

        assert_eq!(shown(&frames), Some((0, 0.2)));
        assert_eq!(frames.next_pts(0), Some(0.3));
    }

    #[test]
    fn nothing_comes_before_the_first_frame() {
        let mut frames = Frames::default();
        assert!(!frames.select(0, 5.0));
        assert_eq!(shown(&frames), None);

        queue(&mut frames, 0, &[20.002, 20.019]);
        frames.select(0, 20.0);

        assert_eq!(shown(&frames), Some((0, 20.002)));
        frames.select(0, 20.01);
        assert_eq!(shown(&frames), Some((0, 20.002)));
    }

    #[test]
    fn frames_of_an_old_epoch_are_thrown_away() {
        let mut frames = Frames::default();
        queue(&mut frames, 0, &[1.0]);
        frames.select(0, 1.0);
        queue(&mut frames, 0, &[1.1, 1.2]);
        queue(&mut frames, 1, &[40.0, 40.1]);

        frames.select(1, 40.0);

        assert_eq!(shown(&frames), Some((1, 40.0)));
        assert_eq!(frames.queue.len(), 1);
        assert!(!frames.has(0));
    }

    #[test]
    fn the_old_frame_stays_until_the_new_epoch_has_one() {
        let mut frames = Frames::default();
        queue(&mut frames, 0, &[1.0]);
        frames.select(0, 1.0);

        frames.select(1, 40.0);

        assert_eq!(shown(&frames), Some((0, 1.0)));
    }

    #[test]
    fn a_seek_starts_a_new_epoch_and_drops_the_queued_data() {
        let shared = Shared::new(Options::default());
        assert!(shared.push_frame(frame(0, 0.0)));
        shared.audio().push(0, 0.0, &[0.0; 96]);

        shared.seek(40.0);

        assert_eq!(shared.epoch(), 1);
        assert_eq!(shared.seek_target(), 40.0);
        assert!(!shared.push_frame(frame(0, 0.1)));
        assert!(!shared.audio().push(0, 0.1, &[0.0; 96]));
        assert_eq!(shared.audio().seconds(), 0.0);
        assert!(lock(&shared.frames).queue.is_empty());
        assert_eq!(shared.clock().at(Instant::now()), 40.0);
    }

    #[test]
    fn the_audio_waits_for_the_first_drawn_frame() {
        let shared = Shared::new(Options::default());
        assert!(shared.push_frame(frame(0, 0.0)));
        assert!(!shared.is_audible(0));
        assert_eq!(shared.state(), State::Loading);

        let shown = shared.select(Instant::now()).unwrap();
        assert!(!shared.is_audible(0));
        shared.drawn(shown.epoch);

        assert!(shared.is_audible(0));
        assert_eq!(shared.state(), State::Playing);
        shared.seek(10.0);
        assert!(!shared.is_audible(1));
        shared.drawn(0);
        assert!(!shared.is_audible(1));
    }

    #[test]
    fn ends_when_the_audio_has_played_out_and_no_frame_is_left() {
        let shared = Shared::new(Options::default());
        assert!(shared.push_frame(frame(0, 85.4)));
        shared.drawn(0);
        shared.video_finished(0);
        shared.audio_played_out(0);
        assert_eq!(shared.state(), State::Playing);

        shared.select(Instant::now());

        assert_eq!(shared.state(), State::Ended);
        shared.seek(0.0);
        assert_eq!(shared.state(), State::Playing);
    }

    #[test]
    fn frames_after_the_last_sample_are_shown_at_the_end() {
        let shared = Shared::new(Options::default());
        assert!(shared.push_frame(frame(0, 85.48)));
        shared.select(Instant::now());
        shared.drawn(0);
        assert!(shared.push_frame(frame(0, 85.50)));
        assert!(shared.push_frame(frame(0, 85.52)));
        *shared.clock() = Clock::stopped(85.49);
        shared.video_finished(0);
        shared.audio_played_out(0);

        let last = shared.select(Instant::now()).unwrap();

        assert_eq!(last.pts, 85.52);
        assert_eq!(shared.state(), State::Ended);
    }

    #[test]
    fn the_video_alone_does_not_end_the_trailer() {
        let shared = Shared::new(Options::default());
        shared.drawn(0);
        shared.video_finished(0);

        assert_eq!(shared.state(), State::Playing);
    }

    #[test]
    fn pause_and_failure_show_in_the_state() {
        let shared = Shared::new(Options::default());
        shared.started.store(true, Ordering::SeqCst);
        shared.playing.store(false, Ordering::SeqCst);
        assert_eq!(shared.state(), State::Paused);
        assert_eq!(shared.next_frame_at(Instant::now()), None);

        shared.fail("there is no audio output device".to_string());
        shared.fail("a later error".to_string());

        assert_eq!(
            shared.state(),
            State::Failed("there is no audio output device".to_string())
        );
    }

    #[test]
    fn a_stopped_player_takes_no_more_frames() {
        let shared = Shared::new(Options::default());
        shared.stop();

        assert!(!shared.push_frame(frame(0, 0.0)));
        assert!(!shared.is_audible(0));
    }

    #[test]
    fn without_a_runtime_the_player_fails() {
        let player = Player::new(
            Source::Dash {
                manifest_url: "https://video.akamai.steamstatic.com/a/dash_av1.mpd".to_string(),
                height: 720,
            },
            Options::default(),
        );

        assert!(matches!(player.state(), State::Failed(_)));
    }

    fn idle_player(options: Options) -> Player {
        Player {
            shared: Arc::new(Shared::new(options)),
            source: Source::Dash {
                manifest_url: "https://video.akamai.steamstatic.com/a/dash_av1.mpd".to_string(),
                height: 720,
            },
            options,
            scrub: None,
            hovered: false,
            loading_since: Instant::now(),
            views: AtomicU32::new(0),
            views_since: Instant::now(),
        }
    }

    fn started(options: Options) -> Player {
        let player = idle_player(options);
        player.shared.drawn(0);
        player
    }

    #[test]
    fn the_bar_toggles_pause_and_play() {
        let mut player = started(Options::default());
        assert_eq!(player.state(), State::Playing);

        player.update(Message::Toggle);
        assert_eq!(player.state(), State::Paused);
        player.update(Message::Toggle);
        assert_eq!(player.state(), State::Playing);
    }

    #[test]
    fn a_click_does_nothing_at_the_end() {
        let mut player = started(Options::default());
        player.shared.video_finished(0);
        player.shared.audio_played_out(0);
        assert_eq!(player.state(), State::Ended);

        player.update(Message::Toggle);

        assert_eq!(player.state(), State::Ended);
        assert!(player.shared.playing.load(Ordering::SeqCst));
    }

    #[test]
    fn the_position_seeks_when_it_is_released() {
        let mut player = started(Options::default());

        player.update(Message::Scrub(12.0));
        player.update(Message::Scrub(40.5));
        assert_eq!(player.shared.epoch(), 0);
        player.update(Message::Seek);

        assert_eq!(player.shared.epoch(), 1);
        assert_eq!(player.shared.seek_target(), 40.5);
        assert_eq!(player.scrub, None);
        player.update(Message::Seek);
        assert_eq!(player.shared.epoch(), 1);
    }

    #[test]
    fn mute_switches_the_sound() {
        let mut player = started(Options::default());

        player.update(Message::Mute);
        assert!(player.is_muted());
        player.update(Message::Mute);
        assert!(!player.is_muted());
    }

    #[test]
    fn the_volume_slider_sets_the_gain_and_zero_mutes() {
        let mut player = started(Options::default());

        player.update(Message::Volume(0.5));
        assert_eq!(player.volume(), 0.5);
        assert_eq!(player.shared.gain(), 0.25);
        assert!(!player.is_silent());

        player.update(Message::Volume(0.0));
        assert!(player.is_muted());
        assert_eq!(player.shared.gain(), 0.0);

        player.update(Message::Volume(0.3));
        assert!(!player.is_muted());
    }

    #[test]
    fn unmuting_at_zero_comes_back_at_a_quarter() {
        let mut player = started(Options::default());
        player.update(Message::Volume(0.4));
        player.update(Message::Mute);
        assert!(player.is_muted());
        assert_eq!(player.shared.gain(), 0.0);

        player.update(Message::Mute);
        assert_eq!(player.volume(), 0.4);

        player.update(Message::Volume(0.0));
        player.update(Message::Mute);
        assert!(!player.is_muted());
        assert_eq!(player.volume(), START_VOLUME);
    }

    #[test]
    fn replay_starts_again_from_the_beginning() {
        let mut player = started(Options::default());
        player.seek(Duration::from_secs(82));
        player.pause();

        player.update(Message::Replay);

        assert_eq!(player.shared.epoch(), 2);
        assert_eq!(player.shared.seek_target(), 0.0);
        assert!(player.shared.playing.load(Ordering::SeqCst));
    }

    #[test]
    fn retry_starts_a_new_engine_with_the_same_mute() {
        let mut player = started(Options::default());
        player.set_muted(true);
        player.set_volume(0.6);
        let old = Arc::clone(&player.shared);

        player.update(Message::Retry);

        assert!(old.is_stopped());
        assert!(!Arc::ptr_eq(&old, &player.shared));
        assert!(player.is_muted());
        assert_eq!(player.volume(), 0.6);
    }

    #[test]
    fn the_bar_follows_the_mouse() {
        let mut player = started(Options::default());

        player.update(Message::Hovered(true));
        assert!(player.hovered);
        player.update(Message::Hovered(false));
        assert!(!player.hovered);
    }

    #[test]
    fn an_old_frame_shows_until_the_seek_brings_a_new_one() {
        let player = started(Options::default());
        assert!(player.shared.push_frame(frame(0, 1.0)));
        player.shared.select(Instant::now());
        assert!(!player.shows_an_old_frame());

        player.seek(Duration::from_secs(40));
        player.shared.select(Instant::now());
        assert!(player.shows_an_old_frame());

        assert!(player.shared.push_frame(frame(1, 40.0)));
        player.shared.select(Instant::now());
        assert!(!player.shows_an_old_frame());
    }

    #[test]
    fn the_views_are_counted_once_a_second() {
        let mut player = started(Options::default());
        let start = player.views_since;
        player.views.store(4, Ordering::Relaxed);

        player.log_views(start + Duration::from_millis(500));
        assert_eq!(player.views.load(Ordering::Relaxed), 4);
        player.log_views(start + Duration::from_secs(1));

        assert_eq!(player.views.load(Ordering::Relaxed), 0);
        assert_eq!(player.views_since, start + Duration::from_secs(1));
    }

    const HADES: &str = "https://video.akamai.steamstatic.com/store_trailers/1145360/312686/b0312c49135823cc34946e98148abfb8da71f0fb/1750627549/dash_av1.mpd";

    fn watch(player: &Player, seconds: f64, label: &str) -> usize {
        let start = Instant::now();
        let mut drawn = 0;
        let mut last = None;
        let mut drifts = Vec::new();
        while start.elapsed().as_secs_f64() < seconds {
            if let Some(frame) = player.frame() {
                player.drawn(&frame);
                if last != Some((frame.epoch, frame.pts)) {
                    last = Some((frame.epoch, frame.pts));
                    drawn += 1;
                }
                if player.state() == State::Playing && frame.epoch == player.shared.epoch() {
                    drifts.push((player.position().as_secs_f64() - frame.pts) * 1000.0);
                }
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        let mean = drifts.iter().sum::<f64>() / drifts.len().max(1) as f64;
        let max = drifts.iter().copied().fold(0.0, f64::max);
        println!(
            "{label}: {:?}, at {:.2} s, {drawn} frames, drift mean {mean:.1} ms, max {max:.1} ms",
            player.state(),
            player.position().as_secs_f64()
        );

        drawn
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "plays the Hades trailer from Steam with sound"]
    async fn plays_the_hades_trailer() {
        tokio::task::spawn_blocking(|| {
            let silent = std::env::var_os("GAMEPALATE_TEST_MUTED").is_some();
            let player = Player::new(
                Source::Dash {
                    manifest_url: HADES.to_string(),
                    height: 720,
                },
                Options {
                    muted: silent,
                    ..Options::default()
                },
            );

            assert!(watch(&player, 6.0, "start") > 100);
            player.seek(Duration::from_secs(40));
            assert!(watch(&player, 4.0, "after a seek to 40 s") > 100);
            player.set_muted(true);
            watch(&player, 1.5, "muted");
            player.set_muted(silent);
            player.pause();
            watch(&player, 1.0, "paused");
            player.play();
            watch(&player, 2.0, "playing again");
            player.seek(Duration::from_secs(82));
            watch(&player, 6.0, "the end");

            assert_eq!(player.state(), State::Ended);
        })
        .await
        .unwrap();
    }
}
