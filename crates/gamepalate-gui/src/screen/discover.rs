use std::sync::Arc;

use gamepalate_app::{Gamepalate, RecommendError, Recommendation, RecommendedGame, SearchOptions};
use iced::{
    Element, Fill, Task,
    widget::{button, column, container, row, text},
};

use crate::style;

pub struct Discover {
    state: State,
}

#[derive(Clone, Debug)]
pub enum Message {
    Recommended(Result<Recommendation, RecommendError>),
    Previous,
    Next,
}

enum State {
    Loading,
    Ready {
        games: Vec<RecommendedGame>,
        index: usize,
    },
    Failed(String),
}

impl Discover {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Discover, Task<Message>) {
        let gamepalate = Arc::clone(gamepalate);
        let task = Task::perform(
            async move { gamepalate.recommend(&SearchOptions::default()) },
            Message::Recommended,
        );

        (
            Discover {
                state: State::Loading,
            },
            task,
        )
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Recommended(Ok(recommendation)) => {
                self.state = State::Ready {
                    games: recommendation.games,
                    index: 0,
                };
            }
            Message::Recommended(Err(error)) => {
                self.state = State::Failed(error.to_string());
            }
            Message::Previous => {
                if let State::Ready { index, .. } = &mut self.state {
                    *index = index.saturating_sub(1);
                }
            }
            Message::Next => {
                if let State::Ready { games, index } = &mut self.state
                    && *index + 1 < games.len()
                {
                    *index += 1;
                }
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let filters = container(text("Filters"))
            .padding([6, 12])
            .width(Fill)
            .style(style::bar);
        let main = container(self.game())
            .padding(16)
            .width(Fill)
            .height(Fill)
            .clip(true);
        let votes = container(text("Votes"))
            .padding([10, 12])
            .width(Fill)
            .style(style::bar);

        column![filters, style::separator(), main, style::separator(), votes].into()
    }

    fn game(&self) -> Element<'_, Message> {
        match &self.state {
            State::Loading => text("Loading…").into(),
            State::Failed(error) => text!("Error: {error}").into(),
            State::Ready { games, index } => {
                let game = &games[*index];
                let position = game.position;
                let name = &game.name;

                column![
                    text!("{position}. {name}"),
                    row![
                        button("<--").on_press_maybe((*index > 0).then_some(Message::Previous)),
                        button("-->")
                            .on_press_maybe((*index + 1 < games.len()).then_some(Message::Next)),
                    ],
                ]
                .into()
            }
        }
    }
}
