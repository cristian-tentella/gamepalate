use std::sync::Arc;

use gamepalate_app::{AppId, FamilyChange, GameStanding, GameView, Gamepalate, VoteKind};
use iced::{
    Center, Element, Fill, Size, Subscription, Task,
    keyboard::{self, Key, key::Named},
    never, padding,
    widget::{
        button, column, container, opaque, pick_list, responsive, rich_text, row, scrollable,
        space, span, stack, text,
    },
};
use jiff::civil::Date;

use super::search::standing_text;
use crate::{
    font, format,
    screen::{
        Toasts,
        game::{self, Link, Pick, VOTE_HEIGHT},
        sentence,
        showcase::{self, Showcase},
    },
    style,
};

const HEADER_WIDTH: f32 = 230.0;
const HEADER_HEIGHT: f32 = HEADER_WIDTH * 215.0 / 460.0;
const TOP_HEIGHT: f32 = 160.0;
const MIN_SHOT_WIDTH: f32 = 480.0;
const CONTENT_SPACING: f32 = 18.0;
const SCROLLBAR_GAP: f32 = 12.0;
const BESIDE_VOTES: f32 = 520.0;

/// A game found by name, open over the page you were on until you go back.
pub struct GamePage {
    gamepalate: Arc<Gamepalate>,
    game: GameView,
    showcase: Showcase,
    toasts: Toasts,
    played: bool,
    changed: bool,
}

#[derive(Clone, Debug)]
pub enum Message {
    Vote(Pick),
    Later,
    Played,
    RemoveVote,
    AddToFamily(String),
    Saved(String, bool, Result<Option<GameView>, String>),
    Undo,
    Open(Link),
    Back,
    HideToast(u64),
    Showcase(showcase::Message),
}

pub enum Action {
    Run(Task<Message>),
    /// `changed` says whether the votes, the Library or the families are no longer what the page
    /// under it shows.
    Close {
        changed: bool,
    },
}

#[derive(Debug, PartialEq)]
enum Status {
    Voted(Pick, String),
    Plain(String),
}

