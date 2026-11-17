use std::{ops::Range, sync::Arc};

use gamepalate_app::{
    AppId, BrowseOptions, BrowseOrder, BrowseRow, Choice, Currency, DiscoverMode, FilterOptions,
    Gamepalate, Price, Reviews, SearchOptions,
};
use iced::{
    Center, ContentFit, Element, Fill, Length, Right, Size, Task, never, padding,
    widget::{
        Id, button, checkbox, column, container, image, mouse_area, opaque, operation, rich_text,
        row, rule, scrollable,
        scrollable::{AbsoluteOffset, Viewport},
        sensor, space, span, stack, text,
        text::Wrapping,
        tooltip,
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
    style,
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
const DISCOUNT_WIDTH: f32 = 64.0;
const PRICE_WIDTH: f32 = 120.0;
const RATING_WIDTH: f32 = 120.0;
const ENDS_WIDTH: f32 = 110.0;
const NO_GAMES: &str = "No games match";
const SIDE_WIDTH: f32 = 216.0;
const SIDE_PADDING: f32 = 12.0;
const GROUP_WIDTH: f32 = 126.0;
const GROUP_HEIGHT: f32 = 26.0;
const NAME_MIN_WIDTH: f32 = 260.0;
const SUMMARY_CHAR_WIDTH: f32 = 6.5;
/// What the columns after Name take, with the capsule, the gaps, the padding and the scrollbar.
const COLUMNS_WIDTH: f32 = CAPSULE_WIDTH
    + DISCOUNT_WIDTH
    + PRICE_WIDTH
    + RATING_WIDTH
    + ENDS_WIDTH
    + 5.0 * COLUMN_SPACING
    + 2.0 * ROW_PADDING
    + SCROLLBAR_GAP;
const CHOICES: [Choice; 3] = [Choice::Without, Choice::Any, Choice::Only];
const KINDS: [Kind; 6] = [
    Kind::Library,
    Kind::Wishlist,
    Kind::WouldTry,
    Kind::Starred,
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
    Starred,
    NotForMe,
    Later,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Library => "Library",
            Kind::Wishlist => "Wishlist",
            Kind::WouldTry => "Would try",
            Kind::Starred => "Starred",
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

    fn choice(self, options: &BrowseOptions) -> Choice {
        match self {
            Kind::Library => options.library,
            Kind::Wishlist => options.wished,
            Kind::WouldTry => options.would_try,
            Kind::Starred => options.starred,
            Kind::NotForMe => options.not_for_me,
            Kind::Later => options.later,
        }
    }

    fn set(self, options: &mut BrowseOptions, choice: Choice) {
        let place = match self {
            Kind::Library => &mut options.library,
            Kind::Wishlist => &mut options.wished,
            Kind::WouldTry => &mut options.would_try,
            Kind::Starred => &mut options.starred,
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
    ResetAll,
    ToggleFilters,
    CloseFilters,
    Filters(filters::Message),
    Measured(Size),
    Scrolled(Viewport),
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
            Message::Sort(_) | Message::OnSale(_) | Message::Show(..) | Message::ResetAll => {
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
    /// since, otherwise it comes back where it was. The first time, the options are those of
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
        Task::batch([scroll_to(self.offset), self.load_capsules(gamepalate)])
    }

    /// The list made again from the top, with the options of now.
    pub(super) fn relist(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        name: &str,
    ) -> Task<SearchMessage> {
        self.apply(gamepalate, self.options.clone(), name)
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
        let listed = self.apply(gamepalate, options, name);
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

    fn apply(
        &mut self,
        gamepalate: &Arc<Gamepalate>,
        options: BrowseOptions,
        name: &str,
    ) -> Task<SearchMessage> {
        let rows = gamepalate
            .browse(&options, name)
            .map_err(|error| sentence(&error.to_string()));
        self.currency = Some(gamepalate.currency().clone());
        if !self.accept(options, name, rows) {
            return Task::none();
        }

        Task::batch([scroll_to(0.0), self.load_capsules(gamepalate)])
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
        let body: Element<'a, SearchMessage> = if side_beside(size.width) {
            row![
                table,
                rule::vertical(1).style(rule::weak),
                self.side().width(SIDE_WIDTH)
            ]
            .height(Fill)
            .into()
        } else {
            column![self.chips(), style::separator(), table]
                .height(Fill)
                .into()
        };
        let top = sensor(column![field, style::separator(), self.toolbar()])
            .on_show(|size| SearchMessage::Browse(Message::Measured(size)))
            .on_resize(|size| SearchMessage::Browse(Message::Measured(size)));
        let content = container(
            column![top]
                .push(self.active(size.width))
                .push(body)
                .height(Fill),
        )
        .width(size.width)
        .height(size.height);

        stack![content].push(self.popup(size.height)).into()
    }

    /// The count, and the two controls that change the list the most.
    fn toolbar(&self) -> Element<'_, SearchMessage> {
        let arrow = if self.panel_open { '▴' } else { '▾' };

        row![
            text(games_count(self.rows.len()))
                .size(font::NOTE_SIZE)
                .style(style::muted),
            space().width(Fill),
            checkbox(self.options.filters.only_discounted)
                .label("On sale")
                .text_size(font::NOTE_SIZE)
                .on_toggle(|on| SearchMessage::Browse(Message::OnSale(on))),
            button(text!("Filters {arrow}").size(font::NOTE_SIZE))
                .padding([4, 8])
                .style(style::text_button)
                .on_press(SearchMessage::Browse(Message::ToggleFilters)),
        ]
        .spacing(12)
        .align_y(Center)
        .padding(padding::top(6).bottom(6).left(16).right(16))
        .into()
    }

    /// The filters that are on, or why the last ones were not applied.
    fn active(&self, width: f32) -> Option<Element<'_, SearchMessage>> {
        let line = |line: String| {
            text(line)
                .size(font::NOTE_SIZE)
                .style(style::muted)
                .wrapping(Wrapping::None)
        };
        let line: Element<'_, SearchMessage> = if let Some(error) = &self.error {
            text(error)
                .size(font::NOTE_SIZE)
                .style(text::danger)
                .wrapping(Wrapping::None)
                .into()
        } else {
            let summary = format::filters(
                &search_options(&self.options.filters),
                self.currency.as_ref()?,
            )?;
            match format::short_filters(&summary, summary_chars(width)) {
                Some(short) => tooltip(
                    line(short),
                    container(text(summary).size(font::NOTE_SIZE))
                        .padding([6, 10])
                        .max_width(width - 32.0)
                        .style(style::popup),
                    tooltip::Position::Bottom,
                )
                .into(),
                None => line(summary).into(),
            }
        };

        Some(
            container(line)
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

    /// The three-state filters in a column right of the table.
    fn side(&self) -> iced::widget::Column<'_, SearchMessage> {
        column![text("Show").size(font::NOTE_SIZE).style(style::muted)]
            .extend(KINDS.map(|kind| {
                row![
                    container(self.kind_label(kind)).width(Fill),
                    self.choices(kind)
                ]
                .align_y(Center)
                .into()
            }))
            .push(
                button(text("Reset").size(font::NOTE_SIZE))
                    .padding([4, 8])
                    .style(style::text_button)
                    .on_press(SearchMessage::Browse(Message::ResetAll)),
            )
            .spacing(10)
            .padding(SIDE_PADDING)
    }

    /// The same filters in a row that wraps, when the table needs the room.
    fn chips(&self) -> Element<'_, SearchMessage> {
        container(
            row(KINDS.map(|kind| {
                row![self.kind_label(kind), self.choices(kind)]
                    .spacing(8)
                    .align_y(Center)
                    .into()
            }))
            .push(
                button(text("Reset").size(font::NOTE_SIZE))
                    .padding([4, 8])
                    .style(style::text_button)
                    .on_press(SearchMessage::Browse(Message::ResetAll)),
            )
            .spacing(16)
            .wrap()
            .vertical_spacing(8),
        )
        .padding([8, 16])
        .into()
    }

    fn kind_label(&self, kind: Kind) -> Element<'_, SearchMessage> {
        let label = text(kind.label()).size(font::NOTE_SIZE);
        match kind.tip() {
            Some(tip) => tooltip(
                label,
                container(text(tip).size(font::NOTE_SIZE))
                    .padding([6, 10])
                    .style(style::popup),
                tooltip::Position::Bottom,
            )
            .into(),
            None => label.into(),
        }
    }

    /// Hide, Any or Only, as one button in three parts.
    fn choices(&self, kind: Kind) -> Element<'_, SearchMessage> {
        let current = kind.choice(&self.options);
        let part = |index: usize| {
            let choice = CHOICES[index];
            let place = match index {
                0 => style::Segment::First,
                1 => style::Segment::Middle,
                _ => style::Segment::Last,
            };
            button(container(text(choice_label(choice)).size(font::NOTE_SIZE)).center(Fill))
                .width(Fill)
                .height(Fill)
                .padding([0, 4])
                .style(style::segment(choice == current, place))
                .on_press(SearchMessage::Browse(Message::Show(kind, choice)))
        };

        container(
            row![
                part(0),
                rule::vertical(1).style(style::vote_seam),
                part(1),
                rule::vertical(1).style(style::vote_seam),
                part(2),
            ]
            .height(Fill),
        )
        .width(GROUP_WIDTH)
        .height(GROUP_HEIGHT)
        .padding(1)
        .style(style::vote_group)
        .into()
    }

    fn head(&self) -> Element<'_, SearchMessage> {
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
            button(label)
                .padding([4, 0])
                .width(width)
                .style(style::text_button)
                .on_press(SearchMessage::Browse(Message::Sort(order)))
        };

        row![
            space().width(CAPSULE_WIDTH),
            heading(BrowseOrder::Name, Fill, false),
            heading(BrowseOrder::Discount, DISCOUNT_WIDTH.into(), true),
            heading(BrowseOrder::Price, PRICE_WIDTH.into(), true),
            heading(BrowseOrder::Rating, RATING_WIDTH.into(), true),
            heading(BrowseOrder::Ends, ENDS_WIDTH.into(), true),
        ]
        .spacing(COLUMN_SPACING)
        .align_y(Center)
        .padding(
            padding::left(ROW_PADDING)
                .right(ROW_PADDING + SCROLLBAR_GAP)
                .bottom(4),
        )
        .into()
    }

    /// The table, with only the rows of `shown` built: a space stands for the rows above and
    /// one for those below.
    fn list<'a>(
        &self,
        row: impl Fn(usize) -> Element<'a, SearchMessage>,
    ) -> Element<'a, SearchMessage> {
        let count = self.rows.len();
        let shown = shown(self.offset, self.height(), count);
        let above = rows_height(shown.start);
        let below = rows_height(count - shown.end);

        scrollable(
            container(
                column![space().height(above)]
                    .extend(shown.map(row))
                    .push(space().height(below)),
            )
            .padding(padding::right(SCROLLBAR_GAP)),
        )
        .id(LIST)
        .on_scroll(|viewport| SearchMessage::Browse(Message::Scrolled(viewport)))
        .height(Fill)
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
                .push(game.starred.then(|| {
                    text(font::STAR)
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
        Message::ResetAll => next = BrowseOptions::default(),
        _ => {}
    }
    next
}

fn choice_label(choice: Choice) -> &'static str {
    match choice {
        Choice::Without => "Hide",
        Choice::Any => "Any",
        Choice::Only => "Only",
    }
}

