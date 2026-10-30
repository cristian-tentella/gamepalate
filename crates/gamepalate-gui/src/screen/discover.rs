mod images;
mod pages;
mod queue;

use std::{process::Command, sync::Arc, time::Duration};

use gamepalate_app::{
    AppId, GameDetails, Gamepalate, RecommendError, Recommendation, RecommendedGame, SearchOptions,
    VoteKind,
};
use iced::{
    Center, ContentFit, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    never, padding,
    widget::{
        button, column, container, image, progress_bar, responsive, rich_text, row, scrollable,
        space, span, stack, text,
    },
};

use self::{
    images::{Images, Picture},
    pages::{Page, Pages},
    queue::Queue,
};
use crate::{font, format, style};

const VOTES: [(&str, VoteKind); 4] = [
    ("1", VoteKind::WouldTry),
    ("2", VoteKind::Maybe),
    ("3", VoteKind::NotForMe),
    ("4", VoteKind::Known),
];
const COMPACT_WIDTH: f32 = 1100.0;
const COMPACT_VOTE_SIZE: f32 = 14.0;
const VOTE_WIDTH: f32 = 172.0;
const VOTE_HEIGHT: f32 = 36.0;
const TOAST_DURATION: Duration = Duration::from_secs(4);
const FLASH_DURATION: Duration = Duration::from_millis(600);
const CHECK: char = '\u{E73E}';
const METER_LABEL_WIDTH: f32 = 90.0;
const METER_VALUE_WIDTH: f32 = 90.0;
const SIDE_WIDTH: f32 = 360.0;
const HEADER_HEIGHT: f32 = SIDE_WIDTH * 215.0 / 460.0;
const MIN_SHOT_WIDTH: f32 = 480.0;
const MAX_SHOT_WIDTH: f32 = 820.0;
const CARD_SPACING: f32 = 32.0;
const SCROLLBAR_GAP: f32 = 12.0;
const DESCRIPTION_WIDTH: f32 = 600.0;
const DOT_SIZE: f32 = 8.0;
const IMAGE_RADIUS: f32 = 8.0;
const DETAILS_AHEAD: usize = 2;
const PRELOAD_AHEAD: usize = 5;
const KEEP_BEHIND: usize = 5;

pub struct Discover {
    gamepalate: Arc<Gamepalate>,
    options: SearchOptions,
    votes_this_session: usize,
    bet_every: usize,
    why_open: bool,
    toast: Option<Toast>,
    flash: Option<Flash>,
    timers_started: u64,
    images: Images,
    pages: Pages,
    screenshot: usize,
    shown: Option<AppId>,
    state: State,
}

struct Flash {
    id: u64,
    kind: VoteKind,
}

struct Toast {
    id: u64,
    text: String,
}

#[derive(Clone, Debug)]
pub enum Message {
    Recommended(Result<Recommendation, RecommendError>),
    MoreRecommended(Result<Recommendation, RecommendError>),
    Previous,
    Next,
    Vote(VoteKind),
    Open(Link),
    ToggleWhy,
    HideToast(u64),
    EndFlash(u64),
    ImageLoaded(String, Option<image::Handle>),
    DetailsLoaded(AppId, Result<Option<Arc<GameDetails>>, String>),
    PreviousScreenshot,
    NextScreenshot,
    ShowScreenshot(usize),
    RetryDetails,
}

#[derive(Clone, Copy, Debug)]
pub enum Link {
    Steam,
    Eneba,
    Hltb,
}

enum State {
    Loading,
    Ready(Queue),
    Failed(String),
}

impl Discover {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Discover, Task<Message>) {
        let options = SearchOptions::default();
        let search = options.clone();
        let recommender = Arc::clone(gamepalate);
        let task = Task::perform(
            async move { recommender.recommend(&search) },
            Message::Recommended,
        );

