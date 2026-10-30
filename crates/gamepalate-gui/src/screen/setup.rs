use std::process::Command;

use gamepalate_app::{
    Gamepalate, SteamError,
    account::{ConnectError, SteamIdError, connect, parse_steam_id},
    setup::{FirstRun, Part, error_chain},
};
use iced::{
    Center, Element, Fill, Task, padding,
    widget::{button, column, container, row, text, text_input},
};

use crate::{font, style};

const CHECK: char = '\u{E73E}';
const CROSS: char = '\u{E711}';
const WIDTH: f32 = 760.0;
const KEY_PAGE: &str = "https://steamcommunity.com/dev/apikey";
const FIND_STEAM_ID: &str =
    "Open Steam → your name at the top right → Account details: the Steam ID is under your name.";

pub struct Setup {
    first_run: FirstRun,
    error: Option<String>,
    account: Account,
}

#[derive(Debug, Default)]
struct Account {
    steam_id: String,
    key: String,
    editing: bool,
    connecting: bool,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    CheckAgain,
    OpenFolder,
    SteamId(String),
    Key(String),
    GetKey,
    Connect,
    Connected(Result<(), String>),
    Change,
}

pub enum Action {
    None,
    Run(Task<Message>),
    Open(Box<Gamepalate>),
}

#[derive(Debug, PartialEq)]
struct Line {
    label: &'static str,
    present: bool,
    hint: Option<String>,
}

pub fn start(first_run: FirstRun) -> Result<Gamepalate, Box<Setup>> {
    if !first_run.ready() {
        return Err(Box::new(Setup::new(first_run, None)));
    }

    Gamepalate::open(first_run.data_dir.clone())
        .map_err(|error| Box::new(Setup::new(first_run, Some(error_chain(&error)))))
}

impl Setup {
    fn new(first_run: FirstRun, error: Option<String>) -> Setup {
        let account = Account {
            steam_id: first_run
                .saved_steam_id
                .map(|id| id.0.to_string())
                .unwrap_or_default(),
            ..Account::default()
        };

        Setup {
            first_run,
            error,
            account,
        }
    }

