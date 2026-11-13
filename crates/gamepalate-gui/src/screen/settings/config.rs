use std::time::{Duration, Instant};

use gamepalate_app::{
    Config, ConfigError, ConfigValue,
    settings::{ConfigKey, change_config, reset_config},
    setup::error_chain,
};
use iced::{
    Center, Element, Task,
    widget::{column, radio, row, text, text_input},
};

use super::{Action, Fields, Message, Settings, link, setting, tip};
use crate::{font, format, style};

pub(super) const TASTE: [ConfigKey; 8] = [
    ConfigKey::WishlistHours,
    ConfigKey::WishlistCap,
    ConfigKey::FollowedHours,
    ConfigKey::WouldTryHours,
    ConfigKey::ElsewhereHours,
    ConfigKey::MinHours,
    ConfigKey::MaxHours,
    ConfigKey::RecentHoursMultiplier,
];

pub(super) const DISCOVER: [ConfigKey; 7] = [
    ConfigKey::Count,
    ConfigKey::EarlyAccess,
    ConfigKey::MinReviews,
    ConfigKey::MinPositivePercent,
    ConfigKey::BetEvery,
    ConfigKey::QualityWeight,
    ConfigKey::VotesWeight,
];

pub(super) const CORRECTIONS: [ConfigKey; 2] = [ConfigKey::Boost, ConfigKey::Dampen];
pub(super) const CONFIG_NOTE: &str =
    "Changes made in the file apply after the next update or restart";
const NUMBER_WIDTH: f32 = 96.0;
const SAVE_AFTER: Duration = Duration::from_secs(1);
const SAVED_FOR: Duration = Duration::from_secs(2);

#[derive(Debug, Default)]
pub(super) struct Value {
    pub(super) text: String,
    pub(super) changed_at: Option<Instant>,
    pub(super) saving: bool,
    pub(super) saved_at: Option<Instant>,
    pub(super) error: Option<String>,
}

impl Value {
    pub(super) fn typed(&mut self, text: String, now: Instant) {
        self.text = text;
        self.changed_at = Some(now);
        self.saved_at = None;
        self.error = None;
    }
}

fn config_label(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::WishlistHours => "Wishlist",
        ConfigKey::WishlistCap => "Wishlist cap",
        ConfigKey::FollowedHours => "Followed",
        ConfigKey::WouldTryHours => "Would try",
        ConfigKey::ElsewhereHours => "Played elsewhere",
        ConfigKey::MinHours => "Fewest hours",
        ConfigKey::MaxHours => "Most hours",
        ConfigKey::RecentHoursMultiplier => "Recent play",
        ConfigKey::Count => "Games at a time",
        ConfigKey::EarlyAccess => "Early access",
        ConfigKey::MinReviews => "Fewest reviews",
        ConfigKey::MinPositivePercent => "Lowest score",
        ConfigKey::BetEvery => "Bets",
        ConfigKey::QualityWeight => "Quality",
        ConfigKey::VotesWeight => "Your votes",
        ConfigKey::Boost => "Count more",
        ConfigKey::Dampen => "Count less",
    }
}

fn config_note(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::WishlistHours => "Each game in your wishlist counts as this much play",
        ConfigKey::WishlistCap => {
            "Your wishlist never weighs more than this many times the games you played, 0 turns it off"
        }
        ConfigKey::FollowedHours => "Each game you follow counts as this much play",
        ConfigKey::WouldTryHours => "Each Would try vote counts as this much play",
        ConfigKey::ElsewhereHours => "Each game you played outside Steam counts as this much play",
        ConfigKey::MinHours => "Games played less are left out",
        ConfigKey::MaxHours => "Play past this counts no more",
        ConfigKey::RecentHoursMultiplier => "Hours from the last two weeks count this many times",
        ConfigKey::Count => "How many games Discover brings at once",
        ConfigKey::EarlyAccess | ConfigKey::MinReviews | ConfigKey::MinPositivePercent => {
            "What the Discover filter starts from"
        }
        ConfigKey::BetEvery => {
            "One game in this many is one your votes would rank low, 0 turns them off"
        }
        ConfigKey::QualityWeight => "How much review scores count next to your taste, from 0 to 1",
        ConfigKey::VotesWeight => "How much your votes move the order, 0 turns it off",
        ConfigKey::Boost => "Used by gamepalate correct --boost",
        ConfigKey::Dampen => "Used by Half in Library and by gamepalate correct --dampen",
    }
}

