use gamepalate_core::{
    currency::Currency, filters::FilterOptions, region::Region, timestamp::Timestamp,
};
use gamepalate_steam::archive::PendingUpdate;
use gamepalate_store::gui::{GuiSettings, load_gui_settings};

use super::{GUI_FILE, Gamepalate, change_settings};
use crate::{
    images::MAX_CACHE_BYTES,
    log::failed,
    settings::{self, Settings},
    steam_client::steam_client_file_of,
    update,
};

impl Gamepalate {
    pub fn gui_settings(&self) -> GuiSettings {
        load_gui_settings(&self.data_dir.join(GUI_FILE))
    }

    pub fn change_gui_settings(
        &self,
        change: impl FnOnce(&mut GuiSettings),
    ) -> std::io::Result<()> {
        let _saving = self
            .gui_saving
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change_settings(&self.data_dir.join(GUI_FILE), change)
            .inspect_err(failed("could not save gui.toml"))
    }

    pub fn settings(&self) -> Settings {
        Settings {
            steam_id: self.files.config.steam_id,
            key: settings::key_source(),
            region: self.files.config.region,
            data_dir: self.data_dir.clone(),
            image_cache_max_bytes: MAX_CACHE_BYTES,
            steam_client_hours: self.files.config.signals.steam_client_hours,
            steam_client_file: steam_client_file_of(&self.data_dir),
            catalog_games: self.files.catalog.games().len(),
            tags_updated_at: self.files.catalog.freshness.tags_updated_at,
            config: self.files.config,
        }
    }

    pub fn would_try_hours(&self) -> f64 {
        self.files.config.signals.would_try_hours
    }

    pub fn elsewhere_hours(&self) -> f64 {
        self.files.config.signals.elsewhere_hours
    }

    pub fn bet_every(&self) -> usize {
        self.files.config.results.bet_every
    }

    pub fn config_filters(&self) -> FilterOptions {
        let config = &self.files.config;
        FilterOptions {
            min_reviews: Some(config.relevance.min_reviews),
            min_positive_percent: Some(config.results.min_positive_percent),
            without_early_access: Some(!config.results.early_access),
            ..FilterOptions::default()
        }
    }

    pub fn tag_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .files
            .catalog
            .tags
            .values()
            .map(|tag| tag.english.clone())
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup();
        names
    }

    pub fn signals_read_at(&self) -> Timestamp {
        self.files.signals.read_at
    }

    pub fn prices_updated_at(&self) -> Timestamp {
        self.files.catalog.freshness.prices_updated_at
    }

    pub fn pending_update(&self) -> Option<PendingUpdate> {
        update::pending_update(&self.data_dir, &self.files.config)
    }

    pub fn prices_region(&self) -> Option<Region> {
        let region = self.files.catalog.freshness.region;
        (region != self.files.config.region).then_some(region)
    }

    pub fn currency(&self) -> &Currency {
        &self.files.catalog.currency
    }
}
