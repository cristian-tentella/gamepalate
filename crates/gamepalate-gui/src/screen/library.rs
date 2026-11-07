use std::{fmt, ops::Range, path::PathBuf, sync::Arc, time::Duration};

use gamepalate_app::{
    AppId, Capsule, CatalogMatch, FamiliesChanged, FamiliesUndo, FamilyChange, GameCorrection,
    Gamepalate, HoursSource, LibraryGameView, LibraryView, names::folded,
};
use iced::{
    Center, ContentFit, Element, Fill, Padding, Point, Size, Subscription, Task,
    keyboard::{self, Key, key::Named},
    mouse,
    widget::{
        Id, button, checkbox, column, container, image, mouse_area, opaque, operation, pick_list,
        progress_bar, responsive, row, scrollable,
        scrollable::{AbsoluteOffset, Viewport},
        space, stack, text, text_input, tooltip,
    },
    window,
};

use super::{
    Toasts,
    profile::HELD_OUT_TIP,
    sentence,
    showcase::images::{Images, Picture},
    steam_client::{ElsewhereHint, SteamClientQuestion},
};
use crate::{
    font, format,
    style::{self, Tone},
};

const FILTERS: [Filter; 7] = [
    Filter::All,
    Filter::Owned,
    Filter::NotOwned,
    Filter::Wishlist,
    Filter::Followed,
    Filter::Elsewhere,
    Filter::Corrected,
];
const SORTS: [Sort; 3] = [Sort::TasteWeight, Sort::Hours, Sort::Name];
const GRID: Id = Id::new("library-grid");
const SEARCH: Id = Id::new("library-search");
const NEW_FAMILY: Id = Id::new("library-new-family");
const TILE_WIDTH: f32 = 150.0;
const TILE_HEIGHT: f32 = 225.0;
const GAP: f32 = 12.0;
const GRID_PADDING: f32 = 24.0;
const DIVIDER_HEIGHT: f32 = 56.0;
const NO_MATCH_HEIGHT: f32 = 40.0;
const SETTLE: Duration = Duration::from_millis(300);
const SEARCH_WIDTH: f32 = 300.0;
const SEARCH_PLACEHOLDER: &str = "Search by name (Ctrl+F)";
const ADD_PLACEHOLDER: &str = "Search Steam for a game you played elsewhere";
const FROM_STEAM: &str = "From Steam, add a game you played elsewhere";
const DIMMED: f32 = 0.45;
const SCREENS_AHEAD: f32 = 2.0;
const SCREENS_KEPT: f32 = 2.0;
const CHROME_HEIGHT: f32 = 110.0;
const IMAGE_RADIUS: f32 = 6.0;
const DEFAULT_AREA: Size = Size::new(1280.0, 700.0);
const POPUP_WIDTH: f32 = 320.0;
const POPUP_HEIGHT: f32 = 360.0;
const POPUP_GAP: f32 = 8.0;
const POPUP_MARGIN: f32 = 8.0;
const NOT_OWNED_TIP: &str =
    "Played but not in your library: Family Sharing, free weekends, removed games";
const PICKS: [Choice; 3] = [Choice::Fully, Choice::Half, Choice::Not];

