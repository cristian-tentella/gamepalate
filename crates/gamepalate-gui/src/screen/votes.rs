use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    sync::Arc,
};

use gamepalate_app::{
    AppId, Currency, FilterOptions, Gamepalate, Price, SearchOptions, Vote, VoteKind, VotedGame,
    VotesOrder, VotesSettings, VotesShow,
};
use iced::{
    Center, Element, Event, Fill, Size, Subscription, Task, event,
    keyboard::{self, Key, key::Named},
    never, padding,
    widget::{
        Id, button, checkbox, column, container, mouse_area, opaque, operation, pick_list,
        responsive, rich_text, row, rule, scrollable,
        scrollable::{AbsoluteOffset, Viewport},
        sensor, space, span, stack, text,
        text::Wrapping,
        text_input, tooltip,
    },
};

use super::{
    Toasts,
    discover::filters::{self, Action, Panel},
    game::{self, Link, Pick, VOTE_HEIGHT},
    sentence,
    showcase::{self, Showcase},
};
use crate::{font, format, style};

// Two rows of two: the yeses and the Bets on top, the two noes below.
const FILTERS: [[Filter; 2]; 2] = [
    [Filter::WouldTry, Filter::Bets],
    [Filter::NotThisGame, Filter::NotMyGenre],
];
const ORDERS: [Order; 4] = [
    Order::Newest,
    Order::Price,
    Order::Discount,
    Order::BestMatch,
];
const LIST: Id = Id::new("votes-list");
const SEARCH: Id = Id::new("votes-search");
const LIST_WIDTH: f32 = 340.0;
const LIST_PADDING: f32 = 12.0;
// The summary of the filters fits the list on one line.
const SUMMARY_CHARS: usize = 46;
const ROW_HEIGHT: f32 = 32.0;
const SCROLLBAR_GAP: f32 = 12.0;
const SIGN_SIZE: f32 = 11.0;
const SIGN_WIDTH: f32 = 18.0;
const NO_VOTES: &str = "No votes yet. Vote in Discover with keys 1–4.";

pub struct Votes {
    gamepalate: Arc<Gamepalate>,
    games: BTreeMap<AppId, VotedGame>,
    filter: Filter,
    // The chip saved in gui.toml: it changes only when a chip is chosen here.
    chip: VotesShow,
    order: Order,
    hide_owned: bool,
    filters: FilterOptions,
    passing: Option<BTreeSet<AppId>>,
    panel: Panel,
    panel_open: bool,
    tools_height: f32,
    search: String,
    matches: Option<HashMap<AppId, f64>>,
    matching: u64,
    rows: Vec<AppId>,
    selected: usize,
    viewport: Option<(f32, f32)>,
    toasts: Toasts,
    played: Option<AppId>,
    showcase: Showcase,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    WouldTry,
    Bets,
    NotThisGame,
    NotMyGenre,
    /// No chip chosen: every vote, the Laters too.
    All,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    Newest,
    Price,
    Discount,
    BestMatch,
}

#[derive(Clone, Debug)]
pub enum Message {
    Filter(Filter),
    Order(Order),
    HideOwned(bool),
    Filters(filters::Message),
    ToggleFilters,
    CloseFilters,
    ResetFilters,
    ToolsMeasured(Size),
    Search(String),
    FocusSearch,
    Select(usize),
    Previous,
    Next,
    Scrolled(Viewport),
    Vote(Pick),
    VoteSaved(Result<(), String>),
    Played,
    PlayedSaved(AppId, String, Result<bool, String>),
    Undo,
    Undone(Result<(), String>),
    SettingsSaved(Result<(), String>),
    Matched(u64, HashMap<AppId, f64>),
    Open(Link),
    HideToast(u64),
    Showcase(showcase::Message),
}