        (
            Discover {
                gamepalate: Arc::clone(gamepalate),
                options,
                votes_this_session: 0,
                bet_every: gamepalate.bet_every(),
                why_open: true,
                toast: None,
                flash: None,
                timers_started: 0,
                images: Images::default(),
                pages: Pages::default(),
                screenshot: 0,
                shown: None,
                state: State::Loading,
            },
            task,
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.follow_current();
        let details = self.load_details();

        Task::batch([task, details, self.load_images()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Recommended(Ok(recommendation)) => {
                let mut queue = Queue::new(recommendation.games);
                let load = queue.wants_next();
                self.state = State::Ready(queue);
                if load {
                    return self.load_more();
                }
            }
            Message::Recommended(Err(error)) => {
                self.state = State::Failed(error.to_string());
            }
            Message::MoreRecommended(result) => {
                if let State::Ready(queue) = &mut self.state {
                    match result {
                        Ok(recommendation) => queue.receive(recommendation.games),
                        Err(error) => {
                            queue.failed();
                            return self.show_toast(error.to_string());
                        }
                    }
                    if queue.wants_next() {
                        return self.load_more();
                    }
                }
            }
            Message::Previous => {
                if let State::Ready(queue) = &mut self.state {
                    queue.back();
                }
            }
            Message::Next => return self.advance(),
            Message::Vote(kind) => {
                if let Some(game) = self.current() {
                    let text = format::voted(&game.name, kind);
                    return Task::batch([self.show_toast(text), self.flash(kind), self.advance()]);
                }
            }
            Message::Open(link) => {
                if let Some(game) = self.current() {
                    let url = match link {
                        Link::Steam => game.store_url.clone(),
                        Link::Eneba => format::eneba_url(&game.name),
                        Link::Hltb => format::hltb_url(&game.name),
                    };
                    let _ = Command::new("rundll32")
                        .args(["url.dll,FileProtocolHandler", &url])
                        .spawn();
                }
            }
            Message::ToggleWhy => self.why_open = !self.why_open,
            Message::EndFlash(id) => {
                if self.flash.as_ref().is_some_and(|flash| flash.id == id) {
                    self.flash = None;
                }
            }
            Message::HideToast(id) => {
                if self.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.toast = None;
                }
            }
            Message::ImageLoaded(url, handle) => self.images.loaded(url, handle),
            Message::DetailsLoaded(id, result) => self.pages.loaded(id, result),
            Message::PreviousScreenshot => self.screenshot = self.screenshot.saturating_sub(1),
            Message::NextScreenshot => {
                self.screenshot = next_index(self.screenshot, self.screenshot_count());
            }
            Message::ShowScreenshot(index) => self.screenshot = index,
            Message::RetryDetails => {
                if let Some(id) = self.current().map(|game| game.id) {
                    self.pages.retry(id);
                }
            }
        }

        Task::none()
    }

    pub fn subscription() -> Subscription<Message> {
        keyboard::listen().filter_map(key_message)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let filters = container(
            row![
                text!("{} ·", format::filters(&self.options))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                button(text("Filters (Ctrl+F) ▾").size(font::NOTE_SIZE))
                    .padding(0)
                    .style(style::link),
                space().width(Fill),
                text(format::votes_this_session(self.votes_this_session))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .spacing(4)
            .align_y(Center),
        )
        .padding([6, 20])
        .width(Fill);
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
        let game = container(self.game())
            .padding(16)
            .width(Fill)
            .height(Fill)
            .clip(true);
        let main = stack![game].push(toast);
        let votes = container(
            container(responsive(|size| self.vote_row(size.width < COMPACT_WIDTH)))
                .height(VOTE_HEIGHT),
        )
        .padding([10, 20])
        .width(Fill);

        column![filters, style::separator(), main, style::separator(), votes].into()
    }

    fn vote_row(&self, compact: bool) -> Element<'_, Message> {
        let game = self.current();
        let hint = (!compact).then(|| {
            container(
                text("Space skip · ⌫ back · ← → screenshots")
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            )
            .padding(padding::left(4))
        });
        let flashed = self.flash.as_ref().map(|flash| flash.kind);
        let votes = row(VOTES.map(|(key, kind)| {
            vote_button(key, kind, compact, game.is_some(), flashed == Some(kind))
        }))
        .push(hint)
        .spacing(8)
        .width(Fill)
        .align_y(Center);
        let links = row![
            link_button("5 Steam ↗", Link::Steam, game.is_some()),
            link_button("6 Eneba ↗", Link::Eneba, game.is_some()),
            link_button("7 HLTB ↗", Link::Hltb, game.is_some()),
        ]
        .spacing(20);

        row![votes, links]
            .spacing(8)
            .height(Fill)
            .align_y(Center)
            .into()
    }

    fn advance(&mut self) -> Task<Message> {
        let load = match &mut self.state {
            State::Ready(queue) => queue.advance(),
            State::Loading | State::Failed(_) => false,
        };

        if load { self.load_more() } else { Task::none() }
    }