    pub fn first_run(&self) -> &FirstRun {
        &self.first_run
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::CheckAgain | Message::Connected(Ok(())) => self.check_again(),
            Message::OpenFolder => {
                if let Some(dir) = &self.first_run.data_dir {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = Command::new("explorer").arg(dir).spawn();
                }
                Action::None
            }
            Message::SteamId(steam_id) => {
                self.account.steam_id = steam_id;
                Action::None
            }
            Message::Key(key) => {
                self.account.key = key;
                Action::None
            }
            Message::GetKey => {
                let _ = Command::new("rundll32")
                    .args(["url.dll,FileProtocolHandler", KEY_PAGE])
                    .spawn();
                Action::None
            }
            Message::Connect => self.connect(),
            Message::Connected(Err(error)) => {
                self.account.connecting = false;
                self.account.error = Some(error);
                Action::None
            }
            Message::Change => {
                self.account.editing = true;
                self.account.key.clear();
                Action::None
            }
        }
    }

    fn check_again(&mut self) -> Action {
        match start(FirstRun::check(None)) {
            Ok(gamepalate) => Action::Open(Box::new(gamepalate)),
            Err(setup) => {
                *self = *setup;
                Action::None
            }
        }
    }

    fn connect(&mut self) -> Action {
        if !can_connect(&self.account, &self.first_run) {
            return Action::None;
        }
        let (Some(dir), Ok(steam_id)) = (
            self.first_run.data_dir.clone(),
            parse_steam_id(&self.account.steam_id),
        ) else {
            return Action::None;
        };
        let key = Some(self.account.key.trim().to_string()).filter(|key| !key.is_empty());
        self.account.connecting = true;
        self.account.error = None;

        Action::Run(Task::perform(
            async move {
                connect(&dir, steam_id, key)
                    .await
                    .map_err(|error| connect_error_text(&error))
            },
            Message::Connected,
        ))
    }

    pub fn view(&self) -> Element<'_, Message> {
        let title = text("gamepalate needs a few things before it can recommend games.")
            .size(font::PRICE_SIZE)
            .font(font::SEMIBOLD);
        let folder: Element<'_, Message> = match &self.first_run.data_dir {
            Some(dir) => row![
                text!("Data folder: {}", dir.display()).size(font::BAR_SIZE),
                button(text("Open folder").size(font::BAR_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenFolder),
            ]
            .spacing(16)
            .align_y(Center)
            .into(),
            None => text("Could not find the user data folder: set GAMEPALATE_DATA_DIR.")
                .size(font::BAR_SIZE)
                .style(text::danger)
                .into(),
        };
        let lines = column(lines(&self.first_run).into_iter().map(line_view)).spacing(16);
        let error = self.error.as_ref().map(|error| {
            text!("gamepalate could not open the data files: {error}")
                .size(font::BAR_SIZE)
                .style(text::danger)
        });
        let check = button(text("Check again").size(font::BAR_SIZE))
            .padding([8, 20])
            .on_press(Message::CheckAgain);

        container(
            column![title, folder, self.account_view(), lines]
                .push(error)
                .push(check)
                .spacing(28)
                .max_width(WIDTH),
        )
        .padding(40)
        .center_x(Fill)
        .into()
    }

    fn account_view(&self) -> Element<'_, Message> {
        let account = &self.account;
        if let Some(steam_id) = self.first_run.saved_steam_id
            && account_done(account, &self.first_run)
        {
            return row![
                mark(true),
                text!("Steam account: {}", steam_id.0)
                    .size(font::INFO_SIZE)
                    .font(font::SEMIBOLD),
                button(text("Change").size(font::BAR_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::Change),
            ]
            .spacing(12)
            .align_y(Center)
            .into();
        }

        let editable = !account.connecting;
        let steam_id = text_input("Steam ID or profile link", &account.steam_id)
            .on_input_maybe(editable.then_some(Message::SteamId))
            .on_submit(Message::Connect)
            .padding(8)
            .size(font::BAR_SIZE);
        let steam_id_note = match steam_id_note(&account.steam_id) {
            Some(note) => text(note).size(font::NOTE_SIZE).style(text::danger),
            None => text(FIND_STEAM_ID)
                .size(font::NOTE_SIZE)
                .style(style::muted),
        };
        let key_placeholder = if self.first_run.api_key {
            "Saved: leave empty to keep it"
        } else {
            "Steam Web API key"
        };
        let key = row![
            text_input(key_placeholder, &account.key)
                .secure(true)
                .on_input_maybe(editable.then_some(Message::Key))
                .on_submit(Message::Connect)
                .padding(8)
                .size(font::BAR_SIZE),
            button(text("Get a key ↗").size(font::BAR_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::GetKey),
        ]
        .spacing(16)
        .align_y(Center);
        let invalid = match &self.first_run.steam_id {
            Part::Invalid(message) => Some(
                text(message.clone())
                    .size(font::NOTE_SIZE)
                    .style(text::danger),
            ),
            Part::Present | Part::Missing => None,
        };
        let connect = row![
            button(
                text(if account.connecting {
                    "Connecting…"
                } else {
                    "Connect"
                })
                .size(font::BAR_SIZE)
            )
            .padding([8, 20])
            .on_press_maybe(can_connect(account, &self.first_run).then_some(Message::Connect)),
        ]
        .push(
            account
                .error
                .as_ref()
                .map(|error| text(error.clone()).size(font::BAR_SIZE).style(text::danger)),
        )
        .spacing(16)
        .align_y(Center);

        column![
            row![
                mark(false),
                text("Steam account")
                    .size(font::INFO_SIZE)
                    .font(font::SEMIBOLD)
            ]
            .spacing(12)
            .align_y(Center),
            container(
                column![steam_id, steam_id_note, key]
                    .push(invalid)
                    .push(connect)
                    .spacing(8)
            )
            .padding(padding::left(24)),
        ]
        .spacing(8)
        .into()
    }
}

