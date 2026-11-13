use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gamepalate_app::{AppId, FoundGame, GameStanding, Gamepalate, MIN_SEARCH_CHARS};
use iced::{
    Center, ContentFit, Element, Event, Fill, Size, Subscription, Task, event,
    keyboard::{self, Key, key::Named},
    padding,
    widget::{
        Id, button, column, container, image, mouse_area, opaque, operation,
        operation::RelativeOffset, responsive, row, scrollable, space, stack, text, text_input,
    },
    window,
};

use super::backdrop::{self, Blurred, faded_in};
use crate::{
    font,
    screen::{
        library::{handle, hours},
        showcase::images::{Images, Picture},
    },
    style,
};

const FIELD: Id = Id::new("game-search");
const LIST: Id = Id::new("game-search-list");
const PLACEHOLDER: &str = "Search games";
const HINT: &str = "Type at least 3 letters, an appid or a store link";
const NO_GAMES: &str = "No games found";
pub const TIP: &str = "Search games (Ctrl+K)";
pub const LENS: char = '\u{E8B6}';
/// The panel's top edge, as a share of the height below the top bar.
const TOP_SHARE: f32 = 0.12;
const MAX_WIDTH: f32 = 640.0;
const WIDTH_SHARE: f32 = 0.9;
const BOTTOM_MARGIN: f32 = 24.0;
const FIELD_SIZE: f32 = 18.0;
const NAME_SIZE: f32 = 15.0;
const ROW_HEIGHT: f32 = 58.0;
const CAPSULE_WIDTH: f32 = 120.0;
const CAPSULE_HEIGHT: f32 = CAPSULE_WIDTH * 87.0 / 231.0;
const CAPSULE_RADIUS: f32 = 4.0;

/// The search of the whole catalog: a panel over the page, which keeps its state, with the page
/// blurred behind it.
#[derive(Default)]
pub struct Search {
    text: String,
    found: Vec<FoundGame>,
    selected: Option<usize>,
    state: State,
    backdrop: Option<Backdrop>,
    now: Option<Instant>,
    capsules: Images<image::Allocation, AppId>,
}

#[derive(Clone, Copy, Default)]
enum State {
    #[default]
    Closed,
    /// Waiting for the screenshot of the page, taken before the panel covers it.
    Opening(Instant),
    Open(Timing),
}

/// When each step of the opening happened, for the log.
#[derive(Clone, Copy)]
struct Timing {
    asked: Instant,
    screenshot: Duration,
}

struct Backdrop {
    handle: image::Handle,
    /// The first frame that shows the picture: its fade starts there.
    shown: Option<Instant>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Toggle,
    Input(String),
    Down,
    Up,
    Submit,
    Close,
    Pick(AppId),
    Screenshot(window::Screenshot),
    Blurred(Arc<Blurred>),
    Frame(Instant),
    CapsuleLoaded(AppId, Option<image::Handle>),
    CapsuleAllocated(AppId, Option<image::Allocation>),
}

pub enum Action {
    Run(Task<Message>),
    Open(AppId),
}

impl std::fmt::Debug for Blurred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blurred({:?})", self.took)
    }
}

