use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{
    Gamepalate, GuiSettings, PendingUpdate, Region, SavedWindow, ThemeChoice, Timestamp,
    refresh::{Step, refresh},
    setup::{FirstRun, error_chain},
};
use iced::{
    Center, Color, Element, Fill, Length, Shrink, Size, Subscription, Task, Theme,
    futures::{SinkExt, channel::mpsc},
    keyboard::{self, Key},
    mouse, padding, stream, task,
    theme::Palette,
    widget::{
        button, column, container, mouse_area, progress_bar, row, space, stack, text, tooltip,
    },
    window::{self, Direction},
};

use crate::{
    font, format,
    progress::{self, Stage, Tracker},
    screen::{
        Discover, Library, Profile, Screen, Settings, Votes, discover, library, profile, settings,
        setup, votes,
    },
    style,
};

const UNDERLINE_HEIGHT: f32 = 2.0;
const RESIZE_BORDER: f32 = 5.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
const MINIMIZE: char = '\u{E921}';
const MAXIMIZE: char = '\u{E922}';
const RESTORE: char = '\u{E923}';
const CLOSE: char = '\u{E8BB}';
const SUN: char = '\u{E706}';
const MOON: char = '\u{E708}';
const REFRESH: char = '\u{E72C}';
const CANCEL: char = '\u{E711}';
const GEAR: char = '\u{E713}';
const DEFAULT_SIZE: Size = Size::new(1280.0, 800.0);
const MIN_SIZE: Size = Size::new(960.0, 600.0);
const DARK_BACKGROUND: Color = Color::from_rgb8(0x10, 0x11, 0x13);
const DARK_TEXT: Color = Color::from_rgb8(0xf5, 0xf5, 0xf5);
const DARK_SUCCESS: Color = Color::from_rgb8(0x78, 0xe3, 0xb3);
const DARK_WARNING: Color = Color::from_rgb8(0xff, 0xe2, 0xa3);

pub struct App {
    gamepalate: Option<Arc<Gamepalate>>,
    screen: Screen,
    theme: Theme,
    window: Option<window::Id>,
    maximized: bool,
    size: Size,
    resized: Size,
    last_bar_press: Option<Instant>,
    refresh: Refresh,
    pending: Option<PendingUpdate>,
}

#[derive(Debug, Default)]
enum Refresh {
    #[default]
    Idle,
    Running(Refreshing, task::Handle),
    Failed(String),
}

#[derive(Debug)]
enum Refreshing {
    Catalog(Tracker),
    Signals,
    Opening,
}

#[derive(Clone, Debug)]
pub enum Message {
    Navigate(Page),
    Discover(discover::Message),
    Votes(votes::Message),
    Library(library::Message),
    Profile(profile::Message),
    Settings(settings::Message),
    Setup(setup::Message),
    WindowFound(Option<window::Id>),
    Window(WindowAction),
    BarPressed,
    Resized(Size),
    Maximized(bool),
    ToggleTheme,
    Refresh,
    Refreshing(Step),
    Refreshed(Result<Arc<Gamepalate>, String>),
    CancelRefresh,
    DismissRefreshError,
    Reopened(Result<Arc<Gamepalate>, String>),
}

