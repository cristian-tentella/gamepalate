use gamepalate_core::{
    ids::AppId,
    search::{SearchInputs, SearchOptions},
    taste::game_name,
    timestamp::Timestamp,
};

use super::Gamepalate;
use crate::{
    discover_place::{DiscoverPlace, discover_place},
    library::{
        FoundGame, GameCorrection, GameCorrectionError, GameView, LibraryView,
        PlayedElsewhereError, changed_played_elsewhere, corrected_game, found_games,
        game_correction, game_view, library_view,
    },
    log::failed,
    profile::{ProfileView, profile_view},
    tags::{GameTagsView, game_tags_view},
};

impl Gamepalate {
    pub fn profile_view(&self) -> ProfileView {
        self.with_inputs(profile_view)
    }

    pub fn game_tags(&self, id: AppId) -> Option<GameTagsView> {
        self.with_inputs(|inputs| game_tags_view(id, inputs))
    }

    pub fn library_view(&self) -> LibraryView {
        self.with_inputs(library_view)
    }

    pub fn correct_game(
        &self,
        id: AppId,
        correction: GameCorrection,
    ) -> Result<GameCorrection, GameCorrectionError> {
        let votes = self.lock_votes();
        let mut corrections = self.write_corrections();
        let families = self.lock_families();
        let before = game_correction(id, &corrections);
        *corrections = corrected_game(
            &self.data_dir.join("corrections.toml"),
            id,
            correction,
            &SearchInputs {
                votes: &votes,
                corrections: &corrections,
                families: &families,
                ..self.files.search_inputs(&self.vectors)
            },
        )
        .inspect_err(failed("could not save the correction"))?;
        Ok(before)
    }

    pub fn set_played_elsewhere(
        &self,
        id: AppId,
        played: bool,
    ) -> Result<bool, PlayedElsewhereError> {
        let mut corrections = self.write_corrections();
        let before = corrections.played_elsewhere.contains(&id);
        let name = game_name(id, &self.files.catalog, &self.files.off_catalog);
        *corrections = changed_played_elsewhere(
            &self.data_dir.join("corrections.toml"),
            id,
            played,
            &corrections,
            (name != format!("#{}", id.0)).then_some(name.as_str()),
        )
        .inspect_err(failed("could not save the game played elsewhere"))?;
        Ok(before)
    }

    /// The games of the whole catalog for a name, an appid or a store link, Library included,
    /// each with where it stands for you.
    pub fn find_games(&self, text: &str) -> Vec<FoundGame> {
        let ids = self.names.find(text, &self.files.catalog);
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());
        self.with_inputs(|inputs| found_games(&ids, inputs, now))
    }

    /// One game of the catalog for its page, read again after each change.
    pub fn game_view(&self, id: AppId) -> Option<GameView> {
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());
        self.with_inputs(|inputs| game_view(id, inputs, now))
    }

    /// Where a game of the catalog stands in a Discover opened now with these options: its
    /// position, or the first filter that leaves it out.
    pub fn discover_place(&self, id: AppId, options: &SearchOptions) -> Option<DiscoverPlace> {
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());
        self.with_inputs(|inputs| discover_place(id, options, inputs, now))
    }
}
