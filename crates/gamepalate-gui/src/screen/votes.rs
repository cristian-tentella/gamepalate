use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    ops::Range,
    sync::Arc,
};

use gamepalate_app::{
    AppId, Currency, DiscoverMode, FilterOptions, Gamepalate, ListId, Lists, Price, SaveChange,
    SearchOptions, Vote, VoteKind, VotedGame, VotesOrder, VotesSettings, VotesShow,
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
    list_edit::{self, ListEdit},
    save_menu::{self, SaveMenu},
    sentence,
    showcase::{self, Showcase},
};
use crate::{font, format, style};

// Two rows of two equal segments: four in one row would not fit the 340 px of the list.
const FILTERS: [Filter; 4] = [
    Filter::Saved,
    Filter::WouldTry,
    Filter::NotForMe,
    Filter::All,
];
const SEGMENT_HEIGHT: f32 = 26.0;
// Two rows, the seam between them and the padding inside the border.
const SEGMENTS_HEIGHT: f32 = 2.0 * SEGMENT_HEIGHT + 1.0 + 2.0;
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
/// The height of the window before the first `Scrolled` tells the real one.
const FIRST_HEIGHT: f32 = 1_500.0;
const SIGN_SIZE: f32 = 11.0;
const SIGN_WIDTH: f32 = 18.0;
const NO_VOTES: &str = "No votes yet. Vote in Discover with keys 1–3.";
const NO_SAVED: &str = "No saved games";
const SAVED_HINT: &str = "Press S on a game you are thinking of buying";
const NO_LIST: &str = "No list";

pub struct Votes {
    gamepalate: Arc<Gamepalate>,
    games: BTreeMap<AppId, VotedGame>,
    filter: Filter,
    // The segment saved in gui.toml: it changes only when a segment is chosen here.
    chip: VotesShow,
    order: Order,
    hide_owned: bool,
    only_bets: bool,
    filters: FilterOptions,
    passing: Option<BTreeSet<AppId>>,
    panel: Panel,
    panel_open: bool,
    tools_height: f32,
    search: String,
    matches: Option<HashMap<AppId, f64>>,
    matching: u64,
    rows: Vec<Line>,
    // Always on a game, when there is one.
    selected: usize,
    viewport: Option<(f32, f32)>,
    toasts: Toasts,
    played: Option<AppId>,
    showcase: Showcase,
    save_menu: SaveMenu,
    list_edit: ListEdit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    /// The saved Would try votes.
    Saved,
    WouldTry,
    NotForMe,
    /// Every vote, the Laters too.
    All,
}

/// A line of the list: a game, or in Saved the title of a section and the note of an empty list.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    Game(AppId, Section),
    Title(Section),
    Empty(ListId),
}

/// The part of the list a game line is in. Saved has a section for each list and one for the
/// games in none; the other segments, and Saved without lists, have only `Whole`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Whole,
    List(ListId),
    NoList,
}