fn unit(key: ConfigKey) -> Option<&'static str> {
    match key {
        ConfigKey::WishlistHours
        | ConfigKey::FollowedHours
        | ConfigKey::WouldTryHours
        | ConfigKey::ElsewhereHours
        | ConfigKey::MinHours
        | ConfigKey::MaxHours => Some("hours"),
        ConfigKey::WishlistCap
        | ConfigKey::RecentHoursMultiplier
        | ConfigKey::Boost
        | ConfigKey::Dampen => Some("times"),
        ConfigKey::Count => Some("games"),
        ConfigKey::MinReviews => Some("reviews"),
        ConfigKey::MinPositivePercent => Some("%"),
        ConfigKey::EarlyAccess
        | ConfigKey::BetEvery
        | ConfigKey::QualityWeight
        | ConfigKey::VotesWeight => None,
    }
}

fn value_text(key: ConfigKey, value: ConfigValue) -> String {
    match (key, value) {
        (_, ConfigValue::Flag(true)) => "Shown".to_string(),
        (_, ConfigValue::Flag(false)) => "Hidden".to_string(),
        (ConfigKey::BetEvery, ConfigValue::Integer(0)) => "Off".to_string(),
        (ConfigKey::WishlistCap, ConfigValue::Number(0.0)) => "Off".to_string(),
        (ConfigKey::BetEvery, ConfigValue::Integer(every)) => format!("1 in {every}"),
        (ConfigKey::MinReviews, ConfigValue::Integer(reviews)) => {
            format!(
                "{} reviews",
                format::count(u32::try_from(reviews).unwrap_or(u32::MAX))
            )
        }
        (ConfigKey::MinPositivePercent, ConfigValue::Integer(percent)) => format!("{percent}%"),
        (ConfigKey::Count, ConfigValue::Integer(count)) => format!("{count} games"),
        (_, ConfigValue::Integer(integer)) => integer.to_string(),
        (
            ConfigKey::RecentHoursMultiplier
            | ConfigKey::WishlistCap
            | ConfigKey::Boost
            | ConfigKey::Dampen,
            ConfigValue::Number(times),
        ) => format!("×{times}"),
        (
            ConfigKey::WishlistHours
            | ConfigKey::FollowedHours
            | ConfigKey::WouldTryHours
            | ConfigKey::ElsewhereHours
            | ConfigKey::MinHours
            | ConfigKey::MaxHours,
            ConfigValue::Number(1.0),
        ) => "1 hour".to_string(),
        (
            ConfigKey::WishlistHours
            | ConfigKey::FollowedHours
            | ConfigKey::WouldTryHours
            | ConfigKey::ElsewhereHours
            | ConfigKey::MinHours
            | ConfigKey::MaxHours,
            ConfigValue::Number(hours),
        ) => format!("{hours} hours"),
        (_, ConfigValue::Number(number)) => number.to_string(),
    }
}

pub(super) fn value_input(value: ConfigValue) -> String {
    match value {
        ConfigValue::Number(number) => number.to_string(),
        ConfigValue::Integer(integer) => integer.to_string(),
        ConfigValue::Flag(flag) => flag.to_string(),
    }
}

pub(super) fn parse_value(key: ConfigKey, text: &str) -> Option<ConfigValue> {
    let text = text.trim();
    match key.default_value() {
        ConfigValue::Number(_) => text
            .replace(',', ".")
            .parse()
            .ok()
            .filter(|number: &f64| number.is_finite())
            .map(ConfigValue::Number),
        ConfigValue::Integer(_) => text.replace(',', "").parse().ok().map(ConfigValue::Integer),
        ConfigValue::Flag(_) => None,
    }
}

