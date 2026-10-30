use std::sync::Arc;

use gamepalate_app::{Gamepalate, Timestamp};
use iced::{
    Center, Element, Fill, Shrink, Task, Theme,
    widget::{button, column, container, row, space, text},
};

use crate::{
    font, format,
    screen::{Discover, Screen, discover},
    style,
};

const UNDERLINE_HEIGHT: f32 = 2.0;

pub struct App {
    gamepalate: Arc<Gamepalate>,
    screen: Screen,
    theme: Theme,
}

#[derive(Clone, Debug)]
pub enum Message {
    Navigate(Page),
    Discover(discover::Message),
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

        (
            App {
                gamepalate,
                screen: Screen::Discover(discover),
                theme: Theme::Light,
            },
            task.map(Message::Discover),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(page) => self.open(page),
            Message::Discover(message) => {
                if let Screen::Discover(screen) = &mut self.screen {
                    screen.update(message);
                }
                Task::none()
            }
        }
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
            ]
            .spacing(24)
            .height(Fill)
            .align_y(Center),
        )
        .padding([0, 20])
        .width(Fill)
        .height(44)
        .style(style::top_bar);

        let content = match &self.screen {
            Screen::Discover(screen) => screen.view().map(Message::Discover),
            Screen::Votes | Screen::Profile => column![].into(),
        };

        column![
            bar,
            style::separator(),
            container(content).width(Fill).height(Fill),
        ]
        .into()
    }

    pub fn theme(&self) -> Theme {
        self.theme.clone()
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
                self.screen = Screen::Discover(discover);
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
