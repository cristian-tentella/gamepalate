use std::sync::Arc;

use gamepalate_app::{Gamepalate, RecommendError, Recommendation, RecommendedGame, SearchOptions};
use iced::{
    Element, Task,
    widget::{button, column, row, text},
};

struct Viewer {
    gamepalate: Arc<Gamepalate>,
    state: State,
}

#[derive(Clone, Debug)]
enum Message {
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

impl Viewer {
    fn new(gamepalate: Arc<Gamepalate>) -> (Viewer, Task<Message>) {
        let for_task = Arc::clone(&gamepalate);

        let task = Task::perform(
            async move { for_task.recommend(&SearchOptions::default()) },
            Message::Recommended,
        );

        (
            Viewer {
                gamepalate,
                state: State::Loading,
            },
            task,
        )
    }

    fn update(&mut self, message: Message) {
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

    fn view(&self) -> Element<'_, Message> {
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

fn main() -> anyhow::Result<()> {
    let gamepalate = Arc::new(Gamepalate::open(None)?);

    iced::application(
        move || Viewer::new(Arc::clone(&gamepalate)),
        Viewer::update,
        Viewer::view,
    )
    .run()?;

    Ok(())
}
