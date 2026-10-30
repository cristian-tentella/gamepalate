mod queue;

use std::{process::Command, sync::Arc, thread, time::Duration};

use gamepalate_app::{
    Gamepalate, RecommendError, Recommendation, RecommendedGame, SearchOptions, VoteKind,
};
use iced::{
    Center, Element, Fill, Subscription, Task,
    futures::channel::oneshot,
    keyboard::{self, Key, key::Named},
    padding,
    widget::{button, column, container, responsive, row, space, stack, text},
};

use self::queue::Queue;
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
const TOAST_DURATION: Duration = Duration::from_secs(4);
const FLASH_DURATION: Duration = Duration::from_millis(600);
const PREVIOUS: char = '\u{E76B}';
const NEXT: char = '\u{E76C}';
const CHECK: char = '\u{E73E}';

pub struct Discover {
    gamepalate: Arc<Gamepalate>,
    options: SearchOptions,
    votes_this_session: usize,
    toast: Option<Toast>,
    flash: Option<Flash>,
    timers_started: u64,
    state: State,
}

struct Flash {
    id: u64,
    kind: VoteKind,
}

struct Toast {
    id: u64,
    text: String,
}

#[derive(Clone, Debug)]
pub enum Message {
    Recommended(Result<Recommendation, RecommendError>),
    MoreRecommended(Result<Recommendation, RecommendError>),
    Previous,
    Next,
    Vote(VoteKind),
    Open(Link),
    HideToast(u64),
    EndFlash(u64),
}

#[derive(Clone, Copy, Debug)]
pub enum Link {
    Steam,
    Eneba,
    Hltb,
}

enum State {
    Loading,
    Ready(Queue),
    Failed(String),
}

impl Discover {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Discover, Task<Message>) {
        let options = SearchOptions::default();
        let search = options.clone();
        let recommender = Arc::clone(gamepalate);
        let task = Task::perform(
            async move { recommender.recommend(&search) },
            Message::Recommended,
        );

        (
            Discover {
                gamepalate: Arc::clone(gamepalate),
                options,
                votes_this_session: 0,
                toast: None,
                flash: None,
                timers_started: 0,
                state: State::Loading,
            },
            task,
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Recommended(Ok(recommendation)) => {
                let mut queue = Queue::new(recommendation.games);
                let load = queue.wants_next();
                self.state = State::Ready(queue);
                if load {
                    return self.load_more();
                }
            }
            Message::Recommended(Err(error)) => {
                self.state = State::Failed(error.to_string());
            }
            Message::MoreRecommended(result) => {
                if let State::Ready(queue) = &mut self.state {
                    match result {
                        Ok(recommendation) => queue.receive(recommendation.games),
                        Err(error) => {
                            queue.failed();
                            return self.show_toast(error.to_string());
                        }
                    }
                    if queue.wants_next() {
                        return self.load_more();
                    }
                }
            }
            Message::Previous => {
                if let State::Ready(queue) = &mut self.state {
                    queue.back();
                }
            }
            Message::Next => return self.advance(),
            Message::Vote(kind) => {
                if let Some(game) = self.current() {
                    let text = format::voted(&game.name, kind);
                    return Task::batch([self.show_toast(text), self.flash(kind), self.advance()]);
                }
            }
            Message::Open(link) => {
                if let Some(game) = self.current() {
                    let url = match link {
                        Link::Steam => game.store_url.clone(),
                        Link::Eneba => format::eneba_url(&game.name),
                        Link::Hltb => format::hltb_url(&game.name),
                    };
                    let _ = Command::new("rundll32")
                        .args(["url.dll,FileProtocolHandler", &url])
                        .spawn();
                }
            }
            Message::EndFlash(id) => {
                if self.flash.as_ref().is_some_and(|flash| flash.id == id) {
                    self.flash = None;
                }
            }
            Message::HideToast(id) => {
                if self.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.toast = None;
                }
            }
        }

        Task::none()
    }

    pub fn subscription() -> Subscription<Message> {
        keyboard::listen().filter_map(key_message)
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
        let toast = self.toast.as_ref().map(|toast| {
            container(
                container(text(&toast.text))
                    .padding([8, 14])
                    .style(style::toast),
            )
            .center_x(Fill)
            .align_bottom(Fill)
            .padding(16)
        });
        let game = container(self.game())
            .padding(16)
            .width(Fill)
            .height(Fill)
            .clip(true);
        let main = stack![game].push(toast);
        let votes = container(
            container(responsive(|size| self.vote_row(size.width < COMPACT_WIDTH)))
                .height(VOTE_HEIGHT),
        )
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
        let flashed = self.flash.as_ref().map(|flash| flash.kind);
        let votes = row(VOTES.map(|(key, kind)| {
            vote_button(key, kind, compact, game.is_some(), flashed == Some(kind))
        }))
        .push(hint)
        .spacing(8)
        .width(Fill)
        .align_y(Center);
        let links = row![
            link_button("5 Steam ↗", Link::Steam, game.is_some()),
            link_button("6 Eneba ↗", Link::Eneba, game.is_some()),
            link_button("7 HLTB ↗", Link::Hltb, game.is_some()),
        ]
        .spacing(20);

        row![votes, links]
            .spacing(8)
            .height(Fill)
            .align_y(Center)
            .into()
    }

    fn advance(&mut self) -> Task<Message> {
        let load = match &mut self.state {
            State::Ready(queue) => queue.advance(),
            State::Loading | State::Failed(_) => false,
        };

        if load { self.load_more() } else { Task::none() }
    }

    fn load_more(&self) -> Task<Message> {
        let State::Ready(queue) = &self.state else {
            return Task::none();
        };
        let gamepalate = Arc::clone(&self.gamepalate);
        let options = self.options.clone();
        let seen = queue.seen();

        Task::perform(
            async move { gamepalate.recommend_after(&options, &seen) },
            Message::MoreRecommended,
        )
    }

    fn show_toast(&mut self, text: String) -> Task<Message> {
        let id = self.start_timer();
        self.toast = Some(Toast { id, text });

        after(TOAST_DURATION, Message::HideToast(id))
    }

    fn flash(&mut self, kind: VoteKind) -> Task<Message> {
        let id = self.start_timer();
        self.flash = Some(Flash { id, kind });

        after(FLASH_DURATION, Message::EndFlash(id))
    }

    fn start_timer(&mut self) -> u64 {
        self.timers_started += 1;
        self.timers_started
    }

    fn current(&self) -> Option<&RecommendedGame> {
        match &self.state {
            State::Ready(queue) => queue.current(),
            State::Loading | State::Failed(_) => None,
        }
    }

    fn game(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.state {
            State::Loading => text("Loading…").style(style::muted).into(),
            State::Failed(error) => text!("Error: {error}").into(),
            State::Ready(queue) => match queue.current() {
                None => text("No games match these filters.")
                    .style(style::muted)
                    .into(),
                Some(game) => {
                    let (position, total) = queue.position();
                    let progress = if queue.is_waiting() {
                        "Loading the next games…".to_string()
                    } else {
                        format!("{position} / {total}")
                    };

                    let details = column![
                        text(&game.name).size(font::TITLE_SIZE).font(font::SEMIBOLD),
                        text(progress).size(font::NOTE_SIZE).style(style::muted),
                    ]
                    .spacing(8)
                    .align_x(Center);

                    row![
                        arrow_button(PREVIOUS, queue.has_previous().then_some(Message::Previous)),
                        container(details).center_x(Fill),
                        arrow_button(NEXT, queue.has_next().then_some(Message::Next)),
                    ]
                    .align_y(Center)
                    .into()
                }
            },
        };

        container(content).center(Fill).into()
    }
}

