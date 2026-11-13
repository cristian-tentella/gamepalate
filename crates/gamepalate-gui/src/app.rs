mod backdrop;
mod frame;
mod game_page;
mod install;
mod refresh;
mod search;
mod theme;

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gamepalate_app::{
    AppId, Gamepalate, GuiSettings, PendingUpdate, SavedWindow,
    install::{Downloaded, Installed, Target},
    new_version::NewVersion,
    refresh::{Plan, Step},
    setup::{FirstRun, error_chain},
    update::ArchiveState,
};
use iced::{
    Center, Element, Fill, Right, Shrink, Size, Subscription, Task, Theme, Top,
    keyboard::{self, Key},
    padding,
    widget::{button, column, container, image, mouse_area, row, space, stack, text, tooltip},
    window,
};

use crate::{
    font, format,
    new_version::{NewVersions, Problem},
    screen::{
        Discover, Library, Profile, Screen, Settings, Votes, discover, library, profile, settings,
        setup, votes,
    },
    smooth::smooth,
    style,
};
pub use frame::window_settings;
use frame::{
    CLOSE, MAXIMIZE, MINIMIZE, RESTORE, WindowAction, rescaled, resize_borders, saved_size,
    window_button,
};
use game_page::GamePage;
use refresh::{Refresh, data_times, refresh_button, refresh_dot, refresh_tooltip};
use search::Search;
use theme::{choice, light, theme, theme_button, toggled};

const UNDERLINE_HEIGHT: f32 = 2.0;
const BAR_HEIGHT: f32 = 44.0;
/// The top bar and the line under it: the game search leaves them sharp.
const SEARCH_TOP: f32 = BAR_HEIGHT + 1.0;
const LOGO_BYTES: &[u8] = include_bytes!("../gamepalate-24.png");
const LOGO_SIZE: f32 = 24.0;
const LOGO_SPACING: f32 = 8.0;
const BAR_ICON_SCALE: f32 = 0.85;
const GEAR: char = '\u{E8B8}';
const DOT_SIZE: f32 = 7.0;
const NOTICE_DURATION: Duration = Duration::from_secs(12);

