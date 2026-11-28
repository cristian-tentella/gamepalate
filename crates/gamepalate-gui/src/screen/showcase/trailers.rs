use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use iced::Subscription;

use crate::video::{self, Player, START_VOLUME, State, Surface};

/// The sound of the trailers, one for the whole GUI: every gallery, in every screen, until the
/// GUI closes. Nothing is saved.
static SOUND: LazyLock<Arc<Mutex<Sound>>> = LazyLock::new(Arc::default);

/// Until the sound is touched, a trailer that starts by itself is muted and a click plays with
/// sound, both at [`START_VOLUME`]; afterwards every trailer starts as the sound was left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sound {
    muted: bool,
    volume: f32,
    touched: bool,
}

impl Default for Sound {
    fn default() -> Sound {
        Sound {
            muted: false,
            volume: START_VOLUME,
            touched: false,
        }
    }
}

impl Sound {
    /// Muted and volume for a trailer that starts by itself, or with a click.
    fn start(&self, by_click: bool) -> (bool, f32) {
        match self.touched {
            true => (self.muted, self.volume),
            false => (!by_click, self.volume),
        }
    }

    /// `M` on the poster: the same switch as the mute button of a trailer.
    fn switch(&mut self) {
        if self.muted || self.volume == 0.0 {
            if self.volume == 0.0 {
                self.volume = START_VOLUME;
            }
            self.muted = false;
        } else {
            self.muted = true;
        }
        self.touched = true;
    }

    fn chosen(&mut self, player: &Player) {
        *self = Sound {
            muted: player.is_muted(),
            volume: player.volume(),
            touched: true,
        };
    }
}

/// The one trailer playing in a gallery, the size it is drawn at, and the trailer that may start
/// by itself once the gallery shows it.
pub(super) struct Trailers {
    playing: Option<Playing>,
    surface: Option<Surface>,
    armed: Option<usize>,
    sound: Arc<Mutex<Sound>>,
}

impl Default for Trailers {
    fn default() -> Trailers {
        Trailers {
            playing: None,
            surface: None,
            armed: None,
            sound: Arc::clone(&SOUND),
        }
    }
}

struct Playing {
    index: usize,
    player: Player,
}

/// What the gallery says about the armed trailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Due {
    /// The details are not there yet.
    Wait,
    /// The gallery shows something else, or something covers it.
    Never,
    /// The index and the manifest of the trailer shown.
    Play(usize, String),
}

impl Trailers {
    pub(super) fn measured(&mut self, surface: Surface) {
        self.surface = Some(surface);
    }

    /// A click on the poster plays with sound, unless the sound was turned off before.
    pub(super) fn play(&mut self, index: usize, manifest_url: &str) {
        self.start(index, manifest_url, true);
    }

    fn start(&mut self, index: usize, manifest_url: &str, by_click: bool) {
        let Some(surface) = self.surface else {
            return;
        };
        let (muted, volume) = self.sound().start(by_click);
        self.armed = None;
        self.playing = Some(Playing {
            index,
            player: Player::new(
                surface.source(manifest_url),
                surface.options(muted || test_muted(), volume),
            ),
        });
    }

    fn sound(&self) -> Sound {
        *self.sound.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `P`: on the poster it plays, on the video it pauses or plays again, at the end it replays.
    pub(super) fn play_or_pause(&mut self, index: usize, manifest_url: &str) {
        match self.player(index).map(Player::state) {
            None => self.play(index, manifest_url),
            Some(State::Ended) => self.update(video::Message::Replay),
            Some(_) => self.update(video::Message::Toggle),
        }
    }

    /// `M`: the mute of the trailer playing, or of the shared sound on the poster.
    pub(super) fn switch_mute(&mut self, index: usize) {
        match self.player(index) {
            Some(_) => self.update(video::Message::Mute),
            None => self
                .sound
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .switch(),
        }
    }

    /// Drops the player, which stops its threads: the poster comes back.
    pub(super) fn stop(&mut self) {
        self.playing = None;
        self.armed = None;
    }

    /// The gallery moved to a trailer, or a new game shows one first: it may start by itself.
    pub(super) fn arm(&mut self, index: usize) {
        self.stop();
        self.armed = Some(index);
    }

    pub(super) fn armed(&self) -> Option<usize> {
        self.armed
    }

    /// The armed trailer starts from 0, muted until the sound is touched, once the size of the
    /// poster is known.
    pub(super) fn autoplay(&mut self, due: Due) {
        match due {
            Due::Wait => {}
            Due::Never => self.armed = None,
            Due::Play(index, manifest_url) => {
                if self.armed == Some(index) {
                    self.start(index, &manifest_url, false);
                }
            }
        }
    }

    pub(super) fn update(&mut self, message: video::Message) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let touches_the_sound = matches!(message, video::Message::Mute | video::Message::Volume(_));
        playing.player.update(message);
        if touches_the_sound {
            self.sound
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .chosen(&playing.player);
        }
    }

