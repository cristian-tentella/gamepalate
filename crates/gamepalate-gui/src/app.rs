use std::sync::Arc;

use gamepalate_app::Gamepalate;
use iced::{Element, Task};

use crate::screen::{Recommendations, Screen, recommendations};

pub struct App {
    gamepalate: Arc<Gamepalate>,
    screen: Screen,
}

#[derive(Clone, Debug)]
pub enum Message {
    Recommendations(recommendations::Message),
}

impl App {
    pub fn new(gamepalate: Arc<Gamepalate>) -> (App, Task<Message>) {
        let (recommendations, task) = Recommendations::new(&gamepalate);

        (
            App {
                gamepalate,
                screen: Screen::Recommendations(recommendations),
            },
            task.map(Message::Recommendations),
        )
    }

    pub fn update(&mut self, message: Message) {
        match (message, &mut self.screen) {
            (Message::Recommendations(message), Screen::Recommendations(screen)) => {
                screen.update(message);
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Recommendations(screen) => screen.view().map(Message::Recommendations),
        }
    }
}
