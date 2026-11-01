use std::{
    cmp::Ordering,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{
    AppId, Gamepalate, SteamError,
    account::{ConnectError, connect, parse_profile},
    ready::{ReadyCheck, check_ready},
    setup::{Covers, FirstRun, Part, error_chain, most_played},
    update::download_catalog,
};
use iced::{
    Center, ContentFit, Element, Fill, Task,
    futures::{SinkExt, channel::mpsc},
    padding, stream,
    widget::{
        button, column, container, image, progress_bar, row, scrollable, space, stack, text,
        text_input,
    },
};

use super::library::hours;
use crate::{
    font, format,
    progress::{Stage, Tracker, update_error_text},
    style,
};

const CHECK: char = '\u{E73E}';
const WIDTH: f32 = 760.0;
const LABEL_WIDTH: f32 = 96.0;
const STEP_MARK: f32 = 28.0;
const PLAYED_ASKED: usize = 12;
const PLAYED_SHOWN: usize = 6;
const TILE_WIDTH: f32 = 112.0;
const TILE_HEIGHT: f32 = 168.0;
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
const FIND_STEAM_ID: &str = "Open your profile on steamcommunity.com and copy the address.";

pub struct Setup {
    first_run: FirstRun,
    error: Option<String>,
    account: Account,
    download: Download,
    ready: Option<Ready>,
    covers: Option<Arc<Covers>>,
    played: Vec<Played>,
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
enum Step {
    Account,
    Catalog,
    Profile,
}

impl Step {
    const ALL: [Step; 3] = [Step::Account, Step::Catalog, Step::Profile];

    fn name(self) -> &'static str {
        match self {
            Step::Account => "Account",
            Step::Catalog => "Catalog",
            Step::Profile => "Profile",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepState {
    Done,
    Current,
    Next,
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
    Download,
    Progress(gamepalate_app::update::Progress),
    Downloaded(Result<(), String>),
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

        Setup {
            first_run,
            error,
            account,
            download: Download::default(),
            ready: None,
            covers: Covers::new().ok().map(Arc::new),
            played,
        }
    }

    pub fn begin(&mut self) -> Task<Message> {
        let covers = self.load_covers();
        if current_step(&self.account, &self.first_run) != Step::Catalog {
            return covers;
        }

        match self.download() {
            Action::Run(download) => Task::batch([covers, download]),
            Action::None | Action::Open(_) => covers,
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
                self.account.editing = true;
                self.account.key.clear();
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
                self.prepare(false)
            }
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
        if self.download.running || !account_done(&self.account, &self.first_run) {
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

    fn connect(&mut self) -> Action {
        if !can_connect(&self.account, &self.first_run) {
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

    pub fn view(&self) -> Element<'_, Message> {
        let step = current_step(&self.account, &self.first_run);
        let card = container(self.step_view(step))
            .padding(24)
            .width(Fill)
            .style(style::card);

        scrollable(
            container(
                column![]
                    .push(self.problems_view())
                    .push(steps_view(step))
                    .push(self.done_view(step))
                    .push(card)
                    .push(self.most_played_view(step))
                    .push(self.folder_view())
                    .spacing(28)
                    .max_width(WIDTH),
            )
            .padding(40)
            .center_x(Fill),
        )
        .height(Fill)
        .into()
    }

    fn problems_view(&self) -> Option<Element<'_, Message>> {
        let problems = file_problems(&self.first_run, self.error.as_deref());
        if problems.is_empty() {
            return None;
        }
        let actions = row![small_button("Check again", Message::CheckAgain)]
            .push(
                self.first_run
                    .data_dir
                    .is_some()
                    .then(|| link("Open folder", Message::OpenFolder)),
            )
            .spacing(16)
            .align_y(Center);

        Some(
            container(
                column(problems.into_iter().map(|problem| {
                    text(problem)
                        .size(font::BAR_SIZE)
                        .style(text::danger)
                        .into()
                }))
                .push(actions)
                .spacing(12),
            )
            .padding(16)
            .width(Fill)
            .style(style::card)
            .into(),
        )
    }

    fn done_view(&self, current: Step) -> Option<Element<'_, Message>> {
        if current == Step::Account {
            return None;
        }
        let steam_id = self
            .first_run
            .saved_steam_id
            .map(|id| id.0.to_string())
            .unwrap_or_default();
        let account = row![
            check_mark(),
            text("Steam account")
                .size(font::BAR_SIZE)
                .font(font::SEMIBOLD),
            text(steam_id).size(font::BAR_SIZE).style(style::muted),
        ]
        .push(can_change(&self.download).then(|| link("Change", Message::Change)))
        .spacing(12)
        .align_y(Center);
        let catalog = (current == Step::Profile).then(|| {
            row![
                check_mark(),
                text("Steam catalog")
                    .size(font::BAR_SIZE)
                    .font(font::SEMIBOLD),
            ]
            .spacing(12)
            .align_y(Center)
        });

        Some(column![account].push(catalog).spacing(8).into())
    }

    fn step_view(&self, step: Step) -> Element<'_, Message> {
        let (title, about, body) = match step {
            Step::Account => ("Steam account", Some(ACCOUNT_ABOUT), self.account_view()),
            Step::Catalog => return self.catalog_view(),
            Step::Profile => (ready_title(self.ready.as_ref()), None, self.ready_view()),
        };

        column![
            column![text(title).size(font::PRICE_SIZE).font(font::SEMIBOLD)]
                .push(about.map(|about| text(about).size(font::BAR_SIZE).style(style::muted)))
                .spacing(6),
            body,
        ]
        .spacing(20)
        .into()
    }

    fn most_played_view(&self, step: Step) -> Option<Element<'_, Message>> {
        if step != Step::Catalog {
            return None;
        }
        let tiles: Vec<Element<'_, Message>> = shown(&self.played).map(cover_tile).collect();
        if tiles.is_empty() {
            return None;
        }

        Some(
            column![
                column![
                    text("Your most played")
                        .size(font::INFO_SIZE)
                        .font(font::SEMIBOLD),
                    note("Recommendations start from these games."),
                ]
                .spacing(4),
                row(tiles).spacing(12),
            ]
            .spacing(12)
            .into(),
        )
    }

    fn folder_view(&self) -> Option<Element<'_, Message>> {
        let dir = self.first_run.data_dir.as_ref()?;

        Some(
            row![
                note(format!("Data folder {}", dir.display())),
                button(text("Open folder").size(font::NOTE_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenFolder),
            ]
            .spacing(12)
            .align_y(Center)
            .into(),
        )
    }

    fn account_view(&self) -> Element<'_, Message> {
        let account = &self.account;
        let editable = !account.connecting;
        let steam_id = text_input(PROFILE_PLACEHOLDER, &account.steam_id)
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
            "Already saved"
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
            link("Get a key ↗", Message::GetKey),
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
        let error = account
            .error
            .as_ref()
            .map(|error| text(error.clone()).size(font::BAR_SIZE).style(text::danger));
        let privacy = account
            .error
            .as_deref()
            .filter(|error| needs_privacy(error))
            .map(|_| note_link("Open privacy settings ↗", Message::OpenPrivacy));
        let connect = row![
            container(column![].push(invalid).push(error).push(privacy).spacing(4)).width(Fill),
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
        .spacing(16)
        .align_y(Center);

        column![
            field("Profile", column![steam_id, steam_id_note].spacing(4)),
            field("API key", column![key, note(KEY_DOMAIN_NOTE)].spacing(4)),
            connect,
        ]
        .spacing(12)
        .into()
    }
}

impl Setup {
    fn catalog_view(&self) -> Element<'_, Message> {
        if self.download.running {
            self.progress_view()
        } else {
            self.start_view()
        }
    }

    fn ready_view(&self) -> Element<'_, Message> {
        let Some(ready) = &self.ready else {
            return column![
                text("Read your library from Steam to finish").size(font::BAR_SIZE),
                small_button("Read my library", Message::Prepare(true)),
            ]
            .spacing(8)
            .into();
        };

        match ready {
            Ready::Preparing => note(PREPARING).into(),
            Ready::Failed(error) => column![
                text(error.clone()).size(font::BAR_SIZE).style(text::danger),
                small_button("Retry", Message::Prepare(false)),
            ]
            .spacing(8)
            .into(),
            Ready::Empty { min_hours } => column![
                text(empty_profile_text(*min_hours)).size(font::BAR_SIZE),
                row![
                    button(text("Open Steam library").size(font::BAR_SIZE))
                        .padding([8, 20])
                        .style(button::secondary)
                        .on_press(Message::OpenLibrary),
                    small_button("Check again", Message::Prepare(true)),
                ]
                .spacing(12),
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
        let label = if self.download.error.is_some() {
            "Retry"
        } else {
            "Download catalog"
        };

        column![
            text("Steam catalog")
                .size(font::PRICE_SIZE)
                .font(font::SEMIBOLD)
        ]
        .push(error)
        .push(
            button(text(label).size(font::BAR_SIZE))
                .padding([8, 20])
                .on_press(Message::Download),
        )
        .spacing(12)
        .into()
    }

    fn progress_view(&self) -> Element<'_, Message> {
        let tracker = &self.download.tracker;
        let stage = tracker.stage();
        let (done, total) = tracker.done_of();

        column![
            row![
                container(
                    text("Downloading the Steam catalog")
                        .size(font::PRICE_SIZE)
                        .font(font::SEMIBOLD)
                )
                .width(Fill)
            ]
            .push(
                percent(stage)
                    .map(|percent| { text(percent).size(font::PRICE_SIZE).font(font::SEMIBOLD) })
            )
            .align_y(Center),
            progress_bar(0.0..=total.max(1) as f32, done as f32)
                .girth(10)
                .length(Fill)
                .style(style::meter),
            note(download_status(
                stage,
                tracker.time_left(),
                self.first_run.resume_within_hours
            )),
        ]
        .spacing(12)
        .into()
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
    let _ = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    Action::None
}

fn small_button(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label).size(font::BAR_SIZE))
        .padding([8, 20])
        .on_press(message)
        .into()
}