    fn load_more(&self) -> Task<Message> {
        let State::Ready(queue) = &self.state else {
            return Task::none();
        };
        let gamepalate = Arc::clone(&self.gamepalate);
        let options = self.options.clone();
        let seen = queue.seen();

        Task::perform(
            async move { gamepalate.recommend_after(&options, &seen) },
            Message::MoreRecommended,
        )
    }

    fn show_toast(&mut self, text: String) -> Task<Message> {
        let id = self.start_timer();
        self.toast = Some(Toast { id, text });

        after(TOAST_DURATION, Message::HideToast(id))
    }

    fn flash(&mut self, kind: VoteKind) -> Task<Message> {
        let id = self.start_timer();
        self.flash = Some(Flash { id, kind });

        after(FLASH_DURATION, Message::EndFlash(id))
    }

    fn start_timer(&mut self) -> u64 {
        self.timers_started += 1;
        self.timers_started
    }

    fn follow_current(&mut self) {
        let current = self.current().map(|game| game.id);
        if current != self.shown {
            self.shown = current;
            self.screenshot = 0;
        }
    }

    fn load_details(&mut self) -> Task<Message> {
        let State::Ready(queue) = &self.state else {
            return Task::none();
        };
        let ids = |games: &[RecommendedGame]| -> Vec<AppId> {
            games.iter().map(|game| game.id).collect()
        };
        let wanted = ids(queue.upcoming(DETAILS_AHEAD));
        let nearby = ids(queue.around(KEEP_BEHIND, PRELOAD_AHEAD));

        self.pages.keep_only(&nearby);
        let started = self.pages.want(&wanted);

        Task::batch(started.into_iter().map(|id| {
            let gamepalate = Arc::clone(&self.gamepalate);
            Task::perform(
                async move {
                    gamepalate
                        .details(id)
                        .await
                        .map(|details| details.map(Arc::new))
                        .map_err(|error| error.to_string())
                },
                move |result| Message::DetailsLoaded(id, result),
            )
        }))
    }

    fn load_images(&mut self) -> Task<Message> {
        let State::Ready(queue) = &self.state else {
            return Task::none();
        };
        let first_screenshot = |game: &RecommendedGame| self.screenshots(game.id).first().cloned();

        let mut wanted = Vec::new();
        if let Some((current, next)) = queue.upcoming(PRELOAD_AHEAD).split_first() {
            let screenshots = self.screenshots(current.id);
            wanted.extend(screenshots.get(self.screenshot).cloned());
            wanted.push(self.header_url(current));
            wanted.extend(screenshots.get(self.screenshot + 1).cloned());
            for game in next {
                wanted.push(self.header_url(game));
                wanted.extend(first_screenshot(game));
            }
        }

        let mut nearby = Vec::new();
        for game in queue.around(KEEP_BEHIND, PRELOAD_AHEAD) {
            nearby.push(self.gamepalate.header_url(game.id));
            nearby.extend(
                self.pages
                    .details(game.id)
                    .and_then(|details| details.store.header_url.clone()),
            );
            nearby.extend(first_screenshot(game));
        }
        if let Some(current) = queue.current() {
            nearby.extend(self.screenshots(current.id).iter().cloned());
        }

        self.images.keep_only(&nearby);
        let started = self.images.want(wanted);

        Task::batch(started.into_iter().map(|url| {
            let gamepalate = Arc::clone(&self.gamepalate);
            Task::perform(
                async move {
                    let handle = gamepalate
                        .image(&url)
                        .await
                        .ok()
                        .map(image::Handle::from_bytes);
                    (url, handle)
                },
                |(url, handle)| Message::ImageLoaded(url, handle),
            )
        }))
    }

    fn screenshots(&self, id: AppId) -> &[String] {
        self.pages
            .details(id)
            .map_or(&[], |details| details.store.screenshot_urls.as_slice())
    }

    fn screenshot_count(&self) -> usize {
        self.current()
            .map_or(0, |game| self.screenshots(game.id).len())
    }

    fn header_url(&self, game: &RecommendedGame) -> String {
        let url = self.gamepalate.header_url(game.id);
        let hashed = self
            .pages
            .details(game.id)
            .and_then(|details| details.store.header_url.clone());

        match (self.images.get(&url), hashed) {
            (Some(Picture::Failed), Some(hashed)) => hashed,
            _ => url,
        }
    }