#[derive(Clone, Copy, Debug)]
pub enum WindowAction {
    Minimize,
    ToggleMaximize,
    Close,
    Resize(Direction),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Page {
    Discover,
    Votes,
    Library,
    Profile,
    Settings,
}

impl App {
    pub fn new(saved: Option<SavedWindow>) -> (App, Task<Message>) {
        let mut app = App {
            gamepalate: None,
            screen: Screen::Starting,
            theme: Theme::Light,
            window: None,
            maximized: saved.is_some_and(|saved| saved.maximized),
            size: saved_size(saved),
            resized: saved_size(saved),
            last_bar_press: None,
            refresh: Refresh::Idle,
            pending: None,
        };
        let task = match setup::start(FirstRun::check(None)) {
            Ok(gamepalate) => app.opened(gamepalate),
            Err(mut setup) => {
                app.theme = theme(setup.first_run().gui_settings().theme);
                let task = setup.begin().map(Message::Setup);
                app.screen = Screen::Setup(setup);
                task
            }
        };

        (
            app,
            Task::batch([task, window::latest().map(Message::WindowFound)]),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(page) if self.page() == Some(page) => Task::none(),
            Message::Navigate(page) => self.open(page),
            Message::Discover(message) => {
                if let Screen::Discover(screen) = &mut self.screen {
                    screen.update(message).map(Message::Discover)
                } else {
                    Task::none()
                }
            }
            Message::Votes(message) => {
                if let Screen::Votes(screen) = &mut self.screen {
                    screen.update(message).map(Message::Votes)
                } else {
                    Task::none()
                }
            }
            Message::Library(library::Message::SteamClientAnswered(Ok(()))) => self.reopen(),
            Message::Library(message) => {
                if let Screen::Library(screen) = &mut self.screen {
                    screen.update(message).map(Message::Library)
                } else {
                    Task::none()
                }
            }
            Message::Profile(profile::Message::DiscoverFamily(family)) => {
                let Some(gamepalate) = &self.gamepalate else {
                    return Task::none();
                };
                let (discover, task) = Discover::from_family(gamepalate, family);
                self.screen = Screen::Discover(Box::new(discover));
                task.map(Message::Discover)
            }
            Message::Profile(profile::Message::OpenInLibrary(id)) => {
                let Some(gamepalate) = &self.gamepalate else {
                    return Task::none();
                };
                let (library, task) = Library::showing(gamepalate, id);
                self.screen = Screen::Library(Box::new(library));
                task.map(Message::Library)
            }
            Message::Profile(profile::Message::OpenLibrary) => self.open(Page::Library),
            Message::Profile(profile::Message::OpenVotes(filter)) => {
                let Some(gamepalate) = &self.gamepalate else {
                    return Task::none();
                };
                let (votes, task) = Votes::filtered(gamepalate, filter);
                self.screen = Screen::Votes(Box::new(votes));
                task.map(Message::Votes)
            }
            Message::Profile(message) => {
                if let Screen::Profile(screen) = &mut self.screen {
                    screen.update(message).map(Message::Profile)
                } else {
                    Task::none()
                }
            }
            Message::Settings(message) => {
                let Screen::Settings(screen) = &mut self.screen else {
                    return Task::none();
                };
                match screen.update(message) {
                    settings::Action::None => Task::none(),
                    settings::Action::Run(task) => task.map(Message::Settings),
                    settings::Action::Theme(choice) => self.set_theme(choice),
                    settings::Action::Reopen => self.reopen(),
                    settings::Action::RegionChanged => {
                        Task::batch([self.reopen(), self.start_refresh()])
                    }
                    settings::Action::Close(page) => self.open(page),
                }
            }
            Message::Reopened(Ok(gamepalate)) => {
                self.gamepalate = Some(gamepalate);
                self.find_pending();
                match self.page() {
                    Some(page) => self.open(page),
                    None => Task::none(),
                }
            }
            Message::Reopened(Err(error)) => {
                if let Screen::Settings(screen) = &mut self.screen {
                    screen.failed(error);
                }
                Task::none()
            }
            Message::Setup(message) => {
                let Screen::Setup(screen) = &mut self.screen else {
                    return Task::none();
                };
                match screen.update(message) {
                    setup::Action::None => Task::none(),
                    setup::Action::Run(task) => task.map(Message::Setup),
                    setup::Action::Open(gamepalate) => self.opened(*gamepalate),
                }
            }
            Message::WindowFound(id) => {
                self.window = id;
                Task::none()
            }
            Message::Window(action) => self.window_action(action),
            Message::BarPressed => self.bar_pressed(),
            Message::Resized(size) => {
                self.resized = size;
                match self.window {
                    Some(id) => window::is_maximized(id).map(Message::Maximized),
                    None => Task::none(),
                }
            }
            Message::Maximized(maximized) => {
                self.maximized = maximized;
                if let Some(size) = restored(self.resized, maximized) {
                    self.size = size;
                }
                Task::none()
            }
            Message::Refresh => self.start_refresh(),
            Message::Refreshing(step) => {
                if let Refresh::Running(refreshing, _) = &mut self.refresh {
                    refreshing.advance(step, Instant::now());
                }
                Task::none()
            }
            Message::Refreshed(result) => {
                self.refresh = Refresh::Idle;
                let gamepalate = match result {
                    Ok(gamepalate) => gamepalate,
                    Err(error) => {
                        self.refresh = Refresh::Failed(error);
                        self.find_pending();
                        return Task::none();
                    }
                };
                self.gamepalate = Some(gamepalate);
                self.find_pending();
                match self.page() {
                    Some(page) => self.open(page),
                    None => Task::none(),
                }
            }
            Message::CancelRefresh => {
                if let Refresh::Running(refreshing, handle) = &self.refresh
                    && refreshing.can_cancel()
                {
                    handle.abort();
                    self.refresh = Refresh::Idle;
                    self.find_pending();
                }
                Task::none()
            }
            Message::DismissRefreshError => {
                if matches!(self.refresh, Refresh::Failed(_)) {
                    self.refresh = Refresh::Idle;
                }
                Task::none()
            }
            Message::ToggleTheme => self.set_theme(choice(&toggled(&self.theme))),
        }
    }