impl Votes {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Votes, Task<Message>) {
        Votes::opened(gamepalate, None)
    }

    /// Votes on a chip of its own, without saving it.
    pub fn filtered(gamepalate: &Arc<Gamepalate>, filter: Filter) -> (Votes, Task<Message>) {
        Votes::opened(gamepalate, Some(filter))
    }

    fn opened(gamepalate: &Arc<Gamepalate>, chosen: Option<Filter>) -> (Votes, Task<Message>) {
        let saved = gamepalate.gui_settings().votes;
        let (filter, chip) = opening_chip(&saved, chosen);
        let (filters, passing, error) = match passing(gamepalate, &saved.filters) {
            Ok(passing) => (saved.filters, passing, None),
            Err(error) => (FilterOptions::default(), None, Some(error)),
        };
        let mut votes = Votes {
            gamepalate: Arc::clone(gamepalate),
            games: by_id(gamepalate.voted_games()),
            filter,
            chip,
            order: saved.order.into(),
            hide_owned: saved.hide_owned,
            filters,
            passing,
            panel: Panel::new(
                gamepalate.tag_names(),
                &FilterOptions::default(),
                format::currency_symbol(gamepalate.currency()),
                &SearchOptions::default(),
            )
            .without_bets(),
            panel_open: false,
            tools_height: 0.0,
            search: String::new(),
            matches: None,
            matching: 0,
            rows: Vec::new(),
            selected: 0,
            viewport: None,
            toasts: Toasts::default(),
            played: None,
            showcase: Showcase::new(gamepalate),
        };
        votes.rows = votes.shown_rows();
        let toast = error.map_or_else(Task::none, |error| votes.show_toast(error));
        let task = Task::batch([votes.load(), votes.find_matches(), toast]);

        (votes, task)
    }

    pub fn showing(gamepalate: &Arc<Gamepalate>, id: AppId) -> (Votes, Task<Message>) {
        let (mut votes, task) = Votes::new(gamepalate);
        let Some(filter) = votes.games.get(&id).map(|game| Filter::of(&game.vote)) else {
            return (votes, task);
        };
        votes.filter = filter;
        votes.rows = votes.shown_rows();
        votes.selected = votes.rows.iter().position(|&row| row == id).unwrap_or(0);
        let scroll = operation::scroll_to(
            LIST,
            AbsoluteOffset {
                x: None,
                y: Some(votes.selected as f32 * ROW_HEIGHT),
            },
        );
        let load = votes.load();

        (votes, Task::batch([task, load, scroll]))
    }

    pub fn resized(&mut self) {
        self.showcase.resized();
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let message = scrolled_by_popup(message, self.showcase.is_popup_open());
        if self.showcase.is_open() && stopped_by_popup(&message) {
            return Task::none();
        }
        if self.panel_open && acts_on_the_page(&message) {
            return Task::none();
        }
        let task = self.handle(message);

        Task::batch([task, self.load()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Filter(filter) => {
                self.filter = chosen_filter(self.filter, filter);
                self.chip = self.filter.into();
                self.search.clear();
                return Task::batch([self.refresh(), self.save_settings()]);
            }
            Message::Order(order) => {
                self.order = order;
                return Task::batch([self.restart(), self.save_settings()]);
            }
            Message::HideOwned(hide_owned) => {
                self.hide_owned = hide_owned;
                return Task::batch([self.refresh(), self.save_settings()]);
            }
            Message::Filters(message) => return self.change_filters(message),
            Message::ToggleFilters if self.panel_open => self.panel_open = false,
            Message::ToggleFilters => {
                self.panel_open = true;
                self.panel.show(&search_options(&self.filters));
            }
            Message::CloseFilters => self.panel_open = false,
            Message::ResetFilters => {
                self.filters = FilterOptions::default();
                self.passing = None;
                return Task::batch([self.refresh(), self.save_settings()]);
            }
            Message::ToolsMeasured(size) => self.tools_height = size.height,
            Message::Search(search) => {
                self.search = search;
                return self.refresh();
            }
            Message::FocusSearch => return operation::focus(SEARCH),
            Message::Select(index) => self.selected = index,
            Message::Previous => {
                self.selected = self.selected.saturating_sub(1);
                return self.reveal();
            }
            Message::Next => return self.next(),
            Message::Scrolled(viewport) => {
                self.viewport = Some((viewport.absolute_offset().y, viewport.bounds().height));
            }
            Message::Vote(pick) => {
                if let (Some(game), Some((kind, genre_ok))) = (self.current(), pick.ballot()) {
                    let text = format::voted(&game.name, pick);
                    let id = game.id;
                    let gamepalate = Arc::clone(&self.gamepalate);
                    let save = Task::perform(
                        async move {
                            gamepalate
                                .vote(id, kind, genre_ok, false, false)
                                .map_err(|error| error.to_string())
                        },
                        Message::VoteSaved,
                    );
                    return Task::batch([save, self.show_toast(text), self.next()]);
                }
            }
            Message::VoteSaved(Ok(())) => {
                self.games = by_id(self.gamepalate.voted_games());
                // A game voted into another kind leaves the list of this filter at once.
                return Task::batch([self.refresh(), self.find_matches()]);
            }
            Message::VoteSaved(Err(error)) => return self.show_toast(error),
            Message::Played => {
                if let Some(game) = self.current() {
                    let (id, name) = (game.id, game.name.clone());
                    let gamepalate = Arc::clone(&self.gamepalate);
                    return Task::perform(
                        async move {
                            gamepalate
                                .set_played_elsewhere(id, true)
                                .map_err(|error| sentence(&error.to_string()))
                        },
                        move |result| Message::PlayedSaved(id, name.clone(), result),
                    );
                }
            }
            Message::PlayedSaved(id, name, Ok(before)) => {
                self.played = (!before).then_some(id);
                self.games = by_id(self.gamepalate.voted_games());
                let text = format::elsewhere_text(&name, true);
                let toast = self.toasts.show(text, !before, Message::HideToast);
                return Task::batch([toast, self.refresh()]);
            }
            Message::PlayedSaved(_, _, Err(error)) => return self.show_toast(error),
            Message::Undo => {
                if let Some(id) = self.played.take() {
                    let gamepalate = Arc::clone(&self.gamepalate);
                    return Task::perform(
                        async move {
                            gamepalate
                                .set_played_elsewhere(id, false)
                                .map(|_| ())
                                .map_err(|error| sentence(&error.to_string()))
                        },
                        Message::Undone,
                    );
                }
            }
            Message::Undone(Ok(())) => {
                self.games = by_id(self.gamepalate.voted_games());
                let toast = self.show_toast("Undone".to_string());
                return Task::batch([toast, self.refresh()]);
            }
            Message::Undone(Err(error)) => return self.show_toast(error),
            Message::SettingsSaved(Ok(())) => {}
            Message::SettingsSaved(Err(error)) => return self.show_toast(error),
            Message::Matched(matching, matches) => {
                if matching == self.matching {
                    let first = self.matches.replace(matches).is_none();
                    if first && self.order == Order::BestMatch {
                        return self.restart();
                    }
                }
            }
            Message::Open(link) => {
                if let Some(game) = self.current() {
                    game::open(link, &game.name, &game.store_url);
                }
            }
            Message::HideToast(id) => self.toasts.hide(id),
            Message::Showcase(message) => {
                return self.showcase.update(message).map(Message::Showcase);
            }
        }

        Task::none()
    }

    pub fn subscription(keys: bool) -> Subscription<Message> {
        if !keys {
            return Subscription::none();
        }
        Subscription::batch([
            keyboard::listen().filter_map(key_message),
            event::listen_with(escape_message),
        ])
    }

    pub fn view(&self, scale: f32) -> Element<'_, Message> {
        let toast = self.toasts.view(Message::Undo);
        let card = container(self.card(scale))
            .padding(16)
            .width(Fill)
            .height(Fill)
            .clip(true);
        let votes =
            container(container(responsive(|size| self.vote_row(size.width))).height(VOTE_HEIGHT))
                .padding([10, 20])
                .width(Fill);

        let popup = self.panel_open.then(|| {
            let panel = container(scrollable(self.panel.view().map(Message::Filters)).spacing(8))
                .padding(20)
                .style(style::popup);
            stack![
                opaque(
                    mouse_area(space().width(Fill).height(Fill)).on_press(Message::CloseFilters)
                ),
                container(opaque(panel)).padding(
                    padding::top(LIST_PADDING + self.tools_height + 8.0).left(LIST_PADDING)
                ),
            ]
        });

        stack![row![
            container(self.list()).width(LIST_WIDTH).height(Fill),
            rule::vertical(1).style(rule::weak),
            column![stack![card].push(toast), style::separator(), votes],
        ]]
        .push(popup)
        .push(self.showcase.zoom().map(|zoom| zoom.map(Message::Showcase)))
        .push(
            self.current()
                .and_then(|game| self.showcase.popup(&game.name))
                .map(|popup| popup.map(Message::Showcase)),
        )
        .into()
    }

    fn list(&self) -> Element<'_, Message> {
        let searching = !self.search.trim().is_empty();
        let chip = |filter: Filter| {
            let count = count(&self.games, filter, self.shown());
            button(
                text!("{} {}", filter_label(filter), format::count(count as u32))
                    .size(font::NOTE_SIZE)
                    .wrapping(Wrapping::None)
                    .width(Fill)
                    .align_x(Center),
            )
            .width(Fill)
            .padding([3, 10])
            .style(style::choice(!searching && filter == self.filter))
            .on_press(Message::Filter(filter))
            .into()
        };
        let filters = column(FILTERS.map(|line| row(line.map(chip)).spacing(6).into())).spacing(6);
        let search = text_input("Search (/)", &self.search)
            .style(style::field)
            .id(SEARCH)
            .on_input(Message::Search)
            .size(font::NOTE_SIZE)
            .padding([5, 8])
            .width(Fill);
        let hide_owned = tooltip(
            checkbox(self.hide_owned)
                .label("Hide owned")
                .text_size(font::NOTE_SIZE)
                .on_toggle(Message::HideOwned),
            container(
                text("Also hides borrowed games and games played elsewhere").size(font::NOTE_SIZE),
            )
            .padding([6, 10])
            .style(style::popup),
            tooltip::Position::Bottom,
        );
        let arrow = if self.panel_open { '▴' } else { '▾' };
        let tools = row![
            pick_list(ORDERS, Some(self.order), Message::Order)
                .text_size(font::NOTE_SIZE)
                .padding([4, 8])
                .style(style::dropdown)
                .menu_style(style::menu),
            button(text!("Filters {arrow}").size(font::NOTE_SIZE))
                .padding([4, 8])
                .style(style::text_button)
                .on_press(Message::ToggleFilters),
            hide_owned,
        ]
        .spacing(8)
        .align_y(Center);
        let active = self.filters_summary().map(|summary| {
            let line = |line: String| {
                text(line)
                    .size(font::NOTE_SIZE)
                    .style(style::muted)
                    .wrapping(Wrapping::None)
            };
            let line: Element<'_, Message> = match format::short_filters(&summary, SUMMARY_CHARS) {
                Some(short) => tooltip(
                    line(short),
                    container(text(summary).size(font::NOTE_SIZE))
                        .padding([6, 10])
                        .max_width(LIST_WIDTH)
                        .style(style::popup),
                    tooltip::Position::Bottom,
                )
                .into(),
                None => line(summary).into(),
            };
            container(line).width(Fill).clip(true)
        });
        let summary = (!searching && self.filter == Filter::Bets)
            .then(|| bets_summary(&self.games))
            .flatten()
            .map(|(would_try, bets, others)| {
                text(format::bets_summary(would_try, bets, others))
                    .size(font::NOTE_SIZE)
                    .style(style::muted)
            });

        let sorting = self.order == Order::BestMatch && self.matches.is_none();
        let body: Element<'_, Message> = if sorting || self.rows.is_empty() {
            let note = if sorting {
                "Sorting…".to_string()
            } else {
                empty_text(
                    self.games.is_empty(),
                    self.filter,
                    self.passing.is_some(),
                    self.owned_hidden(),
                    &self.search,
                )
            };
            let reset = (!sorting && !searching && self.passing.is_some()).then(|| {
                button(text("Reset filters").size(font::NOTE_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::ResetFilters)
            });
            container(
                column![text(note).size(font::NOTE_SIZE).style(style::muted)]
                    .push(reset)
                    .spacing(8),
            )
            .padding(16)
            .into()
        } else {
            scrollable(
                container(column(self.rows.iter().enumerate().filter_map(
                    |(index, id)| self.games.get(id).map(|game| self.list_row(index, game)),
                )))
                .padding(padding::right(SCROLLBAR_GAP)),
            )
            .id(LIST)
            .on_scroll(Message::Scrolled)
            .height(Fill)
            .into()
        };

        let measured = sensor(column![filters, search, tools].spacing(10))
            .on_show(Message::ToolsMeasured)
            .on_resize(Message::ToolsMeasured);

        column![
            column![measured]
                .push(active)
                .push(summary)
                .spacing(10)
                .padding(LIST_PADDING),
            style::separator(),
            body,
        ]
        .into()
    }

    fn list_row<'a>(&self, index: usize, game: &'a VotedGame) -> Element<'a, Message> {
        let note = |line: String| text(line).size(font::NOTE_SIZE).wrapping(Wrapping::None);
        let sign = vote_sign(self.filter, &game.vote);
        // A Later keeps the column empty, so its name lines up with the others.
        let thumb = sign.map(|sign| {
            let thumb: Element<'a, Message> = match sign.thumb() {
                Some(thumb) => text(thumb)
                    .font(font::ICONS)
                    .size(font::icon(SIGN_SIZE))
                    .style(style::muted)
                    .into(),
                None => space().into(),
            };
            container(thumb).width(SIGN_WIDTH)
        });
        let later =
            (sign == Some(Sign::Later)).then(|| note("Later ·".to_string()).style(style::muted));
        let name = container(
            row![]
                .push(later)
                .push(
                    text(&game.name)
                        .size(font::BAR_SIZE)
                        .wrapping(Wrapping::None),
                )
                .spacing(4)
                .align_y(Center),
        )
        .width(Fill)
        .clip(true);
        let positive = game
            .reviews
            .as_ref()
            .map(|reviews| format::reviews(reviews).0)
            .filter(|percent| !percent.is_empty())
            .map(|percent| note(percent).style(style::muted));
        let price = note(price_text(game.price, self.gamepalate.currency()));
        let discount = game
            .price
            .and_then(|price| price.discount)
            .map(|discount| note(format::discount(discount.percent)).style(style::accent));

        button(
            row![]
                .push(thumb)
                .push(name)
                .push(positive)
                .push(row![price].push(discount).spacing(6))
                .spacing(8)
                .height(Fill)
                .align_y(Center),
        )
        .width(Fill)
        .height(ROW_HEIGHT)
        .padding([0, 12])
        .style(style::list_row(index == self.selected))
        .on_press(Message::Select(index))
        .into()
    }

    fn card(&self, scale: f32) -> Element<'_, Message> {
        match self.current() {
            Some(game) => responsive(move |size| self.details(game, size, scale)).into(),
            None => {
                let note = if self.games.is_empty() { NO_VOTES } else { "" };
                container(text(note).style(style::muted))
                    .center(Fill)
                    .into()
            }
        }
    }

    fn details<'a>(&'a self, game: &'a VotedGame, size: Size, scale: f32) -> Element<'a, Message> {
        let today = jiff::Zoned::now().date();
        let price = match &game.price {
            Some(price) => game::price_row(price, self.gamepalate.currency(), game::PRICE_GAP),
            None => text("Not on sale on Steam")
                .size(font::INFO_SIZE)
                .style(style::muted)
                .into(),
        };
        let voted = rich_text![
            span("Voted "),
            span(format::pick(Pick::of(&game.vote))).font(font::SEMIBOLD),
            span(format!(" · {}", format::voted_day(&game.vote.date, today))),
        ]
        .size(font::BAR_SIZE)
        .style(style::muted)
        .on_link_click(never);
        let showcase = &self.showcase;
        let bet = game.vote.bet.then(|| self.gamepalate.bet_every());
        let links: Element<'a, Message> =
            game::link_buttons(font::NOTE_SIZE, |link| Some(Message::Open(link))).into();

        game::body(
            game::Body {
                heading: column![game::title(&game.name), voted].spacing(4).into(),
                figures: row![]
                    .push(game.reviews.as_ref().map(|reviews| {
                        game::reviews(reviews, Message::Showcase(showcase::Message::ToggleReviews))
                    }))
                    .push(price)
                    .spacing(28)
                    .into(),
                gallery: Box::new(move |width, height| {
                    showcase
                        .gallery(game.id, width, height)
                        .map(Message::Showcase)
                }),
                header: Box::new(move |width, height| {
                    game::with_bet(
                        showcase
                            .header(game.id, width, height)
                            .map(Message::Showcase),
                        bet,
                    )
                }),
                side: [
                    showcase.description(game.id),
                    showcase.facts(game.id, &showcase.used_tags()),
                ]
                .into_iter()
                .flatten()
                .map(|part| part.map(Message::Showcase))
                .chain([links])
                .collect(),
            },
            size,
            showcase::max_screenshot_width(scale),
        )
    }

    fn vote_row(&self, width: f32) -> Element<'_, Message> {
        let game = self.current();
        let vote_width = game::vote_width(width, 0.0);
        let (would_try_hours, elsewhere_hours) = (
            self.gamepalate.would_try_hours(),
            self.gamepalate.elsewhere_hours(),
        );
        let votes = game::vote_buttons(
            |key, pick| {
                let message = match pick {
                    Pick::Played => Message::Played,
                    pick => Message::Vote(pick),
                };
                let current = game.is_some_and(|game| Pick::of(&game.vote) == pick);
                game::vote_button(
                    key,
                    pick,
                    game::tip(pick, would_try_hours, elsewhere_hours),
                    true,
                    vote_width,
                    game.is_some().then_some(message),
                    current,
                )
            },
            true,
        )
        .width(Fill);

        container(votes).center_y(Fill).into()
    }

    fn current(&self) -> Option<&VotedGame> {
        self.rows
            .get(self.selected)
            .and_then(|id| self.games.get(id))
    }

    fn next(&mut self) -> Task<Message> {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
        self.reveal()
    }

    fn refresh(&mut self) -> Task<Message> {
        let selected = self.current().map(|game| game.id);
        self.rows = self.shown_rows();
        self.selected = kept_selection(&self.rows, selected, self.selected);

        self.reveal()
    }

    fn restart(&mut self) -> Task<Message> {
        self.rows = self.shown_rows();
        self.selected = 0;

        operation::scroll_to(
            LIST,
            AbsoluteOffset {
                x: None,
                y: Some(0.0),
            },
        )
    }

    fn shown(&self) -> Shown<'_> {
        Shown {
            hide_owned: self.hide_owned,
            passing: self.passing.as_ref(),
        }
    }

    fn filters_summary(&self) -> Option<String> {
        filters_summary(&self.filters, self.gamepalate.currency())
    }

    fn change_filters(&mut self, message: filters::Message) -> Task<Message> {
        match self.panel.update(&search_options(&self.filters), message) {
            Action::Stay => Task::none(),
            Action::Close => {
                self.panel_open = false;
                Task::none()
            }
            Action::Search(options) => {
                self.panel_open = false;
                match passing(&self.gamepalate, &options.filters) {
                    Ok(passing) => {
                        self.filters = options.filters;
                        self.passing = passing;
                        Task::batch([self.refresh(), self.save_settings()])
                    }
                    Err(error) => self.show_toast(error),
                }
            }
        }
    }

    fn save_settings(&self) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        let settings = VotesSettings {
            show: self.chip,
            order: self.order.into(),
            hide_owned: self.hide_owned,
            filters: self.filters.clone(),
        };

        Task::perform(
            async move {
                gamepalate
                    .change_gui_settings(|gui| gui.votes = settings)
                    .map_err(|error| sentence(&error.to_string()))
            },
            Message::SettingsSaved,
        )
    }

    /// True when Hide owned is the only reason the chip has no games left.
    fn owned_hidden(&self) -> bool {
        self.hide_owned && count(&self.games, self.filter, Shown::default()) > 0
    }

    fn shown_rows(&self) -> Vec<AppId> {
        rows(
            &self.games,
            self.filter,
            self.order,
            self.shown(),
            &self.search,
            self.matches.as_ref(),
        )
    }

    fn find_matches(&mut self) -> Task<Message> {
        self.matching += 1;
        let matching = self.matching;
        let gamepalate = Arc::clone(&self.gamepalate);

        Task::perform(async move { gamepalate.best_matches() }, move |matches| {
            Message::Matched(matching, matches)
        })
    }

    fn reveal(&self) -> Task<Message> {
        let Some((offset, height)) = self.viewport else {
            return Task::none();
        };

        match scroll_target(self.selected, offset, height) {
            Some(y) => operation::scroll_to(
                LIST,
                AbsoluteOffset {
                    x: None,
                    y: Some(y),
                },
            ),
            None => Task::none(),
        }
    }

    fn load(&mut self) -> Task<Message> {
        let has_reviews = self
            .current()
            .and_then(|game| game.reviews.as_ref())
            .is_some_and(|reviews| reviews.total > 0);
        self.showcase
            .follow(self.current().map(|game| game.id), has_reviews);
        let end = (self.selected + showcase::AHEAD + 1).min(self.rows.len());
        let start = self.selected.saturating_sub(showcase::BEHIND).min(end);
        let upcoming = &self.rows[self.selected.min(end)..end];
        let nearby = &self.rows[start..end];

        self.showcase
            .load(upcoming, nearby, &[])
            .map(Message::Showcase)
    }

    fn show_toast(&mut self, text: String) -> Task<Message> {
        self.toasts.show(text, false, Message::HideToast)
    }
}