pub struct App {
    gamepalate: Option<Arc<Gamepalate>>,
    screen: Screen,
    theme: Theme,
    window: Option<window::Id>,
    maximized: bool,
    size: Size,
    resized: Size,
    scale: f32,
    last_bar_press: Option<Instant>,
    refresh: Refresh,
    pending: Option<PendingUpdate>,
    renewal_due: bool,
    startup_error: Option<String>,
    new_versions: NewVersions,
    downloaded: Option<Downloaded>,
    installed: Option<Installed>,
    logo: image::Handle,
    families_notice: Option<usize>,
    search: Search,
    game_page: Option<Box<GamePage>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Start {
    Startup,
    Button,
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
    Search(search::Message),
    GamePage(game_page::Message),
    WindowFound(Option<window::Id>),
    Window(WindowAction),
    BarPressed,
    Resized(Size),
    Rescaled(f32),
    Maximized(bool),
    ToggleTheme,
    Refresh,
    ArchiveRead(Start, Result<ArchiveState, String>),
    Refreshing(Step),
    Entered(Arc<Gamepalate>),
    Refreshed(Start, Result<Option<Arc<Gamepalate>>, String>),
    CancelRefresh,
    DismissRefreshError,
    HideFamiliesNotice,
    OpenProfileFromNotice,
    Reopened(Result<Arc<Gamepalate>, String>),
    NewVersionChecked(Result<Option<NewVersion>, String>),
    NewVersionDownloading(Option<u8>),
    NewVersionDownloaded(Result<Arc<Mutex<Option<Downloaded>>>, Problem>),
    InstallNewVersion,
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
    pub fn new(saved: Option<SavedWindow>, check_for_new_versions: bool) -> (App, Task<Message>) {
        let target = Target::current();
        let mut app = App {
            gamepalate: None,
            screen: Screen::Starting,
            theme: light(),
            window: None,
            maximized: saved.is_some_and(|saved| saved.maximized),
            size: saved_size(saved),
            resized: saved_size(saved),
            scale: 1.0,
            last_bar_press: None,
            refresh: Refresh::Idle,
            pending: None,
            renewal_due: false,
            startup_error: None,
            new_versions: NewVersions::new(target, check_for_new_versions),
            downloaded: None,
            installed: None,
            logo: image::Handle::from_bytes(LOGO_BYTES),
            families_notice: None,
            search: Search::default(),
            game_page: None,
        };
        let task = match setup::start(FirstRun::check(None)) {
            Ok(gamepalate) => {
                let opened = app.opened(gamepalate);
                Task::batch([opened, app.read_archive(Start::Startup)])
            }
            Err(mut setup) => {
                app.theme = theme(setup.first_run().gui_settings().theme);
                let task = setup.begin().map(Message::Setup);
                app.screen = Screen::Setup(setup);
                task
            }
        };

        let clean = match app.new_versions.target() {
            Some(target) => {
                let target = target.clone();
                Task::future(async move { target.clean_up() }).discard()
            }
            None => Task::none(),
        };
        let check = app.check_new_version(Start::Startup);

        (
            app,
            Task::batch([
                task,
                window::latest().map(Message::WindowFound),
                clean,
                check,
            ]),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(Page::Settings) if self.page() == Some(Page::Settings) => {
                let back = self.back_page();
                self.leave(back)
            }
            Message::Navigate(page) if self.page() == Some(page) => {
                self.search.close();
                let changed = self.game_page.take().is_some_and(|page| page.changed());
                self.close_game_page(changed)
            }
            Message::Navigate(page) => self.leave(page),
            Message::Discover(discover::Message::OpenSimilar(id)) => self.show_game(id),
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
            Message::Profile(profile::Message::OpenLibrary(filter, sort)) => {
                let Some(gamepalate) = &self.gamepalate else {
                    return Task::none();
                };
                let (library, task) = Library::filtered(gamepalate, filter, sort);
                self.screen = Screen::Library(Box::new(library));
                task.map(Message::Library)
            }
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
                    return if message.changed_data() {
                        self.reopen()
                    } else {
                        Task::none()
                    };
                };
                match screen.update(message) {
                    settings::Action::None => Task::none(),
                    settings::Action::Run(task) => task.map(Message::Settings),
                    settings::Action::Theme(choice) => self.set_theme(choice),
                    settings::Action::Reopen => self.reopen(),
                    settings::Action::RegionChanged => Task::batch([
                        self.reopen(),
                        self.run_refresh(Plan::EVERYTHING, Start::Button),
                    ]),
                    settings::Action::Close(page, saving) => {
                        Task::batch([self.open(page), saving.map(Message::Settings)])
                    }
                    settings::Action::CheckNewVersion => self.check_new_version(Start::Button),
                    settings::Action::InstallNewVersion => self.download_new_version(),
                    settings::Action::CheckAtStartup(check) => {
                        self.new_versions.set_check_at_startup(check);
                        self.change_gui_settings(move |settings| {
                            settings.check_for_new_versions = check;
                        })
                    }
                }
            }
            Message::Reopened(Ok(gamepalate)) => {
                if let Screen::Settings(screen) = &mut self.screen {
                    screen.reopened(&gamepalate);
                }
                let placed = self.game_page_uses(&gamepalate);
                self.gamepalate = Some(gamepalate);
                self.find_pending();
                if matches!(self.screen, Screen::Settings(_)) {
                    return placed;
                }
                match self.page() {
                    Some(page) => Task::batch([self.open(page), placed]),
                    None => placed,
                }
            }
            Message::Reopened(Err(error)) => {
                if let Screen::Settings(screen) = &mut self.screen {
                    screen.failed(error);
                }
                Task::none()
            }
            Message::Search(message) => {
                let Some(gamepalate) = &self.gamepalate else {
                    return Task::none();
                };
                match self
                    .search
                    .update(message, gamepalate, self.window, SEARCH_TOP)
                {
                    search::Action::Run(task) => task.map(Message::Search),
                    search::Action::Open(id) => {
                        let saving = self.save_settings();
                        Task::batch([self.open_game(id), saving])
                    }
                }
            }
            Message::GamePage(message) => {
                let Some(page) = &mut self.game_page else {
                    return Task::none();
                };
                match page.update(message) {
                    game_page::Action::Run(task) => task.map(Message::GamePage),
                    game_page::Action::Close { changed } => self.close_game_page(changed),
                }
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
            Message::WindowFound(_)
            | Message::Rescaled(_)
            | Message::Window(_)
            | Message::BarPressed
            | Message::Resized(_)
            | Message::Maximized(_) => self.update_frame(message),
            Message::Refresh
            | Message::ArchiveRead(..)
            | Message::Refreshing(_)
            | Message::Entered(_)
            | Message::Refreshed(..)
            | Message::CancelRefresh
            | Message::DismissRefreshError => self.update_refresh(message),
            Message::HideFamiliesNotice => {
                self.families_notice = None;
                Task::none()
            }
            Message::OpenProfileFromNotice => {
                self.families_notice = None;
                self.update(Message::Navigate(Page::Profile))
            }
            Message::ToggleTheme => self.set_theme(choice(&toggled(&self.theme))),
            Message::NewVersionChecked(_)
            | Message::NewVersionDownloading(_)
            | Message::NewVersionDownloaded(_)
            | Message::InstallNewVersion => self.update_install(message),
        }
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
                let opened = tokio::task::spawn_blocking(move || Gamepalate::open(Some(dir))).await;
                match opened {
                    Ok(opened) => opened.map(Arc::new).map_err(|error| {
                        format!(
                            "gamepalate could not open the data files: {}",
                            error_chain(&error)
                        )
                    }),
                    Err(error) => Err(format!("gamepalate could not open the data files: {error}")),
                }
            },
            Message::Reopened,
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let search_open = self.search.is_open();
        let keys = !search_open && self.game_page.is_none();
        let screen = match &self.screen {
            Screen::Discover(_) => Discover::subscription(keys).map(Message::Discover),
            Screen::Votes(_) => Votes::subscription(keys).map(Message::Votes),
            Screen::Library(_) => Library::subscription(keys).map(Message::Library),
            Screen::Profile(_) => Profile::subscription(keys).map(Message::Profile),
            Screen::Settings(screen) => screen.subscription(keys).map(Message::Settings),
            Screen::Setup(_) | Screen::Starting => Subscription::none(),
        };
        let search = self.search.subscription().map(Message::Search);
        let game_page = if self.game_page.is_some() {
            GamePage::subscription(!search_open).map(Message::GamePage)
        } else {
            Subscription::none()
        };

