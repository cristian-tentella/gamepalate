mod filters;
mod queue;

use std::{sync::Arc, time::Duration};

use gamepalate_app::{
    AppId, Blame, FilterOptions, Gamepalate, RecommendError, Recommendation, RecommendedGame,
    SearchOptions, VoteKind,
};
use iced::{
    Center, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    never, padding,
    widget::{
        button, column, container, mouse_area, opaque, progress_bar, responsive, rich_text, row,
        scrollable, space, span, stack, text,
    },
};

use self::{filters::Panel, queue::Queue};
use super::{
    game::{self, Link, VOTE_HEIGHT, VOTES},
    showcase::{self, Showcase},
};
use crate::{font, format, style};

const COMPACT_WIDTH: f32 = 1100.0;
const TOAST_DURATION: Duration = Duration::from_secs(4);
const FLASH_DURATION: Duration = Duration::from_millis(600);
const METER_LABEL_WIDTH: f32 = 64.0;
const METER_VALUE_WIDTH: f32 = 72.0;
const SIDE_WIDTH: f32 = 360.0;
const HEADER_HEIGHT: f32 = SIDE_WIDTH * 215.0 / 460.0;
const MIN_SHOT_WIDTH: f32 = 480.0;
const MAX_SHOT_WIDTH: f32 = 820.0;
const CARD_SPACING: f32 = 32.0;
const SCROLLBAR_GAP: f32 = 12.0;

pub struct Discover {
    gamepalate: Arc<Gamepalate>,
    options: SearchOptions,
    panel: Panel,
    panel_open: bool,
    undo: Option<SearchOptions>,
    searches: u64,
    updating: bool,
    empty_because: Option<Blame>,
    votes_this_session: usize,
    bet_every: usize,
    why_open: bool,
    toast: Option<Toast>,
    flash: Option<Flash>,
    timers_started: u64,
    showcase: Showcase,
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
    Recommended(u64, Result<Recommendation, RecommendError>),
    MoreRecommended(u64, Result<Recommendation, RecommendError>),
    Filters(filters::Message),
    ToggleFilters,
    CloseFilters,
    ResetFilters,
    UndoFilters,
    FiltersSaved(Result<(), String>),
    Previous,
    Next,
    Vote(VoteKind),
    VoteSaved(Result<(), String>),
    Open(Link),
    ToggleWhy,
    HideToast(u64),
    EndFlash(u64),
    Showcase(showcase::Message),
}

enum State {
    Loading,
    Ready(Queue),
    Failed(String),
}

impl Discover {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Discover, Task<Message>) {
        let options = discover_options(gamepalate.gui_settings().search);
        let panel = Panel::new(gamepalate.family_names(), gamepalate.tag_names(), &options);
        let mut discover = Discover {
            gamepalate: Arc::clone(gamepalate),
            options,
            panel,
            panel_open: false,
            undo: None,
            searches: 0,
            updating: false,
            empty_because: None,
            votes_this_session: 0,
            bet_every: gamepalate.bet_every(),
            why_open: true,
            toast: None,
            flash: None,
            timers_started: 0,
            showcase: Showcase::new(gamepalate),
            state: State::Loading,
        };
        let task = discover.recommend();

        (discover, task)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.showcase.follow(self.current().map(|game| game.id));

