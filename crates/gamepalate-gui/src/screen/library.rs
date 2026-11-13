mod filter;
mod grid;
mod popup;

use std::{ops::Range, path::PathBuf, sync::Arc, time::Duration};

use gamepalate_app::{
    AppId, Capsule, CatalogMatch, FamiliesChanged, FamiliesUndo, FamilyChange, GameCorrection,
    GameTagsView, Gamepalate, LibraryGameView, LibraryView,
};
use iced::{
    Center, Element, Fill, Size, Subscription, Task,
    keyboard::{self, Key, key::Named},
    widget::{
        Id, button, column, container, image, mouse_area, opaque, operation, pick_list, responsive,
        row,
        scrollable::{AbsoluteOffset, Viewport},
        space, stack, text, text_input, tooltip,
    },
    window,
};

use super::{
    Toasts, sentence,
    showcase::{SCROLL_STEP, images::Images, scroll_sheet, sheet, tags_body},
    steam_client::{ElsewhereHint, SteamClientQuestion},
};
use crate::{font, format, style};
use filter::{FILTERS, SORTS, count, empty_text, filter_label, shown, shows_filter};
pub use filter::{Filter, Sort};
use grid::{GRID_PADDING, Row, columns, moved, reveal_offset, rows, visible_rows, wanted_rows};
use popup::{changed_game, corrected_text, families_text};

const GRID: Id = Id::new("library-grid");
const SEARCH: Id = Id::new("library-search");
const NEW_FAMILY: Id = Id::new("library-new-family");
const SETTLE: Duration = Duration::from_millis(300);
const SEARCH_WIDTH: f32 = 300.0;
const SEARCH_PLACEHOLDER: &str = "Search by name (Ctrl+F)";
const ADD_PLACEHOLDER: &str = "Search Steam for a game you played elsewhere";
const SCREENS_KEPT: f32 = 2.0;
const CHROME_HEIGHT: f32 = 110.0;
const DEFAULT_AREA: Size = Size::new(1280.0, 700.0);
const NOT_OWNED_TIP: &str =
    "Played but not in your library: Family Sharing, free weekends, removed games";

pub struct Library {
    gamepalate: Arc<Gamepalate>,
    data_dir: PathBuf,
    view: LibraryView,
    heaviest: f64,
    question: Option<SteamClientQuestion>,
    open: Option<AppId>,
    tags: Option<(AppId, Option<GameTagsView>)>,
    tags_popup: TagsPopup,
    selected: Option<AppId>,
    reveal_pending: bool,
    undo: Option<Undo>,
    new_family: Option<String>,
    toasts: Toasts,
    filter: Filter,
    sort: Sort,
    search: String,
    adding: bool,
    shown: Vec<usize>,
    found: Found,
    offset: f32,
    upward: bool,
    area: Size,
    capsules: Images<image::Allocation, AppId>,
}

#[derive(Debug, Default)]
struct TagsPopup {
    open: bool,
    all: bool,
}

#[derive(Debug, PartialEq)]
enum Over {
    Scroll(f32),
    Ignore,
    Through,
}

impl TagsPopup {
    fn is_open(&self) -> bool {
        self.open
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn toggle(&mut self, available: bool) {
        self.open = !self.open && available;
    }

    fn over(&self, message: &Message) -> Over {
        match message {
            Message::Move(Move::Up) | Message::ScrollUp => Over::Scroll(-SCROLL_STEP),
            Message::Move(Move::Down) | Message::ScrollDown => Over::Scroll(SCROLL_STEP),
            Message::Move(_) | Message::OpenSelected | Message::Pick(_) | Message::Undo => {
                Over::Ignore
            }
            _ => Over::Through,
        }
    }
}

enum Undo {
    Correction(AppId, GameCorrection),
    Families(FamiliesUndo),
    Elsewhere(AppId, bool),
}

#[derive(Debug, Default)]
struct Found {
    games: Vec<CatalogMatch>,
    settled: bool,
}

impl Found {
    fn searched(&mut self, games: Vec<CatalogMatch>) {
        self.games = games;
        self.settled = false;
    }

