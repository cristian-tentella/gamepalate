use std::{fmt, ops::Range, path::PathBuf, process::Command, sync::Arc, time::Duration};

use gamepalate_app::{
    AppId, Capsule, GameCorrection, Gamepalate, HoursSource, LibraryGameView, LibraryView,
};
use iced::{
    Center, ContentFit, Element, Fill, Size, Subscription, Task,
    keyboard::{self, Key, key::Named},
    mouse,
    widget::{
        Id, button, checkbox, column, container, image, mouse_area, opaque, operation, pick_list,
        responsive, row, scrollable,
        scrollable::{AbsoluteOffset, Viewport},
        space, stack, text, text_input, tooltip,
    },
    window,
};

use super::showcase::images::{Images, Picture};
use crate::{font, style};

const FILTERS: [Filter; 4] = [
    Filter::All,
    Filter::Owned,
    Filter::Wishlist,
    Filter::Followed,
];
const SORTS: [Sort; 3] = [Sort::TasteWeight, Sort::Hours, Sort::Name];
const GRID: Id = Id::new("library-grid");
const SEARCH: Id = Id::new("library-search");
const TILE_WIDTH: f32 = 150.0;
const TILE_HEIGHT: f32 = 225.0;
const GAP: f32 = 12.0;
const GRID_PADDING: f32 = 24.0;
const DIVIDER_HEIGHT: f32 = 56.0;
const DIMMED: f32 = 0.45;
const SCREENS_AHEAD: f32 = 1.0;
const SCREENS_KEPT: f32 = 2.0;
const CHROME_HEIGHT: f32 = 110.0;
const IMAGE_RADIUS: f32 = 6.0;
const DEFAULT_AREA: Size = Size::new(1280.0, 700.0);
const PANEL_WIDTH: f32 = 360.0;
const PANEL_CAPSULE_WIDTH: f32 = 160.0;
const PANEL_CAPSULE_HEIGHT: f32 = 240.0;
const TOAST_DURATION: Duration = Duration::from_secs(5);

pub struct Library {
    gamepalate: Arc<Gamepalate>,
    data_dir: PathBuf,
    view: LibraryView,
    open: Option<AppId>,
    selected: Option<AppId>,
    reveal_pending: bool,
    last_correction: Option<(AppId, GameCorrection)>,
    toast: Option<Toast>,
    toasts_shown: u64,
    filter: Filter,
    sort: Sort,
    search: String,
    shown: Vec<usize>,
    offset: f32,
    area: Size,
    capsules: Images<image::Allocation, AppId>,
}

struct Toast {
    id: u64,
    text: String,
    undo: bool,
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
    Wishlist,
    Followed,
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
    OpenDataFolder,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Games(Range<usize>),
    Divider,
}