fn vote_button<'a>(
    key: &'a str,
    kind: VoteKind,
    compact: bool,
    enabled: bool,
    flashed: bool,
) -> Element<'a, Message> {
    let mark = if flashed {
        text(CHECK).font(font::ICONS).size(10)
    } else {
        text(key).size(11).style(style::muted)
    };
    let keycap = container(mark)
        .center_x(18)
        .center_y(18)
        .style(style::keycap(flashed));
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
        .style(style::vote(flashed))
        .on_press_maybe(enabled.then_some(Message::Vote(kind))),
    )
    .width(Fill)
    .max_width(VOTE_WIDTH)
    .into()
}

fn after(duration: Duration, message: Message) -> Task<Message> {
    let (sender, receiver) = oneshot::channel();
    thread::spawn(move || {
        thread::sleep(duration);
        let _ = sender.send(());
    });

    Task::perform(receiver, move |_| message)
}

fn arrow_button<'a>(icon: char, message: Option<Message>) -> Element<'a, Message> {
    button(text(icon).font(font::ICONS).size(20).center())
        .width(44)
        .height(44)
        .padding(0)
        .style(style::arrow)
        .on_press_maybe(message)
        .into()
}

fn link_button(label: &str, link: Link, enabled: bool) -> Element<'_, Message> {
    button(text(label).size(font::BAR_SIZE))
        .padding(0)
        .style(style::link)
        .on_press_maybe(enabled.then_some(Message::Open(link)))
        .into()
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };

    match key {
        Key::Named(Named::Space | Named::ArrowRight) => Some(Message::Next),
        Key::Named(Named::Backspace | Named::ArrowLeft) => Some(Message::Previous),
        Key::Character(character) => match character.as_str() {
            "5" => Some(Message::Open(Link::Steam)),
            "6" => Some(Message::Open(Link::Eneba)),
            "7" => Some(Message::Open(Link::Hltb)),
            key => VOTES
                .iter()
                .find(|(vote_key, _)| *vote_key == key)
                .map(|(_, kind)| Message::Vote(*kind)),
        },
        Key::Named(_) | Key::Unidentified => None,
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::{Location, Modifiers, key};

    use super::*;

    #[test]
    fn votes_with_the_keys_from_one_to_four() {
        assert!(matches!(
            key_message(pressed(Key::Character("1".into()))),
            Some(Message::Vote(VoteKind::WouldTry))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("4".into()))),
            Some(Message::Vote(VoteKind::Known))
        ));
    }

    #[test]
    fn opens_the_links_with_the_keys_from_five_to_seven() {
        assert!(matches!(
            key_message(pressed(Key::Character("5".into()))),
            Some(Message::Open(Link::Steam))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("7".into()))),
            Some(Message::Open(Link::Hltb))
        ));
    }

    #[test]
    fn ignores_the_other_keys() {
        assert!(key_message(pressed(Key::Character("8".into()))).is_none());
        assert!(key_message(pressed(Key::Named(Named::Enter))).is_none());
    }

    fn pressed(key: Key) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        }
    }
}