fn config_change(key: ConfigKey, text: &str, config: &Config) -> Option<ConfigValue> {
    parse_value(key, text).filter(|value| *value != key.value(config))
}

fn can_reset(key: ConfigKey, config: &Config) -> bool {
    key.value(config) != key.default_value()
}

fn input_note(key: ConfigKey, text: &str) -> Option<String> {
    if text.trim().is_empty() || parse_value(key, text).is_some() {
        return None;
    }
    let example = value_input(key.default_value());
    Some(match key.default_value() {
        ConfigValue::Integer(_) => format!("A whole number, such as {example}"),
        ConfigValue::Number(_) | ConfigValue::Flag(_) => format!("A number, such as {example}"),
    })
}

fn config_error_text(error: &ConfigError) -> String {
    match error {
        ConfigError::Invalid { expected, .. } => format!("Must be {expected}"),
        error => error_chain(error),
    }
}

impl Settings {
    pub(super) fn update_config(&mut self, message: Message) -> Action {
        match message {
            Message::ConfigInput(key, input) => {
                if let Some(value) = self.fields.value_mut(key) {
                    value.typed(input, Instant::now());
                }
                Action::None
            }
            Message::SaveConfig(key) => {
                let change = self.fields.value_mut(key).and_then(|value| {
                    value.changed_at = None;
                    config_change(key, &value.text, &self.settings.config)
                });
                match change {
                    Some(change) => self.write_config(key, Some(change)),
                    None => Action::None,
                }
            }
            Message::ResetConfig(key) => {
                if let Some(value) = self.fields.value_mut(key) {
                    value.text = value_input(key.default_value());
                    value.changed_at = None;
                }
                self.write_config(key, None)
            }
            Message::SetConfig(key, value) => {
                if key.value(&self.settings.config) == value {
                    return Action::None;
                }
                self.write_config(key, Some(value))
            }
            Message::ConfigSaved(key, result) => {
                let value = self.fields.value_mut(key);
                match (result, value) {
                    (Ok(()), Some(value)) => {
                        value.saving = false;
                        value.saved_at = Some(Instant::now());
                        Action::Reopen
                    }
                    (Ok(()), None) => Action::Reopen,
                    (Err(error), Some(value)) => {
                        value.saving = false;
                        value.error = Some(error);
                        Action::None
                    }
                    (Err(error), None) => {
                        self.error = Some(error);
                        Action::None
                    }
                }
            }
            Message::Flushed(true) => Action::Reopen,
            Message::Flushed(false) => Action::None,
            Message::Tick(now) => {
                let due = self.fields.take_due(&self.settings.config, now);
                let tasks: Vec<_> = due
                    .into_iter()
                    .map(|(key, value)| self.config_task(key, Some(value)))
                    .collect();
                Action::Run(Task::batch(tasks))
            }
            _ => Action::None,
        }
    }

    fn write_config(&mut self, key: ConfigKey, value: Option<ConfigValue>) -> Action {
        if let Some(shown) = self.fields.value_mut(key) {
            shown.saving = true;
            shown.error = None;
            shown.saved_at = None;
        }
        self.error = None;

        Action::Run(self.config_task(key, value))
    }

    fn config_task(&self, key: ConfigKey, value: Option<ConfigValue>) -> Task<Message> {
        let dir = self.settings.data_dir.clone();

        Task::perform(
            async move {
                match value {
                    Some(value) => change_config(&dir, key, value),
                    None => reset_config(&dir, key),
                }
                .map_err(|error| config_error_text(&error))
            },
            move |result| Message::ConfigSaved(key, result),
        )
    }