    fn waited(&mut self, waited: &str, search: &str) {
        if waited == search {
            self.settled = true;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Enter {
    Open,
    Add,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug)]
pub enum Message {
    Filter(Filter),
    Sort(Sort),
    Search(String),
    Settled(String),
    AddGame,
    PlayElsewhere(AppId, String, bool),
    PlayedElsewhere(AppId, String, bool, Result<bool, String>),
    Scrolled(Viewport),
    WindowResized(Size),
    CapsuleLoaded(AppId, Option<image::Handle>),
    CapsuleAllocated(AppId, Option<image::Allocation>),
    Open(AppId),
    Close,
    ToggleTags,
    ToggleAllTags,
    CloseTags,
    ScrollUp,
    ScrollDown,
    Move(Move),
    OpenSelected,
    Pick(usize),
    FocusSearch,
    Correct(AppId, GameCorrection),
    Corrected(AppId, GameCorrection, Result<GameCorrection, String>),
    Undo,
    Undone(Result<(), String>),
    HideToast(u64),
    ChangeFamilies(FamilyChange),
    FamiliesSaved(FamilyChange, Result<FamiliesChanged, String>),
    NewFamily,
    NewFamilyName(String),
    CreateFamily,
    OpenFamilies,
    SteamClient(bool),
    SteamClientAnswered(Result<(), String>),
}

impl Library {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Library, Task<Message>) {
        Library::filtered(gamepalate, Filter::All, Sort::TasteWeight)
    }

    pub fn filtered(
        gamepalate: &Arc<Gamepalate>,
        filter: Filter,
        sort: Sort,
    ) -> (Library, Task<Message>) {
        let mut library = Library {
            gamepalate: Arc::clone(gamepalate),
            data_dir: gamepalate.data_dir().to_path_buf(),
            view: gamepalate.library_view(),
            heaviest: 0.0,
            question: SteamClientQuestion::ask(gamepalate.data_dir()),
            open: None,
            tags: None,
            tags_popup: TagsPopup::default(),
            selected: None,
            reveal_pending: false,
            undo: None,
            new_family: None,
            toasts: Toasts::default(),
            filter,
            sort,
            search: String::new(),
            adding: false,
            shown: Vec::new(),
            found: Found::default(),
            offset: 0.0,
            upward: false,
            area: DEFAULT_AREA,
            capsules: Images::default(),
        };
        library.heaviest = heaviest(&library.view.games);
        library.shown = shown(&library.view.games, library.filter, library.sort, "");
        let window_size = window::latest()
            .and_then(window::size)
            .map(Message::WindowResized);
        let task = library.load();

        (library, Task::batch([window_size, task]))
    }

    pub fn showing(gamepalate: &Arc<Gamepalate>, id: AppId) -> (Library, Task<Message>) {
        let (mut library, task) = Library::new(gamepalate);
        if library.game(id).is_some() {
            library.open = Some(id);
            library.selected = Some(id);
            library.reveal_pending = true;
        }

        (library, task)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.sync_tags();

        Task::batch([task, self.load()])
    }

    fn sync_tags(&mut self) {
        let Some(id) = self.open else {
            self.tags_popup.close();
            return;
        };
        if self.tags.as_ref().is_none_or(|(shown, _)| *shown != id) {
            self.tags = Some((id, self.gamepalate.game_tags(id)));
        }
    }

    fn game_tags(&self) -> Option<&GameTagsView> {
        self.tags.as_ref().and_then(|(_, view)| view.as_ref())
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        if self.tags_popup.is_open() {
            match self.tags_popup.over(&message) {
                Over::Scroll(y) => return scroll_sheet(y),
                Over::Ignore => return Task::none(),
                Over::Through => {}
            }
        }

        match message {
            Message::ScrollUp | Message::ScrollDown => {}
            Message::ToggleTags => {
                self.sync_tags();
                self.tags_popup.toggle(self.game_tags().is_some());
            }
            Message::ToggleAllTags => self.tags_popup.all = !self.tags_popup.all,
            Message::CloseTags => self.tags_popup.close(),
            Message::Close if self.tags_popup.is_open() => self.tags_popup.close(),
            Message::Filter(filter) => {
                self.filter = filter;
                self.search.clear();
                self.adding = false;
                self.found.searched(Vec::new());
                return self.refresh();
            }
            Message::Sort(sort) => {
                self.sort = sort;
                return self.refresh();
            }
            Message::Search(search) => {
                self.found.searched(self.gamepalate.search_catalog(&search));
                self.search = search.clone();
                let settle = if self.found.games.is_empty() {
                    Task::none()
                } else {
                    Task::perform(tokio::time::sleep(SETTLE), move |_| {
                        Message::Settled(search)
                    })
                };
                return Task::batch([self.refresh(), settle]);
            }
            Message::Settled(search) => self.found.waited(&search, &self.search),
            Message::AddGame => {
                self.adding = true;
                return operation::focus(SEARCH);
            }
            Message::PlayElsewhere(id, name, played) => {
                let gamepalate = Arc::clone(&self.gamepalate);
                return Task::perform(
                    async move {
                        gamepalate
                            .set_played_elsewhere(id, played)
                            .map_err(|error| sentence(&error.to_string()))
                    },
                    move |result| Message::PlayedElsewhere(id, name, played, result),
                );
            }
            Message::PlayedElsewhere(id, name, played, Ok(before)) => {
                if before != played {
                    self.undo = Some(Undo::Elsewhere(id, before));
                }
                if !played && self.open == Some(id) {
                    self.open = None;
                    self.new_family = None;
                }
                self.adding = false;
                self.read_view();
                return self.show_toast(format::elsewhere_text(&name, played), before != played);
            }
            Message::Scrolled(viewport) => {
                let offset = viewport.absolute_offset().y;
                if offset != self.offset {
                    self.upward = offset < self.offset;
                }
                self.offset = offset;
                self.area = viewport.bounds().size();
            }
            Message::WindowResized(size) => {
                self.area = Size::new(size.width, (size.height - CHROME_HEIGHT).max(0.0));
                if std::mem::take(&mut self.reveal_pending) {
                    return self.reveal_selected();
                }
            }
            Message::CapsuleLoaded(id, None) => self.capsules.loaded(id, None),
            Message::CapsuleLoaded(id, Some(handle)) => {
                return image::allocate(handle)
                    .map(move |allocation| Message::CapsuleAllocated(id, allocation.ok()));
            }
            Message::CapsuleAllocated(id, allocation) => self.capsules.loaded(id, allocation),
            Message::Open(id) => {
                self.open = Some(id);
                self.tags_popup.close();
                self.selected = Some(id);
                self.new_family = None;
                self.adding = false;
            }
            Message::Close => {
                self.open = None;
                self.new_family = None;
                self.adding = false;
            }
            Message::Move(direction) => {
                let rows = self.rows(self.area.width);
                let position = self.selected_position();
                let Some(next) = moved(&rows, position, direction) else {
                    return Task::none();
                };
                let Some(id) = self.id_at(next) else {
                    return Task::none();
                };
                self.selected = Some(id);
                if self.open.is_some() {
                    self.open = (next < self.shown.len()).then_some(id);
                    self.new_family = None;
                }
                return self.reveal_selected();
            }
            Message::OpenSelected => {
                let Some(position) = self.selected_position() else {
                    return Task::none();
                };
                match enter(position, self.shown.len()) {
                    Enter::Open => self.open = self.selected,
                    Enter::Add => {
                        let found = &self.found.games[position - self.shown.len()];
                        let (id, name) = (found.id, found.name.clone());
                        return self.update(Message::PlayElsewhere(id, name, true));
                    }
                }
            }
            Message::Pick(index) => return self.pick(index),
            Message::FocusSearch => {
                self.adding = false;
                return operation::focus(SEARCH);
            }
            Message::Correct(id, correction) => {
                let gamepalate = Arc::clone(&self.gamepalate);
                return Task::perform(
                    async move {
                        gamepalate
                            .correct_game(id, correction)
                            .map_err(|error| sentence(&error.to_string()))
                    },
                    move |result| Message::Corrected(id, correction, result),
                );
            }
            Message::Corrected(id, correction, Ok(before)) => {
                if before != correction {
                    self.undo = Some(Undo::Correction(id, before));
                }
                self.read_view();
                let text = self.game(id).map_or_else(String::new, |game| {
                    corrected_text(&game.name, before, correction, self.view.dampen)
                });
                return self.show_toast(text, before != correction);
            }
            Message::Corrected(_, _, Err(error))
            | Message::PlayedElsewhere(_, _, _, Err(error))
            | Message::Undone(Err(error))
            | Message::FamiliesSaved(_, Err(error)) => {
                return self.show_toast(error, false);
            }
            Message::Undo => {
                let gamepalate = Arc::clone(&self.gamepalate);
                return match self.undo.take() {
                    Some(Undo::Correction(id, before)) => Task::perform(
                        async move {
                            gamepalate
                                .correct_game(id, before)
                                .map(|_| ())
                                .map_err(|error| sentence(&error.to_string()))
                        },
                        Message::Undone,
                    ),
                    Some(Undo::Families(undo)) => Task::perform(
                        async move {
                            gamepalate
                                .undo_families(&undo)
                                .map_err(|error| sentence(&error.to_string()))
                        },
                        Message::Undone,
                    ),
                    Some(Undo::Elsewhere(id, before)) => Task::perform(
                        async move {
                            gamepalate
                                .set_played_elsewhere(id, before)
                                .map(|_| ())
                                .map_err(|error| sentence(&error.to_string()))
                        },
                        Message::Undone,
                    ),
                    None => Task::none(),
                };
            }
            Message::Undone(Ok(())) => {
                let reveal = self.reload();
                return Task::batch([reveal, self.show_toast("Undone".to_string(), false)]);
            }
            Message::HideToast(id) => self.toasts.hide(id),
            Message::ChangeFamilies(change) => {
                let gamepalate = Arc::clone(&self.gamepalate);
                return Task::perform(
                    async move {
                        let result = gamepalate
                            .change_families(&change)
                            .map_err(|error| sentence(&error.to_string()));
                        (change, result)
                    },
                    |(change, result)| Message::FamiliesSaved(change, result),
                );
            }
            Message::FamiliesSaved(change, Ok(changed)) => {
                let text = changed_game(&change)
                    .and_then(|id| self.game(id))
                    .map_or_else(String::new, |game| {
                        families_text(&change, game, &changed.gone)
                    });
                if matches!(change, FamilyChange::AddGame { .. }) {
                    self.new_family = None;
                }
                self.undo = Some(Undo::Families(changed.undo));
                let reveal = self.reload();
                return Task::batch([reveal, self.show_toast(text, true)]);
            }
            Message::NewFamily => {
                self.new_family = Some(String::new());
                return operation::focus(NEW_FAMILY);
            }
            Message::NewFamilyName(name) => self.new_family = Some(name),
            Message::CreateFamily => {
                if let (Some(game), Some(family)) = (self.open, &self.new_family)
                    && !family.trim().is_empty()
                {
                    let family = family.clone();
                    return self.update(Message::ChangeFamilies(FamilyChange::AddGame {
                        family,
                        game,
                    }));
                }
            }
            Message::OpenFamilies => super::open_families(&self.data_dir),
            Message::SteamClient(read) => {
                if let Some(question) = &mut self.question {
                    return question
                        .answer(self.data_dir.clone(), read)
                        .map(Message::SteamClientAnswered);
                }
            }
            Message::SteamClientAnswered(Ok(())) => self.question = None,
            Message::SteamClientAnswered(Err(error)) => {
                if let Some(question) = &mut self.question {
                    question.failed(error);
                }
            }
        }

        Task::none()
    }

    pub fn subscription() -> Subscription<Message> {
        Subscription::batch([
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            keyboard::listen().filter_map(key_message),
        ])
    }

    pub fn contains(&self, id: AppId) -> bool {
        self.game(id).is_some()
    }

    fn game(&self, id: AppId) -> Option<&LibraryGameView> {
        self.view.games.iter().find(|game| game.id == id)
    }

    fn reload(&mut self) -> Task<Message> {
        self.read_view();
        self.reveal_selected()
    }

    fn read_view(&mut self) {
        self.view = self.gamepalate.library_view();
        self.tags = None;
        self.heaviest = heaviest(&self.view.games);
        self.shown = shown(&self.view.games, self.filter, self.sort, &self.search);
        self.found.games = self.gamepalate.search_catalog(&self.search);
    }

    fn selected_position(&self) -> Option<usize> {
        let id = self.selected?;
        self.shown
            .iter()
            .position(|&index| self.view.games[index].id == id)
            .or_else(|| {
                self.found
                    .games
                    .iter()
                    .position(|game| game.id == id)
                    .map(|position| self.shown.len() + position)
            })
    }

    fn id_at(&self, position: usize) -> Option<AppId> {
        match self.shown.get(position) {
            Some(&index) => Some(self.view.games[index].id),
            None => self
                .found
                .games
                .get(position - self.shown.len())
                .map(|game| game.id),
        }
    }

    fn reveal_selected(&mut self) -> Task<Message> {
        let Some(position) = self.selected_position() else {
            return Task::none();
        };
        let rows = self.rows(self.area.width);
        match reveal_offset(&rows, position, self.offset, self.area.height) {
            Some(y) => {
                self.offset = y;
                operation::scroll_to(
                    GRID,
                    AbsoluteOffset {
                        x: None,
                        y: Some(y),
                    },
                )
            }
            None => Task::none(),
        }
    }

    fn show_toast(&mut self, text: String, undo: bool) -> Task<Message> {
        self.toasts.show(text, undo, Message::HideToast)
    }

    fn refresh(&mut self) -> Task<Message> {
        self.shown = shown(&self.view.games, self.filter, self.sort, &self.search);
        self.offset = 0.0;

        operation::scroll_to(
            GRID,
            AbsoluteOffset {
                x: None,
                y: Some(0.0),
            },
        )
    }

    fn rows(&self, width: f32) -> Vec<Row> {
        rows(
            self.counting(),
            self.shown.len(),
            self.found.games.len(),
            columns(width),
            self.sort == Sort::TasteWeight,
        )
    }

    fn counting(&self) -> usize {
        self.shown
            .iter()
            .take_while(|&&index| self.view.games[index].counts)
            .count()
    }

    fn load(&mut self) -> Task<Message> {
        let rows = self.rows(self.area.width);
        let height = self.area.height;
        let library = self.shown.len();
        let settled = self.found.settled;
        let ids = |range: Range<usize>| -> Vec<AppId> {
            rows[range]
                .iter()
                .filter_map(|row| match row {
                    Row::Games(games) => Some(games.clone()),
                    Row::Divider | Row::NoMatch | Row::FromSteam => None,
                })
                .flatten()
                .filter(|&position| asks_cover(position, library, settled))
                .filter_map(|position| self.id_at(position))
                .collect()
        };
        let open = self.open.into_iter();
        let wanted: Vec<AppId> = open
            .clone()
            .chain(
                wanted_rows(&rows, self.offset, height, self.upward)
                    .into_iter()
                    .flat_map(|index| ids(index..index + 1)),
            )
            .collect();
        let kept: Vec<AppId> = open
            .chain(ids(visible_rows(
                &rows,
                self.offset - height * SCREENS_KEPT,
                height * (1.0 + 2.0 * SCREENS_KEPT),
            )))
            .collect();

        self.capsules.keep_only(&kept);
        let started = self.capsules.want(wanted);

        Task::batch(started.into_iter().map(|id| {
            let gamepalate = Arc::clone(&self.gamepalate);
            Task::perform(
                async move { gamepalate.capsule(id).await.ok().map(handle) },
                move |handle| Message::CapsuleLoaded(id, handle),
            )
        }))
    }

    pub fn view(&self) -> Element<'_, Message> {
        let searching = !self.search.trim().is_empty();
        let filters = row(FILTERS
            .into_iter()
            .filter(|&filter| shows_filter(filter, &self.view.games))
            .map(|filter| {
                let chip = button(
                    text!(
                        "{} {}",
                        filter_label(filter),
                        count(&self.view.games, filter)
                    )
                    .size(font::NOTE_SIZE),
                )
                .padding([3, 10])
                .style(style::choice(!searching && filter == self.filter))
                .on_press(Message::Filter(filter));
                if filter != Filter::NotOwned {
                    return chip.into();
                }
                tooltip(
                    chip,
                    container(text(NOT_OWNED_TIP).size(font::NOTE_SIZE))
                        .padding([6, 10])
                        .style(style::popup),
                    tooltip::Position::Bottom,
                )
                .into()
            }))
        .spacing(6);
        let tools = row![
            filters,
            space().width(Fill),
            text("Sort").size(font::NOTE_SIZE).style(style::muted),
            pick_list(SORTS, Some(self.sort), Message::Sort)
                .text_size(font::NOTE_SIZE)
                .padding([4, 8])
                .style(style::dropdown)
                .menu_style(style::menu),
            text_input(placeholder(self.adding), &self.search)
                .style(style::field)
                .id(SEARCH)
                .on_input(Message::Search)
                .size(font::NOTE_SIZE)
                .padding([5, 8])
                .width(SEARCH_WIDTH),
            button(text("+ Add a game").size(font::NOTE_SIZE))
                .padding([4, 12])
                .style(style::chip)
                .on_press(Message::AddGame),
        ]
        .spacing(8)
        .align_y(Center);

        let body: Element<'_, Message> = if self.shown.is_empty() && self.found.games.is_empty() {
            container(
                text(empty_text(
                    self.view.games.is_empty(),
                    self.filter,
                    &self.search,
                ))
                .size(font::BAR_SIZE)
                .style(style::muted),
            )
            .padding(GRID_PADDING)
            .into()
        } else {
            responsive(move |size| self.grid(size.width)).into()
        };

