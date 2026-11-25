use std::collections::{BTreeMap, HashSet};

use gamepalate_app::Block;
use iced::Subscription;

use crate::video::{self, Options, Player, Source};

/// The videos of the description in the About sheet, by the index of their block. Every video
/// with more than a quarter of its height in sight plays by itself; under that it pauses, and
/// picks up where it was when it comes back. A video the user paused stays paused while the sheet
/// is open.
#[derive(Default)]
pub(super) struct Loops {
    /// Whether the frames are drawn by tiny-skia, once the sheet has measured it.
    software: Option<bool>,
    /// The search or a game is over the screen.
    covered: bool,
    /// The videos with more than a quarter of their height in sight, from `sight`.
    in_sight: HashSet<usize>,
    /// The videos the user paused.
    paused: HashSet<usize>,
    players: BTreeMap<usize, Player>,
}

impl Loops {
    pub(super) fn measured(&mut self, software: bool) {
        self.software = Some(software);
    }

    pub(super) fn sight(&mut self, index: usize, in_sight: bool) {
        match in_sight {
            true => self.in_sight.insert(index),
            false => self.in_sight.remove(&index),
        };
    }

    /// Drops the players under the search or a game; they come back with the next sync.
    pub(super) fn cover(&mut self, covered: bool) {
        self.covered = covered;
        if covered {
            self.players.clear();
        }
    }

    /// The sheet closed or the game changed: the players and what the user chose go.
    pub(super) fn clear(&mut self) {
        *self = Loops {
            software: self.software,
            covered: self.covered,
            ..Loops::default()
        };
    }

    /// The pause button in the corner, or the play button on a video the user paused.
    fn toggle(&mut self, index: usize) {
        match self.plays(index) {
            true => self.paused.insert(index),
            false => self.paused.remove(&index),
        };
    }

    pub(super) fn plays(&self, index: usize) -> bool {
        self.in_sight.contains(&index) && !self.paused.contains(&index)
    }

    pub(super) fn update(&mut self, index: usize, message: video::Message) {
        match message {
            video::Message::Toggle => self.toggle(index),
            message => {
                if let Some(player) = self.players.get_mut(&index) {
                    player.update(message);
                }
            }
        }
    }

    /// Makes the players that should play and do not exist yet, then plays or pauses each one.
    /// `blocks` is the description in the About sheet, `None` while no sheet shows one.
    pub(super) fn sync(&mut self, blocks: Option<&[Block]>) {
        let Some(blocks) = blocks else {
            self.clear();
            return;
        };
        let Some(software) = self.software.filter(|_| !self.covered) else {
            self.players.clear();
            return;
        };
        let wanted: Vec<usize> = self
            .in_sight
            .iter()
            .copied()
            .filter(|&index| self.plays(index) && !self.players.contains_key(&index))
            .collect();
        for index in wanted {
            if let Some(Block::Video {
                webm: Some(url), ..
            }) = blocks.get(index)
            {
                let source = Source::Loop { url: url.clone() };
                let options = Options {
                    muted: true,
                    volume: 0.0,
                    rgba: software,
                };
                self.players.insert(index, Player::new(source, options));
            }
        }
        for (&index, player) in &self.players {
            match self.plays(index) {
                true => player.play(),
                false => player.pause(),
            }
        }
    }

    pub(super) fn player(&self, index: usize) -> Option<&Player> {
        self.players.get(&index)
    }