    fn set_theme(&mut self, choice: ThemeChoice) -> Task<Message> {
        self.theme = theme(choice);
        self.change_gui_settings(move |settings| settings.theme = choice)
    }

    fn change_gui_settings(
        &self,
        change: impl FnOnce(&mut GuiSettings) + Send + 'static,
    ) -> Task<Message> {
        if let Some(gamepalate) = &self.gamepalate {
            let gamepalate = Arc::clone(gamepalate);
            Task::future(async move { gamepalate.change_gui_settings(change) }).discard()
        } else if let Screen::Setup(setup) = &self.screen {
            let first_run = setup.first_run().clone();
            Task::future(async move { first_run.change_gui_settings(change) }).discard()
        } else {
            Task::none()
        }
    }

    fn reopen(&mut self) -> Task<Message> {
        let Some(gamepalate) = &self.gamepalate else {
            return Task::none();
        };
        let dir = gamepalate.data_dir().to_path_buf();

        Task::perform(
            async move {
                Gamepalate::open(Some(dir)).map(Arc::new).map_err(|error| {
                    format!(
                        "gamepalate could not open the data files: {}",
                        error_chain(&error)
                    )
                })
            },
            Message::Reopened,
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let screen = match &self.screen {
            Screen::Discover(_) => Discover::subscription().map(Message::Discover),
            Screen::Votes(_) => Votes::subscription().map(Message::Votes),
            Screen::Library(_) => Library::subscription().map(Message::Library),
            Screen::Profile(_) => Profile::subscription().map(Message::Profile),
            Screen::Settings(_) => Settings::subscription().map(Message::Settings),
            Screen::Setup(_) | Screen::Starting => Subscription::none(),
        };

        Subscription::batch([
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            window::close_requests().map(|_| Message::Window(WindowAction::Close)),
            keyboard::listen().filter_map(page_key),
            screen,
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        let bar = container(
            row![
                column![
                    container(text("gamepalate").font(font::BOLD).size(font::BAR_SIZE))
                        .height(Fill)
                        .align_y(Center),
                    space().height(UNDERLINE_HEIGHT),
                ]
                .height(Fill),
            ]
            .push(self.gamepalate.is_some().then(|| {
                row![
                    self.tab("Discover", Page::Discover),
                    self.tab("Votes", Page::Votes),
                    self.tab("Library", Page::Library),
                    self.tab("Profile", Page::Profile),
                ]
                .spacing(24)
                .height(Fill)
            }))
            .push(space().width(Fill))
            .push(self.gamepalate.as_deref().map(|gamepalate| {
                column![
                    container(self.status(gamepalate))
                        .height(Fill)
                        .align_y(Center),
                    space().height(UNDERLINE_HEIGHT),
                ]
                .height(Fill)
            }))
            .push(
                row![]
                    .push(
                        self.gamepalate
                            .is_some()
                            .then(|| refresh_button(&self.refresh, self.pending)),
                    )
                    .push(self.gamepalate.is_some().then(settings_button))
                    .push(theme_button(&self.theme))
                    .push(window_button(MINIMIZE, WindowAction::Minimize))
                    .push(window_button(
                        if self.maximized { RESTORE } else { MAXIMIZE },
                        WindowAction::ToggleMaximize,
                    ))
                    .push(window_button(CLOSE, WindowAction::Close))
                    .height(Fill),
            )
            .spacing(24)
            .height(Fill)
            .align_y(Center),
        )
        .padding(padding::left(20))
        .width(Fill)
        .height(44)
        .style(style::top_bar);
        let bar = mouse_area(bar).on_press(Message::BarPressed);

        let content = match &self.screen {
            Screen::Discover(screen) => screen.view().map(Message::Discover),
            Screen::Votes(screen) => screen.view().map(Message::Votes),
            Screen::Library(screen) => screen.view().map(Message::Library),
            Screen::Setup(screen) => screen.view().map(Message::Setup),
            Screen::Profile(screen) => screen.view().map(Message::Profile),
            Screen::Settings(screen) => screen.view(choice(&self.theme)).map(Message::Settings),
            Screen::Starting => column![].into(),
        };

        let app = column![
            bar,
            style::separator(),
            container(content).width(Fill).height(Fill),
        ];

        if self.maximized {
            app.into()
        } else {
            stack![app, resize_borders()].into()
        }
    }

    pub fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn window_action(&self, action: WindowAction) -> Task<Message> {
        if matches!(action, WindowAction::Close) {
            let window = SavedWindow {
                width: self.size.width.round() as u32,
                height: self.size.height.round() as u32,
                maximized: self.maximized,
            };
            return self
                .change_gui_settings(move |settings| settings.window = Some(window))
                .chain(iced::exit());
        }
        let Some(id) = self.window else {
            return Task::none();
        };

        match action {
            WindowAction::Minimize => window::minimize(id, true),
            WindowAction::ToggleMaximize => window::toggle_maximize(id),
            WindowAction::Resize(direction) => window::drag_resize(id, direction),
            WindowAction::Close => Task::none(),
        }
    }

    fn status<'a>(&'a self, gamepalate: &Gamepalate) -> Element<'a, Message> {
        let refreshing = match &self.refresh {
            Refresh::Idle => return freshness(gamepalate),
            Refresh::Failed(error) => return refresh_failed(error),
            Refresh::Running(refreshing, _) => refreshing,
        };
        let bar = match refreshing {
            Refreshing::Catalog(tracker)
                if matches!(tracker.stage(), Stage::StoreData { .. } | Stage::Writing) =>
            {
                Some(
                    progress_bar(0.0..=1.0, tracker.fraction())
                        .girth(4)
                        .length(160)
                        .style(style::meter),
                )
            }
            _ => None,
        };

        row![text(refreshing_text(refreshing)).size(font::NOTE_SIZE)]
            .push(bar)
            .push(
                refreshing_count(refreshing)
                    .map(|count| text(count).size(font::NOTE_SIZE).style(style::muted)),
            )
            .spacing(10)
            .align_y(Center)
            .into()
    }

    fn start_refresh(&mut self) -> Task<Message> {
        let Some(gamepalate) = &self.gamepalate else {
            return Task::none();
        };
        if matches!(self.refresh, Refresh::Running(..)) {
            return Task::none();
        }
        let dir = gamepalate.data_dir().to_path_buf();

        let (task, handle) = Task::run(
            stream::channel(64, async move |mut output: mpsc::Sender<Message>| {
                let mut events = output.clone();
                let result = refresh(&dir, move |step| {
                    let _ = events.try_send(Message::Refreshing(step));
                })
                .await
                .map(|refreshed| Arc::new(refreshed.gamepalate))
                .map_err(|error| progress::refresh_error_text(&error));
                let _ = output.send(Message::Refreshed(result)).await;
            }),
            |message| message,
        )
        .abortable();
        self.refresh = Refresh::Running(Refreshing::Catalog(Tracker::default()), handle);

        task
    }

    fn find_pending(&mut self) {
        self.pending = self
            .gamepalate
            .as_ref()
            .and_then(|gamepalate| gamepalate.pending_update());
    }

    fn bar_pressed(&mut self) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        let now = Instant::now();
        let double = self
            .last_bar_press
            .is_some_and(|last| now.duration_since(last) < DOUBLE_CLICK);
        self.last_bar_press = (!double).then_some(now);

        if double {
            window::toggle_maximize(id)
        } else {
            window::drag(id)
        }
    }

