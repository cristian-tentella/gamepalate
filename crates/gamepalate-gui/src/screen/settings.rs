use std::{
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{
    Config, ConfigError, ConfigValue, Gamepalate, Region, SteamId, ThemeChoice, Timestamp,
    account::{Profile, connect, parse_profile},
    settings::{self, ConfigKey, KeySource, change_config, change_region, reset_config},
    setup::error_chain,
    steam_client::answer_steam_client,
};
use iced::{
    Center, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    padding, time,
    widget::{button, column, container, radio, row, scrollable, text, text_input, tooltip},
};

use crate::{
    app::Page,
    font, format,
    new_version::{NewVersions, Offer},
    screen::game::open_url,
    screen::setup::{
        KEY_DOMAIN_NOTE, PRIVACY_PAGE, PROFILE_PLACEHOLDER, connect_error_text, needs_privacy,
        note_link, steam_id_note,
    },
    style,
};

const BACK: char = '\u{E5CB}';
const COLUMN_WIDTH: f32 = 760.0;
const LABEL_WIDTH: f32 = 120.0;
const NUMBER_WIDTH: f32 = 96.0;
const REGION_WIDTH: f32 = 64.0;
const MEGABYTE: u64 = 1024 * 1024;
const SAVE_AFTER: Duration = Duration::from_secs(1);
const SAVED_FOR: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(250);
const CONFIG_NOTE: &str = "Changes made in the file apply after the next update or restart";
const KEY_FROM_ENVIRONMENT: &str =
    "Set by the STEAM_API_KEY environment variable, change it there to replace it";
const TASTE: [ConfigKey; 8] = [
    ConfigKey::WishlistHours,
    ConfigKey::WishlistCap,
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
    fields: Fields,
    confirming: Option<Region>,
    steam_client: Answer,
    error: Option<String>,
}

#[derive(Debug, Default)]
struct Fields {
    values: Vec<(ConfigKey, Value)>,
    steam_id: Account,
    key: Account,
    region: Account,
}

#[derive(Debug, Default)]
struct Value {
    text: String,
    changed_at: Option<Instant>,
    saving: bool,
    saved_at: Option<Instant>,
    error: Option<String>,
}

#[derive(Debug, Default)]
struct Account {
    text: String,
    saving: bool,
    error: Option<String>,
}

#[derive(Debug, Default)]
struct Answer {
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
    Input(Field, String),
    Save(Field),
    Saved(Field, Result<(), String>),
    ConfirmRegion,
    CancelRegion,
    OpenPrivacy,
    RegionSaved(Result<(), String>),
    Theme(ThemeChoice),
    OpenFolder,
    OpenConfig,
    ClearCache,
    CacheMeasured(u64),
    SteamClient(bool),
    SteamClientSaved(Result<(), String>),
    ConfigInput(ConfigKey, String),
    SaveConfig(ConfigKey),
    ResetConfig(ConfigKey),
    SetConfig(ConfigKey, ConfigValue),
    ConfigSaved(ConfigKey, Result<(), String>),
    Flushed(bool),
    Tick(Instant),
    Install,
    CheckNow,
    CheckAtStartup(bool),
    OpenRelease(String),
    Back,
    Escape,
}

impl Message {
    pub fn changed_data(&self) -> bool {
        matches!(
            self,
            Message::Saved(_, Ok(()))
                | Message::RegionSaved(Ok(()))
                | Message::SteamClientSaved(Ok(()))
                | Message::ConfigSaved(_, Ok(()))
                | Message::Flushed(true)
        )
    }
}

pub enum Action {
    None,
    Run(Task<Message>),
    Theme(ThemeChoice),
    Reopen,
    RegionChanged,
    Close(Page, Task<Message>),
    CheckNewVersion,
    InstallNewVersion,
    CheckAtStartup(bool),
}

impl Settings {
    pub fn new(gamepalate: &Arc<Gamepalate>, back: Page) -> (Settings, Task<Message>) {
        let settings = gamepalate.settings();
        let settings = Settings {
            gamepalate: Arc::clone(gamepalate),
            fields: Fields::of(&settings),
            settings,
            back,
            cache_bytes: None,
            confirming: None,
            steam_client: Answer::default(),
            error: None,
        };
        let task = settings.measure_cache();

        (settings, task)
    }

    pub fn back(&self) -> Page {
        self.back
    }

    pub fn failed(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn reopened(&mut self, gamepalate: &Arc<Gamepalate>) {
        self.gamepalate = Arc::clone(gamepalate);
        self.settings = gamepalate.settings();
        self.fields.refresh(&self.settings);
        self.error = None;
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::listen().filter_map(key_message);
        if !self.fields.waiting() {
            return keys;
        }

        Subscription::batch([keys, time::every(TICK).map(Message::Tick)])
    }

    pub fn leave(&mut self) -> Task<Message> {
        let pending = self.fields.pending(&self.settings.config);
        if pending.is_empty() {
            return Task::none();
        }
        let dir = self.settings.data_dir.clone();

        Task::perform(
            async move {
                pending
                    .into_iter()
                    .filter(|&(key, value)| change_config(&dir, key, value).is_ok())
                    .count()
                    > 0
            },
            Message::Flushed,
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Input(field, input) => {
                if field == Field::Region {
                    self.confirming = None;
                }
                self.fields.account_mut(field).typed(input);
                Action::None
            }
            Message::Save(Field::Region) => {
                self.confirming = region_change(&self.fields.region.text, self.settings.region);
                Action::None
            }
            Message::Save(field) => self.save_account(field),
            Message::Saved(_, Ok(())) => Action::Reopen,
            Message::Saved(field, Err(error)) => {
                self.fields.account_mut(field).failed(error);
                Action::None
            }
            Message::ConfirmRegion => self.save_region(),
            Message::CancelRegion => {
                self.confirming = None;
                self.fields.region.text = self.settings.region.to_string();
                Action::None
            }
            Message::RegionSaved(Ok(())) => {
                self.confirming = None;
                Action::RegionChanged
            }
            Message::RegionSaved(Err(error)) => {
                self.confirming = None;
                self.fields.region.failed(error);
                Action::None
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
            Message::SteamClient(read) => {
                if self.steam_client.saving || self.settings.steam_client_hours == Some(read) {
                    return Action::None;
                }
                self.steam_client = Answer {
                    saving: true,
                    error: None,
                };
                let dir = self.settings.data_dir.clone();
                Action::Run(Task::perform(
                    async move { answer_steam_client(&dir, read).map_err(|error| error_chain(&error)) },
                    Message::SteamClientSaved,
                ))
            }
            Message::SteamClientSaved(Ok(())) => {
                self.steam_client.saving = false;
                Action::Reopen
            }
            Message::SteamClientSaved(Err(error)) => {
                self.steam_client = Answer {
                    saving: false,
                    error: Some(error),
                };
                Action::None
            }
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
            Message::Install => Action::InstallNewVersion,
            Message::CheckNow => Action::CheckNewVersion,
            Message::CheckAtStartup(check) => Action::CheckAtStartup(check),
            Message::OpenRelease(page) => {
                open_url(&page);
                Action::None
            }
            Message::Back => self.close(),
            Message::Escape => {
                if self.confirming.is_some() {
                    return self.update(Message::CancelRegion);
                }
                self.close()
            }
        }
    }

    fn close(&mut self) -> Action {
        Action::Close(self.back, self.leave())
    }

    fn measure_cache(&self) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        Task::perform(
            async move { gamepalate.image_cache_bytes() },
            Message::CacheMeasured,
        )
    }

    fn save_account(&mut self, field: Field) -> Action {
        let account = self.fields.account_mut(field);
        if account.saving {
            return Action::None;
        }
        let Some((profile, key)) = account_change(field, &account.text, self.settings.steam_id)
        else {
            return Action::None;
        };
        account.saving = true;
        account.error = None;
        let dir = self.settings.data_dir.clone();

        Action::Run(Task::perform(
            async move {
                connect(&dir, profile, key)
                    .await
                    .map_err(|error| connect_error_text(&error))
            },
            move |result| Message::Saved(field, result),
        ))
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

    fn save_region(&mut self) -> Action {
        let Some(region) = self.confirming else {
            return Action::None;
        };
        if self.fields.region.saving {
            return Action::None;
        }
        self.fields.region.saving = true;
        self.fields.region.error = None;
        let dir = self.settings.data_dir.clone();

        Action::Run(Task::perform(
            async move { change_region(&dir, region).map_err(|error| error_chain(&error)) },
            Message::RegionSaved,
        ))
    }

    pub fn view<'a>(
        &'a self,
        theme: ThemeChoice,
        new_versions: &'a NewVersions,
    ) -> Element<'a, Message> {
        let account = group(
            "Account",
            column![
                self.account_line(Field::SteamId),
                self.account_line(Field::Key),
            ]
            .spacing(12),
        );
        let country = group("Country", self.account_line(Field::Region));
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
        let error = self.error.as_ref().map(|error| {
            container(text(error.clone()).size(font::BAR_SIZE).style(text::danger))
                .padding(padding::bottom(32))
        });

        let page = scrollable(
            container(
                column![
                    column![].push(error),
                    column![
                        account,
                        country,
                        appearance,
                        taste,
                        discover,
                        corrections,
                        data,
                        about(new_versions),
                    ]
                    .spacing(32),
                ]
                .width(Fill)
                .max_width(COLUMN_WIDTH),
            )
            .padding(24)
            .center_x(Fill),
        )
        .height(Fill);

        column![title_bar(), style::separator(), page].into()
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

    fn steam_client_line(&self) -> Element<'_, Message> {
        let Some(file) = &self.settings.steam_client_file else {
            return setting("Steam app", value("Not found on this PC".to_string()), None);
        };
        let choice = |label, read| {
            radio(
                label,
                read,
                self.settings.steam_client_hours,
                Message::SteamClient,
            )
            .size(16)
            .text_size(font::BAR_SIZE)
        };
        let error = self.steam_client.error.as_ref().map(|error| {
            text(error.clone())
                .size(font::NOTE_SIZE)
                .style(text::danger)
        });

        setting(
            "Steam app",
            column![
                row![choice("Read", true), choice("Don't read", false)].spacing(24),
                text(steam_client_note(&file.display().to_string()))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .push(error)
            .spacing(4)
            .into(),
            None,
        )
    }

    fn account_line(&self, field: Field) -> Element<'_, Message> {
        let label = match field {
            Field::SteamId => "Steam ID",
            Field::Key => "API key",
            Field::Region => "Prices from",
        };
        let account = self.fields.account(field);
        let locked = field == Field::Key && self.settings.key == KeySource::Environment;
        let placeholder = match field {
            Field::SteamId => PROFILE_PLACEHOLDER,
            Field::Key => key_text(self.settings.key),
            Field::Region => "IT",
        };
        let input = text_input(placeholder, &account.text)
            .style(if account.error.is_some() {
                style::wrong_field
            } else {
                style::field
            })
            .secure(field == Field::Key)
            .on_input_maybe(
                (!account.saving && !locked).then_some(move |input| Message::Input(field, input)),
            )
            .on_submit(Message::Save(field))
            .width(if field == Field::Region {
                REGION_WIDTH.into()
            } else {
                Fill
            })
            .padding(8)
            .size(font::BAR_SIZE);
        let edited = account_edited(field, &account.text, &saved_text(field, &self.settings));
        let can_save = !account.saving
            && match field {
                Field::Region => region_change(&account.text, self.settings.region).is_some(),
                Field::SteamId | Field::Key => {
                    account_change(field, &account.text, self.settings.steam_id).is_some()
                }
            };
        let save = ((edited && self.confirming.is_none())
            || (account.saving && field != Field::Region))
            .then(|| {
                button(
                    text(if account.saving {
                        "Checking with Steam…"
                    } else {
                        "Save"
                    })
                    .size(font::BAR_SIZE),
                )
                .style(style::primary)
                .padding([8, 20])
                .on_press_maybe(can_save.then_some(Message::Save(field)))
            });
        let note = match (field, &account.error) {
            (_, Some(error)) => Some(
                text(error.clone())
                    .size(font::NOTE_SIZE)
                    .style(text::danger),
            ),
            (Field::SteamId, None) => steam_id_note(&account.text)
                .map(|note| text(note).size(font::NOTE_SIZE).style(text::danger)),
            (Field::Region, None) => region_note(&account.text)
                .map(|note| text(note).size(font::NOTE_SIZE).style(text::danger)),
            (Field::Key, None) => Some(
                text(if locked {
                    KEY_FROM_ENVIRONMENT
                } else {
                    KEY_DOMAIN_NOTE
                })
                .size(font::NOTE_SIZE)
                .style(style::muted),
            ),
        };
        let privacy = account
            .error
            .as_deref()
            .filter(|error| needs_privacy(error))
            .map(|_| note_link("Open privacy settings ↗", Message::OpenPrivacy));
        let confirmation = self
            .confirming
            .filter(|_| field == Field::Region)
            .map(|region| confirmation(region, account.saving));

        setting(
            label,
            column![row![input].push(save).spacing(12).align_y(Center)]
                .push(note)
                .push(privacy)
                .push(confirmation)
                .spacing(8)
                .into(),
            None,
        )
    }
}

impl Fields {
    fn of(settings: &settings::Settings) -> Fields {
        let values = TASTE
            .iter()
            .chain(&DISCOVER)
            .chain(&CORRECTIONS)
            .filter_map(|&key| match key.value(&settings.config) {
                ConfigValue::Flag(_) => None,
                current => Some((
                    key,
                    Value {
                        text: value_input(current),
                        ..Value::default()
                    },
                )),
            })
            .collect();

        Fields {
            values,
            steam_id: Account::showing(saved_text(Field::SteamId, settings)),
            key: Account::default(),
            region: Account::showing(saved_text(Field::Region, settings)),
        }
    }

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

    fn account(&self, field: Field) -> &Account {
        match field {
            Field::SteamId => &self.steam_id,
            Field::Key => &self.key,
            Field::Region => &self.region,
        }
    }

    fn account_mut(&mut self, field: Field) -> &mut Account {
        match field {
            Field::SteamId => &mut self.steam_id,
            Field::Key => &mut self.key,
            Field::Region => &mut self.region,
        }
    }

    fn waiting(&self) -> bool {
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

    fn pending(&self, config: &Config) -> Vec<(ConfigKey, ConfigValue)> {
        self.values
            .iter()
            .filter(|(_, value)| value.changed_at.is_some() && !value.saving)
            .filter_map(|(key, value)| {
                config_change(*key, &value.text, config).map(|change| (*key, change))
            })
            .collect()
    }

    fn refresh(&mut self, settings: &settings::Settings) {
        for (key, value) in &mut self.values {
            let current = key.value(&settings.config);
            if value.changed_at.is_none()
                && !value.saving
                && value.error.is_none()
                && parse_value(*key, &value.text) != Some(current)
            {
                value.text = value_input(current);
            }
        }
        for field in [Field::SteamId, Field::Key, Field::Region] {
            let account = self.account_mut(field);
            if account.saving {
                *account = Account::showing(saved_text(field, settings));
            }
        }
    }
}

impl Value {
    fn typed(&mut self, text: String, now: Instant) {
        self.text = text;
        self.changed_at = Some(now);
        self.saved_at = None;
        self.error = None;
    }
}

impl Account {
    fn showing(text: String) -> Account {
        Account {
            text,
            ..Account::default()
        }
    }

    fn typed(&mut self, text: String) {
        if !self.saving {
            self.text = text;
            self.error = None;
        }
    }

    fn failed(&mut self, error: String) {
        self.saving = false;
        self.error = Some(error);
    }
}

fn saved_text(field: Field, settings: &settings::Settings) -> String {
    match field {
        Field::SteamId => settings
            .steam_id
            .map(|id| id.0.to_string())
            .unwrap_or_default(),
        Field::Key => String::new(),
        Field::Region => settings.region.to_string(),
    }
}

fn account_edited(field: Field, text: &str, saved: &str) -> bool {
    let text = text.trim();
    match field {
        Field::SteamId => text != saved,
        Field::Key => !text.is_empty(),
        Field::Region => !text.eq_ignore_ascii_case(saved),
    }
}

fn parse_region(text: &str) -> Option<Region> {
    Region::new(&text.trim().to_ascii_uppercase())
}

fn region_change(text: &str, saved: Region) -> Option<Region> {
    parse_region(text).filter(|region| *region != saved)
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
            "One game in this many is one your votes would skip, 0 turns them off"
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

fn region_note(text: &str) -> Option<&'static str> {
    (!text.trim().is_empty() && parse_region(text).is_none()).then_some("Two letters, such as IT")
}

fn region_confirmation(region: Region) -> String {
    format!(
        "Prices in {region} need a full update, about 20 minutes like the first one. You can keep using gamepalate meanwhile."
    )
}

fn confirmation<'a>(region: Region, saving: bool) -> Element<'a, Message> {
    let buttons = row![
        button(
            text(if saving {
                "Saving…"
            } else {
                "Change and update"
            })
            .size(font::BAR_SIZE)
        )
        .style(style::primary)
        .padding([8, 20])
        .on_press_maybe((!saving).then_some(Message::ConfirmRegion)),
    ]
    .push((!saving).then(|| link("Cancel", Message::CancelRegion)))
    .spacing(16)
    .align_y(Center);

    column![
        text(region_confirmation(region)).size(font::BAR_SIZE),
        buttons,
    ]
    .spacing(8)
    .into()
}

fn account_change(
    field: Field,
    text: &str,
    saved: Option<SteamId>,
) -> Option<(Profile, Option<String>)> {
    match field {
        Field::SteamId => parse_profile(text)
            .ok()
            .filter(|profile| !matches!(profile, Profile::Id(id) if Some(*id) == saved))
            .map(|profile| (profile, None)),
        Field::Region => None,
        Field::Key => {
            let key = text.trim();
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

fn steam_client_note(file: &str) -> String {
    format!("The hours of games you don't own, from {file}")
}

fn key_text(key: KeySource) -> &'static str {
    match key {
        KeySource::Saved if cfg!(windows) => "Saved in Credential Manager",
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

fn title_bar<'a>() -> Element<'a, Message> {
    let back = tip(
        button(
            container(
                text(BACK)
                    .font(font::ICONS)
                    .size(font::icon(14.0))
                    .width(Fill)
                    .height(Fill)
                    .center(),
            )
            .width(40)
            .height(40),
        )
        .padding(0)
        .style(style::window_button)
        .on_press(Message::Back)
        .into(),
        "Back (Esc)".to_string(),
    );

    container(
        row![
            back,
            text("Settings").font(font::SEMIBOLD).size(font::INFO_SIZE)
        ]
        .spacing(12)
        .align_y(Center),
    )
    .padding([8, 12])
    .width(Fill)
    .into()
}

fn tip<'a>(content: Element<'a, Message>, label: String) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Bottom,
    )
    .into()
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

fn about(new_versions: &NewVersions) -> Element<'_, Message> {
    let offer =
        new_versions
            .offer()
            .zip(new_versions.available())
            .map(|((offer, enabled), new)| {
                let page = Message::OpenRelease(new.release_page.clone());
                let (label, message) = match offer {
                    Offer::Install => ("Install", Message::Install),
                    Offer::OpenReleasePage => ("Open release page", page.clone()),
                };
                row![
                    button(text(label).size(font::BAR_SIZE))
                        .style(style::primary)
                        .padding([8, 20])
                        .on_press_maybe(enabled.then_some(message)),
                ]
                .push((offer == Offer::Install).then(|| link("What's new", page)))
                .spacing(16)
                .align_y(Center)
            });
    let problem = new_versions
        .problem_text()
        .map(|problem| text(problem).size(font::NOTE_SIZE).style(text::danger));
    let check = Some(new_versions.check_at_startup());
    let choice = |label, flag| {
        radio(label, flag, check, Message::CheckAtStartup)
            .size(16)
            .text_size(font::BAR_SIZE)
    };

    group(
        "About",
        column![
            setting(
                "Version",
                column![value(new_versions.version_text())]
                    .push(offer)
                    .push(problem)
                    .spacing(8)
                    .into(),
                None,
            ),
            setting(
                "New versions",
                row![
                    choice("Check at startup", true),
                    choice("Don't check", false)
                ]
                .spacing(24)
                .into(),
                Some(link("Check now", Message::CheckNow)),
            ),
        ]
        .spacing(12),
    )
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

    fn typed(fields: &mut Fields, key: ConfigKey, text: &str, at: Instant) {
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
    fn the_catalog_says_how_many_games_and_when_its_tags_were_read() {
        assert_eq!(
            catalog_text(115_668, "28 Oct"),
            "115,668 games · tags 28 Oct"
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
        assert_eq!(shown(ConfigKey::Count), "20 games");
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
    fn the_saved_key_is_in_credential_manager() {
        assert_eq!(key_text(KeySource::Saved), "Saved in Credential Manager");
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
    fn save_shows_only_when_the_text_is_not_the_saved_one() {
        let saved = ID.0.to_string();

        assert!(!account_edited(
            Field::SteamId,
            &format!(" {saved} "),
            &saved
        ));
        assert!(account_edited(Field::SteamId, "76561198000000001", &saved));
        assert!(!account_edited(Field::Key, "  ", ""));
        assert!(account_edited(Field::Key, "KEY", ""));
        assert!(!account_edited(Field::Region, "it", "IT"));
        assert!(account_edited(Field::Region, "US", "IT"));
    }

    #[test]
    fn a_new_steam_id_keeps_the_saved_key() {
        assert_eq!(
            account_change(
                Field::SteamId,
                "https://steamcommunity.com/profiles/76561198000000000",
                None
            ),
            Some((Profile::Id(ID), None))
        );
        assert_eq!(
            account_change(
                Field::SteamId,
                "https://steamcommunity.com/id/someone",
                Some(ID)
            ),
            Some((Profile::CustomName("someone".to_string()), None))
        );
        assert_eq!(account_change(Field::SteamId, "123", Some(ID)), None);
        assert_eq!(
            account_change(Field::SteamId, "76561198000000000", Some(ID)),
            None
        );
    }

    #[test]
    fn a_new_key_goes_with_the_saved_steam_id() {
        assert_eq!(
            account_change(Field::Key, " KEY ", Some(ID)),
            Some((Profile::Id(ID), Some("KEY".to_string())))
        );
        assert_eq!(account_change(Field::Key, "  ", Some(ID)), None);
        assert_eq!(account_change(Field::Key, "KEY", None), None);
    }

    #[test]
    fn a_region_is_two_letters_different_from_the_saved_one() {
        let us = Region::new("US").unwrap();

        assert_eq!(region_change(" us ", Region::ITALY), Some(us));
        assert_eq!(region_change("IT", Region::ITALY), None);
        assert_eq!(region_change("USA", Region::ITALY), None);
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
    fn only_saves_ask_the_others_to_reopen() {
        assert!(Message::ConfigSaved(ConfigKey::Count, Ok(())).changed_data());
        assert!(Message::Flushed(true).changed_data());
        assert!(Message::Saved(Field::SteamId, Ok(())).changed_data());
        assert!(!Message::Flushed(false).changed_data());
        assert!(!Message::ConfigSaved(ConfigKey::Count, Err(String::new())).changed_data());
        assert!(!Message::Back.changed_data());
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
