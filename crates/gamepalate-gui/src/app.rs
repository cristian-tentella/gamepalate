use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{Gamepalate, Timestamp};
use iced::{
    Center, Element, Fill, Length, Shrink, Subscription, Task, Theme, mouse, padding,
    widget::{button, column, container, mouse_area, row, space, stack, text},
    window::{self, Direction},
};

use crate::{
    font, format,
    screen::{Discover, Screen, discover},
    style,
};

const UNDERLINE_HEIGHT: f32 = 2.0;
const RESIZE_BORDER: f32 = 5.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
const MINIMIZE: char = '\u{E921}';
const MAXIMIZE: char = '\u{E922}';
const RESTORE: char = '\u{E923}';
const CLOSE: char = '\u{E8BB}';

pub struct App {
    gamepalate: Arc<Gamepalate>,
    screen: Screen,
    theme: Theme,
    window: Option<window::Id>,
    maximized: bool,
    last_bar_press: Option<Instant>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Navigate(Page),
    Discover(discover::Message),
    WindowFound(Option<window::Id>),
    Window(WindowAction),
    BarPressed,
    Resized,
    Maximized(bool),
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
    Profile,
}

impl App {
    pub fn new(gamepalate: Arc<Gamepalate>) -> (App, Task<Message>) {
        let (discover, task) = Discover::new(&gamepalate);
        let pruner = Arc::clone(&gamepalate);
        let prune_images = Task::future(async move { pruner.prune_images() }).discard();

        (
            App {
                gamepalate,
                screen: Screen::Discover(Box::new(discover)),
                theme: Theme::Light,
                window: None,
                maximized: false,
                last_bar_press: None,
            },
            Task::batch([
                task.map(Message::Discover),
                window::latest().map(Message::WindowFound),
                prune_images,
            ]),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(page) => self.open(page),
            Message::Discover(message) => {
                if let Screen::Discover(screen) = &mut self.screen {
                    screen.update(message).map(Message::Discover)
                } else {
                    Task::none()
                }
            }
            Message::WindowFound(id) => {
                self.window = id;
                Task::none()
            }
            Message::Window(action) => self.window_action(action),
            Message::BarPressed => self.bar_pressed(),
            Message::Resized => match self.window {
                Some(id) => window::is_maximized(id).map(Message::Maximized),
                None => Task::none(),
            },
            Message::Maximized(maximized) => {
                self.maximized = maximized;
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let screen = match &self.screen {
            Screen::Discover(_) => Discover::subscription().map(Message::Discover),
            Screen::Votes | Screen::Profile => Subscription::none(),
        };

        Subscription::batch([window::resize_events().map(|_| Message::Resized), screen])
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
                row![
                    self.tab("Discover", Page::Discover),
                    self.tab("Votes", Page::Votes),
                    self.tab("Profile", Page::Profile),
                ]
                .spacing(24)
                .height(Fill),
                space().width(Fill),
                column![
                    container(self.freshness()).height(Fill).align_y(Center),
                    space().height(UNDERLINE_HEIGHT),
                ]
                .height(Fill),
                row![
                    window_button(MINIMIZE, WindowAction::Minimize),
                    window_button(
                        if self.maximized { RESTORE } else { MAXIMIZE },
                        WindowAction::ToggleMaximize
                    ),
                    window_button(CLOSE, WindowAction::Close),
                ]
                .height(Fill),
            ]
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
            Screen::Votes | Screen::Profile => column![].into(),
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
            return iced::exit();
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

    fn freshness(&self) -> Element<'_, Message> {
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());
        let signals = format::when(self.gamepalate.signals_read_at(), now);
        let prices = format::when(self.gamepalate.prices_updated_at(), now);

        text!("Signals {signals} · Prices {prices}")
            .size(font::NOTE_SIZE)
            .style(style::muted)
            .into()
    }

    fn page(&self) -> Page {
        match &self.screen {
            Screen::Discover(_) => Page::Discover,
            Screen::Votes => Page::Votes,
            Screen::Profile => Page::Profile,
        }
    }

    fn open(&mut self, page: Page) -> Task<Message> {
        match page {
            Page::Discover => {
                let (discover, task) = Discover::new(&self.gamepalate);
                self.screen = Screen::Discover(Box::new(discover));
                task.map(Message::Discover)
            }
            Page::Votes => {
                self.screen = Screen::Votes;
                Task::none()
            }
            Page::Profile => {
                self.screen = Screen::Profile;
                Task::none()
            }
        }
    }

    fn tab(&self, label: &'static str, page: Page) -> Element<'_, Message> {
        let current = self.page() == page;
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
