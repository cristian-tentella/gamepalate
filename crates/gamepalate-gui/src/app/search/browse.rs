use std::{ops::Range, sync::Arc};

use gamepalate_app::{
    AppId, BrowseOptions, BrowseOrder, BrowseRow, Choice, Currency, DiscoverMode, FilterOptions,
    GameStanding, Gamepalate, Price, Reviews, SearchOptions,
};
use iced::{
    Center, ContentFit, Element, Fill, Length, Right, Size, Task, never, padding,
    widget::{
        Id, button, column, container, image, mouse_area, opaque, operation, rich_text, row,
        scrollable,
        scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport},
        sensor, space, span, stack, text,
        text::Wrapping,
        tooltip, vertical_slider,
    },
};
use jiff::{civil::Date, tz::TimeZone};

use super::{
    CAPSULE_HEIGHT, CAPSULE_RADIUS, CAPSULE_WIDTH, FIELD, Message as SearchMessage, NAME_SIZE,
    standing_text,
};
use crate::{
    font, format,
    screen::{
        discover::filters::{self, Action, Panel},
        library::handle,
        sentence,
        showcase::images::{Images, Picture},
    },
    style::{self, Chip},
};

const LIST: Id = Id::new("browse-list");
/// Every row is as tall, with or without a standing: the first row in sight is a division.
const ROW_HEIGHT: f32 = 56.0;
/// The height of the list until it tells its own: a few rows too many beat an empty screen.
const FIRST_HEIGHT: f32 = 1_500.0;
/// Capsules are small, and scrolling back should not load them again.
const CAPSULES_KEPT: usize = 200;
const SCROLLBAR_GAP: f32 = 12.0;
const ROW_PADDING: f32 = 10.0;
const COLUMN_SPACING: f32 = 12.0;
/// How far a column heading reaches past its column on each side.
const HEADING_PAD: f32 = 4.0;
const DISCOUNT_WIDTH: f32 = 64.0;
const PRICE_WIDTH: f32 = 120.0;
const RATING_WIDTH: f32 = 120.0;
const ENDS_WIDTH: f32 = 110.0;
const NO_GAMES: &str = "No games match";
const KINDS: [Kind; 6] = [
    Kind::Library,
    Kind::Wishlist,
    Kind::WouldTry,
    Kind::Saved,
    Kind::NotForMe,
    Kind::Later,
];

/// The whole catalog in a table, in the search panel. It keeps its state while the app is open,
/// also with the panel closed or back to Search.
pub(super) struct Browse {
    options: BrowseOptions,
    /// The options come from `gui.toml` the first time Browse is entered.
    loaded: bool,
    rows: Vec<BrowseRow>,
    /// The name the rows were listed for: none before the first list.
    listed: Option<String>,
    /// The revision of the facade the rows were listed on, or last agreed with.
    revision: Option<u64>,
    /// The facade is another one since: the list is made again the next time Browse is entered.
    stale: bool,
    currency: Option<Currency>,
    /// Why the last list could not be made: the options and the rows are those of before.
    error: Option<String>,
    panel: Option<Panel>,
    panel_open: bool,
    tools_height: f32,
    selected: Option<usize>,
    offset: f32,
    height: Option<f32>,
    upward: bool,
    capsules: Images<image::Allocation, AppId>,
}

/// The kinds of game the three-state filters are about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Library,
    Wishlist,
    WouldTry,
    Saved,
    NotForMe,
    Later,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Library => "Library",
            Kind::Wishlist => "Wishlist",
            Kind::WouldTry => "Would try",
            Kind::Saved => "Saved",
            Kind::NotForMe => "Not for me",
            Kind::Later => "Later",
        }
    }

    fn tip(self) -> Option<&'static str> {
        match self {
            Kind::Library => Some("Owned, borrowed and played elsewhere"),
            Kind::Wishlist => Some("Wishlist and followed"),
            _ => None,
        }
    }

    /// The tooltip of its chip: what the next click does, and what the kind means if it needs
    /// saying. The second line is empty when it does not.
    fn advice(self, choice: Choice) -> [String; 2] {
        let label = self.label();
        let click = match choice {
            Choice::Any => format!("Show only {label}"),
            Choice::Only => format!("Hide {label}"),
            Choice::Without => format!("Show {label} again"),
        };

        [click, self.tip().map(str::to_string).unwrap_or_default()]
    }

    fn choice(self, options: &BrowseOptions) -> Choice {
        match self {
            Kind::Library => options.library,
            Kind::Wishlist => options.wished,
            Kind::WouldTry => options.would_try,
            Kind::Saved => options.saved,
            Kind::NotForMe => options.not_for_me,
            Kind::Later => options.later,
        }
    }

    fn set(self, options: &mut BrowseOptions, choice: Choice) {
        let place = match self {
            Kind::Library => &mut options.library,
            Kind::Wishlist => &mut options.wished,
            Kind::WouldTry => &mut options.would_try,
            Kind::Saved => &mut options.saved,
            Kind::NotForMe => &mut options.not_for_me,
            Kind::Later => &mut options.later,
        };
        *place = choice;
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    Sort(BrowseOrder),
    OnSale(bool),
    Show(Kind, Choice),
    ToggleFilters,
    CloseFilters,
    Filters(filters::Message),
    Measured(Size),
    Scrolled(Viewport),
    /// The scrubber moved to a row position: 0 is the top of the list.
    Scrub(u32),
    CapsuleLoaded(AppId, Option<image::Handle>),
    CapsuleAllocated(AppId, Option<image::Allocation>),
}

impl Default for Browse {
    fn default() -> Browse {
        Browse {
            options: BrowseOptions::default(),
            loaded: false,
            rows: Vec::new(),
            listed: None,
            revision: None,
            stale: false,
            currency: None,
            error: None,
            panel: None,
            panel_open: false,
            tools_height: 0.0,
            selected: None,
            offset: 0.0,
            height: None,
            upward: false,
            capsules: Images::with_capacity(CAPSULES_KEPT),
        }
    }
}

impl Browse {
    pub(super) fn update(
        &mut self,
        message: Message,
        gamepalate: &Arc<Gamepalate>,
        name: &str,
    ) -> Task<SearchMessage> {
        match message {
            Message::Sort(_) | Message::OnSale(_) | Message::Show(..) => {
                let next = changed(&self.options, &message);
                Task::batch([self.change(gamepalate, name, next), operation::focus(FIELD)])
            }
            Message::ToggleFilters if self.panel_open => {
                self.panel_open = false;
                Task::none()
            }
            Message::ToggleFilters => {
                self.open_filters();
                Task::none()
            }
            Message::CloseFilters => {
                self.panel_open = false;
                Task::none()
            }
            Message::Filters(message) => self.change_filters(gamepalate, name, message),
            Message::Measured(size) => {
                self.tools_height = size.height;
                Task::none()
            }
            Message::Scrolled(viewport) => {
                let offset = viewport.absolute_offset().y;
                if offset != self.offset {
                    self.upward = offset < self.offset;
                }
                self.offset = offset;
                self.height = Some(viewport.bounds().height);
                self.load_capsules(gamepalate)
            }
            Message::Scrub(position) => {
                let offset = scrub_offset(position, self.rows.len(), self.height());
                self.upward = offset < self.offset;
                self.offset = offset;
                Task::batch([
                    scroll_to(offset),
                    self.load_capsules(gamepalate),
                    operation::focus(FIELD),
                ])
            }
            Message::CapsuleLoaded(id, None) => {
                self.capsules.loaded(id, None);
                self.load_capsules(gamepalate)
            }
            Message::CapsuleLoaded(id, Some(handle)) => {
                image::allocate(handle).map(move |allocation| {
                    SearchMessage::Browse(Message::CapsuleAllocated(id, allocation.ok()))
                })
            }
            Message::CapsuleAllocated(id, allocation) => {
                self.capsules.loaded(id, allocation);
                self.load_capsules(gamepalate)
            }
        }
    }