    fn opened(&mut self, gamepalate: Gamepalate) -> Task<Message> {
        let gamepalate = Arc::new(gamepalate);
        let pruner = Arc::clone(&gamepalate);
        let prune_images = Task::future(async move { pruner.prune_images() }).discard();
        self.theme = theme(gamepalate.gui_settings().theme);
        self.gamepalate = Some(gamepalate);
        self.find_pending();

        Task::batch([self.open(Page::Discover), prune_images])
    }

    fn page(&self) -> Option<Page> {
        match &self.screen {
            Screen::Discover(_) => Some(Page::Discover),
            Screen::Votes(_) => Some(Page::Votes),
            Screen::Library(_) => Some(Page::Library),
            Screen::Profile(_) => Some(Page::Profile),
            Screen::Settings(_) => Some(Page::Settings),
            Screen::Setup(_) | Screen::Starting => None,
        }
    }

    fn back_page(&self) -> Page {
        match &self.screen {
            Screen::Settings(settings) => settings.back(),
            _ => self.page().unwrap_or(Page::Discover),
        }
    }

    fn open(&mut self, page: Page) -> Task<Message> {
        let Some(gamepalate) = &self.gamepalate else {
            return Task::none();
        };

        match page {
            Page::Discover => {
                let (discover, task) = Discover::new(gamepalate);
                self.screen = Screen::Discover(Box::new(discover));
                task.map(Message::Discover)
            }
            Page::Votes => {
                let (votes, task) = Votes::new(gamepalate);
                self.screen = Screen::Votes(Box::new(votes));
                task.map(Message::Votes)
            }
            Page::Library => {
                let (library, task) = Library::new(gamepalate);
                self.screen = Screen::Library(Box::new(library));
                task.map(Message::Library)
            }
            Page::Profile => {
                let (profile, task) = Profile::new(gamepalate);
                self.screen = Screen::Profile(Box::new(profile));
                task.map(Message::Profile)
            }
            Page::Settings => {
                let (settings, task) = Settings::new(gamepalate, self.back_page());
                self.screen = Screen::Settings(Box::new(settings));
                task.map(Message::Settings)
            }
        }
    }