fn account_done(account: &Account, first_run: &FirstRun) -> bool {
    !account.editing && first_run.steam_id == Part::Present && first_run.api_key
}

fn can_connect(account: &Account, first_run: &FirstRun) -> bool {
    first_run.data_dir.is_some()
        && !account.connecting
        && parse_steam_id(&account.steam_id).is_ok()
        && (!account.key.trim().is_empty() || first_run.api_key)
}

fn steam_id_note(text: &str) -> Option<&'static str> {
    if text.trim().is_empty() {
        return None;
    }
    match parse_steam_id(text) {
        Ok(_) => None,
        Err(SteamIdError::CustomName) => {
            Some("That link has the custom name of the profile: write the Steam ID instead.")
        }
        Err(SteamIdError::Invalid) => Some("A Steam ID has 17 digits, like 76561198000000000."),
    }
}

fn connect_error_text(error: &ConnectError) -> String {
    let down = "Steam is not responding: try again in a minute.";
    match error.steam_error() {
        Some(SteamError::KeyRejected { .. }) => {
            "Steam rejected the key: check that you copied all of it.".to_string()
        }
        Some(SteamError::LibraryHidden { .. }) => "Your game details are private: open Steam → Profile → Edit Profile → Privacy Settings → Game details: Public.".to_string(),
        Some(SteamError::MissingKey { .. }) => "Write the Steam Web API key.".to_string(),
        Some(SteamError::SteamDown { .. }) => down.to_string(),
        _ if matches!(error, ConnectError::SteamDown) => down.to_string(),
        _ => error_chain(error),
    }
}

fn lines(first_run: &FirstRun) -> Vec<Line> {
    vec![
        part_line(
            "Catalog of Steam games",
            &first_run.catalog,
            "run: gamepalate update (15-25 minutes the first time)",
        ),
        part_line(
            "Your Signals",
            &first_run.signals,
            "connect your Steam account, or run: gamepalate profile",
        ),
    ]
}

fn part_line(label: &'static str, part: &Part, missing: &str) -> Line {
    Line {
        label,
        present: *part == Part::Present,
        hint: match part {
            Part::Present => None,
            Part::Missing => Some(missing.to_string()),
            Part::Invalid(message) => Some(message.clone()),
        },
    }
}

fn mark<'a>(present: bool) -> Element<'a, Message> {
    text(if present { CHECK } else { CROSS })
        .font(font::ICONS)
        .size(12)
        .style(if present { text::success } else { text::danger })
        .into()
}