        Task::batch([task, self.load()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Recommended(search, _) | Message::MoreRecommended(search, _)
                if search != self.searches => {}
            Message::Recommended(_, Ok(recommendation)) => {
                let save = if std::mem::take(&mut self.updating) {
                    self.save_filters()
                } else {
                    Task::none()
                };
                self.empty_because = recommendation.empty_because;
                let mut queue = Queue::new(recommendation.games);
                let load = queue.wants_next();
                self.state = State::Ready(queue);
                return Task::batch([save, self.load_more_if(load)]);
            }
            Message::Recommended(_, Err(error)) => {
                let updating = std::mem::take(&mut self.updating);
                let previous = self
                    .undo
                    .take()
                    .filter(|_| updating && matches!(self.state, State::Ready(_)));
                let Some(previous) = previous else {
                    self.state = State::Failed(error.to_string());
                    return Task::none();
                };
                self.options = previous;
                self.panel.show(&self.options);
                let load = match &mut self.state {
                    State::Ready(queue) => {
                        queue.failed();
                        queue.wants_next()
                    }
                    State::Loading | State::Failed(_) => false,
                };
                return Task::batch([self.show_toast(error.to_string()), self.load_more_if(load)]);
            }
            Message::MoreRecommended(..) if self.updating => {}
            Message::MoreRecommended(_, result) => {
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
                    let gamepalate = Arc::clone(&self.gamepalate);
                    let (id, bet) = (game.id, game.bet);
                    let save = Task::perform(
                        async move {
                            gamepalate
                                .vote(id, kind, bet)
                                .map_err(|error| error.to_string())
                        },
                        Message::VoteSaved,
                    );
                    return Task::batch([
                        save,
                        self.show_toast(text),
                        self.flash(kind),
                        self.advance(),
                    ]);
                }
            }
            Message::VoteSaved(Ok(())) => self.votes_this_session += 1,
            Message::VoteSaved(Err(error)) => return self.show_toast(error),
            Message::Open(link) => {
                if let Some(game) = self.current() {
                    game::open(link, &game.name, &game.store_url);
                }
            }
            Message::ToggleWhy => self.why_open = !self.why_open,
            Message::Filters(message) => return self.change_filters(message),
            Message::ToggleFilters if self.panel_open => return self.close_filters(),
            Message::ToggleFilters => {
                self.panel_open = true;
                self.panel.show(&self.options);
            }
            Message::CloseFilters => return self.close_filters(),
            Message::ResetFilters => {
                let defaults = discover_options(SearchOptions::default());
                if defaults != self.options {
                    self.panel.show(&defaults);
                    return self.search(defaults);
                }
            }
            Message::UndoFilters => {
                if let Some(previous) = self.undo.clone() {
                    self.panel.show(&previous);
                    return self.search(previous);
                }
            }
            Message::FiltersSaved(Ok(())) => {}
            Message::FiltersSaved(Err(error)) => return self.show_toast(error),
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
        let summary = if self.updating {
            "Updating…".to_string()
        } else {
            format::filters(&self.options, self.gamepalate.currency())
        };
        let arrow = if self.panel_open { '▴' } else { '▾' };
        let line = row![
            button(
                row![
                    text!("{summary} ·")
                        .size(font::NOTE_SIZE)
                        .style(style::muted),
                    text!("Filters (Ctrl+F) {arrow}").size(font::NOTE_SIZE),
                ]
                .spacing(4),
            )
            .padding(0)
            .style(style::link)
            .on_press(Message::ToggleFilters),
            space().width(Fill),
            text(format::votes_this_session(self.votes_this_session))
                .size(font::NOTE_SIZE)
                .style(style::muted),
        ]
        .align_y(Center);
        let filters = container(line).padding([6, 20]).width(Fill);
        let popup = self.panel_open.then(|| {
            let panel = container(self.panel.view(&self.options).map(Message::Filters))
                .padding(20)
                .style(style::popup);
            stack![
                opaque(
                    mouse_area(space().width(Fill).height(Fill)).on_press(Message::CloseFilters)
                ),
                container(opaque(panel)).padding([8, 20]),
            ]
        });
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
        let veil = self.updating.then(|| {
            container(text("Updating…").style(style::muted))
                .center(Fill)
                .style(style::veil)
        });
        let main = stack![game].push(veil).push(toast);
        let votes = container(
            container(responsive(|size| self.vote_row(size.width < COMPACT_WIDTH)))
                .height(VOTE_HEIGHT),
        )
        .padding([10, 20])
        .width(Fill);

        let below = stack![column![main, style::separator(), votes]].push(popup);

        column![filters, style::separator(), below].into()
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
            game::vote_button(
                key,
                kind,
                compact,
                game.is_some().then_some(Message::Vote(kind)),
                flashed == Some(kind),
            )
        }))
        .push(hint)
        .spacing(8)
        .width(Fill)
        .align_y(Center);
        let links = game::links(|link| game.is_some().then_some(Message::Open(link)));

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

        self.load_more_if(load)
    }

    fn load_more_if(&self, load: bool) -> Task<Message> {
        if load { self.load_more() } else { Task::none() }
    }

    fn recommend(&mut self) -> Task<Message> {
        self.searches += 1;
        let search = self.searches;
        let gamepalate = Arc::clone(&self.gamepalate);
        let options = self.options.clone();

        Task::perform(
            async move { gamepalate.recommend(&options) },
            move |result| Message::Recommended(search, result),
        )
    }

    fn search(&mut self, options: SearchOptions) -> Task<Message> {
        self.undo = Some(std::mem::replace(&mut self.options, options));
        self.updating = true;
        self.recommend()
    }

    fn change_filters(&mut self, message: filters::Message) -> Task<Message> {
        match self.panel.update(&self.options, message) {
            Ok(Some(options)) => self.search(options),
            Ok(None) => Task::none(),
            Err(error) => self.show_toast(error),
        }
    }

    fn close_filters(&mut self) -> Task<Message> {
        if !std::mem::take(&mut self.panel_open) {
            return Task::none();
        }
        self.change_filters(filters::Message::Submit)
    }

    fn save_filters(&self) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        let search = self.options.clone();

        Task::perform(
            async move {
                gamepalate
                    .change_gui_settings(|settings| settings.search = search)
                    .map_err(|error| error.to_string())
            },
            Message::FiltersSaved,
        )
    }

    fn load_more(&self) -> Task<Message> {
        let State::Ready(queue) = &self.state else {
            return Task::none();
        };
        let gamepalate = Arc::clone(&self.gamepalate);
        let options = self.options.clone();
        let seen = queue.seen();
        let search = self.searches;

        Task::perform(
            async move { gamepalate.recommend_after(&options, &seen) },
            move |result| Message::MoreRecommended(search, result),
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

    fn load(&mut self) -> Task<Message> {
        let State::Ready(queue) = &self.state else {
            return Task::none();
        };
        let ids = |games: &[RecommendedGame]| -> Vec<AppId> {
            games.iter().map(|game| game.id).collect()
        };
        let upcoming = ids(queue.upcoming(showcase::AHEAD));
        let nearby = ids(queue.around(showcase::BEHIND, showcase::AHEAD));

        self.showcase
            .load(&upcoming, &nearby)
            .map(Message::Showcase)
    }

    fn card<'a>(
        &'a self,
        queue: &'a Queue,
        game: &'a RecommendedGame,
        width: f32,
    ) -> Element<'a, Message> {
        let (shot_width, shot_height) = screenshot_size(width);
        let (position, total) = queue.position();
        let progress = if queue.is_waiting() {
            "Loading the next games…".to_string()
        } else {
            format!("{position} / {total}")
        };

        let media = self
            .showcase
            .media(game.id, shot_width, shot_height)
            .map(Message::Showcase);
        let info = column![
            self.showcase
                .header(game.id, SIDE_WIDTH, HEADER_HEIGHT)
                .map(Message::Showcase),
            text(&game.name).size(font::TITLE_SIZE).font(font::SEMIBOLD),
            game::price_row(&game.price, game.bet, self.gamepalate.currency()),
            game::reviews_line(&game.reviews),
        ]
        .push(similar_lines(game))
        .push(self.why(game))
        .push(
            self.showcase
                .facts(game.id)
                .map(|facts| facts.map(Message::Showcase)),
        )
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
            State::Failed(error) => column![
                text!("Error: {error}"),
                small_button("Reset filters", Message::ResetFilters),
            ]
            .spacing(12)
            .align_x(Center)
            .into(),
            State::Ready(queue) => match queue.current() {
                None => self.empty(),
                Some(game) => responsive(move |size| self.card(queue, game, size.width)).into(),
            },
        };

        container(content).center(Fill).into()
    }

    fn empty(&self) -> Element<'_, Message> {
        let buttons = row![small_button("Reset filters", Message::ResetFilters)]
            .push(
                self.undo
                    .is_some()
                    .then(|| small_button("Undo last change", Message::UndoFilters)),
            )
            .spacing(8);

        column![
            text(format::empty_because(self.empty_because.as_ref())).style(style::muted),
            buttons,
        ]
        .spacing(12)
        .align_x(Center)
        .into()
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
                .size(font::NOTE_SIZE)
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
            .spacing(6),
        )
        .padding([10, 14])
        .width(Fill)
        .style(style::card)
        .into()
    }
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

