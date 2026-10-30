use std::{
    collections::VecDeque,
    process::Command,
    time::{Duration, Instant},
};

use gamepalate_app::{
    Gamepalate, SteamError,
    account::{ConnectError, SteamIdError, connect, parse_steam_id},
    ready::{ReadyCheck, check_ready},
    setup::{FirstRun, Part, error_chain},
    update::{Progress, UpdateError, download_catalog},
};
use iced::{
    Center, Element, Fill, Task,
    futures::{SinkExt, channel::mpsc},
    padding, stream,
    widget::{button, column, container, progress_bar, row, text, text_input},
};

use crate::{font, format, style};

const CHECK: char = '\u{E73E}';
const CROSS: char = '\u{E711}';
const WIDTH: f32 = 760.0;
const KEY_PAGE: &str = "https://steamcommunity.com/dev/apikey";
const RECENT_BATCHES: usize = 20;
const MIN_BATCHES: usize = 10;
const HOW_IT_WORKS: [&str; 3] = [
    "Signals: the hours you played tell gamepalate what you like.",
    "Families: groups of your games, to ask for more of the same kind.",
    "Votes: every game you vote on teaches gamepalate a bit more.",
];
const FIND_STEAM_ID: &str =
    "Open Steam → your name at the top right → Account details: the Steam ID is under your name.";

pub struct Setup {
    first_run: FirstRun,
    error: Option<String>,
    account: Account,
    download: Download,
    ready: Option<Ready>,
}

#[derive(Debug)]
enum Ready {
    Preparing,
    Checked(ReadyCheck),
    Failed(String),
}

#[derive(Debug, Default)]
struct Download {
    running: bool,
    stage: Stage,
    batches: VecDeque<Instant>,
    error: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Stage {
    #[default]
    Starting,
    Reading,
    StoreData {
        done: usize,
        total: usize,
    },
    Writing,
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
    Progress(Progress),
    Downloaded(Result<(), String>),
    Prepare(bool),
    Prepared(Result<ReadyCheck, String>),
    StartVoting,
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
            download: Download::default(),
            ready: None,
        }
    }

    pub fn first_run(&self) -> &FirstRun {
        &self.first_run
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::CheckAgain | Message::Connected(Ok(())) | Message::StartVoting => {
                self.check_again()
            }
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
            Message::Download => self.download(),
            Message::Progress(progress) => {
                self.download.advance(progress, Instant::now());
                Action::None
            }
            Message::Downloaded(Ok(())) => {
                self.first_run = FirstRun::check(None);
                self.download = Download::default();
                self.prepare(false)
            }
            Message::Prepare(read_signals_again) => self.prepare(read_signals_again),
            Message::Prepared(result) => {
                self.ready = Some(match result {
                    Ok(check) => Ready::Checked(check),
                    Err(error) => Ready::Failed(error),
                });
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
            column![title, folder, self.account_view(), self.catalog_view(),]
                .push(self.ready_view())
                .push(lines)
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

impl Setup {
    fn catalog_view(&self) -> Element<'_, Message> {
        let ready = self.first_run.catalog == Part::Present;
        let header = row![
            mark(ready),
            text("Catalog of Steam games")
                .size(font::INFO_SIZE)
                .font(font::SEMIBOLD)
        ]
        .spacing(12)
        .align_y(Center);
        if ready {
            return header.into();
        }

        let body: Element<'_, Message> = if !account_done(&self.account, &self.first_run) {
            note("Connect your Steam account first.").into()
        } else if self.download.running {
            self.progress_view()
        } else {
            self.start_view()
        };

        column![header, container(body).padding(padding::left(24))]
            .spacing(8)
            .into()
    }

    fn ready_view(&self) -> Option<Element<'_, Message>> {
        let (done, body): (bool, Element<'_, Message>) = match self.ready.as_ref()? {
            Ready::Preparing => (false, note("Preparing your profile…").into()),
            Ready::Failed(error) => (
                false,
                column![
                    text(error.clone()).size(font::BAR_SIZE).style(text::danger),
                    small_button("Retry", Message::Prepare(false)),
                ]
                .spacing(8)
                .into(),
            ),
            Ready::Checked(check) if check.empty_profile => (
                false,
                column![
                    text(empty_profile_text(check.min_hours)).size(font::BAR_SIZE),
                    small_button("Check again", Message::Prepare(true)),
                ]
                .spacing(8)
                .into(),
            ),
            Ready::Checked(check) => (
                true,
                column![
                    text(ready_text(check)).size(font::BAR_SIZE),
                    small_button("Start voting", Message::StartVoting),
                ]
                .spacing(8)
                .into(),
            ),
        };

        Some(
            column![
                row![
                    mark(done),
                    text("Ready").size(font::INFO_SIZE).font(font::SEMIBOLD)
                ]
                .spacing(12)
                .align_y(Center),
                container(body).padding(padding::left(24)),
            ]
            .spacing(8)
            .into(),
        )
    }

    fn start_view(&self) -> Element<'_, Message> {
        let download = &self.download;
        let pending = self.first_run.pending_update;
        let label = if download.error.is_some() {
            "Retry"
        } else if pending.is_some() {
            "Resume"
        } else {
            "Download catalog"
        };
        let about = match pending {
            Some(pending) => format!(
                "An update started on {} is waiting: {} responses saved.",
                format::day_time(pending.started_at),
                pending.responses
            ),
            None => "It takes 15-25 minutes the first time.".to_string(),
        };

        column![]
            .push(
                download
                    .error
                    .as_ref()
                    .map(|error| text(error.clone()).size(font::BAR_SIZE).style(text::danger)),
            )
            .push(note(about))
            .push(
                button(text(label).size(font::BAR_SIZE))
                    .padding([8, 20])
                    .on_press(Message::Download),
            )
            .spacing(8)
            .into()
    }

    fn progress_view(&self) -> Element<'_, Message> {
        let download = &self.download;
        let (done, total) = match download.stage {
            Stage::StoreData { done, total } => (done, total),
            Stage::Writing => (1, 1),
            Stage::Starting | Stage::Reading => (0, 1),
        };
        let left = download
            .time_left(done, total)
            .map(|left| note(format::time_left(left)));

        column![
            text(stage_text(download.stage)).size(font::BAR_SIZE),
            progress_bar(0.0..=total.max(1) as f32, done as f32)
                .girth(6)
                .length(Fill)
                .style(style::meter),
        ]
        .push(left)
        .push(note(format!(
            "You can close gamepalate: the update resumes where it stopped, within {} hours.",
            self.first_run.resume_within_hours
        )))
        .push(column(HOW_IT_WORKS.into_iter().map(|line| note(line).into())).spacing(2))
        .spacing(8)
        .into()
    }
}

