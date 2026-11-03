mod filters;
mod queue;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use gamepalate_app::{
    AppId, Blame, FilterOptions, Gamepalate, RecommendError, Recommendation, RecommendedGame,
    SearchOptions, VoteKind,
};
use iced::{
    Center, Element, Event, Fill, Size, Subscription, Task, event,
    keyboard::{self, Key, key::Named},
    mouse, padding,
    widget::{
        Column, button, column, container, mouse_area, opaque, pick_list, responsive, row,
        scrollable, space, stack, text,
    },
};

use self::{
    filters::{Action, Panel, SORTS, Sort, Source},
    queue::Queue,
};
use super::{
    game::{self, Link, VOTE_HEIGHT},
    showcase::{self, Showcase},
};
use crate::{font, format, style};

static SEARCHES: AtomicU64 = AtomicU64::new(0);
const COMPACT_WIDTH: f32 = 1100.0;
const TOAST_DURATION: Duration = Duration::from_secs(4);
const FLASH_DURATION: Duration = Duration::from_millis(600);
const SIDE_WIDTH: f32 = 360.0;
const MIN_SHOT_WIDTH: f32 = 480.0;
const CARD_SPACING: f32 = 32.0;
const INFO_SPACING: f32 = 14.0;
const TITLE_SPACING: f32 = 12.0;
const HEADER_SPACING: f32 = 16.0;
const SIMILAR_SIZE: f32 = 17.0;
const GROUP_SPACING: f32 = 32.0;
const THUMBNAIL_WIDTH: f32 = 144.0;
const SIMILAR_LINE_HEIGHT: f32 = 1.1;
const SIMILAR_GAP: f32 = 12.0;
const SCROLLBAR_GAP: f32 = 12.0;
const GAME_PADDING: f32 = 16.0;
const FILTERS_HEIGHT: f32 = 34.0;
const VOTES_PADDING: f32 = 10.0;
const SEPARATOR: f32 = 1.0;
const HINT_WIDTH: f32 = 180.0;
const HINT_SPACING: f32 = 12.0;
const MEDIA_BELOW: f32 = 80.0;
const SIDE_HEIGHT: f32 = 560.0;
const STACKED_BELOW: f32 = 440.0;
const STACKED_TOP: f32 = 24.0;

pub struct Discover {
    gamepalate: Arc<Gamepalate>,
    options: SearchOptions,
    sources: Vec<Source>,
    panel: Panel,
    panel_open: bool,
    undo: Option<SearchOptions>,
    searches: u64,
    updating: bool,
    empty_because: Option<Blame>,
    votes_this_session: usize,
    bet_every: usize,
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
    Source(Source),
    Sort(Sort),
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
    HideToast(u64),
    EndFlash(u64),
    Showcase(showcase::Message),
    OpenSimilar(AppId),
}

enum State {
    Loading,
    Ready(Queue),
    Failed(String),
}