impl Line {
    fn game(self) -> Option<AppId> {
        match self {
            Line::Game(id, _) => Some(id),
            Line::Title(_) | Line::Empty(_) => None,
        }
    }
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
    Save,
    SaveMenu(save_menu::Message),
    ListCreated(AppId, Result<ListId, String>),
    ListEdit(list_edit::Message),
    ListRenamed(Result<(), String>),
    ListDeleted(Result<(), String>),
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
            only_bets: saved.only_bets,
            filters,
            passing,
            panel: Panel::new(
                gamepalate.tag_names(),
                &FilterOptions::default(),
                format::currency_symbol(gamepalate.currency()),
                &SearchOptions::default(),
            ),
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
            save_menu: SaveMenu::new(gamepalate),
            list_edit: ListEdit::default(),
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
        votes.selected = votes
            .rows
            .iter()
            .position(|line| line.game() == Some(id))
            .unwrap_or_else(|| first_game(&votes.rows));
        let top = line_top(&votes.rows, votes.selected) as f32 * ROW_HEIGHT;
        votes.viewport = Some((top, FIRST_HEIGHT));
        let scroll = operation::scroll_to(
            LIST,
            AbsoluteOffset {
                x: None,
                y: Some(top),
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
        if stops_the_trailer(&message) {
            self.showcase.stop_trailer();
        }
        let task = self.handle(message);
        self.save_menu.keep_for(self.current().map(|game| game.id));

        Task::batch([task, self.load()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Filter(filter) => {
                self.filter = filter;
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
                self.panel
                    .show(&search_options(&self.filters, self.only_bets));
            }
            Message::CloseFilters => self.panel_open = false,
            Message::ResetFilters => {
                self.filters = FilterOptions::default();
                self.passing = None;
                self.only_bets = false;
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
                self.selected = previous_game(&self.rows, self.selected);
                return self.reveal();
            }
            Message::Next => return self.next(),
            Message::Scrolled(viewport) => {
                self.viewport = Some((viewport.absolute_offset().y, viewport.bounds().height));
            }
            Message::Vote(pick) => {
                if let (Some(game), Some(kind)) = (self.current(), pick.ballot()) {
                    let text = format::voted(&game.name, pick);
                    let id = game.id;
                    let gamepalate = Arc::clone(&self.gamepalate);
                    let save = Task::perform(
                        async move {
                            gamepalate
                                .vote(id, kind, false, false)
                                .map_err(|error| error.to_string())
                        },
                        Message::VoteSaved,
                    );
                    return Task::batch([save, self.show_toast(text), self.next()]);
                }
            }
            Message::Save => {
                if let Some(id) = self
                    .current()
                    .filter(|game| may_save(game))
                    .map(|game| game.id)
                {
                    self.save_menu.open(&self.gamepalate, id);
                }
            }
            Message::SaveMenu(message) => {
                let Some(game) = self
                    .current()
                    .filter(|game| self.save_menu.game() == Some(game.id))
                else {
                    return Task::none();
                };
                let (id, saved, lists) = (game.id, game.vote.saved, game.vote.lists.clone());
                let (action, task) = self.save_menu.update(message, saved, &lists);
                let task = task.map(Message::SaveMenu);
                return match action {
                    Some(save_menu::Action::Save(change)) => {
                        Task::batch([task, self.change_saved(id, change)])
                    }
                    Some(save_menu::Action::Create(name)) => {
                        let gamepalate = Arc::clone(&self.gamepalate);
                        let create = Task::perform(
                            async move {
                                gamepalate
                                    .create_list(&name)
                                    .map_err(|error| sentence(&error.to_string()))
                            },
                            move |result| Message::ListCreated(id, result),
                        );
                        Task::batch([task, create])
                    }
                    None => task,
                };
            }
            Message::ListCreated(id, Ok(list)) => {
                self.save_menu.created(&self.gamepalate);
                return self.change_saved(id, SaveChange::AddTo(list));
            }
            Message::ListCreated(_, Err(error)) => self.save_menu.refused(error),
            Message::ListEdit(message) => {
                let games = &self.games;
                let (action, task) =
                    self.list_edit
                        .update(message, self.save_menu.lists(), |list| {
                            games_in(games, list)
                        });
                let task = task.map(Message::ListEdit);
                let gamepalate = Arc::clone(&self.gamepalate);
                return match action {
                    Some(list_edit::Action::Rename(list, name)) => {
                        let rename = Task::perform(
                            async move {
                                gamepalate
                                    .rename_list(list, &name)
                                    .map_err(|error| sentence(&error.to_string()))
                            },
                            Message::ListRenamed,
                        );
                        Task::batch([task, rename])
                    }
                    Some(list_edit::Action::Delete(list)) => {
                        let delete = Task::perform(
                            async move {
                                gamepalate
                                    .delete_list(list)
                                    .map_err(|error| sentence(&error.to_string()))
                            },
                            Message::ListDeleted,
                        );
                        Task::batch([task, delete])
                    }
                    None => task,
                };
            }
            Message::ListRenamed(Ok(())) => {
                self.save_menu.follow(&self.gamepalate);
                self.list_edit.renamed();
                return self.refresh();
            }
            Message::ListRenamed(Err(error)) => self.list_edit.refused(error),
            Message::ListDeleted(Ok(())) => {
                self.save_menu.follow(&self.gamepalate);
                self.games = by_id(self.gamepalate.voted_games());
                return self.refresh();
            }
            Message::ListDeleted(Err(error)) => return self.show_toast(error),
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

    pub fn subscription(&self, keys: bool) -> Subscription<Message> {
        let trailer = self.showcase.subscription().map(Message::Showcase);
        if !keys {
            return trailer;
        }
        if self.save_menu.is_open() {
            return Subscription::batch([save_menu::keys().map(Message::SaveMenu), trailer]);
        }
        if self.list_edit.is_open() {
            return Subscription::batch([list_edit::keys().map(Message::ListEdit), trailer]);
        }
        Subscription::batch([
            keyboard::listen().filter_map(key_message),
            event::listen_with(escape_message),
            trailer,
        ])
    }

    /// A game or the search over Votes stops the trailer.
    pub fn stop_trailer(&mut self) {
        self.showcase.stop_trailer();
    }

    /// The videos of the About sheet stop while a game or the search covers Votes.
    pub fn cover(&mut self, covered: bool) {
        self.showcase.cover(covered);
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
            let panel = container(
                scrollable(self.panel.view(DiscoverMode::New).map(Message::Filters)).spacing(8),
            )
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
        let segment = |index: usize| -> Element<'_, Message> {
            let filter = FILTERS[index];
            let count = format::count(count(&self.games, filter, self.shown()) as u32);
            button(
                container(
                    text(format!("{} {count}", filter_label(filter)))
                        .size(font::NOTE_SIZE)
                        .wrapping(Wrapping::None),
                )
                .center(Fill),
            )
            .width(Fill)
            .height(Fill)
            .padding([0, 4])
            .style(style::segment(
                !searching && filter == self.filter,
                segment_place(index),
            ))
            .on_press(Message::Filter(filter))
            .into()
        };
        let segments = |first: usize| {
            row![
                segment(first),
                rule::vertical(1).style(style::vote_seam),
                segment(first + 1)
            ]
            .height(Fill)
        };
        let filters = container(
            column![
                segments(0),
                rule::horizontal(1).style(style::vote_seam),
                segments(2)
            ]
            .height(Fill),
        )
        .height(SEGMENTS_HEIGHT)
        .padding(1)
        .style(style::vote_group);
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
        let summary = (!searching && self.only_bets)
            .then(|| bets_summary(&self.games))
            .flatten()
            .map(|(would_try, bets, others)| {
                text(format::bets_summary(would_try, bets, others))
                    .size(font::NOTE_SIZE)
                    .style(style::muted)
            });

        let sorting = self.order == Order::BestMatch && self.matches.is_none();
        let has_games = self.rows.iter().any(|line| line.game().is_some());
        // Without games Saved can still have the sections of its empty lists, under the note.
        let note = (sorting || !has_games).then(|| {
            let note = if sorting {
                "Sorting…".to_string()
            } else {
                empty_text(
                    self.games.is_empty(),
                    self.filter,
                    self.is_filtered(),
                    self.owned_hidden(),
                    &self.search,
                )
            };
            let reset = (!sorting && !searching && self.is_filtered()).then(|| {
                button(text("Reset filters").size(font::NOTE_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::ResetFilters)
            });
            let hint = (note == NO_SAVED)
                .then(|| text(SAVED_HINT).size(font::NOTE_SIZE).style(style::muted));
            container(
                column![text(note).size(font::NOTE_SIZE).style(style::muted)]
                    .push(hint)
                    .push(reset)
                    .spacing(8),
            )
            .padding(16)
        });
        let lines = (!sorting && !self.rows.is_empty()).then(|| {
            let (offset, height) = self.viewport.unwrap_or((0.0, FIRST_HEIGHT));
            let count = self.rows.len();
            let range = shown(offset, height, count);
            let above = rows_height(range.start);
            let below = rows_height(count - range.end);
            let built = range.clone().filter_map(|index| self.line(index));
            scrollable(
                container(
                    column![space().height(above)]
                        .extend(built)
                        .push(space().height(below)),
                )
                .padding(padding::right(SCROLLBAR_GAP)),
            )
            .id(LIST)
            .on_scroll(Message::Scrolled)
            .height(Fill)
        });
        let body = column![].push(note).push(lines);

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

    fn line(&self, index: usize) -> Option<Element<'_, Message>> {
        let note = |line: String| text(line).size(font::NOTE_SIZE).wrapping(Wrapping::None);
        let fixed = |content: Element<'static, Message>| {
            container(content)
                .width(Fill)
                .height(ROW_HEIGHT)
                .padding([0, 12])
                .center_y(ROW_HEIGHT)
                .clip(true)
        };
        match self.rows.get(index)? {
            Line::Game(id, _) => self.games.get(id).map(|game| self.list_row(index, game)),
            &Line::Title(section) => {
                let name = match section {
                    Section::List(list) => self.lists().get(&list)?.as_str(),
                    Section::NoList | Section::Whole => NO_LIST,
                };
                let games = self
                    .rows
                    .iter()
                    .filter(|line| matches!(line, Line::Game(_, of) if *of == section))
                    .count();
                let title = fixed(
                    text(section_title(name, games))
                        .size(font::BAR_SIZE)
                        .font(font::SEMIBOLD)
                        .wrapping(Wrapping::None)
                        .into(),
                );
                Some(match section {
                    Section::List(list) => self.list_edit.title(
                        mouse_area(title)
                            .on_right_press(Message::ListEdit(list_edit::Message::Open(list)))
                            .into(),
                        list,
                        name,
                        games_in(&self.games, list),
                        Message::ListEdit,
                    ),
                    Section::NoList | Section::Whole => title.into(),
                })
            }
            Line::Empty(list) => {
                let name = self.lists().get(list)?;
                Some(
                    fixed(
                        note(format!("No games in {name}"))
                            .style(style::muted)
                            .into(),
                    )
                    .into(),
                )
            }
        }
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
        let save = shows_saved(self.filter, &game.vote).then(|| {
            text(font::BOOKMARK)
                .font(font::ICONS)
                .size(font::icon(SIGN_SIZE))
                .style(style::muted)
        });
        let name = container(
            row![]
                .push(later)
                .push(save)
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
                        .gallery(game.id, width, height, scale)
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
                    showcase.facts(game.id, &showcase.specific_tags()),
                    showcase.description(game.id),
                    showcase.more(game.id, true),
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
        let (would_try_hours, elsewhere_hours) = (
            self.gamepalate.would_try_hours(),
            self.gamepalate.elsewhere_hours(),
        );
        let save = game.is_some_and(may_save);
        let vote_width = game::vote_width(width, if save { game::SAVE_SPACE } else { 0.0 });
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
            game.filter(|game| may_save(game)).map(|game| {
                self.save_menu.save(
                    game.vote.saved,
                    &game.vote.lists,
                    Some(Message::Save),
                    Message::SaveMenu,
                )
            }),
        )
        .width(Fill);

        container(votes).center_y(Fill).into()
    }

    /// Saves the change to Save on `id`. The game stays selected.
    fn change_saved(&mut self, id: AppId, change: SaveChange) -> Task<Message> {
        let Some(game) = self.games.get(&id) else {
            return Task::none();
        };
        let text = match change {
            SaveChange::Save | SaveChange::Unsave => format::save_toast(&game.name, change.saves()),
            SaveChange::AddTo(list) => format::list_toast(self.save_menu.name(list), true),
            SaveChange::RemoveFrom(list) => format::list_toast(self.save_menu.name(list), false),
        };
        let gamepalate = Arc::clone(&self.gamepalate);
        let save = Task::perform(
            async move {
                gamepalate
                    .save(id, change, false, false)
                    .map_err(|error| error.to_string())
            },
            Message::VoteSaved,
        );
        Task::batch([save, self.show_toast(text)])
    }

    fn current(&self) -> Option<&VotedGame> {
        self.rows
            .get(self.selected)
            .and_then(|line| line.game())
            .and_then(|id| self.games.get(&id))
    }

    fn next(&mut self) -> Task<Message> {
        self.selected = next_game(&self.rows, self.selected);
        self.reveal()
    }

    fn refresh(&mut self) -> Task<Message> {
        let selected = self.rows.get(self.selected).copied();
        self.rows = self.shown_rows();
        self.selected = kept_selection(&self.rows, selected, self.selected);

        self.reveal()
    }

    fn restart(&mut self) -> Task<Message> {
        self.rows = self.shown_rows();
        self.selected = first_game(&self.rows);
        self.viewport = Some(scrolled_to(self.viewport, 0.0));

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
            only_bets: self.only_bets,
            passing: self.passing.as_ref(),
        }
    }

    fn lists(&self) -> &Lists {
        self.save_menu.lists()
    }

    /// True when Bets only or a filter of the panel is on.
    fn is_filtered(&self) -> bool {
        self.only_bets || self.passing.is_some()
    }

    fn filters_summary(&self) -> Option<String> {
        filters_summary(&self.filters, self.only_bets, self.gamepalate.currency())
    }

    fn change_filters(&mut self, message: filters::Message) -> Task<Message> {
        let options = search_options(&self.filters, self.only_bets);
        match self.panel.update(&options, message) {
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
                        self.only_bets = options.only_bets;
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
            only_bets: self.only_bets,
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

    /// True when Hide owned is the only reason the segment has no games left.
    fn owned_hidden(&self) -> bool {
        self.hide_owned && count(&self.games, self.filter, Shown::default()) > 0
    }

    fn shown_rows(&self) -> Vec<Line> {
        let rows = rows(
            &self.games,
            self.filter,
            self.order,
            self.shown(),
            &self.search,
            self.matches.as_ref(),
        );
        let sections = self.filter == Filter::Saved && self.search.trim().is_empty();
        lines(&rows, &self.games, sections.then(|| self.lists()))
    }

    fn find_matches(&mut self) -> Task<Message> {
        self.matching += 1;
        let matching = self.matching;
        let gamepalate = Arc::clone(&self.gamepalate);

        Task::perform(async move { gamepalate.best_matches() }, move |matches| {
            Message::Matched(matching, matches)
        })
    }

    fn reveal(&mut self) -> Task<Message> {
        let Some((offset, height)) = self.viewport else {
            return Task::none();
        };

        let top = line_top(&self.rows, self.selected);
        match scroll_target(top, self.selected, offset, height) {
            Some(y) => {
                self.viewport = Some(scrolled_to(self.viewport, y));
                operation::scroll_to(
                    LIST,
                    AbsoluteOffset {
                        x: None,
                        y: Some(y),
                    },
                )
            }
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
        let ids: Vec<AppId> = self.rows.iter().filter_map(|line| line.game()).collect();
        let selected = self.rows[..self.selected.min(self.rows.len())]
            .iter()
            .filter(|line| line.game().is_some())
            .count();
        let end = (selected + showcase::AHEAD + 1).min(ids.len());
        let start = selected.saturating_sub(showcase::BEHIND).min(end);
        let upcoming = &ids[selected.min(end)..end];
        let nearby = &ids[start..end];

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
            Filter::Saved => vote.kind == VoteKind::WouldTry && vote.saved,
            Filter::WouldTry => vote.kind == VoteKind::WouldTry,
            Filter::NotForMe => vote.kind == VoteKind::NotForMe,
            Filter::All => true,
        }
    }

    /// The segment of a vote; a Later has none of its own, so it shows in All. A saved vote
    /// goes to Saved, the segment that tells most about it.
    fn of(vote: &Vote) -> Filter {
        [Filter::Saved, Filter::WouldTry, Filter::NotForMe]
            .into_iter()
            .find(|filter| filter.keeps(vote))
            .unwrap_or(Filter::All)
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
        Filter::Saved => "Saved",
        Filter::WouldTry => format::pick(Pick::Vote(VoteKind::WouldTry)),
        Filter::NotForMe => format::pick(Pick::Vote(VoteKind::NotForMe)),
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

/// Only All mixes the votes and shows them; in the others the segment says the vote.
fn vote_sign(filter: Filter, vote: &Vote) -> Option<Sign> {
    if filter != Filter::All {
        return None;
    }
    Some(match vote.kind {
        VoteKind::WouldTry => Sign::Up,
        VoteKind::NotForMe => Sign::Down,
        VoteKind::Later => Sign::Later,
    })
}

/// The small bookmark before a name: in Would try and in All, where the segment does not say it.
fn shows_saved(filter: Filter, vote: &Vote) -> bool {
    vote.saved && matches!(filter, Filter::WouldTry | Filter::All)
}

fn price_text(price: Option<Price>, currency: &Currency) -> String {
    price.map_or_else(|| "—".to_string(), |price| format::price(&price, currency))
}

/// The segment shown and the segment saved: one chosen from outside is not saved.
fn opening_chip(saved: &VotesSettings, chosen: Option<Filter>) -> (Filter, VotesShow) {
    (chosen.unwrap_or(saved.show.into()), saved.show)
}

impl From<VotesShow> for Filter {
    fn from(show: VotesShow) -> Filter {
        match show {
            VotesShow::Saved => Filter::Saved,
            VotesShow::WouldTry => Filter::WouldTry,
            VotesShow::NotForMe => Filter::NotForMe,
            VotesShow::All => Filter::All,
        }
    }
}

impl From<Filter> for VotesShow {
    fn from(filter: Filter) -> VotesShow {
        match filter {
            Filter::Saved => VotesShow::Saved,
            Filter::WouldTry => VotesShow::WouldTry,
            Filter::NotForMe => VotesShow::NotForMe,
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

/// The options the panel works on: the filters and Bets only.
fn search_options(filters: &FilterOptions, only_bets: bool) -> SearchOptions {
    SearchOptions {
        filters: filters.clone(),
        only_bets,
        ..SearchOptions::default()
    }
}

fn filters_summary(
    filters: &FilterOptions,
    only_bets: bool,
    currency: &Currency,
) -> Option<String> {
    format::filters(&search_options(filters, only_bets), currency)
}

/// The corner of the grid a segment sits in, counting along the rows.
fn segment_place(index: usize) -> style::Segment {
    match index {
        0 => style::Segment::TopLeft,
        1 => style::Segment::TopRight,
        2 => style::Segment::BottomLeft,
        _ => style::Segment::BottomRight,
    }
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

/// Which games of a segment the list shows, besides the segment itself.
#[derive(Clone, Copy, Default)]
struct Shown<'a> {
    hide_owned: bool,
    only_bets: bool,
    /// The games that pass the filters of the panel, when some are chosen.
    passing: Option<&'a BTreeSet<AppId>>,
}

impl Shown<'_> {
    fn keeps(self, game: &VotedGame) -> bool {
        !(self.hide_owned && game.owned)
            && (!self.only_bets || game.vote.bet)
            && self
                .passing
                .is_none_or(|passing| passing.contains(&game.id))
    }
}

fn count(games: &BTreeMap<AppId, VotedGame>, filter: Filter, shown: Shown) -> usize {
    games
        .values()
        .filter(|game| in_segment(filter, shown, game))
        .count()
}

fn in_segment(filter: Filter, shown: Shown, game: &VotedGame) -> bool {
    filter.keeps(&game.vote) && shown.keeps(game)
}

/// Where the selection goes once the lines change: on the same line if it is still there, then
/// on the same game in another section, otherwise on the game that took its place.
fn kept_selection(lines: &[Line], selected: Option<Line>, index: usize) -> usize {
    let game = selected.and_then(Line::game);
    selected
        .and_then(|selected| lines.iter().position(|&line| line == selected))
        .or_else(|| game.and_then(|id| lines.iter().position(|line| line.game() == Some(id))))
        .unwrap_or_else(|| {
            let index = index.min(lines.len().saturating_sub(1));
            let after = lines[index..].iter().position(|line| line.game().is_some());
            after
                .map(|after| index + after)
                .unwrap_or_else(|| previous_game(lines, index))
        })
}

/// The lines of the list. With `lists`, Saved shows a section for each list, in their order,
/// and then the games in none: a game in two lists has a line in each. A list without games
/// keeps its title, so it can be renamed or deleted; one whose games are all hidden does not.
fn lines(rows: &[AppId], games: &BTreeMap<AppId, VotedGame>, lists: Option<&Lists>) -> Vec<Line> {
    let Some(lists) = lists.filter(|lists| !lists.is_empty()) else {
        return rows
            .iter()
            .map(|&id| Line::Game(id, Section::Whole))
            .collect();
    };
    let lists_of = |id: &AppId| games.get(id).map(|game| &game.vote.lists);
    let mut lines = Vec::new();
    for &list in lists.keys() {
        let section = Section::List(list);
        let shown: Vec<Line> = rows
            .iter()
            .filter(|id| lists_of(id).is_some_and(|lists| lists.contains(&list)))
            .map(|&id| Line::Game(id, section))
            .collect();
        if !shown.is_empty() {
            lines.push(Line::Title(section));
            lines.extend(shown);
        } else if games_in(games, list) == 0 {
            lines.extend([Line::Title(section), Line::Empty(list)]);
        }
    }
    let no_list: Vec<Line> = rows
        .iter()
        .filter(|id| lists_of(id).is_some_and(BTreeSet::is_empty))
        .map(|&id| Line::Game(id, Section::NoList))
        .collect();
    if !no_list.is_empty() {
        lines.push(Line::Title(Section::NoList));
        lines.extend(no_list);
    }
    lines
}

/// The saved games in `list`, whatever Votes shows.
fn games_in(games: &BTreeMap<AppId, VotedGame>, list: ListId) -> usize {
    games
        .values()
        .filter(|game| game.vote.saved && game.vote.lists.contains(&list))
        .count()
}

fn section_title(name: &str, games: usize) -> String {
    format!("{name} · {}", format::count(games as u32))
}

fn first_game(lines: &[Line]) -> usize {
    lines
        .iter()
        .position(|line| line.game().is_some())
        .unwrap_or(0)
}

/// The game after `index`; the last one stays.
fn next_game(lines: &[Line], index: usize) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(index + 1)
        .find(|(_, line)| line.game().is_some())
        .map_or(index, |(next, _)| next)
}

/// The game before `index`; the first one stays.
fn previous_game(lines: &[Line], index: usize) -> usize {
    lines[..index.min(lines.len())]
        .iter()
        .rposition(|line| line.game().is_some())
        .unwrap_or(index)
}

/// The first line to bring into view with the line at `index`: the title right above it too.
fn line_top(lines: &[Line], index: usize) -> usize {
    match index.checked_sub(1).and_then(|above| lines.get(above)) {
        Some(Line::Title(_)) => index - 1,
        _ => index,
    }
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
                in_segment(filter, shown, game)
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
            Filter::All => "No votes match the filters".to_string(),
            Filter::Saved => "No saved games match the filters".to_string(),
            _ => format!("No {} votes match the filters", filter_label(filter)),
        }
    } else if owned_hidden {
        match filter {
            Filter::All => "You own all your voted games".to_string(),
            Filter::Saved => "You own all your saved games".to_string(),
            _ => format!("You own all your {} games", filter_label(filter)),
        }
    } else if filter == Filter::Saved {
        NO_SAVED.to_string()
    } else {
        format!("No {} votes", filter_label(filter))
    }
}

/// The rows from the first in sight to the last, both cut at `count`.
fn visible(offset: f32, height: f32, count: usize) -> Range<usize> {
    let start = (offset.max(0.0) / ROW_HEIGHT).floor() as usize;
    let end = ((offset.max(0.0) + height.max(0.0)) / ROW_HEIGHT).ceil() as usize;

    start.min(count)..end.min(count)
}

/// The rows a view builds: those in sight, and a screen more above and below.
fn shown(offset: f32, height: f32, count: usize) -> Range<usize> {
    let visible = visible(offset, height, count);
    let screen = (height.max(0.0) / ROW_HEIGHT).ceil() as usize;

    visible.start.saturating_sub(screen)..(visible.end + screen).min(count)
}

/// The viewport once the list is sent to `offset`: iced reports it only later, and until then
/// the rows built are those of the new place.
fn scrolled_to(viewport: Option<(f32, f32)>, offset: f32) -> (f32, f32) {
    (offset, viewport.map_or(FIRST_HEIGHT, |(_, height)| height))
}

fn rows_height(rows: usize) -> f32 {
    rows as f32 * ROW_HEIGHT
}

/// Where to scroll so the lines from `first` to `index` are in sight, if they are not.
fn scroll_target(first: usize, index: usize, offset: f32, height: f32) -> Option<f32> {
    let top = first as f32 * ROW_HEIGHT;
    let bottom = (index + 1) as f32 * ROW_HEIGHT;

    if top < offset {
        Some(top)
    } else if bottom > offset + height {
        Some(bottom - height)
    } else {
        None
    }
}

/// A game you have cannot be saved, and S leaves it alone.
fn may_save(game: &VotedGame) -> bool {
    !game.owned
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
            | Message::Save
            | Message::Open(_)
            | Message::FocusSearch
            | Message::ToggleFilters
    )
}

/// A vote moves on: the trailer stops even on the last game.
fn stops_the_trailer(message: &Message) -> bool {
    matches!(message, Message::Vote(_) | Message::Played | Message::Save)
}

fn acts_on_the_page(message: &Message) -> bool {
    matches!(
        message,
        Message::Previous
            | Message::Next
            | Message::Vote(_)
            | Message::Played
            | Message::Save
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
                    | showcase::Message::PlayOrPause
                    | showcase::Message::SwitchMute
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
        Key::Character(character) if character.eq_ignore_ascii_case("p") => {
            Some(Message::Showcase(showcase::Message::PlayOrPause))
        }
        Key::Character(character) if character.eq_ignore_ascii_case("m") => {
            Some(Message::Showcase(showcase::Message::SwitchMute))
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
        Key::Character(character) if game::save_key(&character) => Some(Message::Save),
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
        only_bets: false,
        passing: None,
    };

    const BETS_ONLY: Shown = Shown {
        hide_owned: false,
        only_bets: true,
        passing: None,
    };

    fn passing_only(passing: &BTreeSet<AppId>) -> Shown<'_> {
        Shown {
            hide_owned: false,
            only_bets: false,
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
            &search_options(filters, false),
        )
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
    fn p_plays_or_pauses_the_trailer_and_m_mutes_it() {
        assert!(matches!(
            key_message(pressed(Key::Character("p".into()))),
            Some(Message::Showcase(showcase::Message::PlayOrPause))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("M".into()))),
            Some(Message::Showcase(showcase::Message::SwitchMute))
        ));
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

    fn whole(numbers: &[u32]) -> Vec<Line> {
        lines(&ids(numbers), &games(), None)
    }

    #[test]
    fn the_selection_stays_on_its_game_when_the_rows_change() {
        let four = Some(game(4, Section::Whole));

        assert_eq!(kept_selection(&whole(&[1, 4]), four, 2), 1);
    }

    #[test]
    fn a_selected_game_that_left_the_list_gives_its_place_to_the_next() {
        let one = Some(game(1, Section::Whole));
        let four = Some(game(4, Section::Whole));

        assert_eq!(kept_selection(&whole(&[2, 4]), one, 1), 1);
        assert_eq!(kept_selection(&whole(&[2, 1]), four, 2), 1);
        assert_eq!(kept_selection(&[], four, 2), 0);
        assert_eq!(kept_selection(&whole(&[2, 1]), None, 0), 0);
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
    fn bets_only_keeps_only_the_bets_of_the_segment() {
        let games = games();

        assert_eq!(
            rows(&games, Filter::All, Order::Newest, BETS_ONLY, "", None),
            ids(&[5, 2])
        );
        assert_eq!(
            rows(&games, Filter::WouldTry, Order::Newest, BETS_ONLY, "", None),
            ids(&[2])
        );
        assert_eq!(count(&games, Filter::NotForMe, BETS_ONLY), 1);
    }

    #[test]
    fn the_search_looks_among_all_the_votes() {
        let rows = rows(
            &games(),
            Filter::NotForMe,
            Order::Newest,
            Shown::default(),
            " sat ",
            None,
        );

        assert_eq!(rows, ids(&[2]));
    }

    #[test]
    fn the_segments_sit_in_the_four_corners_of_the_grid() {
        assert_eq!(FILTERS.len(), 4);
        assert_eq!(segment_place(0), style::Segment::TopLeft);
        assert_eq!(segment_place(1), style::Segment::TopRight);
        assert_eq!(segment_place(2), style::Segment::BottomLeft);
        assert_eq!(segment_place(3), style::Segment::BottomRight);
        assert_eq!(FILTERS[0], Filter::Saved);
    }

    fn saved(mut game: VotedGame) -> VotedGame {
        game.vote.saved = true;
        game
    }

    fn with_saved() -> BTreeMap<AppId, VotedGame> {
        let mut games = games();
        let mut owned = saved(voted(6, "Hades", VoteKind::WouldTry, "2026-10-05", false));
        owned.owned = true;
        games.extend(by_id(vec![
            saved(voted(7, "Tunic", VoteKind::WouldTry, "2026-10-25", true)),
            saved(voted(8, "Celeste", VoteKind::WouldTry, "2026-10-30", false)),
            owned,
        ]));
        games.get_mut(&AppId(2)).unwrap().vote.saved = true;
        games
    }

    #[test]
    fn the_saved_segment_keeps_the_would_try_votes_with_the_save() {
        let games = with_saved();

        assert_eq!(
            rows(
                &games,
                Filter::Saved,
                Order::Newest,
                Shown::default(),
                "",
                None
            ),
            ids(&[8, 7, 2, 6])
        );
        assert_eq!(count(&games, Filter::Saved, Shown::default()), 4);
        assert_eq!(count(&games, Filter::WouldTry, Shown::default()), 6);
        assert_eq!(count(&games, Filter::All, Shown::default()), 8);
    }

    const CO_OP: ListId = ListId(2);
    const ON_SALE: ListId = ListId(5);

    fn some_lists() -> Lists {
        Lists::from([
            (CO_OP, "Co-op".to_string()),
            (ON_SALE, "On sale".to_string()),
        ])
    }

    /// Tunic and Hades in Co-op, Tunic in On sale too; Satisfactory and Celeste in no list.
    fn with_lists() -> BTreeMap<AppId, VotedGame> {
        let mut games = with_saved();
        let mut put = |id: u32, lists: &[ListId]| {
            games.get_mut(&AppId(id)).unwrap().vote.lists = lists.iter().copied().collect();
        };
        put(7, &[CO_OP, ON_SALE]);
        put(6, &[CO_OP]);
        games
    }

    fn saved_lines(games: &BTreeMap<AppId, VotedGame>, lists: &Lists) -> Vec<Line> {
        let rows = rows(
            games,
            Filter::Saved,
            Order::Newest,
            Shown::default(),
            "",
            None,
        );
        lines(&rows, games, Some(lists))
    }

    fn game(id: u32, section: Section) -> Line {
        Line::Game(AppId(id), section)
    }

    #[test]
    fn saved_shows_a_section_for_each_list_and_no_list_last() {
        use Section::{List, NoList};

        assert_eq!(
            saved_lines(&with_lists(), &some_lists()),
            vec![
                Line::Title(List(CO_OP)),
                game(7, List(CO_OP)),
                game(6, List(CO_OP)),
                Line::Title(List(ON_SALE)),
                game(7, List(ON_SALE)),
                Line::Title(NoList),
                game(8, NoList),
                game(2, NoList),
            ]
        );
    }

    #[test]
    fn a_list_without_games_keeps_its_title() {
        let mut lists = some_lists();
        lists.insert(ListId(9), "Later".to_string());
        let lines = saved_lines(&with_lists(), &lists);

        assert_eq!(
            lines[lines.len() - 5..lines.len() - 3],
            [
                Line::Title(Section::List(ListId(9))),
                Line::Empty(ListId(9))
            ]
        );
    }

    #[test]
    fn a_list_whose_games_are_all_hidden_leaves_no_section() {
        let games = with_lists();
        let rows = ids(&[8, 6]);

        assert_eq!(
            lines(&rows, &games, Some(&some_lists())),
            vec![
                Line::Title(Section::List(CO_OP)),
                game(6, Section::List(CO_OP)),
                Line::Title(Section::NoList),
                game(8, Section::NoList),
            ]
        );
    }

    #[test]
    fn without_lists_there_are_no_sections() {
        let games = with_lists();
        let rows = ids(&[8, 7]);
        let whole = vec![game(8, Section::Whole), game(7, Section::Whole)];

        assert_eq!(lines(&rows, &games, None), whole);
        assert_eq!(lines(&rows, &games, Some(&Lists::new())), whole);
    }

    #[test]
    fn the_title_counts_the_games_of_its_list() {
        let games = with_lists();

        assert_eq!(games_in(&games, CO_OP), 2);
        assert_eq!(games_in(&games, ON_SALE), 1);
        assert_eq!(section_title("Co-op", 2), "Co-op · 2");
    }

    #[test]
    fn the_arrows_skip_the_titles() {
        let lines = saved_lines(&with_lists(), &some_lists());

        assert_eq!(first_game(&lines), 1);
        assert_eq!(next_game(&lines, 2), 4);
        assert_eq!(previous_game(&lines, 4), 2);
        assert_eq!(next_game(&lines, 7), 7);
        assert_eq!(previous_game(&lines, 1), 1);
    }

    #[test]
    fn the_first_game_of_a_section_brings_its_title_into_view() {
        let lines = saved_lines(&with_lists(), &some_lists());

        assert_eq!(line_top(&lines, 1), 0);
        assert_eq!(line_top(&lines, 2), 2);
        assert_eq!(line_top(&lines, 6), 5);
    }

    #[test]
    fn the_selection_follows_its_game_into_another_section() {
        use Section::{List, NoList};
        let lines = saved_lines(&with_lists(), &some_lists());

        assert_eq!(kept_selection(&lines, Some(game(7, List(ON_SALE))), 0), 4);
        // Taken out of its lists, the game is in No list
        assert_eq!(kept_selection(&lines, Some(game(6, NoList)), 0), 2);
    }

    #[test]
    fn a_vote_stops_the_trailer() {
        assert!(stops_the_trailer(&Message::Vote(Pick::Played)));
        assert!(stops_the_trailer(&Message::Played));
        assert!(stops_the_trailer(&Message::Save));
        assert!(!stops_the_trailer(&Message::Next));
    }

    #[test]
    fn the_saved_count_follows_hide_owned_bets_only_and_the_filters() {
        let games = with_saved();
        let passing = set(&[2, 8]);

        assert_eq!(count(&games, Filter::Saved, HIDE_OWNED), 3);
        assert_eq!(count(&games, Filter::Saved, BETS_ONLY), 2);
        assert_eq!(count(&games, Filter::Saved, passing_only(&passing)), 2);
        assert_eq!(
            rows(&games, Filter::Saved, Order::Newest, HIDE_OWNED, "", None),
            ids(&[8, 7, 2])
        );
        assert_eq!(
            rows(&games, Filter::Saved, Order::Newest, BETS_ONLY, "", None),
            ids(&[7, 2])
        );
        assert_eq!(
            rows(
                &games,
                Filter::Saved,
                Order::Price,
                passing_only(&passing),
                "",
                None
            ),
            ids(&[2, 8])
        );
    }

    #[test]
    fn a_game_that_loses_the_save_leaves_the_saved_segment() {
        let mut games = with_saved();
        games.get_mut(&AppId(7)).unwrap().vote.saved = false;
        let rows = rows(
            &games,
            Filter::Saved,
            Order::Newest,
            Shown::default(),
            "",
            None,
        );

        assert_eq!(rows, ids(&[8, 2, 6]));
        let lines = lines(&rows, &games, None);
        assert_eq!(kept_selection(&lines, Some(game(7, Section::Whole)), 1), 1);
    }

    #[test]
    fn a_saved_vote_opens_on_the_saved_segment() {
        let mut vote = Vote::new(VoteKind::WouldTry, "2026-11-16");
        vote.saved = true;

        assert_eq!(Filter::of(&vote), Filter::Saved);
        assert_eq!(
            Filter::of(&Vote::new(VoteKind::WouldTry, "2026-11-16")),
            Filter::WouldTry
        );
    }

    #[test]
    fn the_small_save_shows_only_in_would_try_and_in_all() {
        let mut vote = Vote::new(VoteKind::WouldTry, "2026-11-16");
        let plain = vote.clone();
        vote.saved = true;

        assert!(shows_saved(Filter::WouldTry, &vote));
        assert!(shows_saved(Filter::All, &vote));
        assert!(!shows_saved(Filter::Saved, &vote));
        assert!(!shows_saved(Filter::NotForMe, &vote));
        assert!(!shows_saved(Filter::WouldTry, &plain));
        assert!(!shows_saved(Filter::All, &plain));
    }

    #[test]
    fn the_saved_segment_says_why_it_is_empty() {
        assert_eq!(empty_text(false, Filter::Saved, false, false, ""), NO_SAVED);
        assert_eq!(
            empty_text(false, Filter::Saved, true, false, ""),
            "No saved games match the filters"
        );
        assert_eq!(
            empty_text(false, Filter::Saved, false, true, ""),
            "You own all your saved games"
        );
        assert_eq!(
            empty_text(false, Filter::Saved, false, false, "hades"),
            "No votes match \"hades\""
        );
        assert_eq!(empty_text(true, Filter::Saved, false, false, ""), NO_VOTES);
    }

    #[test]
    fn a_later_has_no_filter_and_shows_only_in_all() {
        assert_eq!(
            Filter::of(&Vote::new(VoteKind::Later, "2026-10-20")),
            Filter::All
        );
        assert!([Filter::WouldTry, Filter::NotForMe].iter().all(|filter| {
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
        assert_eq!(count(&games, Filter::NotForMe, Shown::default()), 1);
        assert_eq!(count(&games, Filter::All, Shown::default()), 5);
    }

    #[test]
    fn a_vote_opens_on_its_own_chip() {
        assert_eq!(
            Filter::of(&Vote::new(VoteKind::WouldTry, "2026-11-13")),
            Filter::WouldTry
        );
        assert_eq!(
            Filter::of(&Vote::new(VoteKind::NotForMe, "2026-11-13")),
            Filter::NotForMe
        );
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
        let not_for_me = Filter::NotForMe;

        assert_eq!(empty_text(true, not_for_me, false, false, ""), NO_VOTES);
        assert_eq!(
            empty_text(false, not_for_me, false, false, ""),
            "No Not for me votes"
        );
        assert_eq!(
            empty_text(false, Filter::WouldTry, false, false, ""),
            "No Would try votes"
        );
        assert_eq!(
            empty_text(false, not_for_me, false, false, " hades "),
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
            rows(&games, Filter::All, Order::Newest, shown, "", None),
            ids(&[3, 5, 1])
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
            empty_text(false, Filter::NotForMe, true, false, ""),
            "No Not for me votes match the filters"
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
    fn the_panel_of_votes_has_bets_only_and_starts_from_no_limit() {
        let mut panel = votes_panel(&FilterOptions::default());
        let none = search_options(&FilterOptions::default(), false);

        panel.update(&none, filters::Message::OnlyBets(true));
        panel.update(&none, filters::Message::OnSale(true));
        let Action::Search(changed) = panel.update(&none, filters::Message::Done) else {
            panic!("expected new filters");
        };

        assert!(changed.only_bets);
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
        let options = search_options(&filters, false);
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

        assert_eq!(
            filters_summary(&FilterOptions::default(), false, &currency),
            None
        );
        assert_eq!(
            filters_summary(&filters, false, &currency).as_deref(),
            Some("On sale · ≤ 20,00€ · ≥ 85% positive · Tags: Roguelike")
        );
        assert_eq!(
            filters_summary(&FilterOptions::default(), true, &currency).as_deref(),
            Some("Bets only")
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
            rows(&games, Filter::All, Order::Newest, HIDE_OWNED, "", None),
            ids(&[3, 2])
        );
        assert_eq!(count(&games, would_try, HIDE_OWNED), 1);
        assert_eq!(count(&games, Filter::NotForMe, HIDE_OWNED), 0);
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
            only_bets: false,
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
            empty_text(false, Filter::NotForMe, false, true, ""),
            "You own all your Not for me games"
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
        assert_eq!(scroll_target(2, 2, 0.0, 320.0), None);
        assert_eq!(scroll_target(2, 2, 100.0, 320.0), Some(64.0));
        assert_eq!(scroll_target(12, 12, 0.0, 320.0), Some(96.0));
        // With the title above it, the line needs one row more
        assert_eq!(scroll_target(3, 4, 100.0, 320.0), Some(96.0));
    }

    #[test]
    fn builds_only_the_rows_near_the_window() {
        // 320 px is ten rows: a screen of ten more above and below.
        assert_eq!(shown(0.0, 320.0, 689), 0..20);
        assert_eq!(shown(3_200.0, 320.0, 689), 90..120);
        assert_eq!(shown(3_216.0, 320.0, 689), 90..121);
        assert_eq!(shown(rows_height(689) - 320.0, 320.0, 689), 669..689);
        assert_eq!(shown(0.0, 320.0, 4), 0..4);
        assert_eq!(shown(0.0, 320.0, 0), 0..0);
    }

    #[test]
    fn a_list_sent_to_the_top_builds_the_top_rows_at_once() {
        let (offset, height) = scrolled_to(Some((3_216.0, 320.0)), 0.0);
        assert_eq!((offset, height), (0.0, 320.0));
        assert_eq!(shown(offset, height, 689).start, 0);
        assert_eq!(scrolled_to(None, 64.0), (64.0, FIRST_HEIGHT));
    }

    #[test]
    fn the_spaces_and_the_rows_make_the_whole_list() {
        for (offset, count) in [(0.0, 689), (3_216.0, 689), (21_728.0, 689), (0.0, 4)] {
            let range = shown(offset, 320.0, count);
            let total = rows_height(range.start)
                + rows_height(range.len())
                + rows_height(count - range.end);

            assert_eq!(total, rows_height(count));
        }
        assert_eq!(rows_height(689), 689.0 * ROW_HEIGHT);
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
            key_message(pressed(Key::Character("2".into()))),
            Some(Message::Vote(Pick::Vote(VoteKind::NotForMe)))
        ));
        assert!(matches!(
            key_message(pressed(Key::Character("3".into()))),
            Some(Message::Played)
        ));
        assert!(key_message(pressed(Key::Character("4".into()))).is_none());
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
    fn s_opens_the_save_menu_in_either_case_and_ctrl_s_does_nothing() {
        for key in ["s", "S"] {
            assert!(matches!(
                key_message(pressed(Key::Character(key.into()))),
                Some(Message::Save)
            ));
        }
        assert!(key_message(pressed_with(Key::Character("s".into()), Modifiers::CTRL)).is_none());
    }

    #[test]
    fn a_game_you_have_cannot_get_a_save() {
        let mut game = voted(1, "Hades", VoteKind::WouldTry, "2026-10-01", false);
        assert!(may_save(&game));
        game.owned = true;
        assert!(!may_save(&game));
    }

    #[test]
    fn an_open_popup_stops_the_save() {
        assert!(stopped_by_popup(&Message::Save));
    }

    #[test]
    fn the_open_filters_stop_the_keys_of_the_page() {
        let number = key_message(pressed(Key::Character("1".into()))).unwrap();

        assert!(acts_on_the_page(&number));
        assert!(acts_on_the_page(&Message::Save));
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
    fn the_vote_shows_beside_the_name_only_in_all() {
        let would_try = Vote::new(VoteKind::WouldTry, "2026-10-01");
        let not_for_me = Vote::new(VoteKind::NotForMe, "2026-10-01");
        let later = Vote::new(VoteKind::Later, "2026-10-01");

        assert_eq!(vote_sign(Filter::All, &would_try), Some(Sign::Up));
        assert_eq!(vote_sign(Filter::All, &not_for_me), Some(Sign::Down));
        assert_eq!(vote_sign(Filter::All, &later), Some(Sign::Later));
        for filter in [Filter::WouldTry, Filter::NotForMe] {
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
        for filter in FILTERS {
            assert_eq!(Filter::from(VotesShow::from(filter)), filter);
        }
        for order in ORDERS {
            assert_eq!(Order::from(VotesOrder::from(order)), order);
        }
    }

    #[test]
    fn votes_open_on_the_saved_chip() {
        let saved = VotesSettings {
            show: VotesShow::All,
            ..VotesSettings::default()
        };

        assert_eq!(opening_chip(&saved, None), (Filter::All, VotesShow::All));
        assert_eq!(
            opening_chip(&VotesSettings::default(), None),
            (Filter::WouldTry, VotesShow::WouldTry)
        );
    }

    #[test]
    fn a_chip_chosen_from_outside_leaves_the_saved_one() {
        let saved = VotesSettings {
            show: VotesShow::NotForMe,
            ..VotesSettings::default()
        };

        assert_eq!(
            opening_chip(&saved, Some(Filter::WouldTry)),
            (Filter::WouldTry, VotesShow::NotForMe)
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