impl Download {
    fn advance(&mut self, progress: Progress, now: Instant) {
        match progress {
            Progress::Resumed { .. } => {}
            Progress::ReadingApps => self.stage = Stage::Reading,
            Progress::StoreData { done, total } => {
                self.stage = Stage::StoreData { done, total };
                if done > 0 {
                    self.batches.push_back(now);
                    if self.batches.len() > RECENT_BATCHES {
                        self.batches.pop_front();
                    }
                }
            }
            Progress::Writing => self.stage = Stage::Writing,
        }
    }

    fn time_left(&self, done: usize, total: usize) -> Option<Duration> {
        let first = *self.batches.front()?;
        let batches: Vec<Duration> = self
            .batches
            .iter()
            .map(|at| at.duration_since(first))
            .collect();
        time_left(&batches, done, total)
    }
}

fn time_left(batches: &[Duration], done: usize, total: usize) -> Option<Duration> {
    if done < MIN_BATCHES || batches.len() < 2 {
        return None;
    }
    let span = *batches.last()? - batches[0];
    let per_batch = span / (batches.len() as u32 - 1);

    Some(per_batch * total.saturating_sub(done) as u32)
}

fn stage_text(stage: Stage) -> String {
    match stage {
        Stage::Starting | Stage::Reading => "Reading the list of Steam games…".to_string(),
        Stage::StoreData { done, total } => format!("Store data: {done} of {total} requests"),
        Stage::Writing => "Writing the catalog…".to_string(),
    }
}

fn update_error_text(error: &UpdateError) -> String {
    match error {
        UpdateError::Stopped {
            resume_before,
            source,
        } => {
            let why = match source {
                SteamError::SteamDown { .. } => "Steam is not responding".to_string(),
                source => error_chain(source),
            };
            format!(
                "Update stopped: {why}. Run it again before {} to resume.",
                format::day_time(*resume_before)
            )
        }
        UpdateError::StartOver { .. } => {
            "Steam answered in an unexpected way: the next update starts over.".to_string()
        }
        error => error_chain(error),
    }
}