    pub(super) fn player(&self, index: usize) -> Option<&Player> {
        self.playing
            .as_ref()
            .filter(|playing| playing.index == index)
            .map(|playing| &playing.player)
    }

    pub(super) fn subscription(&self) -> Subscription<video::Message> {
        self.playing
            .as_ref()
            .map_or_else(Subscription::none, |playing| playing.player.subscription())
    }
}

// The trials of the GUI on a copy of the data stay silent, as the ignored test of the player does.
fn test_muted() -> bool {
    std::env::var_os("GAMEPALATE_TEST_MUTED").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "https://video.akamai.steamstatic.com/a/dash_av1.mpd";

    /// A gallery with a sound of its own, so that the tests do not share the GUI's.
    fn alone() -> Trailers {
        Trailers {
            sound: Arc::default(),
            ..Trailers::default()
        }
    }

    fn measured() -> Trailers {
        let mut trailers = alone();
        trailers.measured(Surface {
            height: 450,
            software: false,
        });
        trailers
    }

    #[test]
    fn play_makes_the_player_of_that_trailer() {
        let mut trailers = measured();

        trailers.play(1, MANIFEST);

        assert!(trailers.player(1).is_some());
        assert!(trailers.player(0).is_none());
    }

    #[test]
    fn nothing_plays_before_the_poster_is_measured() {
        let mut trailers = alone();

        trailers.play(0, MANIFEST);

        assert!(trailers.player(0).is_none());
    }

    #[test]
    fn stopping_drops_the_player() {
        let mut trailers = measured();
        trailers.play(0, MANIFEST);

        trailers.stop();

        assert!(trailers.player(0).is_none());
    }

    #[test]
    fn a_new_game_stops_the_trailer_and_arms_the_first() {
        let mut trailers = measured();
        trailers.play(2, MANIFEST);

        trailers.arm(0);

        assert!(trailers.player(2).is_none());
        assert_eq!(trailers.armed(), Some(0));
    }

    #[test]
    fn the_first_trailer_starts_by_itself_muted() {
        let mut trailers = measured();
        trailers.arm(0);

        trailers.autoplay(Due::Wait);
        assert!(trailers.player(0).is_none());
        assert_eq!(trailers.armed(), Some(0));
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));

        assert!(trailers.player(0).is_some_and(Player::is_muted));
        assert_eq!(trailers.armed(), None);
    }

    #[test]
    fn the_first_trailer_waits_for_the_size_of_the_poster() {
        let mut trailers = alone();
        trailers.arm(0);

        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        assert!(trailers.player(0).is_none());
        assert_eq!(trailers.armed(), Some(0));

        trailers.measured(Surface {
            height: 450,
            software: true,
        });
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        assert!(trailers.player(0).is_some());
    }

    #[test]
    fn nothing_starts_by_itself_when_it_is_not_due_or_not_armed() {
        let mut trailers = measured();
        trailers.arm(0);
        trailers.autoplay(Due::Never);
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        assert!(trailers.player(0).is_none());

        trailers.arm(0);
        trailers.stop();
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        assert!(trailers.player(0).is_none());
    }

    #[test]
    fn until_the_sound_is_touched_a_click_plays_with_sound() {
        let mut trailers = measured();
        trailers.arm(0);
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        assert!(trailers.player(0).is_some_and(Player::is_muted));

        trailers.play(0, MANIFEST);
        assert!(trailers.player(0).is_some_and(|player| !player.is_muted()));
        assert_eq!(trailers.player(0).map(Player::volume), Some(START_VOLUME));
    }

    #[test]
    fn once_unmuted_the_first_trailer_starts_with_sound() {
        let mut trailers = measured();
        trailers.arm(0);
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        trailers.update(video::Message::Mute);
        trailers.update(video::Message::Volume(0.4));

        trailers.arm(0);
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));

        let player = trailers.player(0).unwrap();
        assert!(!player.is_muted());
        assert_eq!(player.volume(), 0.4);
    }

    #[test]
    fn once_muted_a_click_stays_muted() {
        let mut trailers = measured();
        trailers.play(0, MANIFEST);
        trailers.update(video::Message::Mute);

        trailers.play(1, MANIFEST);
        assert!(trailers.player(1).is_some_and(Player::is_muted));

        trailers.update(video::Message::Mute);
        trailers.play(0, MANIFEST);
        assert!(trailers.player(0).is_some_and(|player| !player.is_muted()));
    }

    #[test]
    fn the_sound_is_one_for_every_gallery() {
        let sound = Arc::new(Mutex::new(Sound::default()));
        let mut discover = Trailers {
            sound: Arc::clone(&sound),
            ..measured()
        };
        let mut votes = Trailers {
            sound,
            ..measured()
        };
        discover.play(0, MANIFEST);
        discover.update(video::Message::Volume(0.0));

        votes.play(0, MANIFEST);

        assert!(votes.player(0).is_some_and(Player::is_silent));
    }

    #[test]
    fn p_on_the_poster_plays_and_on_the_video_pauses() {
        let mut trailers = measured();

        trailers.play_or_pause(1, MANIFEST);
        assert!(trailers.player(1).is_some_and(|player| !player.is_muted()));

        // Without a runtime the player has failed, and P leaves it alone.
        trailers.play_or_pause(1, MANIFEST);
        assert!(trailers.player(1).is_some());
    }

    #[test]
    fn m_on_the_poster_switches_the_shared_sound() {
        let mut trailers = measured();

        trailers.switch_mute(0);
        assert_eq!(
            trailers.sound(),
            Sound {
                muted: true,
                volume: START_VOLUME,
                touched: true
            }
        );
        trailers.play(0, MANIFEST);
        assert!(trailers.player(0).is_some_and(Player::is_muted));

        trailers.switch_mute(0);
        assert!(trailers.player(0).is_some_and(|player| !player.is_muted()));
        assert!(!trailers.sound().muted);
    }

    #[test]
    fn m_brings_back_a_quarter_after_zero() {
        let mut trailers = measured();
        trailers.play(0, MANIFEST);
        trailers.update(video::Message::Volume(0.0));
        trailers.stop();

        trailers.switch_mute(0);

        assert_eq!(
            trailers.sound(),
            Sound {
                muted: false,
                volume: START_VOLUME,
                touched: true
            }
        );
    }

    #[test]
    fn the_first_unmute_brings_a_quarter_of_the_volume() {
        let mut trailers = measured();
        trailers.arm(0);
        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        trailers.update(video::Message::Hovered(true));
        assert!(!trailers.sound().touched);

        trailers.update(video::Message::Mute);

        let player = trailers.player(0).unwrap();
        assert!(!player.is_muted());
        assert_eq!(player.volume(), START_VOLUME);
    }

    #[test]
    fn any_trailer_can_be_armed_and_start_by_itself() {
        let mut trailers = measured();
        trailers.arm(2);

        trailers.autoplay(Due::Play(0, MANIFEST.to_string()));
        assert!(trailers.player(0).is_none());
        trailers.autoplay(Due::Play(2, MANIFEST.to_string()));

        assert!(trailers.player(2).is_some_and(Player::is_muted));
        assert_eq!(trailers.armed(), None);
    }

    #[test]
    fn leaving_a_trailer_stops_it_and_coming_back_starts_a_new_player() {
        let mut trailers = measured();
        trailers.arm(1);
        trailers.autoplay(Due::Play(1, MANIFEST.to_string()));
        trailers.update(video::Message::Hovered(true));

        trailers.stop();
        assert!(trailers.player(1).is_none());

        trailers.arm(1);
        assert!(trailers.player(1).is_none());
        trailers.autoplay(Due::Play(1, MANIFEST.to_string()));
        assert!(trailers.player(1).is_some());
    }

    #[test]
    fn an_armed_trailer_waits_for_the_size_and_then_starts() {
        let mut trailers = alone();
        trailers.arm(1);

        trailers.autoplay(Due::Play(1, MANIFEST.to_string()));
        assert!(trailers.player(1).is_none());
        assert_eq!(trailers.armed(), Some(1));

        trailers.measured(Surface {
            height: 450,
            software: false,
        });
        trailers.autoplay(Due::Play(1, MANIFEST.to_string()));
        assert!(trailers.player(1).is_some());
    }

    #[test]
    fn under_a_sheet_an_armed_trailer_does_not_start() {
        let mut trailers = measured();
        trailers.arm(1);

        trailers.autoplay(Due::Never);
        trailers.autoplay(Due::Play(1, MANIFEST.to_string()));

        assert!(trailers.player(1).is_none());
        assert_eq!(trailers.armed(), None);
    }

    #[test]
    fn a_trailer_reached_is_muted_until_the_sound_is_touched() {
        let mut trailers = measured();
        trailers.arm(1);
        trailers.autoplay(Due::Play(1, MANIFEST.to_string()));
        assert!(trailers.player(1).is_some_and(Player::is_muted));

        trailers.update(video::Message::Mute);
        trailers.arm(2);
        trailers.autoplay(Due::Play(2, MANIFEST.to_string()));

        let player = trailers.player(2).unwrap();
        assert!(!player.is_muted());
        assert_eq!(player.volume(), START_VOLUME);
    }
}
