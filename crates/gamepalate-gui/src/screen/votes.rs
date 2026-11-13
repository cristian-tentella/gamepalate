use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap},
    fmt,
    sync::Arc,
};

use gamepalate_app::{AppId, Gamepalate, Vote, VoteKind, VotedGame};
use iced::{
    Center, Element, Fill, Size, Subscription, Task,
    keyboard::{self, Key, key::Named},
    never, padding,
    widget::{
        Id, button, checkbox, column, container, operation, pick_list, responsive, rich_text, row,
        rule, scrollable,
        scrollable::{AbsoluteOffset, Viewport},
        span, stack, text,
        text::Wrapping,
        text_input,
    },
};

use super::{
    Toasts,
    game::{self, Link, Pick, VOTE_HEIGHT},
    sentence,
    showcase::{self, Showcase},
};
use crate::{font, format, style};

const FILTERS: [Filter; 4] = [
    Filter::Kind(VoteKind::WouldTry),
    Filter::Kind(VoteKind::NotForMe),
    Filter::Bets,
    Filter::All,
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
const ROW_HEIGHT: f32 = 32.0;
const HEADER_WIDTH: f32 = 230.0;
const HEADER_HEIGHT: f32 = HEADER_WIDTH * 215.0 / 460.0;
const TOP_HEIGHT: f32 = 160.0;
const MIN_SHOT_WIDTH: f32 = 480.0;
const CONTENT_SPACING: f32 = 18.0;
const SCROLLBAR_GAP: f32 = 12.0;
const NO_VOTES: &str = "No votes yet. Vote in Discover with keys 1–4.";

pub struct Votes {
    gamepalate: Arc<Gamepalate>,
    games: BTreeMap<AppId, VotedGame>,
    filter: Filter,
    order: Order,
    on_sale: bool,
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
    Kind(VoteKind),
    Bets,
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
    OnSale(bool),
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
    Matched(u64, HashMap<AppId, f64>),
    Open(Link),
    HideToast(u64),
    Showcase(showcase::Message),
}

impl Votes {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Votes, Task<Message>) {
        Votes::filtered(gamepalate, Filter::Kind(VoteKind::WouldTry))
    }

    pub fn filtered(gamepalate: &Arc<Gamepalate>, filter: Filter) -> (Votes, Task<Message>) {
        let mut votes = Votes {
            gamepalate: Arc::clone(gamepalate),
            games: by_id(gamepalate.voted_games()),
            filter,
            order: Order::Newest,
            on_sale: false,
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
        let task = Task::batch([votes.load(), votes.find_matches()]);

        (votes, task)
    }

    pub fn showing(gamepalate: &Arc<Gamepalate>, id: AppId) -> (Votes, Task<Message>) {
        let (mut votes, task) = Votes::new(gamepalate);
        let Some(kind) = votes.games.get(&id).map(|game| game.vote.kind) else {
            return (votes, task);
        };
        votes.filter = Filter::Kind(kind);
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

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let message = scrolled_by_popup(message, self.showcase.is_popup_open());
        if self.showcase.is_open() && stopped_by_popup(&message) {
            return Task::none();
        }
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
            Message::Order(order) => {
                self.order = order;
                return self.restart();
            }
            Message::OnSale(on_sale) => {
                self.on_sale = on_sale;
                return self.refresh();
            }
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
                let text = format::elsewhere_text(&name, true);
                return self.toasts.show(text, !before, Message::HideToast);
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
            Message::Undone(Ok(())) => return self.show_toast("Undone".to_string()),
            Message::Undone(Err(error)) => return self.show_toast(error),
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
        keyboard::listen().filter_map(key_message)
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

        stack![row![
            container(self.list()).width(LIST_WIDTH).height(Fill),
            rule::vertical(1).style(rule::weak),
            column![stack![card].push(toast), style::separator(), votes],
        ]]
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
        let filters = row(FILTERS.map(|filter| {
            button(
                text!(
                    "{} {}",
                    filter_label(filter),
                    count(&self.games, filter, self.on_sale)
                )
                .size(font::NOTE_SIZE),
            )
            .padding([3, 10])
            .style(style::choice(!searching && filter == self.filter))
            .on_press(Message::Filter(filter))
            .into()
        }))
        .spacing(6)
        .wrap()
        .vertical_spacing(6);
        let tools = row![
            pick_list(ORDERS, Some(self.order), Message::Order)
                .text_size(font::NOTE_SIZE)
                .padding([4, 8])
                .style(style::dropdown)
                .menu_style(style::menu),
            checkbox(self.on_sale)
                .label("On sale")
                .text_size(font::NOTE_SIZE)
                .on_toggle(Message::OnSale),
            text_input("Search (Ctrl+F)", &self.search)
                .style(style::field)
                .id(SEARCH)
                .on_input(Message::Search)
                .size(font::NOTE_SIZE)
                .padding([5, 8])
                .width(Fill),
        ]
        .spacing(8)
        .align_y(Center);
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
                    self.on_sale,
                    &self.search,
                )
            };
            container(text(note).size(font::NOTE_SIZE).style(style::muted))
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

        column![
            column![filters, tools]
                .push(summary)
                .spacing(10)
                .padding(12),
            style::separator(),
            body,
        ]
        .into()
    }

    fn list_row<'a>(&self, index: usize, game: &'a VotedGame) -> Element<'a, Message> {
        let currency = self.gamepalate.currency();
        let price = game.price.map(|price| {
            row![text(format::price(&price, currency)).size(font::NOTE_SIZE)]
                .push(price.discount.map(|discount| {
                    text(format::discount(discount.percent))
                        .size(font::NOTE_SIZE)
                        .style(style::accent)
                }))
                .spacing(6)
        });

        button(
            row![
                container(
                    text(&game.name)
                        .size(font::BAR_SIZE)
                        .wrapping(Wrapping::None)
                )
                .width(Fill)
                .clip(true),
            ]
            .push(price)
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
        let shot_width = showcase::screenshot_width(
            size.width - SCROLLBAR_GAP,
            size.height - TOP_HEIGHT - CONTENT_SPACING,
            MIN_SHOT_WIDTH,
            showcase::max_screenshot_width(scale),
        );
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
        let info = column![game::title(&game.name), voted]
            .push(game.reviews.as_ref().map(game::reviews))
            .push(price)
            .spacing(10)
            .width(Fill);
        let top = row![
            game::with_bet(
                self.showcase
                    .header(game.id, HEADER_WIDTH, HEADER_HEIGHT)
                    .map(Message::Showcase),
                game.vote.bet.then(|| self.gamepalate.bet_every()),
            ),
            info,
            // Here the links leave the row of the votes all its width.
            game::header_links(HEADER_HEIGHT, |link| Some(Message::Open(link))),
        ]
        .spacing(20)
        .align_y(Center);

        let content = column![
            top,
            self.showcase
                .media(game.id, shot_width, shot_width * 9.0 / 16.0)
                .map(Message::Showcase),
        ]
        .push(
            self.showcase
                .facts(game.id, &self.showcase.used_tags())
                .map(|facts| facts.map(Message::Showcase)),
        )
        .spacing(CONTENT_SPACING)
        .max_width(shot_width);

        scrollable(
            container(content)
                .center_x(Fill)
                .padding(padding::right(SCROLLBAR_GAP)),
        )
        .height(Fill)
        .into()
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

    fn shown_rows(&self) -> Vec<AppId> {
        rows(
            &self.games,
            self.filter,
            self.order,
            self.on_sale,
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
        self.showcase.follow(self.current().map(|game| game.id));
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
            Filter::Kind(kind) => vote.kind == kind,
            Filter::Bets => vote.bet,
            Filter::All => true,
        }
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
        Filter::Kind(kind) => format::vote(kind),
        Filter::Bets => "Bets",
        Filter::All => "All",
    }
}