impl Discover {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Discover, Task<Message>) {
        Discover::opened(gamepalate, None)
    }

    pub fn from_family(gamepalate: &Arc<Gamepalate>, family: String) -> (Discover, Task<Message>) {
        Discover::opened(gamepalate, Some(family))
    }

    fn opened(gamepalate: &Arc<Gamepalate>, family: Option<String>) -> (Discover, Task<Message>) {
        let saved = discover_options(gamepalate.gui_settings().search);
        let options = match &family {
            Some(family) => family_options(&saved, family),
            None => saved.clone(),
        };
        let panel = Panel::new(
            gamepalate.tag_names(),
            &gamepalate.config_filters(),
            format::currency_symbol(gamepalate.currency()),
            &options,
        );
        let changed = options != saved;
        let mut discover = Discover {
            gamepalate: Arc::clone(gamepalate),
            options,
            sources: filters::sources(gamepalate.family_names()),
            panel,
            panel_open: false,
            undo: changed.then_some(saved),
            searches: 0,
            updating: changed,
            empty_because: None,
            votes_this_session: 0,
            bet_every: gamepalate.bet_every(),
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
        if self.showcase.is_open() {
            match message {
                Message::CloseFilters => {
                    return self.update(Message::Showcase(showcase::Message::Close));
                }
                Message::Next
                | Message::Previous
                | Message::Vote(_)
                | Message::Open(_)
                | Message::ToggleFilters => return Task::none(),
                _ => {}
            }
        }
        if self.panel_open && acts_on_the_game(&message) {
            return Task::none();
        }
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
            Message::OpenSimilar(_) => {}
            Message::Source(source) => {
                let options = filters::with_source(&self.options, source);
                return self.search_if_changed(options);
            }
            Message::Sort(Sort(sort)) => {
                let options = SearchOptions {
                    sort,
                    ..self.options.clone()
                };
                return self.search_if_changed(options);
            }
            Message::Filters(message) => return self.change_filters(message),
            Message::ToggleFilters if self.panel_open => self.panel_open = false,
            Message::ToggleFilters => {
                self.panel_open = true;
                self.panel.show(&self.options);
            }
            Message::CloseFilters => self.panel_open = false,
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
        Subscription::batch([
            keyboard::listen().filter_map(key_message),
            event::listen_with(escape_message),
        ])
    }

    pub fn view(&self, scale: f32) -> Element<'_, Message> {
        stack![responsive(move |size| self.screen(size, scale))]
            .push(self.showcase.zoom().map(|zoom| zoom.map(Message::Showcase)))
            .push(
                self.current()
                    .and_then(|game| self.showcase.about(&game.name))
                    .map(|about| about.map(Message::Showcase)),
            )
            .into()
    }

    fn screen(&self, size: Size, scale: f32) -> Element<'_, Message> {
        let game_size = Size::new(
            size.width - 2.0 * GAME_PADDING,
            size.height
                - FILTERS_HEIGHT
                - VOTE_HEIGHT
                - 2.0 * VOTES_PADDING
                - 2.0 * SEPARATOR
                - 2.0 * GAME_PADDING,
        );
        let layout = Layout::new(game_size, showcase::max_screenshot_width(scale));
        let (left, right) = bar_edges(size.width, layout.width());
        let summary = if self.updating {
            Some("Updating…".to_string())
        } else {
            format::filters(&self.options, self.gamepalate.currency())
        };
        let arrow = if self.panel_open { '▴' } else { '▾' };
        let line = row![
            text("From").size(font::NOTE_SIZE).style(style::muted),
            pick_list(
                self.sources.as_slice(),
                Some(filters::source(&self.options)),
                Message::Source
            )
            .text_size(font::NOTE_SIZE)
            .padding([4, 8])
            .style(style::dropdown)
            .menu_style(style::menu),
            pick_list(SORTS, Some(Sort(self.options.sort)), Message::Sort)
                .text_size(font::NOTE_SIZE)
                .padding([4, 8])
                .style(style::dropdown)
                .menu_style(style::menu),
            button(text!("Filters (Ctrl+F) {arrow}").size(font::NOTE_SIZE))
                .padding([4, 8])
                .style(style::text_button)
                .on_press(Message::ToggleFilters),
        ]
        .push(summary.map(|summary| {
            text(summary)
                .size(font::NOTE_SIZE)
                .style(style::muted)
                .wrapping(text::Wrapping::None)
        }))
        .push(space().width(Fill))
        .push(
            text(format::votes_this_session(self.votes_this_session))
                .size(font::NOTE_SIZE)
                .style(style::muted),
        )
        .spacing(8)
        .align_y(Center);
        let filters = container(line)
            .padding(padding::left(left).right(right))
            .width(Fill)
            .height(FILTERS_HEIGHT)
            .align_y(Center);
        let popup = self.panel_open.then(|| {
            let panel = container(scrollable(self.panel.view().map(Message::Filters)).spacing(8))
                .padding(20)
                .style(style::popup);
            stack![
                opaque(
                    mouse_area(space().width(Fill).height(Fill)).on_press(Message::CloseFilters)
                ),
                container(opaque(panel)).padding(padding::top(8).left(left)),
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
        let game = container(self.game(layout, game_size.height))
            .padding(GAME_PADDING)
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
            container(self.vote_row(size.width - left - right, layout)).height(VOTE_HEIGHT),
        )
        .padding(
            padding::top(VOTES_PADDING)
                .bottom(VOTES_PADDING)
                .left(left)
                .right(right),
        )
        .width(Fill);

        let below = stack![column![main, style::separator(), votes]].push(popup);

        column![filters, style::separator(), below].into()
    }

    fn vote_row(&self, width: f32, layout: Layout) -> Element<'_, Message> {
        let compact = width < COMPACT_WIDTH;
        let available = match layout {
            Layout::Beside(shot_width) => shot_width,
            Layout::Stacked(_) => width - game::LINKS_WIDTH,
        };
        let vote_width = game::vote_width(available, if compact { 0.0 } else { HINT_WIDTH });
        let game = self.current();
        let hints = (!compact).then(|| {
            row![
                style::divider(),
                row![game::hint("Space", "Skip"), game::hint("⌫", "Back")].spacing(HINT_SPACING),
            ]
            .spacing(8)
            .align_y(Center)
        });
        let flashed = self.flash.as_ref().map(|flash| flash.kind);
        let votes = game::vote_buttons(|key, kind| {
            game::vote_button(
                key,
                kind,
                compact,
                vote_width,
                game.is_some().then_some(Message::Vote(kind)),
                flashed == Some(kind),
            )
        })
        .push(hints);
        let links = game::links(
            |link| game.is_some().then_some(Message::Open(link)),
            game.is_some()
                .then_some(Message::Showcase(showcase::Message::ToggleAbout)),
        );
        let (votes, spacing) = match layout {
            Layout::Beside(shot_width) => (votes.width(shot_width), CARD_SPACING),
            Layout::Stacked(_) => (votes.width(Fill), 8.0),
        };

        row![votes, links]
            .spacing(spacing)
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
        self.searches = SEARCHES.fetch_add(1, Ordering::Relaxed) + 1;
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

    fn search_if_changed(&mut self, options: SearchOptions) -> Task<Message> {
        if options == self.options {
            Task::none()
        } else {
            self.search(options)
        }
    }

    fn change_filters(&mut self, message: filters::Message) -> Task<Message> {
        match self.panel.update(&self.options, message) {
            Action::Stay => Task::none(),
            Action::Close => {
                self.panel_open = false;
                Task::none()
            }
            Action::Search(options) => {
                self.panel_open = false;
                self.search(options)
            }
        }
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
        let similar: Vec<AppId> = queue
            .upcoming(showcase::AHEAD)
            .iter()
            .filter_map(|game| game.similar_to_id)
            .collect();

        self.showcase
            .load(&upcoming, &nearby, &similar)
            .map(Message::Showcase)
    }

    fn card<'a>(
        &'a self,
        queue: &'a Queue,
        game: &'a RecommendedGame,
        layout: Layout,
        height: f32,
    ) -> Element<'a, Message> {
        let (shot_width, shot_height) = layout.shot_size();
        let content: Element<'a, Message> = match layout {
            Layout::Beside(_) => row![
                self.showcase
                    .media(game.id, shot_width, shot_height)
                    .map(Message::Showcase),
                column![
                    self.summary(game, SIDE_WIDTH, true),
                    self.reasons(queue, game, false)
                ]
                .spacing(GROUP_SPACING)
                .width(SIDE_WIDTH),
            ]
            .spacing(CARD_SPACING)
            .into(),
            Layout::Stacked(_) => {
                let header_width = ((shot_width - CARD_SPACING) / 2.0).min(SIDE_WIDTH);
                column![
                    row![
                        self.summary(game, header_width, false).width(header_width),
                        self.reasons(queue, game, true).width(Fill),
                    ]
                    .spacing(CARD_SPACING),
                    self.showcase
                        .gallery(game.id, shot_width, shot_height)
                        .map(Message::Showcase),
                ]
                .spacing(CARD_SPACING)
                .width(shot_width)
                .into()
            }
        };

        scrollable(
            container(content)
                .center_x(Fill)
                .padding(padding::right(SCROLLBAR_GAP).top(layout.top_space(height))),
        )
        .height(Fill)
        .into()
    }

    fn summary<'a>(
        &'a self,
        game: &'a RecommendedGame,
        header_width: f32,
        beside: bool,
    ) -> Column<'a, Message> {
        let reviews = game::reviews(&game.reviews);
        let currency = self.gamepalate.currency();
        let figures: Element<'a, Message> = if beside {
            let (shift, gap) = game::beside_reviews();
            let price = game::price_row(&game.price, currency, gap);

            row![
                container(reviews).width(THUMBNAIL_WIDTH + SIMILAR_GAP),
                container(price).padding(padding::top(shift)),
            ]
            .into()
        } else {
            let price = game::price_row(&game.price, currency, game::PRICE_GAP);

            column![reviews, price].spacing(INFO_SPACING).into()
        };

        column![
            game::with_bet(
                self.showcase
                    .header(game.id, header_width, header_width * 215.0 / 460.0)
                    .map(Message::Showcase),
                game.bet.then_some(self.bet_every),
            ),
            column![game::title(&game.name), figures].spacing(TITLE_SPACING),
        ]
        .spacing(HEADER_SPACING)
    }

    fn reasons<'a>(
        &'a self,
        queue: &'a Queue,
        game: &'a RecommendedGame,
        with_description: bool,
    ) -> Column<'a, Message> {
        let waiting = queue.is_waiting().then(|| {
            text("Loading the next games…")
                .size(font::NOTE_SIZE)
                .style(style::muted)
        });

        column![]
            .push(self.similar(game))
            .push(self.why(game))
            .push(
                with_description
                    .then(|| self.showcase.description(game.id))
                    .flatten()
                    .map(|description| description.map(Message::Showcase)),
            )
            .push(
                self.showcase
                    .facts(game.id, &game.top_tags)
                    .map(|facts| facts.map(Message::Showcase)),
            )
            .push(waiting)
            .spacing(GROUP_SPACING)
    }

    fn current(&self) -> Option<&RecommendedGame> {
        match &self.state {
            State::Ready(queue) => queue.current(),
            State::Loading | State::Failed(_) => None,
        }
    }

    fn game(&self, layout: Layout, height: f32) -> Element<'_, Message> {
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
                Some(game) => self.card(queue, game, layout, height),
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
    fn similar<'a>(&self, game: &'a RecommendedGame) -> Option<Element<'a, Message>> {
        game.similar_to.as_deref().map(|name| {
            let label = column![
                text("Similar to").size(font::NOTE_SIZE).style(style::muted),
                text(name)
                    .size(SIMILAR_SIZE)
                    .font(font::SEMIBOLD)
                    .line_height(SIMILAR_LINE_HEIGHT),
            ]
            .spacing(2);

            let similar = row![]
                .push(game.similar_to_id.map(|id| {
                    self.showcase
                        .thumbnail(id, THUMBNAIL_WIDTH, THUMBNAIL_WIDTH * 215.0 / 460.0)
                        .map(Message::Showcase)
                }))
                .push(label)
                .spacing(SIMILAR_GAP)
                .align_y(Center);

            match game.similar_to_id {
                Some(id) => mouse_area(similar)
                    .interaction(mouse::Interaction::Pointer)
                    .on_press(Message::OpenSimilar(id))
                    .into(),
                None => similar.into(),
            }
        })
    }

    fn why<'a>(&self, game: &'a RecommendedGame) -> Option<Element<'a, Message>> {
        let reviews = format::few_reviews(&game.reviews, game.quality)?;

        Some(
            container(
                column![
                    text("Why this game")
                        .size(font::NOTE_SIZE)
                        .style(style::muted),
                    text(reviews).size(font::BAR_SIZE),
                ]
                .spacing(4),
            )
            .padding([10, 14])
            .width(Fill)
            .style(style::card)
            .into(),
        )
    }
}