impl Filter {
    fn keeps(self, vote: &Vote) -> bool {
        match self {
            Filter::WouldTry => vote.kind == VoteKind::WouldTry,
            Filter::Bets => vote.bet,
            Filter::NotThisGame => Pick::of(vote) == Pick::NotThisOne,
            Filter::NotMyGenre => Pick::of(vote) == Pick::Vote(VoteKind::NotForMe),
            Filter::All => true,
        }
    }

    /// The chip of a vote; a Later has none, so it shows in All.
    fn of(vote: &Vote) -> Filter {
        [Filter::WouldTry, Filter::NotThisGame, Filter::NotMyGenre]
            .into_iter()
            .find(|filter| filter.keeps(vote))
            .unwrap_or(Filter::All)
    }
}

/// A click on the chosen chip lets it go, and the list shows every vote.
fn chosen_filter(current: Filter, clicked: Filter) -> Filter {
    if clicked == current {
        Filter::All
    } else {
        clicked
    }
}

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Order::Newest => "Newest",
            Order::Price => "Price",
            Order::Discount => "Discount",
            Order::BestMatch => "Best match",
        })
    }
}

pub(super) fn by_id(games: Vec<VotedGame>) -> BTreeMap<AppId, VotedGame> {
    games.into_iter().map(|game| (game.id, game)).collect()
}