impl Library {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Library, Task<Message>) {
        let mut library = Library {
            gamepalate: Arc::clone(gamepalate),
            data_dir: gamepalate.data_dir().to_path_buf(),
            view: gamepalate.library_view(),
            open: None,
            selected: None,
            reveal_pending: false,
            last_correction: None,
            toast: None,
            toasts_shown: 0,
            filter: Filter::All,
            sort: Sort::TasteWeight,
            search: String::new(),
            shown: Vec::new(),
            offset: 0.0,
            area: DEFAULT_AREA,
            capsules: Images::default(),
        };
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
                return self.refresh();
            }
            Message::Sort(sort) => {
                self.sort = sort;
                return self.refresh();
            }
            Message::Search(search) => {
                self.search = search;
                return self.refresh();
            }
            Message::Scrolled(viewport) => {
                self.offset = viewport.absolute_offset().y;
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
            }
            Message::Close => self.open = None,
            Message::Move(direction) => {
                let rows = self.rows(self.area.width);
                let position = self.selected_position();
                let Some(next) = moved(&rows, position, direction) else {
                    return Task::none();
                };
                let id = self.view.games[self.shown[next]].id;
                self.selected = Some(id);
                if self.open.is_some() {
                    self.open = Some(id);
                }
                return self.reveal_selected();
            }
            Message::OpenSelected => {
                if self.selected_position().is_some() {
                    self.open = self.selected;
                }
            }
            Message::Pick(index) => {
                let view = &self.view;
                let Some(game) = self.open.and_then(|id| self.game(id)) else {
                    return Task::none();
                };
                let choice = [Choice::Fully, Choice::Half, Choice::Not][index.min(2)];
                if game.hours >= view.min_hours
                    && choice != selected_choice(game.correction.multiplier, view.dampen)
                {
                    let correction = GameCorrection {
                        multiplier: choice_multiplier(choice, view.dampen),
                        ..game.correction
                    };
                    return self.update(Message::Correct(game.id, correction));
                }
            }
            Message::FocusSearch => return operation::focus(SEARCH),
            Message::Correct(id, correction) => {
                let gamepalate = Arc::clone(&self.gamepalate);
                return Task::perform(
                    async move {
                        gamepalate
                            .correct_game(id, correction)
                            .map_err(|error| error.to_string())
                    },
                    move |result| Message::Corrected(id, correction, result),
                );
            }
            Message::Corrected(id, correction, Ok(before)) => {
                if before != correction {
                    self.last_correction = Some((id, before));
                }
                let reveal = self.reload();
                let text = self.game(id).map_or_else(String::new, |game| {
                    corrected_text(&game.name, before, correction, self.view.dampen)
                });
                return Task::batch([reveal, self.show_toast(text, before != correction)]);
            }
            Message::Corrected(_, _, Err(error)) | Message::Undone(Err(error)) => {
                return self.show_toast(error, false);
            }
            Message::Undo => {
                if let Some((id, before)) = self.last_correction.take() {
                    let gamepalate = Arc::clone(&self.gamepalate);
                    return Task::perform(
                        async move {
                            gamepalate
                                .correct_game(id, before)
                                .map(|_| ())
                                .map_err(|error| error.to_string())
                        },
                        Message::Undone,
                    );
                }
            }
            Message::Undone(Ok(())) => {
                let reveal = self.reload();
                return Task::batch([reveal, self.show_toast("Undone.".to_string(), false)]);
            }
            Message::HideToast(id) => {
                if self.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.toast = None;
                }
            }
            Message::OpenDataFolder => {
                let _ = Command::new("explorer").arg(&self.data_dir).spawn();
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

    fn game(&self, id: AppId) -> Option<&LibraryGameView> {
        self.view.games.iter().find(|game| game.id == id)
    }

    fn reload(&mut self) -> Task<Message> {
        self.view = self.gamepalate.library_view();
        self.shown = shown(&self.view.games, self.filter, self.sort, &self.search);
        self.reveal_selected()
    }

    fn selected_position(&self) -> Option<usize> {
        let id = self.selected?;
        self.shown
            .iter()
            .position(|&index| self.view.games[index].id == id)
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
        self.toasts_shown += 1;
        let id = self.toasts_shown;
        self.toast = Some(Toast { id, text, undo });

        Task::perform(tokio::time::sleep(TOAST_DURATION), move |_| {
            Message::HideToast(id)
        })
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
        let ids = |range: Range<usize>| -> Vec<AppId> {
            rows[range]
                .iter()
                .filter_map(|row| match row {
                    Row::Games(games) => Some(games.clone()),
                    Row::Divider => None,
                })
                .flatten()
                .map(|index| self.view.games[self.shown[index]].id)
                .collect()
        };
        let open = self.open.into_iter();
        let wanted: Vec<AppId> = open
            .clone()
            .chain(ids(visible_rows(
                &rows,
                self.offset,
                height * (1.0 + SCREENS_AHEAD),
            )))
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
        let filters = row(FILTERS.map(|filter| {
            button(
                text!(
                    "{} {}",
                    filter_label(filter),
                    count(&self.view.games, filter)
                )
                .size(font::NOTE_SIZE),
            )
            .padding([3, 10])
            .style(style::choice(!searching && filter == self.filter))
            .on_press(Message::Filter(filter))
            .into()
        }))
        .spacing(6);
        let tools = row![
            filters,
            space().width(Fill),
            text("Sort").size(font::NOTE_SIZE).style(style::muted),
            pick_list(SORTS, Some(self.sort), Message::Sort)
                .text_size(font::NOTE_SIZE)
                .padding([4, 8]),
            text_input("Search by name (Ctrl+F)", &self.search)
                .id(SEARCH)
                .on_input(Message::Search)
                .size(font::NOTE_SIZE)
                .padding([5, 8])
                .width(240),
        ]
        .spacing(8)
        .align_y(Center);

        let body: Element<'_, Message> = if self.shown.is_empty() {
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

        let panel = self.open.and_then(|id| self.game(id)).map(|game| {
            stack![
                opaque(mouse_area(space().width(Fill).height(Fill)).on_press(Message::Close)),
                container(opaque(
                    container(scrollable(self.panel(game)).height(Fill))
                        .width(PANEL_WIDTH)
                        .height(Fill)
                        .style(style::popup),
                ))
                .align_right(Fill)
                .padding(8),
            ]
        });
        let toast = self.toast.as_ref().map(|toast| {
            let undo = toast.undo.then(|| {
                button(text("Undo").size(font::NOTE_SIZE).font(font::SEMIBOLD))
                    .padding(0)
                    .style(style::toast_link)
                    .on_press(Message::Undo)
            });
            container(
                container(
                    row![text(&toast.text)]
                        .push(undo)
                        .spacing(14)
                        .align_y(Center),
                )
                .padding([8, 14])
                .style(style::toast),
            )
            .center_x(Fill)
            .align_bottom(Fill)
            .padding(16)
        });

        column![
            container(tools).padding([12, GRID_PADDING as u16]),
            style::separator(),
            stack![body].push(panel).push(toast),
        ]
        .into()
    }

    fn grid(&self, width: f32) -> Element<'_, Message> {
        let rows = self.rows(width);
        let shown = visible_rows(&rows, self.offset, self.area.height.max(1.0));
        let above: f32 = rows[..shown.start].iter().map(row_height).sum();
        let below: f32 = rows[shown.end..].iter().map(row_height).sum();

        let content = column![space().height(above)]
            .extend(rows[shown.clone()].iter().map(|line| self.line(line)))
            .push(space().height(below));

        scrollable(container(content).padding([GRID_PADDING, GRID_PADDING]))
            .id(GRID)
            .on_scroll(Message::Scrolled)
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn line<'a>(&'a self, line: &Row) -> Element<'a, Message> {
        match line {
            Row::Games(games) => row(self.shown[games.clone()]
                .iter()
                .map(|&index| self.tile(&self.view.games[index])))
            .spacing(GAP)
            .height(TILE_HEIGHT + GAP)
            .into(),
            Row::Divider => container(
                column![
                    text("Not shaping your taste")
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
        let strip = container(text(tile_label(game)).size(font::NOTE_SIZE))
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

    fn panel<'a>(&'a self, game: &'a LibraryGameView) -> Element<'a, Message> {
        let view = &self.view;
        let capsule: Element<'a, Message> = match self.capsules.get(&game.id) {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
                .width(PANEL_CAPSULE_WIDTH)
                .height(PANEL_CAPSULE_HEIGHT)
                .content_fit(ContentFit::Cover)
                .border_radius(IMAGE_RADIUS)
                .into(),
            _ => container(space())
                .width(PANEL_CAPSULE_WIDTH)
                .height(PANEL_CAPSULE_HEIGHT)
                .style(style::picture)
                .into(),
        };
        let tags = (!game.tags.is_empty()).then(|| {
            text!("Brings {}", game.tags.join(", "))
                .size(font::BAR_SIZE)
                .style(style::muted)
        });
        let correctable = game.hours >= view.min_hours;
        let selected = selected_choice(game.correction.multiplier, view.dampen);
        let choices = row(choices(selected).into_iter().map(|choice| {
            let correction = GameCorrection {
                multiplier: choice_multiplier(choice, view.dampen),
                ..game.correction
            };
            button(text(choice_label(choice, view.dampen)).size(font::NOTE_SIZE))
                .padding([4, 12])
                .style(style::choice(choice == selected))
                .on_press_maybe(
                    (correctable && choice != selected)
                        .then_some(Message::Correct(game.id, correction)),
                )
                .into()
        }))
        .spacing(6)
        .wrap()
        .vertical_spacing(6);
        let hide = game.owned_hours.is_none().then(|| {
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
        });

        column![
            capsule,
            text(&game.name).size(font::INFO_SIZE).font(font::SEMIBOLD),
            text(source_line(game, view)).size(font::BAR_SIZE),
            text(weight_line(game, view)).size(font::BAR_SIZE),
        ]
        .push(tags)
        .push(space().height(8))
        .push(
            text("Counts for your taste")
                .size(font::BAR_SIZE)
                .font(font::SEMIBOLD),
        )
        .push(choices)
        .push((!correctable).then(|| {
            text(no_effect_text(game.hours, view.min_hours))
                .size(font::NOTE_SIZE)
                .style(style::muted)
        }))
        .push(hide.map(|hide| column![space().height(8), hide]))
        .push(space().height(8))
        .push(
            column![
                text("To make a game count more, put it in a family.")
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                button(text("Open folder ›").size(font::NOTE_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenDataFolder),
            ]
            .spacing(2),
        )
        .spacing(8)
        .padding(20)
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
            Row::Divider => None,
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
            Filter::Wishlist => game.wishlist,
            Filter::Followed => game.followed,
        }
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
        Filter::Wishlist => "Wishlist",
        Filter::Followed => "Followed",
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

fn folded(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => 'u',
            'ý' | 'ÿ' => 'y',
            'ś' | 'š' => 's',
            'ź' | 'ż' | 'ž' => 'z',
            'ł' => 'l',
            'ř' => 'r',
            other => other,
        })
        .collect()
}

fn columns(width: f32) -> usize {
    let inner = width - 2.0 * GRID_PADDING;
    (((inner + GAP) / (TILE_WIDTH + GAP)).floor() as usize).max(1)
}

fn rows(counting: usize, total: usize, columns: usize, divided: bool) -> Vec<Row> {
    let chunks = |range: Range<usize>| {
        range
            .clone()
            .step_by(columns)
            .map(move |start| Row::Games(start..(start + columns).min(range.end)))
    };

    if divided && counting < total {
        chunks(0..counting)
            .chain([Row::Divider])
            .chain(chunks(counting..total))
            .collect()
    } else {
        chunks(0..total).collect()
    }
}

fn row_height(row: &Row) -> f32 {
    match row {
        Row::Games(_) => TILE_HEIGHT + GAP,
        Row::Divider => DIVIDER_HEIGHT,
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
    match (game.owned_hours, game.source) {
        (Some(owned), _) => hours(owned),
        (None, HoursSource::Played) => hours(game.hours),
        (None, _) if game.wishlist => "wishlist".to_string(),
        (None, _) => "followed".to_string(),
    }
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
        "Under {} of play, set to ×0, or without tags.",
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

fn choice_label(choice: Choice, dampen: f64) -> String {
    match choice {
        Choice::Fully => "Fully".to_string(),
        Choice::Half if dampen == 0.5 => "Half".to_string(),
        Choice::Half => format!("Half {}", multiplier(dampen)),
        Choice::Not => "Not".to_string(),
        Choice::Custom(factor) => multiplier(factor),
    }
}

fn counts_as(hours: f64) -> String {
    format!("counts as {}", hours_text(hours))
}

fn source_line(game: &LibraryGameView, view: &LibraryView) -> String {
    let signals: Vec<&str> = [
        (game.owned_hours.is_some(), "Owned"),
        (game.wishlist, "Wishlist"),
        (game.followed, "Followed"),
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
    });
    if game.recent {
        parts.push("played in the last two weeks".to_string());
    }
    parts.join(" · ")
}

fn share(share: f64) -> String {
    let percent = share * 100.0;
    if percent < 1.0 {
        "less than 1%".to_string()
    } else {
        format!("{percent:.0}%")
    }
}

fn weight_line(game: &LibraryGameView, view: &LibraryView) -> String {
    if game.counts {
        format!("Shapes {} of your taste", share(game.share))
    } else if game.hours < view.min_hours {
        format!(
            "Doesn't shape your taste: {}, under the {} that count.",
            hours(game.hours),
            hours_text(view.min_hours)
        )
    } else if game.correction.multiplier == Some(0.0) {
        "Doesn't shape your taste: set to ×0.".to_string()
    } else {
        "Doesn't shape your taste: it has no tags.".to_string()
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
        "Correcting it would have no effect: it has {}, under {}.",
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
            format!("{name} is hidden from Discover.")
        } else {
            format!("{name} shows in Discover again.")
        };
    }
    match selected_choice(after.multiplier, dampen) {
        Choice::Fully => format!("{name} counts fully."),
        Choice::Half => format!("{name} counts half."),
        Choice::Not => format!("{name} doesn't count."),
        Choice::Custom(factor) => format!("{name} counts {}.", multiplier(factor)),
    }
}

fn empty_text(no_games: bool, filter: Filter, search: &str) -> String {
    let search = search.trim();
    if no_games {
        "No games in your signals yet.".to_string()
    } else if !search.is_empty() {
        format!("No game matches \"{search}\".")
    } else {
        match filter {
            Filter::All | Filter::Owned => "No games in your library.".to_string(),
            Filter::Wishlist => "No games in your wishlist.".to_string(),
            Filter::Followed => "No followed games.".to_string(),
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
            recent: false,
            wishlist: false,
            followed: false,
            hours: 0.0,
            source: HoursSource::Played,
            counts: false,
            share: 0.0,
            tags: Vec::new(),
            correction: GameCorrection::default(),
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
            rows(3, 7, 2, true),
            [
                Row::Games(0..2),
                Row::Games(2..3),
                Row::Divider,
                Row::Games(3..5),
                Row::Games(5..7),
            ]
        );
        assert_eq!(rows(3, 7, 4, false), [Row::Games(0..4), Row::Games(4..7)]);
        assert_eq!(rows(5, 5, 4, true), [Row::Games(0..4), Row::Games(4..5)]);
        assert_eq!(rows(0, 2, 4, true), [Row::Divider, Row::Games(0..2)]);
    }

    #[test]
    fn only_the_rows_in_view_are_drawn() {
        let rows = rows(20, 40, 4, true);
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

    fn view_of(games: Vec<LibraryGameView>) -> LibraryView {
        LibraryView {
            games,
            min_hours: 2.0,
            wishlist_hours: 5.0,
            followed_hours: 3.0,
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
        assert_eq!(choice_label(Choice::Custom(0.25), 0.5), "×0.25");
        assert_eq!(choice_label(Choice::Half, 0.5), "Half");
        assert_eq!(choice_label(Choice::Half, 0.6), "Half ×0.6");
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
    }

    #[test]
    fn the_weight_line_says_how_much_or_why_not() {
        let games = games();
        let view = view_of(Vec::new());
        let line = |game: &LibraryGameView| weight_line(game, &view);

        assert_eq!(line(&games[1]), "Shapes 50% of your taste");
        assert_eq!(
            line(&LibraryGameView {
                share: 0.004,
                ..games[1].clone()
            }),
            "Shapes less than 1% of your taste"
        );
        assert_eq!(
            line(&games[2]),
            "Doesn't shape your taste: 1.0 h, under the 2 hours that count."
        );
        let zero = LibraryGameView {
            counts: false,
            correction: GameCorrection {
                multiplier: Some(0.0),
                hidden: false,
            },
            ..games[1].clone()
        };
        assert_eq!(line(&zero), "Doesn't shape your taste: set to ×0.");
        assert_eq!(
            no_effect_text(0.5, 2.0),
            "Correcting it would have no effect: it has 0.5 h, under 2 hours."
        );
    }

    #[test]
    fn the_toast_says_what_changed() {
        let none = GameCorrection::default();
        let with = |multiplier, hidden| GameCorrection { multiplier, hidden };

        assert_eq!(
            corrected_text("Black Ops II", none, with(Some(0.5), false), 0.5),
            "Black Ops II counts half."
        );
        assert_eq!(
            corrected_text("Hades", with(Some(0.5), false), none, 0.5),
            "Hades counts fully."
        );
        assert_eq!(
            corrected_text("Hades", none, with(Some(0.0), false), 0.5),
            "Hades doesn't count."
        );
        assert_eq!(
            corrected_text(
                "Hades",
                with(Some(0.0), false),
                with(Some(0.25), false),
                0.5
            ),
            "Hades counts ×0.25."
        );
        assert_eq!(
            corrected_text("Pentiment", none, with(None, true), 0.5),
            "Pentiment is hidden from Discover."
        );
        assert_eq!(
            corrected_text("Pentiment", with(None, true), none, 0.5),
            "Pentiment shows in Discover again."
        );
    }

    #[test]
    fn the_open_game_is_scrolled_into_view() {
        let rows = rows(20, 40, 4, true);
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
        let rows = rows(5, 9, 4, true);
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
            "Under 2 hours of play, set to ×0, or without tags."
        );
        assert_eq!(
            divider_note(1.0),
            "Under 1 hour of play, set to ×0, or without tags."
        );
        assert_eq!(
            divider_note(0.5),
            "Under 0.5 hours of play, set to ×0, or without tags."
        );
    }

    #[test]
    fn an_empty_grid_says_why() {
        assert_eq!(
            empty_text(true, Filter::All, ""),
            "No games in your signals yet."
        );
        assert_eq!(
            empty_text(false, Filter::All, " xyz "),
            "No game matches \"xyz\"."
        );
        assert_eq!(
            empty_text(false, Filter::Wishlist, ""),
            "No games in your wishlist."
        );
    }
}
