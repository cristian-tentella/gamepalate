use std::sync::Arc;

use gamepalate_app::Gamepalate;
use iced::{
    Element, Fill, Task, widget::{button, column, container, row, scrollable, text},
};

use crate::screen::{Discover, Screen, discover};

pub struct App {
    gamepalate: Arc<Gamepalate>,
    screen: Screen,
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
            },
            task.map(Message::Discover),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(page) => self.open(page),
            Message::Discover(message) => {
                match &mut self.screen {
                    Screen::Discover(screen) => screen.update(message),
                }
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let bar = container(
            row![
                text("gamepalate"),
                self.tab("Discover", Page::Discover),
                self.tab("Votes", Page::Votes),
                self.tab("Profile", Page::Profile),
            ]
            .spacing(8),
        )
        .padding(10)
        .width(Fill)
        .style(container::bordered_box);

        let content = match &self.screen {
            Screen::Discover(screen) => screen.view().map(Message::Discover),
        };

        column![bar, scrollable(content).width(Fill).height(Fill)].into()
    }

    fn page(&self) -> Page {
        match &self.screen {
            Screen::Discover(_) => Page::Discover,
        }
    }

    fn open(&mut self, page: Page) -> Task<Message> {
        match page {
            Page::Discover => {
                let (discover, task) = Discover::new(&self.gamepalate);
                self.screen = Screen::Discover(discover);
                task.map(Message::Discover)
            }
            Page::Votes | Page::Profile => Task::none(),
        }
    }

    fn tab(&self, label: &'static str, page: Page) -> Element<'_, Message> {
        let current = self.page() == page;
        let available = matches!(page, Page::Discover);
        let style = if current {
            button::primary
        } else {
            button::secondary
        };

        button(label)
            .style(style)
            .on_press_maybe((available && !current).then_some(Message::Navigate(page)))
            .into()
    }
}