pub struct Library {
    gamepalate: Arc<Gamepalate>,
    data_dir: PathBuf,
    view: LibraryView,
    heaviest: f64,
    question: Option<SteamClientQuestion>,
    open: Option<AppId>,
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
enum Choice {
    Fully,
    Half,
    Not,
    Custom(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    All,
    Owned,
    NotOwned,
    Wishlist,
    Followed,
    Elsewhere,
    Corrected,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sort {
    TasteWeight,
    Hours,
    Name,
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

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Games(Range<usize>),
    Divider,
    NoMatch,
    FromSteam,
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

        Task::batch([task, self.load()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
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
                return self.show_toast(elsewhere_text(&name, played), before != played);
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
            Message::Pick(index) => {
                let view = &self.view;
                let Some(game) = self.open.and_then(|id| self.game(id)) else {
                    return Task::none();
                };
                let choice = PICKS[index.min(2)];
                if game.hours >= view.min_hours
                    && choice != selected_choice(game.correction.multiplier, view.dampen)
                {
                    let id = game.id;
                    let correction = GameCorrection {
                        multiplier: choice_multiplier(choice, view.dampen),
                        ..game.correction
                    };
                    self.open = None;
                    self.new_family = None;
                    return self.update(Message::Correct(id, correction));
                }
            }
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
        .push(stack![body].push(popup).push(toast))
        .into()
    }

    fn grid(&self, width: f32) -> Element<'_, Message> {
        let rows = self.rows(width);
        let shown = visible_rows(&rows, self.offset, self.area.height.max(1.0));
        let above: f32 = rows[..shown.start].iter().map(row_height).sum();
        let below: f32 = rows[shown.end..].iter().map(row_height).sum();

        let content = column![space().height(above)]
            .extend(rows[shown.clone()].iter().map(|line| self.line(line)))
            .push(space().height(below))
            .width(grid_width(columns(width)));

        scrollable(
            container(content)
                .padding([GRID_PADDING, GRID_PADDING])
                .center_x(Fill),
        )
        .id(GRID)
        .on_scroll(Message::Scrolled)
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn line<'a>(&'a self, line: &Row) -> Element<'a, Message> {
        match line {
            Row::Games(games) => {
                row(games
                    .clone()
                    .map(|position| match self.shown.get(position) {
                        Some(&index) => self.tile(&self.view.games[index]),
                        None => self.found_tile(&self.found.games[position - self.shown.len()]),
                    }))
                .spacing(GAP)
                .height(TILE_HEIGHT + GAP)
                .into()
            }
            Row::NoMatch => container(
                text(empty_text(false, self.filter, &self.search))
                    .size(font::BAR_SIZE)
                    .style(style::muted),
            )
            .height(NO_MATCH_HEIGHT)
            .align_y(Center)
            .into(),
            Row::FromSteam => container(text(FROM_STEAM).size(font::BAR_SIZE).font(font::SEMIBOLD))
                .height(DIVIDER_HEIGHT)
                .align_y(Center)
                .into(),
            Row::Divider => container(
                column![
                    text("Doesn't count")
                        .size(font::BAR_SIZE)
                        .font(font::SEMIBOLD),
                    text(divider_note(self.view.min_hours))
                        .size(font::NOTE_SIZE)
                        .style(style::muted),
                ]
                .spacing(2),
            )
            .height(DIVIDER_HEIGHT)
            .align_y(Center)
            .into(),
        }
    }

    fn tile<'a>(&'a self, game: &'a LibraryGameView) -> Element<'a, Message> {
        let picture: Element<'a, Message> = match self.capsules.get(&game.id) {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .content_fit(ContentFit::Cover)
                .border_radius(IMAGE_RADIUS)
                .opacity(if game.counts { 1.0 } else { DIMMED })
                .into(),
            Some(Picture::Failed) => container(
                text(&game.name)
                    .size(font::BAR_SIZE)
                    .center()
                    .style(if game.counts {
                        text::default
                    } else {
                        style::muted
                    }),
            )
            .padding(12)
            .center_x(TILE_WIDTH)
            .center_y(TILE_HEIGHT)
            .style(style::picture)
            .into(),
            Some(Picture::Loading) | None => container(space())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .style(style::picture)
                .into(),
        };
        let badge = correction_badge(game.correction).map(|label| {
            container(text(label).size(font::NOTE_SIZE))
                .padding([1, 6])
                .style(style::capsule_badge)
        });
        let bar = weight_fraction(game.share, self.heaviest).map(|fraction| {
            progress_bar(0.0..=1.0, fraction)
                .girth(3)
                .length(Fill)
                .style(style::weight_bar)
        });
        let strip = container(
            column![text(tile_label(game)).size(font::NOTE_SIZE)]
                .push(bar)
                .spacing(3),
        )
        .padding([3, 8])
        .width(Fill)
        .style(style::capsule_strip);
        let overlay = column![
            row![space().width(Fill)].push(badge).padding(6),
            space().height(Fill),
            strip,
        ];
        let frame = (self.selected == Some(game.id)).then(|| {
            container(space())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .style(style::capsule_selected)
        });

        tooltip(
            mouse_area(
                stack![picture, overlay]
                    .push(frame)
                    .width(TILE_WIDTH)
                    .height(TILE_HEIGHT),
            )
            .interaction(mouse::Interaction::Pointer)
            .on_press(Message::Open(game.id)),
            container(text(&game.name).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    }

    fn found_tile<'a>(&'a self, game: &'a CatalogMatch) -> Element<'a, Message> {
        let picture: Element<'a, Message> = match self.capsules.get(&game.id) {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .content_fit(ContentFit::Cover)
                .border_radius(IMAGE_RADIUS)
                .into(),
            Some(Picture::Failed | Picture::Loading) | None => container(
                text(&game.name)
                    .size(font::BAR_SIZE)
                    .center()
                    .style(style::muted),
            )
            .padding(12)
            .center_x(TILE_WIDTH)
            .center_y(TILE_HEIGHT)
            .style(style::picture)
            .into(),
        };
        let plus = container(
            container(text("+").size(font::PRICE_SIZE).font(font::SEMIBOLD))
                .padding([0, 12])
                .style(style::capsule_badge),
        )
        .center_x(TILE_WIDTH)
        .center_y(TILE_HEIGHT);
        let frame = (self.selected == Some(game.id)).then(|| {
            container(space())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .style(style::capsule_selected)
        });

        tooltip(
            mouse_area(
                stack![picture, plus]
                    .push(frame)
                    .width(TILE_WIDTH)
                    .height(TILE_HEIGHT),
            )
            .interaction(mouse::Interaction::Pointer)
            .on_press(Message::PlayElsewhere(game.id, game.name.clone(), true)),
            container(text(&game.name).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    }

    fn placed_popup<'a>(&'a self, game: &'a LibraryGameView, size: Size) -> Element<'a, Message> {
        let rows = self.rows(size.width);
        let Some(tile) = self
            .shown
            .iter()
            .position(|&index| self.view.games[index].id == game.id)
            .and_then(|position| tile_origin(&rows, position, size.width, self.offset))
        else {
            return space().into();
        };
        let origin = popup_origin(tile, size);

        container(opaque(
            container(self.popup(game))
                .width(POPUP_WIDTH)
                .style(style::popup),
        ))
        .padding(Padding {
            top: origin.y,
            left: origin.x,
            ..Padding::ZERO
        })
        .into()
    }

    fn popup<'a>(&'a self, game: &'a LibraryGameView) -> Element<'a, Message> {
        let view = &self.view;
        let correctable = game.hours >= view.min_hours;
        let selected = selected_choice(game.correction.multiplier, view.dampen);
        let choices = row(choices(selected).into_iter().map(|choice| {
            let pick = PICKS.iter().position(|&pick| pick == choice);
            button(
                column![
                    text(choice_factor(choice, view.dampen))
                        .size(font::INFO_SIZE)
                        .font(font::SEMIBOLD),
                    text(choice_label(choice)).size(font::NOTE_SIZE),
                ]
                .spacing(2)
                .align_x(Center)
                .width(Fill),
            )
            .width(Fill)
            .padding([6, 4])
            .style(style::tinted(choice_tone(choice), choice == selected))
            .on_press_maybe(
                pick.filter(|_| correctable && choice != selected)
                    .map(Message::Pick),
            )
            .into()
        }))
        .spacing(6);
        let hide =
            (game.owned_hours.is_none() && !game.borrowed && !game.recent && !game.elsewhere).then(
                || {
                    let id = game.id;
                    let correction = game.correction;
                    checkbox(correction.hidden)
                        .label("Hide from Discover")
                        .text_size(font::BAR_SIZE)
                        .on_toggle(move |hidden| {
                            Message::Correct(
                                id,
                                GameCorrection {
                                    hidden,
                                    ..correction
                                },
                            )
                        })
                },
            );

        column![
            column![
                text(&game.name).size(font::INFO_SIZE).font(font::SEMIBOLD),
                text(source_line(game, view))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                text(weight_line(game, view)).size(font::BAR_SIZE),
            ]
            .push((!game.tags.is_empty()).then(|| {
                text!("Brings {}", game.tags.join(", "))
                    .size(font::NOTE_SIZE)
                    .style(style::muted)
            }))
            .spacing(2),
            text("Counts for your taste")
                .size(font::NOTE_SIZE)
                .style(style::muted),
            choices,
        ]
        .push((!correctable).then(|| {
            text(no_effect_text(game.hours, view.min_hours))
                .size(font::NOTE_SIZE)
                .style(style::muted)
        }))
        .push(style::separator())
        .push(text("Families").size(font::NOTE_SIZE).style(style::muted))
        .push(self.family_chips(game))
        .push(row![held_out(game)].push(hide).spacing(16))
        .push(
            button(text("Open families.toml ›").size(font::NOTE_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::OpenFamilies),
        )
        .push(removal(game).map(|_| style::separator()))
        .push(removal(game).map(|message| {
            button(text("Remove from Library").size(font::NOTE_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(message)
        }))
        .spacing(10)
        .padding(16)
        .into()
    }

    fn family_chips<'a>(&'a self, game: &'a LibraryGameView) -> Element<'a, Message> {
        let chips = self.view.families.iter().map(|family| {
            let inside = game.families.contains(family);
            button(text(family).size(font::NOTE_SIZE))
                .padding([4, 12])
                .style(style::choice(inside))
                .on_press(Message::ChangeFamilies(chip_change(
                    family, game.id, inside,
                )))
                .into()
        });
        let new: Element<'a, Message> = match &self.new_family {
            Some(name) => text_input("Family name", name)
                .id(NEW_FAMILY)
                .style(style::field)
                .on_input(Message::NewFamilyName)
                .on_submit(Message::CreateFamily)
                .size(font::NOTE_SIZE)
                .padding([4, 10])
                .width(160)
                .into(),
            None => button(text("+ New family").size(font::NOTE_SIZE))
                .padding([4, 12])
                .style(style::chip)
                .on_press(Message::NewFamily)
                .into(),
        };

        row(chips)
            .push(new)
            .spacing(6)
            .wrap()
            .vertical_spacing(6)
            .into()
    }
}

fn held_out(game: &LibraryGameView) -> Element<'_, Message> {
    let id = game.id;
    tooltip(
        checkbox(game.held_out)
            .label("Held out")
            .text_size(font::BAR_SIZE)
            .on_toggle(move |held_out| {
                Message::ChangeFamilies(if held_out {
                    FamilyChange::HoldOut { game: id }
                } else {
                    FamilyChange::Release { game: id }
                })
            }),
        container(text(HELD_OUT_TIP).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Bottom,
    )
    .into()
}

fn chip_change(family: &str, game: AppId, inside: bool) -> FamilyChange {
    let family = family.to_string();
    if inside {
        FamilyChange::RemoveGame { family, game }
    } else {
        FamilyChange::AddGame { family, game }
    }
}

fn changed_game(change: &FamilyChange) -> Option<AppId> {
    match change {
        FamilyChange::AddGame { game, .. }
        | FamilyChange::RemoveGame { game, .. }
        | FamilyChange::HoldOut { game }
        | FamilyChange::Release { game } => Some(*game),
        FamilyChange::Rename { .. } | FamilyChange::Delete { .. } => None,
    }
}

fn families_text(change: &FamilyChange, game: &LibraryGameView, gone: &[String]) -> String {
    match change {
        FamilyChange::AddGame { family, .. } if game.held_out => {
            format!("Added to {}, no longer held out", family.trim())
        }
        FamilyChange::AddGame { family, .. } => format!("Added to {}", family.trim()),
        FamilyChange::RemoveGame { family, .. } if gone.contains(family) => {
            format!("Removed from {family}, which is now gone")
        }
        FamilyChange::RemoveGame { family, .. } => format!("Removed from {family}"),
        FamilyChange::HoldOut { .. } if game.families.is_empty() => "Held out".to_string(),
        FamilyChange::HoldOut { .. } => {
            format!("Held out, removed from {}", game.families.join(", "))
        }
        FamilyChange::Release { .. } => "No longer held out".to_string(),
        FamilyChange::Rename { .. } | FamilyChange::Delete { .. } => String::new(),
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
        Key::Character("1") => Some(Message::Pick(0)),
        Key::Character("2") => Some(Message::Pick(1)),
        Key::Character("3") => Some(Message::Pick(2)),
        _ => None,
    }
}

fn moved(rows: &[Row], position: Option<usize>, direction: Move) -> Option<usize> {
    let games: Vec<&Range<usize>> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Games(games) => Some(games),
            Row::Divider | Row::NoMatch | Row::FromSteam => None,
        })
        .collect();
    let last = games.last()?.end.checked_sub(1)?;
    let Some(position) = position else {
        return Some(0);
    };
    let line = games.iter().position(|games| games.contains(&position))?;
    let column = position - games[line].start;
    let at = |line: usize| {
        let games = games[line];
        games.start + column.min(games.len() - 1)
    };

    Some(match direction {
        Move::Left => position.saturating_sub(1),
        Move::Right => (position + 1).min(last),
        Move::Up if line > 0 => at(line - 1),
        Move::Down if line + 1 < games.len() => at(line + 1),
        Move::Up | Move::Down => position,
    })
}

fn handle(capsule: Capsule) -> image::Handle {
    image::Handle::from_rgba(capsule.width, capsule.height, capsule.rgba)
}

impl Filter {
    fn keeps(self, game: &LibraryGameView) -> bool {
        match self {
            Filter::All => true,
            Filter::Owned => game.owned_hours.is_some(),
            Filter::NotOwned => game.borrowed,
            Filter::Wishlist => game.wishlist,
            Filter::Followed => game.followed,
            Filter::Elsewhere => game.elsewhere,
            Filter::Corrected => {
                game.correction.multiplier.is_some() || game.correction.hidden || game.held_out
            }
        }
    }

    fn hides_when_empty(self) -> bool {
        matches!(
            self,
            Filter::NotOwned | Filter::Elsewhere | Filter::Corrected
        )
    }
}

fn shows_filter(filter: Filter, games: &[LibraryGameView]) -> bool {
    !filter.hides_when_empty() || count(games, filter) > 0
}

fn removal(game: &LibraryGameView) -> Option<Message> {
    game.elsewhere_only
        .then(|| Message::PlayElsewhere(game.id, game.name.clone(), false))
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

fn elsewhere_text(name: &str, played: bool) -> String {
    if played {
        format!("Added {name} as played elsewhere")
    } else {
        format!("Removed {name}")
    }
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Sort::TasteWeight => "Taste weight",
            Sort::Hours => "Hours",
            Sort::Name => "Name",
        })
    }
}

fn filter_label(filter: Filter) -> &'static str {
    match filter {
        Filter::All => "All",
        Filter::Owned => "Owned",
        Filter::NotOwned => "Not owned",
        Filter::Wishlist => "Wishlist",
        Filter::Followed => "Followed",
        Filter::Elsewhere => "Elsewhere",
        Filter::Corrected => "Corrected",
    }
}

fn count(games: &[LibraryGameView], filter: Filter) -> usize {
    games.iter().filter(|game| filter.keeps(game)).count()
}

fn shown(games: &[LibraryGameView], filter: Filter, sort: Sort, search: &str) -> Vec<usize> {
    let search = folded(search.trim());
    let mut shown: Vec<usize> = (0..games.len())
        .filter(|&index| {
            let game = &games[index];
            if search.is_empty() {
                filter.keeps(game)
            } else {
                folded(&game.name).contains(&search)
            }
        })
        .collect();
    let name = |index: &usize| folded(&games[*index].name);

    match sort {
        Sort::TasteWeight => shown.sort_by(|a, b| {
            let (a_game, b_game) = (&games[*a], &games[*b]);
            b_game
                .counts
                .cmp(&a_game.counts)
                .then(if a_game.counts {
                    b_game.share.total_cmp(&a_game.share)
                } else {
                    b_game.hours.total_cmp(&a_game.hours)
                })
                .then_with(|| name(a).cmp(&name(b)))
        }),
        Sort::Hours => shown.sort_by(|a, b| {
            games[*b]
                .hours
                .total_cmp(&games[*a].hours)
                .then_with(|| name(a).cmp(&name(b)))
        }),
        Sort::Name => shown.sort_by_cached_key(name),
    }

    shown
}

fn columns(width: f32) -> usize {
    let inner = width - 2.0 * GRID_PADDING;
    (((inner + GAP) / (TILE_WIDTH + GAP)).floor() as usize).max(1)
}

fn grid_width(columns: usize) -> f32 {
    columns as f32 * (TILE_WIDTH + GAP) - GAP
}

fn rows(counting: usize, total: usize, found: usize, columns: usize, divided: bool) -> Vec<Row> {
    let chunks = |range: Range<usize>| {
        range
            .clone()
            .step_by(columns)
            .map(move |start| Row::Games(start..(start + columns).min(range.end)))
    };

    let mut rows: Vec<Row> = if divided && counting < total {
        chunks(0..counting)
            .chain([Row::Divider])
            .chain(chunks(counting..total))
            .collect()
    } else {
        chunks(0..total).collect()
    };
    if found > 0 {
        if total == 0 {
            rows.push(Row::NoMatch);
        }
        rows.push(Row::FromSteam);
        rows.extend(chunks(total..total + found));
    }
    rows
}

fn row_height(row: &Row) -> f32 {
    match row {
        Row::Games(_) => TILE_HEIGHT + GAP,
        Row::Divider | Row::FromSteam => DIVIDER_HEIGHT,
        Row::NoMatch => NO_MATCH_HEIGHT,
    }
}

fn wanted_rows(rows: &[Row], offset: f32, height: f32, upward: bool) -> Vec<usize> {
    let visible = visible_rows(rows, offset, height);
    let ahead = height * SCREENS_AHEAD;

    if upward {
        visible
            .chain(visible_rows(rows, offset - ahead, ahead).rev())
            .collect()
    } else {
        visible
            .chain(visible_rows(rows, offset + height, ahead))
            .collect()
    }
}

fn visible_rows(rows: &[Row], offset: f32, height: f32) -> Range<usize> {
    let top = offset - GRID_PADDING;
    let bottom = top + height;
    let mut start = rows.len();
    let mut end = rows.len();
    let mut y = 0.0;

    for (index, row) in rows.iter().enumerate() {
        let next = y + row_height(row);
        if start == rows.len() && next > top {
            start = index;
        }
        if y >= bottom {
            end = index;
            break;
        }
        y = next;
    }

    start.min(end)..end
}

pub(crate) fn hours(hours: f64) -> String {
    if hours == 0.0 || hours >= 10.0 {
        format!("{hours:.0} h")
    } else {
        format!("{hours:.1} h")
    }
}

fn tile_label(game: &LibraryGameView) -> String {
    match game.source {
        HoursSource::Played => hours(game.owned_hours.unwrap_or(game.hours)),
        HoursSource::Wishlist => "wishlist".to_string(),
        HoursSource::Followed => "followed".to_string(),
        HoursSource::WouldTry => "would try".to_string(),
        HoursSource::Elsewhere => "elsewhere".to_string(),
    }
}

fn heaviest(games: &[LibraryGameView]) -> f64 {
    games.iter().map(|game| game.share).fold(0.0, f64::max)
}

fn weight_fraction(share: f64, heaviest: f64) -> Option<f32> {
    (share > 0.0 && heaviest > 0.0).then(|| (share / heaviest) as f32)
}

fn multiplier(factor: f64) -> String {
    let text = format!("{factor:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("×{text}")
}

fn correction_badge(correction: GameCorrection) -> Option<String> {
    let parts: Vec<String> = correction
        .multiplier
        .map(multiplier)
        .into_iter()
        .chain(correction.hidden.then(|| "hidden".to_string()))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn divider_note(min_hours: f64) -> String {
    format!(
        "Under {} of play, set to ×0, or without tags",
        hours_text(min_hours)
    )
}

fn reveal_offset(rows: &[Row], position: usize, offset: f32, height: f32) -> Option<f32> {
    let mut top = GRID_PADDING;
    for row in rows {
        if let Row::Games(games) = row
            && games.contains(&position)
        {
            let bottom = top + TILE_HEIGHT;
            return if top < offset {
                Some(top - GRID_PADDING)
            } else if bottom > offset + height {
                Some((bottom + GRID_PADDING - height).max(0.0))
            } else {
                None
            };
        }
        top += row_height(row);
    }
    None
}

fn selected_choice(multiplier: Option<f64>, dampen: f64) -> Choice {
    match multiplier {
        None => Choice::Fully,
        Some(1.0) => Choice::Fully,
        Some(factor) if factor == dampen => Choice::Half,
        Some(0.0) => Choice::Not,
        Some(factor) => Choice::Custom(factor),
    }
}

fn choices(selected: Choice) -> Vec<Choice> {
    let mut choices = vec![Choice::Fully, Choice::Half, Choice::Not];
    if let Choice::Custom(_) = selected {
        choices.push(selected);
    }
    choices
}

fn choice_multiplier(choice: Choice, dampen: f64) -> Option<f64> {
    match choice {
        Choice::Fully => None,
        Choice::Half => Some(dampen),
        Choice::Not => Some(0.0),
        Choice::Custom(factor) => Some(factor),
    }
}

fn choice_factor(choice: Choice, dampen: f64) -> String {
    multiplier(choice_multiplier(choice, dampen).unwrap_or(1.0))
}

fn choice_label(choice: Choice) -> &'static str {
    match choice {
        Choice::Fully => "Fully",
        Choice::Half => "Half",
        Choice::Not => "Not",
        Choice::Custom(_) => "Custom",
    }
}

fn choice_tone(choice: Choice) -> Tone {
    match choice {
        Choice::Fully => Tone::Success,
        Choice::Half => Tone::Warning,
        Choice::Not => Tone::Danger,
        Choice::Custom(_) => Tone::Primary,
    }
}

fn tile_origin(rows: &[Row], position: usize, width: f32, offset: f32) -> Option<Point> {
    let free = width - 2.0 * GRID_PADDING - grid_width(columns(width));
    let left = GRID_PADDING + (free / 2.0).max(0.0);
    let mut top = GRID_PADDING - offset;
    for row in rows {
        if let Row::Games(games) = row
            && games.contains(&position)
        {
            let column = (position - games.start) as f32;
            return Some(Point::new(left + column * (TILE_WIDTH + GAP), top));
        }
        top += row_height(row);
    }
    None
}

fn popup_origin(tile: Point, area: Size) -> Point {
    let right = tile.x + TILE_WIDTH + POPUP_GAP;
    let x = if right + POPUP_WIDTH + POPUP_MARGIN <= area.width {
        right
    } else {
        tile.x - POPUP_GAP - POPUP_WIDTH
    };
    let y = tile.y.min(area.height - POPUP_HEIGHT - POPUP_MARGIN);

    Point::new(x.max(POPUP_MARGIN), y.max(POPUP_MARGIN))
}

fn counts_as(hours: f64) -> String {
    format!("counts as {}", hours_text(hours))
}

fn source_line(game: &LibraryGameView, view: &LibraryView) -> String {
    let signals: Vec<&str> = [
        (game.owned_hours.is_some(), "Owned"),
        (game.borrowed, "Not owned"),
        (game.wishlist, "Wishlist"),
        (game.followed, "Followed"),
        (game.elsewhere, "Played elsewhere"),
    ]
    .into_iter()
    .filter(|&(is_in, _)| is_in)
    .map(|(_, name)| name)
    .collect();
    let mut parts = vec![match signals.as_slice() {
        [] => "Played".to_string(),
        [one] => one.to_string(),
        [first, rest @ ..] => format!("{first} and {}", rest.join(" and ").to_lowercase()),
    }];

    parts.push(match (game.source, game.owned_hours) {
        (HoursSource::Played, Some(owned)) => hours(owned),
        (HoursSource::Played, None) => hours(game.hours),
        (HoursSource::Wishlist, _) => counts_as(view.wishlist_hours),
        (HoursSource::Followed, _) => counts_as(view.followed_hours),
        (HoursSource::WouldTry, _) => counts_as(game.hours),
        (HoursSource::Elsewhere, _) => counts_as(view.elsewhere_hours),
    });
    if game.recent {
        parts.push("played in the last two weeks".to_string());
    }
    parts.join(" · ")
}

fn ordinal(number: usize) -> String {
    let suffix = match (number % 10, number % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

fn weight_line(game: &LibraryGameView, view: &LibraryView) -> String {
    if game.counts {
        let counting: Vec<&LibraryGameView> =
            view.games.iter().filter(|other| other.counts).collect();
        let name = folded(&game.name);
        let heavier = counting
            .iter()
            .filter(|other| {
                other.share > game.share
                    || (other.share == game.share && folded(&other.name) < name)
            })
            .count();
        format!(
            "{} of {} · {} the average game",
            ordinal(heavier + 1),
            counting.len(),
            format::affinity(game.share * counting.len() as f64)
        )
    } else if game.hours < view.min_hours {
        format!(
            "{}, under the {} that count",
            hours(game.hours),
            hours_text(view.min_hours)
        )
    } else if game.correction.multiplier == Some(0.0) {
        "Set to ×0, so it doesn't count".to_string()
    } else {
        "No tags, so it doesn't count".to_string()
    }
}

fn hours_text(hours: f64) -> String {
    if hours == 1.0 {
        "1 hour".to_string()
    } else {
        format!("{hours} hours")
    }
}

fn no_effect_text(game_hours: f64, min_hours: f64) -> String {
    format!(
        "It has {}, under {}, so a correction changes nothing",
        hours(game_hours),
        hours_text(min_hours)
    )
}

fn corrected_text(
    name: &str,
    before: GameCorrection,
    after: GameCorrection,
    dampen: f64,
) -> String {
    if before.hidden != after.hidden {
        return if after.hidden {
            format!("{name} is hidden from Discover")
        } else {
            format!("{name} shows in Discover again")
        };
    }
    match selected_choice(after.multiplier, dampen) {
        Choice::Fully => format!("{name} counts fully"),
        Choice::Half => format!("{name} counts half"),
        Choice::Not => format!("{name} doesn't count"),
        Choice::Custom(factor) => format!("{name} counts {}", multiplier(factor)),
    }
}

fn empty_text(no_games: bool, filter: Filter, search: &str) -> String {
    let search = search.trim();
    if no_games {
        "No games in your Signals yet".to_string()
    } else if !search.is_empty() {
        format!("No game matches \"{search}\"")
    } else {
        match filter {
            Filter::All | Filter::Owned => "No games in your library".to_string(),
            Filter::NotOwned => "No games played without owning them".to_string(),
            Filter::Wishlist => "No games in your wishlist".to_string(),
            Filter::Followed => "No followed games".to_string(),
            Filter::Elsewhere => "No games played elsewhere".to_string(),
            Filter::Corrected => "No corrected games".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: u32, name: &str) -> LibraryGameView {
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

    fn owned(id: u32, name: &str, hours: f64, share: f64) -> LibraryGameView {
        LibraryGameView {
            owned_hours: Some(hours),
            hours,
            counts: share > 0.0,
            share,
            ..game(id, name)
        }
    }

    fn games() -> Vec<LibraryGameView> {
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

    fn ids(games: &[LibraryGameView], shown: &[usize]) -> Vec<u32> {
        shown.iter().map(|&index| games[index].id.0).collect()
    }

    #[test]
    fn the_columns_fit_the_width() {
        assert_eq!(columns(1280.0), 7);
        assert_eq!(columns(1400.0), 8);
        assert_eq!(columns(100.0), 1);
    }

    #[test]
    fn the_grid_is_as_wide_as_its_columns() {
        assert_eq!(grid_width(1), TILE_WIDTH);
        assert_eq!(grid_width(7), 7.0 * TILE_WIDTH + 6.0 * GAP);
        assert!(grid_width(columns(2560.0)) <= 2560.0 - 2.0 * GRID_PADDING);
    }

    #[test]
    fn taste_weight_puts_the_games_that_do_not_count_last_by_hours() {
        let games = games();

        let shown = shown(&games, Filter::All, Sort::TasteWeight, "");

        assert_eq!(ids(&games, &shown), [2, 1, 4, 5, 3, 6]);
    }

    #[test]
    fn hours_and_name_sort_every_game_together() {
        let games = games();

        let by_hours = shown(&games, Filter::All, Sort::Hours, "");
        let by_name = shown(&games, Filter::All, Sort::Name, "");

        assert_eq!(ids(&games, &by_hours), [2, 1, 4, 5, 3, 6]);
        assert_eq!(ids(&games, &by_name), [6, 3, 2, 5, 1, 4]);
    }

    #[test]
    fn a_filter_keeps_one_signal() {
        let games = games();
        let kept = |filter| ids(&games, &shown(&games, filter, Sort::Name, ""));

        assert_eq!(kept(Filter::Owned), [6, 3, 2, 1]);
        assert_eq!(kept(Filter::Wishlist), [4]);
        assert_eq!(kept(Filter::Followed), [5]);
        assert_eq!(count(&games, Filter::All), 6);
        assert_eq!(count(&games, Filter::Owned), 4);
    }

    #[test]
    fn the_search_ignores_case_accents_and_the_filter() {
        let games = games();
        let found = |search| ids(&games, &shown(&games, Filter::Wishlist, Sort::Name, search));

        assert_eq!(found("okami"), [5]);
        assert_eq!(found("  HADES "), [2]);
        assert_eq!(found("e"), [6, 3, 2, 1, 4]);
        assert!(found("xyz").is_empty());
    }

    #[test]
    fn the_rows_break_at_the_divider() {
        assert_eq!(
            rows(3, 7, 0, 2, true),
            [
                Row::Games(0..2),
                Row::Games(2..3),
                Row::Divider,
                Row::Games(3..5),
                Row::Games(5..7),
            ]
        );
        assert_eq!(
            rows(3, 7, 0, 4, false),
            [Row::Games(0..4), Row::Games(4..7)]
        );
        assert_eq!(rows(5, 5, 0, 4, true), [Row::Games(0..4), Row::Games(4..5)]);
        assert_eq!(rows(0, 2, 0, 4, true), [Row::Divider, Row::Games(0..2)]);
    }

    #[test]
    fn only_the_rows_in_view_are_drawn() {
        let rows = rows(20, 40, 0, 4, true);
        let row = TILE_HEIGHT + GAP;

        assert_eq!(visible_rows(&rows, 0.0, row), 0..1);
        assert_eq!(visible_rows(&rows, 0.0, row + GRID_PADDING + 1.0), 0..2);
        assert_eq!(visible_rows(&rows, GRID_PADDING + row * 5.0, 1.0), 5..6);
        assert_eq!(
            visible_rows(&rows, GRID_PADDING + row * 5.0, DIVIDER_HEIGHT + 1.0),
            5..7
        );
        assert_eq!(visible_rows(&rows, 100_000.0, 500.0), 11..11);
        assert_eq!(visible_rows(&rows, -5_000.0, 1.0), 0..0);
    }

    #[test]
    fn loads_the_rows_in_view_then_two_screens_the_way_it_scrolls() {
        let rows = rows(20, 40, 0, 4, true);
        let row = TILE_HEIGHT + GAP;

        assert_eq!(
            wanted_rows(&rows, GRID_PADDING + row * 2.0, row, false),
            [2, 3, 4]
        );
        assert_eq!(
            wanted_rows(&rows, GRID_PADDING + row * 4.0, row, true),
            [4, 3, 2]
        );
    }

    #[test]
    fn a_tile_shows_its_hours_or_where_it_comes_from() {
        let games = games();

        assert_eq!(tile_label(&games[1]), "120 h");
        assert_eq!(tile_label(&games[2]), "1.0 h");
        assert_eq!(tile_label(&games[3]), "wishlist");
        assert_eq!(tile_label(&games[4]), "followed");
        assert_eq!(tile_label(&games[5]), "0 h");
        let borrowed = LibraryGameView {
            recent: true,
            hours: 6.0,
            ..game(7, "Drova")
        };
        assert_eq!(tile_label(&borrowed), "6.0 h");
    }

    #[test]
    fn a_tile_names_the_signal_its_hours_come_from() {
        let followed = LibraryGameView {
            followed: true,
            hours: 3.0,
            source: HoursSource::Followed,
            ..owned(7, "Paradise Killer", 0.0, 0.01)
        };
        let would_try = LibraryGameView {
            hours: 4.0,
            source: HoursSource::WouldTry,
            ..game(8, "Tunic")
        };

        assert_eq!(tile_label(&followed), "followed");
        assert_eq!(tile_label(&would_try), "would try");
        assert_eq!(tile_label(&owned(9, "Elden Ring", 376.0, 0.3)), "376 h");
    }

    #[test]
    fn the_weight_bar_is_as_long_as_the_share_of_the_heaviest_game() {
        let games = games();
        let heaviest = heaviest(&games);

        assert_eq!(heaviest, 0.5);
        assert_eq!(weight_fraction(games[1].share, heaviest), Some(1.0));
        assert_eq!(weight_fraction(games[0].share, heaviest), Some(0.4));
        assert_eq!(weight_fraction(games[2].share, heaviest), None);
        assert_eq!(weight_fraction(0.1, 0.0), None);
    }

    fn view_of(games: Vec<LibraryGameView>) -> LibraryView {
        LibraryView {
            games,
            families: Vec::new(),
            min_hours: 2.0,
            wishlist_hours: 5.0,
            followed_hours: 3.0,
            elsewhere_hours: 20.0,
            dampen: 0.5,
        }
    }

    #[test]
    fn the_selected_choice_follows_the_multiplier() {
        assert_eq!(selected_choice(None, 0.5), Choice::Fully);
        assert_eq!(selected_choice(Some(1.0), 0.5), Choice::Fully);
        assert_eq!(selected_choice(Some(0.5), 0.5), Choice::Half);
        assert_eq!(selected_choice(Some(0.0), 0.5), Choice::Not);
        assert_eq!(selected_choice(Some(0.25), 0.5), Choice::Custom(0.25));
        assert_eq!(selected_choice(Some(0.5), 0.6), Choice::Custom(0.5));
    }

    #[test]
    fn a_factor_written_by_hand_is_a_fourth_choice() {
        assert_eq!(
            choices(Choice::Half),
            [Choice::Fully, Choice::Half, Choice::Not]
        );
        assert_eq!(
            choices(Choice::Custom(0.25)),
            [
                Choice::Fully,
                Choice::Half,
                Choice::Not,
                Choice::Custom(0.25)
            ]
        );
        assert_eq!(choice_factor(Choice::Custom(0.25), 0.5), "×0.25");
        assert_eq!(choice_label(Choice::Custom(0.25)), "Custom");
    }

    #[test]
    fn each_choice_shows_its_factor_and_its_colour() {
        assert_eq!(choice_factor(Choice::Fully, 0.5), "×1");
        assert_eq!(choice_factor(Choice::Half, 0.5), "×0.5");
        assert_eq!(choice_factor(Choice::Half, 0.6), "×0.6");
        assert_eq!(choice_factor(Choice::Not, 0.5), "×0");
        assert_eq!(choice_tone(Choice::Fully), Tone::Success);
        assert_eq!(choice_tone(Choice::Half), Tone::Warning);
        assert_eq!(choice_tone(Choice::Not), Tone::Danger);
        assert_eq!(choice_tone(Choice::Custom(0.25)), Tone::Primary);
    }

    #[test]
    fn the_tile_corner_follows_the_column_the_divider_and_the_scroll() {
        let width = 2.0 * GRID_PADDING + grid_width(4);
        let rows = rows(5, 9, 0, 4, true);

        assert_eq!(
            tile_origin(&rows, 0, width, 0.0),
            Some(Point::new(GRID_PADDING, GRID_PADDING))
        );
        assert_eq!(
            tile_origin(&rows, 6, width, 100.0),
            Some(Point::new(
                GRID_PADDING + TILE_WIDTH + GAP,
                GRID_PADDING + 2.0 * (TILE_HEIGHT + GAP) + DIVIDER_HEIGHT - 100.0
            ))
        );
        assert_eq!(
            tile_origin(&rows, 0, width + 100.0, 0.0),
            Some(Point::new(GRID_PADDING + 50.0, GRID_PADDING))
        );
        assert_eq!(tile_origin(&rows, 99, width, 0.0), None);
    }

    #[test]
    fn the_popup_opens_beside_the_tile_and_stays_in_the_grid() {
        let area = Size::new(1280.0, 700.0);
        let first = Point::new(GRID_PADDING, GRID_PADDING);
        let last = Point::new(area.width - GRID_PADDING - TILE_WIDTH, GRID_PADDING);

        assert_eq!(
            popup_origin(first, area),
            Point::new(first.x + TILE_WIDTH + POPUP_GAP, first.y)
        );
        assert_eq!(popup_origin(last, area).x, last.x - POPUP_GAP - POPUP_WIDTH);
        assert_eq!(
            popup_origin(Point::new(first.x, 600.0), area).y,
            area.height - POPUP_HEIGHT - POPUP_MARGIN
        );
        assert_eq!(
            popup_origin(Point::new(first.x, -100.0), area).y,
            POPUP_MARGIN
        );
    }

    #[test]
    fn fully_removes_the_multiplier() {
        assert_eq!(choice_multiplier(Choice::Fully, 0.5), None);
        assert_eq!(choice_multiplier(Choice::Half, 0.6), Some(0.6));
        assert_eq!(choice_multiplier(Choice::Not, 0.5), Some(0.0));
        assert_eq!(choice_multiplier(Choice::Custom(0.25), 0.5), Some(0.25));
    }

    #[test]
    fn the_source_line_says_every_signal_and_the_winning_hours() {
        let games = games();
        let view = view_of(Vec::new());
        let line = |game: &LibraryGameView| source_line(game, &view);

        assert_eq!(line(&games[1]), "Owned · 120 h");
        assert_eq!(line(&games[3]), "Wishlist · counts as 5 hours");
        assert_eq!(line(&games[4]), "Followed · counts as 3 hours");
        let both = LibraryGameView {
            followed: true,
            recent: true,
            ..games[0].clone()
        };
        assert_eq!(
            line(&both),
            "Owned and followed · 40 h · played in the last two weeks"
        );
        let borrowed = LibraryGameView {
            recent: true,
            hours: 6.0,
            ..game(7, "Drova")
        };
        assert_eq!(
            line(&borrowed),
            "Played · 6.0 h · played in the last two weeks"
        );
        let shared = LibraryGameView {
            borrowed: true,
            hours: 147.8,
            ..game(8, "Baldur's Gate 3")
        };
        assert_eq!(line(&shared), "Not owned · 148 h");
    }

    #[test]
    fn not_owned_keeps_the_games_played_without_owning_them() {
        let mut games = games();
        games.push(LibraryGameView {
            borrowed: true,
            hours: 40.0,
            ..game(7, "Baldur's Gate 3")
        });

        assert_eq!(
            ids(&games, &shown(&games, Filter::NotOwned, Sort::Name, "")),
            [7]
        );
        assert_eq!(count(&games, Filter::Owned), 4);
        assert_eq!(count(&games, Filter::All), 7);
        assert_eq!(
            empty_text(false, Filter::NotOwned, ""),
            "No games played without owning them"
        );
    }

    #[test]
    fn corrected_keeps_the_games_with_a_correction_or_held_out() {
        let mut games = games();
        games.push(LibraryGameView {
            correction: GameCorrection {
                multiplier: Some(0.5),
                hidden: false,
            },
            ..owned(7, "Valheim", 266.0, 0.1)
        });
        games.push(LibraryGameView {
            correction: GameCorrection {
                multiplier: None,
                hidden: true,
            },
            ..game(8, "Splitgate")
        });
        games.push(LibraryGameView {
            held_out: true,
            ..owned(9, "Factorio", 90.0, 0.1)
        });
        games.push(LibraryGameView {
            families: vec!["Strategy".to_string()],
            ..owned(10, "Civilization VI", 70.0, 0.1)
        });

        assert_eq!(
            ids(&games, &shown(&games, Filter::Corrected, Sort::Name, "")),
            [9, 8, 7]
        );
        assert_eq!(count(&games, Filter::Corrected), 3);
        assert_eq!(count(&games[..6], Filter::Corrected), 0);
        assert!(Filter::Corrected.hides_when_empty());
        assert!(!Filter::Followed.hides_when_empty());
        assert_eq!(
            empty_text(false, Filter::Corrected, ""),
            "No corrected games"
        );
    }

    #[test]
    fn the_weight_line_says_how_much_or_why_not() {
        let games = games();
        let view = view_of(games.clone());
        let line = |game: &LibraryGameView| weight_line(game, &view);

        assert_eq!(line(&games[1]), "1st of 4 · 2× the average game");
        assert_eq!(line(&games[0]), "2nd of 4 · 0.8× the average game");
        assert_eq!(line(&games[4]), "4th of 4 · 0.2× the average game");
        assert_eq!(line(&games[2]), "1.0 h, under the 2 hours that count");
        let zero = LibraryGameView {
            counts: false,
            correction: GameCorrection {
                multiplier: Some(0.0),
                hidden: false,
            },
            ..games[1].clone()
        };
        assert_eq!(line(&zero), "Set to ×0, so it doesn't count");
        assert_eq!(
            no_effect_text(0.5, 2.0),
            "It has 0.5 h, under 2 hours, so a correction changes nothing"
        );
    }

    #[test]
    fn the_place_takes_its_english_ordinal() {
        let ordinals: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 111, 374]
            .into_iter()
            .map(ordinal)
            .collect();

        assert_eq!(
            ordinals,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st",
                "111th", "374th"
            ]
        );
    }

    #[test]
    fn the_toast_says_what_changed() {
        let none = GameCorrection::default();
        let with = |multiplier, hidden| GameCorrection { multiplier, hidden };

        assert_eq!(
            corrected_text("Black Ops II", none, with(Some(0.5), false), 0.5),
            "Black Ops II counts half"
        );
        assert_eq!(
            corrected_text("Hades", with(Some(0.5), false), none, 0.5),
            "Hades counts fully"
        );
        assert_eq!(
            corrected_text("Hades", none, with(Some(0.0), false), 0.5),
            "Hades doesn't count"
        );
        assert_eq!(
            corrected_text(
                "Hades",
                with(Some(0.0), false),
                with(Some(0.25), false),
                0.5
            ),
            "Hades counts ×0.25"
        );
        assert_eq!(
            corrected_text("Pentiment", none, with(None, true), 0.5),
            "Pentiment is hidden from Discover"
        );
        assert_eq!(
            corrected_text("Pentiment", with(None, true), none, 0.5),
            "Pentiment shows in Discover again"
        );
    }

    #[test]
    fn the_open_game_is_scrolled_into_view() {
        let rows = rows(20, 40, 0, 4, true);
        let row = TILE_HEIGHT + GAP;

        assert_eq!(reveal_offset(&rows, 0, 0.0, 600.0), None);
        assert_eq!(reveal_offset(&rows, 0, 100.0, 600.0), Some(0.0));
        let below = reveal_offset(&rows, 24, 0.0, 600.0).unwrap();
        assert_eq!(
            below,
            2.0 * GRID_PADDING + row * 6.0 + DIVIDER_HEIGHT + TILE_HEIGHT - 600.0
        );
        assert_eq!(reveal_offset(&rows, 99, 0.0, 600.0), None);
    }

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

    #[test]
    fn the_arrows_walk_the_grid_across_the_divider() {
        let rows = rows(5, 9, 0, 4, true);
        let to = |position, direction| moved(&rows, position, direction);

        assert_eq!(to(None, Move::Down), Some(0));
        assert_eq!(to(Some(0), Move::Left), Some(0));
        assert_eq!(to(Some(3), Move::Right), Some(4));
        assert_eq!(to(Some(8), Move::Right), Some(8));
        assert_eq!(to(Some(2), Move::Down), Some(4));
        assert_eq!(to(Some(4), Move::Down), Some(5));
        assert_eq!(to(Some(7), Move::Up), Some(4));
        assert_eq!(to(Some(5), Move::Up), Some(4));
        assert_eq!(to(Some(1), Move::Up), Some(1));
        assert_eq!(to(Some(6), Move::Down), Some(6));
        assert_eq!(moved(&[], None, Move::Down), None);
    }

    #[test]
    fn the_badge_names_the_correction() {
        let badge = |multiplier, hidden| correction_badge(GameCorrection { multiplier, hidden });

        assert_eq!(badge(None, false), None);
        assert_eq!(badge(Some(0.5), false).as_deref(), Some("×0.5"));
        assert_eq!(badge(Some(0.0), false).as_deref(), Some("×0"));
        assert_eq!(badge(None, true).as_deref(), Some("hidden"));
        assert_eq!(badge(Some(0.25), true).as_deref(), Some("×0.25 · hidden"));
    }

    #[test]
    fn the_divider_says_why_a_game_does_not_count() {
        assert_eq!(
            divider_note(2.0),
            "Under 2 hours of play, set to ×0, or without tags"
        );
        assert_eq!(
            divider_note(1.0),
            "Under 1 hour of play, set to ×0, or without tags"
        );
        assert_eq!(
            divider_note(0.5),
            "Under 0.5 hours of play, set to ×0, or without tags"
        );
    }

    #[test]
    fn a_chip_puts_the_game_in_or_takes_it_out() {
        assert_eq!(
            chip_change("cozy", AppId(1), false),
            FamilyChange::AddGame {
                family: "cozy".to_string(),
                game: AppId(1),
            }
        );
        assert_eq!(
            chip_change("cozy", AppId(1), true),
            FamilyChange::RemoveGame {
                family: "cozy".to_string(),
                game: AppId(1),
            }
        );
    }

    #[test]
    fn the_toast_says_what_changed_in_the_families() {
        let mut celeste = game(1, "Celeste");
        let add = FamilyChange::AddGame {
            family: " slow puzzles ".to_string(),
            game: AppId(1),
        };
        let remove = FamilyChange::RemoveGame {
            family: "cozy".to_string(),
            game: AppId(1),
        };
        let hold_out = FamilyChange::HoldOut { game: AppId(1) };

        assert_eq!(families_text(&add, &celeste, &[]), "Added to slow puzzles");
        assert_eq!(families_text(&hold_out, &celeste, &[]), "Held out");
        assert_eq!(
            families_text(&FamilyChange::Release { game: AppId(1) }, &celeste, &[]),
            "No longer held out"
        );
        assert_eq!(families_text(&remove, &celeste, &[]), "Removed from cozy");
        assert_eq!(
            families_text(&remove, &celeste, &["cozy".to_string()]),
            "Removed from cozy, which is now gone"
        );

        celeste.families = vec!["cozy".to_string(), "platformers".to_string()];
        assert_eq!(
            families_text(&hold_out, &celeste, &[]),
            "Held out, removed from cozy, platformers"
        );
        celeste.held_out = true;
        assert_eq!(
            families_text(&add, &celeste, &[]),
            "Added to slow puzzles, no longer held out"
        );
    }

    fn elsewhere(id: u32, name: &str) -> LibraryGameView {
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

    fn found(id: u32, name: &str) -> CatalogMatch {
        CatalogMatch {
            id: AppId(id),
            name: name.to_string(),
        }
    }

    #[test]
    fn the_games_from_steam_come_after_the_library_under_their_heading() {
        assert_eq!(
            rows(2, 3, 3, 2, true),
            [
                Row::Games(0..2),
                Row::Divider,
                Row::Games(2..3),
                Row::FromSteam,
                Row::Games(3..5),
                Row::Games(5..6),
            ]
        );
        assert_eq!(rows(3, 3, 0, 2, true), [Row::Games(0..2), Row::Games(2..3)]);
        assert!(rows(0, 0, 0, 2, true).is_empty());
    }

    #[test]
    fn without_library_matches_the_no_match_line_stays_above_the_games_from_steam() {
        assert_eq!(
            rows(0, 0, 3, 2, true),
            [
                Row::NoMatch,
                Row::FromSteam,
                Row::Games(0..2),
                Row::Games(2..3)
            ]
        );
        assert_eq!(
            empty_text(false, Filter::All, "it takes"),
            "No game matches \"it takes\""
        );
    }

    #[test]
    fn the_arrows_walk_from_the_library_to_the_games_from_steam_and_back() {
        let rows = rows(3, 3, 2, 2, true);
        let to = |position, direction| moved(&rows, Some(position), direction);

        assert_eq!(to(2, Move::Down), Some(3));
        assert_eq!(to(1, Move::Down), Some(2));
        assert_eq!(to(2, Move::Right), Some(3));
        assert_eq!(to(4, Move::Up), Some(2));
        assert_eq!(to(3, Move::Left), Some(2));
        assert_eq!(enter(2, 3), Enter::Open);
        assert_eq!(enter(3, 3), Enter::Add);
    }

    #[test]
    fn the_heading_counts_in_the_scroll_and_in_the_popup_place() {
        let rows = rows(0, 0, 3, 2, true);

        assert_eq!(
            tile_origin(&rows, 0, 2.0 * GRID_PADDING + grid_width(2), 0.0),
            Some(Point::new(
                GRID_PADDING,
                GRID_PADDING + NO_MATCH_HEIGHT + DIVIDER_HEIGHT
            ))
        );
        assert_eq!(
            reveal_offset(&rows, 2, 0.0, 300.0),
            Some(
                GRID_PADDING
                    + NO_MATCH_HEIGHT
                    + DIVIDER_HEIGHT
                    + 2.0 * TILE_HEIGHT
                    + GAP
                    + GRID_PADDING
                    - 300.0
            )
        );
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
            elsewhere_text("It Takes Two", true),
            "Added It Takes Two as played elsewhere"
        );
        assert_eq!(
            elsewhere_text("It Takes Two", false),
            "Removed It Takes Two"
        );
        assert!(matches!(
            character("z", keyboard::Modifiers::CTRL),
            Some(Message::Undo)
        ));
        assert!(matches!(named(Named::Enter), Some(Message::OpenSelected)));
    }

    #[test]
    fn only_a_game_played_only_elsewhere_can_be_removed_from_the_library() {
        let only = elsewhere(7, "It Takes Two");
        let owned_too = LibraryGameView {
            elsewhere_only: false,
            ..owned(8, "Portal 2", 0.0, 0.1)
        };

        assert!(matches!(
            removal(&only),
            Some(Message::PlayElsewhere(AppId(7), _, false))
        ));
        assert!(removal(&owned_too).is_none());
        assert!(removal(&games()[0]).is_none());
    }

    #[test]
    fn the_elsewhere_filter_shows_with_the_first_game_played_elsewhere() {
        let mut games = games();
        assert!(!shows_filter(Filter::Elsewhere, &games));
        assert!(shows_filter(Filter::Followed, &games));

        games.push(elsewhere(7, "It Takes Two"));
        assert!(shows_filter(Filter::Elsewhere, &games));
        assert_eq!(
            ids(&games, &shown(&games, Filter::Elsewhere, Sort::Name, "")),
            [7]
        );
        assert_eq!(filter_label(Filter::Elsewhere), "Elsewhere");

        games.pop();
        assert!(!shows_filter(Filter::Elsewhere, &games));
        assert_eq!(
            empty_text(false, Filter::Elsewhere, ""),
            "No games played elsewhere"
        );
    }

    #[test]
    fn adding_a_game_changes_what_the_search_field_asks() {
        assert_eq!(placeholder(false), "Search by name (Ctrl+F)");
        assert_eq!(
            placeholder(true),
            "Search Steam for a game you played elsewhere"
        );
    }

    #[test]
    fn a_game_played_elsewhere_says_so_on_its_tile_and_in_the_popup() {
        let view = view_of(Vec::new());
        let only = elsewhere(7, "It Takes Two");
        let owned_too = LibraryGameView {
            elsewhere: true,
            ..owned(8, "Portal 2", 40.0, 0.1)
        };

        assert_eq!(tile_label(&only), "elsewhere");
        assert_eq!(
            source_line(&only, &view),
            "Played elsewhere · counts as 20 hours"
        );
        assert_eq!(
            source_line(&owned_too, &view),
            "Owned and played elsewhere · 40 h"
        );
    }

    #[test]
    fn an_empty_grid_says_why() {
        assert_eq!(
            empty_text(true, Filter::All, ""),
            "No games in your Signals yet"
        );
        assert_eq!(
            empty_text(false, Filter::All, " xyz "),
            "No game matches \"xyz\""
        );
        assert_eq!(
            empty_text(false, Filter::Wishlist, ""),
            "No games in your wishlist"
        );
    }
}