/// Whether the three-state filters have a column of their own, and Name still has its room.
fn side_beside(width: f32) -> bool {
    width >= COLUMNS_WIDTH + 1.0 + SIDE_WIDTH + NAME_MIN_WIDTH
}

/// How many characters of the filters' line fit in `width`.
fn summary_chars(width: f32) -> usize {
    ((width - 32.0).max(0.0) / SUMMARY_CHAR_WIDTH) as usize
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
    const HEIGHT: f32 = 700.0;

    fn rows(count: u32) -> Vec<BrowseRow> {
        (1..=count)
            .map(|id| BrowseRow {
                id: AppId(id),
                name: Arc::from(format!("Game {id}")),
                standing: None,
                starred: false,
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
            for choice in CHOICES {
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
    fn reset_gives_the_defaults_filters_and_order_included() {
        let mut options = BrowseOptions::default();
        options.filters.tags = vec!["Roguelike".to_string()];
        options.library = Choice::Only;
        options.order = BrowseOrder::Price;
        options.descending = true;

        assert_eq!(
            changed(&options, &Message::ResetAll),
            BrowseOptions::default()
        );
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
    fn the_three_states_sit_beside_the_table_only_when_name_keeps_its_room() {
        // The narrowest window is 960 wide, and the panel takes 90% of it.
        assert!(!side_beside(960.0 * 0.9));
        assert!(side_beside(1_200.0));
        let least = COLUMNS_WIDTH + 1.0 + SIDE_WIDTH + NAME_MIN_WIDTH;
        assert!(side_beside(least));
        assert!(!side_beside(least - 1.0));
    }

    #[test]
    fn the_summary_takes_the_characters_that_fit() {
        assert_eq!(summary_chars(0.0), 0);
        assert!(summary_chars(864.0) > 100);
    }

    #[test]
    fn the_choices_have_their_words() {
        assert_eq!(CHOICES.map(choice_label), ["Hide", "Any", "Only"]);
        assert_eq!(
            KINDS.map(Kind::label),
            [
                "Library",
                "Wishlist",
                "Would try",
                "Starred",
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
}