    fn picture<'a>(&self, url: &str, width: f32, height: f32) -> Element<'a, Message> {
        let content: Element<'a, Message> = match self.images.get(url) {
            Some(Picture::Ready(handle)) => image(handle.clone())
                .width(width)
                .height(height)
                .content_fit(ContentFit::Cover)
                .border_radius(IMAGE_RADIUS)
                .into(),
            Some(Picture::Failed) => note("Image unavailable"),
            Some(Picture::Loading) | None => space().into(),
        };

        container(content)
            .center_x(width)
            .center_y(height)
            .style(style::picture)
            .into()
    }

    fn screenshot<'a>(
        &self,
        game: &RecommendedGame,
        width: f32,
        height: f32,
    ) -> Element<'a, Message> {
        let content: Element<'a, Message> = match self.pages.get(game.id) {
            Some(Page::Ready(details)) => {
                match details.store.screenshot_urls.get(self.screenshot) {
                    Some(url) => return self.picture(url, width, height),
                    None => note("No screenshots"),
                }
            }
            Some(Page::Missing) => note("Not on the Steam store"),
            Some(Page::Failed) => column![
                note("The Steam store didn't answer: showing catalog data only."),
                button(text("Retry").size(font::NOTE_SIZE))
                    .padding([4, 12])
                    .on_press(Message::RetryDetails),
            ]
            .spacing(8)
            .align_x(Center)
            .into(),
            Some(Page::Loading) | None => space().into(),
        };

        container(content)
            .center_x(width)
            .center_y(height)
            .style(style::picture)
            .into()
    }

    fn dots<'a>(&self, count: usize) -> Option<Element<'a, Message>> {
        (count > 1).then(|| {
            row((0..count).map(|index| {
                button(space().width(DOT_SIZE).height(DOT_SIZE))
                    .padding(0)
                    .style(style::dot(index == self.screenshot))
                    .on_press(Message::ShowScreenshot(index))
                    .into()
            }))
            .push(
                container(
                    text(format::screenshot_position(self.screenshot, count))
                        .size(font::NOTE_SIZE)
                        .style(style::muted),
                )
                .padding(padding::left(6)),
            )
            .spacing(6)
            .align_y(Center)
            .wrap()
            .into()
        })
    }

    fn card<'a>(
        &'a self,
        queue: &'a Queue,
        game: &'a RecommendedGame,
        width: f32,
    ) -> Element<'a, Message> {
        let (shot_width, shot_height) = screenshot_size(width);
        let details = self.pages.details(game.id);
        let (position, total) = queue.position();
        let progress = if queue.is_waiting() {
            "Loading the next games…".to_string()
        } else {
            format!("{position} / {total}")
        };

        let media = column![self.screenshot(game, shot_width, shot_height)]
            .push(details.and_then(|details| self.dots(details.store.screenshot_urls.len())))
            .push(
                details
                    .and_then(|details| details.store.short_description.as_deref())
                    .map(|description| {
                        container(text(description).size(font::BAR_SIZE))
                            .max_width(DESCRIPTION_WIDTH)
                    }),
            )
            .push(self.why(game))
            .spacing(14)
            .width(shot_width);
        let info = column![
            self.picture(&self.header_url(game), SIDE_WIDTH, HEADER_HEIGHT),
            text(&game.name).size(font::TITLE_SIZE).font(font::SEMIBOLD),
            price_row(game),
            reviews_line(game),
        ]
        .push(similar_lines(game))
        .push(details.map(facts))
        .push(text(progress).size(font::NOTE_SIZE).style(style::muted))
        .spacing(14)
        .width(SIDE_WIDTH);

        scrollable(
            container(row![media, info].spacing(CARD_SPACING))
                .center_x(Fill)
                .padding(padding::right(SCROLLBAR_GAP)),
        )
        .height(Fill)
        .into()
    }

    fn current(&self) -> Option<&RecommendedGame> {
        match &self.state {
            State::Ready(queue) => queue.current(),
            State::Loading | State::Failed(_) => None,
        }
    }

    fn game(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.state {
            State::Loading => text("Loading…").style(style::muted).into(),
            State::Failed(error) => text!("Error: {error}").into(),
            State::Ready(queue) => match queue.current() {
                None => text("No games match these filters.")
                    .style(style::muted)
                    .into(),
                Some(game) => responsive(move |size| self.card(queue, game, size.width)).into(),
            },
        };

        container(content).center(Fill).into()
    }
}