        let popup = self.open.and_then(|id| self.game(id)).map(|game| {
            stack![
                opaque(mouse_area(space().width(Fill).height(Fill)).on_press(Message::Close)),
                responsive(move |size| self.placed_popup(game, size)),
            ]
        });
        let tags = self
            .open
            .and_then(|id| self.game(id))
            .filter(|_| self.tags_popup.is_open())
            .map(|game| {
                sheet(
                    "Tags",
                    &game.name,
                    tags_body(
                        self.game_tags(),
                        self.tags_popup.all,
                        Message::ToggleAllTags,
                    ),
                    Message::CloseTags,
                )
            });
        let toast = self.toasts.view(Message::Undo);

        let question = self.question.as_ref().map(|question| {
            container(question.view(Message::SteamClient, ElsewhereHint::None))
                .padding([16, GRID_PADDING as u16])
                .center_x(Fill)
                .max_width(800)
        });

        column![
            container(tools).padding([12, GRID_PADDING as u16]),
            style::separator(),
        ]
        .push(question)
        .push(stack![body].push(popup).push(tags).push(toast))
        .into()
    }
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("z" | "Z") => Some(Message::Undo),
            Key::Character("f" | "F") => Some(Message::FocusSearch),
            _ => None,
        };
    }

    match key.as_ref() {
        Key::Named(Named::ArrowLeft) => Some(Message::Move(Move::Left)),
        Key::Named(Named::ArrowRight) => Some(Message::Move(Move::Right)),
        Key::Named(Named::ArrowUp) => Some(Message::Move(Move::Up)),
        Key::Named(Named::ArrowDown) => Some(Message::Move(Move::Down)),
        Key::Named(Named::Enter) => Some(Message::OpenSelected),
        Key::Named(Named::Escape) => Some(Message::Close),
        Key::Character("t" | "T") => Some(Message::ToggleTags),
        Key::Character("j" | "J") => Some(Message::ScrollDown),
        Key::Character("k" | "K") => Some(Message::ScrollUp),
        Key::Character("1") => Some(Message::Pick(0)),
        Key::Character("2") => Some(Message::Pick(1)),
        Key::Character("3") => Some(Message::Pick(2)),
        _ => None,
    }
}