    /// Into Browse with `name` in the field: the list is made again only if the name has changed
    /// since, otherwise it comes back where it was. If the votes, the corrections or the families
    /// have changed since, or the facade is another one, the list is made again too, with the
    /// same game selected if it is still there. The first time, the options are those of
    /// `gui.toml`. The panel of the filters is made again each time, for the tags of the catalog.
    pub(super) fn enter(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        name: &str,
    ) -> Task<SearchMessage> {
        if !self.loaded {
            self.loaded = true;
            self.options = gamepalate.gui_settings().browse;
        }
        self.panel = Some(
            Panel::new(
                gamepalate.tag_names(),
                &FilterOptions::default(),
                format::currency_symbol(gamepalate.currency()),
                &search_options(&self.options.filters),
            )
            .without_bets(),
        );
        self.panel_open = false;

        if self.listed.as_deref() != Some(name) {
            return self.relist(gamepalate, name);
        }
        if self.outdated(gamepalate) {
            return self.apply(gamepalate, self.options.clone(), name, true);
        }
        self.come_back(gamepalate)
    }

    /// Back from a game's page: the table where it was, with the same row selected. The list is
    /// not made again.
    pub(super) fn come_back(&mut self, gamepalate: &Arc<Gamepalate>) -> Task<SearchMessage> {
        self.panel_open = false;
        Task::batch([scroll_to(self.offset), self.load_capsules(gamepalate)])
    }

    /// The game opened from the table: its row stays the selected one.
    pub(super) fn picked(&mut self, id: AppId) {
        if let Some(index) = self.rows.iter().position(|row| row.id == id) {
            self.selected = Some(index);
        }
    }

    /// The votes, the corrections or the families are not what the rows were made from.
    fn outdated(&self, gamepalate: &Gamepalate) -> bool {
        self.outdated_by(gamepalate.revision())
    }

    fn outdated_by(&self, revision: u64) -> bool {
        self.stale || self.revision != Some(revision)
    }

    pub(super) fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// A game opened from the table was changed in its page: only its row takes the new standing,
    /// and keeps its place even if the filters would now leave it out. The next list will not.
    pub(super) fn game_changed(&mut self, gamepalate: &Gamepalate, id: AppId) {
        self.revision = Some(gamepalate.revision());
        if let Some(view) = gamepalate.game_view(id) {
            let saved = view.vote.as_ref().is_some_and(|vote| vote.saved);
            self.update_row(id, view.standing, saved);
        }
    }

