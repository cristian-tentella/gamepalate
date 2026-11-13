use std::sync::Arc;

use gamepalate_app::{AppId, FoundGame, GameStanding, Gamepalate, MIN_SEARCH_CHARS};
use iced::{
    Center, ContentFit, Element, Event, Fill, Task, event,
    keyboard::{self, Key, key::Named},
    widget::{
        Id, button, column, container, image, operation, operation::RelativeOffset, row,
        scrollable, space, text, text_input,
    },
    window,
};

use crate::{
    dropdown::dropdown,
    font,
    screen::{
        library::{handle, hours},
        showcase::images::{Images, Picture},
    },
    style,
};

const FIELD: Id = Id::new("game-search");
const MENU: Id = Id::new("game-search-menu");
const PLACEHOLDER: &str = "Search games (Ctrl+K)";
const NO_GAMES: &str = "No games found";
const WIDE_WIDTH: f32 = 280.0;
const NARROW_WIDTH: f32 = 200.0;
const ROOM_FOR_THE_REST: f32 = 1000.0;
const MENU_WIDTH: f32 = 440.0;
const ROW_HEIGHT: f32 = 42.0;
const CAPSULE_WIDTH: f32 = 64.0;
const CAPSULE_HEIGHT: f32 = CAPSULE_WIDTH * 87.0 / 231.0;
const CAPSULE_RADIUS: f32 = 3.0;

#[derive(Default)]
pub struct Search {
    text: String,
    found: Vec<FoundGame>,
    selected: Option<usize>,
    open: bool,
    capsules: Images<image::Allocation, AppId>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Input(String),
    Focus,
    Down,
    Up,
    Submit,
    Close,
    Pick(AppId),
    CapsuleLoaded(AppId, Option<image::Handle>),
    CapsuleAllocated(AppId, Option<image::Allocation>),
}

pub enum Action {
    Run(Task<Message>),
    Open(AppId),
}

impl Search {
    pub fn update(&mut self, message: Message, gamepalate: &Arc<Gamepalate>) -> Action {
        match message {
            Message::Input(text) => {
                self.found = gamepalate.find_games(&text);
                self.text = text;
                self.selected = None;
                self.open = true;
                return Action::Run(self.load_capsules(gamepalate));
            }
            Message::Focus => {
                self.found = gamepalate.find_games(&self.text);
                self.selected = None;
                self.open = true;
                return Action::Run(Task::batch([
                    operation::focus(FIELD),
                    operation::select_all(FIELD),
                    self.load_capsules(gamepalate),
                ]));
            }
            Message::Down => {
                return Action::Run(self.select(below(self.selected, self.found.len())));
            }
            Message::Up => return Action::Run(self.select(above(self.selected))),
            Message::Submit => {
                if let Some(id) = self.is_open().then(|| self.chosen()).flatten() {
                    return self.pick(id);
                }
            }
            Message::Close => self.close(),
            Message::Pick(id) => return self.pick(id),
            Message::CapsuleLoaded(id, None) => {
                self.capsules.loaded(id, None);
                return Action::Run(self.load_capsules(gamepalate));
            }
            Message::CapsuleLoaded(id, Some(handle)) => {
                return Action::Run(
                    image::allocate(handle)
                        .map(move |allocation| Message::CapsuleAllocated(id, allocation.ok())),
                );
            }
            Message::CapsuleAllocated(id, allocation) => {
                self.capsules.loaded(id, allocation);
                return Action::Run(self.load_capsules(gamepalate));
            }
        }
        Action::Run(Task::none())
    }

    /// While the menu is open, its keys are not the page's.
    pub fn is_open(&self) -> bool {
        self.open && shows_menu(&self.text, &self.found)
    }

    pub fn close(&mut self) {
        self.open = false;
        self.selected = None;
    }

    pub fn view(&self, window_width: f32) -> Element<'_, Message> {
        let field = text_input(PLACEHOLDER, &self.text)
            .id(FIELD)
            .on_input(Message::Input)
            .on_submit(Message::Submit)
            .style(style::field)
            .size(font::NOTE_SIZE)
            .padding([5, 8])
            .width(field_width(window_width));

