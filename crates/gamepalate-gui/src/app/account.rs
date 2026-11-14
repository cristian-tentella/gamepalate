use std::{sync::Arc, time::Duration};

use gamepalate_app::{Gamepalate, SteamAccount, SteamId, settings::KeySource};
use iced::{
    Center, ContentFit, Element, Event, Fill, Right, Size, Subscription, Task, clipboard, event,
    keyboard::{self, Key, key::Named},
    padding,
    widget::{button, column, container, image, mouse_area, opaque, row, space, stack, text},
    window,
};

use crate::{font, format::key_text, screen, style};

pub const ICON: char = '\u{E853}';
const TIP: &str = "Steam account";
const POPUP_WIDTH: f32 = 300.0;
const POPUP_PADDING: f32 = 16.0;
const AVATAR_SIZE: f32 = 64.0;
const GAP: f32 = 6.0;
/// The icons right of the account in the top bar: Settings, the theme and the three window
/// buttons, 46 px each. The popup ends where the account icon does.
const ICONS_AFTER: f32 = 5.0 * 46.0;
const COPIED_FOR: Duration = Duration::from_secs(2);

/// Who the app is used as, shown in the top bar and, with a click, in a popup below it.
#[derive(Default)]
pub struct Account {
    profile: Option<Profile>,
    open: bool,
    copied: bool,
    /// Counts the loads: an answer for an older facade is dropped.
    loads: u64,
    /// Counts the copies: only the last one ends the "Copied" of the button.
    copies: u64,
}

struct Profile {
    steam_id: SteamId,
    name: Option<String>,
    avatar: Option<image::Handle>,
    key: KeySource,
    key_ending: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Toggle,
    Close,
    Copy,
    Copied(u64),
    OpenProfile,
    EditSettings,
    Loaded(u64, Option<Box<SteamAccount>>),
}

pub enum Action {
    Run(Task<Message>),
    EditSettings,
}

impl Account {
    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Toggle if self.open => self.close(),
            Message::Toggle => self.open = self.profile.is_some(),
            Message::Close => self.close(),
            Message::Copy => {
                let Some(profile) = &self.profile else {
                    return Action::Run(Task::none());
                };
                self.copied = true;
                self.copies += 1;
                let copy = self.copies;
                return Action::Run(Task::batch([
                    clipboard::write(profile.steam_id.0.to_string()),
                    Task::perform(
                        async move {
                            tokio::time::sleep(COPIED_FOR).await;
                            copy
                        },
                        Message::Copied,
                    ),
                ]));
            }
            Message::Copied(copy) => {
                if copy == self.copies {
                    self.copied = false;
                }
            }
            Message::OpenProfile => {
                if let Some(profile) = &self.profile {
                    screen::open(profile_url(profile.steam_id));
                }
                self.close();
            }
            Message::EditSettings => {
                self.close();
                return Action::EditSettings;
            }
            Message::Loaded(load, account) => {
                if load == self.loads {
                    self.profile = account.map(|account| Profile::new(*account));
                }
            }
        }
        Action::Run(Task::none())
    }

    /// Asks the facade who the account is, in the background. Until the answer comes there is no
    /// account to show, so the one before does not stand for the new one.
    pub fn load(&mut self, gamepalate: &Arc<Gamepalate>) -> Task<Message> {
        self.loads += 1;
        self.profile = None;
        self.close();
        let load = self.loads;
        let gamepalate = Arc::clone(gamepalate);

        Task::perform(
            async move { gamepalate.steam_account().await.map(Box::new) },
            move |account| Message::Loaded(load, account),
        )
    }

    pub fn close(&mut self) {
        self.open = false;
        self.copied = false;
    }

    /// While the popup is open, the keys are its own and not the page's.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_ready(&self) -> bool {
        self.profile.is_some()
    }

    pub fn avatar(&self) -> Option<&image::Handle> {
        self.profile.as_ref()?.avatar.as_ref()
    }

    pub fn tip(&self) -> String {
        match &self.profile {
            Some(profile) => shown_name(profile.name.as_deref(), profile.steam_id),
            None => TIP.to_string(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.open {
            event::listen_with(panel_key)
        } else {
            Subscription::none()
        }
    }

    /// The popup over the page, right under the account icon. `area` is the size below the top
    /// bar. The page under it stays as it is: a click anywhere else closes the popup.
    pub fn view(&self, area: Size) -> Option<Element<'_, Message>> {
        let profile = self.profile.as_ref().filter(|_| self.open)?;
        let outside = opaque(
            mouse_area(container(space()).width(Fill).height(Fill)).on_press(Message::Close),
        );
        let popup = opaque(
            container(profile.card(self.copied))
                .width(POPUP_WIDTH)
                .padding(POPUP_PADDING)
                .style(style::popup),
        );

        Some(
            stack![
                outside,
                container(popup)
                    .width(Fill)
                    .height(Fill)
                    .align_x(Right)
                    .padding(padding::top(GAP).right(popup_right(area.width))),
            ]
            .into(),
        )
    }
}

