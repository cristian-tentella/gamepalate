use std::path::Path;

use gamepalate_core::{filters::FilterOptions, ids::AppId, search::SearchOptions};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

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

/// The chip Votes opens on; `all` is no chip chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VotesShow {
    #[default]
    WouldTry,
    Bets,
    NotThisGame,
    NotMyGenre,
    All,
}

/// The order of the Votes list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VotesOrder {
    #[default]
    Newest,
    Price,
    Discount,
    BestMatch,
}

/// How Votes was left: it opens the same way next time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VotesSettings {
    #[serde(deserialize_with = "or_default")]
    pub show: VotesShow,
    #[serde(deserialize_with = "or_default")]
    pub order: VotesOrder,
    pub hide_owned: bool,
    pub filters: FilterOptions,
}

impl Default for VotesSettings {
    fn default() -> VotesSettings {
        VotesSettings {
            show: VotesShow::default(),
            order: VotesOrder::default(),
            hide_owned: true,
            filters: FilterOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiSettings {
    pub theme: ThemeChoice,
    pub check_for_new_versions: bool,
    pub search: SearchOptions,
    pub window: Option<SavedWindow>,
    // The played games when the families were made from them.
    pub families_played: Option<Vec<AppId>>,
    // A section of Votes that cannot be read is the default one, and the rest of the file stays.
    #[serde(deserialize_with = "or_default")]
    pub votes: VotesSettings,
}

impl Default for GuiSettings {
    fn default() -> GuiSettings {
        GuiSettings {
            theme: ThemeChoice::default(),
            check_for_new_versions: true,
            search: SearchOptions::default(),
            window: None,
            families_played: None,
            votes: VotesSettings::default(),
        }
    }
}

// A value from a newer or a hand-edited file is the default, instead of losing the whole file.
fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = toml::Value::deserialize(deserializer)?;
    Ok(value.try_into().unwrap_or_default())
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
            families_played: Some(vec![AppId(220), AppId(730)]),
            votes: VotesSettings {
                show: VotesShow::NotThisGame,
                order: VotesOrder::BestMatch,
                hide_owned: false,
                filters: FilterOptions {
                    only_discounted: true,
                    min_positive_percent: Some(85),
                    ..FilterOptions::default()
                },
            },
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
        assert!(text.contains("families_played = [220, 730]\n"));
        assert!(text.contains("[votes]\nshow = \"not_this_game\"\norder = \"best_match\"\n"));
    }

    #[test]
    fn a_file_without_the_votes_opens_them_as_the_first_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"dark\"").unwrap();

        let votes = load_gui_settings(&path).votes;

        assert_eq!(votes, VotesSettings::default());
        assert_eq!(votes.show, VotesShow::WouldTry);
        assert_eq!(votes.order, VotesOrder::Newest);
        assert!(votes.hide_owned);
    }

    #[test]
    fn a_half_section_of_the_votes_keeps_the_defaults_for_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "[votes]\norder = \"price\"").unwrap();

        let votes = load_gui_settings(&path).votes;

        assert_eq!(votes.order, VotesOrder::Price);
        assert_eq!(votes.show, VotesShow::WouldTry);
        assert!(votes.hide_owned);
    }

    #[test]
    fn an_unknown_chip_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(
            &path,
            "theme = \"dark\"\n[votes]\nshow = \"later\"\norder = \"discount\"\nhide_owned = false",
        )
        .unwrap();

        let settings = load_gui_settings(&path);

        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.votes.show, VotesShow::WouldTry);
        assert_eq!(settings.votes.order, VotesOrder::Discount);
        assert!(!settings.votes.hide_owned);
    }

    #[test]
    fn a_votes_section_that_cannot_be_read_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"dark\"\n[votes]\nhide_owned = \"yes\"").unwrap();

        let settings = load_gui_settings(&path);

        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.votes, VotesSettings::default());
    }

    #[test]
    fn a_file_without_the_played_games_reads_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"dark\"").unwrap();

        assert_eq!(load_gui_settings(&path).families_played, None);
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
