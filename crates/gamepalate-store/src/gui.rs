use std::path::Path;

use serde::Deserialize;

use crate::atomic::write_atomically;

const HEADER: &str = "\
# Settings of the gamepalate window. gamepalate rewrites this file when they change.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct GuiSettings {
    #[serde(default)]
    pub theme: ThemeChoice,
}

pub fn load_gui_settings(path: &Path) -> GuiSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_gui_settings(path: &Path, settings: &GuiSettings) -> std::io::Result<()> {
    let theme = match settings.theme {
        ThemeChoice::Light => "light",
        ThemeChoice::Dark => "dark",
    };
    write_atomically(path, &format!("{HEADER}theme = \"{theme}\"\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_settings_are_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        let settings = GuiSettings {
            theme: ThemeChoice::Dark,
        };

        save_gui_settings(&path, &settings).unwrap();

        assert_eq!(load_gui_settings(&path), settings);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .ends_with("theme = \"dark\"\n")
        );
    }

    #[test]
    fn a_missing_file_is_the_light_theme() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            load_gui_settings(&dir.path().join("gui.toml")).theme,
            ThemeChoice::Light
        );
    }

    #[test]
    fn a_file_that_is_not_valid_is_the_light_theme() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"purple\"").unwrap();

        assert_eq!(load_gui_settings(&path).theme, ThemeChoice::Light);
    }
}
