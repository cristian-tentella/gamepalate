use std::{process::Command, sync::Arc};

use gamepalate_app::{
    Config, ConfigError, ConfigValue, Gamepalate, Region, SteamId, ThemeChoice, Timestamp,
    account::{Profile, connect, parse_profile},
    settings::{self, ConfigKey, KeySource, change_config, change_region, reset_config},
    setup::error_chain,
};
use iced::{
    Center, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    widget::{Id, button, column, container, operation, radio, row, scrollable, text, text_input},
};

use crate::{
    app::Page,
    font, format,
    screen::{
        setup::{
            KEY_DOMAIN_NOTE, PRIVACY_PAGE, PROFILE_PLACEHOLDER, connect_error_text, needs_privacy,
            note_link, steam_id_note,
        },
        steam_client::{ElsewhereHint, SteamClientQuestion},
    },
    style,
};

const COLUMN_WIDTH: f32 = 760.0;
const LABEL_WIDTH: f32 = 120.0;
const MEGABYTE: u64 = 1024 * 1024;
const CONFIG_NOTE: &str = "Changes made in the file apply after the next update or restart";
const CONFIG_FIELD: Id = Id::new("settings-config-field");
const TASTE: [ConfigKey; 7] = [
    ConfigKey::WishlistHours,
    ConfigKey::FollowedHours,
    ConfigKey::WouldTryHours,
    ConfigKey::ElsewhereHours,
    ConfigKey::MinHours,
    ConfigKey::MaxHours,
    ConfigKey::RecentHoursMultiplier,
];
const DISCOVER: [ConfigKey; 7] = [
    ConfigKey::Count,
    ConfigKey::EarlyAccess,
    ConfigKey::MinReviews,
    ConfigKey::MinPositivePercent,
    ConfigKey::BetEvery,
    ConfigKey::QualityWeight,
    ConfigKey::VotesWeight,
];
const CORRECTIONS: [ConfigKey; 2] = [ConfigKey::Boost, ConfigKey::Dampen];

pub struct Settings {
    gamepalate: Arc<Gamepalate>,
    settings: settings::Settings,
    back: Page,
    cache_bytes: Option<u64>,
    editing: Option<Editing>,
    confirming: Option<Region>,
    tuning: Option<Tuning>,
    question: Option<SteamClientQuestion>,
    error: Option<String>,
}

#[derive(Debug)]
struct Tuning {
    key: ConfigKey,
    text: String,
    saving: bool,
    error: Option<String>,
}

#[derive(Debug)]
struct Editing {
    field: Field,
    text: String,
    saving: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    SteamId,
    Key,
    Region,
}

#[derive(Clone, Debug)]
pub enum Message {
    Edit(Field),
    Input(String),
    Save,
    Cancel,
    Saved(Result<(), String>),
    ConfirmRegion,
    OpenPrivacy,
    RegionSaved(Result<(), String>),
    Theme(ThemeChoice),
    OpenFolder,
    OpenConfig,
    ClearCache,
    CacheMeasured(u64),
    AskSteamClient,
    SteamClient(bool),
    SteamClientSaved(Result<(), String>),
    EditConfig(ConfigKey),
    ConfigInput(String),
    SaveConfig,
    ResetConfig,
    SetConfig(ConfigKey, ConfigValue),
    ConfigSaved(Result<(), String>),
    OpenLibrary,
    Escape,
}

pub enum Action {
    None,
    Run(Task<Message>),
    Theme(ThemeChoice),
    Reopen,
    RegionChanged,
    Close(Page),
}

impl Settings {
    pub fn new(gamepalate: &Arc<Gamepalate>, back: Page) -> (Settings, Task<Message>) {
        let settings = Settings {
            gamepalate: Arc::clone(gamepalate),
            settings: gamepalate.settings(),
            back,
            cache_bytes: None,
            editing: None,
            confirming: None,
            tuning: None,
            question: None,
            error: None,
        };
        let task = settings.measure_cache();

        (settings, task)
    }

    pub fn back(&self) -> Page {
        self.back
    }

