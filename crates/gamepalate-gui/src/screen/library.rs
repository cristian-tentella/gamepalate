use std::{fmt, ops::Range, sync::Arc};

use gamepalate_app::{
    AppId, Capsule, GameCorrection, Gamepalate, HoursSource, LibraryGameView, LibraryView,
};
use iced::{
    Center, ContentFit, Element, Fill, Size, Subscription, Task,
    widget::{
        Id, button, column, container, image, operation, pick_list, responsive, row, scrollable,
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

pub struct Library {
    gamepalate: Arc<Gamepalate>,
    view: LibraryView,
    filter: Filter,
    sort: Sort,
    search: String,
    shown: Vec<usize>,
    offset: f32,
    area: Size,
    capsules: Images<image::Allocation, AppId>,
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
            view: gamepalate.library_view(),
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
            }
            Message::CapsuleLoaded(id, None) => self.capsules.loaded(id, None),
            Message::CapsuleLoaded(id, Some(handle)) => {
                return image::allocate(handle)
                    .map(move |allocation| Message::CapsuleAllocated(id, allocation.ok()));
            }
            Message::CapsuleAllocated(id, allocation) => self.capsules.loaded(id, allocation),
        }

        Task::none()
    }

    pub fn subscription() -> Subscription<Message> {
        window::resize_events().map(|(_, size)| Message::WindowResized(size))
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
        let wanted = ids(visible_rows(
            &rows,
            self.offset,
            height * (1.0 + SCREENS_AHEAD),
        ));
        let kept = ids(visible_rows(
            &rows,
            self.offset - height * SCREENS_KEPT,
            height * (1.0 + 2.0 * SCREENS_KEPT),
        ));

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
            text_input("Search by name", &self.search)
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

        column![
            container(tools).padding([12, GRID_PADDING as u16]),
            style::separator(),
            body,
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

        tooltip(
            stack![picture, overlay]
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT),
            container(text(&game.name).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    }
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

fn hours(hours: f64) -> String {
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
        if min_hours == 1.0 {
            "1 hour".to_string()
        } else {
            format!("{min_hours} hours")
        }
    )
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
