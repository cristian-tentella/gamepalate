use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use iced::Subscription;

use crate::video::{self, FULL_VOLUME, Player, Surface};

/// The sound of the trailers, one for the whole GUI: every gallery, in every screen, until the
/// GUI closes. Nothing is saved.
static SOUND: LazyLock<Arc<Mutex<Sound>>> = LazyLock::new(Arc::default);

/// Until the sound is touched, a trailer that starts by itself is muted and a click plays with
/// sound; afterwards every trailer starts as the sound was left.
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
            volume: FULL_VOLUME,
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

    fn chosen(&mut self, player: &Player) {
        *self = Sound {
            muted: player.is_muted(),
            volume: player.volume(),
            touched: true,
        };
    }
}

/// The one trailer playing in a gallery, the size it is drawn at, and whether the first trailer
/// of the game just shown may start by itself.
pub(super) struct Trailers {
    playing: Option<Playing>,
    surface: Option<Surface>,
    armed: bool,
    sound: Arc<Mutex<Sound>>,
}

impl Default for Trailers {
    fn default() -> Trailers {
        Trailers {
            playing: None,
            surface: None,
            armed: false,
            sound: Arc::clone(&SOUND),
        }
    }
}

struct Playing {
    index: usize,
    player: Player,
}

/// What the gallery says about the first trailer of a new game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Due {
    /// The details are not there yet.
    Wait,
    /// No trailer comes first, or something covers the gallery.
    Never,
    /// The manifest of the first trailer.
    Play(String),
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
        self.armed = false;
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

    /// Drops the player, which stops its threads: the poster comes back.
    pub(super) fn stop(&mut self) {
        self.playing = None;
        self.armed = false;
    }

    /// A new game is shown: its first trailer may start by itself.
    pub(super) fn arm(&mut self) {
        self.stop();
        self.armed = true;
    }

    pub(super) fn is_armed(&self) -> bool {
        self.armed
    }

    /// The first trailer starts muted until the sound is touched, once the size of the poster
    /// is known.
    pub(super) fn autoplay(&mut self, due: Due) {
        match due {
            Due::Wait => {}
            Due::Never => self.armed = false,
            Due::Play(manifest_url) => {
                if self.armed {
                    self.start(0, &manifest_url, false);
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

        trailers.arm();

        assert!(trailers.player(2).is_none());
        assert!(trailers.is_armed());
    }

    #[test]
    fn the_first_trailer_starts_by_itself_muted() {
        let mut trailers = measured();
        trailers.arm();

        trailers.autoplay(Due::Wait);
        assert!(trailers.player(0).is_none());
        assert!(trailers.is_armed());
        trailers.autoplay(Due::Play(MANIFEST.to_string()));

        assert!(trailers.player(0).is_some_and(Player::is_muted));
        assert!(!trailers.is_armed());
    }

    #[test]
    fn the_first_trailer_waits_for_the_size_of_the_poster() {
        let mut trailers = alone();
        trailers.arm();

        trailers.autoplay(Due::Play(MANIFEST.to_string()));
        assert!(trailers.player(0).is_none());
        assert!(trailers.is_armed());

        trailers.measured(Surface {
            height: 450,
            software: true,
        });
        trailers.autoplay(Due::Play(MANIFEST.to_string()));
        assert!(trailers.player(0).is_some());
    }

    #[test]
    fn nothing_starts_by_itself_when_it_is_not_due_or_not_armed() {
        let mut trailers = measured();
        trailers.arm();
        trailers.autoplay(Due::Never);
        trailers.autoplay(Due::Play(MANIFEST.to_string()));
        assert!(trailers.player(0).is_none());

        trailers.arm();
        trailers.stop();
        trailers.autoplay(Due::Play(MANIFEST.to_string()));
        assert!(trailers.player(0).is_none());
    }

    #[test]
    fn until_the_sound_is_touched_a_click_plays_with_sound() {
        let mut trailers = measured();
        trailers.arm();
        trailers.autoplay(Due::Play(MANIFEST.to_string()));
        assert!(trailers.player(0).is_some_and(Player::is_muted));

        trailers.play(0, MANIFEST);
        assert!(trailers.player(0).is_some_and(|player| !player.is_muted()));
        assert_eq!(trailers.player(0).map(Player::volume), Some(FULL_VOLUME));
    }

    #[test]
    fn once_unmuted_the_first_trailer_starts_with_sound() {
        let mut trailers = measured();
        trailers.arm();
        trailers.autoplay(Due::Play(MANIFEST.to_string()));
        trailers.update(video::Message::Mute);
        trailers.update(video::Message::Volume(0.4));

        trailers.arm();
        trailers.autoplay(Due::Play(MANIFEST.to_string()));

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
    fn hovering_the_bar_does_not_touch_the_sound() {
        let mut trailers = measured();
        trailers.play(0, MANIFEST);
        trailers.update(video::Message::Hovered(true));
        trailers.update(video::Message::VolumeHovered(true));

        assert!(!trailers.sound().touched);
    }
}
