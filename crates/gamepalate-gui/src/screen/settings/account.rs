use gamepalate_app::{
    Region, SteamId,
    account::{Profile, connect, parse_profile},
    settings::{self, KeySource, change_region},
    setup::error_chain,
};
use iced::{
    Center, Element, Fill, Task,
    widget::{button, column, row, text, text_input},
};

use super::{Action, Field, Fields, Message, Settings, link, setting};
use crate::{
    font,
    screen::setup::{
        KEY_DOMAIN_NOTE, PROFILE_PLACEHOLDER, connect_error_text, needs_privacy, note_link,
        steam_id_note,
    },
    style,
};

const KEY_FROM_ENVIRONMENT: &str =
    "Set by the STEAM_API_KEY environment variable, change it there to replace it";

const REGION_WIDTH: f32 = 64.0;

#[derive(Debug, Default)]
pub(super) struct Account {
    pub(super) text: String,
    pub(super) saving: bool,
    pub(super) error: Option<String>,
}

impl Account {
    pub(super) fn showing(text: String) -> Account {
        Account {
            text,
            ..Account::default()
        }
    }

    pub(super) fn typed(&mut self, text: String) {
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

pub(super) fn saved_text(field: Field, settings: &settings::Settings) -> String {
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

fn key_text(key: KeySource) -> &'static str {
    match key {
        KeySource::Saved if cfg!(windows) => "Saved in Credential Manager",
        KeySource::Saved => "Saved in the system keyring",
        KeySource::Environment => "From STEAM_API_KEY",
        KeySource::Missing => "Missing",
    }
}

impl Settings {
    pub(super) fn update_account(&mut self, message: Message) -> Action {
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
            _ => Action::None,
        }
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

    pub(super) fn account_line(&self, field: Field) -> Element<'_, Message> {
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
    fn account(&self, field: Field) -> &Account {
        match field {
            Field::SteamId => &self.steam_id,
            Field::Key => &self.key,
            Field::Region => &self.region,
        }
    }

    pub(super) fn account_mut(&mut self, field: Field) -> &mut Account {
        match field {
            Field::SteamId => &mut self.steam_id,
            Field::Key => &mut self.key,
            Field::Region => &mut self.region,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;

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
}
