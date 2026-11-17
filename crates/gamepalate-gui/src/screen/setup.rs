use std::{
    cmp::Ordering,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{
    AppId, Gamepalate, Region, SteamError, SteamId, ThemeChoice, Timestamp,
    account::{ConnectError, connect, parse_profile},
    catalog_file::{CatalogFileError, ImportedCatalog, import_catalog},
    ready::{ReadyCheck, check_ready},
    setup::{Covers, FirstRun, Part, error_chain, most_played, steam_accounts},
    steam_client::{
        ClientAccount, answer_steam_accounts, answer_steam_client, local_config_shown,
        login_users_file, steam_accounts_answer, steam_client_hours_answer,
    },
    update::download_catalog,
};
use iced::{
    Center, ContentFit, Element, Event, Fill, Padding, Subscription, Task, Theme, border, event,
    futures::{SinkExt, channel::mpsc},
    keyboard::{self, Key, key::Named},
    padding, stream,
    widget::{
        button, column, container, image, progress_bar, radio, row, scrollable, space, stack, text,
        text_input, toggler,
    },
};

use self::accounts::Accounts;
use super::library::hours;
use crate::{
    app::theme::theme,
    font, format,
    progress::{Stage, Tracker, update_error_text},
    style,
};

const REFRESH: char = '\u{E5D5}';
const SHEET_WIDTH: f32 = 720.0;
const SHEET_HEIGHT: f32 = 520.0;
const SHEET_MARGIN: f32 = 32.0;
const HEAD_HEIGHT: f32 = 56.0;
const FOOT_HEIGHT: f32 = 64.0;
const DOT: f32 = 12.0;
const DOT_CENTER: f32 = 6.0;
const DOT_LINE: f32 = 48.0;
const LABEL_WIDTH: f32 = 96.0;
const PLAYED_ASKED: usize = 12;
const PLAYED_SHOWN: usize = 5;
const TILE_WIDTH: f32 = 116.0;
const TILE_HEIGHT: f32 = 174.0;
const IMAGE_RADIUS: f32 = 6.0;
const KEY_PAGE: &str = "https://steamcommunity.com/dev/apikey";
const ACCOUNT_ABOUT: &str = "gamepalate only reads your library, hours played and wishlist";
const PREPARING: &str = "Looking up the tags of your games that are no longer on the store";
const LIBRARY_PAGE: &str = "steam://open/games";
pub(crate) const PRIVACY_PAGE: &str = "https://steamcommunity.com/my/edit/settings";
pub(crate) const PRIVATE_LIBRARY: &str =
    "Your game details are private. Set Game details to Public in your privacy settings.";
pub(crate) const KEY_DOMAIN_NOTE: &str =
    "Steam asks for a domain name. Any will do, like localhost.";
pub(crate) const PROFILE_PLACEHOLDER: &str = "Steam profile address or Steam ID";
const FIND_STEAM_ID: &str = "Open your profile on steamcommunity.com and copy the address";
const DROP_CATALOG: &str = "Have a catalog file? Drop it on this window.";
const IMPORTING: &str = "Importing the catalog…";
const DROP_TO_IMPORT: &str = "Drop to import";
const LOGO_BYTES: &[u8] = include_bytes!("../../gamepalate.png");
const LOGO_SIZE: f32 = 96.0;
const WELCOME_ABOUT: &str =
    "Recommends games from the Steam catalog, based on the hours you play and the votes you give";
const WELCOME_WIDTH: f32 = 480.0;
const THEME_ABOUT: &str = "You can change it later in Settings";
const THIS_PC_ABOUT: &str = "Two files of the Steam app can fill in your account and your hours";
const THIS_PC_SAFE: &str = "Nothing is sent anywhere and the files are never changed";
const ACCOUNTS_TITLE: &str = "Your Steam accounts";
const ACCOUNTS_ABOUT: &str = "Fills in your Steam ID from the accounts signed in here";
const HOURS_TITLE: &str = "Games you played but don't own";
const HOURS_ABOUT: &str =
    "Adds the hours of Family Sharing games, free weekends and free games you removed";
const ONLINE_TITLE: &str = "What goes online";
const ONLINE_ABOUT: &str = "Steam, for your library and the catalog · GitHub, to check for new versions · ProtonDB, for Steam Deck reports";
const BODY_WIDTH: f32 = 600.0;
const VANITY_NOTE: &str = "Checked after the API key";
const TOGGLER_SIZE: f32 = 24.0;
const KEY_ABOUT: &str = "gamepalate uses it to read your library from Steam";
const KEY_STEPS: [&str; 3] = [
    "Open the key page on Steam",
    "Type any domain name, like localhost",
    "Copy the key and paste it below",
];
const KEY_STEP_HEIGHT: f32 = 128.0;
const KEY_WARNING: &str =
    "Steam gives keys only to accounts with Steam Guard on and at least $5 spent in the store";
const KEY_KEYRING_NOTE: &str = "Saved in the system keyring, never in a file";
const WARNING: char = '\u{E002}';
const KEY_REJECTED: &str = "Steam rejected the key. Check that you copied all of it.";
const NO_PROFILE: &str = "Steam has no profile at that address. Check the name after /id/.";
const PREVIEW_WIDTH: f32 = 300.0;
const PREVIEW_HEIGHT: f32 = 180.0;
const PREVIEW_BAR: f32 = 24.0;
const PREVIEW_RADIUS: f32 = 6.0;

mod accounts;

pub struct Setup {
    first_run: FirstRun,
    error: Option<String>,
    page: Page,
    account: Account,
    accounts: Accounts,
    this_pc: ThisPc,
    download: Download,
    import: Import,
    ready: Option<Ready>,
    covers: Option<Arc<Covers>>,
    played: Vec<Played>,
    theme: ThemeChoice,
}

#[derive(Debug)]
struct Played {
    id: AppId,
    hours: f64,
    cover: Cover,
}

#[derive(Debug)]
enum Cover {
    Loading,
    Ready(image::Handle),
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Page {
    Welcome,
    Theme,
    ThisPc,
    Account,
    Key,
    Region,
    Catalog,
    Profile,
}

impl Page {
    const ALL: [Page; 8] = [
        Page::Welcome,
        Page::Theme,
        Page::ThisPc,
        Page::Account,
        Page::Key,
        Page::Region,
        Page::Catalog,
        Page::Profile,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepState {
    Done,
    Current,
    Next,
}

#[derive(Clone, Debug)]
struct Primary {
    label: &'static str,
    message: Option<Message>,
}

impl Primary {
    fn new(label: &'static str, message: Option<Message>) -> Primary {
        Primary { label, message }
    }

    fn proceed() -> Primary {
        Primary::new("Continue", Some(Message::Continue))
    }
}

#[derive(Debug)]
enum Ready {
    Preparing,
    Empty { min_hours: f64 },
    Failed(String),
}

#[derive(Debug, Default)]
struct Download {
    running: bool,
    tracker: Tracker,
    error: Option<String>,
}

#[derive(Debug, Default)]
struct Import {
    running: bool,
    hovering: bool,
    result: Option<Result<ImportedCatalog, String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPlace {
    Account,
    Catalog,
    Problems,
}

#[derive(Debug, Default)]
struct ThisPc {
    accounts: bool,
    hours: bool,
    saving: bool,
    error: Option<String>,
    accounts_file: Option<PathBuf>,
    hours_file: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct Account {
    steam_id: String,
    key: String,
    connecting: bool,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    CheckAgain,
    OpenFolder,
    Continue,
    Back,
    Enter,
    Theme(ThemeChoice),
    ThisPcAccounts(bool),
    ThisPcHours(bool),
    ThisPcSaved(Result<Vec<ClientAccount>, String>),
    Accounts(accounts::Message),
    SteamId(String),
    Key(String),
    GetKey,
    Paste,
    Pasted(Option<String>),
    Connect,
    Connected(Result<(), String>),
    Change,
    Download,
    Progress(gamepalate_app::update::Progress),
    Downloaded(Result<(), String>),
    Hovering(bool),
    Import(PathBuf),
    Imported(Result<ImportedCatalog, String>),
    Prepare(bool),
    Prepared(Result<ReadyCheck, String>),
    OpenLibrary,
    OpenPrivacy,
    Cover(AppId, Option<image::Handle>),
}

pub enum Action {
    None,
    Run(Task<Message>),
    Open(Box<Gamepalate>),
    Theme(ThemeChoice),
}

pub fn start(first_run: FirstRun) -> Result<Gamepalate, Box<Setup>> {
    if !first_run.ready() {
        return Err(Box::new(Setup::new(first_run, None)));
    }

    Gamepalate::open(first_run.data_dir.clone()).map_err(|error| {
        let message = error_chain(&error);
        Box::new(if error.unreadable_catalog() {
            let first_run = FirstRun {
                catalog: Part::Invalid(message),
                ..first_run
            };
            Setup::new(first_run, None)
        } else {
            Setup::new(first_run, Some(message))
        })
    })
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

        let played = first_run
            .data_dir
            .as_deref()
            .map(|dir| most_played(dir, PLAYED_ASKED))
            .unwrap_or_default()
            .into_iter()
            .map(|(id, hours)| Played {
                id,
                hours,
                cover: Cover::Loading,
            })
            .collect();
        let theme = first_run.gui_settings().theme;
        let this_pc = this_pc(&first_run);
        let accounts = Accounts::new(
            first_run
                .data_dir
                .as_deref()
                .map(steam_accounts)
                .unwrap_or_default(),
            first_run.saved_steam_id,
        );
        let account = Account {
            steam_id: accounts
                .selected()
                .map_or(account.steam_id, |id| id.0.to_string()),
            ..account
        };

        Setup {
            page: first_page(&first_run),
            first_run,
            error,
            account,
            accounts,
            this_pc,
            download: Download::default(),
            import: Import::default(),
            ready: None,
            covers: Covers::new().ok().map(Arc::new),
            played,
            theme,
        }
    }

    pub fn begin(&mut self) -> Task<Message> {
        let covers = self.load_covers();
        if !downloads_at_start(&self.first_run) {
            return covers;
        }

        match self.download() {
            Action::Run(download) => Task::batch([covers, download]),
            Action::None | Action::Open(_) | Action::Theme(_) => covers,
        }
    }

    fn load_covers(&self) -> Task<Message> {
        let Some(covers) = &self.covers else {
            return Task::none();
        };

        Task::batch(self.played.iter().map(|game| {
            let covers = Arc::clone(covers);
            let id = game.id;
            Task::perform(
                async move {
                    covers.capsule(id).await.ok().map(|capsule| {
                        image::Handle::from_rgba(capsule.width, capsule.height, capsule.rgba)
                    })
                },
                move |cover| Message::Cover(id, cover),
            )
        }))
    }

    pub fn first_run(&self) -> &FirstRun {
        &self.first_run
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::CheckAgain => self.check_again(),
            Message::Connected(Ok(())) => self.connected(),
            Message::OpenFolder => {
                if let Some(dir) = &self.first_run.data_dir {
                    let _ = std::fs::create_dir_all(dir);
                    super::open(dir);
                }
                Action::None
            }
            Message::Continue if self.page == Page::ThisPc => self.save_this_pc(),
            Message::Continue => self.go_on(),
            Message::Back => {
                if let Some(page) = self.back() {
                    self.page = page;
                }
                Action::None
            }
            Message::Theme(choice) if choice == self.theme => Action::None,
            Message::Theme(choice) => {
                self.theme = choice;
                let _ = self
                    .first_run
                    .change_gui_settings(move |settings| settings.theme = choice);
                Action::Theme(choice)
            }
            Message::Enter => match self.primary().and_then(|primary| primary.message) {
                Some(message) => self.update(message),
                None => Action::None,
            },
            Message::ThisPcAccounts(read) => {
                self.this_pc.accounts = read;
                Action::None
            }
            Message::ThisPcHours(read) => {
                self.this_pc.hours = read;
                Action::None
            }
            Message::ThisPcSaved(Ok(list)) => self.this_pc_saved(list),
            Message::ThisPcSaved(Err(error)) => {
                self.this_pc.saving = false;
                self.this_pc.error = Some(error);
                Action::None
            }
            Message::Accounts(message) => {
                self.accounts.update(message);
                self.account.error = None;
                match message {
                    accounts::Message::UseAnother => self.account.steam_id.clear(),
                    accounts::Message::Select(_) | accounts::Message::BackToList => {
                        if let Some(id) = self.accounts.selected() {
                            self.account.steam_id = id.0.to_string();
                        }
                    }
                }
                Action::None
            }
            Message::SteamId(steam_id) => {
                self.account.steam_id = steam_id;
                Action::None
            }
            Message::Key(key) => {
                self.account.key = key;
                self.account.error = None;
                Action::None
            }
            Message::Paste if !self.account.connecting => {
                Action::Run(iced::clipboard::read().map(Message::Pasted))
            }
            Message::Paste => Action::None,
            Message::Pasted(Some(text)) if !self.account.connecting => {
                self.account.key = text.trim().to_string();
                self.account.error = None;
                Action::None
            }
            Message::Pasted(_) => Action::None,
            Message::GetKey => open_link(KEY_PAGE),
            Message::OpenLibrary => open_link(LIBRARY_PAGE),
            Message::OpenPrivacy => open_link(PRIVACY_PAGE),
            Message::Connect => self.connect(),
            Message::Connected(Err(error)) => {
                self.account.connecting = false;
                self.account.error = Some(error);
                Action::None
            }
            Message::Change => {
                if !self.download.running {
                    self.page = Page::Account;
                }
                Action::None
            }
            Message::Download => self.download(),
            Message::Progress(progress) => {
                self.download.tracker.advance(progress, Instant::now());
                Action::None
            }
            Message::Downloaded(Ok(())) => {
                self.first_run = FirstRun::check(None);
                self.download = Download::default();
                self.page = page_from(self.page, &self.first_run);
                self.prepare(false)
            }
            Message::Hovering(hovering) => {
                self.import.hovering = hovering;
                Action::None
            }
            Message::Import(file) => {
                self.import.hovering = false;
                self.import(file)
            }
            Message::Imported(result) => self.imported(result),
            Message::Prepare(read_signals_again) => self.prepare(read_signals_again),
            Message::Prepared(Ok(check)) if !check.empty_profile => self.check_again(),
            Message::Prepared(result) => {
                self.ready = Some(match result {
                    Ok(check) => Ready::Empty {
                        min_hours: check.min_hours,
                    },
                    Err(error) => Ready::Failed(error),
                });
                Action::None
            }
            Message::Cover(id, cover) => {
                if let Some(game) = self.played.iter_mut().find(|game| game.id == id) {
                    game.cover = cover.map_or(Cover::Missing, Cover::Ready);
                }
                Action::None
            }
            Message::Downloaded(Err(error)) => {
                self.first_run = FirstRun::check(None);
                self.download.running = false;
                self.download.error = Some(error);
                Action::None
            }
        }
    }

    fn back(&self) -> Option<Page> {
        let busy = self.download.running
            || self.this_pc.saving
            || self.account.connecting
            || self.import.running
            || matches!(self.ready, Some(Ready::Preparing));
        if busy {
            return None;
        }
        previous_page(self.page, &self.first_run)
    }

    fn primary(&self) -> Option<Primary> {
        let account = &self.account;
        match self.page {
            Page::Account => Some(Primary::new(
                "Continue",
                (!self.import.running && parse_profile(&account.steam_id).is_ok())
                    .then_some(Message::Continue),
            )),
            Page::Key => Some(Primary::new(
                if account.connecting {
                    "Connecting…"
                } else {
                    "Connect"
                },
                (!self.import.running && can_connect(account, &self.first_run))
                    .then_some(Message::Connect),
            )),
            Page::Catalog if self.download.running => None,
            Page::Catalog if self.first_run.catalog == Part::Present => Some(Primary::proceed()),
            Page::Catalog => Some(Primary::new(
                if self.download.error.is_some() {
                    "Retry"
                } else {
                    "Download catalog"
                },
                (!self.import.running).then_some(Message::Download),
            )),
            Page::Profile => match &self.ready {
                None => Some(Primary::new(
                    "Read my library",
                    Some(Message::Prepare(true)),
                )),
                Some(Ready::Preparing) => None,
                Some(Ready::Failed(_)) => {
                    Some(Primary::new("Retry", Some(Message::Prepare(false))))
                }
                Some(Ready::Empty { .. }) => {
                    Some(Primary::new("Check again", Some(Message::Prepare(true))))
                }
            },
            Page::Welcome => Some(Primary::new("Get started", Some(Message::Continue))),
            Page::ThisPc => Some(Primary::new(
                if self.this_pc.saving {
                    "Saving…"
                } else {
                    "Continue"
                },
                (!self.this_pc.saving).then_some(Message::Continue),
            )),
            Page::Theme | Page::Region => Some(Primary::proceed()),
        }
    }

    fn save_this_pc(&mut self) -> Action {
        if self.this_pc.saving {
            return Action::None;
        }
        let Some(dir) = self.first_run.data_dir.clone() else {
            return self.go_on();
        };
        self.this_pc.saving = true;
        self.this_pc.error = None;
        let (accounts, hours) = (self.this_pc.accounts, self.this_pc.hours);

        Action::Run(Task::perform(
            async move { save_this_pc(&dir, accounts, hours) },
            Message::ThisPcSaved,
        ))
    }

    fn this_pc_saved(&mut self, list: Vec<ClientAccount>) -> Action {
        self.this_pc.saving = false;
        let typed = self.account.steam_id.trim().parse().ok().map(SteamId);
        self.accounts = Accounts::new(list, typed);
        if let Some(id) = self.accounts.selected() {
            self.account.steam_id = id.0.to_string();
        }
        self.go_on()
    }

    fn go_on(&mut self) -> Action {
        if self.page == Page::Account
            && account_done(&self.first_run)
            && changes_account(&self.account, &self.first_run)
        {
            self.first_run.steam_id = Part::Missing;
        }
        if let Some(page) = next_page(self.page, &self.first_run) {
            self.page = page;
        }
        Action::None
    }

    fn prepare(&mut self, read_signals_again: bool) -> Action {
        let Some(dir) = self.first_run.data_dir.clone() else {
            return Action::None;
        };
        self.ready = Some(Ready::Preparing);

        Action::Run(Task::perform(
            async move {
                check_ready(&dir, read_signals_again)
                    .await
                    .map_err(|error| error_chain(&error))
            },
            Message::Prepared,
        ))
    }

    fn download(&mut self) -> Action {
        let Some(dir) = self.first_run.data_dir.clone() else {
            return Action::None;
        };
        if self.download.running || self.import.running || !account_done(&self.first_run) {
            return Action::None;
        }
        self.download = Download {
            running: true,
            ..Download::default()
        };

        Action::Run(Task::run(
            stream::channel(64, async move |mut output: mpsc::Sender<Message>| {
                let mut events = output.clone();
                let result = download_catalog(&dir, move |progress| {
                    let _ = events.try_send(Message::Progress(progress));
                })
                .await
                .map(|_| ())
                .map_err(|error| update_error_text(&error));
                let _ = output.send(Message::Downloaded(result)).await;
            }),
            |message| message,
        ))
    }

    fn check_again(&mut self) -> Action {
        match start(FirstRun::check(None)) {
            Ok(gamepalate) => Action::Open(Box::new(gamepalate)),
            Err(setup) => {
                *self = *setup;
                Action::Run(self.begin())
            }
        }
    }

    fn connected(&mut self) -> Action {
        let first_run = FirstRun::check(None);
        if !prepares_after_connect(&first_run) {
            return self.check_again();
        }

        *self = Setup::new(first_run, None);
        let covers = self.begin();
        match self.prepare(false) {
            Action::Run(prepare) => Action::Run(Task::batch([covers, prepare])),
            Action::None | Action::Open(_) | Action::Theme(_) => Action::Run(covers),
        }
    }

    fn import_place(&self) -> Option<ImportPlace> {
        import_place(self.page, &self.first_run, &self.download)
    }

    fn import(&mut self, file: PathBuf) -> Action {
        let Some(dir) = self.first_run.data_dir.clone() else {
            return Action::None;
        };
        if !can_import(self.import_place(), &self.import, &self.account) {
            return Action::None;
        }
        self.import = Import {
            running: true,
            ..Import::default()
        };

        Action::Run(Task::perform(
            async move { import_catalog(&dir, &file).map_err(|error| import_error_text(&error)) },
            Message::Imported,
        ))
    }

    fn imported(&mut self, result: Result<ImportedCatalog, String>) -> Action {
        let succeeded = result.is_ok();
        self.import = Import {
            result: Some(result),
            ..Import::default()
        };
        if !succeeded {
            return Action::None;
        }

        self.first_run = FirstRun::check(None);
        self.page = page_from(self.page, &self.first_run);
        if account_done(&self.first_run) {
            self.prepare(false)
        } else {
            Action::None
        }
    }

    fn connect(&mut self) -> Action {
        if self.import.running || !can_connect(&self.account, &self.first_run) {
            return Action::None;
        }
        let (Some(dir), Ok(profile)) = (
            self.first_run.data_dir.clone(),
            parse_profile(&self.account.steam_id),
        ) else {
            return Action::None;
        };
        let key = Some(self.account.key.trim().to_string()).filter(|key| !key.is_empty());
        self.account.connecting = true;
        self.account.error = None;

        Action::Run(Task::perform(
            async move {
                connect(&dir, profile, key)
                    .await
                    .map_err(|error| connect_error_text(&error))
            },
            Message::Connected,
        ))
    }

    pub fn subscription() -> Subscription<Message> {
        event::listen_with(window_message)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let sheet = column![self.head_view(), self.body_view(), self.foot_view()];

        container(
            container(sheet)
                .width(Fill)
                .max_width(SHEET_WIDTH)
                .height(Fill)
                .max_height(SHEET_HEIGHT)
                .style(style::card),
        )
        .padding(24)
        .center(Fill)
        .into()
    }

    fn head_view(&self) -> Element<'_, Message> {
        let content = step_of(self.page, &self.first_run).map(|(number, count)| {
            row![
                self.dots_view(),
                space().width(Fill),
                note(format!("Step {number} of {count}")),
            ]
            .align_y(Center)
        });

        container(column![].push(content))
            .width(Fill)
            .height(HEAD_HEIGHT)
            .padding([0.0, SHEET_MARGIN])
            .align_y(Center)
            .into()
    }

    fn dots_view(&self) -> Element<'_, Message> {
        let mut dots = row![].spacing(8).align_y(Center);
        for (index, page) in counted_pages(&self.first_run).enumerate() {
            let state = step_state(page, self.page, page_done(page, &self.first_run));
            if index > 0 {
                dots = dots.push(
                    container(column![])
                        .width(DOT_LINE)
                        .height(2)
                        .style(style::step_line(state != StepState::Next)),
                );
            }
            dots = dots.push(dot(state));
        }
        dots.into()
    }

    fn body_view(&self) -> Element<'_, Message> {
        if self.page == Page::Welcome {
            return self.welcome_view();
        }
        let content = column![]
            .push(self.page_view())
            .push(self.most_played_view())
            .spacing(16);

        scrollable(
            container(
                column![]
                    .push(self.problems_view())
                    .push(self.title_view())
                    .push(content)
                    .spacing(24),
            )
            .width(Fill)
            .padding(Padding {
                top: 8.0,
                right: SHEET_MARGIN,
                bottom: 24.0,
                left: SHEET_MARGIN,
            }),
        )
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn foot_view(&self) -> Element<'_, Message> {
        let back: Element<'_, Message> = match self.back() {
            Some(_) => button(text("Back").size(font::BAR_SIZE))
                .padding([8, 0])
                .style(style::link)
                .on_press(Message::Back)
                .into(),
            None if self.page == Page::Welcome => text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(font::BAR_SIZE)
                .style(style::muted)
                .into(),
            None => space().into(),
        };
        let primary = self.primary().map(|primary| {
            button(text(primary.label).size(font::BAR_SIZE))
                .style(style::primary)
                .padding([10, 28])
                .on_press_maybe(primary.message)
        });

        column![
            style::separator(),
            container(
                row![back, space().width(Fill)]
                    .push(primary)
                    .align_y(Center)
            )
            .width(Fill)
            .height(FOOT_HEIGHT - 1.0)
            .padding([0.0, SHEET_MARGIN])
            .align_y(Center),
        ]
        .into()
    }

    fn problems_view(&self) -> Option<Element<'_, Message>> {
        let problems = file_problems(&self.first_run, self.error.as_deref());
        if problems.is_empty() {
            return None;
        }
        let actions = row![
            button(text("Check again").size(font::BAR_SIZE))
                .style(style::secondary)
                .padding([8, 20])
                .on_press(Message::CheckAgain)
        ]
        .push(
            self.first_run
                .data_dir
                .is_some()
                .then(|| link("Open data folder", Message::OpenFolder)),
        )
        .spacing(16)
        .align_y(Center);

        Some(
            container(
                column![
                    column(problems.into_iter().map(|problem| {
                        text(problem)
                            .size(font::BAR_SIZE)
                            .style(text::danger)
                            .into()
                    }))
                    .spacing(8)
                ]
                .push(
                    (self.import_place() == Some(ImportPlace::Problems))
                        .then(|| self.import_view()),
                )
                .push(actions)
                .spacing(16),
            )
            .padding(16)
            .width(Fill)
            .style(style::card)
            .into(),
        )
    }

    fn title_view(&self) -> Element<'_, Message> {
        let (title, about) = match self.page {
            Page::Account => ("Steam account", Some(ACCOUNT_ABOUT)),
            Page::Key => ("Steam Web API key", Some(KEY_ABOUT)),
            Page::Catalog if self.download.running => {
                return row![container(page_title("Downloading the Steam catalog")).width(Fill)]
                    .push(self.download.tracker.percent().map(page_title))
                    .align_y(Center)
                    .into();
            }
            Page::Catalog => ("Steam catalog", None),
            Page::Profile => (ready_title(self.ready.as_ref()), None),
            Page::Theme => ("Theme", Some(THEME_ABOUT)),
            Page::ThisPc => ("Files on this PC", Some(THIS_PC_ABOUT)),
            Page::Welcome | Page::Region => {
                return column![].into();
            }
        };

        column![page_title(title)]
            .push(about.map(|about| text(about).size(font::INFO_SIZE).style(style::muted)))
            .spacing(8)
            .into()
    }

    fn page_view(&self) -> Element<'_, Message> {
        match self.page {
            Page::Account => self.account_view(),
            Page::Key => self.key_view(),
            Page::Catalog if self.download.running => self.progress_view(),
            Page::Catalog => self.start_view(),
            Page::Profile => self.ready_view(),
            Page::Theme => self.theme_view(),
            Page::ThisPc => self.this_pc_view(),
            Page::Welcome | Page::Region => column![].into(),
        }
    }

    fn welcome_view(&self) -> Element<'_, Message> {
        let logo = image(image::Handle::from_bytes(LOGO_BYTES))
            .width(LOGO_SIZE)
            .height(LOGO_SIZE)
            .filter_method(image::FilterMethod::Linear);

        container(
            column![
                logo,
                column![
                    text("gamepalate")
                        .size(font::TITLE_SIZE)
                        .font(font::SEMIBOLD),
                    container(
                        text(WELCOME_ABOUT)
                            .size(font::INFO_SIZE)
                            .style(style::muted)
                            .align_x(Center),
                    )
                    .max_width(WELCOME_WIDTH),
                ]
                .spacing(8)
                .align_x(Center),
            ]
            .spacing(24)
            .align_x(Center),
        )
        .padding(SHEET_MARGIN)
        .center(Fill)
        .into()
    }

    fn theme_view(&self) -> Element<'_, Message> {
        container(
            row![
                self.theme_choice("Light", ThemeChoice::Light),
                self.theme_choice("Dark", ThemeChoice::Dark),
            ]
            .spacing(SHEET_MARGIN),
        )
        .width(Fill)
        .center_x(Fill)
        .into()
    }

    fn theme_choice(&self, label: &'static str, choice: ThemeChoice) -> Element<'_, Message> {
        column![
            button(preview(choice))
                .padding(0)
                .style(preview_button(self.theme == choice))
                .on_press(Message::Theme(choice)),
            radio(label, choice, Some(self.theme), Message::Theme).size(16),
        ]
        .spacing(16)
        .align_x(Center)
        .into()
    }

    fn most_played_view(&self) -> Option<Element<'_, Message>> {
        if self.page != Page::Catalog {
            return None;
        }
        let tiles: Vec<Element<'_, Message>> = ready_covers(&self.played)?
            .into_iter()
            .map(|(handle, hours)| cover_tile(handle, hours))
            .collect();

        Some(
            column![
                column![
                    text("Your most played")
                        .size(font::INFO_SIZE)
                        .font(font::SEMIBOLD),
                    note("From your Steam library"),
                ]
                .spacing(8),
                row(tiles).spacing(16),
            ]
            .spacing(16)
            .into(),
        )
    }

    fn this_pc_view(&self) -> Element<'_, Message> {
        let pc = &self.this_pc;
        let group = container(
            column![
                permission(
                    ACCOUNTS_TITLE,
                    ACCOUNTS_ABOUT,
                    pc.accounts_file.as_deref(),
                    pc.accounts,
                    Message::ThisPcAccounts,
                ),
                style::separator(),
                permission(
                    HOURS_TITLE,
                    HOURS_ABOUT,
                    pc.hours_file.as_deref(),
                    pc.hours,
                    Message::ThisPcHours,
                ),
            ]
            .spacing(16),
        )
        .padding(16)
        .width(Fill)
        .style(style::card);
        let error = pc
            .error
            .as_ref()
            .map(|error| text(error.clone()).size(font::BAR_SIZE).style(text::danger));

        column![
            column![group, note(THIS_PC_SAFE)].spacing(8),
            column![
                text(ONLINE_TITLE).size(font::BAR_SIZE).font(font::SEMIBOLD),
                container(note(ONLINE_ABOUT)).max_width(BODY_WIDTH),
            ]
            .spacing(8),
        ]
        .push(error)
        .spacing(16)
        .into()
    }

    fn account_view(&self) -> Element<'_, Message> {
        let account = &self.account;
        let editable = !account.connecting;
        let steam_id = text_input(PROFILE_PLACEHOLDER, &account.steam_id)
            .style(style::field)
            .on_input_maybe(editable.then_some(Message::SteamId))
            .on_submit(Message::Enter)
            .padding(8)
            .size(font::BAR_SIZE);
        let steam_id_note = match (
            steam_id_note(&account.steam_id),
            is_vanity(&account.steam_id),
        ) {
            (Some(note), _) => text(note).size(font::NOTE_SIZE).style(text::danger),
            (None, true) => text(VANITY_NOTE).size(font::NOTE_SIZE).style(style::muted),
            (None, false) => text(FIND_STEAM_ID)
                .size(font::NOTE_SIZE)
                .style(style::muted),
        };
        let invalid = match &self.first_run.steam_id {
            Part::Invalid(message) => Some(
                text(message.clone())
                    .size(font::NOTE_SIZE)
                    .style(text::danger),
            ),
            Part::Present | Part::Missing => None,
        };
        let profile: Element<'_, Message> = if self.accounts.listed() {
            self.accounts.view(Message::Accounts)
        } else {
            field(
                "Profile",
                column![steam_id, steam_id_note]
                    .push(
                        (!self.accounts.is_empty()).then(|| Accounts::back_link(Message::Accounts)),
                    )
                    .spacing(8),
            )
        };

        column![profile]
            .push(invalid)
            .push((self.import_place() == Some(ImportPlace::Account)).then(|| self.import_view()))
            .spacing(16)
            .into()
    }

    fn key_view(&self) -> Element<'_, Message> {
        let account = &self.account;
        let editable = !account.connecting;
        let error = account.error.as_deref();
        let rejected = error.is_some_and(is_key_rejected);
        let steps = row(KEY_STEPS.iter().enumerate().map(|(index, line)| {
            let open = (index == 0).then(|| link("Open page ↗", Message::GetKey));
            key_step(index + 1, line, open)
        }))
        .spacing(16);
        let warning = container(
            row![
                text(WARNING)
                    .font(font::ICONS)
                    .size(font::icon(font::BAR_SIZE)),
                text(KEY_WARNING).size(font::BAR_SIZE),
            ]
            .spacing(16)
            .align_y(Center),
        )
        .padding(16)
        .width(Fill)
        .style(style::card);
        let placeholder = if self.first_run.api_key {
            "Already saved"
        } else {
            "Steam Web API key"
        };
        let input = row![
            text_input(placeholder, &account.key)
                .style(if rejected {
                    style::wrong_field
                } else {
                    style::field
                })
                .secure(true)
                .on_input_maybe(editable.then_some(Message::Key))
                .on_submit(Message::Enter)
                .padding(8)
                .size(font::BAR_SIZE),
            link("Paste", Message::Paste),
        ]
        .spacing(16)
        .align_y(Center);
        let problem = error.map(|error| {
            column![
                text(error.to_string())
                    .size(font::BAR_SIZE)
                    .style(text::danger)
            ]
            .push(is_unknown_profile(error).then(|| note_link("Change account", Message::Change)))
            .push(
                needs_privacy(error)
                    .then(|| note_link("Open privacy settings ↗", Message::OpenPrivacy)),
            )
            .spacing(8)
        });

        column![
            steps,
            warning,
            field(
                "API key",
                column![input, note(KEY_KEYRING_NOTE)]
                    .push(problem)
                    .spacing(8)
            ),
        ]
        .spacing(16)
        .into()
    }
}