    pub(super) fn subscription(&self) -> Subscription<(usize, video::Message)> {
        Subscription::batch(
            self.players
                .iter()
                .map(|(&index, player)| player.subscription().with(index)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(webm: Option<&str>) -> Block {
        Block::Video {
            webm: webm.map(str::to_string),
            mp4: None,
            poster: None,
            width: 600,
            height: 338,
        }
    }

    /// A heading, then three videos at 1, 2 and 3, and one without a `webm` at 4.
    fn blocks() -> Vec<Block> {
        vec![
            Block::Heading("Hades".to_string()),
            video(Some("https://a/1.webm")),
            video(Some("https://a/2.webm")),
            video(Some("https://a/3.webm")),
            video(None),
        ]
    }

    fn measured() -> Loops {
        let mut loops = Loops::default();
        loops.measured(false);
        loops
    }

    fn players(loops: &Loops) -> Vec<usize> {
        loops.players.keys().copied().collect()
    }

    #[test]
    fn every_video_in_sight_plays() {
        let mut loops = measured();
        loops.sight(2, true);
        loops.sight(3, true);

        loops.sync(Some(&blocks()));

        assert_eq!(players(&loops), [2, 3]);
        assert!(loops.plays(2));
        assert!(loops.plays(3));
        assert!(!loops.plays(1));
    }

    #[test]
    fn nothing_plays_before_the_sheet_is_measured() {
        let mut loops = Loops::default();
        loops.sight(1, true);

        loops.sync(Some(&blocks()));

        assert!(players(&loops).is_empty());
    }

    #[test]
    fn a_video_out_of_sight_pauses_and_comes_back() {
        let mut loops = measured();
        loops.sight(1, true);
        loops.sight(2, true);
        loops.sync(Some(&blocks()));

        loops.sight(1, false);
        loops.sync(Some(&blocks()));
        assert_eq!(players(&loops), [1, 2]);
        assert!(!loops.plays(1));
        assert!(loops.plays(2));

        loops.sight(1, true);
        loops.sync(Some(&blocks()));
        assert!(loops.plays(1));
        assert!(loops.plays(2));
    }

    #[test]
    fn a_video_never_in_sight_gets_no_player() {
        let mut loops = measured();
        loops.sight(1, true);
        loops.sight(3, false);

        loops.sync(Some(&blocks()));

        assert_eq!(players(&loops), [1]);
    }

    #[test]
    fn a_click_pauses_a_video_and_a_second_click_plays_it() {
        let mut loops = measured();
        loops.sight(1, true);
        loops.sync(Some(&blocks()));

        loops.update(1, video::Message::Toggle);
        assert!(!loops.plays(1));

        loops.update(1, video::Message::Toggle);
        assert!(loops.plays(1));
    }

    #[test]
    fn a_click_on_a_video_out_of_sight_changes_nothing() {
        let mut loops = measured();
        loops.sight(1, false);

        loops.update(1, video::Message::Toggle);
        loops.sight(1, true);

        assert!(loops.plays(1));
    }

    #[test]
    fn a_video_paused_by_the_user_does_not_start_again_by_itself() {
        let mut loops = measured();
        loops.sight(1, true);
        loops.sync(Some(&blocks()));

        loops.update(1, video::Message::Toggle);
        loops.sight(1, false);
        loops.sight(1, true);
        loops.sync(Some(&blocks()));

        assert!(!loops.plays(1));
        assert!(loops.player(1).is_some());
    }

    #[test]
    fn a_video_without_a_webm_gets_no_player() {
        let mut loops = measured();
        loops.sight(4, true);

        loops.sync(Some(&blocks()));

        assert!(players(&loops).is_empty());
    }

    #[test]
    fn the_players_drop_while_covered_and_come_back_after() {
        let mut loops = measured();
        loops.sight(1, true);
        loops.sync(Some(&blocks()));

        loops.cover(true);
        loops.sync(Some(&blocks()));
        assert!(players(&loops).is_empty());

        loops.cover(false);
        loops.sync(Some(&blocks()));
        assert_eq!(players(&loops), [1]);
    }

    #[test]
    fn no_player_without_the_sheet() {
        let mut loops = measured();
        loops.sight(1, true);
        loops.sync(Some(&blocks()));
        loops.update(1, video::Message::Toggle);

        loops.sync(None);

        assert!(players(&loops).is_empty());
        assert!(loops.in_sight.is_empty());
        assert!(loops.paused.is_empty());
        assert_eq!(loops.software, Some(false));
    }

    #[test]
    fn a_new_game_forgets_the_players_and_the_choices() {
        let mut loops = measured();
        loops.sight(1, true);
        loops.sync(Some(&blocks()));
        loops.update(1, video::Message::Toggle);

        loops.clear();

        assert!(players(&loops).is_empty());
        assert!(loops.paused.is_empty());
        assert!(!loops.plays(1));
    }
}