    pub fn failed(&mut self, error: String) {
        self.tuning = None;
        self.error = Some(error);
    }

    pub fn reopened(&mut self, gamepalate: &Arc<Gamepalate>) {
        self.gamepalate = Arc::clone(gamepalate);
        self.settings = gamepalate.settings();
        self.tuning = None;
        self.error = None;
    }

    pub fn subscription() -> Subscription<Message> {
        keyboard::listen().filter_map(key_message)
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Edit(field) => {
                self.confirming = None;
                self.tuning = None;
                self.editing = Some(Editing {
                    field,
                    text: String::new(),
                    saving: false,
                    error: None,
                });
                Action::None
            }
            Message::Input(input) => {
                if let Some(editing) = &mut self.editing
                    && !editing.saving
                {
                    editing.text = input;
                }
                Action::None
            }
            Message::Save => self.save(),
            Message::Cancel => {
                self.step_back();
                Action::None
            }
            Message::Saved(Ok(())) => {
                self.editing = None;
                Action::Reopen
            }
            Message::Saved(Err(error)) | Message::RegionSaved(Err(error)) => {
                self.confirming = None;
                if let Some(editing) = &mut self.editing {
                    editing.saving = false;
                    editing.error = Some(error);
                }
                Action::None
            }
            Message::ConfirmRegion => self.save_region(),
            Message::RegionSaved(Ok(())) => {
                self.editing = None;
                self.confirming = None;
                Action::RegionChanged
            }
            Message::Theme(choice) => Action::Theme(choice),
            Message::OpenFolder => {
                let _ = Command::new("explorer")
                    .arg(&self.settings.data_dir)
                    .spawn();
                Action::None
            }
            Message::OpenPrivacy => {
                let _ = Command::new("rundll32")
                    .arg("url.dll,FileProtocolHandler")
                    .arg(PRIVACY_PAGE)
                    .spawn();
                Action::None
            }
            Message::OpenConfig => {
                let _ = Command::new("rundll32")
                    .arg("url.dll,FileProtocolHandler")
                    .arg(self.settings.data_dir.join("config.toml"))
                    .spawn();
                Action::None
            }
            Message::ClearCache => {
                let gamepalate = Arc::clone(&self.gamepalate);
                self.cache_bytes = None;
                Action::Run(Task::perform(
                    async move {
                        gamepalate.clear_image_cache();
                        gamepalate.image_cache_bytes()
                    },
                    Message::CacheMeasured,
                ))
            }
            Message::CacheMeasured(bytes) => {
                self.cache_bytes = Some(bytes);
                Action::None
            }
            Message::AskSteamClient => {
                self.question = self
                    .settings
                    .steam_client_file
                    .clone()
                    .map(SteamClientQuestion::about);
                Action::None
            }
            Message::SteamClient(read) => match &mut self.question {
                Some(question) => Action::Run(
                    question
                        .answer(self.settings.data_dir.clone(), read)
                        .map(Message::SteamClientSaved),
                ),
                None => Action::None,
            },
            Message::SteamClientSaved(Ok(())) => {
                self.question = None;
                Action::Reopen
            }
            Message::SteamClientSaved(Err(error)) => {
                if let Some(question) = &mut self.question {
                    question.failed(error);
                }
                Action::None
            }
            Message::EditConfig(key) => {
                self.editing = None;
                self.confirming = None;
                self.tuning = Some(Tuning {
                    key,
                    text: value_input(key.value(&self.settings.config)),
                    saving: false,
                    error: None,
                });
                Action::Run(Task::batch([
                    operation::focus(CONFIG_FIELD),
                    operation::select_all(CONFIG_FIELD),
                ]))
            }
            Message::ConfigInput(input) => {
                if let Some(tuning) = &mut self.tuning
                    && !tuning.saving
                {
                    tuning.text = input;
                    tuning.error = None;
                }
                Action::None
            }
            Message::SaveConfig => {
                let change = self.tuning.as_ref().and_then(|tuning| {
                    config_change(tuning, &self.settings.config).map(|value| (tuning.key, value))
                });
                match change {
                    Some((key, value)) => self.write_config(key, Some(value)),
                    None => Action::None,
                }
            }
            Message::ResetConfig => match &self.tuning {
                Some(tuning) if !tuning.saving => self.write_config(tuning.key, None),
                _ => Action::None,
            },
            Message::SetConfig(key, value) => {
                if self.tuning.is_some() || key.value(&self.settings.config) == value {
                    return Action::None;
                }
                self.write_config(key, Some(value))
            }
            Message::ConfigSaved(Ok(())) => Action::Reopen,
            Message::ConfigSaved(Err(error)) => {
                match &mut self.tuning {
                    Some(tuning) => {
                        tuning.saving = false;
                        tuning.error = Some(error);
                    }
                    None => self.error = Some(error),
                }
                Action::None
            }
            Message::OpenLibrary => Action::Close(Page::Library),
            Message::Escape => {
                if self.question.take().is_some() {
                    return Action::None;
                }
                if self.tuning.is_some() {
                    self.step_back();
                    return Action::None;
                }
                if self.editing.is_none() {
                    return Action::Close(self.back);
                }
                self.step_back();
                Action::None
            }
        }
    }

    fn step_back(&mut self) {
        if let Some(tuning) = &self.tuning {
            if !tuning.saving {
                self.tuning = None;
            }
            return;
        }
        if self.editing.as_ref().is_some_and(|editing| editing.saving) {
            return;
        }
        if self.confirming.take().is_none() {
            self.editing = None;
        }
    }

    fn measure_cache(&self) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        Task::perform(
            async move { gamepalate.image_cache_bytes() },
            Message::CacheMeasured,
        )
    }

    fn save(&mut self) -> Action {
        let Some(editing) = &mut self.editing else {
            return Action::None;
        };
        if editing.field == Field::Region {
            self.confirming = region_change(editing, self.settings.region);
            return Action::None;
        }
        let Some((profile, key)) = account_change(editing, self.settings.steam_id) else {
            return Action::None;
        };
        editing.saving = true;
        editing.error = None;
        let dir = self.settings.data_dir.clone();

        Action::Run(Task::perform(
            async move {
                connect(&dir, profile, key)
                    .await
                    .map_err(|error| connect_error_text(&error))
            },
            Message::Saved,
        ))
    }

    fn write_config(&mut self, key: ConfigKey, value: Option<ConfigValue>) -> Action {
        if let Some(tuning) = &mut self.tuning {
            tuning.saving = true;
            tuning.error = None;
        }
        self.error = None;
        let dir = self.settings.data_dir.clone();

        Action::Run(Task::perform(
            async move {
                match value {
                    Some(value) => change_config(&dir, key, value),
                    None => reset_config(&dir, key),
                }
                .map_err(|error| config_error_text(&error))
            },
            Message::ConfigSaved,
        ))
    }

    fn save_region(&mut self) -> Action {
        let (Some(editing), Some(region)) = (&mut self.editing, self.confirming) else {
            return Action::None;
        };
        editing.saving = true;
        editing.error = None;
        let dir = self.settings.data_dir.clone();

        Action::Run(Task::perform(
            async move { change_region(&dir, region).map_err(|error| error_chain(&error)) },
            Message::RegionSaved,
        ))
    }

    pub fn view(&self, theme: ThemeChoice) -> Element<'_, Message> {
        let account = group(
            "Account",
            column![self.line(Field::SteamId), self.line(Field::Key),].spacing(12),
        );
        let country = group("Country", self.line(Field::Region));
        let appearance = group(
            "Appearance",
            setting(
                "Theme",
                row![
                    radio("Light", ThemeChoice::Light, Some(theme), Message::Theme)
                        .size(16)
                        .text_size(font::BAR_SIZE),
                    radio("Dark", ThemeChoice::Dark, Some(theme), Message::Theme)
                        .size(16)
                        .text_size(font::BAR_SIZE),
                ]
                .spacing(24)
                .into(),
                None,
            ),
        );
        let taste = group("Taste", self.config_lines(&TASTE));
        let discover = group("Discover", self.config_lines(&DISCOVER));
        let corrections = group("Corrections", self.config_lines(&CORRECTIONS));
        let data = group(
            "Data",
            column![
                setting(
                    "Folder",
                    value(self.settings.data_dir.display().to_string()),
                    Some(link("Open folder", Message::OpenFolder)),
                ),
                setting(
                    "Image cache",
                    value(cache_text(
                        self.cache_bytes,
                        self.settings.image_cache_max_bytes
                    )),
                    self.cache_bytes
                        .is_some_and(|bytes| bytes > 0)
                        .then(|| link("Clear", Message::ClearCache)),
                ),
                self.steam_client_line(),
                setting(
                    "Catalog",
                    value(catalog_text(
                        self.settings.catalog_games,
                        &format::when(
                            self.settings.tags_updated_at,
                            Timestamp(jiff::Timestamp::now().as_millisecond()),
                        ),
                    )),
                    None,
                ),
                setting(
                    "Advanced",
                    column![
                        link("Open config.toml", Message::OpenConfig),
                        text(CONFIG_NOTE).size(font::NOTE_SIZE).style(style::muted),
                    ]
                    .spacing(4)
                    .into(),
                    None,
                ),
            ]
            .spacing(12),
        );
        let error = self
            .error
            .as_ref()
            .map(|error| text(error.clone()).size(font::BAR_SIZE).style(text::danger));

        scrollable(
            container(
                column![]
                    .push(error)
                    .push(account)
                    .push(country)
                    .push(appearance)
                    .push(taste)
                    .push(discover)
                    .push(corrections)
                    .push(data)
                    .spacing(32)
                    .width(Fill)
                    .max_width(COLUMN_WIDTH),
            )
            .padding(24)
            .center_x(Fill),
        )
        .height(Fill)
        .into()
    }

    fn config_lines(&self, keys: &[ConfigKey]) -> Element<'_, Message> {
        column(keys.iter().map(|&key| self.config_line(key)))
            .spacing(12)
            .into()
    }

    fn config_line(&self, key: ConfigKey) -> Element<'_, Message> {
        let current = key.value(&self.settings.config);
        let note = text(config_note(key))
            .size(font::NOTE_SIZE)
            .style(style::muted);

        if let ConfigValue::Flag(shown) = current {
            let choice = |label, flag| {
                radio(label, flag, Some(shown), move |flag| {
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
        }

        let Some(tuning) = self.tuning.as_ref().filter(|tuning| tuning.key == key) else {
            let open = self.editing.is_none() && self.tuning.is_none();
            return setting(
                config_label(key),
                column![value(value_text(key, current)), note]
                    .spacing(2)
                    .into(),
                open.then(|| link("Change", Message::EditConfig(key))),
            );
        };

        let input = text_input("", &tuning.text)
            .id(CONFIG_FIELD)
            .style(style::field)
            .on_input_maybe((!tuning.saving).then_some(Message::ConfigInput))
            .on_submit(Message::SaveConfig)
            .padding(8)
            .size(font::BAR_SIZE);
        let can_save = !tuning.saving && config_change(tuning, &self.settings.config).is_some();
        let default = key.default_value();
        let reset = (!tuning.saving && current != default).then(|| {
            link_owned(
                format!("Reset to {}", value_text(key, default)),
                Message::ResetConfig,
            )
        });
        let buttons = row![
            button(text(if tuning.saving { "Saving…" } else { "Save" }).size(font::BAR_SIZE))
                .style(style::primary)
                .padding([8, 20])
                .on_press_maybe(can_save.then_some(Message::SaveConfig)),
        ]
        .push((!tuning.saving).then(|| link("Cancel", Message::Cancel)))
        .push(reset)
        .spacing(16)
        .align_y(Center);
        let message = match &tuning.error {
            Some(error) => text(error.clone())
                .size(font::NOTE_SIZE)
                .style(text::danger),
            None => match input_note(key, &tuning.text) {
                Some(problem) => text(problem).size(font::NOTE_SIZE).style(text::danger),
                None => note,
            },
        };

        setting(
            config_label(key),
            column![input, message, buttons].spacing(8).into(),
            None,
        )
    }

    fn steam_client_line(&self) -> Element<'_, Message> {
        if let Some(question) = &self.question {
            return setting(
                "Steam app",
                question.view(
                    Message::SteamClient,
                    ElsewhereHint::Link(Message::OpenLibrary),
                ),
                None,
            );
        }
        if self.settings.steam_client_file.is_none() {
            return setting("Steam app", value("Not found on this PC".to_string()), None);
        }
        setting(
            "Steam app",
            value(steam_client_text(self.settings.steam_client_hours).to_string()),
            Some(link("Change", Message::AskSteamClient)),
        )
    }

    fn line(&self, field: Field) -> Element<'_, Message> {
        let label = match field {
            Field::SteamId => "Steam ID",
            Field::Key => "API key",
            Field::Region => "Prices from",
        };
        let shown = match field {
            Field::SteamId => self
                .settings
                .steam_id
                .map(|id| id.0.to_string())
                .unwrap_or_else(|| "Missing".to_string()),
            Field::Key => key_text(self.settings.key).to_string(),
            Field::Region => self.settings.region.to_string(),
        };

        let Some(editing) = self
            .editing
            .as_ref()
            .filter(|editing| editing.field == field)
        else {
            let action = match field {
                Field::SteamId | Field::Region => Some("Change"),
                Field::Key => (self.settings.key != KeySource::Environment).then_some("Replace"),
            };
            let action = action.filter(|_| self.editing.is_none());
            return setting(
                label,
                value(shown),
                action.map(|action| link(action, Message::Edit(field))),
            );
        };

        if let Some(region) = self.confirming.filter(|_| field == Field::Region) {
            return setting(label, self.confirmation(region, editing), None);
        }

        let placeholder = match field {
            Field::SteamId => PROFILE_PLACEHOLDER,
            Field::Key => "Steam Web API key",
            Field::Region => "Two letters, such as IT",
        };
        let input = text_input(placeholder, &editing.text)
            .style(style::field)
            .secure(field == Field::Key)
            .on_input_maybe((!editing.saving).then_some(Message::Input))
            .on_submit(Message::Save)
            .padding(8)
            .size(font::BAR_SIZE);
        let can_save = !editing.saving
            && match field {
                Field::Region => region_change(editing, self.settings.region).is_some(),
                Field::SteamId | Field::Key => {
                    account_change(editing, self.settings.steam_id).is_some()
                }
            };
        let buttons = row![
            button(
                text(if editing.saving {
                    "Checking with Steam…"
                } else {
                    "Save"
                })
                .size(font::BAR_SIZE)
            )
            .style(style::primary)
            .padding([8, 20])
            .on_press_maybe(can_save.then_some(Message::Save)),
        ]
        .push((!editing.saving).then(|| link("Cancel", Message::Cancel)))
        .spacing(16)
        .align_y(Center);
        let note = match (field, &editing.error) {
            (_, Some(error)) => Some(
                text(error.clone())
                    .size(font::NOTE_SIZE)
                    .style(text::danger),
            ),
            (Field::SteamId, None) => steam_id_note(&editing.text)
                .map(|note| text(note).size(font::NOTE_SIZE).style(text::danger)),
            (Field::Region, None) => region_note(&editing.text)
                .map(|note| text(note).size(font::NOTE_SIZE).style(text::danger)),
            (Field::Key, None) => Some(
                text(KEY_DOMAIN_NOTE)
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ),
        };
        let privacy = editing
            .error
            .as_deref()
            .filter(|error| needs_privacy(error))
            .map(|_| note_link("Open privacy settings ↗", Message::OpenPrivacy));

        setting(
            label,
            column![input]
                .push(note)
                .push(privacy)
                .push(buttons)
                .spacing(8)
                .into(),
            None,
        )
    }

    fn confirmation<'a>(&self, region: Region, editing: &Editing) -> Element<'a, Message> {
        let buttons = row![
            button(
                text(if editing.saving {
                    "Saving…"
                } else {
                    "Change and update"
                })
                .size(font::BAR_SIZE)
            )
            .style(style::primary)
            .padding([8, 20])
            .on_press_maybe((!editing.saving).then_some(Message::ConfirmRegion)),
        ]
        .push((!editing.saving).then(|| link("Cancel", Message::Cancel)))
        .spacing(16)
        .align_y(Center);

        column![
            text(region_confirmation(region)).size(font::BAR_SIZE),
            buttons,
        ]
        .spacing(8)
        .into()
    }
}

