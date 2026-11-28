use std::path::Path;

use gamepalate_core::{
    browse::{BrowseOptions, BrowseOrder, Choice},
    filters::FilterOptions,
    ids::AppId,
    search::SearchOptions,
};
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

/// Where the gallery puts the trailers among the screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrailerPlace {
    #[default]
    First,
    Spread,
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// The segment Votes opens on; `all` is every vote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VotesShow {
    #[default]
    WouldTry,
    // Called starred until 0.31.
    #[serde(alias = "starred")]
    Saved,
    // The two noes of 0.23 are one vote now.
    #[serde(alias = "not_this_game", alias = "not_my_genre")]
    NotForMe,
    All,
}

// Up to 0.24.0 the Bets were a chip; now they are a filter.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SavedShow {
    WouldTry,
    #[serde(alias = "starred")]
    Saved,
    Bets,
    #[serde(alias = "not_this_game", alias = "not_my_genre")]
    NotForMe,
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
#[serde(from = "SavedVotes")]
pub struct VotesSettings {
    pub show: VotesShow,
    pub order: VotesOrder,
    pub hide_owned: bool,
    pub only_bets: bool,
    pub filters: FilterOptions,
}

impl Default for VotesSettings {
    fn default() -> VotesSettings {
        VotesSettings {
            show: VotesShow::default(),
            order: VotesOrder::default(),
            hide_owned: true,
            only_bets: false,
            filters: FilterOptions::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct SavedVotes {
    #[serde(deserialize_with = "or_default")]
    show: Option<SavedShow>,
    #[serde(deserialize_with = "or_default")]
    order: VotesOrder,
    hide_owned: bool,
    only_bets: bool,
    filters: FilterOptions,
}

impl Default for SavedVotes {
    fn default() -> SavedVotes {
        let votes = VotesSettings::default();
        SavedVotes {
            show: None,
            order: votes.order,
            hide_owned: votes.hide_owned,
            only_bets: votes.only_bets,
            filters: votes.filters,
        }
    }
}

impl From<SavedVotes> for VotesSettings {
    fn from(saved: SavedVotes) -> VotesSettings {
        let show = match saved.show {
            None | Some(SavedShow::WouldTry) => VotesShow::WouldTry,
            Some(SavedShow::Saved) => VotesShow::Saved,
            Some(SavedShow::NotForMe) => VotesShow::NotForMe,
            Some(SavedShow::All | SavedShow::Bets) => VotesShow::All,
        };
        VotesSettings {
            show,
            order: saved.order,
            hide_owned: saved.hide_owned,
            only_bets: saved.only_bets || matches!(saved.show, Some(SavedShow::Bets)),
            filters: saved.filters,
        }
    }
}

/// How Browse was left, as the file has it. A field that cannot be read is left out, so it takes
/// the default and the others stay.
#[derive(Deserialize, Default)]
#[serde(default)]
struct SavedBrowse {
    #[serde(deserialize_with = "or_none")]
    order: Option<BrowseOrder>,
    #[serde(deserialize_with = "or_none")]
    descending: Option<bool>,
    #[serde(deserialize_with = "or_none")]
    library: Option<Choice>,
    #[serde(deserialize_with = "or_none")]
    wished: Option<Choice>,
    #[serde(deserialize_with = "or_none")]
    would_try: Option<Choice>,
    #[serde(deserialize_with = "or_none")]
    #[serde(alias = "starred")]
    saved: Option<Choice>,
    #[serde(deserialize_with = "or_none")]
    not_for_me: Option<Choice>,
    #[serde(deserialize_with = "or_none")]
    later: Option<Choice>,
    #[serde(deserialize_with = "or_none")]
    filters: Option<FilterOptions>,
}

impl From<SavedBrowse> for BrowseOptions {
    fn from(saved: SavedBrowse) -> BrowseOptions {
        let first = BrowseOptions::default();
        let order = saved.order.unwrap_or(first.order);
        BrowseOptions {
            filters: saved.filters.unwrap_or(first.filters),
            order,
            descending: saved.descending.unwrap_or(order.descending_first()),
            library: saved.library.unwrap_or(first.library),
            wished: saved.wished.unwrap_or(first.wished),
            would_try: saved.would_try.unwrap_or(first.would_try),
            saved: saved.saved.unwrap_or(first.saved),
            not_for_me: saved.not_for_me.unwrap_or(first.not_for_me),
            later: saved.later.unwrap_or(first.later),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiSettings {
    pub theme: ThemeChoice,
    pub check_for_new_versions: bool,
    #[serde(deserialize_with = "or_default")]
    pub trailers: TrailerPlace,
    pub search: SearchOptions,
    pub window: Option<SavedWindow>,
    // The played games when the families were made from them.
    pub families_played: Option<Vec<AppId>>,
    // A section of Votes that cannot be read is the default one, and the rest of the file stays.
    #[serde(deserialize_with = "or_default")]
    pub votes: VotesSettings,
    // The same for Browse, which also has a default of its own: on sale, without the Library.
    #[serde(deserialize_with = "saved_browse")]
    pub browse: BrowseOptions,
}

impl Default for GuiSettings {
    fn default() -> GuiSettings {
        GuiSettings {
            theme: ThemeChoice::default(),
            check_for_new_versions: true,
            trailers: TrailerPlace::default(),
            search: SearchOptions::default(),
            window: None,
            families_played: None,
            votes: VotesSettings::default(),
            browse: BrowseOptions::default(),
        }
    }
}

fn saved_browse<'de, D>(deserializer: D) -> Result<BrowseOptions, D::Error>
where
    D: Deserializer<'de>,
{
    let value = toml::Value::deserialize(deserializer)?;
    Ok(value
        .try_into::<SavedBrowse>()
        .map(BrowseOptions::from)
        .unwrap_or_default())
}

fn or_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = toml::Value::deserialize(deserializer)?;
    Ok(value.try_into().ok())
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
            trailers: TrailerPlace::Spread,
            search: SearchOptions::default(),
            window: Some(SavedWindow {
                width: 1440,
                height: 900,
                maximized: true,
            }),
            families_played: Some(vec![AppId(220), AppId(730)]),
            votes: VotesSettings {
                show: VotesShow::NotForMe,
                order: VotesOrder::BestMatch,
                hide_owned: false,
                only_bets: true,
                filters: FilterOptions {
                    only_discounted: true,
                    min_positive_percent: Some(85),
                    ..FilterOptions::default()
                },
            },
            browse: BrowseOptions::default(),
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
        assert!(text.contains("[votes]\nshow = \"not_for_me\"\norder = \"best_match\"\n"));
    }

    #[test]
    fn the_saved_chip_is_saved_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        let mut settings = GuiSettings::default();
        settings.votes.show = VotesShow::Saved;

        save_gui_settings(&path, &settings).unwrap();

        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("[votes]\nshow = \"saved\"\n")
        );
        assert_eq!(load_gui_settings(&path).votes.show, VotesShow::Saved);
    }

    #[test]
    fn an_old_saved_chip_opens_votes_on_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(
            &path,
            "[votes]
show = \"starred\"
order = \"price\"
",
        )
        .unwrap();

        let votes = load_gui_settings(&path).votes;

        assert_eq!(votes.show, VotesShow::Saved);
        assert_eq!(votes.order, VotesOrder::Price);
    }

    #[test]
    fn a_list_chosen_in_saved_is_read_and_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");

        for list in ["list = 2", "list = \"no_list\"", "list = -3"] {
            std::fs::write(
                &path,
                format!(
                    "[votes]
show = \"starred\"
{list}
order = \"price\"
"
                ),
            )
            .unwrap();

            let settings = load_gui_settings(&path);
            assert_eq!(settings.votes.show, VotesShow::Saved, "{list}");
            assert_eq!(settings.votes.order, VotesOrder::Price, "{list}");

            save_gui_settings(&path, &settings).unwrap();
            assert!(
                !std::fs::read_to_string(&path).unwrap().contains("list"),
                "{list}"
            );
        }
    }

