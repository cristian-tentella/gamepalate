mod browse;

use std::{sync::Arc, time::Instant};

use gamepalate_app::{AppId, FoundGame, GameStanding, Gamepalate, MIN_SEARCH_CHARS};
use iced::{
    Center, ContentFit, Element, Event, Fill, Size, Subscription, Task, event,
    keyboard::{self, Key, key::Named},
    padding,
    widget::{
        Id, button, column, container, image, mouse_area, opaque, operation,
        operation::RelativeOffset, row, scrollable, space, stack, text, text_input, tooltip,
    },
    window,
};

use super::{
    backdrop::{self, Blurred, faded_in},
    game_page::sheet_size,
};
use crate::{
    font,
    screen::{
        library::{handle, hours},
        showcase::images::{Images, Picture},
    },
    style,
};
use browse::Browse;

const FIELD: Id = Id::new("game-search");
const LIST: Id = Id::new("game-search-list");
const PLACEHOLDER: &str = "Search games";
const BROWSE_PLACEHOLDER: &str = "Filter by name";
const HINT: &str = "Type at least 3 letters, an appid or a store link";
const NO_GAMES: &str = "No games found";
const BROWSE_KEYS: &str = "PgUp PgDn · Ctrl+Home Ctrl+End";
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
    /// Typed since the panel opened: the late focus leaves the field alone.
    typed: bool,
    backdrop: Option<Backdrop>,
    now: Option<Instant>,
    capsules: Images<image::Allocation, AppId>,
    mode: Mode,
    browse: Browse,
}

/// The panel finds a game by name, or browses the whole catalog in a table. Tab goes from one to
/// the other with the same text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Find,
    Browse,
}

#[derive(Clone, Copy, Default)]
enum State {
    #[default]
    Closed,
    /// Waiting for the screenshot of the page, taken before the panel covers it.
    Opening,
    Open,
}

struct Backdrop {
    handle: image::Handle,
    /// The first frame that shows the picture: its fade starts there.
    shown: Option<Instant>,
}

/// A picture that is already there: no fade is left at `now`.
fn faded_in_already(handle: image::Handle, now: Instant) -> Backdrop {
    Backdrop {
        handle,
        shown: Some(now.checked_sub(backdrop::FADE).unwrap_or(now)),
    }
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
    SwitchMode,
    PageDown,
    PageUp,
    First,
    Last,
    Browse(browse::Message),
}

pub enum Action {
    Run(Task<Message>),
    /// A game to open, with the blurred page when it has come: the game's sheet lies on it.
    /// `from_browse` says the game was picked in Browse, which the sheet goes back to.
    Open {
        id: AppId,
        backdrop: Option<image::Handle>,
        from_browse: bool,
    },
}