fn parse_region(text: &str) -> Option<Region> {
    Region::new(&text.trim().to_ascii_uppercase())
}

fn region_change(editing: &Editing, saved: Region) -> Option<Region> {
    parse_region(&editing.text).filter(|region| *region != saved)
}

fn config_label(key: ConfigKey) -> &'static str {
    match key {
        ConfigKey::WishlistHours => "Wishlist",
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
            "One game in this many is one your votes would skip, 0 turns them off"
        }
        ConfigKey::QualityWeight => "How much review scores count next to your taste, from 0 to 1",
        ConfigKey::VotesWeight => "How much your votes move the order, 0 turns it off",
        ConfigKey::Boost => "Used by gamepalate correct --boost",
        ConfigKey::Dampen => "Used by Half in Library and by gamepalate correct --dampen",
    }
}

fn value_text(key: ConfigKey, value: ConfigValue) -> String {
    match (key, value) {
        (_, ConfigValue::Flag(true)) => "Shown".to_string(),
        (_, ConfigValue::Flag(false)) => "Hidden".to_string(),
        (ConfigKey::BetEvery, ConfigValue::Integer(0)) => "Off".to_string(),
        (ConfigKey::BetEvery, ConfigValue::Integer(every)) => format!("1 in {every}"),
        (ConfigKey::MinReviews, ConfigValue::Integer(reviews)) => {
            format::count(u32::try_from(reviews).unwrap_or(u32::MAX))
        }
        (ConfigKey::MinPositivePercent, ConfigValue::Integer(percent)) => format!("{percent}%"),
        (_, ConfigValue::Integer(integer)) => integer.to_string(),
        (
            ConfigKey::RecentHoursMultiplier | ConfigKey::Boost | ConfigKey::Dampen,
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

fn value_input(value: ConfigValue) -> String {
    match value {
        ConfigValue::Number(number) => number.to_string(),
        ConfigValue::Integer(integer) => integer.to_string(),
        ConfigValue::Flag(flag) => flag.to_string(),
    }
}

fn parse_value(key: ConfigKey, text: &str) -> Option<ConfigValue> {
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

fn config_change(tuning: &Tuning, config: &Config) -> Option<ConfigValue> {
    parse_value(tuning.key, &tuning.text).filter(|value| *value != tuning.key.value(config))
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

fn region_note(text: &str) -> Option<&'static str> {
    (!text.trim().is_empty() && parse_region(text).is_none()).then_some("Two letters, such as IT")
}

fn region_confirmation(region: Region) -> String {
    format!(
        "Prices in {region} need a full update, about 20 minutes like the first one. You can keep using gamepalate meanwhile."
    )
}

fn account_change(editing: &Editing, saved: Option<SteamId>) -> Option<(Profile, Option<String>)> {
    match editing.field {
        Field::SteamId => parse_profile(&editing.text)
            .ok()
            .map(|profile| (profile, None)),
        Field::Region => None,
        Field::Key => {
            let key = editing.text.trim();
            (!key.is_empty())
                .then_some(saved)
                .flatten()
                .map(|steam_id| (Profile::Id(steam_id), Some(key.to_string())))
        }
    }
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    match event {
        keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        } => Some(Message::Escape),
        _ => None,
    }
}

fn steam_client_text(answer: Option<bool>) -> &'static str {
    match answer {
        Some(true) => "Reads the hours of games you don't own",
        Some(false) | None => "Doesn't read the hours of games you don't own",
    }
}

fn key_text(key: KeySource) -> &'static str {
    match key {
        KeySource::Saved if cfg!(windows) => "Saved in Windows",
        KeySource::Saved => "Saved in the system keyring",
        KeySource::Environment => "From STEAM_API_KEY",
        KeySource::Missing => "Missing",
    }
}

fn cache_text(bytes: Option<u64>, max: u64) -> String {
    match bytes {
        None => "Measuring…".to_string(),
        Some(bytes) => format!("{} MB of {} MB", bytes.div_ceil(MEGABYTE), max / MEGABYTE),
    }
}

fn group<'a>(title: &'a str, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![
        text(title).size(font::PRICE_SIZE).font(font::SEMIBOLD),
        content.into(),
    ]
    .spacing(14)
    .into()
}