    pub(super) fn config_lines(&self, keys: &[ConfigKey]) -> Element<'_, Message> {
        column(keys.iter().map(|&key| self.config_line(key)))
            .spacing(12)
            .into()
    }

    fn config_line(&self, key: ConfigKey) -> Element<'_, Message> {
        let current = key.value(&self.settings.config);
        let note = text(config_note(key))
            .size(font::NOTE_SIZE)
            .style(style::muted);

        let Some(shown) = self.fields.value(key) else {
            let shown = match current {
                ConfigValue::Flag(shown) => Some(shown),
                ConfigValue::Number(_) | ConfigValue::Integer(_) => None,
            };
            let choice = |label, flag| {
                radio(label, flag, shown, move |flag| {
                    Message::SetConfig(key, ConfigValue::Flag(flag))
                })
                .size(16)
                .text_size(font::BAR_SIZE)
            };
            return setting(
                config_label(key),
                column![
                    row![choice("Shown", true), choice("Hidden", false)].spacing(24),
                    note,
                ]
                .spacing(4)
                .into(),
                None,
            );
        };

        let problem = shown.error.clone().or_else(|| input_note(key, &shown.text));
        let input = text_input("", &shown.text)
            .style(if problem.is_some() {
                style::wrong_field
            } else {
                style::field
            })
            .on_input(move |input| Message::ConfigInput(key, input))
            .on_submit(Message::SaveConfig(key))
            .width(NUMBER_WIDTH)
            .padding(8)
            .size(font::BAR_SIZE);
        let reset = can_reset(key, &self.settings.config).then(|| {
            tip(
                link("Reset", Message::ResetConfig(key)),
                format!("Back to {}", value_text(key, key.default_value())),
            )
        });
        let field = row![input]
            .push(unit(key).map(|unit| text(unit).size(font::BAR_SIZE)))
            .push(reset)
            .spacing(12)
            .align_y(Center);
        let status = match problem {
            Some(problem) => Some(text(problem).size(font::NOTE_SIZE).style(text::danger)),
            None => shown
                .saved_at
                .map(|_| text("Saved").size(font::NOTE_SIZE).style(style::muted)),
        };

        setting(
            config_label(key),
            column![field].push(status).push(note).spacing(4).into(),
            None,
        )
    }
}

impl Fields {
    fn value(&self, key: ConfigKey) -> Option<&Value> {
        self.values
            .iter()
            .find(|(shown, _)| *shown == key)
            .map(|(_, value)| value)
    }

    fn value_mut(&mut self, key: ConfigKey) -> Option<&mut Value> {
        self.values
            .iter_mut()
            .find(|(shown, _)| *shown == key)
            .map(|(_, value)| value)
    }

    pub(super) fn waiting(&self) -> bool {
        self.values
            .iter()
            .any(|(_, value)| value.changed_at.is_some() || value.saved_at.is_some())
    }

    fn take_due(&mut self, config: &Config, now: Instant) -> Vec<(ConfigKey, ConfigValue)> {
        let mut due = Vec::new();
        for (key, value) in &mut self.values {
            if value
                .saved_at
                .is_some_and(|at| now.saturating_duration_since(at) >= SAVED_FOR)
            {
                value.saved_at = None;
            }
            if value.saving
                || !value
                    .changed_at
                    .is_some_and(|at| now.saturating_duration_since(at) >= SAVE_AFTER)
            {
                continue;
            }
            value.changed_at = None;
            if let Some(change) = config_change(*key, &value.text, config) {
                value.saving = true;
                value.saved_at = None;
                due.push((*key, change));
            }
        }
        due
    }