impl Search {
    /// `window` takes the screenshot of the page; `top` is the height of the top bar, which stays
    /// sharp.
    pub fn update(
        &mut self,
        message: Message,
        gamepalate: &Arc<Gamepalate>,
        window: Option<window::Id>,
        top: f32,
    ) -> Action {
        match message {
            Message::Toggle if self.is_open() => self.close(),
            Message::Toggle => {
                let asked = Instant::now();
                return Action::Run(match window {
                    Some(window) => {
                        self.state = State::Opening(asked);
                        window::screenshot(window).map(Message::Screenshot)
                    }
                    None => self.show(gamepalate, asked, Duration::ZERO),
                });
            }
            Message::Screenshot(screenshot) => {
                let State::Opening(asked) = self.state else {
                    return Action::Run(Task::none());
                };
                let shown = self.show(gamepalate, asked, asked.elapsed());
                let blurred = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || backdrop::blur(&screenshot, top)).await
                    },
                    |blurred| {
                        blurred
                            .ok()
                            .map(|blurred| Message::Blurred(Arc::new(blurred)))
                    },
                )
                .and_then(Task::done);
                return Action::Run(Task::batch([shown, blurred]));
            }
            Message::Blurred(blurred) => {
                if let State::Open(timing) = self.state {
                    tracing::info!(
                        "game search: screenshot {} ms, blur {} ms",
                        timing.screenshot.as_millis(),
                        blurred.took.as_millis()
                    );
                    self.backdrop = Some(Backdrop {
                        handle: blurred.handle.clone(),
                        shown: None,
                    });
                }
            }
            Message::Frame(now) => {
                self.now = Some(now);
                if let (State::Open(timing), Some(backdrop)) = (self.state, &mut self.backdrop)
                    && backdrop.shown.is_none()
                {
                    backdrop.shown = Some(now);
                    tracing::info!(
                        "game search: blurred page shown {} ms after opening",
                        now.saturating_duration_since(timing.asked).as_millis()
                    );
                }
            }
            Message::Input(text) => {
                self.found = gamepalate.find_games(&text);
                self.text = text;
                self.selected = None;
                return Action::Run(Task::batch([
                    operation::snap_to(LIST, RelativeOffset::START),
                    self.load_capsules(gamepalate),
                ]));
            }
            Message::Down => {
                return Action::Run(self.select(below(self.selected, self.found.len())));
            }
            Message::Up => return Action::Run(self.select(above(self.selected))),
            Message::Submit => {
                if let Some(id) = self.chosen() {
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

    /// While the panel is open, the keys are its own and not the page's.
    pub fn is_open(&self) -> bool {
        !matches!(self.state, State::Closed)
    }

    pub fn close(&mut self) {
        self.state = State::Closed;
        self.selected = None;
        self.backdrop = None;
    }

    /// The picture of the page no longer fits the window: the veil stays alone.
    pub fn resized(&mut self) {
        self.backdrop = None;
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if !self.is_open() {
            return Subscription::none();
        }
        let fading = self.backdrop.as_ref().is_some_and(|backdrop| {
            backdrop
                .shown
                .zip(self.now)
                .is_none_or(|(shown, now)| faded_in(shown, now) < 1.0)
        });

        Subscription::batch([
            event::listen_with(panel_key),
            if fading {
                window::frames().map(Message::Frame)
            } else {
                Subscription::none()
            },
        ])
    }

    /// The panel over the page, or nothing while the screenshot is on its way.
    pub fn view(&self) -> Option<Element<'_, Message>> {
        if !matches!(self.state, State::Open(_)) {
            return None;
        }
        let picture = self.backdrop.as_ref().map(|backdrop| {
            let opacity = backdrop
                .shown
                .zip(self.now)
                .map_or(0.0, |(shown, now)| faded_in(shown, now));
            image(backdrop.handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Fill)
                .opacity(opacity)
        });
        let veil = opaque(
            mouse_area(
                container(space())
                    .width(Fill)
                    .height(Fill)
                    .style(style::search_veil),
            )
            .on_press(Message::Close),
        );

        Some(
            stack![]
                .push(picture)
                .push(veil)
                .push(responsive(move |size| self.placed(size)))
                .into(),
        )
    }

    fn placed(&self, size: Size) -> Element<'_, Message> {
        let (top, width) = panel_place(size);

        container(opaque(
            container(self.panel(size.height - top - BOTTOM_MARGIN))
                .width(width)
                .style(style::popup),
        ))
        .padding(padding::top(top))
        .center_x(Fill)
        .into()
    }

    fn panel(&self, max_height: f32) -> Element<'_, Message> {
        let field = row![
            text(LENS)
                .font(font::ICONS)
                .size(font::icon(14.0))
                .style(style::muted),
            text_input(PLACEHOLDER, &self.text)
                .id(FIELD)
                .on_input(Message::Input)
                .on_submit(Message::Submit)
                .style(style::bare_field)
                .size(FIELD_SIZE)
                .padding([12, 0])
                .width(Fill),
            keycap("Esc"),
        ]
        .spacing(10)
        .padding([0, 16])
        .align_y(Center);

        let body: Element<'_, Message> = if self.found.is_empty() {
            let note = if self.text.trim().chars().count() < MIN_SEARCH_CHARS {
                HINT
            } else {
                NO_GAMES
            };
            container(text(note).size(font::NOTE_SIZE).style(style::muted))
                .padding([12, 16])
                .into()
        } else {
            scrollable(
                column(
                    self.found
                        .iter()
                        .enumerate()
                        .map(|(index, game)| self.row(index, game)),
                )
                .padding(6),
            )
            .id(LIST)
            .into()
        };

        container(column![field, style::separator(), body])
            .max_height(max_height)
            .into()
    }

    fn row<'a>(&'a self, index: usize, game: &'a FoundGame) -> Element<'a, Message> {
        let selected = self.selected == Some(index);
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
        let words = column![text(&game.name).size(NAME_SIZE)]
            .push(game.standing.map(|standing| {
                text(standing_text(standing))
                    .size(font::NOTE_SIZE)
                    .style(style::muted)
            }))
            .spacing(2)
            .width(Fill);

        button(
            row![picture, words]
                .push(selected.then(|| keycap("Enter")))
                .spacing(12)
                .align_y(Center),
        )
        .padding([6, 10])
        .width(Fill)
        .height(ROW_HEIGHT)
        .style(style::list_row(selected))
        .on_press(Message::Pick(game.id))
        .into()
    }

    /// The panel opens with the last text selected, so typing replaces it.
    fn show(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        asked: Instant,
        screenshot: Duration,
    ) -> Task<Message> {
        self.state = State::Open(Timing { asked, screenshot });
        self.found = gamepalate.find_games(&self.text);
        self.selected = None;
        self.backdrop = None;

        Task::batch([
            operation::focus(FIELD),
            operation::select_all(FIELD),
            self.load_capsules(gamepalate),
        ])
    }

    fn select(&mut self, selected: Option<usize>) -> Task<Message> {
        if self.found.is_empty() {
            return Task::none();
        }
        self.selected = selected;
        let Some(index) = selected else {
            return Task::none();
        };

        operation::snap_to(
            LIST,
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

fn keycap<'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(font::NOTE_SIZE - 1.0))
        .padding([1, 6])
        .style(style::muted_keycap)
        .into()
}

/// The panel's top edge and width in the area below the top bar.
fn panel_place(area: Size) -> (f32, f32) {
    (
        (area.height * TOP_SHARE).round(),
        (area.width * WIDTH_SHARE).min(MAX_WIDTH),
    )
}

fn panel_key(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
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

// Snapping in proportion to the row always shows the whole row, whatever the list's height.
fn scroll_fraction(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.0
    } else {
        index as f32 / (count - 1) as f32
    }
}

pub(super) fn standing_text(standing: GameStanding) -> String {
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
            state: State::Open(Timing {
                asked: Instant::now(),
                screenshot: Duration::ZERO,
            }),
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
    fn down_moves_the_selection() {
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
        assert_eq!(open_with(0).chosen(), None);
    }

    #[test]
    fn opening_a_game_closes_the_panel_and_empties_the_field() {
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
    fn closing_keeps_the_text_and_drops_the_picture() {
        let mut search = open_with(3);
        search.backdrop = Some(Backdrop {
            handle: image::Handle::from_rgba(1, 1, vec![0; 4]),
            shown: None,
        });

        search.close();

        assert!(!search.is_open());
        assert!(search.backdrop.is_none());
        assert_eq!(search.text, "game");
    }

    #[test]
    fn the_panel_waits_for_the_screenshot() {
        let mut search = Search {
            state: State::Opening(Instant::now()),
            ..Search::default()
        };

        assert!(search.is_open());
        assert!(search.view().is_none());

        search.state = State::Closed;
        assert!(!search.is_open());
        assert!(search.view().is_none());
        assert!(open_with(1).view().is_some());
    }

    #[test]
    fn the_arrows_and_escape_belong_to_the_panel() {
        assert!(matches!(
            panel_key_of(Named::ArrowDown),
            Some(Message::Down)
        ));
        assert!(matches!(panel_key_of(Named::ArrowUp), Some(Message::Up)));
        assert!(matches!(panel_key_of(Named::Escape), Some(Message::Close)));
        assert!(panel_key_of(Named::Space).is_none());
    }

    fn panel_key_of(key: Named) -> Option<Message> {
        panel_key(pressed(key), event::Status::Captured, window::Id::unique())
    }

    #[test]
    fn the_selected_row_scrolls_into_view() {
        assert_eq!(scroll_fraction(0, 1), 0.0);
        assert_eq!(scroll_fraction(0, 8), 0.0);
        assert_eq!(scroll_fraction(7, 8), 1.0);
    }

    #[test]
    fn the_panel_sits_in_the_upper_part_and_narrows_with_the_window() {
        assert_eq!(panel_place(Size::new(1600.0, 900.0)), (108.0, 640.0));
        assert_eq!(panel_place(Size::new(600.0, 500.0)), (60.0, 540.0));
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