fn note<'a>(line: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(line).size(font::NOTE_SIZE).style(style::muted)
}

fn account_done(account: &Account, first_run: &FirstRun) -> bool {
    !account.editing && first_run.steam_id == Part::Present && first_run.api_key
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
        .then_some("Paste the address of your profile, or the 17-digit Steam ID.")
}

pub(crate) fn needs_privacy(error: &str) -> bool {
    error == PRIVATE_LIBRARY
}

pub(crate) fn note_link<'a, M: Clone + 'a>(label: &'static str, message: M) -> Element<'a, M> {
    button(text(label).size(font::NOTE_SIZE))
        .padding(0)
        .style(style::link)
        .on_press(message)
        .into()
}

pub(crate) fn connect_error_text(error: &ConnectError) -> String {
    let down = "Steam is not responding: try again in a minute.";
    match error.steam_error() {
        Some(SteamError::KeyRejected { .. }) => {
            "Steam rejected the key: check that you copied all of it.".to_string()
        }
        Some(SteamError::LibraryHidden { .. }) => PRIVATE_LIBRARY.to_string(),
        Some(SteamError::MissingKey { .. }) => "Write the Steam Web API key.".to_string(),
        Some(SteamError::SteamDown { .. }) => down.to_string(),
        _ if matches!(error, ConnectError::SteamDown) => down.to_string(),
        _ if matches!(error, ConnectError::NoProfile(_)) => {
            "Steam has no profile at that address. Check the name after /id/.".to_string()
        }
        _ => error_chain(error),
    }
}

fn current_step(account: &Account, first_run: &FirstRun) -> Step {
    if !account_done(account, first_run) {
        Step::Account
    } else if first_run.catalog != Part::Present {
        Step::Catalog
    } else {
        Step::Profile
    }
}

fn step_state(step: Step, current: Step) -> StepState {
    match step.cmp(&current) {
        Ordering::Less => StepState::Done,
        Ordering::Equal => StepState::Current,
        Ordering::Greater => StepState::Next,
    }
}

fn download_status(stage: Stage, left: Option<Duration>, resume_within_hours: u32) -> String {
    let status = match (stage, left) {
        (Stage::Starting | Stage::Reading, _) => "Reading the list of Steam games".to_string(),
        (Stage::StoreData { .. }, Some(left)) => format::time_left(left),
        (Stage::StoreData { .. }, None) => "Working out the time left".to_string(),
        (Stage::Writing, _) => "Saving the catalog".to_string(),
    };
    format!(
        "{status}. If you close gamepalate now, the download picks up from here when you open it again within {resume_within_hours} hours."
    )
}

fn percent(stage: Stage) -> Option<String> {
    match stage {
        Stage::StoreData { done, total } if total > 0 => {
            Some(format!("{}%", (done as f64 / total as f64 * 100.0).round()))
        }
        _ => None,
    }
}

fn shown(played: &[Played]) -> impl Iterator<Item = &Played> {
    played
        .iter()
        .filter(|game| !matches!(game.cover, Cover::Missing))
        .take(PLAYED_SHOWN)
}

fn cover_tile(game: &Played) -> Element<'_, Message> {
    let picture: Element<'_, Message> = match &game.cover {
        Cover::Ready(handle) => image(handle.clone())
            .width(TILE_WIDTH)
            .height(TILE_HEIGHT)
            .content_fit(ContentFit::Cover)
            .border_radius(IMAGE_RADIUS)
            .into(),
        Cover::Loading | Cover::Missing => container(space())
            .width(TILE_WIDTH)
            .height(TILE_HEIGHT)
            .style(style::picture)
            .into(),
    };
    let strip = container(text(hours(game.hours)).size(font::NOTE_SIZE))
        .padding([3, 8])
        .width(Fill)
        .style(style::capsule_strip);

    stack![picture, column![space().height(Fill), strip]]
        .width(TILE_WIDTH)
        .height(TILE_HEIGHT)
        .into()
}