    fn tab(&self, label: &'static str, page: Page) -> Element<'_, Message> {
        let current = self.page() == Some(page);
        let label = text(label)
            .font(if current { font::BOLD } else { font::REGULAR })
            .size(font::BAR_SIZE);
        let underline = container(space())
            .width(Fill)
            .height(UNDERLINE_HEIGHT)
            .style(if current {
                style::tab_underline
            } else {
                container::transparent
            });

        button(
            column![container(label).height(Fill).align_y(Center), underline]
                .width(Shrink)
                .height(Fill),
        )
        .padding(0)
        .height(Fill)
        .style(style::tab(current))
        .on_press_maybe((!current).then_some(Message::Navigate(page)))
        .into()
    }
}

fn page_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if !modifiers.command() {
        return None;
    }

    match key.as_ref() {
        Key::Character("1") => Some(Message::Navigate(Page::Discover)),
        Key::Character("2") => Some(Message::Navigate(Page::Votes)),
        Key::Character("3") => Some(Message::Navigate(Page::Library)),
        Key::Character("4") => Some(Message::Navigate(Page::Profile)),
        _ => None,
    }
}

fn freshness<'a>(gamepalate: &Gamepalate) -> Element<'a, Message> {
    let now = Timestamp(jiff::Timestamp::now().as_millisecond());
    let signals = format::when(gamepalate.signals_read_at(), now);
    let prices = prices_text(
        format::when(gamepalate.prices_updated_at(), now),
        gamepalate.prices_region(),
    );

    text!("Signals {signals} · {prices}")
        .size(font::NOTE_SIZE)
        .style(style::muted)
        .into()
}

fn prices_text(when: String, other_region: Option<Region>) -> String {
    match other_region {
        Some(region) => format!("Prices {when} · {region}"),
        None => format!("Prices {when}"),
    }
}

fn window_button<'a>(icon: char, action: WindowAction) -> Element<'a, Message> {
    button(
        container(
            text(icon)
                .font(font::ICONS)
                .size(10)
                .width(Fill)
                .height(Fill)
                .center(),
        )
        .padding(padding::bottom(UNDERLINE_HEIGHT)),
    )
    .width(46)
    .height(Fill)
    .padding(0)
    .style(if matches!(action, WindowAction::Close) {
        style::close_button
    } else {
        style::window_button
    })
    .on_press(Message::Window(action))
    .into()
}

pub fn window_settings(saved: Option<SavedWindow>) -> window::Settings {
    window::Settings {
        size: saved_size(saved),
        maximized: saved.is_some_and(|saved| saved.maximized),
        position: window::Position::Centered,
        min_size: Some(MIN_SIZE),
        decorations: false,
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            undecorated_shadow: true,
            corner_preference: window::settings::platform::CornerPreference::Round,
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    }
}

fn saved_size(saved: Option<SavedWindow>) -> Size {
    saved.map_or(DEFAULT_SIZE, |saved| {
        Size::new(
            (saved.width as f32).max(MIN_SIZE.width),
            (saved.height as f32).max(MIN_SIZE.height),
        )
    })
}

fn restored(size: Size, maximized: bool) -> Option<Size> {
    (!maximized && size.width >= MIN_SIZE.width && size.height >= MIN_SIZE.height).then_some(size)
}

fn dark() -> Theme {
    Theme::custom(
        "Dark",
        Palette {
            background: DARK_BACKGROUND,
            text: DARK_TEXT,
            success: DARK_SUCCESS,
            warning: DARK_WARNING,
            ..Palette::DARK
        },
    )
}

fn is_dark(theme: &Theme) -> bool {
    theme.extended_palette().is_dark
}

fn toggled(theme: &Theme) -> Theme {
    if is_dark(theme) { Theme::Light } else { dark() }
}

fn theme(choice: ThemeChoice) -> Theme {
    match choice {
        ThemeChoice::Light => Theme::Light,
        ThemeChoice::Dark => dark(),
    }
}

fn choice(theme: &Theme) -> ThemeChoice {
    if is_dark(theme) {
        ThemeChoice::Dark
    } else {
        ThemeChoice::Light
    }
}

fn theme_icon(theme: &Theme) -> char {
    if is_dark(theme) { SUN } else { MOON }
}

impl Refreshing {
    fn advance(&mut self, step: Step, now: Instant) {
        match step {
            Step::Catalog(progress) => {
                if let Refreshing::Catalog(tracker) = self {
                    tracker.advance(progress, now);
                }
            }
            Step::Signals => *self = Refreshing::Signals,
            Step::Opening => *self = Refreshing::Opening,
        }
    }