impl GamePage {
    pub fn new(gamepalate: &Arc<Gamepalate>, id: AppId) -> Option<(GamePage, Task<Message>)> {
        let game = gamepalate.game_view(id)?;
        let mut page = GamePage {
            gamepalate: Arc::clone(gamepalate),
            game,
            showcase: Showcase::new(gamepalate),
            toasts: Toasts::default(),
            played: false,
            changed: false,
        };
        let task = page.load();

        Some((page, task))
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    pub fn use_gamepalate(&mut self, gamepalate: &Arc<Gamepalate>) {
        self.gamepalate = Arc::clone(gamepalate);
        if let Some(game) = gamepalate.game_view(self.game.id) {
            self.game = game;
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        if self.showcase.is_open() && stopped_by_popup(&message) {
            return Action::Run(Task::none());
        }
        let task = match self.handle(message) {
            Action::Run(task) => task,
            close @ Action::Close { .. } => return close,
        };

        Action::Run(Task::batch([task, self.load()]))
    }

    fn handle(&mut self, message: Message) -> Action {
        let id = self.game.id;
        let task = match message {
            Message::Vote(pick) => match pick.ballot() {
                Some((kind, genre_ok)) => self.save(
                    format::voted(&self.game.name, pick),
                    false,
                    move |gamepalate| {
                        gamepalate
                            .vote(id, kind, genre_ok, false, true)
                            .map_err(|error| error.to_string())
                    },
                ),
                None => Task::none(),
            },
            Message::Later => self.save(
                format::voted(&self.game.name, Pick::Vote(VoteKind::Later)),
                false,
                move |gamepalate| {
                    gamepalate
                        .vote(id, VoteKind::Later, false, false, true)
                        .map_err(|error| error.to_string())
                },
            ),
            Message::Played if offers_played(&self.game) && !self.game.played_elsewhere => self
                .save(
                    format::elsewhere_text(&self.game.name, true),
                    true,
                    move |gamepalate| {
                        gamepalate
                            .set_played_elsewhere(id, true)
                            .map(|_| ())
                            .map_err(|error| sentence(&error.to_string()))
                    },
                ),
            Message::Played => Task::none(),
            Message::RemoveVote if self.game.vote.is_some() => self.save(
                format::vote_removed(&self.game.name),
                false,
                move |gamepalate| {
                    gamepalate
                        .remove_vote(id)
                        .map_err(|error| error.to_string())
                },
            ),
            Message::RemoveVote => Task::none(),
            Message::AddToFamily(family) if joins_families(&self.game) => {
                let text = format::added_to_family(&family);
                let change = FamilyChange::AddGame { family, game: id };
                self.save(text, false, move |gamepalate| {
                    gamepalate
                        .change_families(&change)
                        .map(|_| ())
                        .map_err(|error| sentence(&error.to_string()))
                })
            }
            Message::AddToFamily(_) => Task::none(),
            Message::Saved(text, undo, Ok(game)) => {
                if let Some(game) = game {
                    self.game = game;
                }
                self.changed = true;
                self.played = undo;
                self.toasts.show(text, undo, Message::HideToast)
            }
            Message::Saved(_, _, Err(error)) => self.show_toast(error),
            Message::Undo if self.played => {
                self.played = false;
                self.save("Undone".to_string(), false, move |gamepalate| {
                    gamepalate
                        .set_played_elsewhere(id, false)
                        .map(|_| ())
                        .map_err(|error| sentence(&error.to_string()))
                })
            }
            Message::Undo => Task::none(),
            Message::Open(link) => {
                game::open(link, &self.game.name, &self.game.store_url);
                Task::none()
            }
            Message::Back if self.showcase.is_open() => self
                .showcase
                .update(showcase::Message::Close)
                .map(Message::Showcase),
            Message::Back => {
                return Action::Close {
                    changed: self.changed,
                };
            }
            Message::HideToast(toast) => {
                self.toasts.hide(toast);
                Task::none()
            }
            Message::Showcase(message) => self.showcase.update(message).map(Message::Showcase),
        };

        Action::Run(task)
    }

    pub fn subscription(keys: bool) -> Subscription<Message> {
        if !keys {
            return Subscription::none();
        }
        keyboard::listen().filter_map(key_message)
    }

    pub fn view(&self, scale: f32) -> Element<'_, Message> {
        let toast = self.toasts.view(Message::Undo);
        let card = container(responsive(move |size| self.details(size, scale)))
            .padding(16)
            .width(Fill)
            .height(Fill)
            .clip(true);
        let actions = container(
            container(responsive(|size| self.action_row(size.width))).height(VOTE_HEIGHT),
        )
        .padding([10, 20])
        .width(Fill);
        let page = container(column![
            stack![card].push(toast),
            style::separator(),
            actions
        ])
        .width(Fill)
        .height(Fill)
        .style(style::page);

        stack![opaque(page)]
            .push(self.showcase.zoom().map(|zoom| zoom.map(Message::Showcase)))
            .push(
                self.showcase
                    .popup(&self.game.name)
                    .map(|popup| popup.map(Message::Showcase)),
            )
            .into()
    }

    fn details(&self, size: Size, scale: f32) -> Element<'_, Message> {
        let game = &self.game;
        let shot_width = showcase::screenshot_width(
            size.width - SCROLLBAR_GAP,
            size.height - TOP_HEIGHT - CONTENT_SPACING,
            MIN_SHOT_WIDTH,
            showcase::max_screenshot_width(scale),
        );
        let status = status(game, jiff::Zoned::now().date()).map(|status| {
            let spans = match status {
                Status::Voted(pick, day) => vec![
                    span("Voted "),
                    span(format::pick(pick)).font(font::SEMIBOLD),
                    span(format!(" · {day}")),
                ],
                Status::Plain(line) => vec![span(line)],
            };
            rich_text(spans)
                .size(font::BAR_SIZE)
                .style(style::muted)
                .on_link_click(never)
        });
        let info = column![game::title(&game.name)]
            .push(status)
            .push(game::reviews(&game.reviews))
            .push(game::price_row(
                &game.price,
                self.gamepalate.currency(),
                game::PRICE_GAP,
            ))
            .spacing(10)
            .width(Fill);
        let top = row![
            self.showcase
                .header(game.id, HEADER_WIDTH, HEADER_HEIGHT)
                .map(Message::Showcase),
            info,
            game::header_links(HEADER_HEIGHT, |link| Some(Message::Open(link))),
        ]
        .spacing(20)
        .align_y(Center);

        let content = column![
            top,
            self.showcase
                .media(game.id, shot_width, shot_width * 9.0 / 16.0)
                .map(Message::Showcase),
        ]
        .push(
            self.showcase
                .facts(game.id, &self.showcase.used_tags())
                .map(|facts| facts.map(Message::Showcase)),
        )
        .spacing(CONTENT_SPACING)
        .max_width(shot_width);

        scrollable(
            container(content)
                .center_x(Fill)
                .padding(padding::right(SCROLLBAR_GAP)),
        )
        .height(Fill)
        .into()
    }

