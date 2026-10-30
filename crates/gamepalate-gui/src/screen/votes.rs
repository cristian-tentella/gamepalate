use std::{cmp::Reverse, collections::BTreeMap, fmt, sync::Arc, time::Duration};

use gamepalate_app::{AppId, Gamepalate, Vote, VoteKind, VotedGame};
use iced::{
    Center, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    never, padding,
    widget::{
        Id, button, column, container, operation, pick_list, responsive, rich_text, row, rule,
        scrollable,
        scrollable::{AbsoluteOffset, Viewport},
        span, stack, text,
        text::Wrapping,
        text_input,
    },
};

use super::{
    game::{self, Link, VOTE_HEIGHT, VOTES},
    showcase::{self, Showcase},
};
use crate::{font, format, style};

const FILTERS: [Filter; 6] = [
    Filter::Kind(VoteKind::WouldTry),
    Filter::Kind(VoteKind::Maybe),
    Filter::Kind(VoteKind::NotForMe),
    Filter::Kind(VoteKind::Known),
    Filter::Bets,
    Filter::All,
];
const ORDERS: [Order; 3] = [Order::Newest, Order::Price, Order::Discount];
const LIST: Id = Id::new("votes-list");
const SEARCH: Id = Id::new("votes-search");
const LIST_WIDTH: f32 = 340.0;
const ROW_HEIGHT: f32 = 32.0;
const HEADER_WIDTH: f32 = 230.0;
const HEADER_HEIGHT: f32 = HEADER_WIDTH * 215.0 / 460.0;
const MAX_SHOT_WIDTH: f32 = 820.0;
const SCROLLBAR_GAP: f32 = 12.0;
const TOAST_DURATION: Duration = Duration::from_secs(4);
const NO_VOTES: &str = "No votes yet. Vote games in Discover with keys 1-4.";

pub struct Votes {
    gamepalate: Arc<Gamepalate>,
    games: BTreeMap<AppId, VotedGame>,
    filter: Filter,
    order: Order,
    search: String,
    rows: Vec<AppId>,
    selected: usize,
    viewport: Option<(f32, f32)>,
    toast: Option<Toast>,
    toasts_shown: u64,
    showcase: Showcase,
}

struct Toast {
    id: u64,
    text: String,
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
}

#[derive(Clone, Debug)]
pub enum Message {
    Filter(Filter),
    Order(Order),
    Search(String),
    FocusSearch,
    Select(usize),
    Previous,
    Next,
    Scrolled(Viewport),
    Vote(VoteKind),
    VoteSaved(Result<(), String>),
    Open(Link),
    HideToast(u64),
    Showcase(showcase::Message),
}

impl Votes {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Votes, Task<Message>) {
        let mut votes = Votes {
            gamepalate: Arc::clone(gamepalate),
            games: by_id(gamepalate.voted_games()),
            filter: Filter::Kind(VoteKind::WouldTry),
            order: Order::Newest,
            search: String::new(),
            rows: Vec::new(),
            selected: 0,
            viewport: None,
            toast: None,
            toasts_shown: 0,
            showcase: Showcase::new(gamepalate),
        };
        votes.rows = rows(&votes.games, votes.filter, votes.order, &votes.search);
        let task = votes.load();