    fn update_row(&mut self, id: AppId, standing: Option<GameStanding>, saved: bool) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.standing = standing;
            row.saved = saved;
        }
    }

    /// The list made again from the top, with the options of now.
    pub(super) fn relist(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        name: &str,
    ) -> Task<SearchMessage> {
        self.apply(gamepalate, self.options.clone(), name, false)
    }

    /// The list of `options`, and saved if they are new. With an error nothing changes.
    fn change(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        name: &str,
        options: BrowseOptions,
    ) -> Task<SearchMessage> {
        if options == self.options {
            return Task::none();
        }
        let listed = self.apply(gamepalate, options, name, false);
        if self.error.is_some() {
            return listed;
        }
        let gamepalate = Arc::clone(gamepalate);
        let options = self.options.clone();
        let saved = Task::future(async move {
            gamepalate.change_gui_settings(|settings| settings.browse = options)
        })
        .discard();

        Task::batch([listed, saved])
    }

    /// The list of `options`: from the top, or with `keeping` around the game that was selected,
    /// if it is still there, at the same height in the window.
    fn apply(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        options: BrowseOptions,
        name: &str,
        keeping: bool,
    ) -> Task<SearchMessage> {
        // Read before the list: a change in between only makes the next entry list again
        let revision = gamepalate.revision();
        let before = if keeping { self.place() } else { None };
        let rows = gamepalate
            .browse(&options, name)
            .map_err(|error| sentence(&error.to_string()));
        self.currency = Some(gamepalate.currency().clone());
        if !self.accept(options, name, rows) {
            return Task::none();
        }
        self.revision = Some(revision);
        if let Some(place) = before {
            self.keep(place);
        }

        Task::batch([scroll_to(self.offset), self.load_capsules(gamepalate)])
    }

    /// The selected game, and how far below the top of the window its row is.
    fn place(&self) -> Option<(AppId, f64)> {
        let index = self.selected?;
        let row = self.rows.get(index)?;

        Some((
            row.id,
            f64::from(ROW_HEIGHT) * index as f64 - f64::from(self.offset),
        ))
    }

    /// In a list just made, selects the game of `place` if it is there, and puts it as far below
    /// the top as before, as far as the list allows.
    fn keep(&mut self, (id, below): (AppId, f64)) {
        let Some(index) = self.rows.iter().position(|row| row.id == id) else {
            return;
        };
        let top = f64::from(ROW_HEIGHT) * index as f64;
        let end =
            (f64::from(ROW_HEIGHT) * self.rows.len() as f64 - f64::from(self.height())).max(0.0);

        self.selected = Some(index);
        self.offset = (top - below).clamp(0.0, end) as f32;
    }

    /// Takes a list made from the top. An error keeps the options, the rows and the position of
    /// before, and shows in the line of the filters.
    fn accept(
        &mut self,
        options: BrowseOptions,
        name: &str,
        rows: Result<Vec<BrowseRow>, String>,
    ) -> bool {
        match rows {
            Ok(rows) => {
                self.options = options;
                self.rows = rows;
                self.error = None;
                self.stale = false;
                self.listed = Some(name.to_string());
                self.selected = None;
                self.offset = 0.0;
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    /// Browse with its panel of filters open, for the tests of the panel that holds it.
    #[cfg(test)]
    pub(super) fn with_filters_open() -> Browse {
        let mut browse = Browse {
            panel: Some(
                Panel::new(
                    vec!["Roguelike".to_string()],
                    &FilterOptions::default(),
                    "€".to_string(),
                    &SearchOptions::default(),
                )
                .without_bets(),
            ),
            ..Browse::default()
        };
        browse.open_filters();
        browse
    }

    /// Browse with a list of `count` rows, for the tests of the panel that holds it.
    #[cfg(test)]
    pub(super) fn with_rows(count: u32) -> Browse {
        Browse {
            rows: tests::rows(count),
            listed: Some(String::new()),
            height: Some(tests::HEIGHT),
            ..Browse::default()
        }
    }

    /// Where the list stands: the address of its rows, how many, the selected row and the offset.
    #[cfg(test)]
    pub(super) fn state(&self) -> (usize, usize, Option<usize>, f32) {
        (
            self.rows.as_ptr() as usize,
            self.rows.len(),
            self.selected,
            self.offset,
        )
    }

    fn open_filters(&mut self) {
        if let Some(panel) = &mut self.panel {
            panel.show(&search_options(&self.options.filters));
            self.panel_open = true;
        }
    }

    pub(super) fn filters_open(&self) -> bool {
        self.panel_open
    }

    pub(super) fn close_filters(&mut self) {
        self.panel_open = false;
    }

    fn change_filters(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        name: &str,
        message: filters::Message,
    ) -> Task<SearchMessage> {
        let Some(panel) = &mut self.panel else {
            return Task::none();
        };
        let current = search_options(&self.options.filters);
        match panel.update(&current, message) {
            Action::Stay => Task::none(),
            Action::Close => {
                self.panel_open = false;
                Task::none()
            }
            Action::Search(chosen) => {
                self.panel_open = false;
                let options = BrowseOptions {
                    filters: chosen.filters,
                    ..self.options.clone()
                };
                Task::batch([
                    self.change(gamepalate, name, options),
                    operation::focus(FIELD),
                ])
            }
        }
    }

    pub(super) fn down(&mut self) -> Task<SearchMessage> {
        self.select(super::below(self.selected, self.rows.len()))
    }

    pub(super) fn up(&mut self) -> Task<SearchMessage> {
        self.select(super::above(self.selected))
    }

    /// A screen down: the rows that fit in the window, as far as the last.
    pub(super) fn page_down(&mut self) -> Task<SearchMessage> {
        let page = page_rows(self.height());
        let last = self.rows.len().checked_sub(1);

        self.select(last.map(|last| (self.selected.unwrap_or(0) + page).min(last)))
    }

    pub(super) fn page_up(&mut self) -> Task<SearchMessage> {
        let page = page_rows(self.height());

        self.select(Some(self.selected.unwrap_or(0).saturating_sub(page)))
    }

    pub(super) fn first(&mut self) -> Task<SearchMessage> {
        self.select(Some(0))
    }

    pub(super) fn last(&mut self) -> Task<SearchMessage> {
        self.select(self.rows.len().checked_sub(1))
    }

    /// The row Enter opens: the selected one, or the first.
    pub(super) fn chosen(&self) -> Option<AppId> {
        let index = self.selected.unwrap_or(0);
        self.rows.get(index).map(|row| row.id)
    }

    /// The list follows the selection only when the row leaves the sight, so holding ↓ scrolls
    /// one row at a time. The offset is the list's at once: the next keys count from there.
    fn select(&mut self, selected: Option<usize>) -> Task<SearchMessage> {
        if self.rows.is_empty() {
            return Task::none();
        }
        self.selected = selected;
        let Some(index) = selected else {
            return Task::none();
        };

        match reveal(index, self.offset, self.height()) {
            Some(offset) => {
                self.upward = offset < self.offset;
                self.offset = offset;
                scroll_to(offset)
            }
            None => Task::none(),
        }
    }

    fn height(&self) -> f32 {
        self.height.unwrap_or(FIRST_HEIGHT)
    }

    fn load_capsules(&mut self, gamepalate: &Arc<Gamepalate>) -> Task<SearchMessage> {
        Task::batch(self.capsules_to_load().into_iter().map(|id| {
            let gamepalate = Arc::clone(gamepalate);
            Task::perform(
                async move { gamepalate.capsule(id).await.ok().map(handle) },
                move |handle| SearchMessage::Browse(Message::CapsuleLoaded(id, handle)),
            )
        }))
    }

    /// The rows in sight first, then a screen ahead in the direction of the scroll. Each call
    /// replaces the queue, so the rows already passed are not loaded.
    fn capsules_to_load(&mut self) -> Vec<AppId> {
        let count = self.rows.len();
        let height = self.height();
        let near: Vec<AppId> = self.rows[shown(self.offset, height, count)]
            .iter()
            .map(|row| row.id)
            .collect();
        self.capsules.keep_only(&near);
        let wanted = wanted(self.offset, height, count, self.upward)
            .into_iter()
            .map(|index| self.rows[index].id)
            .collect();

        self.capsules.want(wanted)
    }

    /// The panel below the field: the toolbar, the filters and the table, with the popup of the
    /// filters over it.
    pub(super) fn view<'a>(
        &'a self,
        field: Element<'a, SearchMessage>,
        size: Size,
    ) -> Element<'a, SearchMessage> {
        let table: Element<'a, SearchMessage> = if self.rows.is_empty() {
            if self.error.is_none() {
                note(NO_GAMES.to_string())
            } else {
                space().into()
            }
        } else {
            let zone = TimeZone::system();
            let today = jiff::Zoned::now().date();
            column![
                self.head(),
                style::separator(),
                self.list(|index| self.row(index, &zone, today)),
            ]
            .width(Fill)
            .height(Fill)
            .into()
        };
        let top = sensor(column![field, style::separator(), self.toolbar()])
            .on_show(|size| SearchMessage::Browse(Message::Measured(size)))
            .on_resize(|size| SearchMessage::Browse(Message::Measured(size)));
        let content = container(
            column![top]
                .push(self.problem())
                .push(style::separator())
                .push(table)
                .height(Fill),
        )
        .width(size.width)
        .height(size.height);

        stack![content].push(self.popup(size.height)).into()
    }

    /// The count, the chips and the button of the other filters, on one line while they fit.
    fn toolbar(&self) -> Element<'_, SearchMessage> {
        let arrow = if self.panel_open { '▴' } else { '▾' };
        let others = others_on(&self.options.filters);
        let label = if others == 0 {
            format!("Filters {arrow}")
        } else {
            format!("Filters · {others} {arrow}")
        };

        row![
            text(games_count(self.rows.len()))
                .size(font::NOTE_SIZE)
                .style(style::muted),
            self.chips(),
            button(text(label).size(font::NOTE_SIZE))
                .padding([4, 8])
                .style(style::text_button)
                .on_press(SearchMessage::Browse(Message::ToggleFilters)),
        ]
        .spacing(12)
        .align_y(Center)
        .padding(padding::top(6).bottom(6).left(16).right(16))
        .into()
    }

    /// Why the last filters were not applied.
    fn problem(&self) -> Option<Element<'_, SearchMessage>> {
        let error = self.error.as_ref()?;

        Some(
            container(
                text(error)
                    .size(font::NOTE_SIZE)
                    .style(text::danger)
                    .wrapping(Wrapping::None),
            )
            .width(Fill)
            .padding(padding::left(16).right(16).bottom(6))
            .clip(true)
            .into(),
        )
    }

    /// The filters of the panel, in a popup under the toolbar and over the table.
    fn popup(&self, height: f32) -> Option<Element<'_, SearchMessage>> {
        let panel = self.panel.as_ref().filter(|_| self.panel_open)?;
        let panel = container(
            scrollable(
                panel
                    .view(DiscoverMode::New)
                    .map(|message| SearchMessage::Browse(Message::Filters(message))),
            )
            .spacing(8),
        )
        .padding(20)
        .max_height((height - self.tools_height - 16.0).max(0.0))
        .style(style::popup);

        Some(
            stack![
                opaque(
                    mouse_area(space().width(Fill).height(Fill))
                        .on_press(SearchMessage::Browse(Message::CloseFilters))
                ),
                container(opaque(panel))
                    .width(Fill)
                    .align_x(Right)
                    .padding(padding::top(self.tools_height + 4.0).right(16)),
            ]
            .into(),
        )
    }

    /// The filters on the kind of game, one chip each, in a row that wraps if the line is
    /// narrow. A chip goes from not filtering to showing only those games, to hiding them.
    fn chips(&self) -> Element<'_, SearchMessage> {
        let on_sale = if self.options.filters.only_discounted {
            Chip::Only
        } else {
            Chip::Off
        };
        let sale = self.chip(
            "On sale",
            on_sale,
            [
                if on_sale == Chip::Only {
                    "Show games at any price".to_string()
                } else {
                    "Show only games on sale".to_string()
                },
                String::new(),
            ],
            Message::OnSale(on_sale == Chip::Off),
        );

        container(
            row![sale]
                .extend(KINDS.map(|kind| {
                    let choice = kind.choice(&self.options);
                    self.chip(
                        kind.label(),
                        chip_state(choice),
                        kind.advice(choice),
                        Message::Show(kind, next_choice(choice)),
                    )
                }))
                .spacing(8)
                .wrap()
                .vertical_spacing(6),
        )
        .width(Fill)
        .center_x(Fill)
        .into()
    }

    /// A chip: its icon says what it does to the list, its tooltip what the next click does.
    /// `tip` has one or two lines; a line left empty is not shown.
    fn chip<'a>(
        &self,
        label: &'static str,
        state: Chip,
        tip: [String; 2],
        message: Message,
    ) -> Element<'a, SearchMessage> {
        // Every state has its icon, all as wide as each other: the chip keeps its size.
        let glyph = match state {
            Chip::Off => font::DASH,
            Chip::Only => font::CHECK,
            Chip::Hide => font::BLOCK,
        };
        let icon = text(glyph).font(font::ICONS).size(font::icon(11.0));
        let chip = button(
            row![icon, text(label).size(font::NOTE_SIZE)]
                .spacing(3)
                .align_y(Center),
        )
        .padding([2, 8])
        .style(style::filter_chip(state))
        .on_press(SearchMessage::Browse(message));
        let lines: Vec<String> = tip.into_iter().filter(|line| !line.is_empty()).collect();
        let several = lines.len() > 1;

        tooltip(
            chip,
            container(
                column(lines.into_iter().map(|line| {
                    let line = if several { format!("{line}.") } else { line };
                    text(line).size(font::NOTE_SIZE).into()
                }))
                .spacing(2),
            )
            .padding([6, 10])
            .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    }

    fn head(&self) -> Element<'_, SearchMessage> {
        // A heading is a few pixels wider than its column on each side, so its hover has some
        // room around the text, and the text stays over the figures of the column.
        let heading = |order: BrowseOrder, width: Length, right: bool| {
            let mut label = column_name(order).to_string();
            if self.options.order == order {
                label.push_str(if self.options.descending {
                    " ▼"
                } else {
                    " ▲"
                });
            }
            let label = text(label).size(font::NOTE_SIZE).style(style::muted);
            let label = if right {
                container(label).width(Fill).align_x(Right)
            } else {
                container(label).width(Fill)
            };
            let width = match width {
                Length::Fixed(width) => Length::Fixed(width + 2.0 * HEADING_PAD),
                other => other,
            };
            button(label)
                .padding([4.0, HEADING_PAD])
                .width(width)
                .style(style::column_heading)
                .on_press(SearchMessage::Browse(Message::Sort(order)))
        };

        row![
            space().width(CAPSULE_WIDTH + HEADING_PAD),
            heading(BrowseOrder::Name, Fill, false),
            heading(BrowseOrder::Discount, DISCOUNT_WIDTH.into(), true),
            heading(BrowseOrder::Price, PRICE_WIDTH.into(), true),
            heading(BrowseOrder::Rating, RATING_WIDTH.into(), true),
            heading(BrowseOrder::Ends, ENDS_WIDTH.into(), true),
        ]
        .spacing(COLUMN_SPACING - 2.0 * HEADING_PAD)
        .align_y(Center)
        .padding(
            padding::left(ROW_PADDING)
                .right(ROW_PADDING + SCROLLBAR_GAP - HEADING_PAD)
                .bottom(4),
        )
        .into()
    }

    /// The table, with only the rows of `shown` built: a space stands for the rows above and
    /// one for those below.
    fn list<'a>(
        &self,
        row_of: impl Fn(usize) -> Element<'a, SearchMessage>,
    ) -> Element<'a, SearchMessage> {
        let count = self.rows.len();
        let shown = shown(self.offset, self.height(), count);
        let above = rows_height(shown.start);
        let below = rows_height(count - shown.end);

        row![
            scrollable(
                column![space().height(above)]
                    .extend(shown.map(row_of))
                    .push(space().height(below))
            )
            .id(LIST)
            .direction(Direction::Vertical(Scrollbar::hidden()))
            .on_scroll(|viewport| SearchMessage::Browse(Message::Scrolled(viewport)))
            .width(Fill)
            .height(Fill),
            self.scrubber(),
        ]
        .into()
    }

    /// The place in the list, as a handle that is always easy to grab: the bar of a scrollable
    /// shrinks to 2 px with 100,000 rows. Its tooltip says which rows are in sight.
    fn scrubber<'a>(&self) -> Element<'a, SearchMessage> {
        let (count, height) = (self.rows.len(), self.height());
        let last = scrub_last(count, height);
        if last == 0 {
            return space().width(SCROLLBAR_GAP).into();
        }
        let position = scrub_position(self.offset, count, height);
        let seen = visible(self.offset, height, count);
        let number = |count: usize| format::count(u32::try_from(count).unwrap_or(u32::MAX));
        let tip = format!(
            "{}–{} of {}",
            number(seen.start + 1),
            number(seen.end),
            number(count)
        );

        tooltip(
            vertical_slider(0..=last, last - position, move |value| {
                SearchMessage::Browse(Message::Scrub(last - value))
            })
            .width(SCROLLBAR_GAP)
            .height(Fill)
            .style(style::scrubber),
            container(text(tip).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Left,
        )
        .into()
    }

    fn row<'a>(&'a self, index: usize, zone: &TimeZone, today: Date) -> Element<'a, SearchMessage> {
        let game = &self.rows[index];
        let small = |line: String| {
            text(line)
                .size(font::NOTE_SIZE)
                .wrapping(Wrapping::None)
                .style(style::muted)
        };
        let picture: Element<'a, SearchMessage> = match self.capsules.get(&game.id) {
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
            row![]
                .push(game.saved.then(|| {
                    text(font::BOOKMARK)
                        .font(font::ICONS)
                        .size(font::icon(font::NOTE_SIZE))
                        .style(style::muted)
                }))
                .push(small(standing_text(standing)))
                .spacing(4)
                .align_y(Center)
        });
        let name = container(
            column![text(&*game.name).size(NAME_SIZE).wrapping(Wrapping::None)]
                .push(standing)
                .spacing(2),
        )
        .width(Fill)
        .clip(true);
        let discount = game.price.discount.map(|discount| {
            text(format::discount(discount.percent))
                .size(font::NOTE_SIZE)
                .style(style::accent)
        });
        let (price, original) = match &self.currency {
            Some(currency) => price_lines(&game.price, currency),
            None => (String::new(), None),
        };
        let original = original.map(|original| {
            rich_text(vec![span(original).strikethrough(true)])
                .size(font::NOTE_SIZE)
                .style(style::muted)
                .on_link_click(never)
        });
        let (rating, reviews) = rating_lines(&game.reviews);
        let ends = game
            .price
            .discount
            .and_then(|discount| discount.end)
            .and_then(|end| format::short_ends(end, zone, today));

        let cell = |content: Element<'a, SearchMessage>, width: f32| {
            container(content).width(width).align_x(Right)
        };
        button(
            row![
                picture,
                name,
                cell(column![].push(discount).into(), DISCOUNT_WIDTH),
                cell(
                    column![text(price).size(NAME_SIZE)]
                        .push(original)
                        .align_x(Right)
                        .into(),
                    PRICE_WIDTH,
                ),
                cell(
                    column![text(rating).size(NAME_SIZE)]
                        .push(reviews.map(small))
                        .align_x(Right)
                        .into(),
                    RATING_WIDTH,
                ),
                cell(
                    column![]
                        .push(ends.map(|ends| text(ends).size(font::NOTE_SIZE)))
                        .into(),
                    ENDS_WIDTH,
                ),
            ]
            .spacing(COLUMN_SPACING)
            .height(Fill)
            .align_y(Center),
        )
        .padding([0.0, ROW_PADDING])
        .width(Fill)
        .height(ROW_HEIGHT)
        .style(style::list_row(self.selected == Some(index)))
        .on_press(SearchMessage::Pick(game.id))
        .into()
    }
}