impl std::fmt::Debug for Blurred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Blurred")
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
                return Action::Run(match window {
                    Some(window) => {
                        self.state = State::Opening;
                        window::screenshot(window).map(Message::Screenshot)
                    }
                    None => self.show(gamepalate),
                });
            }
            Message::Screenshot(screenshot) => {
                if !matches!(self.state, State::Opening) {
                    return Action::Run(Task::none());
                }
                let shown = self.show(gamepalate);
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
            // The blurred page comes a few milliseconds after the panel: by then the field is
            // surely in the window, so it gets the focus again unless typing has begun.
            Message::Blurred(blurred) => {
                if matches!(self.state, State::Open) {
                    self.backdrop = Some(Backdrop {
                        handle: blurred.handle.clone(),
                        shown: None,
                    });
                    if !self.typed {
                        return Action::Run(focus());
                    }
                }
            }
            Message::Frame(now) => {
                self.now = Some(now);
                if let Some(backdrop) = &mut self.backdrop {
                    backdrop.shown.get_or_insert(now);
                }
            }
            Message::Input(text) if self.mode == Mode::Browse => {
                self.typed = true;
                self.text = text;
                return Action::Run(self.browse.relist(gamepalate, &self.text));
            }
            Message::Input(text) => {
                self.typed = true;
                self.found = gamepalate.find_games(&text);
                self.text = text;
                self.selected = None;
                return Action::Run(Task::batch([
                    operation::snap_to(LIST, RelativeOffset::START),
                    self.load_capsules(gamepalate),
                ]));
            }
            // The popup of the filters has the keys while it is open.
            Message::Down
            | Message::Up
            | Message::PageDown
            | Message::PageUp
            | Message::First
            | Message::Last
            | Message::Submit
            | Message::SwitchMode
                if self.filters_open() => {}
            // The pages and the ends of the list are Browse's
            Message::PageDown | Message::PageUp | Message::First | Message::Last
                if !self.browsing() => {}
            Message::PageDown => return Action::Run(self.browse.page_down()),
            Message::PageUp => return Action::Run(self.browse.page_up()),
            Message::First => return Action::Run(self.browse.first()),
            Message::Last => return Action::Run(self.browse.last()),
            Message::Down if self.mode == Mode::Browse => return Action::Run(self.browse.down()),
            Message::Up if self.mode == Mode::Browse => return Action::Run(self.browse.up()),
            Message::Down => {
                return Action::Run(self.select(below(self.selected, self.found.len())));
            }
            Message::Up => return Action::Run(self.select(above(self.selected))),
            Message::Submit => {
                // With no row, Enter takes the text for an appid or a store link, as Search does
                let chosen = match self.mode {
                    Mode::Find => self.chosen(),
                    Mode::Browse => self
                        .browse
                        .chosen()
                        .or_else(|| gamepalate.game_in(&self.text)),
                };
                if let Some(id) = chosen {
                    return self.pick(id);
                }
            }
            Message::SwitchMode => {
                let task = match self.switch() {
                    Mode::Find => {
                        self.found = gamepalate.find_games(&self.text);
                        Task::batch([
                            operation::snap_to(LIST, RelativeOffset::START),
                            self.load_capsules(gamepalate),
                        ])
                    }
                    Mode::Browse => self.browse.enter(gamepalate, &self.text),
                };
                return Action::Run(Task::batch([task, operation::focus(FIELD)]));
            }
            Message::Browse(browse::Message::ToggleFilters) if !self.browsing() => {}
            Message::Browse(message) => {
                return Action::Run(self.browse.update(message, gamepalate, &self.text));
            }
            Message::Close => self.escape(),
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

    /// A game picked in Browse changed: its row has the new standing, and the list is still the
    /// one of the facade now.
    pub fn game_changed(&mut self, gamepalate: &Gamepalate, id: AppId) {
        self.browse.game_changed(gamepalate, id);
    }

    /// The facade is another one: the list of Browse is made again the next time it opens.
    pub fn use_gamepalate(&mut self) {
        self.browse.mark_stale();
    }

    pub fn close(&mut self) {
        self.state = State::Closed;
        self.selected = None;
        self.backdrop = None;
        self.browse.close_filters();
    }

    /// Esc closes the popup of the filters first, and only then the panel.
    fn escape(&mut self) {
        if self.filters_open() {
            self.browse.close_filters();
        } else {
            self.close();
        }
    }

    fn browsing(&self) -> bool {
        matches!(self.state, State::Open) && self.mode == Mode::Browse
    }

    fn filters_open(&self) -> bool {
        self.browsing() && self.browse.filters_open()
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
    /// `area` is the size below the top bar.
    pub fn view(&self, area: Size) -> Option<Element<'_, Message>> {
        if !matches!(self.state, State::Open) {
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
                    .style(style::frosted_veil),
            )
            .on_press(Message::Close),
        );

        Some(
            stack![]
                .push(picture)
                .push(veil)
                .push(self.placed(area))
                .into(),
        )
    }

    fn placed(&self, size: Size) -> Element<'_, Message> {
        if self.mode == Mode::Browse {
            let sheet = sheet_size(size);
            return container(opaque(
                container(self.browse.view(self.field(), sheet)).style(style::popup),
            ))
            .center(Fill)
            .into();
        }
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

    /// The field, with the way to the other mode and Esc on its right.
    fn field(&self) -> Element<'_, Message> {
        let (placeholder, other, tip) = match self.mode {
            Mode::Find => (PLACEHOLDER, "Browse", "Browse the catalog (Tab)"),
            Mode::Browse => (BROWSE_PLACEHOLDER, "Search", "Search by name (Tab)"),
        };
        let keys = (self.mode == Mode::Browse)
            .then(|| text(BROWSE_KEYS).size(font::NOTE_SIZE).style(style::muted));
        let switch = button(
            row![text(other).size(font::NOTE_SIZE), keycap("Tab")]
                .spacing(6)
                .align_y(Center),
        )
        .padding([4, 8])
        .style(style::text_button)
        .on_press(Message::SwitchMode);
        let close = button(
            row![text("Close").size(font::NOTE_SIZE), keycap("Esc")]
                .spacing(6)
                .align_y(Center),
        )
        .padding([4, 8])
        .style(style::text_button)
        .on_press(Message::Close);

        row![
            text(LENS)
                .font(font::ICONS)
                .size(font::icon(14.0))
                .style(style::muted),
            text_input(placeholder, &self.text)
                .id(FIELD)
                .on_input(Message::Input)
                .on_submit(Message::Submit)
                .style(style::bare_field)
                .size(FIELD_SIZE)
                .padding([12, 0])
                .width(Fill),
            tooltip(
                switch,
                container(
                    column![text(tip).size(font::NOTE_SIZE)]
                        .push(keys)
                        .spacing(2)
                )
                .padding([6, 10])
                .style(style::popup),
                tooltip::Position::Bottom,
            ),
            close,
        ]
        .spacing(10)
        .padding([0, 16])
        .align_y(Center)
        .into()
    }

    fn panel(&self, max_height: f32) -> Element<'_, Message> {
        let field = self.field();

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

    /// The panel opens on Search with the last text selected, so typing replaces it. Browse
    /// keeps its state for Tab, and for the way back from a game's page.
    fn show(&mut self, gamepalate: &Arc<Gamepalate>) -> Task<Message> {
        self.open_state(false);
        self.found = gamepalate.find_games(&self.text);

        Task::batch([focus(), self.load_capsules(gamepalate)])
    }

    /// The panel opens again on Browse with its list as it was, after the page of a game that
    /// came from it. The page already holds the blurred picture of what is under the panel, and
    /// it shows at once: nothing but the table and that picture is seen in between. A screenshot
    /// would catch the page under before it is drawn without the sheet.
    pub fn reopen(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        blurred: Option<image::Handle>,
    ) -> Task<Message> {
        if self.is_open() || self.mode != Mode::Browse {
            return Task::none();
        }
        let now = Instant::now();
        self.open_state(true);
        self.backdrop = blurred.map(|handle| faded_in_already(handle, now));
        self.now = Some(now);

        Task::batch([focus(), self.browse.come_back(gamepalate)])
    }

    /// Open, on Search or, coming back, on Browse: its list is not made again.
    fn open_state(&mut self, returning: bool) {
        self.state = State::Open;
        self.typed = false;
        self.selected = None;
        self.backdrop = None;
        if !returning {
            self.mode = Mode::Find;
        }
    }

    /// To the other mode, with the same text.
    fn switch(&mut self) -> Mode {
        self.selected = None;
        self.browse.close_filters();
        self.mode = match self.mode {
            Mode::Find => Mode::Browse,
            Mode::Browse => Mode::Find,
        };
        self.mode
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

    /// Search empties its field. Browse keeps everything, to come back to it.
    fn pick(&mut self, id: AppId) -> Action {
        let from_browse = self.browsing();
        let backdrop = self.backdrop.take().map(|backdrop| backdrop.handle);
        self.close();
        if from_browse {
            self.browse.picked(id);
        } else {
            self.text.clear();
            self.found.clear();
        }
        Action::Open {
            id,
            backdrop,
            from_browse,
        }
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

fn focus() -> Task<Message> {
    Task::batch([operation::focus(FIELD), operation::select_all(FIELD)])
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
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };

    match key.as_ref() {
        Key::Named(Named::ArrowDown) => Some(Message::Down),
        Key::Named(Named::ArrowUp) => Some(Message::Up),
        Key::Named(Named::Escape) => Some(Message::Close),
        Key::Named(Named::Tab) => Some(Message::SwitchMode),
        Key::Named(Named::PageDown) => Some(Message::PageDown),
        Key::Named(Named::PageUp) => Some(Message::PageUp),
        // Home and End alone are for the field
        Key::Named(Named::Home) if modifiers.command() => Some(Message::First),
        Key::Named(Named::End) if modifiers.command() => Some(Message::Last),
        // F alone is for the field
        Key::Character("f" | "F") if modifiers.command() => {
            Some(Message::Browse(browse::Message::ToggleFilters))
        }
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
        GameStanding::NotForMe => "Not for me".to_string(),
        GameStanding::Later { days_left: 1 } => "Later · back in 1 day".to_string(),
        GameStanding::Later { days_left } => format!("Later · back in {days_left} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Size = Size::new(1280.0, 755.0);

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
            state: State::Open,
            ..Search::default()
        }
    }

    fn pressed(key: Named) -> Event {
        pressed_with(key, keyboard::Modifiers::empty())
    }

    fn pressed_with(key: Named, modifiers: keyboard::Modifiers) -> Event {
        let key = Key::Named(key);
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
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

        let Action::Open {
            id, from_browse, ..
        } = search.pick(AppId(2))
        else {
            panic!("picking a game should open it");
        };

        assert_eq!(id, AppId(2));
        assert!(!from_browse);
        assert!(!search.is_open());
        assert!(search.text.is_empty());
        assert_eq!(search.selected, None);
    }

    fn browsing_with(count: u32) -> Search {
        let mut search = open_with(3);
        search.switch();
        search.browse = Browse::with_rows(count);
        search
    }

    #[test]
    fn opening_a_game_in_browse_keeps_the_text_the_rows_and_the_place() {
        let mut search = browsing_with(50);
        search.text = "dark".to_string();
        let _ = search.browse.page_down();
        let before = search.browse.state();

        let Action::Open {
            id, from_browse, ..
        } = search.pick(AppId(9))
        else {
            panic!("picking a game should open it");
        };

        assert_eq!(id, AppId(9));
        assert!(from_browse);
        assert!(!search.is_open());
        assert_eq!(search.mode, Mode::Browse);
        assert_eq!(search.text, "dark");
        let (rows, len, selected, offset) = search.browse.state();
        assert_eq!((rows, len, offset), (before.0, before.1, before.3));
        assert_eq!(selected, Some(8), "the row of the game is the selected one");
    }

    #[test]
    fn coming_back_opens_on_browse_with_the_list_as_it_was() {
        let mut search = browsing_with(50);
        let _ = search.browse.last();
        let _ = search.pick(AppId(50));
        let before = search.browse.state();

        search.open_state(true);

        assert!(search.is_open());
        assert_eq!(search.mode, Mode::Browse);
        assert_eq!(search.browse.state(), before, "the list was not made again");
    }

    #[test]
    fn the_picture_kept_by_the_page_is_there_at_once() {
        let now = Instant::now();
        let handle = image::Handle::from_rgba(1, 1, vec![0; 4]);

        let backdrop = faded_in_already(handle, now);

        assert_eq!(faded_in(backdrop.shown.expect("it was shown"), now), 1.0);
    }

    #[test]
    fn ctrl_k_opens_on_search_even_after_browse() {
        let mut search = browsing_with(50);
        let _ = search.pick(AppId(5));
        let before = search.browse.state();

        search.open_state(false);

        assert_eq!(search.mode, Mode::Find);
        assert_eq!(
            search.browse.state(),
            before,
            "Browse keeps its state for Tab"
        );
    }

    #[test]
    fn the_pages_and_the_ends_belong_to_the_panel_and_home_alone_to_the_field() {
        let command = keyboard::Modifiers::COMMAND;
        let heard = |key, modifiers| {
            panel_key(
                pressed_with(key, modifiers),
                event::Status::Ignored,
                window::Id::unique(),
            )
        };

        assert!(matches!(
            heard(Named::PageDown, keyboard::Modifiers::empty()),
            Some(Message::PageDown)
        ));
        assert!(matches!(
            heard(Named::PageUp, keyboard::Modifiers::empty()),
            Some(Message::PageUp)
        ));
        assert!(matches!(heard(Named::Home, command), Some(Message::First)));
        assert!(matches!(heard(Named::End, command), Some(Message::Last)));
        assert!(heard(Named::Home, keyboard::Modifiers::empty()).is_none());
        assert!(heard(Named::End, keyboard::Modifiers::empty()).is_none());
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
            state: State::Opening,
            ..Search::default()
        };

        assert!(search.is_open());
        assert!(search.view(AREA).is_none());

        search.state = State::Closed;
        assert!(!search.is_open());
        assert!(search.view(AREA).is_none());
        assert!(open_with(1).view(AREA).is_some());
    }

    #[test]
    fn the_arrows_and_escape_belong_to_the_panel() {
        assert!(matches!(
            panel_key_of(Named::ArrowDown),
            Some(Message::Down)
        ));
        assert!(matches!(panel_key_of(Named::ArrowUp), Some(Message::Up)));
        assert!(matches!(panel_key_of(Named::Escape), Some(Message::Close)));
        assert!(matches!(
            panel_key_of(Named::Tab),
            Some(Message::SwitchMode)
        ));
        assert!(panel_key_of(Named::Space).is_none());
    }

    #[test]
    fn tab_goes_to_browse_and_back_with_the_same_text() {
        let mut search = open_with(3);
        search.selected = Some(1);

        assert_eq!(search.switch(), Mode::Browse);
        assert_eq!(search.text, "game");
        assert_eq!(search.selected, None);
        assert!(search.view(AREA).is_some());

        assert_eq!(search.switch(), Mode::Find);
        assert_eq!(search.text, "game");
    }

    #[test]
    fn escape_closes_browse_and_keeps_its_state() {
        let mut search = open_with(3);
        search.switch();

        search.close();

        assert!(!search.is_open());
        assert_eq!(search.mode, Mode::Browse);
        assert!(search.view(AREA).is_none());
    }

    #[test]
    fn escape_closes_the_popup_of_the_filters_before_the_panel() {
        let mut search = open_with(3);
        search.switch();
        search.browse = Browse::with_filters_open();
        assert!(search.filters_open());

        search.escape();

        assert!(search.is_open());
        assert!(!search.filters_open());
        search.escape();
        assert!(!search.is_open());
    }

    #[test]
    fn closing_the_panel_closes_the_popup_too() {
        let mut search = open_with(3);
        search.switch();
        search.browse = Browse::with_filters_open();

        search.close();

        assert!(!search.browse.filters_open());
    }

    #[test]
    fn search_has_no_popup_of_filters() {
        let mut search = open_with(3);
        search.browse = Browse::with_filters_open();

        assert!(!search.browsing());
        assert!(!search.filters_open());
        search.escape();
        assert!(!search.is_open());
    }

    #[test]
    fn control_f_opens_the_filters_and_f_alone_does_not() {
        let heard = |modifiers| {
            let key = Key::Character("f".into());
            let event = Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key,
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            });
            panel_key(event, event::Status::Ignored, window::Id::unique())
        };

        assert!(matches!(
            heard(keyboard::Modifiers::COMMAND),
            Some(Message::Browse(browse::Message::ToggleFilters))
        ));
        assert!(heard(keyboard::Modifiers::empty()).is_none());
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
