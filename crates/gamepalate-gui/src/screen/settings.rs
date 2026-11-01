use std::{process::Command, sync::Arc};

use gamepalate_app::{
    Gamepalate, Region, SteamId, ThemeChoice,
    account::{Profile, connect, parse_profile},
    settings::{self, KeySource, change_region},
    setup::error_chain,
};
use iced::{
    Center, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    widget::{button, column, container, radio, row, scrollable, text, text_input},
};

use crate::{
    app::Page,
    font,
    screen::setup::{
        KEY_DOMAIN_NOTE, PRIVACY_PAGE, PROFILE_PLACEHOLDER, connect_error_text, needs_privacy,
        note_link, steam_id_note,
    },
    style,
};

const COLUMN_WIDTH: f32 = 760.0;
const LABEL_WIDTH: f32 = 120.0;
const MEGABYTE: u64 = 1024 * 1024;
const CONFIG_NOTE: &str = "Changes to config.toml apply after the next update or restart";

pub struct Settings {
    gamepalate: Arc<Gamepalate>,
    settings: settings::Settings,
    back: Page,
    cache_bytes: Option<u64>,
    editing: Option<Editing>,
    confirming: Option<Region>,
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

    pub fn subscription() -> Subscription<Message> {
        keyboard::listen().filter_map(key_message)
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Edit(field) => {
                self.confirming = None;
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
            Message::Escape => {
                if self.editing.is_none() {
                    return Action::Close(self.back);
                }
                self.step_back();
                Action::None
            }
        }
    }

    fn step_back(&mut self) {
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
                    .push(data)
                    .spacing(32)
                    .max_width(COLUMN_WIDTH),
            )
            .padding(24)
            .width(Fill),
        )
        .height(Fill)
        .into()
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

fn key_text(key: KeySource) -> &'static str {
    match key {
        KeySource::Saved => "Saved in Windows",
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

    fn editing(field: Field, text: &str) -> Editing {
        Editing {
            field,
            text: text.to_string(),
            saving: false,
            error: None,
        }
    }

    #[test]
    fn each_key_source_has_its_text() {
        assert_eq!(key_text(KeySource::Saved), "Saved in Windows");
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
