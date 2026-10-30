use std::{process::Command, sync::Arc};

use gamepalate_app::{
    Gamepalate, RecommendError, Recommendation, RecommendedGame, SearchOptions, VoteKind,
};
use iced::{
    Center, Element, Fill, Task, padding,
    widget::{button, column, container, responsive, row, space, text},
};

use crate::{font, format, style};

const VOTES: [(&str, VoteKind); 4] = [
    ("1", VoteKind::WouldTry),
    ("2", VoteKind::Maybe),
    ("3", VoteKind::NotForMe),
    ("4", VoteKind::Known),
];
const COMPACT_WIDTH: f32 = 1100.0;
const COMPACT_VOTE_SIZE: f32 = 14.0;
const VOTE_WIDTH: f32 = 172.0;
const VOTE_HEIGHT: f32 = 36.0;

pub struct Discover {
    options: SearchOptions,
    votes_this_session: usize,
    feedback: Option<String>,
    state: State,
}

#[derive(Clone, Debug)]
pub enum Message {
    Recommended(Result<Recommendation, RecommendError>),
    Previous,
    Next,
    Vote(VoteKind),
    Open(String),
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
                feedback: None,
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
            Message::Vote(kind) => {
                if let Some(game) = self.current() {
                    self.feedback = Some(format::voted(&game.name, kind));
                }
            }
            Message::Open(url) => {
                let _ = Command::new("explorer").arg(url).spawn();
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
        let feedback = self
            .feedback
            .as_deref()
            .map(|feedback| text(feedback).size(font::NOTE_SIZE).style(style::muted));
        let buttons = container(responsive(|size| self.vote_row(size.width < COMPACT_WIDTH)))
            .height(VOTE_HEIGHT);
        let votes = container(column![].push(feedback).push(buttons).spacing(6))
            .padding([10, 20])
            .width(Fill);

        column![filters, style::separator(), main, style::separator(), votes].into()
    }

    fn vote_row(&self, compact: bool) -> Element<'_, Message> {
        let game = self.current();
        let hint = (!compact).then(|| {
            container(
                text("Space skip · ⌫ back")
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            )
            .padding(padding::left(4))
        });
        let votes = row(VOTES.map(|(key, kind)| vote_button(key, kind, compact, game.is_some())))
            .push(hint)
            .spacing(8)
            .width(Fill)
            .align_y(Center);
        let links = row![
            link_button("5 Steam ↗", game.map(|game| game.store_url.clone())),
            link_button("6 Eneba ↗", game.map(|game| format::eneba_url(&game.name))),
            link_button("7 HLTB ↗", game.map(|game| format::hltb_url(&game.name))),
        ]
        .spacing(20);

        row![votes, links]
            .spacing(8)
            .height(Fill)
            .align_y(Center)
            .into()
    }

    fn current(&self) -> Option<&RecommendedGame> {
        match &self.state {
            State::Ready { games, index } => games.get(*index),
            State::Loading | State::Failed(_) => None,
        }
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

fn vote_button<'a>(
    key: &'a str,
    kind: VoteKind,
    compact: bool,
    enabled: bool,
) -> Element<'a, Message> {
    let keycap = container(text(key).size(11).style(style::muted))
        .center_x(18)
        .center_y(18)
        .style(style::keycap);
    let label = text(format::vote(kind))
        .size(if compact {
            COMPACT_VOTE_SIZE
        } else {
            font::VOTE_SIZE
        })
        .font(font::SEMIBOLD);

    container(
        button(
            container(row![keycap, label].spacing(10).align_y(Center))
                .center_x(Fill)
                .center_y(Fill),
        )
        .width(Fill)
        .height(VOTE_HEIGHT)
        .padding(0)
        .style(style::vote)
        .on_press_maybe(enabled.then_some(Message::Vote(kind))),
    )
    .width(Fill)
    .max_width(VOTE_WIDTH)
    .into()
}

fn link_button(label: &str, url: Option<String>) -> Element<'_, Message> {
    button(text(label).size(font::BAR_SIZE))
        .padding(0)
        .style(style::link)
        .on_press_maybe(url.map(Message::Open))
        .into()
}