fn setting<'a>(
    label: &'a str,
    content: Element<'a, Message>,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        container(text(label).size(font::BAR_SIZE).style(style::muted)).width(LABEL_WIDTH),
        container(content).width(Fill),
    ]
    .push(action)
    .spacing(16)
    .align_y(Center)
    .into()
}

fn catalog_text(games: usize, tags_updated: &str) -> String {
    format!(
        "{} games · tags {tags_updated}",
        format::count(games as u32)
    )
}

fn value<'a>(shown: String) -> Element<'a, Message> {
    text(shown).size(font::BAR_SIZE).into()
}

fn link_owned<'a>(label: String, message: Message) -> Element<'a, Message> {
    button(text(label).size(font::BAR_SIZE))
        .padding(0)
        .style(style::link)
        .on_press(message)
        .into()
}

fn link(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label).size(font::BAR_SIZE))
        .padding(0)
        .style(style::link)
        .on_press(message)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: SteamId = SteamId(76_561_198_000_000_000);

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

    fn editing(field: Field, text: &str) -> Editing {
        Editing {
            field,
            text: text.to_string(),
            saving: false,
            error: None,
        }
    }

    #[test]
    fn the_catalog_says_how_many_games_and_when_its_tags_were_read() {
        assert_eq!(
            catalog_text(115_668, "28 Oct"),
            "115,668 games · tags 28 Oct"
        );
    }

    fn tuning(key: ConfigKey, text: &str) -> Tuning {
        Tuning {
            key,
            text: text.to_string(),
            saving: false,
            error: None,
        }
    }

    #[test]
    fn every_value_reads_as_it_is_used() {
        let config = Config::default();
        let shown = |key: ConfigKey| value_text(key, key.value(&config));

        assert_eq!(shown(ConfigKey::WouldTryHours), "5 hours");
        assert_eq!(shown(ConfigKey::MaxHours), "200 hours");
        assert_eq!(shown(ConfigKey::RecentHoursMultiplier), "×3");
        assert_eq!(shown(ConfigKey::Dampen), "×0.5");
        assert_eq!(shown(ConfigKey::Count), "20");
        assert_eq!(shown(ConfigKey::EarlyAccess), "Shown");
        assert_eq!(shown(ConfigKey::MinReviews), "1,000");
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
    fn the_field_starts_from_the_value_of_today() {
        assert_eq!(value_input(ConfigValue::Number(5.0)), "5");
        assert_eq!(value_input(ConfigValue::Number(0.25)), "0.25");
        assert_eq!(value_input(ConfigValue::Integer(1000)), "1000");
    }

    #[test]
    fn only_a_new_number_can_be_saved() {
        let config = Config::default();

        assert_eq!(
            config_change(&tuning(ConfigKey::WouldTryHours, " 3 "), &config),
            Some(ConfigValue::Number(3.0))
        );
        assert_eq!(
            config_change(&tuning(ConfigKey::QualityWeight, "0,3"), &config),
            Some(ConfigValue::Number(0.3))
        );
        assert_eq!(
            config_change(&tuning(ConfigKey::MinReviews, "2,000"), &config),
            Some(ConfigValue::Integer(2000))
        );
        assert_eq!(
            config_change(&tuning(ConfigKey::WouldTryHours, "5"), &config),
            None
        );
        assert_eq!(
            config_change(&tuning(ConfigKey::Count, "10.5"), &config),
            None
        );
        assert_eq!(
            config_change(&tuning(ConfigKey::WouldTryHours, "five"), &config),
            None
        );
    }

    #[test]
    fn a_text_that_is_not_a_number_says_what_is_expected() {
        assert_eq!(
            input_note(ConfigKey::WouldTryHours, "five").as_deref(),
            Some("A number, such as 5")
        );
        assert_eq!(
            input_note(ConfigKey::Count, "10.5").as_deref(),
            Some("A whole number, such as 20")
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

    #[cfg(windows)]
    #[test]
    fn the_saved_key_is_in_windows() {
        assert_eq!(key_text(KeySource::Saved), "Saved in Windows");
    }

    #[cfg(not(windows))]
    #[test]
    fn the_saved_key_is_in_the_system_keyring() {
        assert_eq!(key_text(KeySource::Saved), "Saved in the system keyring");
    }

    #[test]
    fn each_key_source_has_its_text() {
        assert_eq!(key_text(KeySource::Environment), "From STEAM_API_KEY");
        assert_eq!(key_text(KeySource::Missing), "Missing");
    }

    #[test]
    fn the_cache_is_in_megabytes_of_the_maximum() {
        assert_eq!(cache_text(None, 300 * MEGABYTE), "Measuring…");
        assert_eq!(cache_text(Some(0), 300 * MEGABYTE), "0 MB of 300 MB");
        assert_eq!(
            cache_text(Some(212 * MEGABYTE - 10), 300 * MEGABYTE),
            "212 MB of 300 MB"
        );
    }

    #[test]
    fn a_new_steam_id_keeps_the_saved_key() {
        assert_eq!(
            account_change(
                &editing(
                    Field::SteamId,
                    "https://steamcommunity.com/profiles/76561198000000000"
                ),
                None
            ),
            Some((Profile::Id(ID), None))
        );
        assert_eq!(
            account_change(
                &editing(Field::SteamId, "https://steamcommunity.com/id/someone"),
                Some(ID)
            ),
            Some((Profile::CustomName("someone".to_string()), None))
        );
        assert_eq!(
            account_change(&editing(Field::SteamId, "123"), Some(ID)),
            None
        );
    }

    #[test]
    fn a_new_key_goes_with_the_saved_steam_id() {
        assert_eq!(
            account_change(&editing(Field::Key, " KEY "), Some(ID)),
            Some((Profile::Id(ID), Some("KEY".to_string())))
        );
        assert_eq!(account_change(&editing(Field::Key, "  "), Some(ID)), None);
        assert_eq!(account_change(&editing(Field::Key, "KEY"), None), None);
    }

    #[test]
    fn a_region_is_two_letters_different_from_the_saved_one() {
        let us = Region::new("US").unwrap();

        assert_eq!(
            region_change(&editing(Field::Region, " us "), Region::ITALY),
            Some(us)
        );
        assert_eq!(
            region_change(&editing(Field::Region, "IT"), Region::ITALY),
            None
        );
        assert_eq!(
            region_change(&editing(Field::Region, "USA"), Region::ITALY),
            None
        );
        assert_eq!(region_note("USA"), Some("Two letters, such as IT"));
        assert_eq!(region_note(""), None);
        assert_eq!(region_note("us"), None);
    }

    #[test]
    fn the_confirmation_names_the_new_region() {
        assert!(
            region_confirmation(Region::new("US").unwrap())
                .starts_with("Prices in US need a full update")
        );
    }

    #[test]
    fn escape_is_the_only_key() {
        let pressed = |key: Key| keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        };

        assert!(matches!(
            key_message(pressed(Key::Named(Named::Escape))),
            Some(Message::Escape)
        ));
        assert!(key_message(pressed(Key::Character("1".into()))).is_none());
    }
}