    fn can_cancel(&self) -> bool {
        matches!(self, Refreshing::Catalog(tracker) if tracker.stage() != Stage::Writing)
    }
}

fn refreshing_text(refreshing: &Refreshing) -> &'static str {
    match refreshing {
        Refreshing::Catalog(tracker) => match tracker.stage() {
            Stage::Starting | Stage::Reading => "Reading the list of Steam games…",
            Stage::StoreData { .. } => "Updating prices",
            Stage::Writing => "Writing the catalog…",
        },
        Refreshing::Signals => "Reading your Signals…",
        Refreshing::Opening => "Loading…",
    }
}

fn refreshing_count(refreshing: &Refreshing) -> Option<String> {
    let Refreshing::Catalog(tracker) = refreshing else {
        return None;
    };
    let Stage::StoreData { done, total } = tracker.stage() else {
        return None;
    };
    let count = format!(
        "{} of {}",
        format::count(done as u32),
        format::count(total as u32)
    );

    Some(match tracker.time_left() {
        Some(left) => format!("{count} · {}", format::time_left(left)),
        None => count,
    })
}

fn refresh_tooltip(pending: Option<PendingUpdate>) -> String {
    match pending {
        Some(pending) => format!(
            "Resume the update started on {}",
            format::day_time(pending.started_at)
        ),
        None => "Update prices and Signals".to_string(),
    }
}

fn refresh_failed<'a>(error: &str) -> Element<'a, Message> {
    row![
        tooltip(
            text("Update stopped")
                .size(font::NOTE_SIZE)
                .style(text::danger),
            container(text(error.to_string()).size(font::NOTE_SIZE))
                .padding([6, 10])
                .max_width(420)
                .style(style::popup),
            tooltip::Position::Bottom,
        ),
        button(text(CLOSE).font(font::ICONS).size(9))
            .padding(0)
            .style(style::link)
            .on_press(Message::DismissRefreshError),
    ]
    .spacing(8)
    .align_y(Center)
    .into()
}

fn refresh_button<'a>(refresh: &Refresh, pending: Option<PendingUpdate>) -> Element<'a, Message> {
    let (icon, message, tip) = match refresh {
        Refresh::Running(refreshing, _) if refreshing.can_cancel() => (
            CANCEL,
            Some(Message::CancelRefresh),
            "Pause the update".to_string(),
        ),
        Refresh::Running(..) => (REFRESH, None, refresh_tooltip(pending)),
        Refresh::Idle | Refresh::Failed(_) => {
            (REFRESH, Some(Message::Refresh), refresh_tooltip(pending))
        }
    };

    with_tooltip(bar_icon(icon, message), tip)
}