        (votes, task)
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
            Message::Order(order) => {
                self.order = order;
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
            Message::Vote(kind) => {
                if let Some(game) = self.current() {
                    let text = format::voted(&game.name, kind);
                    let id = game.id;
                    let gamepalate = Arc::clone(&self.gamepalate);
                    let save = Task::perform(
                        async move {
                            gamepalate
                                .vote(id, kind, false)
                                .map_err(|error| error.to_string())
                        },
                        Message::VoteSaved,
                    );
                    return Task::batch([save, self.show_toast(text), self.next()]);
                }
            }
            Message::VoteSaved(Ok(())) => self.games = by_id(self.gamepalate.voted_games()),
            Message::VoteSaved(Err(error)) => return self.show_toast(error),
            Message::Open(link) => {
                if let Some(game) = self.current() {
                    game::open(link, &game.name, &game.store_url);
                }
            }
            Message::HideToast(id) => {
                if self.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.toast = None;
                }
            }
            Message::Showcase(message) => {
                return self.showcase.update(message).map(Message::Showcase);
            }
        }

        Task::none()
    }

    pub fn subscription() -> Subscription<Message> {
        keyboard::listen().filter_map(key_message)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let toast = self.toast.as_ref().map(|toast| {
            container(
                container(text(&toast.text))
                    .padding([8, 14])
                    .style(style::toast),
            )
            .center_x(Fill)
            .align_bottom(Fill)
            .padding(16)
        });
        let card = container(self.card())
            .padding(16)
            .width(Fill)
            .height(Fill)
            .clip(true);
        let votes = container(container(self.vote_row()).height(VOTE_HEIGHT))
            .padding([10, 20])
            .width(Fill);

        row![
            container(self.list()).width(LIST_WIDTH).height(Fill),
            rule::vertical(1).style(rule::weak),
            column![stack![card].push(toast), style::separator(), votes],
        ]
        .into()
    }

    fn list(&self) -> Element<'_, Message> {
        let searching = !self.search.trim().is_empty();
        let filters = row(FILTERS.map(|filter| {
            button(
                text!("{} {}", filter_label(filter), count(&self.games, filter))
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
                .padding([4, 8]),
            text_input("Search by name (Ctrl+F)", &self.search)
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

        let body: Element<'_, Message> = if self.rows.is_empty() {
            container(
                text(empty_text(self.games.is_empty(), self.filter, &self.search))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
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
                        .style(style::discount)
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

    fn card(&self) -> Element<'_, Message> {
        match self.current() {
            Some(game) => responsive(move |size| self.details(game, size.width)).into(),
            None => {
                let note = if self.games.is_empty() { NO_VOTES } else { "" };
                container(text(note).style(style::muted))
                    .center(Fill)
                    .into()
            }
        }
    }

    fn details<'a>(&'a self, game: &'a VotedGame, width: f32) -> Element<'a, Message> {
        let shot_width = (width - SCROLLBAR_GAP).min(MAX_SHOT_WIDTH);
        let today = jiff::Zoned::now().date();
        let price = match &game.price {
            Some(price) => game::price_row(price, game.vote.bet, self.gamepalate.currency()),
            None => row![
                text("Not on sale on Steam")
                    .size(font::INFO_SIZE)
                    .style(style::muted)
            ]
            .push(game.vote.bet.then(game::bet_label))
            .spacing(12)
            .align_y(Center)
            .into(),
        };
        let voted = rich_text![
            span("Voted "),
            span(format::vote(game.vote.kind)).font(font::SEMIBOLD),
            span(format!(" · {}", format::voted_day(&game.vote.date, today))),
        ]
        .size(font::BAR_SIZE)
        .style(style::muted)
        .on_link_click(never);
        let info = column![
            text(&game.name).size(font::TITLE_SIZE).font(font::SEMIBOLD),
            voted,
            price,
        ]
        .push(game.reviews.as_ref().map(game::reviews_line))
        .spacing(10)
        .width(Fill);
        let top = row![
            self.showcase
                .header(game.id, HEADER_WIDTH, HEADER_HEIGHT)
                .map(Message::Showcase),
            info,
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
                .facts(game.id)
                .map(|facts| facts.map(Message::Showcase)),
        )
        .spacing(18)
        .max_width(MAX_SHOT_WIDTH);

        scrollable(
            container(content)
                .center_x(Fill)
                .padding(padding::right(SCROLLBAR_GAP)),
        )
        .height(Fill)
        .into()
    }

    fn vote_row(&self) -> Element<'_, Message> {
        let game = self.current();
        let votes = row(VOTES.map(|(key, kind)| {
            game::vote_button(
                key,
                kind,
                true,
                game.is_some().then_some(Message::Vote(kind)),
                game.is_some_and(|game| game.vote.kind == kind),
            )
        }))
        .spacing(8)
        .width(Fill)
        .align_y(Center);

        row![
            votes,
            game::links(|link| game.is_some().then_some(Message::Open(link))),
        ]
        .spacing(8)
        .height(Fill)
        .align_y(Center)
        .into()
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
        self.rows = rows(&self.games, self.filter, self.order, &self.search);
        self.selected = selected
            .and_then(|id| self.rows.iter().position(|&row| row == id))
            .unwrap_or(0);

        self.reveal()
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

        self.showcase.load(upcoming, nearby).map(Message::Showcase)
    }

    fn show_toast(&mut self, text: String) -> Task<Message> {
        self.toasts_shown += 1;
        let id = self.toasts_shown;
        self.toast = Some(Toast { id, text });

        Task::perform(tokio::time::sleep(TOAST_DURATION), move |_| {
            Message::HideToast(id)
        })
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
        })
    }
}

fn by_id(games: Vec<VotedGame>) -> BTreeMap<AppId, VotedGame> {
    games.into_iter().map(|game| (game.id, game)).collect()
}

fn filter_label(filter: Filter) -> &'static str {
    match filter {
        Filter::Kind(VoteKind::Known) => "Known",
        Filter::Kind(kind) => format::vote(kind),
        Filter::Bets => "Bets",
        Filter::All => "All",
    }
}

fn count(games: &BTreeMap<AppId, VotedGame>, filter: Filter) -> usize {
    games
        .values()
        .filter(|game| filter.keeps(&game.vote))
        .count()
}