/// The options the panel of the filters works on: the filters, and nothing else.
fn search_options(filters: &FilterOptions) -> SearchOptions {
    SearchOptions {
        filters: filters.clone(),
        ..SearchOptions::default()
    }
}

/// The options a message asks for; the same options for a message that does not change them.
fn changed(options: &BrowseOptions, message: &Message) -> BrowseOptions {
    let mut next = options.clone();
    match message {
        Message::Sort(order) => sort_by(&mut next, *order),
        Message::OnSale(on) => {
            next.filters.only_discounted = *on;
            if !on {
                next.filters.min_discount_percent = None;
            }
        }
        Message::Show(kind, choice) => kind.set(&mut next, *choice),
        _ => {}
    }
    next
}

/// The chip that stands for `choice`: showing only those games, hiding them, or neither.
fn chip_state(choice: Choice) -> Chip {
    match choice {
        Choice::Without => Chip::Hide,
        Choice::Any => Chip::Off,
        Choice::Only => Chip::Only,
    }
}

/// What a click on a chip does: not filtering, then only, then hide, then not filtering again.
fn next_choice(choice: Choice) -> Choice {
    match choice {
        Choice::Any => Choice::Only,
        Choice::Only => Choice::Without,
        Choice::Without => Choice::Any,
    }
}