fn key_step<'a>(
    number: usize,
    line: &'a str,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(
        column![
            text(number.to_string())
                .size(font::AMOUNT_SIZE)
                .style(style::accent),
            text(line).size(font::BAR_SIZE),
        ]
        .push(action)
        .spacing(8),
    )
    .padding(16)
    .width(Fill)
    .height(KEY_STEP_HEIGHT)
    .style(style::card)
    .into()
}

impl Setup {
    fn ready_view(&self) -> Element<'_, Message> {
        match &self.ready {
            None => column![].into(),
            Some(Ready::Preparing) => note(PREPARING).into(),
            Some(Ready::Failed(error)) => text(error.clone())
                .size(font::BAR_SIZE)
                .style(text::danger)
                .into(),
            Some(Ready::Empty { min_hours }) => column![
                column![
                    text(empty_profile_text(*min_hours)).size(font::BAR_SIZE),
                    note_link("Wrong account? Change it", Message::Change),
                ]
                .spacing(8),
                button(text("Open Steam library").size(font::BAR_SIZE))
                    .padding([8, 20])
                    .style(style::secondary)
                    .on_press(Message::OpenLibrary),
            ]
            .spacing(16)
            .into(),
        }
    }

    fn start_view(&self) -> Element<'_, Message> {
        let error = self
            .download
            .error
            .as_ref()
            .map(|error| text(error.clone()).size(font::BAR_SIZE).style(text::danger));

        column![]
            .push(error)
            .push((self.import_place() == Some(ImportPlace::Catalog)).then(|| self.import_view()))
            .spacing(16)
            .into()
    }

    fn import_view(&self) -> Element<'_, Message> {
        let line = if self.import.running {
            note(IMPORTING)
        } else if self.import.hovering {
            text(DROP_TO_IMPORT)
                .size(font::NOTE_SIZE)
                .style(style::accent)
        } else {
            note(DROP_CATALOG)
        };
        let result: Option<Element<'_, Message>> = match &self.import.result {
            Some(Ok(imported)) => {
                let now = Timestamp(jiff::Timestamp::now().as_millisecond());
                let (before, after) =
                    imported_note(&format::day(imported.prices_at, now), imported.region);
                Some(
                    row![
                        note(before),
                        text(REFRESH)
                            .font(font::ICONS)
                            .size(font::icon(font::NOTE_SIZE))
                            .style(style::muted),
                        note(after),
                    ]
                    .spacing(4)
                    .align_y(Center)
                    .into(),
                )
            }
            Some(Err(error)) => Some(
                text(error.clone())
                    .size(font::NOTE_SIZE)
                    .style(text::danger)
                    .into(),
            ),
            None => None,
        };

        column![line].push(result).spacing(8).into()
    }

    fn progress_view(&self) -> Element<'_, Message> {
        let tracker = &self.download.tracker;

        column![
            progress_bar(0.0..=1.0, tracker.fraction())
                .girth(10)
                .length(Fill)
                .style(style::meter),
            note(download_status(
                tracker.stage(),
                tracker.time_left(),
                tracker.resuming(),
                self.first_run.resume_within_hours
            )),
        ]
        .spacing(8)
        .into()
    }
}