fn on_sale(game: &VotedGame) -> bool {
    game.price.is_some_and(|price| price.discount.is_some())
}

fn count(games: &BTreeMap<AppId, VotedGame>, filter: Filter, only_on_sale: bool) -> usize {
    games
        .values()
        .filter(|game| filter.keeps(&game.vote) && (!only_on_sale || on_sale(game)))
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
    only_on_sale: bool,
    search: &str,
    matches: Option<&HashMap<AppId, f64>>,
) -> Vec<AppId> {
    let search = search.trim().to_lowercase();
    let mut rows: Vec<&VotedGame> = games
        .values()
        .filter(|game| {
            if search.is_empty() {
                filter.keeps(&game.vote) && (!only_on_sale || on_sale(game))
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

fn empty_text(no_votes: bool, filter: Filter, only_on_sale: bool, search: &str) -> String {
    let search = search.trim();
    if no_votes {
        NO_VOTES.to_string()
    } else if !search.is_empty() {
        format!("No votes match \"{search}\"")
    } else if only_on_sale {
        match filter {
            Filter::Bets => "No Bets on sale".to_string(),
            Filter::All => "No votes on sale".to_string(),
            Filter::Kind(_) => format!("No {} votes on sale", filter_label(filter)),
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
    )
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("f" | "F") => Some(Message::FocusSearch),
            Key::Character("z" | "Z") => Some(Message::Undo),
            _ => None,
        };
    }

    match key {
        Key::Named(Named::Escape) => Some(Message::Showcase(showcase::Message::Close)),
        Key::Named(Named::ArrowUp) => Some(Message::Previous),
        Key::Named(Named::ArrowDown) => Some(Message::Next),
        Key::Named(Named::ArrowRight) => Some(Message::Showcase(showcase::Message::NextScreenshot)),
        Key::Named(Named::ArrowLeft) => {
            Some(Message::Showcase(showcase::Message::PreviousScreenshot))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("d") => {
            Some(Message::Showcase(showcase::Message::ToggleAbout))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("t") => {
            Some(Message::Showcase(showcase::Message::ToggleTags))
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
            Filter::Kind(VoteKind::WouldTry),
            Order::Newest,
            false,
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
            Filter::Kind(VoteKind::WouldTry),
            Order::Newest,
            false,
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
        let rows = rows(&games(), Filter::All, Order::Price, false, "", None);

        assert_eq!(rows, ids(&[1, 3, 2, 5, 4]));
    }

    #[test]
    fn sorts_by_discount_then_by_price() {
        let rows = rows(&games(), Filter::All, Order::Discount, false, "", None);

        assert_eq!(rows, ids(&[1, 3, 2, 5, 4]));
    }

    #[test]
    fn the_bets_filter_keeps_only_the_bets() {
        let rows = rows(&games(), Filter::Bets, Order::Newest, false, "", None);

        assert_eq!(rows, ids(&[5, 2]));
    }

    #[test]
    fn the_search_looks_among_all_the_votes() {
        let rows = rows(
            &games(),
            Filter::Kind(VoteKind::NotForMe),
            Order::Newest,
            false,
            " sat ",
            None,
        );

        assert_eq!(rows, ids(&[2]));
    }

    #[test]
    fn a_later_has_no_filter_and_shows_only_in_all() {
        assert!(!FILTERS.contains(&Filter::Kind(VoteKind::Later)));
        assert!(
            FILTERS
                .iter()
                .filter(|filter| **filter != Filter::All)
                .all(
                    |filter| !rows(&games(), *filter, Order::Newest, false, "", None)
                        .contains(&AppId(3))
                )
        );
        assert!(rows(&games(), Filter::All, Order::Newest, false, "", None).contains(&AppId(3)));
    }

    #[test]
    fn counts_the_votes_of_each_filter() {
        let games = games();

        assert_eq!(count(&games, Filter::Kind(VoteKind::WouldTry), false), 3);
        assert_eq!(count(&games, Filter::Kind(VoteKind::NotForMe), false), 1);
        assert_eq!(count(&games, Filter::Bets, false), 2);
        assert_eq!(count(&games, Filter::All, false), 5);
    }

    #[test]
    fn a_not_this_one_is_in_the_not_for_me_filter() {
        let mut games = games();
        let mut not_this_one = voted(6, "Firewatch", VoteKind::NotForMe, "2026-11-13", false);
        not_this_one.vote.genre_ok = true;
        games.insert(not_this_one.id, not_this_one);

        assert_eq!(count(&games, Filter::Kind(VoteKind::NotForMe), false), 2);
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
        let not_for_me = Filter::Kind(VoteKind::NotForMe);

        assert_eq!(empty_text(true, not_for_me, false, ""), NO_VOTES);
        assert_eq!(
            empty_text(false, not_for_me, false, ""),
            "No Not for me votes"
        );
        assert_eq!(
            empty_text(false, Filter::Kind(VoteKind::WouldTry), false, ""),
            "No Would try votes"
        );
        assert_eq!(
            empty_text(false, Filter::Bets, false, ""),
            "No votes on Bets"
        );
        assert_eq!(
            empty_text(false, not_for_me, false, " hades "),
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
            false,
            "",
            Some(&matches),
        );

        assert_eq!(rows, ids(&[4, 2, 1, 3, 5]));
    }

    #[test]
    fn on_sale_keeps_only_the_discounted_games() {
        let games = games();

        let rows = rows(
            &games,
            Filter::Kind(VoteKind::WouldTry),
            Order::Newest,
            true,
            "",
            None,
        );

        assert_eq!(rows, ids(&[1]));
        assert_eq!(count(&games, Filter::Kind(VoteKind::WouldTry), true), 1);
        assert_eq!(count(&games, Filter::All, true), 2);
    }

    #[test]
    fn says_when_nothing_is_on_sale() {
        assert_eq!(
            empty_text(false, Filter::Kind(VoteKind::WouldTry), true, ""),
            "No Would try votes on sale"
        );
        assert_eq!(empty_text(false, Filter::Bets, true, ""), "No Bets on sale");
        assert_eq!(empty_text(false, Filter::All, true, ""), "No votes on sale");
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
        assert!(!stopped_by_popup(&Message::Showcase(
            showcase::Message::ToggleTags
        )));
    }

    #[test]
    fn searches_with_ctrl_f() {
        assert!(matches!(
            key_message(pressed_with(Key::Character("f".into()), Modifiers::CTRL)),
            Some(Message::FocusSearch)
        ));
        assert!(key_message(pressed(Key::Character("f".into()))).is_none());
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