fn rows(
    games: &BTreeMap<AppId, VotedGame>,
    filter: Filter,
    order: Order,
    search: &str,
) -> Vec<AppId> {
    let search = search.trim().to_lowercase();
    let mut rows: Vec<&VotedGame> = games
        .values()
        .filter(|game| {
            if search.is_empty() {
                filter.keeps(&game.vote)
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
    }

    rows.into_iter().map(|game| game.id).collect()
}

fn bets_summary(games: &BTreeMap<AppId, VotedGame>) -> Option<(usize, usize, Option<f64>)> {
    let first = games
        .values()
        .filter(|game| game.vote.bet)
        .map(|game| game.vote.date.as_str())
        .min()?;
    let (bets, others): (Vec<&Vote>, Vec<&Vote>) = games
        .values()
        .map(|game| &game.vote)
        .filter(|vote| vote.bet || vote.date.as_str() >= first)
        .partition(|vote| vote.bet);
    let would_try = |votes: &[&Vote]| {
        votes
            .iter()
            .filter(|vote| vote.kind == VoteKind::WouldTry)
            .count()
    };
    let others_share =
        (!others.is_empty()).then(|| would_try(&others) as f64 / others.len() as f64);

    Some((would_try(&bets), bets.len(), others_share))
}

fn empty_text(no_votes: bool, filter: Filter, search: &str) -> String {
    let search = search.trim();
    if no_votes {
        NO_VOTES.to_string()
    } else if !search.is_empty() {
        format!("No votes match \"{search}\".")
    } else if filter == Filter::Bets {
        "No votes on Bets.".to_string()
    } else {
        format!("No {} votes.", filter_label(filter))
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

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    if modifiers.command() {
        return matches!(key.as_ref(), Key::Character("f" | "F")).then_some(Message::FocusSearch);
    }

    match key {
        Key::Named(Named::ArrowUp) => Some(Message::Previous),
        Key::Named(Named::ArrowDown) => Some(Message::Next),
        Key::Named(Named::ArrowRight) => Some(Message::Showcase(showcase::Message::NextScreenshot)),
        Key::Named(Named::ArrowLeft) => {
            Some(Message::Showcase(showcase::Message::PreviousScreenshot))
        }
        Key::Character(character) => game::vote_key(&character)
            .map(Message::Vote)
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
                kind,
                date: date.to_string(),
                bet,
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
                voted(3, "GRIME", VoteKind::Maybe, "2026-10-20", false),
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
            "",
        );

        assert_eq!(rows, ids(&[2, 1, 4]));
    }

    #[test]
    fn sorts_by_price_with_the_games_out_of_the_store_last() {
        let rows = rows(&games(), Filter::All, Order::Price, "");

        assert_eq!(rows, ids(&[1, 3, 2, 5, 4]));
    }

    #[test]
    fn sorts_by_discount_then_by_price() {
        let rows = rows(&games(), Filter::All, Order::Discount, "");

        assert_eq!(rows, ids(&[1, 3, 2, 5, 4]));
    }

    #[test]
    fn the_bets_filter_keeps_only_the_bets() {
        let rows = rows(&games(), Filter::Bets, Order::Newest, "");

        assert_eq!(rows, ids(&[5, 2]));
    }

    #[test]
    fn the_search_looks_among_all_the_votes() {
        let rows = rows(
            &games(),
            Filter::Kind(VoteKind::Maybe),
            Order::Newest,
            " sat ",
        );

        assert_eq!(rows, ids(&[2]));
    }

    #[test]
    fn counts_the_votes_of_each_filter() {
        let games = games();

        assert_eq!(count(&games, Filter::Kind(VoteKind::WouldTry)), 3);
        assert_eq!(count(&games, Filter::Kind(VoteKind::Known)), 0);
        assert_eq!(count(&games, Filter::Bets), 2);
        assert_eq!(count(&games, Filter::All), 5);
    }

    #[test]
    fn the_bets_summary_compares_with_the_votes_since_the_first_bet() {
        assert_eq!(bets_summary(&games()), Some((1, 2, Some(0.0))));
        assert_eq!(bets_summary(&BTreeMap::new()), None);
    }

    #[test]
    fn says_why_the_list_is_empty() {
        let maybe = Filter::Kind(VoteKind::Maybe);

        assert_eq!(empty_text(true, maybe, ""), NO_VOTES);
        assert_eq!(empty_text(false, maybe, ""), "No Maybe votes.");
        assert_eq!(
            empty_text(false, Filter::Kind(VoteKind::Known), ""),
            "No Known votes."
        );
        assert_eq!(empty_text(false, Filter::Bets, ""), "No votes on Bets.");
        assert_eq!(
            empty_text(false, maybe, " hades "),
            "No votes match \"hades\"."
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
    fn votes_and_opens_the_links_with_the_numbers() {
        assert!(matches!(
            key_message(pressed(Key::Character("3".into()))),
            Some(Message::Vote(VoteKind::NotForMe))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("6".into()))),
            Some(Message::Open(Link::Eneba))
        ));
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