fn can_change(download: &Download) -> bool {
    !download.running
}

fn file_problems(first_run: &FirstRun, error: Option<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    if first_run.data_dir.is_none() {
        problems.push("Could not find the user data folder: set GAMEPALATE_DATA_DIR.".to_string());
    }
    if let Part::Invalid(message) = &first_run.signals {
        problems.push(message.clone());
    }
    if let Some(error) = error {
        problems.push(format!("gamepalate could not open the data files: {error}"));
    }
    problems
}

fn steps_view<'a>(current: Step) -> Element<'a, Message> {
    let mut steps = row![].spacing(12).align_y(Center);
    for (index, step) in Step::ALL.into_iter().enumerate() {
        if index > 0 {
            steps = steps.push(
                container(column![])
                    .width(Fill)
                    .height(2)
                    .style(style::step_line(step <= current)),
            );
        }
        steps = steps.push(step_label(index + 1, step, step_state(step, current)));
    }
    steps.into()
}

fn step_label<'a>(number: usize, step: Step, state: StepState) -> Element<'a, Message> {
    let mark = match state {
        StepState::Done => text(CHECK).font(font::ICONS).size(12),
        StepState::Current | StepState::Next => text(number.to_string())
            .size(font::BAR_SIZE)
            .font(font::SEMIBOLD),
    };
    let name = text(step.name())
        .size(font::BAR_SIZE)
        .font(if state == StepState::Current {
            font::SEMIBOLD
        } else {
            font::REGULAR
        });
    let name = if state == StepState::Next {
        name.style(style::muted)
    } else {
        name
    };

    row![
        container(mark)
            .width(STEP_MARK)
            .height(STEP_MARK)
            .align_x(Center)
            .align_y(Center)
            .style(style::step(
                state == StepState::Done,
                state == StepState::Current
            )),
        name,
    ]
    .spacing(8)
    .align_y(Center)
    .into()
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

fn check_mark<'a>() -> Element<'a, Message> {
    text(CHECK)
        .font(font::ICONS)
        .size(12)
        .style(text::success)
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
            pending_update: None,
            resume_within_hours: 24,
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
    fn the_current_step_is_the_first_missing_one() {
        let saved = account("76561198000000000", "");
        let editing = Account {
            editing: true,
            ..account("76561198000000000", "")
        };
        let without_catalog = FirstRun {
            catalog: Part::Missing,
            ..first_run()
        };
        let without_key = FirstRun {
            api_key: false,
            catalog: Part::Missing,
            ..first_run()
        };

        assert_eq!(current_step(&saved, &without_key), Step::Account);
        assert_eq!(current_step(&editing, &first_run()), Step::Account);
        assert_eq!(current_step(&saved, &without_catalog), Step::Catalog);
        assert_eq!(current_step(&saved, &first_run()), Step::Profile);
    }

    #[test]
    fn the_steps_before_the_current_one_are_done() {
        assert_eq!(step_state(Step::Account, Step::Catalog), StepState::Done);
        assert_eq!(step_state(Step::Catalog, Step::Catalog), StepState::Current);
        assert_eq!(step_state(Step::Profile, Step::Catalog), StepState::Next);
        assert_eq!(step_state(Step::Account, Step::Account), StepState::Current);
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

        let ids: Vec<u32> = shown(&played).map(|game| game.id.0).collect();

        assert_eq!(ids, [1, 3, 4, 6, 7, 8]);
    }

    #[test]
    fn the_download_says_what_it_is_doing() {
        let store_data = Stage::StoreData {
            done: 312,
            total: 757,
        };

        assert_eq!(
            download_status(store_data, Some(Duration::from_secs(720)), 24),
            "About 12 minutes left. If you close gamepalate now, the download picks up from here when you open it again within 24 hours."
        );
        assert!(download_status(store_data, None, 24).starts_with("Working out the time left."));
        assert!(download_status(Stage::Reading, None, 24).starts_with("Reading the list"));
        assert!(download_status(Stage::Writing, None, 24).starts_with("Saving the catalog."));
    }

    #[test]
    fn the_percent_needs_a_total() {
        assert_eq!(
            percent(Stage::StoreData {
                done: 312,
                total: 757
            }),
            Some("41%".to_string())
        );
        assert_eq!(
            percent(Stage::StoreData {
                done: 757,
                total: 757
            }),
            Some("100%".to_string())
        );
        assert_eq!(percent(Stage::StoreData { done: 0, total: 0 }), None);
        assert_eq!(percent(Stage::Reading), None);
        assert_eq!(percent(Stage::Writing), None);
    }

    #[test]
    fn the_account_does_not_change_during_the_download() {
        let running = Download {
            running: true,
            ..Download::default()
        };

        assert!(can_change(&Download::default()));
        assert!(!can_change(&running));
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
        assert!(
            empty_profile_text(2.0).starts_with("No game in your library has 2 hours of play.")
        );
        assert!(empty_profile_text(1.0).starts_with("No game in your library has 1 hour of play."));
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
        assert_eq!(connect_error_text(&missing), "Write the Steam Web API key.");
    }
}