/// How many of the filters in the popup are on: a range counts once, with one end or both.
fn others_on(filters: &FilterOptions) -> usize {
    [
        filters.min_reviews.is_some() || filters.max_reviews.is_some(),
        filters.min_positive_percent.is_some(),
        filters.without_voted,
        filters.without_early_access.is_some(),
        filters.min_price_cents.is_some() || filters.max_price_cents.is_some(),
        filters.min_discount_percent.is_some(),
        !filters.tags.is_empty(),
        !filters.without_tags.is_empty(),
    ]
    .into_iter()
    .filter(|on| *on)
    .count()
}

fn note<'a>(line: String) -> Element<'a, SearchMessage> {
    container(text(line).size(font::NOTE_SIZE).style(style::muted))
        .padding([12, 16])
        .into()
}

fn scroll_to(offset: f32) -> Task<SearchMessage> {
    operation::scroll_to(
        LIST,
        AbsoluteOffset {
            x: None,
            y: Some(offset),
        },
    )
}

/// A click on the column of the order turns it around; on another column it takes that column's
/// natural direction.
fn sort_by(options: &mut BrowseOptions, order: BrowseOrder) {
    if options.order == order {
        options.descending = !options.descending;
    } else {
        options.order = order;
        options.descending = order.descending_first();
    }
}

fn column_name(order: BrowseOrder) -> &'static str {
    match order {
        BrowseOrder::Name => "Name",
        BrowseOrder::Discount => "%",
        BrowseOrder::Price => "Price",
        BrowseOrder::Rating => "Rating",
        BrowseOrder::Ends => "Ends",
        BrowseOrder::Reviews => "Reviews",
    }
}

fn games_count(count: usize) -> String {
    match count {
        1 => "1 game".to_string(),
        count => format!(
            "{} games",
            format::count(u32::try_from(count).unwrap_or(u32::MAX))
        ),
    }
}

/// The price paid and, with a discount, the full price to strike through.
fn price_lines(price: &Price, currency: &Currency) -> (String, Option<String>) {
    (
        format::price(price, currency),
        price
            .discount
            .map(|_| format::original_price(price, currency)),
    )
}

/// The positive share and, below, how many reviews.
fn rating_lines(reviews: &Reviews) -> (String, Option<String>) {
    if reviews.total == 0 {
        return ("—".to_string(), None);
    }
    (
        format::reviews(reviews).0,
        Some(format::count(reviews.total)),
    )
}

/// The rows from the first in sight to the last, both cut at `count`. Kept in `f64`: a list of
/// 100,000 rows is millions of pixels tall, where an `f32` has half a pixel of precision.
fn visible(offset: f32, height: f32, count: usize) -> Range<usize> {
    let row = f64::from(ROW_HEIGHT);
    let top = f64::from(offset.max(0.0));
    let start = (top / row).floor() as usize;
    let end = ((top + f64::from(height.max(0.0))) / row).ceil() as usize;

    start.min(count)..end.min(count)
}

/// The rows a screen moves: those that fit in the window whole, and at least one.
fn page_rows(height: f32) -> usize {
    ((f64::from(height.max(0.0)) / f64::from(ROW_HEIGHT)).floor() as usize).max(1)
}

fn screen_rows(height: f32) -> usize {
    (f64::from(height.max(0.0)) / f64::from(ROW_HEIGHT)).ceil() as usize
}

/// The rows a view builds: those in sight, and a screen more above and below.
fn shown(offset: f32, height: f32, count: usize) -> Range<usize> {
    let visible = visible(offset, height, count);
    let screen = screen_rows(height);

    visible.start.saturating_sub(screen)..(visible.end + screen).min(count)
}

/// The rows whose capsules to ask for, in order: those in sight, then a screen ahead in the
/// direction of the scroll.
fn wanted(offset: f32, height: f32, count: usize, upward: bool) -> Vec<usize> {
    let visible = visible(offset, height, count);
    let screen = screen_rows(height);

    if upward {
        let ahead = visible.start.saturating_sub(screen)..visible.start;
        visible.chain(ahead.rev()).collect()
    } else {
        let ahead = visible.end..(visible.end + screen).min(count);
        visible.chain(ahead).collect()
    }
}

/// The last position of the scrubber: how many rows the top of the list can go down.
fn scrub_last(count: usize, height: f32) -> u32 {
    u32::try_from(count.saturating_sub(page_rows(height))).unwrap_or(u32::MAX)
}