    fn action_row(&self, width: f32) -> Element<'_, Message> {
        let game = &self.game;
        let vote_width = game::vote_width(width, BESIDE_VOTES);
        let current = game.vote.as_ref().map(Pick::of);
        let (would_try_hours, elsewhere_hours) = (
            self.gamepalate.would_try_hours(),
            self.gamepalate.elsewhere_hours(),
        );
        let votes = game::vote_buttons(
            |key, pick| {
                let (message, current) = match pick {
                    Pick::Played => (
                        (!game.played_elsewhere).then_some(Message::Played),
                        game.played_elsewhere,
                    ),
                    pick => (Some(Message::Vote(pick)), current == Some(pick)),
                };
                game::vote_button(
                    key,
                    pick,
                    game::tip(pick, would_try_hours, elsewhere_hours),
                    true,
                    vote_width,
                    message,
                    current,
                )
            },
            offers_played(game),
        )
        .push(
            row![
                style::divider(),
                game::moves(Some(Message::Later), Some(Message::Back)),
            ]
            .spacing(game::ACTION_SPACING)
            .align_y(Center),
        );
        let remove = game.vote.is_some().then(|| {
            button(text("Remove vote").size(font::BAR_SIZE))
                .padding(0)
                .style(style::key_action)
                .on_press(Message::RemoveVote)
        });
        let families = (joins_families(game) && !game.other_families.is_empty()).then(|| {
            pick_list(
                game.other_families.as_slice(),
                None::<String>,
                Message::AddToFamily,
            )
            .placeholder("Add to family")
            .text_size(font::NOTE_SIZE)
            .padding([4, 8])
            .style(style::dropdown)
            .menu_style(style::menu)
        });

        row![votes, space().width(Fill)]
            .push(remove)
            .push(families)
            .spacing(game::ACTION_SPACING)
            .height(Fill)
            .align_y(Center)
            .into()
    }

    fn save(
        &self,
        text: String,
        undo: bool,
        change: impl FnOnce(&Gamepalate) -> Result<(), String> + Send + 'static,
    ) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        let id = self.game.id;

        Task::perform(
            async move {
                change(&gamepalate)?;
                Ok(gamepalate.game_view(id))
            },
            move |result| Message::Saved(text.clone(), undo, result),
        )
    }

    fn load(&mut self) -> Task<Message> {
        let id = [self.game.id];
        self.showcase.follow(Some(self.game.id));

        self.showcase.load(&id, &id, &[]).map(Message::Showcase)
    }

    fn show_toast(&mut self, text: String) -> Task<Message> {
        self.toasts.show(text, false, Message::HideToast)
    }
}

/// A game of the Library is played already: 4 is not offered there.
fn offers_played(game: &GameView) -> bool {
    !game.in_library
}