fn empty_profile_text(min_hours: f64) -> String {
    let hours = if min_hours == 1.0 {
        "1 hour".to_string()
    } else {
        format!("{min_hours} hours")
    };
    format!(
        "None of your games has {hours} of play yet. Play one for a while, or add some to your wishlist, then Check again."
    )
}

fn ready_text(check: &ReadyCheck) -> String {
    format!(
        "Catalog: {} games · Your library: {} games",
        format::count(check.games as u32),
        format::count(check.library as u32)
    )
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
    vec![part_line(
        "Your Signals",
        &first_run.signals,
        "connect your Steam account, or run: gamepalate profile",
    )]
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
            pending_update: None,
            resume_within_hours: 24,
            signals: Part::Present,
        }
    }

    fn seconds(values: &[u64]) -> Vec<Duration> {
        values
            .iter()
            .map(|value| Duration::from_secs(*value))
            .collect()
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
    fn there_is_no_estimate_before_ten_batches() {
        assert_eq!(time_left(&seconds(&[0, 1, 2]), 9, 100), None);
        assert_eq!(time_left(&seconds(&[0]), 10, 100), None);
    }

    #[test]
    fn a_steady_pace_gives_the_time_left() {
        let batches = seconds(&[0, 2, 4, 6, 8, 10, 12, 14, 16, 18]);

        assert_eq!(time_left(&batches, 10, 100), Some(Duration::from_secs(180)));
    }

    #[test]
    fn after_a_fast_start_the_estimate_follows_the_last_batches() {
        let mut download = Download::default();
        let start = Instant::now();
        for done in 1..=30 {
            let at = if done <= 10 {
                start
            } else {
                start + Duration::from_secs(3 * (done - 10))
            };
            download.advance(
                Progress::StoreData {
                    done: done as usize,
                    total: 130,
                },
                at,
            );
        }

        assert_eq!(download.batches.len(), RECENT_BATCHES);
        assert_eq!(download.time_left(30, 130), Some(Duration::from_secs(300)));
    }

    #[test]
    fn an_empty_profile_says_how_many_hours_are_needed() {
        assert!(empty_profile_text(2.0).starts_with("None of your games has 2 hours of play yet."));
        assert!(empty_profile_text(1.0).starts_with("None of your games has 1 hour of play yet."));
        assert!(empty_profile_text(1.5).contains("1.5 hours"));
    }

    #[test]
    fn a_ready_profile_counts_the_catalog_and_the_library() {
        let check = ReadyCheck {
            games: 84_312,
            library: 312,
            min_hours: 2.0,
            empty_profile: false,
        };

        assert_eq!(
            ready_text(&check),
            "Catalog: 84,312 games · Your library: 312 games"
        );
    }

    #[test]
    fn each_stage_has_its_sentence() {
        assert_eq!(
            stage_text(Stage::Reading),
            "Reading the list of Steam games…"
        );
        assert_eq!(
            stage_text(Stage::StoreData {
                done: 312,
                total: 757
            }),
            "Store data: 312 of 757 requests"
        );
        assert_eq!(stage_text(Stage::Writing), "Writing the catalog…");
    }

    #[test]
    fn a_stopped_update_says_until_when_it_resumes() {
        let resume_before = gamepalate_app::Timestamp(1_793_369_460_000);
        let stopped = UpdateError::Stopped {
            resume_before,
            source: SteamError::SteamDown {
                endpoint: "IStoreBrowseService/GetItems/v1".to_string(),
                reason: "timed out".to_string(),
                source: None,
            },
        };

        assert_eq!(
            update_error_text(&stopped),
            format!(
                "Update stopped: Steam is not responding. Run it again before {} to resume.",
                format::day_time(resume_before)
            )
        );
    }

    #[test]
    fn an_update_that_starts_over_says_so() {
        let start_over = UpdateError::StartOver {
            source: SteamError::Shape {
                endpoint: "IStoreBrowseService/GetItems/v1".to_string(),
                reason: "unexpected".to_string(),
            },
            not_marked: None,
        };

        assert_eq!(
            update_error_text(&start_over),
            "Steam answered in an unexpected way: the next update starts over."
        );
    }

    #[test]
    fn an_invalid_file_shows_its_message() {
        let first_run = FirstRun {
            signals: Part::Invalid("the signals file is not valid".to_string()),
            ..first_run()
        };

        let line = &lines(&first_run)[0];

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
