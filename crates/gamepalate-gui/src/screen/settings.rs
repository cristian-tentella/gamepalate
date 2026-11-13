mod account;
mod config;

use std::{
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{
    ConfigValue, Gamepalate, Region, ThemeChoice, Timestamp,
    settings::{self, ConfigKey, change_config},
    setup::error_chain,
    steam_client::answer_steam_client,
};
use iced::{
    Center, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    padding, time,
    widget::{button, column, container, radio, row, scrollable, text, tooltip},
};

use crate::{
    app::Page,
    font, format,
    new_version::{NewVersions, Offer},
    screen::game::open_url,
    screen::setup::PRIVACY_PAGE,
    style,
};
use account::{Account, saved_text};
use config::{CONFIG_NOTE, CORRECTIONS, DISCOVER, TASTE, Value, parse_value, value_input};

const BACK: char = '\u{E5CB}';
const COLUMN_WIDTH: f32 = 760.0;
const LABEL_WIDTH: f32 = 120.0;
const MEGABYTE: u64 = 1024 * 1024;
const TICK: Duration = Duration::from_millis(250);

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
            Message::Input(..)
            | Message::Save(_)
            | Message::Saved(..)
            | Message::ConfirmRegion
            | Message::CancelRegion
            | Message::RegionSaved(_) => self.update_account(message),
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
            Message::ConfigInput(..)
            | Message::SaveConfig(_)
            | Message::ResetConfig(_)
            | Message::SetConfig(..)
            | Message::ConfigSaved(..)
            | Message::Flushed(_)
            | Message::Tick(_) => self.update_config(message),
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
                        about(new_versions),
                        account,
                        country,
                        appearance,
                        taste,
                        discover,
                        corrections,
                        data,
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
mod fixtures {
    use gamepalate_app::SteamId;

    pub(super) const ID: SteamId = SteamId(76_561_198_000_000_000);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_says_how_many_games_and_when_its_tags_were_read() {
        assert_eq!(
            catalog_text(115_668, "28 Oct"),
            "115,668 games · tags 28 Oct"
        );
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