impl Discover {
    fn why<'a>(&self, game: &'a RecommendedGame) -> Element<'a, Message> {
        if !self.why_open {
            return text("Why this game (W) \u{25B8}")
                .size(font::NOTE_SIZE)
                .style(style::muted)
                .into();
        }

        let bet = game.bet.then(|| {
            text(format::bet(self.bet_every))
                .size(font::INFO_SIZE)
                .style(style::muted)
        });

        container(
            column![
                text("Why this game (W)")
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                meter(
                    "Affinity",
                    format::affinity_share(game.affinity),
                    format::affinity(game.affinity),
                ),
                meter(
                    "Quality",
                    game.quality as f32,
                    format::quality(game.quality).to_string(),
                ),
            ]
            .push(bet)
            .spacing(10),
        )
        .padding([14, 18])
        .width(Fill)
        .style(style::card)
        .into()
    }
}

fn price_row(game: &RecommendedGame) -> Element<'_, Message> {
    let discount = game.price.discount;

    row![
        text(format::price(&game.price))
            .size(font::PRICE_SIZE)
            .font(font::SEMIBOLD)
    ]
    .push(discount.map(|discount| {
        text(format::discount(discount.percent))
            .size(font::INFO_SIZE)
            .font(font::SEMIBOLD)
            .style(style::discount)
    }))
    .push(discount.and_then(|discount| discount.end).map(|end| {
        text(format::until(end))
            .size(font::NOTE_SIZE)
            .style(style::muted)
    }))
    .push(game.bet.then(|| {
        container(text("Bet").size(font::NOTE_SIZE).font(font::SEMIBOLD))
            .padding([2, 8])
            .style(style::bet)
    }))
    .spacing(12)
    .align_y(Center)
    .into()
}

fn reviews_line(game: &RecommendedGame) -> Element<'_, Message> {
    let (percent, rest) = format::reviews(&game.reviews);

    rich_text![span(percent).font(font::SEMIBOLD), span(rest)]
        .size(font::INFO_SIZE)
        .on_link_click(never)
        .into()
}

fn similar_lines(game: &RecommendedGame) -> Option<Element<'_, Message>> {
    let similar = game.similar_to.as_deref().map(|name| {
        rich_text![span("Similar to "), span(name).font(font::SEMIBOLD)]
            .size(font::INFO_SIZE)
            .on_link_click(never)
    });

    let mut spans = Vec::new();
    if let Some(family) = &game.from_family {
        spans.push(span("from your family "));
        spans.push(span(family.as_str()).font(font::SEMIBOLD));
    }
    for tag in &game.top_tags {
        if !spans.is_empty() {
            spans.push(span(" \u{b7} "));
        }
        spans.push(span(tag.as_str()));
    }
    let tags = (!spans.is_empty()).then(|| {
        rich_text(spans)
            .size(font::BAR_SIZE)
            .style(style::muted)
            .on_link_click(never)
    });

    (similar.is_some() || tags.is_some())
        .then(|| column![].push(similar).push(tags).spacing(4).into())
}

fn facts(details: &GameDetails) -> Element<'_, Message> {
    let store = &details.store;
    let lines = [
        store
            .steam_deck
            .map(|deck| format::steam_deck(deck).to_string()),
        Some(format::italian(store.italian)),
        format::play(&details.modes, store.controller),
        format::made_by(&store.developers, store.release_date),
    ];

    column(
        lines
            .into_iter()
            .flatten()
            .map(|line| text(line).size(font::BAR_SIZE).into()),
    )
    .spacing(4)
    .into()
}

fn note<'a>(label: &'static str) -> Element<'a, Message> {
    text(label)
        .size(font::NOTE_SIZE)
        .style(style::muted)
        .center()
        .into()
}

fn screenshot_size(width: f32) -> (f32, f32) {
    let shot_width =
        (width - SIDE_WIDTH - CARD_SPACING - SCROLLBAR_GAP).clamp(MIN_SHOT_WIDTH, MAX_SHOT_WIDTH);

    (shot_width, shot_width * 9.0 / 16.0)
}

fn next_index(index: usize, count: usize) -> usize {
    if index + 1 < count { index + 1 } else { index }
}

fn meter<'a>(label: &'a str, share: f32, value: String) -> Element<'a, Message> {
    row![
        text(label).size(font::INFO_SIZE).width(METER_LABEL_WIDTH),
        progress_bar(0.0..=1.0, share)
            .girth(6)
            .length(Fill)
            .style(style::meter),
        container(text(value).size(font::INFO_SIZE).font(font::SEMIBOLD))
            .align_right(METER_VALUE_WIDTH),
    ]
    .spacing(12)
    .align_y(Center)
    .into()
}

