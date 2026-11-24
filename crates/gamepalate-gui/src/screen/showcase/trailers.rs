use std::sync::atomic::{AtomicBool, Ordering};

use iced::Subscription;

use crate::video::{self, Player, Surface};

/// A trailer muted with the bar keeps the next ones muted, in every screen, until the GUI closes.
static MUTED: AtomicBool = AtomicBool::new(false);

/// The one trailer playing in a gallery, the size it is drawn at, and whether the first trailer
/// of the game just shown may start by itself.
#[derive(Default)]
pub(super) struct Trailers {
    playing: Option<Playing>,
    surface: Option<Surface>,
    armed: bool,
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

    /// A click on the poster plays with sound, unless a trailer was muted before.
    pub(super) fn play(&mut self, index: usize, manifest_url: &str) {
        let muted = MUTED.load(Ordering::Relaxed) || test_muted();
        self.start(index, manifest_url, muted);
    }

    fn start(&mut self, index: usize, manifest_url: &str, muted: bool) {
        let Some(surface) = self.surface else {
            return;
        };
        self.armed = false;
        self.playing = Some(Playing {
            index,
            player: Player::new(surface.source(manifest_url), surface.options(muted)),
        });
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

    /// The first trailer starts muted, once the size of the poster is known.
    pub(super) fn autoplay(&mut self, due: Due) {
        match due {
            Due::Wait => {}
            Due::Never => self.armed = false,
            Due::Play(manifest_url) => {
                if self.armed {
                    self.start(0, &manifest_url, true);
                }
            }
        }
    }

    pub(super) fn update(&mut self, message: video::Message) {
        let Some(playing) = &mut self.playing else {
            return;
        };
        let mute = message == video::Message::Mute;
        playing.player.update(message);
        if mute {
            MUTED.store(playing.player.is_muted(), Ordering::Relaxed);
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

    fn measured() -> Trailers {
        let mut trailers = Trailers::default();
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
        let mut trailers = Trailers::default();

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
        let mut trailers = Trailers::default();
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
    fn a_click_keeps_the_mute_of_the_last_trailer() {
        let mut trailers = measured();
        trailers.arm();
        trailers.autoplay(Due::Play(MANIFEST.to_string()));
        trailers.play(0, MANIFEST);
        assert!(trailers.player(0).is_some_and(|player| !player.is_muted()));

        trailers.update(video::Message::Mute);
        trailers.play(1, MANIFEST);
        assert!(trailers.player(1).is_some_and(Player::is_muted));

        trailers.update(video::Message::Mute);
        trailers.play(0, MANIFEST);
        assert!(trailers.player(0).is_some_and(|player| !player.is_muted()));
    }
}