/// The families are made of games you played, as in the Library.
fn joins_families(game: &GameView) -> bool {
    game.in_library || game.played_elsewhere
}

fn status(game: &GameView, today: Date) -> Option<Status> {
    let standing = game.standing?;
    let voted = matches!(
        standing,
        GameStanding::WouldTry | GameStanding::NotThisOne | GameStanding::NotForMe
    );

    match &game.vote {
        Some(vote) if voted => Some(Status::Voted(
            Pick::of(vote),
            format::voted_day(&vote.date, today),
        )),
        _ => Some(Status::Plain(standing_text(standing))),
    }
}

fn stopped_by_popup(message: &Message) -> bool {
    matches!(
        message,
        Message::Vote(_)
            | Message::Later
            | Message::Played
            | Message::RemoveVote
            | Message::AddToFamily(_)
            | Message::Open(_)
    )
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("z" | "Z") => Some(Message::Undo),
            _ => None,
        };
    }

    match key {
        Key::Named(Named::Escape | Named::Backspace) => Some(Message::Back),
        Key::Named(Named::Space) => Some(Message::Later),
        Key::Named(Named::ArrowUp) => Some(Message::Showcase(showcase::Message::ScrollUp)),
        Key::Named(Named::ArrowDown) => Some(Message::Showcase(showcase::Message::ScrollDown)),
        Key::Named(Named::ArrowRight) => Some(Message::Showcase(showcase::Message::NextScreenshot)),
        Key::Named(Named::ArrowLeft) => {
            Some(Message::Showcase(showcase::Message::PreviousScreenshot))
        }
        Key::Character(key) if key.eq_ignore_ascii_case("d") => {
            Some(Message::Showcase(showcase::Message::ToggleAbout))
        }
        Key::Character(key) if key.eq_ignore_ascii_case("t") => {
            Some(Message::Showcase(showcase::Message::ToggleTags))
        }
        Key::Character(key) if key.eq_ignore_ascii_case("k") => {
            Some(Message::Showcase(showcase::Message::ScrollUp))
        }
        Key::Character(key) if key.eq_ignore_ascii_case("j") => {
            Some(Message::Showcase(showcase::Message::ScrollDown))
        }
        Key::Character(key) => game::pick_key(key.as_str())
            .map(|pick| match pick {
                Pick::Played => Message::Played,
                pick => Message::Vote(pick),
            })
            .or_else(|| game::link_key(key.as_str()).map(Message::Open)),
        Key::Named(_) | Key::Unidentified => None,
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::{Price, Reviews, Vote};
    use iced::keyboard::{Location, Modifiers, key};

    use super::*;

    fn found(standing: Option<GameStanding>, vote: Option<Vote>) -> GameView {
        GameView {
            id: AppId(1145360),
            name: "Hades".to_string(),
            price: Price {
                original_cents: 2450,
                discount: None,
            },
            reviews: Reviews {
                total: 100,
                positive: 98,
            },
            store_url: String::new(),
            standing,
            vote,
            in_library: matches!(standing, Some(GameStanding::InLibrary { .. })),
            played_elsewhere: standing == Some(GameStanding::PlayedElsewhere),
            other_families: vec!["cozy".to_string()],
        }
    }

    fn today() -> Date {
        "2026-11-13".parse().unwrap()
    }

    #[test]
    fn the_keys_vote_mark_played_and_open_the_links() {
        assert!(matches!(
            key_message(pressed(Key::Character("1".into()))),
            Some(Message::Vote(Pick::Vote(VoteKind::WouldTry)))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("2".into()))),
            Some(Message::Vote(Pick::NotThisOne))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("3".into()))),
            Some(Message::Vote(Pick::Vote(VoteKind::NotForMe)))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("4".into()))),
            Some(Message::Played)
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(Named::Space))),
            Some(Message::Later)
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("Q".into()))),
            Some(Message::Open(Link::Steam))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("e".into()))),
            Some(Message::Open(Link::Hltb))
        ));
        assert!(key_message(pressed(Key::Character("5".into()))).is_none());
    }

    #[test]
    fn escape_and_backspace_go_back() {
        for key in [Named::Escape, Named::Backspace] {
            assert!(matches!(
                key_message(pressed(Key::Named(key))),
                Some(Message::Back)
            ));
        }
    }

    #[test]
    fn the_description_tags_and_screenshots_have_the_keys_of_discover() {
        assert!(matches!(
            key_message(pressed(Key::Character("d".into()))),
            Some(Message::Showcase(showcase::Message::ToggleAbout))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("T".into()))),
            Some(Message::Showcase(showcase::Message::ToggleTags))
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(Named::ArrowRight))),
            Some(Message::Showcase(showcase::Message::NextScreenshot))
        ));
    }

    #[test]
    fn ctrl_z_undoes_the_game_played_elsewhere() {
        assert!(matches!(
            key_message(pressed_with(Key::Character("z".into()), Modifiers::CTRL)),
            Some(Message::Undo)
        ));
        assert!(key_message(pressed_with(Key::Character("1".into()), Modifiers::CTRL)).is_none());
    }

    #[test]
    fn an_open_popup_stops_the_actions_but_not_going_back() {
        assert!(stopped_by_popup(&Message::Vote(Pick::NotThisOne)));
        assert!(stopped_by_popup(&Message::Later));
        assert!(stopped_by_popup(&Message::Played));
        assert!(stopped_by_popup(&Message::RemoveVote));
        assert!(stopped_by_popup(&Message::Open(Link::Eneba)));
        assert!(!stopped_by_popup(&Message::Back));
        assert!(!stopped_by_popup(&Message::Showcase(
            showcase::Message::Close
        )));
    }

    #[test]
    fn a_game_of_the_library_has_no_played_and_can_join_a_family() {
        let owned = found(Some(GameStanding::InLibrary { hours: 42.0 }), None);
        let elsewhere = found(Some(GameStanding::PlayedElsewhere), None);
        let unknown = found(None, None);

        assert!(!offers_played(&owned) && joins_families(&owned));
        assert!(offers_played(&elsewhere) && joins_families(&elsewhere));
        assert!(offers_played(&unknown) && !joins_families(&unknown));
    }

    #[test]
    fn the_status_says_the_vote_and_its_day() {
        let vote = Vote {
            genre_ok: true,
            ..Vote::new(VoteKind::NotForMe, "2026-11-13")
        };

        assert_eq!(
            status(&found(Some(GameStanding::NotThisOne), Some(vote)), today()),
            Some(Status::Voted(Pick::NotThisOne, "today".to_string()))
        );
        assert_eq!(
            status(
                &found(
                    Some(GameStanding::WouldTry),
                    Some(Vote::new(VoteKind::WouldTry, "2026-11-02"))
                ),
                today()
            ),
            Some(Status::Voted(
                Pick::Vote(VoteKind::WouldTry),
                "2 Nov".to_string()
            ))
        );
    }

    #[test]
    fn the_status_says_the_library_the_games_played_elsewhere_and_the_laters() {
        let later = Vote::new(VoteKind::Later, "2026-11-01");

        assert_eq!(
            status(
                &found(Some(GameStanding::InLibrary { hours: 42.0 }), None),
                today()
            ),
            Some(Status::Plain("In your library · 42 h".to_string()))
        );
        assert_eq!(
            status(&found(Some(GameStanding::PlayedElsewhere), None), today()),
            Some(Status::Plain("Played elsewhere".to_string()))
        );
        assert_eq!(
            status(
                &found(Some(GameStanding::Later { days_left: 12 }), Some(later)),
                today()
            ),
            Some(Status::Plain("Later · back in 12 days".to_string()))
        );
        assert_eq!(status(&found(None, None), today()), None);
    }

    fn pressed(key: Key) -> keyboard::Event {
        pressed_with(key, Modifiers::empty())
    }

    fn pressed_with(key: Key, modifiers: Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }
}