    pub(super) fn pending(&self, config: &Config) -> Vec<(ConfigKey, ConfigValue)> {
        self.values
            .iter()
            .filter(|(_, value)| value.changed_at.is_some() && !value.saving)
            .filter_map(|(key, value)| {
                config_change(*key, &value.text, config).map(|change| (*key, change))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Field, account::Account, fixtures::*};
    use super::*;

    fn fields() -> Fields {
        Fields {
            values: vec![
                (
                    ConfigKey::WouldTryHours,
                    Value {
                        text: "5".to_string(),
                        ..Value::default()
                    },
                ),
                (
                    ConfigKey::QualityWeight,
                    Value {
                        text: "0.75".to_string(),
                        ..Value::default()
                    },
                ),
            ],
            steam_id: Account::showing(ID.0.to_string()),
            key: Account::default(),
            region: Account::showing("IT".to_string()),
        }
    }

    pub(super) fn typed(fields: &mut Fields, key: ConfigKey, text: &str, at: Instant) {
        fields.value_mut(key).unwrap().typed(text.to_string(), at);
    }

    #[test]
    fn the_hours_of_a_game_played_elsewhere_are_with_the_other_fixed_hours() {
        let at = |key| TASTE.iter().position(|&shown| shown == key);

        assert_eq!(
            at(ConfigKey::ElsewhereHours),
            Some(at(ConfigKey::WouldTryHours).unwrap() + 1)
        );
        assert_eq!(config_label(ConfigKey::ElsewhereHours), "Played elsewhere");
        assert_eq!(
            value_text(ConfigKey::ElsewhereHours, ConfigValue::Number(20.0)),
            "20 hours"
        );
    }

    #[test]
    fn every_value_reads_as_it_is_used() {
        let config = Config::default();
        let shown = |key: ConfigKey| value_text(key, key.value(&config));

        assert_eq!(shown(ConfigKey::WouldTryHours), "5 hours");
        assert_eq!(shown(ConfigKey::MaxHours), "200 hours");
        assert_eq!(shown(ConfigKey::RecentHoursMultiplier), "×3");
        assert_eq!(shown(ConfigKey::Dampen), "×0.5");
        assert_eq!(shown(ConfigKey::WishlistCap), "×1");
        assert_eq!(
            value_text(ConfigKey::WishlistCap, ConfigValue::Number(0.0)),
            "Off"
        );
        assert_eq!(shown(ConfigKey::Count), "16 games");
        assert_eq!(shown(ConfigKey::EarlyAccess), "Shown");
        assert_eq!(shown(ConfigKey::MinReviews), "1,000 reviews");
        assert_eq!(shown(ConfigKey::MinPositivePercent), "75%");
        assert_eq!(shown(ConfigKey::BetEvery), "1 in 10");
        assert_eq!(shown(ConfigKey::QualityWeight), "0.75");
        assert_eq!(
            value_text(ConfigKey::BetEvery, ConfigValue::Integer(0)),
            "Off"
        );
        assert_eq!(
            value_text(ConfigKey::MinHours, ConfigValue::Number(1.0)),
            "1 hour"
        );
    }

    #[test]
    fn each_field_has_its_unit_after_it() {
        assert_eq!(unit(ConfigKey::WishlistHours), Some("hours"));
        assert_eq!(unit(ConfigKey::MinPositivePercent), Some("%"));
        assert_eq!(unit(ConfigKey::QualityWeight), None);
        assert_eq!(unit(ConfigKey::Dampen), Some("times"));
    }

    #[test]
    fn the_field_starts_from_the_value_of_today() {
        assert_eq!(value_input(ConfigValue::Number(5.0)), "5");
        assert_eq!(value_input(ConfigValue::Number(0.25)), "0.25");
        assert_eq!(value_input(ConfigValue::Integer(1000)), "1000");
    }

    #[test]
    fn only_a_new_number_can_be_saved() {
        let config = Config::default();

        assert_eq!(
            config_change(ConfigKey::WouldTryHours, " 3 ", &config),
            Some(ConfigValue::Number(3.0))
        );
        assert_eq!(
            config_change(ConfigKey::QualityWeight, "0,3", &config),
            Some(ConfigValue::Number(0.3))
        );
        assert_eq!(
            config_change(ConfigKey::MinReviews, "2,000", &config),
            Some(ConfigValue::Integer(2000))
        );
        assert_eq!(config_change(ConfigKey::WouldTryHours, "5", &config), None);
        assert_eq!(config_change(ConfigKey::Count, "10.5", &config), None);
        assert_eq!(
            config_change(ConfigKey::WouldTryHours, "five", &config),
            None
        );
    }

    #[test]
    fn a_new_valid_value_saves_a_second_after_the_last_key() {
        let config = Config::default();
        let start = Instant::now();
        let mut fields = fields();

        typed(&mut fields, ConfigKey::WouldTryHours, "1", start);
        assert!(fields.waiting());
        typed(
            &mut fields,
            ConfigKey::WouldTryHours,
            "15",
            start + Duration::from_millis(600),
        );

        assert!(
            fields
                .take_due(&config, start + Duration::from_millis(1200))
                .is_empty()
        );
        assert_eq!(
            fields.take_due(&config, start + Duration::from_millis(1600)),
            vec![(ConfigKey::WouldTryHours, ConfigValue::Number(15.0))]
        );
        assert!(fields.value(ConfigKey::WouldTryHours).unwrap().saving);
        assert!(
            fields
                .take_due(&config, start + Duration::from_secs(5))
                .is_empty()
        );
    }

    #[test]
    fn a_value_equal_to_the_saved_one_or_not_valid_is_not_saved() {
        let config = Config::default();
        let start = Instant::now();
        let mut fields = fields();

        typed(&mut fields, ConfigKey::WouldTryHours, "5", start);
        typed(&mut fields, ConfigKey::QualityWeight, "much", start);

        assert!(
            fields
                .take_due(&config, start + Duration::from_secs(2))
                .is_empty()
        );
        assert!(!fields.waiting());
        assert_eq!(fields.value(ConfigKey::QualityWeight).unwrap().text, "much");
    }

    #[test]
    fn saved_shows_for_a_moment() {
        let config = Config::default();
        let start = Instant::now();
        let mut fields = fields();
        fields.value_mut(ConfigKey::WouldTryHours).unwrap().saved_at = Some(start);

        fields.take_due(&config, start + Duration::from_secs(1));
        assert!(fields.waiting());
        fields.take_due(&config, start + SAVED_FOR);
        assert!(!fields.waiting());
    }

    #[test]
    fn closing_saves_the_valid_values_waiting_and_loses_the_others() {
        let config = Config::default();
        let start = Instant::now();
        let mut fields = fields();

        typed(&mut fields, ConfigKey::WouldTryHours, "8", start);
        typed(&mut fields, ConfigKey::QualityWeight, "much", start);

        assert_eq!(
            fields.pending(&config),
            vec![(ConfigKey::WouldTryHours, ConfigValue::Number(8.0))]
        );
    }

    #[test]
    fn the_steam_id_and_the_country_never_save_by_themselves() {
        let config = Config::default();
        let start = Instant::now();
        let mut fields = fields();

        fields
            .account_mut(Field::SteamId)
            .typed("76561198000000001".to_string());
        fields.account_mut(Field::Region).typed("US".to_string());

        assert!(!fields.waiting());
        assert!(
            fields
                .take_due(&config, start + Duration::from_secs(5))
                .is_empty()
        );
        assert!(fields.pending(&config).is_empty());
    }

    #[test]
    fn reset_shows_only_away_from_the_default() {
        let mut config = Config::default();
        assert!(!can_reset(ConfigKey::WouldTryHours, &config));

        config.signals.would_try_hours = 3.0;
        assert!(can_reset(ConfigKey::WouldTryHours, &config));
    }

    #[test]
    fn a_text_that_is_not_a_number_says_what_is_expected() {
        assert_eq!(
            input_note(ConfigKey::WouldTryHours, "five").as_deref(),
            Some("A number, such as 5")
        );
        assert_eq!(
            input_note(ConfigKey::Count, "10.5").as_deref(),
            Some("A whole number, such as 16")
        );
        assert_eq!(input_note(ConfigKey::Count, ""), None);
        assert_eq!(input_note(ConfigKey::Count, "30"), None);
    }

    #[test]
    fn a_value_out_of_range_says_what_it_must_be() {
        let error = ConfigError::Invalid {
            field: "relevance.quality_weight",
            expected: "a number from 0 to 1".to_string(),
        };

        assert_eq!(config_error_text(&error), "Must be a number from 0 to 1");
    }
}