        dropdown(field, self.is_open().then(|| self.menu()), Message::Close).into()
    }

    fn menu(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = if self.found.is_empty() {
            container(text(NO_GAMES).size(font::NOTE_SIZE).style(style::muted))
                .padding([10, 12])
                .into()
        } else {
            scrollable(
                column(
                    self.found
                        .iter()
                        .enumerate()
                        .map(|(index, game)| self.row(index, game)),
                )
                .padding(4),
            )
            .id(MENU)
            .into()
        };

        container(body).width(MENU_WIDTH).style(style::popup).into()
    }

    fn row<'a>(&'a self, index: usize, game: &'a FoundGame) -> Element<'a, Message> {
        let picture: Element<'a, Message> = match self.capsules.get(&game.id) {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
                .width(CAPSULE_WIDTH)
                .height(CAPSULE_HEIGHT)
                .content_fit(ContentFit::Cover)
                .border_radius(CAPSULE_RADIUS)
                .into(),
            Some(Picture::Loading | Picture::Failed) | None => container(space())
                .width(CAPSULE_WIDTH)
                .height(CAPSULE_HEIGHT)
                .style(style::picture)
                .into(),
        };
        let standing = game.standing.map(|standing| {
            text(standing_text(standing))
                .size(font::NOTE_SIZE)
                .style(style::muted)
        });

        button(
            row![picture, text(&game.name).size(font::BAR_SIZE).width(Fill)]
                .push(standing)
                .spacing(10)
                .align_y(Center),
        )
        .padding([4, 8])
        .width(Fill)
        .height(ROW_HEIGHT)
        .style(style::list_row(self.selected == Some(index)))
        .on_press(Message::Pick(game.id))
        .into()
    }

    fn select(&mut self, selected: Option<usize>) -> Task<Message> {
        if !self.is_open() || self.found.is_empty() {
            return Task::none();
        }
        self.selected = selected;
        let Some(index) = selected else {
            return Task::none();
        };

        operation::snap_to(
            MENU,
            RelativeOffset {
                x: None,
                y: Some(scroll_fraction(index, self.found.len())),
            },
        )
    }

    fn chosen(&self) -> Option<AppId> {
        let index = self.selected.unwrap_or(0);
        self.found.get(index).map(|game| game.id)
    }

    fn pick(&mut self, id: AppId) -> Action {
        self.close();
        self.text.clear();
        self.found.clear();
        Action::Open(id)
    }

    fn load_capsules(&mut self, gamepalate: &Arc<Gamepalate>) -> Task<Message> {
        let wanted: Vec<AppId> = self.found.iter().map(|game| game.id).collect();
        self.capsules.keep_only(&wanted);
        let started = self.capsules.want(wanted);

        Task::batch(started.into_iter().map(|id| {
            let gamepalate = Arc::clone(gamepalate);
            Task::perform(
                async move { gamepalate.capsule(id).await.ok().map(handle) },
                move |handle| Message::CapsuleLoaded(id, handle),
            )
        }))
    }
}

pub fn menu_key(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Named(key),
        ..
    }) = event
    else {
        return None;
    };

    match key {
        Named::ArrowDown => Some(Message::Down),
        Named::ArrowUp => Some(Message::Up),
        Named::Escape => Some(Message::Close),
        _ => None,
    }
}

fn shows_menu(text: &str, found: &[FoundGame]) -> bool {
    !found.is_empty() || text.trim().chars().count() >= MIN_SEARCH_CHARS
}

fn below(selected: Option<usize>, count: usize) -> Option<usize> {
    match selected {
        _ if count == 0 => None,
        None => Some(0),
        Some(index) => Some((index + 1).min(count - 1)),
    }
}

fn above(selected: Option<usize>) -> Option<usize> {
    Some(selected?.saturating_sub(1))
}

// Snapping in proportion to the row always shows the whole row, whatever the menu's height.
fn scroll_fraction(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.0
    } else {
        index as f32 / (count - 1) as f32
    }
}

fn field_width(window_width: f32) -> f32 {
    (window_width - ROOM_FOR_THE_REST).clamp(NARROW_WIDTH, WIDE_WIDTH)
}