fn vote_button<'a>(
    key: &'a str,
    kind: VoteKind,
    compact: bool,
    enabled: bool,
    flashed: bool,
) -> Element<'a, Message> {
    let mark = if flashed {
        text(CHECK).font(font::ICONS).size(10)
    } else {
        text(key).size(11).style(style::muted)
    };
    let keycap = container(mark)
        .center_x(18)
        .center_y(18)
        .style(style::keycap(flashed));
    let label = text(format::vote(kind))
        .size(if compact {
            COMPACT_VOTE_SIZE
        } else {
            font::VOTE_SIZE
        })
        .font(font::SEMIBOLD);

    container(
        button(
            container(row![keycap, label].spacing(10).align_y(Center))
                .center_x(Fill)
                .center_y(Fill),
        )
        .width(Fill)
        .height(VOTE_HEIGHT)
        .padding(0)
        .style(style::vote(flashed))
        .on_press_maybe(enabled.then_some(Message::Vote(kind))),
    )
    .width(Fill)
    .max_width(VOTE_WIDTH)
    .into()
}

fn after(duration: Duration, message: Message) -> Task<Message> {
    Task::perform(tokio::time::sleep(duration), move |_| message)
}

fn link_button(label: &str, link: Link, enabled: bool) -> Element<'_, Message> {
    button(text(label).size(font::BAR_SIZE))
        .padding(0)
        .style(style::link)
        .on_press_maybe(enabled.then_some(Message::Open(link)))
        .into()
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };

    match key {
        Key::Named(Named::Space) => Some(Message::Next),
        Key::Named(Named::Backspace) => Some(Message::Previous),
        Key::Named(Named::ArrowRight) => Some(Message::NextScreenshot),
        Key::Named(Named::ArrowLeft) => Some(Message::PreviousScreenshot),
        Key::Character(character) => match character.as_str() {
            "5" => Some(Message::Open(Link::Steam)),
            "6" => Some(Message::Open(Link::Eneba)),
            "7" => Some(Message::Open(Link::Hltb)),
            "w" | "W" => Some(Message::ToggleWhy),
            key => VOTES
                .iter()
                .find(|(vote_key, _)| *vote_key == key)
                .map(|(_, kind)| Message::Vote(*kind)),
        },
        Key::Named(_) | Key::Unidentified => None,
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::{Location, Modifiers, key};

    use super::*;

    #[test]
    fn votes_with_the_keys_from_one_to_four() {
        assert!(matches!(
            key_message(pressed(Key::Character("1".into()))),
            Some(Message::Vote(VoteKind::WouldTry))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("4".into()))),
            Some(Message::Vote(VoteKind::Known))
        ));
    }

    #[test]
    fn opens_the_links_with_the_keys_from_five_to_seven() {
        assert!(matches!(
            key_message(pressed(Key::Character("5".into()))),
            Some(Message::Open(Link::Steam))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("7".into()))),
            Some(Message::Open(Link::Hltb))
        ));
    }

    #[test]
    fn toggles_the_reasons_with_w() {
        assert!(matches!(
            key_message(pressed(Key::Character("w".into()))),
            Some(Message::ToggleWhy)
        ));
    }

    #[test]
    fn scrolls_the_screenshots_with_the_arrows() {
        assert!(matches!(
            key_message(pressed(Key::Named(Named::ArrowRight))),
            Some(Message::NextScreenshot)
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(Named::ArrowLeft))),
            Some(Message::PreviousScreenshot)
        ));
    }

    #[test]
    fn moves_between_games_with_space_and_backspace() {
        assert!(matches!(
            key_message(pressed(Key::Named(Named::Space))),
            Some(Message::Next)
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(Named::Backspace))),
            Some(Message::Previous)
        ));
    }

    #[test]
    fn stops_at_the_last_screenshot() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(2, 3), 2);
        assert_eq!(next_index(0, 0), 0);
    }

    #[test]
    fn fits_the_screenshot_between_its_limits() {
        assert_eq!(screenshot_size(1160.0), (756.0, 425.25));
        assert_eq!(screenshot_size(2000.0).0, MAX_SHOT_WIDTH);
        assert_eq!(screenshot_size(600.0).0, MIN_SHOT_WIDTH);
    }

    #[test]
    fn ignores_the_other_keys() {
        assert!(key_message(pressed(Key::Character("8".into()))).is_none());
        assert!(key_message(pressed(Key::Named(Named::Enter))).is_none());
    }

    fn pressed(key: Key) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        }
    }
}