fn preview<'a>(choice: ThemeChoice) -> Element<'a, Message> {
    let sample = theme(choice);
    let block = |width: iced::Length, height: f32| {
        let sample = sample.clone();
        container(column![])
            .width(width)
            .height(height)
            .style(move |_: &Theme| style::skeleton(&sample))
    };
    let bar = {
        let sample = sample.clone();
        container(column![])
            .width(Fill)
            .height(PREVIEW_BAR)
            .style(move |_: &Theme| container::Style {
                border: iced::Border {
                    radius: border::top(PREVIEW_RADIUS),
                    ..iced::Border::default()
                },
                ..style::top_bar(&sample)
            })
    };
    let page = row![
        block(80.into(), 124.0),
        column![
            block(120.into(), 16.0),
            block(Fill, 8.0),
            block(160.into(), 8.0),
        ]
        .spacing(8),
    ]
    .spacing(16);

    container(column![bar, container(page).padding(16)])
        .width(PREVIEW_WIDTH)
        .height(PREVIEW_HEIGHT)
        .style(move |_: &Theme| container::Style {
            border: iced::Border {
                radius: PREVIEW_RADIUS.into(),
                ..iced::Border::default()
            },
            ..style::card(&sample)
        })
        .into()
}

fn preview_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, _| {
        let palette = theme.extended_palette();
        let (color, width) = if selected {
            (palette.primary.base.color, 2.0)
        } else {
            (palette.background.strong.color, 1.0)
        };

        button::Style {
            border: iced::Border {
                color,
                width,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        }
    }
}