/// How far the list is scrolled at `position`, from 0 at the top to `scrub_last` at the bottom.
fn scrub_offset(position: u32, count: usize, height: f32) -> f32 {
    let last = scrub_last(count, height);
    let most = (f64::from(rows_height(count)) - f64::from(height)).max(0.0);
    if last == 0 {
        return 0.0;
    }

    (f64::from(position.min(last)) / f64::from(last) * most) as f32
}

/// The position of the scrubber for an offset: the inverse of `scrub_offset`.
fn scrub_position(offset: f32, count: usize, height: f32) -> u32 {
    let last = scrub_last(count, height);
    let most = (f64::from(rows_height(count)) - f64::from(height)).max(0.0);
    if most == 0.0 {
        return 0;
    }

    ((f64::from(offset) / most).clamp(0.0, 1.0) * f64::from(last)).round() as u32
}

fn rows_height(rows: usize) -> f32 {
    (rows as f64 * f64::from(ROW_HEIGHT)) as f32
}

/// The offset that brings row `index` into sight, if it is not.
fn reveal(index: usize, offset: f32, height: f32) -> Option<f32> {
    let top = f64::from(ROW_HEIGHT) * index as f64;
    let bottom = top + f64::from(ROW_HEIGHT);
    let offset = f64::from(offset);

    if top < offset {
        Some(top as f32)
    } else if bottom > offset + f64::from(height) {
        Some((bottom - f64::from(height)) as f32)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gamepalate_app::{Discount, Timestamp};

    use super::*;
    use crate::screen::showcase::images::MAX_LOADS;

    const SALES: usize = 118_780;
    pub(super) const HEIGHT: f32 = 700.0;

    pub(super) fn rows(count: u32) -> Vec<BrowseRow> {
        (1..=count)
            .map(|id| BrowseRow {
                id: AppId(id),
                name: Arc::from(format!("Game {id}")),
                standing: None,
                saved: false,
                price: Price {
                    original_cents: 1999,
                    discount: None,
                },
                reviews: Reviews {
                    total: 10,
                    positive: 9,
                },
            })
            .collect()
    }

    fn browse_with(count: u32) -> Browse {
        Browse {
            rows: rows(count),
            listed: Some(String::new()),
            height: Some(HEIGHT),
            ..Browse::default()
        }
    }

    #[test]
    fn shows_a_screen_around_the_rows_in_sight() {
        let screen = screen_rows(HEIGHT);
        assert_eq!(screen, 13);

        assert_eq!(shown(0.0, HEIGHT, 1_000), 0..2 * screen);
        assert_eq!(
            shown(100.0 * ROW_HEIGHT, HEIGHT, 1_000),
            100 - screen..100 + 2 * screen
        );
        assert_eq!(
            shown(100.0 * ROW_HEIGHT + 40.0, HEIGHT, 1_000),
            100 - screen..101 + 2 * screen
        );
        assert_eq!(
            shown(1_000.0 * ROW_HEIGHT - HEIGHT, HEIGHT, 1_000),
            1_000 - 2 * screen..1_000
        );
    }

    #[test]
    fn a_short_list_shows_all_its_rows() {
        assert_eq!(shown(0.0, HEIGHT, 5), 0..5);
        assert_eq!(shown(0.0, HEIGHT, 0), 0..0);
        assert_eq!(shown(500.0, HEIGHT, 0), 0..0);
        assert_eq!(wanted(0.0, HEIGHT, 0, false), Vec::<usize>::new());
    }

    #[test]
    fn the_last_row_of_the_whole_catalog_is_in_sight() {
        let bottom = rows_height(SALES) - HEIGHT;

        let visible = visible(bottom, HEIGHT, SALES);
        let shown = shown(bottom, HEIGHT, SALES);

        assert_eq!(visible, SALES - 13..SALES);
        assert_eq!(shown, SALES - 26..SALES);
        assert_eq!(reveal(SALES - 1, bottom, HEIGHT), None);
        assert_eq!(reveal(SALES - 1, 0.0, HEIGHT), Some(bottom));
    }

    #[test]
    fn the_view_builds_only_the_rows_it_shows() {
        let mut browse = browse_with(120_000);
        browse.offset = 60_000.0 * ROW_HEIGHT;
        let built = Cell::new(0);

        let _ = browse.list(|_| {
            built.set(built.get() + 1);
            space().into()
        });

        let shown = shown(browse.offset, HEIGHT, 120_000);
        assert_eq!(built.get(), shown.len());
        assert!(built.get() <= 3 * screen_rows(HEIGHT) + 1);
    }

    #[test]
    fn a_click_on_the_same_column_turns_the_order_around() {
        let mut options = BrowseOptions::default();
        assert_eq!(
            (options.order, options.descending),
            (BrowseOrder::Reviews, true)
        );

        sort_by(&mut options, BrowseOrder::Price);
        assert_eq!(
            (options.order, options.descending),
            (BrowseOrder::Price, false)
        );
        sort_by(&mut options, BrowseOrder::Price);
        assert_eq!(
            (options.order, options.descending),
            (BrowseOrder::Price, true)
        );
        sort_by(&mut options, BrowseOrder::Discount);
        assert_eq!(
            (options.order, options.descending),
            (BrowseOrder::Discount, true)
        );
        sort_by(&mut options, BrowseOrder::Name);
        assert_eq!(
            (options.order, options.descending),
            (BrowseOrder::Name, false)
        );
    }

    #[test]
    fn scrolling_down_asks_for_the_rows_in_sight_then_those_below() {
        let offset = 100.0 * ROW_HEIGHT;

        let down = wanted(offset, HEIGHT, 1_000, false);
        let up = wanted(offset, HEIGHT, 1_000, true);

        assert_eq!(down, (100..113).chain(113..126).collect::<Vec<_>>());
        assert!(down.iter().all(|&index| index >= 100));
        assert_eq!(up, (100..113).chain((87..100).rev()).collect::<Vec<_>>());
    }

    #[test]
    fn at_most_a_few_capsules_load_at_once() {
        let mut browse = browse_with(1_000);
        browse.offset = 100.0 * ROW_HEIGHT;

        let started = browse.capsules_to_load();

        assert_eq!(started.len(), MAX_LOADS);
        assert_eq!(
            started,
            (101..101 + MAX_LOADS as u32).map(AppId).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_new_scroll_replaces_the_capsules_waiting() {
        let mut browse = browse_with(1_000);
        browse.offset = 100.0 * ROW_HEIGHT;
        let _ = browse.capsules_to_load();
        for id in 101..101 + MAX_LOADS as u32 {
            browse.capsules.loaded(AppId(id), None);
        }

        browse.offset = 500.0 * ROW_HEIGHT;
        let started = browse.capsules_to_load();

        assert_eq!(
            started,
            (501..501 + MAX_LOADS as u32).map(AppId).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_selection_scrolls_only_when_it_leaves_the_sight() {
        let mut browse = browse_with(1_000);

        let _ = browse.down();
        assert_eq!((browse.selected, browse.offset), (Some(0), 0.0));

        browse.selected = Some(10);
        let _ = browse.down();
        assert_eq!((browse.selected, browse.offset), (Some(11), 0.0));
        let _ = browse.down();
        assert_eq!(browse.selected, Some(12));
        assert_eq!(browse.offset, 13.0 * ROW_HEIGHT - HEIGHT);

        browse.offset = 500.0 * ROW_HEIGHT;
        let _ = browse.up();
        assert_eq!(browse.selected, Some(11));
        assert_eq!(browse.offset, 11.0 * ROW_HEIGHT);
        assert!(browse.upward);
    }

    #[test]
    fn enter_opens_the_selected_row_or_the_first() {
        let mut browse = browse_with(3);
        assert_eq!(browse.chosen(), Some(AppId(1)));

        browse.selected = Some(2);
        assert_eq!(browse.chosen(), Some(AppId(3)));
        assert_eq!(browse_with(0).chosen(), None);
    }

    #[test]
    fn a_click_on_only_asks_for_that_choice_and_nothing_else() {
        let options = BrowseOptions::default();

        let next = changed(&options, &Message::Show(Kind::WouldTry, Choice::Only));

        assert_eq!(next.would_try, Choice::Only);
        assert_eq!(
            BrowseOptions {
                would_try: options.would_try,
                ..next
            },
            options
        );
    }

    #[test]
    fn each_kind_has_its_own_choice() {
        for kind in KINDS {
            for choice in [Choice::Without, Choice::Any, Choice::Only] {
                let mut options = BrowseOptions::default();
                kind.set(&mut options, choice);

                assert_eq!(kind.choice(&options), choice, "{kind:?}");
                for other in KINDS.into_iter().filter(|other| *other != kind) {
                    assert_eq!(
                        other.choice(&options),
                        other.choice(&BrowseOptions::default()),
                        "{kind:?} changed {other:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn on_sale_off_drops_the_minimum_discount() {
        let mut options = BrowseOptions::default();
        options.filters.min_discount_percent = Some(50);
        options.filters.max_price_cents = Some(1500);

        let off = changed(&options, &Message::OnSale(false));

        assert!(!off.filters.only_discounted);
        assert_eq!(off.filters.min_discount_percent, None);
        assert_eq!(off.filters.max_price_cents, Some(1500));
        let on = changed(&off, &Message::OnSale(true));
        assert!(on.filters.only_discounted);
    }

    #[test]
    fn a_message_that_does_not_change_the_list_keeps_the_options() {
        let options = BrowseOptions::default();

        assert_eq!(changed(&options, &Message::ToggleFilters), options);
        assert_eq!(changed(&options, &Message::CloseFilters), options);
    }

    #[test]
    fn a_list_that_fails_keeps_the_filters_and_the_rows_of_before() {
        let mut browse = browse_with(3);
        browse.selected = Some(1);
        browse.offset = 120.0;
        let before = browse.options.clone();
        let mut wrong = before.clone();
        wrong.filters.tags = vec!["Nothing".to_string()];

        let taken = browse.accept(wrong, "", Err("Unknown tag: Nothing".to_string()));

        assert!(!taken);
        assert_eq!(browse.options, before);
        assert_eq!(browse.rows.len(), 3);
        assert_eq!((browse.selected, browse.offset), (Some(1), 120.0));
        assert_eq!(browse.error.as_deref(), Some("Unknown tag: Nothing"));
        assert_eq!(browse.chosen(), Some(AppId(2)));
    }

    #[test]
    fn a_list_that_works_takes_the_options_and_clears_the_error() {
        let mut browse = browse_with(3);
        browse.error = Some("Unknown tag: Nothing".to_string());
        browse.selected = Some(1);
        browse.offset = 120.0;
        let options = BrowseOptions {
            library: Choice::Only,
            ..BrowseOptions::default()
        };

        let taken = browse.accept(options.clone(), "hades", Ok(rows(5)));

        assert!(taken);
        assert_eq!(browse.options, options);
        assert_eq!(browse.rows.len(), 5);
        assert_eq!(browse.error, None);
        assert_eq!(browse.listed.as_deref(), Some("hades"));
        assert_eq!((browse.selected, browse.offset), (None, 0.0));
    }

    #[test]
    fn the_filters_open_and_close() {
        let mut browse = Browse::with_filters_open();
        assert!(browse.filters_open());

        browse.close_filters();
        assert!(!browse.filters_open());

        browse.open_filters();
        assert!(browse.filters_open());
    }

    #[test]
    fn without_a_panel_the_filters_stay_closed() {
        let mut browse = Browse::default();

        browse.open_filters();

        assert!(!browse.filters_open());
    }

    #[test]
    fn a_chip_goes_from_nothing_to_only_to_hide_and_back() {
        let mut choice = Choice::Any;
        let mut seen = Vec::new();
        for _ in 0..3 {
            choice = next_choice(choice);
            seen.push(choice);
        }

        assert_eq!(seen, [Choice::Only, Choice::Without, Choice::Any]);
        assert_eq!(
            [Choice::Any, Choice::Only, Choice::Without].map(chip_state),
            [Chip::Off, Chip::Only, Chip::Hide]
        );
    }

    #[test]
    fn the_tooltip_of_a_chip_says_the_next_click() {
        assert_eq!(
            Kind::Saved.advice(Choice::Any),
            ["Show only Saved".to_string(), String::new()]
        );
        assert_eq!(
            Kind::Saved.advice(Choice::Only)[0],
            "Hide Saved".to_string()
        );
        assert_eq!(
            Kind::Library.advice(Choice::Without),
            [
                "Show Library again".to_string(),
                "Owned, borrowed and played elsewhere".to_string()
            ]
        );
    }

    #[test]
    fn the_button_counts_the_filters_of_the_popup_and_not_the_chips() {
        let mut filters = FilterOptions::default();
        assert_eq!(others_on(&filters), 0);

        filters.only_discounted = true;
        assert_eq!(others_on(&filters), 0, "On sale is a chip");

        filters.min_price_cents = Some(100);
        filters.max_price_cents = Some(500);
        filters.max_reviews = Some(10);
        filters.min_reviews = Some(1);
        filters.tags = vec!["Roguelike".to_string()];
        assert_eq!(others_on(&filters), 3, "a range counts once");
    }

    #[test]
    fn the_scrubber_reaches_both_ends_of_a_huge_list() {
        let last = scrub_last(SALES, HEIGHT);
        let bottom = rows_height(SALES) - HEIGHT;

        assert_eq!(scrub_offset(0, SALES, HEIGHT), 0.0);
        assert_eq!(scrub_offset(last, SALES, HEIGHT), bottom);
        assert_eq!(scrub_offset(last + 10, SALES, HEIGHT), bottom, "no further");
        let middle = scrub_offset(last / 2, SALES, HEIGHT);
        assert!((middle - bottom / 2.0).abs() < 1.0);
        let in_sight = visible(bottom, HEIGHT, SALES);
        assert_eq!(
            in_sight.end, SALES,
            "the last game is in sight at the bottom"
        );
    }

    #[test]
    fn the_scrubber_follows_the_scroll_and_back() {
        for position in [0, 1, 777, 59_000, scrub_last(SALES, HEIGHT)] {
            let offset = scrub_offset(position, SALES, HEIGHT);

            let back = scrub_position(offset, SALES, HEIGHT);

            assert!(
                back.abs_diff(position) <= 1,
                "{position} came back as {back}"
            );
        }
        assert_eq!(scrub_position(-5.0, SALES, HEIGHT), 0);
        assert_eq!(
            scrub_position(f32::MAX, SALES, HEIGHT),
            scrub_last(SALES, HEIGHT)
        );
    }

    #[test]
    fn a_list_that_fits_has_no_scrubber() {
        assert_eq!(scrub_last(5, HEIGHT), 0);
        assert_eq!(scrub_offset(3, 5, HEIGHT), 0.0);
        assert_eq!(scrub_position(10.0, 5, HEIGHT), 0);
    }

    #[test]
    fn the_choices_have_their_words() {
        assert_eq!(
            KINDS.map(Kind::label),
            [
                "Library",
                "Wishlist",
                "Would try",
                "Saved",
                "Not for me",
                "Later"
            ]
        );
        assert_eq!(
            Kind::Library.tip(),
            Some("Owned, borrowed and played elsewhere")
        );
        assert_eq!(Kind::Wishlist.tip(), Some("Wishlist and followed"));
        assert_eq!(Kind::Later.tip(), None);
    }

    #[test]
    fn counts_the_games() {
        assert_eq!(games_count(0), "0 games");
        assert_eq!(games_count(1), "1 game");
        assert_eq!(games_count(7_328), "7,328 games");
    }

    #[test]
    fn the_price_strikes_the_full_one_only_with_a_discount() {
        let currency = Currency::italian();
        let full = Price {
            original_cents: 1999,
            discount: None,
        };
        let sale = Price {
            original_cents: 1999,
            discount: Some(Discount {
                percent: 50,
                final_cents: 999,
                end: Some(Timestamp(0)),
            }),
        };

        assert_eq!(
            price_lines(&full, &currency),
            (format::price(&full, &currency), None)
        );
        assert_eq!(
            price_lines(&sale, &currency),
            (
                format::price(&sale, &currency),
                Some(format::original_price(&sale, &currency))
            )
        );
    }

    #[test]
    fn a_game_without_reviews_has_a_dash() {
        assert_eq!(
            rating_lines(&Reviews {
                total: 0,
                positive: 0
            }),
            ("—".to_string(), None)
        );
        assert_eq!(
            rating_lines(&Reviews {
                total: 1_200,
                positive: 1_080
            }),
            ("90%".to_string(), Some("1,200".to_string()))
        );
    }

    fn in_sight(browse: &Browse, index: usize) -> bool {
        visible(browse.offset, HEIGHT, browse.rows.len()).contains(&index)
    }

    #[test]
    fn a_page_moves_the_rows_that_fit_and_the_list_follows() {
        let mut browse = browse_with(100);
        let page = page_rows(HEIGHT);
        assert_eq!(page, 12);

        let _ = browse.page_down();
        assert_eq!(browse.selected, Some(page));
        let _ = browse.page_down();
        assert_eq!(browse.selected, Some(2 * page));
        assert!(in_sight(&browse, 2 * page));

        let _ = browse.page_up();
        assert_eq!(browse.selected, Some(page));
        assert!(in_sight(&browse, page));
    }

    #[test]
    fn the_pages_stop_at_the_first_and_the_last_row() {
        let mut browse = browse_with(30);

        for _ in 0..5 {
            let _ = browse.page_down();
        }
        assert_eq!(browse.selected, Some(29));
        assert!(in_sight(&browse, 29));

        for _ in 0..5 {
            let _ = browse.page_up();
        }
        assert_eq!(browse.selected, Some(0));
        assert_eq!(browse.offset, 0.0);
    }

    #[test]
    fn the_ends_of_a_list_of_the_whole_catalog() {
        let mut browse = browse_with(120_000);

        let _ = browse.last();
        assert_eq!(browse.selected, Some(119_999));
        assert!(in_sight(&browse, 119_999));
        assert_eq!(browse.offset, rows_height(120_000) - HEIGHT);

        let _ = browse.first();
        assert_eq!(browse.selected, Some(0));
        assert_eq!(browse.offset, 0.0);
    }

    #[test]
    fn the_keys_of_an_empty_list_do_nothing() {
        let mut browse = browse_with(0);

        let _ = browse.page_down();
        let _ = browse.page_up();
        let _ = browse.first();
        let _ = browse.last();

        assert_eq!(browse.selected, None);
    }

    #[test]
    fn a_game_picked_stays_the_selected_row() {
        let mut browse = browse_with(50);

        browse.picked(AppId(7));
        assert_eq!(browse.selected, Some(6));

        browse.picked(AppId(9_999));
        assert_eq!(browse.selected, Some(6));
    }

    #[test]
    fn a_row_after_a_vote_has_the_new_standing_and_the_same_place() {
        let mut browse = browse_with(10);
        browse.selected = Some(3);
        let ids: Vec<AppId> = browse.rows.iter().map(|row| row.id).collect();

        browse.update_row(AppId(4), Some(GameStanding::WouldTry), true);

        assert_eq!(browse.rows[3].standing, Some(GameStanding::WouldTry));
        assert!(browse.rows[3].saved);
        assert!(browse.rows[2].standing.is_none() && !browse.rows[4].saved);
        assert_eq!(
            browse.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            ids
        );
        assert_eq!(browse.selected, Some(3));

        browse.update_row(AppId(4), None, false);
        assert!(browse.rows[3].standing.is_none() && !browse.rows[3].saved);
    }

    #[test]
    fn a_list_made_again_keeps_the_game_at_the_same_height() {
        let mut browse = browse_with(100);
        browse.selected = Some(30);
        browse.offset = 25.0 * ROW_HEIGHT;
        let place = browse.place().expect("a row is selected");
        assert_eq!(place, (AppId(31), 5.0 * f64::from(ROW_HEIGHT)));

        // The first five games are gone: game 31 is now the 26th.
        let rows = rows(100).split_off(5);
        assert!(browse.accept(BrowseOptions::default(), "", Ok(rows)));
        assert_eq!((browse.selected, browse.offset), (None, 0.0));
        browse.keep(place);

        assert_eq!(browse.selected, Some(25));
        assert_eq!(browse.offset, 20.0 * ROW_HEIGHT);
    }

    #[test]
    fn a_game_that_is_gone_leaves_the_list_on_top_and_the_end_stops_the_offset() {
        let mut browse = browse_with(100);
        browse.selected = Some(30);
        browse.offset = 25.0 * ROW_HEIGHT;
        let place = browse.place().expect("a row is selected");

        assert!(browse.accept(BrowseOptions::default(), "", Ok(rows(20).split_off(10))));
        browse.keep(place);
        assert_eq!((browse.selected, browse.offset), (None, 0.0));

        // Near the end, the list cannot scroll past its last row
        browse.selected = Some(8);
        browse.offset = 0.0;
        let place = (AppId(20), -2_000.0);
        browse.keep(place);
        assert_eq!(browse.selected, Some(9));
        assert_eq!(browse.offset, 0.0);
    }

    #[test]
    fn a_list_is_outdated_by_a_new_revision_or_a_new_facade() {
        let mut browse = browse_with(3);
        assert!(browse.outdated_by(4), "nothing was listed on a revision");

        browse.revision = Some(4);
        assert!(!browse.outdated_by(4));
        assert!(browse.outdated_by(5));

        browse.mark_stale();
        assert!(browse.outdated_by(4));

        assert!(browse.accept(BrowseOptions::default(), "", Ok(rows(3))));
        assert!(!browse.outdated_by(4));
    }
}