impl Profile {
    fn new(account: SteamAccount) -> Profile {
        Profile {
            steam_id: account.steam_id,
            name: account.name,
            avatar: account.avatar.map(image::Handle::from_bytes),
            key: account.key,
            key_ending: account.key_ending,
        }
    }

    fn card(&self, copied: bool) -> Element<'_, Message> {
        let picture: Element<'_, Message> = match &self.avatar {
            Some(handle) => image(handle.clone())
                .width(AVATAR_SIZE)
                .height(AVATAR_SIZE)
                .content_fit(ContentFit::Cover)
                .border_radius(AVATAR_SIZE / 2.0)
                .into(),
            None => text(ICON)
                .font(font::ICONS)
                .size(AVATAR_SIZE)
                .width(AVATAR_SIZE)
                .height(AVATAR_SIZE)
                .center()
                .style(style::muted)
                .into(),
        };
        let who = column![
            text(shown_name(self.name.as_deref(), self.steam_id))
                .font(font::BOLD)
                .size(font::INFO_SIZE),
            button(text("Open profile ↗").size(font::BAR_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::OpenProfile),
        ]
        .spacing(4)
        .width(Fill);

        let steam_id = row![
            text(self.steam_id.0.to_string())
                .size(font::BAR_SIZE)
                .width(Fill),
            button(text(if copied { "Copied" } else { "Copy" }).size(font::BAR_SIZE))
                .padding([4, 10])
                .style(style::quiet_action)
                .on_press(Message::Copy),
        ]
        .spacing(8)
        .align_y(Center);

        column![
            row![picture, who].spacing(14).align_y(Center),
            style::separator(),
            field("Steam ID", steam_id.into()),
            field(
                "API key",
                text(key_line(self.key, self.key_ending.as_deref()))
                    .size(font::BAR_SIZE)
                    .into()
            ),
            style::separator(),
            button(text("Edit in Settings").size(font::BAR_SIZE))
                .padding([6, 10])
                .style(style::quiet_action)
                .on_press(Message::EditSettings),
        ]
        .spacing(12)
        .into()
    }
}

fn field<'a>(label: &'static str, value: Element<'a, Message>) -> Element<'a, Message> {
    column![text(label).size(font::NOTE_SIZE).style(style::muted), value]
        .spacing(2)
        .into()
}

fn shown_name(name: Option<&str>, steam_id: SteamId) -> String {
    name.map_or_else(|| steam_id.0.to_string(), str::to_string)
}

fn profile_url(steam_id: SteamId) -> String {
    format!("https://steamcommunity.com/profiles/{}", steam_id.0)
}

/// The key is never shown, only where it comes from and how it ends.
fn key_line(key: KeySource, ending: Option<&str>) -> String {
    match (key, ending) {
        (KeySource::Missing, _) => "No key".to_string(),
        (key, Some(ending)) => format!("{} · ends in {ending}", key_text(key)),
        (key, None) => key_text(key).to_string(),
    }
}

/// The popup ends under the account icon, or against the window's edge when that is too narrow.
fn popup_right(width: f32) -> f32 {
    ICONS_AFTER.min((width - POPUP_WIDTH).max(0.0))
}

