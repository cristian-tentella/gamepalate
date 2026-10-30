use std::sync::Arc;

use gamepalate_app::{Gamepalate, RecommendError, Recommendation, RecommendedGame, SearchOptions};
use iced::{
    Center, Element, Fill, Task,
    widget::{button, column, container, row, space, text},
};

use crate::{font, format, style};

pub struct Discover {
    options: SearchOptions,
    votes_this_session: usize,
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
        let options = SearchOptions::default();
        let gamepalate = Arc::clone(gamepalate);
        let search = options.clone();
        let task = Task::perform(
            async move { gamepalate.recommend(&search) },
            Message::Recommended,
        );

        (
            Discover {
                options,
                votes_this_session: 0,
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
        let filters = container(
            row![
                text!("{} ·", format::filters(&self.options))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                button(text("Filters (Ctrl+F) ▾").size(font::NOTE_SIZE))
                    .padding(0)
                    .style(style::link),
                space().width(Fill),
                text(format::votes_this_session(self.votes_this_session))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .spacing(4)
            .align_y(Center),
        )
        .padding([6, 20])
        .width(Fill);
        let main = container(self.game())
            .padding(16)
            .width(Fill)
            .height(Fill)
            .clip(true);
        let votes = container(text("Votes")).padding([10, 12]).width(Fill);

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