fn standing_text(standing: GameStanding) -> String {
    match standing {
        GameStanding::InLibrary { hours: played } => format!("In your library · {}", hours(played)),
        GameStanding::PlayedElsewhere => "Played elsewhere".to_string(),
        GameStanding::WouldTry => "Would try".to_string(),
        GameStanding::NotThisOne => "Not this one".to_string(),
        GameStanding::NotForMe => "Not for me".to_string(),
        GameStanding::Later { days_left: 1 } => "Later · back in 1 day".to_string(),
        GameStanding::Later { days_left } => format!("Later · back in {days_left} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(count: u32) -> Vec<FoundGame> {
        (1..=count)
            .map(|id| FoundGame {
                id: AppId(id),
                name: format!("Game {id}"),
                standing: None,
            })
            .collect()
    }

    fn open_with(count: u32) -> Search {
        Search {
            text: "game".to_string(),
            found: found(count),
            open: true,
            ..Search::default()
        }
    }

    fn pressed(key: Named) -> Event {
        let key = Key::Named(key);
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn down_and_up_stop_at_the_first_and_the_last_row() {
        assert_eq!(below(None, 3), Some(0));
        assert_eq!(below(Some(1), 3), Some(2));
        assert_eq!(below(Some(2), 3), Some(2));
        assert_eq!(below(None, 0), None);

        assert_eq!(above(Some(2)), Some(1));
        assert_eq!(above(Some(0)), Some(0));
        assert_eq!(above(None), None);
    }

    #[test]
    fn down_moves_the_selection_of_an_open_menu() {
        let mut search = open_with(2);

        let _ = search.select(below(search.selected, 2));
        let _ = search.select(below(search.selected, 2));
        let _ = search.select(below(search.selected, 2));

        assert_eq!(search.selected, Some(1));
    }

    #[test]
    fn enter_without_a_selection_opens_the_first_game() {
        let mut search = open_with(3);
        assert_eq!(search.chosen(), Some(AppId(1)));

        search.selected = Some(2);
        assert_eq!(search.chosen(), Some(AppId(3)));
    }

    #[test]
    fn opening_a_game_closes_the_menu_and_empties_the_field() {
        let mut search = open_with(3);
        search.selected = Some(1);

        let Action::Open(id) = search.pick(AppId(2)) else {
            panic!("picking a game should open it");
        };

        assert_eq!(id, AppId(2));
        assert!(!search.is_open());
        assert!(search.text.is_empty());
        assert_eq!(search.selected, None);
    }

    #[test]
    fn the_menu_shows_from_three_characters_or_with_results() {
        assert!(!shows_menu("po", &[]));
        assert!(shows_menu("por", &[]));
        assert!(shows_menu("10", &found(1)));
        assert!(open_with(0).is_open());

        let mut search = open_with(2);
        assert!(search.is_open());
        search.close();
        assert!(!search.is_open());
    }

    #[test]
    fn the_arrows_and_escape_belong_to_the_menu() {
        assert!(matches!(menu_key_of(Named::ArrowDown), Some(Message::Down)));
        assert!(matches!(menu_key_of(Named::ArrowUp), Some(Message::Up)));
        assert!(matches!(menu_key_of(Named::Escape), Some(Message::Close)));
        assert!(menu_key_of(Named::Space).is_none());
    }

    fn menu_key_of(key: Named) -> Option<Message> {
        menu_key(pressed(key), event::Status::Captured, window::Id::unique())
    }

    #[test]
    fn the_selected_row_scrolls_into_view() {
        assert_eq!(scroll_fraction(0, 1), 0.0);
        assert_eq!(scroll_fraction(0, 20), 0.0);
        assert_eq!(scroll_fraction(19, 20), 1.0);
    }

    #[test]
    fn the_field_narrows_with_the_window() {
        assert_eq!(field_width(1600.0), 280.0);
        assert_eq!(field_width(1240.0), 240.0);
        assert_eq!(field_width(1177.0), 200.0);
        assert_eq!(field_width(960.0), 200.0);
    }

    #[test]
    fn each_standing_has_its_words() {
        assert_eq!(
            standing_text(GameStanding::InLibrary { hours: 42.0 }),
            "In your library · 42 h"
        );
        assert_eq!(
            standing_text(GameStanding::PlayedElsewhere),
            "Played elsewhere"
        );
        assert_eq!(standing_text(GameStanding::WouldTry), "Would try");
        assert_eq!(standing_text(GameStanding::NotThisOne), "Not this one");
        assert_eq!(standing_text(GameStanding::NotForMe), "Not for me");
        assert_eq!(
            standing_text(GameStanding::Later { days_left: 12 }),
            "Later · back in 12 days"
        );
        assert_eq!(
            standing_text(GameStanding::Later { days_left: 1 }),
            "Later · back in 1 day"
        );
    }
}