fn settings_button<'a>() -> Element<'a, Message> {
    with_tooltip(
        bar_icon(GEAR, Some(Message::Navigate(Page::Settings))),
        "Settings".to_string(),
    )
}

fn theme_button<'a>(theme: &Theme) -> Element<'a, Message> {
    bar_icon(theme_icon(theme), Some(Message::ToggleTheme)).into()
}

fn bar_icon<'a>(icon: char, message: Option<Message>) -> button::Button<'a, Message> {
    button(
        container(
            text(icon)
                .font(font::ICONS)
                .size(14)
                .width(Fill)
                .height(Fill)
                .center(),
        )
        .padding(padding::bottom(UNDERLINE_HEIGHT)),
    )
    .width(46)
    .height(Fill)
    .padding(0)
    .style(style::window_button)
    .on_press_maybe(message)
}

fn with_tooltip<'a>(icon: button::Button<'a, Message>, tip: String) -> Element<'a, Message> {
    tooltip(
        icon,
        container(text(tip).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Bottom,
    )
    .into()
}

fn resize_borders<'a>() -> Element<'a, Message> {
    column![
        row![
            resize_border(Direction::NorthWest),
            resize_border(Direction::North),
            resize_border(Direction::NorthEast),
        ],
        row![
            resize_border(Direction::West),
            space().width(Fill),
            resize_border(Direction::East),
        ]
        .height(Fill),
        row![
            resize_border(Direction::SouthWest),
            resize_border(Direction::South),
            resize_border(Direction::SouthEast),
        ],
    ]
    .into()
}