fn page_title<'a>(line: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(line).size(font::PRICE_SIZE).font(font::SEMIBOLD)
}

fn dot<'a>(state: StepState) -> Element<'a, Message> {
    let filled = || container(column![]).style(style::step(true, false));
    match state {
        StepState::Done => filled().width(DOT).height(DOT).into(),
        StepState::Current => container(filled().width(DOT_CENTER).height(DOT_CENTER))
            .center(DOT)
            .style(style::step(false, true))
            .into(),
        StepState::Next => container(column![])
            .width(DOT)
            .height(DOT)
            .style(style::step(false, false))
            .into(),
    }
}

fn empty_profile_text(min_hours: f64) -> String {
    let hours = if min_hours == 1.0 {
        "1 hour".to_string()
    } else {
        format!("{min_hours} hours")
    };
    format!(
        "No game in your library has {hours} of play. Play something for a while or add games to your wishlist, then check again."
    )
}

fn ready_title(ready: Option<&Ready>) -> &'static str {
    match ready {
        Some(Ready::Empty { .. }) => "Not enough play time yet",
        Some(Ready::Preparing | Ready::Failed(_)) | None => "Preparing your profile",
    }
}

fn open_link(url: &str) -> Action {
    super::open(url);
    Action::None
}

fn note<'a>(line: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(line).size(font::NOTE_SIZE).style(style::muted)
}

fn permission<'a>(
    title: &'a str,
    about: &'a str,
    file: Option<&std::path::Path>,
    on: bool,
    message: fn(bool) -> Message,
) -> Element<'a, Message> {
    let path = file.map(|file| note(file.display().to_string()));

    row![
        column![
            text(title).size(font::BAR_SIZE).font(font::SEMIBOLD),
            text(about).size(font::BAR_SIZE),
        ]
        .push(path)
        .spacing(8)
        .width(Fill),
        toggler(on)
            .on_toggle(message)
            .size(TOGGLER_SIZE)
            .style(style::switch),
    ]
    .spacing(16)
    .align_y(Center)
    .into()
}

fn this_pc(first_run: &FirstRun) -> ThisPc {
    let dir = first_run.data_dir.as_deref();
    ThisPc {
        accounts: dir.is_some_and(|dir| steam_accounts_answer(dir) == Some(true)),
        hours: dir.is_some_and(|dir| steam_client_hours_answer(dir) == Some(true)),
        accounts_file: first_run.client.then(login_users_file).flatten(),
        hours_file: first_run.client.then(local_config_shown).flatten(),
        ..ThisPc::default()
    }
}

fn answered(dir: &std::path::Path, accounts: bool, hours: bool) -> Result<(), String> {
    answer_steam_accounts(dir, accounts).map_err(|error| error_chain(&error))?;
    answer_steam_client(dir, hours).map_err(|error| error_chain(&error))
}

fn save_this_pc(
    dir: &std::path::Path,
    accounts: bool,
    hours: bool,
) -> Result<Vec<ClientAccount>, String> {
    answered(dir, accounts, hours)?;
    Ok(steam_accounts(dir))
}

fn is_vanity(text: &str) -> bool {
    matches!(
        parse_profile(text),
        Ok(gamepalate_app::account::Profile::CustomName(_))
    )
}

fn account_done(first_run: &FirstRun) -> bool {
    first_run.steam_id == Part::Present && first_run.api_key
}

fn changes_account(account: &Account, first_run: &FirstRun) -> bool {
    first_run
        .saved_steam_id
        .is_some_and(|id| id.0.to_string() != account.steam_id.trim())
}

fn can_connect(account: &Account, first_run: &FirstRun) -> bool {
    first_run.data_dir.is_some()
        && !account.connecting
        && parse_profile(&account.steam_id).is_ok()
        && (!account.key.trim().is_empty() || first_run.api_key)
}

pub(crate) fn steam_id_note(text: &str) -> Option<&'static str> {
    if text.trim().is_empty() {
        return None;
    }
    parse_profile(text)
        .is_err()
        .then_some("Paste the address of your profile, or the 17-digit Steam ID")
}

pub(crate) fn needs_privacy(error: &str) -> bool {
    error == PRIVATE_LIBRARY
}

fn is_key_rejected(error: &str) -> bool {
    error == KEY_REJECTED
}

fn is_unknown_profile(error: &str) -> bool {
    error == NO_PROFILE
}