        Subscription::batch([
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            window::events().filter_map(rescaled),
            window::close_requests().map(|_| Message::Window(WindowAction::Close)),
            keyboard::listen().filter_map(page_key),
            screen,
            search,
            game_page,
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        let bar = container(
            row![
                column![
                    container(
                        row![
                            image(self.logo.clone())
                                .width(LOGO_SIZE)
                                .height(LOGO_SIZE)
                                .filter_method(image::FilterMethod::Nearest),
                            text("gamepalate").font(font::BOLD).size(font::BAR_SIZE),
                        ]
                        .spacing(LOGO_SPACING)
                        .align_y(Center)
                    )
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
            .push(
                container(self.gamepalate.as_deref().map(|_| {
                    column![
                        container(self.status()).height(Fill).align_y(Center),
                        space().height(UNDERLINE_HEIGHT),
                    ]
                    .height(Fill)
                }))
                .width(Fill)
                .height(Fill)
                .align_x(Right)
                .clip(true),
            )
            .push(
                row![]
                    .push(
                        self.gamepalate
                            .is_some()
                            .then(|| search_button(self.search.is_open())),
                    )
                    .push(self.gamepalate.as_deref().map(|gamepalate| {
                        refresh_button(
                            &self.refresh,
                            refresh_tooltip(
                                self.pending,
                                self.renewal_due,
                                self.startup_error.as_deref(),
                            ),
                            data_times(gamepalate),
                            refresh_dot(self.pending, self.startup_error.as_deref()),
                        )
                    }))
                    .push(self.gamepalate.is_some().then(|| {
                        settings_button(
                            self.page() == Some(Page::Settings),
                            self.new_versions.available().is_some(),
                        )
                    }))
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
        .height(BAR_HEIGHT)
        .style(style::top_bar);
        let bar = mouse_area(bar).on_press(Message::BarPressed);

        let content = match &self.screen {
            Screen::Discover(screen) => {
                let notice = self.families_notice.map(families_notice);
                stack![screen.view(self.scale).map(Message::Discover)]
                    .push(notice)
                    .into()
            }
            Screen::Votes(screen) => screen.view(self.scale).map(Message::Votes),
            Screen::Library(screen) => screen.view().map(Message::Library),
            Screen::Setup(screen) => screen.view().map(Message::Setup),
            Screen::Profile(screen) => screen.view().map(Message::Profile),
            Screen::Settings(screen) => screen
                .view(choice(&self.theme), &self.new_versions)
                .map(Message::Settings),
            Screen::Starting => column![].into(),
        };
        let content = stack![content]
            .push(
                self.game_page
                    .as_ref()
                    .map(|page| page.view(self.scale).map(Message::GamePage)),
            )
            .push(
                self.search
                    .view(Size::new(
                        self.resized.width,
                        self.resized.height - SEARCH_TOP,
                    ))
                    .map(|search| search.map(Message::Search)),
            );

        let app = column![
            bar,
            style::separator(),
            smooth(container(content).width(Fill).height(Fill)),
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

    fn opened(&mut self, gamepalate: Gamepalate) -> Task<Message> {
        let gamepalate = Arc::new(gamepalate);
        let pruner = Arc::clone(&gamepalate);
        let prune_images = Task::future(async move { pruner.prune_images() }).discard();
        self.theme = theme(gamepalate.gui_settings().theme);
        self.gamepalate = Some(gamepalate);
        self.find_pending();

        let notice = match self
            .gamepalate
            .as_deref()
            .and_then(|gamepalate| gamepalate.created_families())
        {
            Some(created) => {
                let (count, played) = (created.families.families.len(), created.played.clone());
                self.families_notice = Some(count);
                Task::batch([
                    self.change_gui_settings(move |settings| {
                        settings.families_played = Some(played)
                    }),
                    Task::perform(tokio::time::sleep(NOTICE_DURATION), |_| {
                        Message::HideFamiliesNotice
                    }),
                ])
            }
            None => Task::none(),
        };

        Task::batch([self.open(Page::Discover), prune_images, notice])
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

    fn leave(&mut self, page: Page) -> Task<Message> {
        self.search.close();
        self.game_page = None;
        let saving = self.save_settings();

        Task::batch([self.open(page), saving])
    }

    fn save_settings(&mut self) -> Task<Message> {
        match &mut self.screen {
            Screen::Settings(screen) => screen.leave().map(Message::Settings),
            _ => Task::none(),
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

    /// A game found by name opens over the page, which keeps its state. A game out of the catalog
    /// has no page of its own: it opens where it is.
    fn open_game(&mut self, id: AppId) -> Task<Message> {
        let Some(gamepalate) = &self.gamepalate else {
            return Task::none();
        };
        let Some((page, task)) = GamePage::new(gamepalate, id) else {
            return self.show_game(id);
        };
        let changed = self.game_page.take().is_some_and(|page| page.changed());
        let refreshed = self.close_game_page(changed);
        self.game_page = Some(Box::new(page));

        Task::batch([refreshed, task.map(Message::GamePage)])
    }

    /// The page of a game saves on the new facade, and finds its place in Discover again.
    fn game_page_uses(&mut self, gamepalate: &Arc<Gamepalate>) -> Task<Message> {
        match &mut self.game_page {
            Some(page) => page.use_gamepalate(gamepalate).map(Message::GamePage),
            None => Task::none(),
        }
    }

    /// The lists of Votes, Library and Profile are made when they open: after a change they open
    /// again. Discover keeps its queue.
    fn close_game_page(&mut self, changed: bool) -> Task<Message> {
        self.game_page = None;
        match self.page() {
            Some(page @ (Page::Votes | Page::Library | Page::Profile)) if changed => {
                self.open(page)
            }
            _ => Task::none(),
        }
    }

    fn show_game(&mut self, id: AppId) -> Task<Message> {
        let Some(gamepalate) = &self.gamepalate else {
            return Task::none();
        };
        let (library, task) = Library::showing(gamepalate, id);
        if library.contains(id) {
            self.screen = Screen::Library(Box::new(library));
            return task.map(Message::Library);
        }
        let (votes, task) = Votes::showing(gamepalate, id);
        self.screen = Screen::Votes(Box::new(votes));
        task.map(Message::Votes)
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
        .on_press_maybe((!current || self.game_page.is_some()).then_some(Message::Navigate(page)))
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
        Key::Character("k" | "K") => Some(Message::Search(search::Message::Toggle)),
        Key::Character("1") => Some(Message::Navigate(Page::Discover)),
        Key::Character("2") => Some(Message::Navigate(Page::Votes)),
        Key::Character("3") => Some(Message::Navigate(Page::Library)),
        Key::Character("4") => Some(Message::Navigate(Page::Profile)),
        _ => None,
    }
}

fn families_notice<'a>(count: usize) -> Element<'a, Message> {
    let close = button(
        text(CLOSE)
            .font(font::ICONS)
            .size(font::icon(9.0) * BAR_ICON_SCALE),
    )
    .padding(0)
    .style(style::toast_link)
    .on_press(Message::HideFamiliesNotice);

    container(
        container(
            row![
                text(format::families_made(count)),
                button(
                    text("Open Profile")
                        .size(font::NOTE_SIZE)
                        .font(font::SEMIBOLD)
                )
                .padding(0)
                .style(style::toast_link)
                .on_press(Message::OpenProfileFromNotice),
                close,
            ]
            .spacing(14)
            .align_y(Center),
        )
        .padding([8, 14])
        .style(style::toast),
    )
    .center_x(Fill)
    .align_bottom(Fill)
    .padding(16)
    .into()
}

fn search_button<'a>(open: bool) -> Element<'a, Message> {
    let button = bar_icon(search::LENS, Some(Message::Search(search::Message::Toggle)));
    let button = if open {
        button.style(style::open_bar_icon)
    } else {
        button
    };
    with_tooltip(button.into(), search::TIP.to_string())
}

fn settings_button<'a>(open: bool, new_version: bool) -> Element<'a, Message> {
    let button = bar_icon(GEAR, Some(Message::Navigate(Page::Settings)));
    let button = if open {
        button.style(style::open_bar_icon)
    } else {
        button
    };
    with_tooltip(
        with_dot(button, new_version),
        settings_tip(open, new_version).to_string(),
    )
}

fn with_dot<'a>(button: button::Button<'a, Message>, dot: bool) -> Element<'a, Message> {
    if !dot {
        return button.into();
    }
    stack![
        button,
        container(
            container(space())
                .width(DOT_SIZE)
                .height(DOT_SIZE)
                .style(style::badge)
        )
        .width(Fill)
        .height(Fill)
        .align_x(Right)
        .align_y(Top)
        .padding(padding::top(11).right(12)),
    ]
    .into()
}

fn settings_tip(open: bool, new_version: bool) -> &'static str {
    match (open, new_version) {
        (true, _) => "Close Settings",
        (false, true) => "Settings · New version available",
        (false, false) => "Settings",
    }
}

fn bar_icon<'a>(icon: char, message: Option<Message>) -> button::Button<'a, Message> {
    button(
        container(
            text(icon)
                .font(font::ICONS)
                .size(font::icon(14.0) * BAR_ICON_SCALE)
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

fn with_tooltip<'a>(icon: Element<'a, Message>, tip: String) -> Element<'a, Message> {
    tooltip(
        icon,
        container(text(tip).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Bottom,
    )
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

    #[test]
    fn the_gear_offers_to_close_settings_while_they_are_open() {
        assert_eq!(settings_tip(false, false), "Settings");
        assert_eq!(settings_tip(true, false), "Close Settings");
    }

    #[test]
    fn the_gear_says_when_a_new_version_is_available() {
        assert_eq!(
            settings_tip(false, true),
            "Settings · New version available"
        );
        assert_eq!(settings_tip(true, true), "Close Settings");
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
    fn ctrl_k_goes_to_the_game_search() {
        assert!(matches!(
            page_key(pressed("k", keyboard::Modifiers::CTRL)),
            Some(Message::Search(search::Message::Toggle))
        ));
        assert!(matches!(
            page_key(pressed(
                "K",
                keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT
            )),
            Some(Message::Search(search::Message::Toggle))
        ));
        assert!(page_key(pressed("k", keyboard::Modifiers::empty())).is_none());
    }
}