fn small_button(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label).size(font::NOTE_SIZE))
        .style(style::primary)
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

fn family_options(saved: &SearchOptions, family: &str) -> SearchOptions {
    SearchOptions {
        family: Some(family.to_string()),
        signals_only: false,
        ..saved.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Layout {
    Beside(f32),
    Stacked(f32),
}

impl Layout {
    fn new(size: Size, max_shot: f32) -> Self {
        let width = size.width - SIDE_WIDTH - CARD_SPACING - SCROLLBAR_GAP;
        let beside = showcase::screenshot_width(width, size.height, max_shot).max(MIN_SHOT_WIDTH);
        let stacked = (size.width - SCROLLBAR_GAP)
            .min(max_shot)
            .min((size.height - STACKED_BELOW) * 16.0 / 9.0);

        if stacked > beside {
            Self::Stacked(stacked)
        } else {
            Self::Beside(beside)
        }
    }

    fn width(self) -> f32 {
        match self {
            Self::Beside(shot_width) => shot_width + CARD_SPACING + SIDE_WIDTH,
            Self::Stacked(shot_width) => shot_width,
        }
    }

    fn shot_size(self) -> (f32, f32) {
        let (Self::Beside(width) | Self::Stacked(width)) = self;

        (width, width * 9.0 / 16.0)
    }

    fn top_space(self, height: f32) -> f32 {
        match self {
            Self::Beside(_) => {
                let (_, shot_height) = self.shot_size();
                let content = (shot_height + MEDIA_BELOW).max(SIDE_HEIGHT);
                ((height - content) / 2.0).max(0.0)
            }
            Self::Stacked(_) => STACKED_TOP,
        }
    }
}

fn bar_edges(width: f32, content_width: f32) -> (f32, f32) {
    let start = GAME_PADDING + (width - 2.0 * GAME_PADDING - SCROLLBAR_GAP - content_width) / 2.0;
    let left = start.max(GAME_PADDING);

    (left, (width - left - content_width).max(GAME_PADDING))
}

fn after(duration: Duration, message: Message) -> Task<Message> {
    Task::perform(tokio::time::sleep(duration), move |_| message)
}

fn acts_on_the_game(message: &Message) -> bool {
    matches!(
        message,
        Message::Next
            | Message::Previous
            | Message::Vote(_)
            | Message::Open(_)
            | Message::Showcase(
                showcase::Message::NextScreenshot
                    | showcase::Message::PreviousScreenshot
                    | showcase::Message::ToggleAbout
            )
    )
}

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
        return matches!(key.as_ref(), Key::Character("f" | "F")).then_some(Message::ToggleFilters);
    }

    match key {
        Key::Named(Named::Space) => Some(Message::Next),
        Key::Named(Named::Backspace) => Some(Message::Previous),
        Key::Named(Named::ArrowRight) => Some(Message::Showcase(showcase::Message::NextScreenshot)),
        Key::Named(Named::ArrowLeft) => {
            Some(Message::Showcase(showcase::Message::PreviousScreenshot))
        }
        Key::Character(key) if key.eq_ignore_ascii_case("d") => {
            Some(Message::Showcase(showcase::Message::ToggleAbout))
        }
        Key::Character(key) => game::vote_key(key.as_str())
            .map(Message::Vote)
            .or_else(|| game::link_key(key.as_str()).map(Message::Open)),
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
    fn opens_the_description_with_d() {
        for key in ["d", "D"] {
            assert!(matches!(
                key_message(pressed(Key::Character(key.into()))),
                Some(Message::Showcase(showcase::Message::ToggleAbout))
            ));
        }
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
        assert_eq!(
            Layout::new(Size::new(1160.0, 600.0), 1280.0).shot_size(),
            (756.0, 425.25)
        );
        assert_eq!(
            Layout::new(Size::new(2000.0, 596.5), 1280.0),
            Layout::Beside(1000.0)
        );
        assert_eq!(
            Layout::new(Size::new(3000.0, 1400.0), 1280.0),
            Layout::Beside(1280.0)
        );
        assert_eq!(
            Layout::new(Size::new(600.0, 600.0), 1280.0),
            Layout::Beside(MIN_SHOT_WIDTH)
        );
    }

    #[test]
    fn stacks_the_game_when_the_screenshot_gets_wider() {
        assert_eq!(
            Layout::new(Size::new(1248.0, 632.0), 1280.0),
            Layout::Beside(844.0)
        );
        assert_eq!(
            Layout::new(Size::new(1888.0, 864.0), 1280.0),
            Layout::Beside(1280.0)
        );
        assert_eq!(
            Layout::new(Size::new(2528.0, 1272.0), 1280.0),
            Layout::Beside(1280.0)
        );
        assert_eq!(
            Layout::new(Size::new(1248.0, 1272.0), 1280.0),
            Layout::Stacked(1236.0)
        );
        assert_eq!(
            Layout::new(Size::new(928.0, 890.0), 1280.0),
            Layout::Stacked(800.0)
        );
    }

    #[test]
    fn a_bigger_screenshot_keeps_the_game_beside_it() {
        assert_eq!(
            Layout::new(Size::new(2528.0, 1272.0), 1920.0),
            Layout::Beside(1920.0)
        );
    }

    #[test]
    fn the_bars_line_up_with_the_game() {
        assert_eq!(bar_edges(2560.0, 2312.0), (118.0, 130.0));
        assert_eq!(bar_edges(1280.0, 1236.0), (16.0, 28.0));
        assert_eq!(bar_edges(960.0, 916.0), (16.0, 28.0));
    }

    #[test]
    fn centers_the_game_beside_only_when_it_fits_and_starts_it_on_top_when_stacked() {
        assert_eq!(Layout::Beside(844.0).top_space(632.0), 36.0);
        assert_eq!(Layout::Beside(480.0).top_space(500.0), 0.0);
        assert_eq!(Layout::Stacked(1236.0).top_space(1272.0), 24.0);
        assert_eq!(Layout::Stacked(800.0).top_space(800.0), 24.0);
    }

    #[test]
    fn opens_the_filters_with_ctrl_f_and_closes_them_with_esc() {
        assert!(matches!(
            key_message(with_ctrl(Key::Character("f".into()))),
            Some(Message::ToggleFilters)
        ));
        assert!(key_message(with_ctrl(Key::Character("1".into()))).is_none());
        assert!(key_message(pressed(Key::Named(Named::Escape))).is_none());
    }

    #[test]
    fn esc_closes_even_from_inside_a_field() {
        let escape = Event::Keyboard(pressed(Key::Named(Named::Escape)));
        let other = Event::Keyboard(pressed(Key::Named(Named::Space)));

        assert!(matches!(
            escape_message(escape, event::Status::Captured, iced::window::Id::unique()),
            Some(Message::CloseFilters)
        ));
        assert!(
            escape_message(other, event::Status::Ignored, iced::window::Id::unique()).is_none()
        );
    }

    #[test]
    fn the_open_filters_stop_the_keys_of_the_game() {
        assert!(acts_on_the_game(&Message::Vote(VoteKind::WouldTry)));
        assert!(acts_on_the_game(&Message::Next));
        assert!(acts_on_the_game(&Message::Showcase(
            showcase::Message::NextScreenshot
        )));
        assert!(!acts_on_the_game(&Message::CloseFilters));
        assert!(!acts_on_the_game(&Message::ToggleFilters));
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
    fn opening_from_a_family_keeps_the_other_filters() {
        let saved = SearchOptions {
            signals_only: true,
            filters: FilterOptions {
                max_price_cents: Some(1500),
                ..FilterOptions::default()
            },
            ..SearchOptions::default()
        };

        let options = family_options(&saved, "misteri");

        assert_eq!(options.family.as_deref(), Some("misteri"));
        assert!(!options.signals_only);
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
