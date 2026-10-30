use std::sync::Arc;

use gamepalate_app::Gamepalate;
use iced::{
    Element, Fill, Task, Theme,
    widget::{button, column, container, rich_text, row, scrollable, span, text, text::Rich},
};

use crate::{
    font,
    screen::{Discover, Screen, discover},
};

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
                text("gamepalate").font(font::BOLD),
                row![
                    self.tab("Discover", Page::Discover),
                    self.tab("Votes", Page::Votes),
                    self.tab("Profile", Page::Profile),
                ]
                .spacing(12),
            ]
            .spacing(20),
        )
        .padding([8, 12])
        .width(Fill)
        .style(bar_style);

        let content = match &self.screen {
            Screen::Discover(screen) => screen.view().map(Message::Discover),
            Screen::Votes | Screen::Profile => column![].into(),
        };

        column![bar, scrollable(content).width(Fill).height(Fill)].into()
    }

    pub fn theme(&self) -> Theme {
        self.theme.clone()
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
        let content: Rich<'_, (), Message> = rich_text([span(label).underline(current)]);

        button(content)
            .padding(0)
            .style(move |theme, status| {
                if current {
                    button::text(theme, button::Status::Active)
                } else {
                    button::text(theme, status)
                }
            })
            .on_press_maybe((!current).then_some(Message::Navigate(page)))
            .into()
    }
}

fn bar_style(theme: &Theme) -> container::Style {
    let style = container::bordered_box(theme);

    container::Style {
        border: style.border.rounded(0),
        ..style
    }
}