fn handle(capsule: Capsule) -> image::Handle {
    image::Handle::from_rgba(capsule.width, capsule.height, capsule.rgba)
}

fn enter(position: usize, library: usize) -> Enter {
    if position < library {
        Enter::Open
    } else {
        Enter::Add
    }
}

fn asks_cover(position: usize, library: usize, settled: bool) -> bool {
    position < library || settled
}

fn placeholder(adding: bool) -> &'static str {
    if adding {
        ADD_PLACEHOLDER
    } else {
        SEARCH_PLACEHOLDER
    }
}

pub(crate) fn hours(hours: f64) -> String {
    if hours == 0.0 || hours >= 10.0 {
        format!("{hours:.0} h")
    } else {
        format!("{hours:.1} h")
    }
}

fn heaviest(games: &[LibraryGameView]) -> f64 {
    games.iter().map(|game| game.share).fold(0.0, f64::max)
}

fn multiplier(factor: f64) -> String {
    let text = format!("{factor:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("×{text}")
}

fn hours_text(hours: f64) -> String {
    if hours == 1.0 {
        "1 hour".to_string()
    } else {
        format!("{hours} hours")
    }
}

#[cfg(test)]
mod fixtures {
    use super::*;
    use gamepalate_app::HoursSource;

    pub(super) fn game(id: u32, name: &str) -> LibraryGameView {
        LibraryGameView {
            id: AppId(id),
            name: name.to_string(),
            owned_hours: None,
            borrowed: false,
            recent: false,
            wishlist: false,
            followed: false,
            elsewhere: false,
            elsewhere_only: false,
            hours: 0.0,
            source: HoursSource::Played,
            counts: false,
            share: 0.0,
            tags: Vec::new(),
            correction: GameCorrection::default(),
            families: Vec::new(),
            held_out: false,
        }
    }

    pub(super) fn owned(id: u32, name: &str, hours: f64, share: f64) -> LibraryGameView {
        LibraryGameView {
            owned_hours: Some(hours),
            hours,
            counts: share > 0.0,
            share,
            ..game(id, name)
        }
    }

    pub(super) fn games() -> Vec<LibraryGameView> {
        vec![
            owned(1, "Outer Wilds", 40.0, 0.2),
            owned(2, "Hades", 120.0, 0.5),
            owned(3, "Celeste", 1.0, 0.0),
            LibraryGameView {
                wishlist: true,
                hours: 5.0,
                source: HoursSource::Wishlist,
                counts: true,
                share: 0.1,
                ..game(4, "Pentiment")
            },
            LibraryGameView {
                followed: true,
                hours: 3.0,
                source: HoursSource::Followed,
                counts: true,
                share: 0.05,
                ..game(5, "Ōkami HD")
            },
            owned(6, "Aseprite", 0.0, 0.0),
        ]
    }

    pub(super) fn ids(games: &[LibraryGameView], shown: &[usize]) -> Vec<u32> {
        shown.iter().map(|&index| games[index].id.0).collect()
    }

    pub(super) fn view_of(games: Vec<LibraryGameView>) -> LibraryView {
        LibraryView {
            games,
            families: Vec::new(),
            automatic_families: Vec::new(),
            min_hours: 2.0,
            wishlist_hours: 5.0,
            followed_hours: 3.0,
            elsewhere_hours: 20.0,
            dampen: 0.5,
        }
    }

    pub(super) fn elsewhere(id: u32, name: &str) -> LibraryGameView {
        LibraryGameView {
            elsewhere: true,
            elsewhere_only: true,
            hours: 20.0,
            source: HoursSource::Elsewhere,
            counts: true,
            share: 0.1,
            ..game(id, name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(key: Key, modifiers: keyboard::Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    fn character(character: &str, modifiers: keyboard::Modifiers) -> Option<Message> {
        key_message(pressed(Key::Character(character.into()), modifiers))
    }

    fn named(named: Named) -> Option<Message> {
        key_message(pressed(Key::Named(named), keyboard::Modifiers::empty()))
    }

    #[test]
    fn t_opens_and_closes_the_tags_only_when_there_are_some() {
        let mut popup = TagsPopup::default();

        popup.toggle(false);
        assert!(!popup.is_open());
        popup.toggle(true);
        assert!(popup.is_open());
        popup.toggle(true);
        assert!(!popup.is_open());
        assert!(matches!(
            character("t", keyboard::Modifiers::empty()),
            Some(Message::ToggleTags)
        ));
        assert!(matches!(
            character("T", keyboard::Modifiers::empty()),
            Some(Message::ToggleTags)
        ));
    }

    #[test]
    fn with_the_tags_open_the_game_underneath_does_not_act() {
        let popup = TagsPopup {
            open: true,
            all: false,
        };

        for message in [
            Message::Pick(0),
            Message::Pick(1),
            Message::Pick(2),
            Message::Undo,
            Message::OpenSelected,
            Message::Move(Move::Left),
            Message::Move(Move::Right),
        ] {
            assert_eq!(popup.over(&message), Over::Ignore);
        }
        assert_eq!(
            popup.over(&Message::Move(Move::Down)),
            Over::Scroll(SCROLL_STEP)
        );
        assert_eq!(
            popup.over(&Message::Move(Move::Up)),
            Over::Scroll(-SCROLL_STEP)
        );
        assert_eq!(popup.over(&Message::ScrollDown), Over::Scroll(SCROLL_STEP));
        assert_eq!(popup.over(&Message::ScrollUp), Over::Scroll(-SCROLL_STEP));
        assert_eq!(popup.over(&Message::Close), Over::Through);
    }

    #[test]
    fn j_and_k_scroll_the_tags() {
        let none = keyboard::Modifiers::empty();

        assert!(matches!(character("j", none), Some(Message::ScrollDown)));
        assert!(matches!(character("k", none), Some(Message::ScrollUp)));
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_f_searches() {
        let ctrl = keyboard::Modifiers::CTRL;

        assert!(matches!(character("z", ctrl), Some(Message::Undo)));
        assert!(matches!(character("f", ctrl), Some(Message::FocusSearch)));
        assert!(character("z", keyboard::Modifiers::empty()).is_none());
        assert!(character("1", ctrl).is_none());
    }

    #[test]
    fn the_keys_move_open_close_and_choose() {
        let none = keyboard::Modifiers::empty();

        assert!(matches!(
            named(Named::ArrowDown),
            Some(Message::Move(Move::Down))
        ));
        assert!(matches!(
            named(Named::ArrowLeft),
            Some(Message::Move(Move::Left))
        ));
        assert!(matches!(named(Named::Enter), Some(Message::OpenSelected)));
        assert!(matches!(named(Named::Escape), Some(Message::Close)));
        assert!(matches!(character("1", none), Some(Message::Pick(0))));
        assert!(matches!(character("3", none), Some(Message::Pick(2))));
        assert!(character("4", none).is_none());
    }

    fn found(id: u32, name: &str) -> CatalogMatch {
        CatalogMatch {
            id: AppId(id),
            name: name.to_string(),
        }
    }

    #[test]
    fn the_covers_from_steam_are_asked_only_once_the_text_stays_still() {
        let mut matches = Found::default();

        matches.searched(vec![found(1, "It Takes Two")]);
        assert!(!asks_cover(0, 0, matches.settled));
        assert!(asks_cover(0, 1, matches.settled));

        matches.waited("it t", "it ta");
        assert!(!matches.settled);

        matches.waited("it ta", "it ta");
        assert!(matches.settled);
        assert!(asks_cover(0, 0, matches.settled));

        matches.searched(vec![found(1, "It Takes Two")]);
        assert!(!matches.settled);
    }

    #[test]
    fn adding_and_removing_say_which_game_and_can_be_undone() {
        assert_eq!(
            format::elsewhere_text("It Takes Two", true),
            "Added It Takes Two as played elsewhere"
        );
        assert_eq!(
            format::elsewhere_text("It Takes Two", false),
            "Removed It Takes Two"
        );
        assert!(matches!(
            character("z", keyboard::Modifiers::CTRL),
            Some(Message::Undo)
        ));
        assert!(matches!(named(Named::Enter), Some(Message::OpenSelected)));
    }

    #[test]
    fn adding_a_game_changes_what_the_search_field_asks() {
        assert_eq!(placeholder(false), "Search by name (Ctrl+F)");
        assert_eq!(
            placeholder(true),
            "Search Steam for a game you played elsewhere"
        );
    }
}
