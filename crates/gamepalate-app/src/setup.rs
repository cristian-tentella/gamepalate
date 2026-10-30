use std::{
    error::Error,
    io,
    path::{Path, PathBuf},
};

use gamepalate_core::{config::Config, ids::SteamId};
use gamepalate_steam::archive::PendingUpdate;
use gamepalate_store::{
    config::{ConfigError, load_config},
    gui::{GuiSettings, load_gui_settings},
    signals::{SignalsError, load_signals},
};

use crate::{
    account::steam_api_key,
    app::{GUI_FILE, change_settings},
    data_dir,
    update::{archive, wall_clock},
};

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Present,
    Missing,
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FirstRun {
    pub data_dir: Option<PathBuf>,
    pub steam_id: Part,
    pub saved_steam_id: Option<SteamId>,
    pub api_key: bool,
    pub catalog: Part,
    pub pending_update: Option<PendingUpdate>,
    pub resume_within_hours: u32,
    pub signals: Part,
}

impl FirstRun {
    pub fn check(data_dir: Option<PathBuf>) -> FirstRun {
        first_run(
            data_dir.or_else(data_dir::resolve),
            steam_api_key().is_some(),
        )
    }

    pub fn ready(&self) -> bool {
        [&self.steam_id, &self.catalog, &self.signals]
            .into_iter()
            .all(|part| *part == Part::Present)
    }

    pub fn gui_settings(&self) -> GuiSettings {
        self.data_dir
            .as_ref()
            .map(|dir| load_gui_settings(&dir.join(GUI_FILE)))
            .unwrap_or_default()
    }

    pub fn change_gui_settings(&self, change: impl FnOnce(&mut GuiSettings)) -> io::Result<()> {
        let dir = self
            .data_dir
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
        std::fs::create_dir_all(dir)?;
        change_settings(&dir.join(GUI_FILE), change)
    }
}

pub fn error_chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        text.push_str(": ");
        text.push_str(&error.to_string());
        source = error.source();
    }
    text
}

fn first_run(data_dir: Option<PathBuf>, api_key: bool) -> FirstRun {
    let Some(dir) = data_dir else {
        return FirstRun {
            data_dir: None,
            steam_id: Part::Missing,
            saved_steam_id: None,
            api_key,
            catalog: Part::Missing,
            pending_update: None,
            resume_within_hours: Config::default().prices_max_age_hours,
            signals: Part::Missing,
        };
    };

    let config = load_config(&dir.join("config.toml"));
    let (steam_id, saved_steam_id) = steam_id(&config);
    let settings = config
        .as_ref()
        .map_or_else(|_| Config::default(), |config| *config);

    FirstRun {
        steam_id,
        saved_steam_id,
        api_key,
        catalog: if dir.join("catalog.sqlite").is_file() {
            Part::Present
        } else {
            Part::Missing
        },
        pending_update: archive(&dir)
            .pending_update(wall_clock(), settings.prices_max_age_hours, settings.region)
            .ok()
            .flatten(),
        resume_within_hours: settings.prices_max_age_hours,
        signals: signals(&dir),
        data_dir: Some(dir),
    }
}

fn steam_id(config: &Result<Config, ConfigError>) -> (Part, Option<SteamId>) {
    match config {
        Ok(config) => (Part::Present, config.steam_id),
        Err(ConfigError::NotFound(_) | ConfigError::Missing("steam_id")) => (Part::Missing, None),
        Err(error) => (Part::Invalid(error_chain(&error)), None),
    }
}

fn signals(dir: &Path) -> Part {
    match load_signals(&dir.join("signals.json")) {
        Ok(_) => Part::Present,
        Err(SignalsError::NotFound(_)) => Part::Missing,
        Err(error) => Part::Invalid(error_chain(&error)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_store::gui::ThemeChoice;

    use super::*;

    const STEAM_ID: &str = "steam_id = \"76561198000000000\"\n";
    const SIGNALS: &str = r#"{"readAt":1793000000000,"library":[{"appid":1,"totalHours":3.0}],"recent":[],"wishlist":[],"followed":null}"#;

    fn check(dir: &Path) -> FirstRun {
        first_run(Some(dir.to_path_buf()), false)
    }

    #[test]
    fn an_empty_folder_misses_everything() {
        let dir = tempfile::tempdir().unwrap();

        let first_run = check(dir.path());

        assert_eq!(first_run.steam_id, Part::Missing);
        assert!(!first_run.api_key);
        assert_eq!(first_run.catalog, Part::Missing);
        assert_eq!(first_run.signals, Part::Missing);
        assert!(!first_run.ready());
    }

    #[test]
    fn a_config_without_steam_id_misses_it() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.toml"), "region = \"IT\"\n").unwrap();

        assert_eq!(check(dir.path()).steam_id, Part::Missing);
    }

    #[test]
    fn a_short_steam_id_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "steam_id = \"7656119800000000\"\n",
        )
        .unwrap();

        let Part::Invalid(message) = check(dir.path()).steam_id else {
            panic!("expected an invalid steam_id");
        };
        assert!(message.contains("17 digits"));
    }

    #[test]
    fn broken_signals_are_invalid_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("signals.json"), "not json").unwrap();

        let Part::Invalid(message) = check(dir.path()).signals else {
            panic!("expected invalid signals");
        };
        assert!(message.starts_with("the signals file is not valid: "));
    }

    #[test]
    fn with_config_catalog_and_signals_it_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.toml"), STEAM_ID).unwrap();
        fs::write(dir.path().join("catalog.sqlite"), "").unwrap();
        fs::write(dir.path().join("signals.json"), SIGNALS).unwrap();

        let first_run = first_run(Some(dir.path().to_path_buf()), true);

        assert!(first_run.api_key);
        assert_eq!(
            first_run.saved_steam_id,
            Some(SteamId(76_561_198_000_000_000))
        );
        assert!(first_run.ready());
    }

    #[test]
    fn an_interrupted_update_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check(dir.path()).pending_update, None);
        archive(dir.path())
            .start_update(
                wall_clock(),
                24,
                0.0,
                gamepalate_core::region::Region::ITALY,
            )
            .unwrap();

        let first_run = check(dir.path());

        assert_eq!(
            first_run.pending_update.map(|pending| pending.responses),
            Some(0)
        );
        assert_eq!(first_run.resume_within_hours, 24);
    }

    #[test]
    fn without_a_data_folder_nothing_is_there() {
        let first_run = first_run(None, false);

        assert_eq!(first_run.data_dir, None);
        assert!(!first_run.ready());
        assert_eq!(first_run.gui_settings(), GuiSettings::default());
    }

    #[test]
    fn the_theme_is_saved_in_a_folder_that_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let first_run = check(&dir.path().join("gamepalate"));

        first_run
            .change_gui_settings(|settings| settings.theme = ThemeChoice::Dark)
            .unwrap();

        assert_eq!(first_run.gui_settings().theme, ThemeChoice::Dark);
    }
}