fn filter_label(filter: Filter) -> &'static str {
    match filter {
        Filter::WouldTry => format::pick(Pick::Vote(VoteKind::WouldTry)),
        Filter::Bets => "Bets",
        Filter::NotThisGame => format::pick(Pick::NotThisOne),
        Filter::NotMyGenre => format::pick(Pick::Vote(VoteKind::NotForMe)),
        Filter::All => "All",
    }
}

/// The vote beside the name of a row.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sign {
    Up,
    Down,
    Later,
}

impl Sign {
    fn thumb(self) -> Option<char> {
        match self {
            Sign::Up => Some(font::THUMB_UP),
            Sign::Down => Some(font::THUMB_DOWN),
            Sign::Later => None,
        }
    }
}

/// Only the chips that mix the votes show them; in the others the chip says the vote.
fn vote_sign(filter: Filter, vote: &Vote) -> Option<Sign> {
    if !matches!(filter, Filter::Bets | Filter::All) {
        return None;
    }
    Some(match vote.kind {
        VoteKind::WouldTry => Sign::Up,
        VoteKind::NotForMe => Sign::Down,
        VoteKind::Later => Sign::Later,
    })
}

fn price_text(price: Option<Price>, currency: &Currency) -> String {
    price.map_or_else(|| "—".to_string(), |price| format::price(&price, currency))
}

/// The chip shown and the chip saved: a chip chosen from outside is not saved.
fn opening_chip(saved: &VotesSettings, chosen: Option<Filter>) -> (Filter, VotesShow) {
    (chosen.unwrap_or(saved.show.into()), saved.show)
}

impl From<VotesShow> for Filter {
    fn from(show: VotesShow) -> Filter {
        match show {
            VotesShow::WouldTry => Filter::WouldTry,
            VotesShow::Bets => Filter::Bets,
            VotesShow::NotThisGame => Filter::NotThisGame,
            VotesShow::NotMyGenre => Filter::NotMyGenre,
            VotesShow::All => Filter::All,
        }
    }
}

impl From<Filter> for VotesShow {
    fn from(filter: Filter) -> VotesShow {
        match filter {
            Filter::WouldTry => VotesShow::WouldTry,
            Filter::Bets => VotesShow::Bets,
            Filter::NotThisGame => VotesShow::NotThisGame,
            Filter::NotMyGenre => VotesShow::NotMyGenre,
            Filter::All => VotesShow::All,
        }
    }
}

impl From<VotesOrder> for Order {
    fn from(order: VotesOrder) -> Order {
        match order {
            VotesOrder::Newest => Order::Newest,
            VotesOrder::Price => Order::Price,
            VotesOrder::Discount => Order::Discount,
            VotesOrder::BestMatch => Order::BestMatch,
        }
    }
}

impl From<Order> for VotesOrder {
    fn from(order: Order) -> VotesOrder {
        match order {
            Order::Newest => VotesOrder::Newest,
            Order::Price => VotesOrder::Price,
            Order::Discount => VotesOrder::Discount,
            Order::BestMatch => VotesOrder::BestMatch,
        }
    }
}

/// The options the panel works on: only the filters, without Bets only.
fn search_options(filters: &FilterOptions) -> SearchOptions {
    SearchOptions {
        filters: filters.clone(),
        ..SearchOptions::default()
    }
}

fn filters_summary(filters: &FilterOptions, currency: &Currency) -> Option<String> {
    format::filters(&search_options(filters), currency)
}

/// The voted games that pass the filters, or `None` when no filter is chosen.
fn passing(
    gamepalate: &Gamepalate,
    filters: &FilterOptions,
) -> Result<Option<BTreeSet<AppId>>, String> {
    if *filters == FilterOptions::default() {
        return Ok(None);
    }
    gamepalate
        .votes_passing(filters)
        .map(Some)
        .map_err(|error| sentence(&error.to_string()))
}

/// Which games of a chip the list shows, besides the chip itself.
#[derive(Clone, Copy, Default)]
struct Shown<'a> {
    hide_owned: bool,
    /// The games that pass the filters of the panel, when some are chosen.
    passing: Option<&'a BTreeSet<AppId>>,
}

impl Shown<'_> {
    fn keeps(self, game: &VotedGame) -> bool {
        !(self.hide_owned && game.owned)
            && self
                .passing
                .is_none_or(|passing| passing.contains(&game.id))
    }
}

fn count(games: &BTreeMap<AppId, VotedGame>, filter: Filter, shown: Shown) -> usize {
    games
        .values()
        .filter(|game| filter.keeps(&game.vote) && shown.keeps(game))
        .count()
}

/// Where the selection goes once the rows change: on the same game if it is still
/// there, otherwise on the row that took its place.
fn kept_selection(rows: &[AppId], selected: Option<AppId>, index: usize) -> usize {
    selected
        .and_then(|id| rows.iter().position(|&row| row == id))
        .unwrap_or_else(|| index.min(rows.len().saturating_sub(1)))
}