fn panel_key(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        }) => Some(Message::Close),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: SteamId = SteamId(76561198000000000);

    fn account(name: Option<&str>) -> Box<SteamAccount> {
        Box::new(SteamAccount {
            steam_id: ID,
            name: name.map(str::to_string),
            avatar: None,
            key: KeySource::Saved,
            key_ending: Some("3F9A".to_string()),
        })
    }

    fn loaded(name: Option<&str>) -> Account {
        let mut account_state = Account {
            loads: 1,
            ..Account::default()
        };
        account_state.update(Message::Loaded(1, Some(account(name))));
        account_state
    }

    fn pressed(key: Key) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }

    fn panel_key_of(key: Key) -> Option<Message> {
        panel_key(pressed(key), event::Status::Ignored, window::Id::unique())
    }

    #[test]
    fn the_tip_is_the_name_or_else_the_steam_id() {
        assert_eq!(loaded(Some("Ada")).tip(), "Ada");
        assert_eq!(loaded(None).tip(), ID.0.to_string());
        assert_eq!(Account::default().tip(), "Steam account");
    }

    #[test]
    fn the_profile_link_opens_the_page_of_the_steam_id() {
        assert_eq!(
            profile_url(ID),
            "https://steamcommunity.com/profiles/76561198000000000"
        );
    }

    #[test]
    fn the_key_line_says_the_source_and_how_the_key_ends() {
        assert_eq!(
            key_line(KeySource::Saved, Some("3F9A")),
            format!("{} · ends in 3F9A", key_text(KeySource::Saved))
        );
        assert_eq!(
            key_line(KeySource::Environment, Some("3F9A")),
            "From STEAM_API_KEY · ends in 3F9A"
        );
        assert_eq!(key_line(KeySource::Saved, None), key_text(KeySource::Saved));
        assert_eq!(key_line(KeySource::Missing, None), "No key");
    }

    #[test]
    fn escape_closes_the_popup_and_other_keys_do_not() {
        assert!(matches!(
            panel_key_of(Key::Named(Named::Escape)),
            Some(Message::Close)
        ));
        assert!(panel_key_of(Key::Named(Named::Space)).is_none());
        assert!(panel_key_of(Key::Character("a".into())).is_none());
    }

    #[test]
    fn the_icon_opens_and_closes_the_popup_once_the_account_is_known() {
        let mut account = Account::default();
        account.update(Message::Toggle);
        assert!(!account.is_open());

        let mut account = loaded(None);
        account.update(Message::Toggle);
        assert!(account.is_open());
        account.update(Message::Toggle);
        assert!(!account.is_open());

        account.update(Message::Toggle);
        account.update(Message::Close);
        assert!(!account.is_open());
    }

    #[test]
    fn editing_in_settings_closes_the_popup() {
        let mut account = loaded(None);
        account.update(Message::Toggle);

        assert!(matches!(
            account.update(Message::EditSettings),
            Action::EditSettings
        ));
        assert!(!account.is_open());
    }

    #[test]
    fn an_answer_for_an_older_facade_is_dropped() {
        let mut account = Account {
            loads: 2,
            ..Account::default()
        };

        account.update(Message::Loaded(1, Some(self::account(Some("Old")))));
        assert!(!account.is_ready());

        account.update(Message::Loaded(2, Some(self::account(Some("New")))));
        assert_eq!(account.tip(), "New");

        account.update(Message::Loaded(2, None));
        assert!(!account.is_ready());
    }

    #[test]
    fn only_the_last_copy_ends_the_copied_label() {
        let mut account = loaded(None);
        account.update(Message::Toggle);
        account.update(Message::Copy);
        account.update(Message::Copy);
        assert!(account.copied);

        account.update(Message::Copied(1));
        assert!(account.copied);
        account.update(Message::Copied(2));
        assert!(!account.copied);
    }

    #[test]
    fn the_popup_ends_under_the_icon_and_stays_inside_a_narrow_window() {
        assert_eq!(popup_right(1600.0), ICONS_AFTER);
        assert_eq!(popup_right(500.0), 200.0);
        assert_eq!(popup_right(200.0), 0.0);
    }
}