    #[test]
    fn an_old_saved_choice_of_browse_is_read_as_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(
            &path,
            "[browse]
starred = \"only\"
",
        )
        .unwrap();

        assert_eq!(load_gui_settings(&path).browse.saved, Choice::Only);
    }

    #[test]
    fn the_two_old_noes_open_votes_on_not_for_me() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");

        for old in ["not_this_game", "not_my_genre"] {
            std::fs::write(
                &path,
                format!("[votes]\nshow = \"{old}\"\norder = \"price\""),
            )
            .unwrap();

            let votes = load_gui_settings(&path).votes;

            assert_eq!(votes.show, VotesShow::NotForMe, "{old}");
            assert_eq!(votes.order, VotesOrder::Price, "{old}");
        }
    }

    #[test]
    fn the_old_bets_chip_opens_all_the_votes_with_bets_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(
            &path,
            "[votes]
show = \"bets\"
order = \"price\"",
        )
        .unwrap();

        let votes = load_gui_settings(&path).votes;

        assert_eq!(votes.show, VotesShow::All);
        assert!(votes.only_bets);
        assert_eq!(votes.order, VotesOrder::Price);
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
        assert!(!votes.only_bets);
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
    fn the_browse_options_are_saved_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        let settings = GuiSettings {
            browse: BrowseOptions {
                filters: FilterOptions {
                    only_discounted: false,
                    max_price_cents: Some(2000),
                    tags: vec!["Roguelike".to_string()],
                    ..FilterOptions::default()
                },
                order: BrowseOrder::Price,
                descending: true,
                library: Choice::Any,
                wished: Choice::Only,
                would_try: Choice::Only,
                saved: Choice::Without,
                not_for_me: Choice::Any,
                later: Choice::Without,
            },
            ..GuiSettings::default()
        };

        save_gui_settings(&path, &settings).unwrap();

        assert_eq!(load_gui_settings(&path), settings);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[browse]\n"));
        assert!(text.contains("order = \"price\"\n"));
        assert!(text.contains("saved = \"hide\"\n"));
        assert!(text.contains("would_try = \"only\"\n"));
        assert!(text.contains("[browse.filters]\n"));
    }

    #[test]
    fn a_file_without_browse_opens_it_as_the_first_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"dark\"").unwrap();

        let browse = load_gui_settings(&path).browse;

        assert_eq!(browse, BrowseOptions::default());
        assert!(browse.filters.only_discounted);
        assert_eq!(browse.library, Choice::Without);
        assert_eq!(browse.not_for_me, Choice::Without);
    }

    #[test]
    fn a_wrong_value_in_browse_takes_the_default_of_that_field_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(
            &path,
            "theme = \"dark\"\n[browse]\norder = \"newest\"\ndescending = false\nlibrary = \"any\"\nwished = 3\nwould_try = \"only\"\n[browse.filters]\nmax_price_cents = \"cheap\"",
        )
        .unwrap();

        let settings = load_gui_settings(&path);
        let defaults = BrowseOptions::default();

        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.browse.order, defaults.order);
        assert!(!settings.browse.descending);
        assert_eq!(settings.browse.library, Choice::Any);
        assert_eq!(settings.browse.wished, defaults.wished);
        assert_eq!(settings.browse.would_try, Choice::Only);
        assert_eq!(settings.browse.not_for_me, defaults.not_for_me);
        assert_eq!(settings.browse.filters, defaults.filters);
    }

    #[test]
    fn a_browse_that_is_not_a_section_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "theme = \"dark\"\nbrowse = 3").unwrap();

        let settings = load_gui_settings(&path);

        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.browse, BrowseOptions::default());
    }

    #[test]
    fn a_browse_order_without_its_direction_takes_the_natural_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "[browse]\norder = \"name\"").unwrap();

        let browse = load_gui_settings(&path).browse;

        assert_eq!(browse.order, BrowseOrder::Name);
        assert!(!browse.descending);
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

    #[test]
    fn a_file_without_the_trailers_puts_them_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(
            &path,
            "theme = \"dark\"
check_for_new_versions = false",
        )
        .unwrap();

        let settings = load_gui_settings(&path);

        assert_eq!(settings.trailers, TrailerPlace::First);
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert!(!settings.check_for_new_versions);
    }

    #[test]
    fn every_place_of_the_trailers_is_saved_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        for trailers in [
            TrailerPlace::First,
            TrailerPlace::Spread,
            TrailerPlace::Last,
        ] {
            let settings = GuiSettings {
                trailers,
                ..GuiSettings::default()
            };

            save_gui_settings(&path, &settings).unwrap();

            assert_eq!(load_gui_settings(&path), settings);
        }
    }

    #[test]
    fn a_place_of_the_trailers_that_cannot_be_read_puts_them_first_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        for value in ["\"middle\"", "3"] {
            std::fs::write(
                &path,
                format!(
                    "theme = \"dark\"
trailers = {value}
[votes]
show = \"all\""
                ),
            )
            .unwrap();

            let settings = load_gui_settings(&path);

            assert_eq!(settings.trailers, TrailerPlace::First);
            assert_eq!(settings.theme, ThemeChoice::Dark);
            assert_eq!(settings.votes.show, VotesShow::All);
        }
    }
}