fn small_button(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label).size(font::NOTE_SIZE))
        .padding([4, 12])
        .on_press(message)
        .into()
}

fn discover_options(saved: SearchOptions) -> SearchOptions {
    SearchOptions {
        limit: None,
        filters: FilterOptions {
            without_voted: true,
            ..saved.filters
        },
        ..saved
    }
}

fn screenshot_size(width: f32) -> (f32, f32) {
    let shot_width =
        (width - SIDE_WIDTH - CARD_SPACING - SCROLLBAR_GAP).clamp(MIN_SHOT_WIDTH, MAX_SHOT_WIDTH);

    (shot_width, shot_width * 9.0 / 16.0)
}

fn meter<'a>(label: &'a str, share: f32, value: String) -> Element<'a, Message> {
    row![
        text(label).size(font::BAR_SIZE).width(METER_LABEL_WIDTH),
        progress_bar(0.0..=1.0, share)
            .girth(4)
            .length(Fill)
            .style(style::meter),
        container(text(value).size(font::BAR_SIZE).font(font::SEMIBOLD))
            .align_right(METER_VALUE_WIDTH),
    ]
    .spacing(8)
    .align_y(Center)
    .into()
}

fn after(duration: Duration, message: Message) -> Task<Message> {
    Task::perform(tokio::time::sleep(duration), move |_| message)
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    if modifiers.command() {
        return matches!(key.as_ref(), Key::Character("f" | "F")).then_some(Message::ToggleFilters);
    }

    match key {
        Key::Named(Named::Escape) => Some(Message::CloseFilters),
        Key::Named(Named::Space) => Some(Message::Next),
        Key::Named(Named::Backspace) => Some(Message::Previous),
        Key::Named(Named::ArrowRight) => Some(Message::Showcase(showcase::Message::NextScreenshot)),
        Key::Named(Named::ArrowLeft) => {
            Some(Message::Showcase(showcase::Message::PreviousScreenshot))
        }
        Key::Character(character) => match character.as_str() {
            "w" | "W" => Some(Message::ToggleWhy),
            key => game::vote_key(key)
                .map(Message::Vote)
                .or_else(|| game::link_key(key).map(Message::Open)),
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
            Some(Message::Showcase(showcase::Message::NextScreenshot))
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(Named::ArrowLeft))),
            Some(Message::Showcase(showcase::Message::PreviousScreenshot))
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
    fn fits_the_screenshot_between_its_limits() {
        assert_eq!(screenshot_size(1160.0), (756.0, 425.25));
        assert_eq!(screenshot_size(2000.0).0, MAX_SHOT_WIDTH);
        assert_eq!(screenshot_size(600.0).0, MIN_SHOT_WIDTH);
    }

    #[test]
    fn opens_the_filters_with_ctrl_f_and_closes_them_with_esc() {
        assert!(matches!(
            key_message(with_ctrl(Key::Character("f".into()))),
            Some(Message::ToggleFilters)
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(Named::Escape))),
            Some(Message::CloseFilters)
        ));
        assert!(key_message(with_ctrl(Key::Character("1".into()))).is_none());
    }

    #[test]
    fn the_saved_filters_always_leave_out_the_voted_games() {
        let mut saved = SearchOptions {
            limit: Some(5),
            family: Some("Puzzle".to_string()),
            ..SearchOptions::default()
        };
        saved.filters.max_price_cents = Some(1500);

        let options = discover_options(saved);

        assert!(options.filters.without_voted);
        assert_eq!(options.limit, None);
        assert_eq!(options.family.as_deref(), Some("Puzzle"));
        assert_eq!(options.filters.max_price_cents, Some(1500));
    }

    #[test]
    fn ignores_the_other_keys() {
        assert!(key_message(pressed(Key::Character("8".into()))).is_none());
        assert!(key_message(pressed(Key::Named(Named::Enter))).is_none());
    }

    fn pressed(key: Key) -> keyboard::Event {
        pressed_with(key, Modifiers::empty())
    }

    fn with_ctrl(key: Key) -> keyboard::Event {
        pressed_with(key, Modifiers::CTRL)
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
