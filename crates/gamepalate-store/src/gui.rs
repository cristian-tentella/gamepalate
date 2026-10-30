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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiSettings {
    pub theme: ThemeChoice,
    pub search: SearchOptions,
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
            search: SearchOptions::default(),
        };
        settings.search.family = Some("Puzzle".to_string());
        settings.search.filters.max_price_cents = Some(1500);
        settings.search.filters.tags = vec!["Metroidvania".to_string()];

        save_gui_settings(&path, &settings).unwrap();

        assert_eq!(load_gui_settings(&path), settings);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(HEADER));
        assert!(text.contains("theme = \"dark\"\n"));
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
    }
}