fn line_view<'a>(line: Line) -> Element<'a, Message> {
    column![
        row![
            mark(line.present),
            text(line.label).size(font::INFO_SIZE).font(font::SEMIBOLD)
        ]
        .spacing(12)
        .align_y(Center)
    ]
    .push(line.hint.map(|hint| {
        container(text(hint).size(font::BAR_SIZE).style(style::muted)).padding(padding::left(24))
    }))
    .spacing(4)
    .into()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gamepalate_app::SteamId;

    use super::*;

    fn first_run() -> FirstRun {
        FirstRun {
            data_dir: Some(PathBuf::from("data")),
            steam_id: Part::Present,
            saved_steam_id: Some(SteamId(76_561_198_000_000_000)),
            api_key: true,
            catalog: Part::Present,
            signals: Part::Present,
        }
    }

    fn account(steam_id: &str, key: &str) -> Account {
        Account {
            steam_id: steam_id.to_string(),
            key: key.to_string(),
            ..Account::default()
        }
    }

    #[test]
    fn a_missing_part_opens_the_setup_without_trying_the_files() {
        let first_run = FirstRun {
            catalog: Part::Missing,
            ..first_run()
        };

        let Err(setup) = start(first_run.clone()) else {
            panic!("expected the setup");
        };

        assert_eq!(setup.first_run, first_run);
        assert_eq!(setup.error, None);
        assert_eq!(setup.account.steam_id, "76561198000000000");
    }

    #[test]
    fn files_that_do_not_open_show_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let first_run = FirstRun {
            data_dir: Some(dir.path().to_path_buf()),
            ..first_run()
        };

        let Err(setup) = start(first_run) else {
            panic!("expected the setup");
        };

        assert!(setup.error.unwrap().starts_with("config not found"));
    }

    #[test]
    fn everything_there_has_no_hints() {
        assert!(
            lines(&first_run())
                .iter()
                .all(|line| line.present && line.hint.is_none())
        );
    }

    #[test]
    fn a_missing_catalog_says_which_command_to_run() {
        let first_run = FirstRun {
            catalog: Part::Missing,
            ..first_run()
        };

        assert_eq!(
            lines(&first_run)[0],
            Line {
                label: "Catalog of Steam games",
                present: false,
                hint: Some("run: gamepalate update (15-25 minutes the first time)".to_string()),
            }
        );
    }

    #[test]
    fn an_invalid_file_shows_its_message() {
        let first_run = FirstRun {
            signals: Part::Invalid("the signals file is not valid".to_string()),
            ..first_run()
        };

        let line = &lines(&first_run)[1];

        assert!(!line.present);
        assert_eq!(line.hint.as_deref(), Some("the signals file is not valid"));
    }

    #[test]
    fn connect_needs_a_steam_id_and_a_key() {
        let without_key = FirstRun {
            api_key: false,
            ..first_run()
        };

        assert!(can_connect(
            &account("76561198000000000", "KEY"),
            &without_key
        ));
        assert!(!can_connect(
            &account("76561198000000000", " "),
            &without_key
        ));
        assert!(!can_connect(
            &account("7656119800000000", "KEY"),
            &without_key
        ));
        assert!(!can_connect(&account("", ""), &without_key));
        assert!(can_connect(&account("76561198000000000", ""), &first_run()));
    }

    #[test]
    fn connect_is_off_while_connecting_or_without_a_folder() {
        let connecting = Account {
            connecting: true,
            ..account("76561198000000000", "KEY")
        };
        let without_folder = FirstRun {
            data_dir: None,
            ..first_run()
        };

        assert!(!can_connect(&connecting, &first_run()));
        assert!(!can_connect(
            &account("76561198000000000", "KEY"),
            &without_folder
        ));
    }

    #[test]
    fn the_account_is_done_until_change() {
        let mut account = account("76561198000000000", "");

        assert!(account_done(&account, &first_run()));
        account.editing = true;
        assert!(!account_done(&account, &first_run()));
        assert!(!account_done(
            &Account::default(),
            &FirstRun {
                api_key: false,
                ..first_run()
            }
        ));
    }

    #[test]
    fn the_steam_id_field_says_what_is_wrong() {
        assert_eq!(steam_id_note(""), None);
        assert_eq!(steam_id_note("76561198000000000"), None);
        assert!(steam_id_note("123").unwrap().contains("17 digits"));
        assert!(
            steam_id_note("https://steamcommunity.com/id/someone")
                .unwrap()
                .contains("custom name")
        );
    }

    #[test]
    fn each_connection_error_has_its_sentence() {
        let hidden = ConnectError::Steam(SteamError::LibraryHidden {
            steam_id: 76_561_198_000_000_000,
        });
        let down = ConnectError::Steam(SteamError::SteamDown {
            endpoint: "IPlayerService/GetOwnedGames/v1".to_string(),
            reason: "timed out".to_string(),
            source: None,
        });
        let missing = ConnectError::Steam(SteamError::MissingKey {
            endpoint: "IPlayerService/GetOwnedGames/v1".to_string(),
        });

        assert!(connect_error_text(&hidden).starts_with("Your game details are private"));
        assert!(connect_error_text(&down).starts_with("Steam is not responding"));
        assert!(
            connect_error_text(&ConnectError::SteamDown).starts_with("Steam is not responding")
        );
        assert_eq!(connect_error_text(&missing), "Write the Steam Web API key.");
    }
}