pub(crate) fn note_link<'a, M: Clone + 'a>(label: &'static str, message: M) -> Element<'a, M> {
    button(text(label).size(font::NOTE_SIZE))
        .padding(0)
        .style(style::link)
        .on_press(message)
        .into()
}

pub(crate) fn connect_error_text(error: &ConnectError) -> String {
    let down = "Steam is not responding. Try again in a minute.";
    match error.steam_error() {
        Some(SteamError::KeyRejected { .. }) => KEY_REJECTED.to_string(),
        Some(SteamError::LibraryHidden { .. }) => PRIVATE_LIBRARY.to_string(),
        Some(SteamError::MissingKey { .. }) => "Paste the Steam Web API key".to_string(),
        Some(SteamError::SteamDown { .. }) => down.to_string(),
        _ if matches!(error, ConnectError::SteamDown) => down.to_string(),
        _ if matches!(error, ConnectError::NoProfile(_)) => NO_PROFILE.to_string(),
        _ => error_chain(error),
    }
}

fn shown(page: Page, first_run: &FirstRun) -> bool {
    match page {
        Page::Welcome | Page::Theme => first_run.saved_steam_id.is_none(),
        Page::ThisPc => first_run.client,
        Page::Account | Page::Catalog | Page::Profile => true,
        Page::Key => !account_done(first_run),
        Page::Region => false,
    }
}

fn page_done(page: Page, first_run: &FirstRun) -> bool {
    match page {
        Page::Welcome | Page::Theme => first_run.saved_steam_id.is_some(),
        Page::ThisPc => first_run.saved_steam_id.is_some(),
        Page::Account => first_run.steam_id == Part::Present,
        Page::Key => account_done(first_run),
        Page::Region => first_run.catalog == Part::Present || first_run.pending_update.is_some(),
        Page::Catalog => first_run.catalog == Part::Present,
        Page::Profile => false,
    }
}

fn first_page(first_run: &FirstRun) -> Page {
    page_from(Page::Welcome, first_run)
}

fn page_from(page: Page, first_run: &FirstRun) -> Page {
    Page::ALL
        .into_iter()
        .filter(|&next| next >= page && shown(next, first_run))
        .find(|&next| !page_done(next, first_run))
        .unwrap_or(Page::Profile)
}

fn next_page(page: Page, first_run: &FirstRun) -> Option<Page> {
    Page::ALL
        .into_iter()
        .find(|&next| next > page && shown(next, first_run))
}

fn previous_page(page: Page, first_run: &FirstRun) -> Option<Page> {
    Page::ALL
        .into_iter()
        .rev()
        .find(|&previous| previous < page && shown(previous, first_run))
}

fn counted_pages(first_run: &FirstRun) -> impl Iterator<Item = Page> {
    Page::ALL
        .into_iter()
        .filter(|&page| page >= Page::ThisPc && shown(page, first_run))
}

fn step_of(page: Page, first_run: &FirstRun) -> Option<(usize, usize)> {
    let number = counted_pages(first_run).position(|counted| counted == page)?;
    Some((number + 1, counted_pages(first_run).count()))
}

fn step_state(page: Page, current: Page, done: bool) -> StepState {
    match page.cmp(&current) {
        Ordering::Less => StepState::Done,
        Ordering::Equal => StepState::Current,
        Ordering::Greater if done => StepState::Done,
        Ordering::Greater => StepState::Next,
    }
}

fn downloads_at_start(first_run: &FirstRun) -> bool {
    first_page(first_run) == Page::Catalog && first_run.catalog == Part::Missing
}

fn prepares_after_connect(first_run: &FirstRun) -> bool {
    first_run.catalog == Part::Present && first_run.steam_id == Part::Present && first_run.api_key
}

fn import_place(page: Page, first_run: &FirstRun, download: &Download) -> Option<ImportPlace> {
    if download.running || first_run.data_dir.is_none() {
        return None;
    }
    if matches!(first_run.catalog, Part::Invalid(_)) {
        return Some(ImportPlace::Problems);
    }
    match page {
        Page::Account => Some(ImportPlace::Account),
        Page::Catalog if download.error.is_some() => Some(ImportPlace::Catalog),
        Page::Welcome
        | Page::Theme
        | Page::ThisPc
        | Page::Key
        | Page::Region
        | Page::Catalog
        | Page::Profile => None,
    }
}

fn window_message(
    event: Event,
    status: event::Status,
    window: iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Keyboard(event) => key_message(event, status),
        event => drop_message(event, status, window),
    }
}

fn key_message(event: keyboard::Event, status: event::Status) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    match key {
        Key::Named(Named::Enter) if status == event::Status::Ignored => Some(Message::Enter),
        Key::Named(Named::Escape) => Some(Message::Back),
        Key::Named(Named::ArrowLeft) if modifiers.alt() => Some(Message::Back),
        _ => None,
    }
}

fn drop_message(event: Event, _: event::Status, _: iced::window::Id) -> Option<Message> {
    match event {
        Event::Window(iced::window::Event::FileHovered(_)) => Some(Message::Hovering(true)),
        Event::Window(iced::window::Event::FilesHoveredLeft) => Some(Message::Hovering(false)),
        Event::Window(iced::window::Event::FileDropped(file)) => Some(Message::Import(file)),
        _ => None,
    }
}

fn can_import(place: Option<ImportPlace>, import: &Import, account: &Account) -> bool {
    place.is_some() && !import.running && !account.connecting
}

fn import_error_text(error: &CatalogFileError) -> String {
    match error {
        CatalogFileError::NotACatalog(_) => error.to_string(),
        _ => error_chain(error),
    }
}

fn imported_note(day: &str, region: Region) -> (String, &'static str) {
    (
        format!("Catalog from {day}, prices for {region}. Press"),
        "later to download your own.",
    )
}

fn download_status(
    stage: Stage,
    left: Option<Duration>,
    resuming: bool,
    resume_within_hours: u32,
) -> String {
    let status = match (stage, left) {
        _ if resuming => "Picking up where it left off".to_string(),
        (Stage::Starting | Stage::Reading, _) => "Reading the list of Steam games".to_string(),
        (Stage::StoreData { .. }, Some(left)) => format::time_left(left),
        (Stage::StoreData { .. }, None) => "Working out the time left".to_string(),
        (Stage::Writing, _) => "Saving the catalog".to_string(),
    };
    format!(
        "{status}. If you close gamepalate now, the download picks up from here when you open it again within {resume_within_hours} hours."
    )
}

fn covered(played: &[Played]) -> impl Iterator<Item = &Played> {
    played
        .iter()
        .filter(|game| !matches!(game.cover, Cover::Missing))
        .take(PLAYED_SHOWN)
}

fn ready_covers(played: &[Played]) -> Option<Vec<(&image::Handle, f64)>> {
    covered(played)
        .map(|game| match &game.cover {
            Cover::Ready(handle) => Some((handle, game.hours)),
            Cover::Loading | Cover::Missing => None,
        })
        .collect::<Option<Vec<_>>>()
        .filter(|covers| !covers.is_empty())
}

fn cover_tile<'a>(handle: &image::Handle, played: f64) -> Element<'a, Message> {
    let picture = image(handle.clone())
        .width(TILE_WIDTH)
        .height(TILE_HEIGHT)
        .content_fit(ContentFit::Cover)
        .border_radius(IMAGE_RADIUS);
    let strip = container(text(hours(played)).size(font::NOTE_SIZE))
        .padding([3, 8])
        .width(Fill)
        .style(style::capsule_strip);

    stack![picture, column![space().height(Fill), strip]]
        .width(TILE_WIDTH)
        .height(TILE_HEIGHT)
        .into()
}

fn file_problems(first_run: &FirstRun, error: Option<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    if first_run.data_dir.is_none() {
        problems.push("Could not find the user data folder. Set GAMEPALATE_DATA_DIR.".to_string());
    }
    if let Part::Invalid(message) = &first_run.catalog {
        problems.push(format!(
            "gamepalate could not open the data files: {message}"
        ));
    }
    if let Part::Invalid(message) = &first_run.signals {
        problems.push(message.clone());
    }
    if let Some(error) = error {
        problems.push(format!("gamepalate could not open the data files: {error}"));
    }
    problems
}

