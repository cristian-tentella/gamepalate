use std::path::Path;

use gamepalate_core::search::SearchOptions;
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomically;

const HEADER: &str = "\
# Settings of the gamepalate window. gamepalate rewrites this file when they change.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiSettings {
    pub theme: ThemeChoice,
    pub check_for_new_versions: bool,
    pub search: SearchOptions,
    pub window: Option<SavedWindow>,
}

impl Default for GuiSettings {
    fn default() -> GuiSettings {
        GuiSettings {
            theme: ThemeChoice::default(),
            check_for_new_versions: true,
            search: SearchOptions::default(),
            window: None,
        }
    }
}

pub fn load_gui_settings(path: &Path) -> GuiSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_gui_settings(path: &Path, settings: &GuiSettings) -> std::io::Result<()> {
    let text = toml::to_string(settings).map_err(std::io::Error::other)?;
    write_atomically(path, &format!("{HEADER}{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_settings_are_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        let mut settings = GuiSettings {
            theme: ThemeChoice::Dark,
            check_for_new_versions: false,
            search: SearchOptions::default(),
            window: Some(SavedWindow {
                width: 1440,
                height: 900,
                maximized: true,
            }),
        };
        settings.search.family = Some("Puzzle".to_string());
        settings.search.filters.max_price_cents = Some(1500);
        settings.search.filters.tags = vec!["Metroidvania".to_string()];

        save_gui_settings(&path, &settings).unwrap();

        assert_eq!(load_gui_settings(&path), settings);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(HEADER));
        assert!(text.contains("theme = \"dark\"\n"));
        assert!(text.contains("[window]\nwidth = 1440\nheight = 900\nmaximized = true\n"));
    }

    #[test]
    fn a_missing_file_is_the_light_theme() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            load_gui_settings(&dir.path().join("gui.toml")),
            GuiSettings::default()
        );
    }

    #[test]
    fn a_file_that_is_not_valid_is_the_light_theme() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"purple\"\n[search]\nonly_bets = true").unwrap();

        assert_eq!(load_gui_settings(&path), GuiSettings::default());
    }

    #[test]
    fn a_file_with_only_the_theme_keeps_the_default_search() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"dark\"").unwrap();

        let settings = load_gui_settings(&path);

        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.search, SearchOptions::default());
        assert_eq!(settings.window, None);
        assert!(settings.check_for_new_versions);
    }

    #[test]
    fn new_versions_are_checked_unless_the_file_says_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        assert!(load_gui_settings(&path).check_for_new_versions);

        std::fs::write(
            &path,
            "theme = \"light\"
check_for_new_versions = false",
        )
        .unwrap();

        assert!(!load_gui_settings(&path).check_for_new_versions);
    }
}