fn resize_border<'a>(direction: Direction) -> Element<'a, Message> {
    let (width, height, interaction): (Length, Length, _) = match direction {
        Direction::North | Direction::South => (
            Fill,
            RESIZE_BORDER.into(),
            mouse::Interaction::ResizingVertically,
        ),
        Direction::East | Direction::West => (
            RESIZE_BORDER.into(),
            Fill,
            mouse::Interaction::ResizingHorizontally,
        ),
        Direction::NorthWest | Direction::SouthEast => (
            RESIZE_BORDER.into(),
            RESIZE_BORDER.into(),
            mouse::Interaction::ResizingDiagonallyDown,
        ),
        Direction::NorthEast | Direction::SouthWest => (
            RESIZE_BORDER.into(),
            RESIZE_BORDER.into(),
            mouse::Interaction::ResizingDiagonallyUp,
        ),
    };

    mouse_area(space().width(width).height(height))
        .interaction(interaction)
        .on_press(Message::Window(WindowAction::Resize(direction)))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(character: &str, modifiers: keyboard::Modifiers) -> keyboard::Event {
        let key = Key::Character(character.into());
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

    fn store_data(done: usize, total: usize) -> Step {
        Step::Catalog(gamepalate_app::update::Progress::StoreData { done, total })
    }

    #[test]
    fn the_bar_says_each_step_of_the_update() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let now = Instant::now();
        assert_eq!(
            refreshing_text(&refreshing),
            "Reading the list of Steam games…"
        );
        assert_eq!(refreshing_count(&refreshing), None);

        refreshing.advance(store_data(312, 757), now);
        assert_eq!(refreshing_text(&refreshing), "Updating prices");
        assert_eq!(refreshing_count(&refreshing).as_deref(), Some("312 of 757"));

        refreshing.advance(Step::Signals, now);
        assert_eq!(refreshing_text(&refreshing), "Reading your Signals…");
        assert_eq!(refreshing_count(&refreshing), None);

        refreshing.advance(store_data(1, 2), now);
        refreshing.advance(Step::Opening, now);
        assert_eq!(refreshing_text(&refreshing), "Loading…");
    }

    #[test]
    fn after_ten_requests_the_bar_says_the_time_left() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let start = Instant::now();
        for done in 1..=10 {
            refreshing.advance(
                store_data(done, 1_210),
                start + Duration::from_secs(done as u64),
            );
        }

        assert_eq!(
            refreshing_count(&refreshing).as_deref(),
            Some("10 of 1,210 · About 20 minutes left")
        );
    }

    #[test]
    fn the_update_can_be_stopped_until_it_writes() {
        let mut refreshing = Refreshing::Catalog(Tracker::default());
        let now = Instant::now();
        assert!(refreshing.can_cancel());

        refreshing.advance(store_data(312, 757), now);
        assert!(refreshing.can_cancel());

        refreshing.advance(
            Step::Catalog(gamepalate_app::update::Progress::Writing),
            now,
        );
        assert!(!refreshing.can_cancel());

        refreshing.advance(Step::Signals, now);
        assert!(!refreshing.can_cancel());
    }

    #[test]
    fn the_tooltip_offers_to_resume_a_pending_update() {
        let started_at = Timestamp(1_793_369_460_000);

        assert_eq!(refresh_tooltip(None), "Update prices and Signals");
        assert_eq!(
            refresh_tooltip(Some(PendingUpdate {
                started_at,
                responses: 3,
            })),
            format!(
                "Resume the update started on {}",
                format::day_time(started_at)
            )
        );
    }

    #[test]
    fn prices_of_another_region_say_their_region() {
        assert_eq!(prices_text("28 Oct".to_string(), None), "Prices 28 Oct");
        assert_eq!(
            prices_text("28 Oct".to_string(), Some(Region::ITALY)),
            "Prices 28 Oct · IT"
        );
    }

    #[test]
    fn ctrl_and_a_number_open_a_tab() {
        let page = |character, modifiers| match page_key(pressed(character, modifiers)) {
            Some(Message::Navigate(page)) => Some(page),
            _ => None,
        };

        assert_eq!(page("1", keyboard::Modifiers::CTRL), Some(Page::Discover));
        assert_eq!(page("2", keyboard::Modifiers::CTRL), Some(Page::Votes));
        assert_eq!(page("3", keyboard::Modifiers::CTRL), Some(Page::Library));
        assert_eq!(page("4", keyboard::Modifiers::CTRL), Some(Page::Profile));
        assert_eq!(page("3", keyboard::Modifiers::empty()), None);
        assert_eq!(page("5", keyboard::Modifiers::CTRL), None);
    }

    #[test]
    fn the_window_opens_as_it_was_saved() {
        let saved = |width, height, maximized| {
            Some(SavedWindow {
                width,
                height,
                maximized,
            })
        };

        let first = window_settings(None);
        assert_eq!(first.size, DEFAULT_SIZE);
        assert!(!first.maximized);
        assert!(matches!(first.position, window::Position::Centered));
        assert!(!first.exit_on_close_request);

        let large = window_settings(saved(1440, 900, true));
        assert_eq!(large.size, Size::new(1440.0, 900.0));
        assert!(large.maximized);

        assert_eq!(window_settings(saved(400, 300, false)).size, MIN_SIZE);
    }

    #[test]
    fn keeps_the_size_of_the_window_when_it_is_not_maximized_or_minimized() {
        let size = Size::new(1440.0, 900.0);

        assert_eq!(restored(size, false), Some(size));
        assert_eq!(restored(Size::new(2560.0, 1392.0), true), None);
        assert_eq!(restored(Size::ZERO, false), None);
    }

    #[test]
    fn the_theme_goes_from_light_to_dark_and_back() {
        assert_eq!(toggled(&Theme::Light), dark());
        assert_eq!(toggled(&dark()), Theme::Light);
    }

    #[test]
    fn the_dark_theme_has_a_darker_background_than_the_default() {
        let background = dark().palette().background;

        assert_eq!(background, DARK_BACKGROUND);
        assert!(background.relative_luminance() < Palette::DARK.background.relative_luminance());
    }

    #[test]
    fn links_and_discounts_stand_out_in_the_dark_as_in_the_light() {
        let contrast = |theme: &Theme| {
            let background = theme.palette().background;
            let link = style::link(theme, button::Status::Active).text_color;
            let discount = style::discount(theme).color.unwrap();

            (
                link.relative_contrast(background),
                discount.relative_contrast(background),
            )
        };
        let (light_link, light_discount) = contrast(&Theme::Light);
        let (dark_link, dark_discount) = contrast(&dark());

        assert!(dark_link >= light_link);
        assert!(dark_discount >= light_discount);
    }

    #[test]
    fn the_saved_choice_is_the_theme() {
        for choice_saved in [ThemeChoice::Light, ThemeChoice::Dark] {
            assert_eq!(choice(&theme(choice_saved)), choice_saved);
        }
    }

    #[test]
    fn the_button_shows_the_theme_it_switches_to() {
        assert_eq!(theme_icon(&Theme::Light), MOON);
        assert_eq!(theme_icon(&dark()), SUN);
    }
}