fn rows(
    games: &BTreeMap<AppId, VotedGame>,
    filter: Filter,
    order: Order,
    shown: Shown,
    search: &str,
    matches: Option<&HashMap<AppId, f64>>,
) -> Vec<AppId> {
    let search = search.trim().to_lowercase();
    let mut rows: Vec<&VotedGame> = games
        .values()
        .filter(|game| {
            if search.is_empty() {
                filter.keeps(&game.vote) && shown.keeps(game)
            } else {
                game.name.to_lowercase().contains(&search)
            }
        })
        .collect();
    let price = |game: &VotedGame| game.price.map_or(u32::MAX, |price| price.final_cents());
    let discount = |game: &VotedGame| {
        game.price
            .and_then(|price| price.discount)
            .map_or(0, |discount| discount.percent)
    };
    let score = |game: &VotedGame| {
        matches
            .and_then(|matches| matches.get(&game.id))
            .copied()
            .unwrap_or(f64::NEG_INFINITY)
    };

    match order {
        Order::Newest => rows
            .sort_by_cached_key(|game| (Reverse(game.vote.date.clone()), game.name.to_lowercase())),
        Order::Price => rows.sort_by_cached_key(|game| (price(game), game.name.to_lowercase())),
        Order::Discount => rows.sort_by_cached_key(|game| {
            (
                Reverse(discount(game)),
                price(game),
                game.name.to_lowercase(),
            )
        }),
        Order::BestMatch => rows.sort_by(|a, b| {
            score(b)
                .total_cmp(&score(a))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
    }

    rows.into_iter().map(|game| game.id).collect()
}

pub(super) fn bets_summary(
    games: &BTreeMap<AppId, VotedGame>,
) -> Option<(usize, usize, Option<f64>)> {
    let first = games
        .values()
        .filter(|game| game.vote.bet)
        .map(|game| game.vote.date.as_str())
        .min()?;
    let (bets, others): (Vec<&Vote>, Vec<&Vote>) = games
        .values()
        .map(|game| &game.vote)
        .filter(|vote| vote.kind.label().is_some() && !vote.searched)
        .filter(|vote| vote.bet || vote.date.as_str() >= first)
        .partition(|vote| vote.bet);
    let would_try = |votes: &[&Vote]| votes.iter().filter(|vote| vote.kind.is_yes()).count();
    let others_share =
        (!others.is_empty()).then(|| would_try(&others) as f64 / others.len() as f64);

    Some((would_try(&bets), bets.len(), others_share))
}

fn empty_text(
    no_votes: bool,
    filter: Filter,
    filtered: bool,
    owned_hidden: bool,
    search: &str,
) -> String {
    let search = search.trim();
    if no_votes {
        NO_VOTES.to_string()
    } else if !search.is_empty() {
        format!("No votes match \"{search}\"")
    } else if filtered {
        match filter {
            Filter::Bets => "No Bets match the filters".to_string(),
            Filter::All => "No votes match the filters".to_string(),
            _ => format!("No {} votes match the filters", filter_label(filter)),
        }
    } else if owned_hidden {
        match filter {
            Filter::Bets => "You own all your Bets".to_string(),
            Filter::All => "You own all your voted games".to_string(),
            _ => format!("You own all your {} games", filter_label(filter)),
        }
    } else if filter == Filter::Bets {
        "No votes on Bets".to_string()
    } else {
        format!("No {} votes", filter_label(filter))
    }
}

fn scroll_target(index: usize, offset: f32, height: f32) -> Option<f32> {
    let top = index as f32 * ROW_HEIGHT;
    let bottom = top + ROW_HEIGHT;

    if top < offset {
        Some(top)
    } else if bottom > offset + height {
        Some(bottom - height)
    } else {
        None
    }
}

fn scrolled_by_popup(message: Message, popup_open: bool) -> Message {
    match message {
        Message::Previous if popup_open => Message::Showcase(showcase::Message::ScrollUp),
        Message::Next if popup_open => Message::Showcase(showcase::Message::ScrollDown),
        message => message,
    }
}

fn stopped_by_popup(message: &Message) -> bool {
    matches!(
        message,
        Message::Previous
            | Message::Next
            | Message::Vote(_)
            | Message::Played
            | Message::Open(_)
            | Message::FocusSearch
            | Message::ToggleFilters
    )
}

fn acts_on_the_page(message: &Message) -> bool {
    matches!(
        message,
        Message::Previous
            | Message::Next
            | Message::Vote(_)
            | Message::Played
            | Message::Open(_)
            | Message::FocusSearch
            | Message::Showcase(
                showcase::Message::NextScreenshot
                    | showcase::Message::PreviousScreenshot
                    | showcase::Message::ToggleAbout
                    | showcase::Message::ToggleTags
                    | showcase::Message::ToggleReviews
                    | showcase::Message::ScrollUp
                    | showcase::Message::ScrollDown
            )
    )
}

// A field of the filters keeps Esc for itself, so Esc is heard here whatever took it.
fn escape_message(event: Event, _: event::Status, _: iced::window::Id) -> Option<Message> {
    matches!(
        event,
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        })
    )
    .then_some(Message::CloseFilters)
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("f" | "F") => Some(Message::ToggleFilters),
            Key::Character("z" | "Z") => Some(Message::Undo),
            _ => None,
        };
    }

    match key {
        Key::Named(Named::Escape) => Some(Message::Showcase(showcase::Message::Close)),
        Key::Named(Named::ArrowUp) => Some(Message::Previous),
        Key::Named(Named::ArrowDown) => Some(Message::Next),
        Key::Character(character) if character.as_str() == "/" => Some(Message::FocusSearch),
        Key::Named(Named::ArrowRight) => Some(Message::Showcase(showcase::Message::NextScreenshot)),
        Key::Named(Named::ArrowLeft) => {
            Some(Message::Showcase(showcase::Message::PreviousScreenshot))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("f") => {
            Some(Message::ToggleFilters)
        }
        Key::Character(character) if character.eq_ignore_ascii_case("d") => {
            Some(Message::Showcase(showcase::Message::ToggleAbout))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("t") => {
            Some(Message::Showcase(showcase::Message::ToggleTags))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("r") => {
            Some(Message::Showcase(showcase::Message::ToggleReviews))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("k") => {
            Some(Message::Showcase(showcase::Message::ScrollUp))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("j") => {
            Some(Message::Showcase(showcase::Message::ScrollDown))
        }
        Key::Character(character) => game::pick_key(&character)
            .map(|pick| match pick {
                Pick::Played => Message::Played,
                pick => Message::Vote(pick),
            })
            .or_else(|| game::link_key(&character).map(Message::Open)),
        Key::Named(_) | Key::Unidentified => None,
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::{Discount, Price};
    use iced::keyboard::{Location, Modifiers, key};

    use super::*;

    const HIDE_OWNED: Shown = Shown {
        hide_owned: true,
        passing: None,
    };

    fn passing_only(passing: &BTreeSet<AppId>) -> Shown<'_> {
        Shown {
            hide_owned: false,
            passing: Some(passing),
        }
    }

    fn set(numbers: &[u32]) -> BTreeSet<AppId> {
        numbers.iter().copied().map(AppId).collect()
    }

    fn votes_panel(filters: &FilterOptions) -> Panel {
        Panel::new(
            vec!["Roguelike".to_string()],
            &FilterOptions::default(),
            "€".to_string(),
            &search_options(filters),
        )
        .without_bets()
    }

    fn voted(id: u32, name: &str, kind: VoteKind, date: &str, bet: bool) -> VotedGame {
        VotedGame {
            id: AppId(id),
            name: name.to_string(),
            vote: Vote {
                bet,
                ..Vote::new(kind, date)
            },
            price: None,
            reviews: None,
            store_url: String::new(),
            owned: false,
        }
    }

    fn priced(mut game: VotedGame, cents: u32, percent: Option<u8>) -> VotedGame {
        game.price = Some(Price {
            original_cents: cents,
            discount: percent.map(|percent| Discount {
                percent,
                final_cents: cents * u32::from(100 - percent) / 100,
                end: None,
            }),
        });
        game
    }

    fn games() -> BTreeMap<AppId, VotedGame> {
        by_id(vec![
            priced(
                voted(1, "Subnautica", VoteKind::WouldTry, "2026-10-01", false),
                2999,
                Some(75),
            ),
            priced(
                voted(2, "Satisfactory", VoteKind::WouldTry, "2026-10-12", true),
                3899,
                None,
            ),
            priced(
                voted(3, "GRIME", VoteKind::Later, "2026-10-20", false),
                2099,
                Some(60),
            ),
            voted(4, "Skyrim", VoteKind::WouldTry, "2026-09-01", false),
            voted(5, "Northgard", VoteKind::NotForMe, "2026-10-15", true),
        ])
    }

    fn ids(numbers: &[u32]) -> Vec<AppId> {
        numbers.iter().copied().map(AppId).collect()
    }

    #[test]
    fn shows_the_votes_of_one_kind_newest_first() {
        let rows = rows(
            &games(),
            Filter::WouldTry,
            Order::Newest,
            Shown::default(),
            "",
            None,
        );

        assert_eq!(rows, ids(&[2, 1, 4]));
    }

    #[test]
    fn a_game_voted_into_another_kind_leaves_the_filter() {
        let mut games = games();
        games.get_mut(&AppId(2)).unwrap().vote.kind = VoteKind::NotForMe;

        let rows = rows(
            &games,
            Filter::WouldTry,
            Order::Newest,
            Shown::default(),
            "",
            None,
        );

        assert_eq!(rows, ids(&[1, 4]));
    }

    #[test]
    fn the_selection_stays_on_its_game_when_the_rows_change() {
        assert_eq!(kept_selection(&ids(&[1, 4]), Some(AppId(4)), 2), 1);
    }

    #[test]
    fn a_selected_game_that_left_the_list_gives_its_place_to_the_next() {
        assert_eq!(kept_selection(&ids(&[2, 4]), Some(AppId(1)), 1), 1);
        assert_eq!(kept_selection(&ids(&[2, 1]), Some(AppId(4)), 2), 1);
        assert_eq!(kept_selection(&[], Some(AppId(4)), 2), 0);
        assert_eq!(kept_selection(&ids(&[2, 1]), None, 0), 0);
    }

    #[test]
    fn sorts_by_price_with_the_games_out_of_the_store_last() {
        let rows = rows(
            &games(),
            Filter::All,
            Order::Price,
            Shown::default(),
            "",
            None,
        );

        assert_eq!(rows, ids(&[1, 3, 2, 5, 4]));
    }

    #[test]
    fn sorts_by_discount_then_by_price() {
        let rows = rows(
            &games(),
            Filter::All,
            Order::Discount,
            Shown::default(),
            "",
            None,
        );

        assert_eq!(rows, ids(&[1, 3, 2, 5, 4]));
    }

    #[test]
    fn the_bets_filter_keeps_only_the_bets() {
        let rows = rows(
            &games(),
            Filter::Bets,
            Order::Newest,
            Shown::default(),
            "",
            None,
        );

        assert_eq!(rows, ids(&[5, 2]));
    }

    #[test]
    fn the_search_looks_among_all_the_votes() {
        let rows = rows(
            &games(),
            Filter::NotMyGenre,
            Order::Newest,
            Shown::default(),
            " sat ",
            None,
        );

        assert_eq!(rows, ids(&[2]));
    }

    #[test]
    fn a_later_has_no_filter_and_shows_only_in_all() {
        assert_eq!(
            Filter::of(&Vote::new(VoteKind::Later, "2026-10-20")),
            Filter::All
        );
        assert!(FILTERS.as_flattened().iter().all(|filter| {
            !rows(&games(), *filter, Order::Newest, Shown::default(), "", None).contains(&AppId(3))
        }));
        assert!(
            rows(
                &games(),
                Filter::All,
                Order::Newest,
                Shown::default(),
                "",
                None
            )
            .contains(&AppId(3))
        );
    }

    #[test]
    fn counts_the_votes_of_each_filter() {
        let games = games();

        assert_eq!(count(&games, Filter::WouldTry, Shown::default()), 3);
        assert_eq!(count(&games, Filter::NotMyGenre, Shown::default()), 1);
        assert_eq!(count(&games, Filter::Bets, Shown::default()), 2);
        assert_eq!(count(&games, Filter::All, Shown::default()), 5);
    }

    #[test]
    fn not_this_game_and_not_my_genre_have_a_chip_each() {
        let mut games = games();
        let mut not_this_one = voted(6, "Firewatch", VoteKind::NotForMe, "2026-11-13", false);
        not_this_one.vote.genre_ok = true;
        games.insert(not_this_one.id, not_this_one);

        let shown = |filter| rows(&games, filter, Order::Newest, Shown::default(), "", None);
        assert_eq!(shown(Filter::NotThisGame), ids(&[6]));
        assert_eq!(shown(Filter::NotMyGenre), ids(&[5]));
        assert_eq!(filter_label(Filter::NotThisGame), "Not this game");
        assert_eq!(filter_label(Filter::NotMyGenre), "Not my genre");
    }

    #[test]
    fn a_vote_opens_on_its_own_chip() {
        let not_this_one = Vote {
            genre_ok: true,
            ..Vote::new(VoteKind::NotForMe, "2026-11-13")
        };

        assert_eq!(
            Filter::of(&Vote::new(VoteKind::WouldTry, "2026-11-13")),
            Filter::WouldTry
        );
        assert_eq!(Filter::of(&not_this_one), Filter::NotThisGame);
        assert_eq!(
            Filter::of(&Vote::new(VoteKind::NotForMe, "2026-11-13")),
            Filter::NotMyGenre
        );
    }

    #[test]
    fn a_click_on_the_chosen_chip_shows_every_vote() {
        assert_eq!(chosen_filter(Filter::Bets, Filter::Bets), Filter::All);
        assert_eq!(
            chosen_filter(Filter::Bets, Filter::WouldTry),
            Filter::WouldTry
        );
        assert_eq!(chosen_filter(Filter::All, Filter::Bets), Filter::Bets);
    }

    #[test]
    fn the_bets_summary_compares_with_the_votes_since_the_first_bet() {
        let mut games = games();
        games.extend(by_id(vec![voted(
            6,
            "Tunic",
            VoteKind::NotForMe,
            "2026-10-25",
            false,
        )]));

        assert_eq!(bets_summary(&games), Some((1, 2, Some(0.0))));
        assert_eq!(bets_summary(&BTreeMap::new()), None);
    }

    #[test]
    fn the_bets_summary_leaves_out_the_laters() {
        let mut games = games();
        games.extend(by_id(vec![voted(
            7,
            "Tchia",
            VoteKind::Later,
            "2026-10-26",
            true,
        )]));

        assert_eq!(bets_summary(&games), Some((1, 2, None)));
    }

    #[test]
    fn the_bets_summary_leaves_out_the_votes_from_a_search() {
        let mut games = games();
        let mut found = voted(8, "Hades", VoteKind::WouldTry, "2026-10-27", false);
        found.vote.searched = true;
        games.extend(by_id(vec![found]));

        assert_eq!(bets_summary(&games), Some((1, 2, None)));
    }

    #[test]
    fn says_why_the_list_is_empty() {
        let not_my_genre = Filter::NotMyGenre;

        assert_eq!(empty_text(true, not_my_genre, false, false, ""), NO_VOTES);
        assert_eq!(
            empty_text(false, not_my_genre, false, false, ""),
            "No Not my genre votes"
        );
        assert_eq!(
            empty_text(false, Filter::WouldTry, false, false, ""),
            "No Would try votes"
        );
        assert_eq!(
            empty_text(false, Filter::Bets, false, false, ""),
            "No votes on Bets"
        );
        assert_eq!(
            empty_text(false, not_my_genre, false, false, " hades "),
            "No votes match \"hades\""
        );
    }

    #[test]
    fn sorts_by_best_match_with_the_games_without_a_score_last() {
        let matches = HashMap::from([(AppId(4), 2.0), (AppId(1), 1.5), (AppId(2), 1.5)]);

        let rows = rows(
            &games(),
            Filter::All,
            Order::BestMatch,
            Shown::default(),
            "",
            Some(&matches),
        );

        assert_eq!(rows, ids(&[4, 2, 1, 3, 5]));
    }

    #[test]
    fn the_filters_keep_only_the_passing_games_of_the_chip() {
        let games = games();
        let passing = set(&[1, 3, 5]);
        let shown = passing_only(&passing);

        assert_eq!(
            rows(&games, Filter::WouldTry, Order::Newest, shown, "", None),
            ids(&[1])
        );
        assert_eq!(
            rows(&games, Filter::Bets, Order::Newest, shown, "", None),
            ids(&[5])
        );
        assert_eq!(count(&games, Filter::WouldTry, shown), 1);
        assert_eq!(count(&games, Filter::All, shown), 3);
    }

    #[test]
    fn the_filters_keep_the_order() {
        let passing = set(&[1, 2, 3]);

        assert_eq!(
            rows(
                &games(),
                Filter::All,
                Order::Price,
                passing_only(&passing),
                "",
                None
            ),
            ids(&[1, 3, 2])
        );
    }

    #[test]
    fn the_search_ignores_the_filters() {
        let passing = set(&[1]);

        let rows = rows(
            &games(),
            Filter::WouldTry,
            Order::Newest,
            passing_only(&passing),
            "north",
            None,
        );

        assert_eq!(rows, ids(&[5]));
    }

    #[test]
    fn no_chosen_filter_lets_every_game_through() {
        let shown = Shown::default();

        assert_eq!(count(&games(), Filter::All, shown), 5);
    }

    #[test]
    fn says_when_no_vote_matches_the_filters() {
        assert_eq!(
            empty_text(false, Filter::WouldTry, true, false, ""),
            "No Would try votes match the filters"
        );
        assert_eq!(
            empty_text(false, Filter::NotMyGenre, true, false, ""),
            "No Not my genre votes match the filters"
        );
        assert_eq!(
            empty_text(false, Filter::Bets, true, false, ""),
            "No Bets match the filters"
        );
        assert_eq!(
            empty_text(false, Filter::All, true, false, ""),
            "No votes match the filters"
        );
        assert_eq!(
            empty_text(false, Filter::All, true, false, "hades"),
            "No votes match \"hades\""
        );
    }

    #[test]
    fn the_panel_of_votes_has_no_bets_only_and_starts_from_no_limit() {
        let mut panel = votes_panel(&FilterOptions::default());
        let none = search_options(&FilterOptions::default());

        panel.update(&none, filters::Message::OnlyBets(true));
        panel.update(&none, filters::Message::OnSale(true));
        let Action::Search(changed) = panel.update(&none, filters::Message::Done) else {
            panic!("expected new filters");
        };

        assert!(!changed.only_bets);
        assert_eq!(
            changed.filters,
            FilterOptions {
                only_discounted: true,
                ..FilterOptions::default()
            }
        );
        assert_eq!(changed.filters.without_early_access, None);
    }

    #[test]
    fn cancel_leaves_the_filters_as_they_were() {
        let filters = FilterOptions {
            max_price_cents: Some(2000),
            ..FilterOptions::default()
        };
        let options = search_options(&filters);
        let mut panel = votes_panel(&filters);

        panel.update(&options, filters::Message::OnSale(true));
        panel.update(&options, filters::Message::Reset);
        assert_eq!(
            panel.update(&options, filters::Message::Cancel),
            Action::Close
        );

        panel.show(&options);
        assert_eq!(
            panel.update(&options, filters::Message::Done),
            Action::Close
        );
    }

    #[test]
    fn the_summary_of_the_filters_shows_only_with_a_filter() {
        let currency = Currency::italian();
        let filters = FilterOptions {
            only_discounted: true,
            max_price_cents: Some(2000),
            min_positive_percent: Some(85),
            tags: vec!["Roguelike".to_string()],
            ..FilterOptions::default()
        };

        assert_eq!(filters_summary(&FilterOptions::default(), &currency), None);
        assert_eq!(
            filters_summary(&filters, &currency).as_deref(),
            Some("On sale · ≤ 20,00€ · ≥ 85% positive · Tags: Roguelike")
        );
    }

    fn with_owned() -> BTreeMap<AppId, VotedGame> {
        let mut games = games();
        for id in [1, 4, 5] {
            games.get_mut(&AppId(id)).unwrap().owned = true;
        }
        games
    }

    #[test]
    fn hide_owned_takes_the_owned_games_out_of_every_chip() {
        let games = with_owned();
        let would_try = Filter::WouldTry;

        assert_eq!(
            rows(&games, would_try, Order::Newest, HIDE_OWNED, "", None),
            ids(&[2])
        );
        assert_eq!(
            rows(&games, Filter::Bets, Order::Newest, HIDE_OWNED, "", None),
            ids(&[2])
        );
        assert_eq!(
            rows(&games, Filter::All, Order::Newest, HIDE_OWNED, "", None),
            ids(&[3, 2])
        );
        assert_eq!(count(&games, would_try, HIDE_OWNED), 1);
        assert_eq!(count(&games, Filter::NotMyGenre, HIDE_OWNED), 0);
        assert_eq!(count(&games, Filter::Bets, HIDE_OWNED), 1);
        assert_eq!(count(&games, Filter::All, HIDE_OWNED), 2);
    }

    #[test]
    fn showing_the_owned_games_again_puts_them_back() {
        let games = with_owned();

        assert_eq!(count(&games, Filter::All, Shown::default()), 5);
        assert_eq!(
            rows(
                &games,
                Filter::WouldTry,
                Order::Newest,
                Shown::default(),
                "",
                None
            ),
            ids(&[2, 1, 4])
        );
    }

    #[test]
    fn the_search_finds_an_owned_game_with_hide_owned_on() {
        let rows = rows(
            &with_owned(),
            Filter::WouldTry,
            Order::Newest,
            HIDE_OWNED,
            "sky",
            None,
        );

        assert_eq!(rows, ids(&[4]));
    }

    #[test]
    fn hide_owned_and_the_filters_work_together() {
        let games = with_owned();
        let passing = set(&[1, 3]);
        let both = Shown {
            hide_owned: true,
            passing: Some(&passing),
        };

        // Subnautica passes the filters but is owned, GRIME passes and is not owned.
        assert_eq!(
            rows(&games, Filter::All, Order::Newest, both, "", None),
            ids(&[3])
        );
        assert_eq!(count(&games, Filter::All, both), 1);
    }

    #[test]
    fn says_when_you_own_all_the_games_of_a_chip() {
        let would_try = Filter::WouldTry;

        assert_eq!(
            empty_text(false, would_try, false, true, ""),
            "You own all your Would try games"
        );
        assert_eq!(
            empty_text(false, Filter::NotMyGenre, false, true, ""),
            "You own all your Not my genre games"
        );
        assert_eq!(
            empty_text(false, Filter::Bets, false, true, ""),
            "You own all your Bets"
        );
        assert_eq!(
            empty_text(false, Filter::All, false, true, ""),
            "You own all your voted games"
        );
        assert_eq!(
            empty_text(false, would_try, true, true, ""),
            "No Would try votes match the filters"
        );
    }

    #[test]
    fn scrolls_only_to_bring_the_row_into_view() {
        assert_eq!(scroll_target(2, 0.0, 320.0), None);
        assert_eq!(scroll_target(2, 100.0, 320.0), Some(64.0));
        assert_eq!(scroll_target(12, 0.0, 320.0), Some(96.0));
    }

    #[test]
    fn moves_in_the_list_with_the_arrows_up_and_down() {
        assert!(matches!(
            key_message(pressed(Key::Named(Named::ArrowUp))),
            Some(Message::Previous)
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(Named::ArrowDown))),
            Some(Message::Next)
        ));
    }

    #[test]
    fn votes_with_the_numbers_and_opens_the_links_with_q_w_e() {
        assert!(matches!(
            key_message(pressed(Key::Character("3".into()))),
            Some(Message::Vote(Pick::Vote(VoteKind::NotForMe)))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("2".into()))),
            Some(Message::Vote(Pick::NotThisOne))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("4".into()))),
            Some(Message::Played)
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("w".into()))),
            Some(Message::Open(Link::Eneba))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("E".into()))),
            Some(Message::Open(Link::Hltb))
        ));
        for key in ["5", "6", "7"] {
            assert!(key_message(pressed(Key::Character(key.into()))).is_none());
        }
    }

    #[test]
    fn ctrl_z_undoes_the_last_played_elsewhere() {
        let event = keyboard::Event::KeyPressed {
            key: Key::Character("z".into()),
            modified_key: Key::Character("z".into()),
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers: Modifiers::CTRL,
            text: None,
            repeat: false,
        };

        assert!(matches!(key_message(event), Some(Message::Undo)));
    }

    #[test]
    fn closes_the_zoom_and_the_description_with_escape() {
        assert!(matches!(
            key_message(pressed(Key::Named(Named::Escape))),
            Some(Message::Showcase(showcase::Message::Close))
        ));
    }

    #[test]
    fn scrolls_the_description_with_j_and_k() {
        assert!(matches!(
            key_message(pressed(Key::Character("k".into()))),
            Some(Message::Showcase(showcase::Message::ScrollUp))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("j".into()))),
            Some(Message::Showcase(showcase::Message::ScrollDown))
        ));
    }

    #[test]
    fn opens_the_description_with_d() {
        for key in ["d", "D"] {
            assert!(matches!(
                key_message(pressed(Key::Character(key.into()))),
                Some(Message::Showcase(showcase::Message::ToggleAbout))
            ));
        }
    }

    #[test]
    fn opens_the_tags_with_t() {
        for key in ["t", "T"] {
            assert!(matches!(
                key_message(pressed(Key::Character(key.into()))),
                Some(Message::Showcase(showcase::Message::ToggleTags))
            ));
        }
    }

    #[test]
    fn opens_the_reviews_with_r() {
        for key in ["r", "R"] {
            assert!(matches!(
                key_message(pressed(Key::Character(key.into()))),
                Some(Message::Showcase(showcase::Message::ToggleReviews))
            ));
        }
    }

    #[test]
    fn the_arrows_scroll_an_open_popup() {
        assert!(matches!(
            scrolled_by_popup(Message::Previous, true),
            Message::Showcase(showcase::Message::ScrollUp)
        ));
        assert!(matches!(
            scrolled_by_popup(Message::Next, true),
            Message::Showcase(showcase::Message::ScrollDown)
        ));
        assert!(matches!(
            scrolled_by_popup(Message::Next, false),
            Message::Next
        ));
    }

    #[test]
    fn an_open_popup_stops_the_votes_and_the_links() {
        assert!(stopped_by_popup(&Message::Vote(Pick::Vote(
            VoteKind::WouldTry
        ))));
        assert!(stopped_by_popup(&Message::Open(Link::Hltb)));
        assert!(stopped_by_popup(&Message::FocusSearch));
        assert!(stopped_by_popup(&Message::ToggleFilters));
        assert!(!stopped_by_popup(&Message::Showcase(
            showcase::Message::ToggleTags
        )));
    }

    #[test]
    fn opens_and_closes_the_filters_with_f() {
        for key in ["f", "F"] {
            assert!(matches!(
                key_message(pressed(Key::Character(key.into()))),
                Some(Message::ToggleFilters)
            ));
        }
    }

    #[test]
    fn esc_closes_the_filters_even_from_inside_a_field() {
        let escape = Event::Keyboard(pressed(Key::Named(Named::Escape)));
        let other = Event::Keyboard(pressed(Key::Character("1".into())));

        assert!(matches!(
            escape_message(escape, event::Status::Captured, iced::window::Id::unique()),
            Some(Message::CloseFilters)
        ));
        assert!(
            escape_message(other, event::Status::Ignored, iced::window::Id::unique()).is_none()
        );
    }

    #[test]
    fn the_open_filters_stop_the_keys_of_the_page() {
        let number = key_message(pressed(Key::Character("1".into()))).unwrap();

        assert!(acts_on_the_page(&number));
        for message in [
            Message::Next,
            Message::Previous,
            Message::Played,
            Message::Open(Link::Steam),
            Message::FocusSearch,
            Message::Showcase(showcase::Message::ToggleReviews),
            Message::Showcase(showcase::Message::NextScreenshot),
        ] {
            assert!(acts_on_the_page(&message), "{message:?}");
        }
        assert!(!acts_on_the_page(&Message::CloseFilters));
        assert!(!acts_on_the_page(&Message::ToggleFilters));
        assert!(!acts_on_the_page(&Message::Showcase(
            showcase::Message::Close
        )));
    }

    #[test]
    fn ctrl_f_opens_the_filters_as_in_discover_and_slash_searches() {
        assert!(matches!(
            key_message(pressed_with(Key::Character("f".into()), Modifiers::CTRL)),
            Some(Message::ToggleFilters)
        ));
        assert!(matches!(
            key_message(pressed_with(Key::Character("/".into()), Modifiers::SHIFT)),
            Some(Message::FocusSearch)
        ));
    }

    #[test]
    fn the_vote_shows_beside_the_name_only_in_all_and_bets() {
        let would_try = Vote::new(VoteKind::WouldTry, "2026-10-01");
        let not_this_one = Vote {
            genre_ok: true,
            ..Vote::new(VoteKind::NotForMe, "2026-10-01")
        };
        let later = Vote::new(VoteKind::Later, "2026-10-01");

        for filter in [Filter::All, Filter::Bets] {
            assert_eq!(vote_sign(filter, &would_try), Some(Sign::Up));
            assert_eq!(vote_sign(filter, &not_this_one), Some(Sign::Down));
            assert_eq!(vote_sign(filter, &later), Some(Sign::Later));
        }
        for filter in [Filter::WouldTry, Filter::NotThisGame, Filter::NotMyGenre] {
            assert_eq!(vote_sign(filter, &would_try), None);
            assert_eq!(vote_sign(filter, &later), None);
        }
        assert_eq!(Sign::Later.thumb(), None);
    }

    #[test]
    fn a_game_without_a_price_shows_a_dash() {
        let currency = Currency::italian();
        let game = priced(
            voted(1, "Subnautica", VoteKind::WouldTry, "2026-10-01", false),
            2999,
            None,
        );

        assert_eq!(price_text(None, &currency), "—");
        assert_eq!(
            price_text(game.price, &currency),
            format::price(&game.price.unwrap(), &currency)
        );
    }

    #[test]
    fn the_chips_and_the_orders_go_to_the_file_and_back() {
        for &filter in FILTERS.as_flattened().iter().chain([&Filter::All]) {
            assert_eq!(Filter::from(VotesShow::from(filter)), filter);
        }
        for order in ORDERS {
            assert_eq!(Order::from(VotesOrder::from(order)), order);
        }
    }

    #[test]
    fn votes_open_on_the_saved_chip() {
        let saved = VotesSettings {
            show: VotesShow::Bets,
            ..VotesSettings::default()
        };

        assert_eq!(opening_chip(&saved, None), (Filter::Bets, VotesShow::Bets));
        assert_eq!(
            opening_chip(&VotesSettings::default(), None),
            (Filter::WouldTry, VotesShow::WouldTry)
        );
    }

    #[test]
    fn a_chip_chosen_from_outside_leaves_the_saved_one() {
        let saved = VotesSettings {
            show: VotesShow::NotMyGenre,
            ..VotesSettings::default()
        };

        assert_eq!(
            opening_chip(&saved, Some(Filter::WouldTry)),
            (Filter::WouldTry, VotesShow::NotMyGenre)
        );
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