fn field<'a>(label: &'a str, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        container(text(label).size(font::BAR_SIZE).style(style::muted))
            .width(LABEL_WIDTH)
            .padding(padding::top(8)),
        container(content).width(Fill),
    ]
    .spacing(16)
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
    use std::path::PathBuf;

    use gamepalate_app::{PendingUpdate, SteamId};

    use super::*;

    fn first_run() -> FirstRun {
        FirstRun {
            data_dir: Some(PathBuf::from("data")),
            steam_id: Part::Present,
            saved_steam_id: Some(SteamId(76_561_198_000_000_000)),
            api_key: true,
            catalog: Part::Present,
            pending_update: None,
            resume_within_hours: 24,
            signals: Part::Present,
            client: false,
        }
    }

    fn with_client() -> FirstRun {
        FirstRun {
            client: true,
            ..first_run()
        }
    }

    fn client_account(id: u64, most_recent: bool) -> ClientAccount {
        ClientAccount {
            steam_id: SteamId(id),
            persona_name: None,
            most_recent,
            avatar: None,
        }
    }

    fn without_catalog() -> FirstRun {
        FirstRun {
            catalog: Part::Missing,
            ..first_run()
        }
    }

    fn account(steam_id: &str, key: &str) -> Account {
        Account {
            steam_id: steam_id.to_string(),
            key: key.to_string(),
            ..Account::default()
        }
    }

    fn primary_of(setup: &Setup) -> Option<(&'static str, bool)> {
        setup
            .primary()
            .map(|primary| (primary.label, primary.message.is_some()))
    }

    fn pressed(key: Key, modifiers: keyboard::Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    #[test]
    fn a_missing_part_opens_the_setup_without_trying_the_files() {
        let first_run = without_catalog();

        let Err(setup) = start(first_run.clone()) else {
            panic!("expected the setup");
        };

        assert_eq!(setup.first_run, first_run);
        assert_eq!(setup.error, None);
        assert_eq!(setup.account.steam_id, "76561198000000000");
        assert_eq!(setup.page, Page::Catalog);
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
    fn the_first_page_is_the_first_missing_one() {
        let without_key = FirstRun {
            api_key: false,
            catalog: Part::Missing,
            ..first_run()
        };
        let without_steam_id = FirstRun {
            steam_id: Part::Missing,
            saved_steam_id: None,
            api_key: false,
            catalog: Part::Missing,
            ..first_run()
        };
        let invalid = FirstRun {
            catalog: Part::Invalid("no such table: games".to_string()),
            ..first_run()
        };
        let resuming = FirstRun {
            pending_update: Some(PendingUpdate {
                started_at: Timestamp(0),
                responses: 120,
            }),
            ..without_catalog()
        };

        assert_eq!(first_page(&without_key), Page::Key);
        assert_eq!(first_page(&without_steam_id), Page::Welcome);
        assert_eq!(page_from(Page::Account, &without_steam_id), Page::Account);
        assert_eq!(first_page(&without_catalog()), Page::Catalog);
        assert_eq!(first_page(&invalid), Page::Catalog);
        assert_eq!(first_page(&resuming), Page::Catalog);
        assert_eq!(first_page(&first_run()), Page::Profile);
    }

    #[test]
    fn welcome_and_theme_come_only_before_a_steam_id() {
        let new = FirstRun {
            steam_id: Part::Missing,
            saved_steam_id: None,
            api_key: false,
            catalog: Part::Missing,
            ..first_run()
        };

        for page in [Page::Welcome, Page::Theme] {
            assert!(!page_done(page, &new));
            assert!(page_done(page, &without_catalog()));
            assert!(shown(page, &new));
            assert!(!shown(page, &without_catalog()));
        }
    }

    #[test]
    fn the_region_is_done_with_a_catalog_or_a_download_to_resume() {
        let resuming = FirstRun {
            pending_update: Some(PendingUpdate {
                started_at: Timestamp(0),
                responses: 120,
            }),
            ..without_catalog()
        };

        assert!(page_done(Page::Region, &first_run()));
        assert!(page_done(Page::Region, &resuming));
        assert!(!page_done(Page::Region, &without_catalog()));
    }

    #[test]
    fn the_key_page_shows_until_the_account_is_connected() {
        let without_key = FirstRun {
            api_key: false,
            ..first_run()
        };
        let without_steam_id = FirstRun {
            steam_id: Part::Missing,
            saved_steam_id: None,
            api_key: false,
            ..first_run()
        };

        assert!(!shown(Page::Key, &first_run()));
        assert!(shown(Page::Key, &without_key));
        assert!(shown(Page::Key, &without_steam_id));
        assert!(page_done(Page::Key, &first_run()));
        assert!(!page_done(Page::Key, &without_key));
        assert!(page_done(Page::Account, &without_key));
        assert!(!page_done(Page::Account, &without_steam_id));
        assert_eq!(page_from(Page::Account, &without_key), Page::Key);
        assert_eq!(next_page(Page::Account, &without_key), Some(Page::Key));
        assert_eq!(next_page(Page::Account, &first_run()), Some(Page::Catalog));
        assert_eq!(previous_page(Page::Key, &without_key), Some(Page::Account));
        assert_eq!(previous_page(Page::Catalog, &without_key), Some(Page::Key));
        assert_eq!(step_of(Page::Key, &without_key), Some((2, 4)));
    }

    #[test]
    fn back_and_continue_skip_the_pages_not_shown() {
        let known = first_run();
        let new = FirstRun {
            saved_steam_id: None,
            ..first_run()
        };

        assert_eq!(previous_page(Page::Account, &known), None);
        assert_eq!(previous_page(Page::Catalog, &known), Some(Page::Account));
        assert_eq!(previous_page(Page::Profile, &known), Some(Page::Catalog));
        assert_eq!(next_page(Page::Welcome, &known), Some(Page::Account));
        assert_eq!(next_page(Page::Account, &known), Some(Page::Catalog));
        assert_eq!(next_page(Page::Catalog, &known), Some(Page::Profile));
        assert_eq!(next_page(Page::Profile, &known), None);

        assert_eq!(previous_page(Page::Account, &new), Some(Page::Theme));
        assert_eq!(previous_page(Page::Theme, &new), Some(Page::Welcome));
        assert_eq!(previous_page(Page::Welcome, &new), None);
        assert_eq!(next_page(Page::Welcome, &new), Some(Page::Theme));
        assert_eq!(next_page(Page::Theme, &new), Some(Page::Account));
    }

    #[test]
    fn a_finished_page_moves_to_the_next_missing_one() {
        let without_account = FirstRun {
            steam_id: Part::Missing,
            api_key: false,
            ..first_run()
        };

        assert_eq!(page_from(Page::Catalog, &first_run()), Page::Profile);
        assert_eq!(page_from(Page::Account, &without_account), Page::Account);
        assert_eq!(page_from(Page::Account, &first_run()), Page::Profile);
    }

    #[test]
    fn back_and_continue_move_between_the_pages() {
        let mut setup = Setup::new(without_catalog(), None);
        assert_eq!(setup.page, Page::Catalog);

        setup.update(Message::Back);
        assert_eq!(setup.page, Page::Account);
        setup.update(Message::Back);
        assert_eq!(setup.page, Page::Account);
        setup.update(Message::Continue);
        assert_eq!(setup.page, Page::Catalog);

        setup.update(Message::Back);
        setup.update(Message::Enter);
        assert_eq!(setup.page, Page::Catalog);
    }

    #[test]
    fn there_is_no_back_during_the_download_or_connect() {
        let mut setup = Setup::new(without_catalog(), None);
        assert_eq!(setup.back(), Some(Page::Account));

        setup.download.running = true;
        assert_eq!(setup.back(), None);
        setup.update(Message::Back);
        setup.update(Message::Change);
        assert_eq!(setup.page, Page::Catalog);

        setup.download.running = false;
        setup.account.connecting = true;
        assert_eq!(setup.back(), None);
    }

    #[test]
    fn change_goes_back_to_the_account() {
        let mut setup = Setup::new(first_run(), None);
        setup.ready = Some(Ready::Empty { min_hours: 2.0 });

        setup.update(Message::Change);

        assert_eq!(setup.page, Page::Account);
    }

    #[test]
    fn the_steps_count_the_pages_shown() {
        let known = first_run();
        let new = FirstRun {
            saved_steam_id: None,
            ..first_run()
        };

        for first_run in [&known, &new] {
            assert_eq!(step_of(Page::Account, first_run), Some((1, 3)));
            assert_eq!(step_of(Page::Catalog, first_run), Some((2, 3)));
            assert_eq!(step_of(Page::Profile, first_run), Some((3, 3)));
            assert_eq!(step_of(Page::Welcome, first_run), None);
            assert_eq!(step_of(Page::Theme, first_run), None);
        }
    }

    fn setup_in(dir: &std::path::Path, steam_id: bool) -> Setup {
        Setup::new(
            FirstRun {
                data_dir: Some(dir.to_path_buf()),
                saved_steam_id: steam_id.then_some(SteamId(76_561_198_000_000_000)),
                ..first_run()
            },
            None,
        )
    }

    #[test]
    fn the_welcome_and_theme_pages_lead_to_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let mut setup = setup_in(dir.path(), false);

        assert_eq!(setup.page, Page::Welcome);
        assert_eq!(primary_of(&setup), Some(("Get started", true)));
        assert_eq!(setup.back(), None);

        setup.update(Message::Enter);
        assert_eq!(setup.page, Page::Theme);
        assert_eq!(primary_of(&setup), Some(("Continue", true)));
        assert_eq!(setup.back(), Some(Page::Welcome));

        setup.update(Message::Continue);
        assert_eq!(setup.page, Page::Account);
        setup.update(Message::Back);
        assert_eq!(setup.page, Page::Theme);
    }

    #[test]
    fn a_click_on_dark_applies_and_saves_the_theme() {
        let dir = tempfile::tempdir().unwrap();
        let mut setup = setup_in(dir.path(), false);
        assert_eq!(setup.theme, ThemeChoice::Light);

        assert!(matches!(
            setup.update(Message::Theme(ThemeChoice::Dark)),
            Action::Theme(ThemeChoice::Dark)
        ));
        assert_eq!(setup.theme, ThemeChoice::Dark);
        assert_eq!(setup.first_run().gui_settings().theme, ThemeChoice::Dark);
    }

    #[test]
    fn a_click_on_the_chosen_theme_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut setup = setup_in(dir.path(), false);

        assert!(matches!(
            setup.update(Message::Theme(ThemeChoice::Light)),
            Action::None
        ));
        assert!(!dir.path().join("gui.toml").exists());
    }

    #[test]
    fn the_setup_opens_in_the_saved_theme() {
        let dir = tempfile::tempdir().unwrap();
        setup_in(dir.path(), false)
            .first_run()
            .change_gui_settings(|settings| settings.theme = ThemeChoice::Dark)
            .unwrap();

        let setup = setup_in(dir.path(), false);

        assert_eq!(setup.theme, ThemeChoice::Dark);
    }

    fn new_pc(dir: &std::path::Path) -> FirstRun {
        FirstRun {
            data_dir: Some(dir.to_path_buf()),
            saved_steam_id: None,
            steam_id: Part::Missing,
            api_key: false,
            catalog: Part::Missing,
            ..with_client()
        }
    }

    #[test]
    fn this_pc_shows_only_with_the_steam_client() {
        let without = FirstRun {
            saved_steam_id: None,
            ..first_run()
        };
        let with = FirstRun {
            saved_steam_id: None,
            ..with_client()
        };

        assert!(!shown(Page::ThisPc, &without));
        assert!(shown(Page::ThisPc, &with));
        assert_eq!(next_page(Page::Theme, &without), Some(Page::Account));
        assert_eq!(next_page(Page::Theme, &with), Some(Page::ThisPc));
        assert_eq!(previous_page(Page::Account, &with), Some(Page::ThisPc));
        assert_eq!(step_of(Page::ThisPc, &with), Some((1, 4)));
        assert_eq!(step_of(Page::Account, &with), Some((2, 4)));
        assert_eq!(step_of(Page::Account, &without), Some((1, 3)));
    }

    #[test]
    fn the_togglers_start_off_or_from_the_saved_answers() {
        let dir = tempfile::tempdir().unwrap();
        let setup = Setup::new(new_pc(dir.path()), None);
        assert!(!setup.this_pc.accounts);
        assert!(!setup.this_pc.hours);

        answer_steam_accounts(dir.path(), true).unwrap();
        answer_steam_client(dir.path(), false).unwrap();
        let setup = Setup::new(new_pc(dir.path()), None);
        assert!(setup.this_pc.accounts);
        assert!(!setup.this_pc.hours);
    }

    #[test]
    fn continue_on_this_pc_saves_both_answers() {
        let dir = tempfile::tempdir().unwrap();
        let mut setup = Setup::new(new_pc(dir.path()), None);
        setup.page = Page::ThisPc;
        assert_eq!(primary_of(&setup), Some(("Continue", true)));

        setup.update(Message::ThisPcAccounts(true));
        setup.update(Message::ThisPcHours(true));
        assert!(matches!(setup.update(Message::Continue), Action::Run(_)));
        assert!(setup.this_pc.saving);
        assert_eq!(primary_of(&setup), Some(("Saving…", false)));
        assert_eq!(setup.back(), None);
        assert_eq!(setup.page, Page::ThisPc);

        answered(dir.path(), true, true).unwrap();
        assert_eq!(steam_accounts_answer(dir.path()), Some(true));
        assert_eq!(steam_client_hours_answer(dir.path()), Some(true));
        assert!(matches!(
            setup.update(Message::ThisPcSaved(Ok(vec![
                client_account(76_561_198_000_000_000, true),
                client_account(76_561_198_000_000_001, false),
            ]))),
            Action::None
        ));
        assert_eq!(setup.page, Page::Account);
        assert!(!setup.this_pc.saving);
        assert!(setup.accounts.listed());
        assert_eq!(setup.account.steam_id, "76561198000000000");
    }

    #[test]
    fn a_failed_save_stays_on_this_pc() {
        let dir = tempfile::tempdir().unwrap();
        let mut setup = Setup::new(new_pc(dir.path()), None);
        setup.page = Page::ThisPc;
        setup.this_pc.saving = true;

        setup.update(Message::ThisPcSaved(Err("Access denied".to_string())));

        assert_eq!(setup.page, Page::ThisPc);
        assert!(!setup.this_pc.saving);
        assert_eq!(setup.this_pc.error.as_deref(), Some("Access denied"));
        assert_eq!(primary_of(&setup), Some(("Continue", true)));
    }

    #[test]
    fn without_a_list_the_account_page_keeps_the_field() {
        let dir = tempfile::tempdir().unwrap();
        let mut setup = Setup::new(new_pc(dir.path()), None);
        setup.page = Page::ThisPc;

        setup.update(Message::ThisPcSaved(Ok(Vec::new())));

        assert_eq!(setup.page, Page::Account);
        assert!(!setup.accounts.listed());
        assert_eq!(setup.account.steam_id, "");
        assert_eq!(primary_of(&setup), Some(("Continue", false)));
    }

    #[test]
    fn the_account_list_chooses_the_profile_to_connect() {
        let dir = tempfile::tempdir().unwrap();
        let mut setup = Setup::new(new_pc(dir.path()), None);
        setup.page = Page::ThisPc;
        setup.update(Message::ThisPcSaved(Ok(vec![
            client_account(76_561_198_000_000_000, true),
            client_account(76_561_198_000_000_001, false),
        ])));
        assert_eq!(primary_of(&setup), Some(("Continue", true)));

        setup.update(Message::Accounts(accounts::Message::Select(SteamId(
            76_561_198_000_000_001,
        ))));
        assert_eq!(setup.account.steam_id, "76561198000000001");

        setup.update(Message::Accounts(accounts::Message::UseAnother));
        assert!(!setup.accounts.listed());
        assert_eq!(setup.account.steam_id, "");
        assert_eq!(primary_of(&setup), Some(("Continue", false)));

        setup.update(Message::SteamId("123".to_string()));
        assert_eq!(primary_of(&setup), Some(("Continue", false)));
        setup.update(Message::SteamId("76561198000000007".to_string()));
        assert_eq!(primary_of(&setup), Some(("Continue", true)));

        setup.update(Message::Accounts(accounts::Message::BackToList));
        assert!(setup.accounts.listed());
        assert_eq!(setup.account.steam_id, "76561198000000001");
    }

    #[test]
    fn a_vanity_address_is_checked_after_the_key() {
        assert!(is_vanity("https://steamcommunity.com/id/someone"));
        assert!(!is_vanity("76561198000000000"));
        assert!(!is_vanity(""));
    }

    #[test]
    fn the_catalog_page_has_no_question_about_the_hours() {
        let dir = tempfile::tempdir().unwrap();
        answer_steam_client(dir.path(), true).unwrap();
        let first_run = FirstRun {
            data_dir: Some(dir.path().to_path_buf()),
            catalog: Part::Missing,
            ..first_run()
        };
        let setup = Setup::new(first_run, None);

        assert_eq!(setup.page, Page::Catalog);
        assert!(setup.this_pc.hours);
    }

    fn new_account() -> FirstRun {
        FirstRun {
            steam_id: Part::Missing,
            saved_steam_id: None,
            api_key: false,
            catalog: Part::Missing,
            ..first_run()
        }
    }

    #[test]
    fn the_account_page_only_continues() {
        let mut setup = Setup::new(new_account(), None);
        setup.page = Page::Account;
        assert_eq!(primary_of(&setup), Some(("Continue", false)));

        setup.account = account("76561198000000000", "");
        assert_eq!(primary_of(&setup), Some(("Continue", true)));

        setup.account = account("https://steamcommunity.com/id/someone", "");
        assert_eq!(primary_of(&setup), Some(("Continue", true)));

        setup.import.running = true;
        assert_eq!(primary_of(&setup), Some(("Continue", false)));
    }

    #[test]
    fn continuing_from_the_account_does_not_connect() {
        let mut setup = Setup::new(new_account(), None);
        setup.page = Page::Account;
        setup.account = account("76561198000000000", "");

        let action = setup.update(Message::Continue);

        assert!(matches!(action, Action::None));
        assert_eq!(setup.page, Page::Key);
        assert!(!setup.account.connecting);
    }

    #[test]
    fn a_changed_account_goes_through_the_key_page_again() {
        let mut setup = Setup::new(first_run(), None);
        setup.page = Page::Account;
        setup.update(Message::Continue);
        assert_eq!(setup.page, Page::Catalog);

        setup.page = Page::Account;
        setup.account.steam_id = "76561198000000001".to_string();
        setup.update(Message::Continue);
        assert_eq!(setup.page, Page::Key);
        assert_eq!(primary_of(&setup), Some(("Connect", true)));
        assert_eq!(setup.back(), Some(Page::Account));
    }

    #[test]
    fn connect_is_on_with_a_typed_or_a_saved_key() {
        let mut setup = Setup::new(new_account(), None);
        setup.page = Page::Key;
        setup.account = account("76561198000000000", "");
        assert_eq!(primary_of(&setup), Some(("Connect", false)));

        setup.update(Message::Key("KEY".to_string()));
        assert_eq!(primary_of(&setup), Some(("Connect", true)));

        setup.account.connecting = true;
        assert_eq!(primary_of(&setup), Some(("Connecting…", false)));

        let mut setup = Setup::new(
            FirstRun {
                steam_id: Part::Missing,
                ..first_run()
            },
            None,
        );
        setup.page = Page::Key;
        assert_eq!(primary_of(&setup), Some(("Connect", true)));
    }

    #[test]
    fn paste_trims_the_clipboard() {
        let mut setup = Setup::new(new_account(), None);
        setup.page = Page::Key;

        setup.update(Message::Pasted(Some("  ABC123 \r\n".to_string())));
        assert_eq!(setup.account.key, "ABC123");

        setup.update(Message::Pasted(None));
        assert_eq!(setup.account.key, "ABC123");

        setup.account.connecting = true;
        setup.update(Message::Pasted(Some("OTHER".to_string())));
        assert_eq!(setup.account.key, "ABC123");
    }

    #[test]
    fn an_error_goes_away_when_the_key_changes() {
        for change in [
            Message::Key("NEW".to_string()),
            Message::Pasted(Some(" NEW ".to_string())),
        ] {
            let mut setup = Setup::new(new_account(), None);
            setup.page = Page::Key;
            setup.update(Message::Connected(Err(KEY_REJECTED.to_string())));
            assert!(is_key_rejected(setup.account.error.as_deref().unwrap()));

            setup.update(change);
            assert_eq!(setup.account.error, None);
        }
    }

    #[test]
    fn an_unknown_profile_offers_to_change_the_account() {
        let mut setup = Setup::new(new_account(), None);
        setup.page = Page::Key;
        setup.account = account("https://steamcommunity.com/id/nobody", "KEY");
        let error = connect_error_text(&ConnectError::NoProfile("nobody".to_string()));
        assert!(is_unknown_profile(&error));
        assert!(!is_unknown_profile(KEY_REJECTED));
        assert!(!is_key_rejected(&error));

        setup.update(Message::Connected(Err(error)));
        assert_eq!(setup.page, Page::Key);
        assert!(!setup.account.connecting);

        setup.update(Message::Change);
        assert_eq!(setup.page, Page::Account);
        assert_eq!(
            setup.account.steam_id,
            "https://steamcommunity.com/id/nobody"
        );
    }

    #[test]
    fn the_catalog_page_downloads_or_continues() {
        let mut setup = Setup::new(without_catalog(), None);
        assert_eq!(primary_of(&setup), Some(("Download catalog", true)));

        setup.import.running = true;
        assert_eq!(primary_of(&setup), Some(("Download catalog", false)));

        setup.import.running = false;
        setup.download.error = Some("Steam is not responding".to_string());
        assert_eq!(primary_of(&setup), Some(("Retry", true)));

        setup.download.running = true;
        assert_eq!(primary_of(&setup), None);

        let mut setup = Setup::new(first_run(), None);
        setup.page = Page::Catalog;
        assert_eq!(primary_of(&setup), Some(("Continue", true)));
    }

    #[test]
    fn the_profile_page_retries_or_checks_again() {
        let mut setup = Setup::new(first_run(), None);
        assert_eq!(primary_of(&setup), Some(("Read my library", true)));

        setup.ready = Some(Ready::Preparing);
        assert_eq!(primary_of(&setup), None);
        assert_eq!(setup.back(), None);

        setup.ready = Some(Ready::Failed("Steam is not responding".to_string()));
        assert_eq!(primary_of(&setup), Some(("Retry", true)));

        setup.ready = Some(Ready::Empty { min_hours: 2.0 });
        assert_eq!(primary_of(&setup), Some(("Check again", true)));
    }

    #[test]
    fn enter_presses_the_main_button_and_escape_goes_back() {
        let none = keyboard::Modifiers::empty();

        assert!(matches!(
            key_message(
                pressed(Key::Named(Named::Enter), none),
                event::Status::Ignored
            ),
            Some(Message::Enter)
        ));
        assert!(
            key_message(
                pressed(Key::Named(Named::Enter), none),
                event::Status::Captured
            )
            .is_none()
        );
        assert!(matches!(
            key_message(
                pressed(Key::Named(Named::Escape), none),
                event::Status::Ignored
            ),
            Some(Message::Back)
        ));
        assert!(matches!(
            key_message(
                pressed(Key::Named(Named::ArrowLeft), keyboard::Modifiers::ALT),
                event::Status::Ignored
            ),
            Some(Message::Back)
        ));
        assert!(
            key_message(
                pressed(Key::Named(Named::ArrowLeft), none),
                event::Status::Ignored
            )
            .is_none()
        );
    }

    #[test]
    fn the_pages_before_the_current_one_are_done() {
        assert_eq!(
            step_state(Page::Account, Page::Catalog, true),
            StepState::Done
        );
        assert_eq!(
            step_state(Page::Catalog, Page::Catalog, false),
            StepState::Current
        );
        assert_eq!(
            step_state(Page::Profile, Page::Catalog, false),
            StepState::Next
        );
        assert_eq!(
            step_state(Page::Account, Page::Account, false),
            StepState::Current
        );
    }

    #[test]
    fn an_imported_catalog_is_done_before_the_account() {
        let without_account = FirstRun {
            steam_id: Part::Missing,
            api_key: false,
            ..first_run()
        };
        let current = first_page(&without_account);

        assert_eq!(current, Page::Account);
        assert_eq!(
            step_state(
                Page::Catalog,
                current,
                page_done(Page::Catalog, &without_account)
            ),
            StepState::Done
        );
        assert_eq!(
            step_state(
                Page::Profile,
                current,
                page_done(Page::Profile, &without_account)
            ),
            StepState::Next
        );
        assert!(!downloads_at_start(&without_account));
    }

    #[test]
    fn only_a_missing_catalog_starts_the_download() {
        let invalid = FirstRun {
            catalog: Part::Invalid("no such table: games".to_string()),
            ..first_run()
        };

        assert!(downloads_at_start(&without_catalog()));
        assert!(!downloads_at_start(&invalid));
        assert!(!downloads_at_start(&first_run()));
    }

    #[test]
    fn connect_with_a_catalog_prepares_the_profile() {
        assert!(prepares_after_connect(&first_run()));
        assert!(!prepares_after_connect(&without_catalog()));
        assert!(!prepares_after_connect(&FirstRun {
            api_key: false,
            ..first_run()
        }));
    }

    #[test]
    fn the_import_line_shows_where_a_catalog_can_go() {
        let failed = Download {
            error: Some("Steam is not responding".to_string()),
            ..Download::default()
        };
        let running = Download {
            running: true,
            ..Download::default()
        };
        let invalid = FirstRun {
            catalog: Part::Invalid("no such table: games".to_string()),
            ..first_run()
        };

        assert_eq!(
            import_place(Page::Account, &without_catalog(), &Download::default()),
            Some(ImportPlace::Account)
        );
        assert_eq!(
            import_place(Page::Catalog, &without_catalog(), &failed),
            Some(ImportPlace::Catalog)
        );
        assert_eq!(
            import_place(Page::Catalog, &without_catalog(), &Download::default()),
            None
        );
        assert_eq!(
            import_place(Page::Catalog, &invalid, &Download::default()),
            Some(ImportPlace::Problems)
        );
        assert_eq!(
            import_place(Page::Account, &invalid, &Download::default()),
            Some(ImportPlace::Problems)
        );
        assert_eq!(
            import_place(Page::Profile, &first_run(), &Download::default()),
            None
        );
        assert_eq!(
            import_place(Page::Catalog, &without_catalog(), &running),
            None
        );
        assert_eq!(import_place(Page::Catalog, &invalid, &running), None);
        assert_eq!(
            import_place(
                Page::Account,
                &FirstRun {
                    data_dir: None,
                    ..without_catalog()
                },
                &Download::default()
            ),
            None
        );
    }

    #[test]
    fn an_import_waits_for_the_one_running_and_for_connect() {
        let running = Import {
            running: true,
            ..Import::default()
        };
        let connecting = Account {
            connecting: true,
            ..Account::default()
        };
        let place = Some(ImportPlace::Account);

        assert!(can_import(place, &Import::default(), &Account::default()));
        assert!(!can_import(None, &Import::default(), &Account::default()));
        assert!(!can_import(place, &running, &Account::default()));
        assert!(!can_import(place, &Import::default(), &connecting));
    }

    #[test]
    fn a_file_dropped_during_the_download_or_an_import_does_nothing() {
        let mut setup = Setup::new(without_catalog(), None);
        setup.download.running = true;

        assert!(matches!(
            setup.update(Message::Import(PathBuf::from("catalog.zip"))),
            Action::None
        ));
        assert!(!setup.import.running);

        setup.download.running = false;
        setup.import.running = true;
        assert!(matches!(
            setup.update(Message::Import(PathBuf::from("catalog.zip"))),
            Action::None
        ));
    }

    #[test]
    fn only_window_file_and_key_events_reach_the_setup() {
        let id = iced::window::Id::unique();
        let file = PathBuf::from("catalog.zip");
        let message = |event| window_message(Event::Window(event), event::Status::Ignored, id);

        assert!(matches!(
            message(iced::window::Event::FileHovered(file.clone())),
            Some(Message::Hovering(true))
        ));
        assert!(matches!(
            message(iced::window::Event::FilesHoveredLeft),
            Some(Message::Hovering(false))
        ));
        assert!(matches!(
            message(iced::window::Event::FileDropped(file.clone())),
            Some(Message::Import(dropped)) if dropped == file
        ));
        assert!(message(iced::window::Event::Focused).is_none());
        assert!(matches!(
            window_message(
                Event::Keyboard(pressed(
                    Key::Named(Named::Escape),
                    keyboard::Modifiers::empty()
                )),
                event::Status::Ignored,
                id
            ),
            Some(Message::Back)
        ));
    }

    #[test]
    fn a_dropped_file_imports_only_where_the_line_shows() {
        let mut setup = Setup::new(
            FirstRun {
                steam_id: Part::Missing,
                catalog: Part::Missing,
                ..first_run()
            },
            None,
        );
        setup.update(Message::Hovering(true));
        assert!(setup.import.hovering);

        assert!(matches!(
            setup.update(Message::Import(PathBuf::from("catalog.zip"))),
            Action::Run(_)
        ));
        assert!(setup.import.running);
        assert!(!setup.import.hovering);

        let mut setup = Setup::new(first_run(), None);
        assert!(setup.import_place().is_none());
        assert!(matches!(
            setup.update(Message::Import(PathBuf::from("catalog.zip"))),
            Action::None
        ));
        assert!(!setup.import.running);
    }

    #[test]
    fn a_failed_import_keeps_the_setup_where_it_was() {
        let mut setup = Setup::new(without_catalog(), None);
        setup.import.running = true;

        let action = setup.update(Message::Imported(Err(
            "This file has no catalog in it".to_string()
        )));

        assert!(matches!(action, Action::None));
        assert!(!setup.import.running);
        assert_eq!(
            setup.import.result,
            Some(Err("This file has no catalog in it".to_string()))
        );
        assert_eq!(setup.first_run.catalog, Part::Missing);
        assert_eq!(setup.page, Page::Catalog);
    }

    #[test]
    fn the_import_note_says_where_the_prices_come_from() {
        let (before, after) = imported_note("13 Nov", Region::ITALY);

        assert_eq!(
            format!("{before} ⟳ {after}"),
            "Catalog from 13 Nov, prices for IT. Press ⟳ later to download your own."
        );
    }

    #[test]
    fn an_unreadable_catalog_is_a_problem() {
        let invalid = FirstRun {
            catalog: Part::Invalid("no such table: games".to_string()),
            ..first_run()
        };

        assert_eq!(
            file_problems(&invalid, None),
            ["gamepalate could not open the data files: no such table: games"]
        );
    }

    #[test]
    fn games_without_a_cover_leave_room_to_the_next() {
        let played: Vec<Played> = (1..=8)
            .map(|id| Played {
                id: AppId(id),
                hours: 10.0,
                cover: if id == 2 || id == 5 {
                    Cover::Missing
                } else {
                    Cover::Loading
                },
            })
            .collect();

        let ids: Vec<u32> = covered(&played).map(|game| game.id.0).collect();

        assert_eq!(ids, [1, 3, 4, 6, 7]);
    }

    #[test]
    fn the_covers_show_up_together_once_all_are_ready() {
        let handle = image::Handle::from_rgba(1, 1, vec![0; 4]);
        let mut played: Vec<Played> = (1..=8)
            .map(|id| Played {
                id: AppId(id),
                hours: 10.0,
                cover: match id {
                    2 => Cover::Missing,
                    4 => Cover::Loading,
                    _ => Cover::Ready(handle.clone()),
                },
            })
            .collect();

        assert!(ready_covers(&played).is_none());

        played[3].cover = Cover::Ready(handle.clone());
        assert_eq!(ready_covers(&played).map(|covers| covers.len()), Some(5));

        for game in &mut played {
            game.cover = Cover::Missing;
        }
        assert!(ready_covers(&played).is_none());
    }

    #[test]
    fn the_download_says_what_it_is_doing() {
        let store_data = Stage::StoreData {
            done: 312,
            total: 757,
        };

        assert_eq!(
            download_status(store_data, Some(Duration::from_secs(720)), false, 24),
            "About 12 minutes left. If you close gamepalate now, the download picks up from here when you open it again within 24 hours."
        );
        assert!(
            download_status(store_data, None, false, 24).starts_with("Working out the time left")
        );
        assert!(download_status(Stage::Reading, None, false, 24).starts_with("Reading the list"));
        assert!(download_status(Stage::Writing, None, false, 24).starts_with("Saving the catalog"));
        assert!(
            download_status(store_data, None, true, 24).starts_with("Picking up where it left off")
        );
    }

    #[test]
    fn only_files_that_do_not_open_are_problems() {
        let missing = FirstRun {
            catalog: Part::Missing,
            signals: Part::Missing,
            ..first_run()
        };

        assert!(file_problems(&first_run(), None).is_empty());
        assert!(file_problems(&missing, None).is_empty());
        assert_eq!(
            file_problems(&first_run(), Some("config not found")),
            ["gamepalate could not open the data files: config not found"]
        );
        assert!(
            file_problems(
                &FirstRun {
                    data_dir: None,
                    ..first_run()
                },
                None
            )[0]
            .contains("GAMEPALATE_DATA_DIR")
        );
    }

    #[test]
    fn an_empty_profile_says_how_many_hours_are_needed() {
        assert!(empty_profile_text(2.0).starts_with("No game in your library has 2 hours of play"));
        assert!(empty_profile_text(1.0).starts_with("No game in your library has 1 hour of play"));
        assert!(empty_profile_text(1.5).contains("1.5 hours"));
    }

    #[test]
    fn only_an_empty_profile_stops_the_setup() {
        assert_eq!(
            ready_title(Some(&Ready::Empty { min_hours: 2.0 })),
            "Not enough play time yet"
        );
        assert_eq!(
            ready_title(Some(&Ready::Preparing)),
            "Preparing your profile"
        );
        assert_eq!(ready_title(None), "Preparing your profile");
    }

    #[test]
    fn an_invalid_file_shows_its_message() {
        let first_run = FirstRun {
            signals: Part::Invalid("the signals file is not valid".to_string()),
            ..first_run()
        };

        assert_eq!(
            file_problems(&first_run, None),
            ["the signals file is not valid"]
        );
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
    fn the_account_is_done_with_a_steam_id_and_a_key() {
        let same = account("76561198000000000", "");

        assert!(account_done(&first_run()));
        assert!(!changes_account(&same, &first_run()));
        assert!(changes_account(
            &account("76561198000000001", ""),
            &first_run()
        ));
        assert!(!changes_account(&same, &new_account()));
        assert!(!account_done(&FirstRun {
            api_key: false,
            ..first_run()
        }));
    }

    #[test]
    fn the_steam_id_field_says_what_is_wrong() {
        assert_eq!(steam_id_note(""), None);
        assert_eq!(steam_id_note("76561198000000000"), None);
        assert!(steam_id_note("123").unwrap().contains("17-digit"));
        assert_eq!(steam_id_note("https://steamcommunity.com/id/someone"), None);
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
        assert!(needs_privacy(&connect_error_text(&hidden)));
        assert!(!needs_privacy(&connect_error_text(&down)));
        assert!(
            connect_error_text(&ConnectError::NoProfile("nobody".to_string()))
                .starts_with("Steam has no profile at that address")
        );
        assert!(connect_error_text(&down).starts_with("Steam is not responding"));
        assert!(
            connect_error_text(&ConnectError::SteamDown).starts_with("Steam is not responding")
        );
        assert_eq!(connect_error_text(&missing), "Paste the Steam Web API key");
    }
}
