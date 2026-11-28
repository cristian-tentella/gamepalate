mod about;
mod deck;
mod descriptions;
pub(crate) mod images;
mod loops;
mod media;
mod pages;
mod review_sheet;
mod reviews;
mod sight;
mod tags;
mod trailers;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use gamepalate_app::{
    AppId, Block, GameDetails, GameTagsView, Gamepalate, TrailerPlace,
    reviews::{ReviewPage, ReviewQuery},
};
use iced::{
    Center, ContentFit, Element, Fill, Shrink, Subscription, Task, mouse, padding,
    widget::{
        Id, button, column, container, image, mouse_area, opaque, operation, row, scrollable,
        scrollable::{AbsoluteOffset, RelativeOffset},
        space, stack, text, tooltip,
    },
    window,
};

use self::{
    descriptions::{Description, Descriptions},
    images::{Images, Picture},
    loops::Loops,
    media::Item,
    pages::{Page, Pages},
    reviews::{Request, ReviewLists},
    trailers::{Due, Trailers},
};
use super::game;
use crate::{
    app::{
        PAGE_TOP,
        backdrop::{self, Blurred},
    },
    font, format, style, video,
};
use media::next_index;
pub use media::{SCREENSHOT_WIDTH, max_screenshot_width, screenshot_width};
pub(super) use tags::tags_body;

pub const AHEAD: usize = 5;
pub const BEHIND: usize = 5;
const DETAILS_AHEAD: usize = 3;
const SHEET: Id = Id::new("showcase-sheet");
pub(super) const SCROLL_STEP: f32 = 60.0;
const PLAY: char = '\u{E037}';
const MEASURE: f32 = 680.0;
const BODY_SIZE: f32 = 16.0;
const SHEET_INSET: f32 = 40.0;
const SHEET_MARGIN: f32 = 32.0;
// About a space at the size of the notes, between two tags.
const TAG_SPACING: f32 = 4.0;
// Between two lines of small text that go together, and the lines of the tags when they wrap.
pub const LINE_SPACING: f32 = 4.0;
// Between the keys of the sheets and the features.
const MORE_SPACING: f32 = 12.0;
const CLOSE_SIZE: f32 = 32.0;
const TITLE_SIZE: f32 = 16.0;
const CLOSE: char = '\u{E5CD}';
const CLOSED: char = '\u{E5CC}';
const OPENED: char = '\u{E5CF}';
/// The pause in typing the hours before the reviews are asked again.
const HOURS_PAUSE: Duration = Duration::from_millis(500);

pub struct Showcase {
    gamepalate: Arc<Gamepalate>,
    images: Images,
    pages: Pages,
    descriptions: Descriptions,
    reviews: ReviewLists,
    spoilers: HashSet<(String, usize)>,
    expanded: HashSet<String>,
    hours_typed: String,
    has_reviews: bool,
    extras: Images,
    /// The index of the item of the gallery shown, in the order `place` gives.
    item: usize,
    place: TrailerPlace,
    last_screenshot: Option<String>,
    shown: Option<AppId>,
    browsing: bool,
    zoomed: bool,
    popups: Popups,
    tags: Option<GameTagsView>,
    features_open: bool,
    hovered: bool,
    originals: HashMap<String, Picture>,
    thumbnails: HashMap<String, Picture>,
    trailers: Trailers,
    loops: Loops,
    /// The rows of thumbnails under the gallery and under the zoom, with how far each is scrolled.
    row_ids: [Id; 2],
    row_scroll: [Option<scrollable::Viewport>; 2],
    /// A new Game came: the rows go back to their start, wherever they were scrolled.
    rewind_rows: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Original,
    Thumbnail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Popup {
    About,
    Tags,
    Reviews,
}

impl Popup {
    fn label(self) -> &'static str {
        match self {
            Popup::About => "About this game",
            Popup::Tags => "Tags",
            Popup::Reviews => "Reviews",
        }
    }
}

#[derive(Debug, Default)]
struct Popups {
    open: Option<Popup>,
    /// Waiting for the blurred picture of the page, taken before the sheet covers it.
    opening: Option<Popup>,
    backdrop: Option<image::Handle>,
    all_tags: bool,
}

impl Popups {
    /// True when the sheet needs a picture of the page first. Going from one sheet to another
    /// keeps the picture of the first.
    fn toggle(&mut self, popup: Popup, allowed: bool) -> bool {
        if !allowed {
            return false;
        }
        if self.open.is_some() {
            if self.open == Some(popup) {
                self.close();
            } else {
                self.open = Some(popup);
            }
            return false;
        }
        if self.opening == Some(popup) {
            self.opening = None;
            return false;
        }
        let asked = self.opening.is_some();
        self.opening = Some(popup);
        !asked
    }

    /// The picture has come, or there is none: the sheet waiting for it shows up.
    fn pictured(&mut self, backdrop: Option<image::Handle>) {
        if let Some(popup) = self.opening.take() {
            self.open = Some(popup);
            self.backdrop = backdrop;
        }
    }

    fn close(&mut self) {
        self.open = None;
        self.opening = None;
        self.backdrop = None;
    }

    fn is_shown_or_coming(&self) -> bool {
        self.open.is_some() || self.opening.is_some()
    }

    fn is(&self, popup: Popup) -> bool {
        self.open == Some(popup)
    }
}

enum Shown<'a> {
    Wanted,
    Thumbnail,
    Last(&'a String),
}

#[derive(Clone, Debug)]
pub enum Message {
    ImageLoaded(String, Option<image::Handle>),
    ImageAllocated(String, Option<image::Allocation>),
    VersionLoaded(Version, String, Option<image::Handle>),
    VersionAllocated(Version, String, Option<image::Allocation>),
    DetailsLoaded(AppId, Result<Option<Arc<GameDetails>>, String>),
    DescriptionLoaded(AppId, Result<Option<Arc<Vec<Block>>>, String>),
    ReviewsLoaded(Request, Result<Arc<ReviewPage>, String>),
    WindowFound(Option<window::Id>),
    Screenshot(window::Screenshot),
    Blurred(Option<Arc<Blurred>>),
    ExtraLoaded(String, Option<image::Handle>),
    ExtraAllocated(String, Option<image::Allocation>),
    OpenLink(String),
    PreviousScreenshot,
    NextScreenshot,
    ShowScreenshot(usize),
    RetryDetails,
    Zoom,
    CloseZoom,
    ToggleAbout,
    ToggleTags,
    ToggleAllTags,
    ToggleReviews,
    MoreReviews,
    FilterReviews(ReviewQuery),
    TypeHours(String),
    ApplyHours(String),
    RetryReviews,
    ShowSpoiler(String, usize),
    ExpandReview(String),
    ClosePopup,
    ScrollUp,
    ScrollDown,
    Close,
    ToggleFeatures,
    Hover(bool),
    RowScrolled(media::Row, scrollable::Viewport),
    PlayTrailer,
    PlayOrPause,
    SwitchMute,
    Trailer(video::Message),
    Measured(video::Surface),
    /// A video of the description, by the index of its block.
    Loop(usize, video::Message),
    /// Whether more than a quarter of the height of a video of the description shows.
    LoopSight(usize, bool),
    LoopMeasured(video::Surface),
}

impl Showcase {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> Showcase {
        Showcase {
            gamepalate: Arc::clone(gamepalate),
            images: Images::default(),
            pages: Pages::default(),
            descriptions: Descriptions::default(),
            reviews: ReviewLists::default(),
            spoilers: HashSet::new(),
            expanded: HashSet::new(),
            hours_typed: String::new(),
            has_reviews: false,
            extras: Images::default(),
            item: 0,
            place: gamepalate.gui_settings().trailers,
            last_screenshot: None,
            shown: None,
            browsing: false,
            zoomed: false,
            popups: Popups::default(),
            tags: None,
            features_open: false,
            hovered: false,
            originals: HashMap::new(),
            thumbnails: HashMap::new(),
            trailers: Trailers::default(),
            loops: Loops::default(),
            row_ids: [Id::unique(), Id::unique()],
            row_scroll: [None; 2],
            rewind_rows: false,
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let before = (self.item, self.zoomed);
        match message {
            Message::ScrollUp | Message::ScrollDown if self.popups.open.is_none() => {}
            Message::ScrollUp => return scroll_sheet(-SCROLL_STEP),
            Message::ScrollDown => return scroll_sheet(SCROLL_STEP),
            Message::ImageLoaded(url, None) => self.images.loaded(url, None),
            Message::ImageLoaded(url, Some(handle)) => {
                return image::allocate(handle)
                    .map(move |allocation| Message::ImageAllocated(url.clone(), allocation.ok()));
            }
            Message::ImageAllocated(url, allocation) => self.images.loaded(url, allocation),
            Message::VersionLoaded(version, url, Some(handle)) => {
                return image::allocate(handle).map(move |allocation| {
                    Message::VersionAllocated(version, url.clone(), allocation.ok())
                });
            }
            Message::VersionLoaded(version, url, None) => self.version_loaded(version, &url, None),
            Message::VersionAllocated(version, url, allocation) => {
                self.version_loaded(version, &url, allocation);
            }
            Message::DetailsLoaded(id, result) => self.pages.loaded(id, result),
            Message::DescriptionLoaded(id, result) => self.descriptions.loaded(id, result),
            Message::ReviewsLoaded(request, result) => self.reviews.loaded(request, result),
            Message::WindowFound(window) => {
                match window.filter(|_| self.popups.opening.is_some()) {
                    Some(window) => return window::screenshot(window).map(Message::Screenshot),
                    None => self.popups.pictured(None),
                }
            }
            Message::Screenshot(screenshot) => {
                if self.popups.opening.is_some() {
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                backdrop::blur(&screenshot, PAGE_TOP)
                            })
                            .await
                        },
                        |blurred| Message::Blurred(blurred.ok().map(Arc::new)),
                    );
                }
            }
            Message::Blurred(blurred) => self
                .popups
                .pictured(blurred.map(|blurred| blurred.handle.clone())),
            Message::ExtraLoaded(url, None) => self.extras.loaded(url, None),
            Message::ExtraLoaded(url, Some(handle)) => {
                return image::allocate(handle)
                    .map(move |allocation| Message::ExtraAllocated(url.clone(), allocation.ok()));
            }
            Message::ExtraAllocated(url, allocation) => self.extras.loaded(url, allocation),
            Message::OpenLink(url) => super::open(url),
            Message::PreviousScreenshot | Message::NextScreenshot if self.popups.open.is_some() => {
            }
            Message::PreviousScreenshot => self.show_screenshot(self.item.saturating_sub(1)),
            Message::NextScreenshot => {
                self.show_screenshot(next_index(self.item, self.item_count()));
            }
            Message::ShowScreenshot(index) => self.show_screenshot(index),
            Message::RetryDetails => {
                if let Some(id) = self.shown {
                    self.pages.retry(id);
                }
            }
            Message::Zoom => self.zoomed = self.current_screenshot().is_some(),
            Message::CloseZoom => self.zoomed = false,
            Message::ToggleAbout => return self.toggle(Popup::About, self.can_open_popup()),
            Message::ToggleTags => return self.toggle(Popup::Tags, self.can_open_popup()),
            Message::ToggleAllTags => self.popups.all_tags = !self.popups.all_tags,
            Message::ToggleReviews => {
                return self.toggle(Popup::Reviews, self.can_open_popup() && self.has_reviews);
            }
            Message::MoreReviews => return self.more_reviews(),
            Message::RetryReviews => return self.retry_reviews(),
            Message::FilterReviews(query) => return self.filter_reviews(query),
            Message::TypeHours(typed) => {
                self.hours_typed = review_sheet::hours_typed(&typed);
                let typed = self.hours_typed.clone();
                return Task::perform(tokio::time::sleep(HOURS_PAUSE), move |()| {
                    Message::ApplyHours(typed.clone())
                });
            }
            Message::ApplyHours(typed) if typed == self.hours_typed => {
                let query = ReviewQuery {
                    min_hours: review_sheet::min_hours(&typed),
                    ..self.reviews.query()
                };
                return self.filter_reviews(query);
            }
            Message::ApplyHours(_) => {}
            Message::ShowSpoiler(id, index) => {
                self.spoilers.insert((id, index));
            }
            Message::ExpandReview(id) => {
                self.expanded.insert(id);
            }
            Message::ClosePopup => self.popups.close(),
            Message::Close => {
                self.zoomed = false;
                self.popups.close();
            }
            Message::ToggleFeatures => self.features_open = !self.features_open,
            Message::Hover(hovered) => self.hovered = hovered,
            Message::RowScrolled(row, viewport) => self.row_scroll[row as usize] = Some(viewport),
            Message::PlayTrailer => self.play_trailer(),
            Message::PlayOrPause => self.play_or_pause(),
            Message::SwitchMute => {
                if let (false, Some(Item::Trailer(index))) = (self.is_open(), self.current_item()) {
                    self.trailers.switch_mute(index);
                }
            }
            Message::Trailer(message) => self.trailers.update(message),
            Message::Measured(surface) => self.trailers.measured(surface),
            Message::Loop(index, message) => self.loops.update(index, message),
            Message::LoopSight(index, in_sight) => self.loops.sight(index, in_sight),
            Message::LoopMeasured(surface) => self.loops.measured(surface.software),
        }
        if self.is_open() {
            self.trailers.stop();
        }
        self.sync_loops();
        self.autoplay();

        let reveal = if (self.item, self.zoomed) == before {
            Task::none()
        } else {
            self.reveal()
        };
        Task::batch([reveal, self.load_shown()])
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            self.trailers.subscription().map(Message::Trailer),
            self.loops
                .subscription()
                .map(|(index, message)| Message::Loop(index, message)),
        ])
    }

    /// Stops the trailer, for what covers the gallery or takes the place of the screen.
    pub fn stop_trailer(&mut self) {
        self.trailers.stop();
    }

    /// The search or a game over the screen, or the screen set aside: the videos of the About
    /// sheet stop, and play again once nothing covers them.
    pub fn cover(&mut self, covered: bool) {
        self.loops.cover(covered);
        self.sync_loops();
    }

    /// The videos of the description play only in the About sheet of the game shown.
    fn sync_loops(&mut self) {
        let blocks = match self.shown.filter(|_| self.popups.is(Popup::About)) {
            Some(id) => match self.descriptions.get(id) {
                Some(Description::Ready(blocks)) => Some(blocks.as_slice()),
                _ => None,
            },
            None => None,
        };
        self.loops.sync(blocks);
    }

    fn play_trailer(&mut self) {
        let Some(Item::Trailer(index)) = self.current_item() else {
            return;
        };
        if let Some(manifest_url) = self
            .trailer(index)
            .map(|trailer| trailer.manifest_url.clone())
        {
            self.trailers.play(index, &manifest_url);
        }
    }

    /// `P` does nothing under a sheet or the zoom.
    fn play_or_pause(&mut self) {
        if self.is_open() {
            return;
        }
        let Some(Item::Trailer(index)) = self.current_item() else {
            return;
        };
        if let Some(manifest_url) = self
            .trailer(index)
            .map(|trailer| trailer.manifest_url.clone())
        {
            self.trailers.play_or_pause(index, &manifest_url);
        }
    }

    /// The armed trailer starts by itself, when the gallery shows it.
    fn autoplay(&mut self) {
        let Some(armed) = self.trailers.armed() else {
            return;
        };
        let page = self.shown.and_then(|id| self.pages.get(id));
        let due = match page {
            _ if self.is_open() => Due::Never,
            Some(Page::Ready(details)) => match media::item_of(details, self.place, self.item) {
                Some(Item::Trailer(index)) if index == armed => details
                    .store
                    .trailers
                    .get(index)
                    .map_or(Due::Never, |trailer| {
                        Due::Play(index, trailer.manifest_url.clone())
                    }),
                _ => Due::Never,
            },
            Some(Page::Loading(_)) | None if self.shown.is_some() => Due::Wait,
            _ => Due::Never,
        };
        self.trailers.autoplay(due);
    }

    fn load_shown(&mut self) -> Task<Message> {
        Task::batch([
            self.load_versions(),
            self.load_description(),
            self.load_extras(),
            self.load_reviews(),
        ])
    }

    /// The sheet shows up once the page behind it is blurred.
    fn toggle(&mut self, popup: Popup, allowed: bool) -> Task<Message> {
        let picture = if self.popups.toggle(popup, allowed) {
            window::latest().map(Message::WindowFound)
        } else {
            Task::none()
        };

        Task::batch([picture, self.load_shown()])
    }

    fn filter_reviews(&mut self, query: ReviewQuery) -> Task<Message> {
        if !self.reviews.choose(query) {
            return Task::none();
        }

        Task::batch([
            operation::snap_to(SHEET, RelativeOffset::START),
            self.load_reviews(),
        ])
    }

    /// The blurred page no longer fits the window: the veil stays alone.
    pub fn resized(&mut self) {
        self.popups.backdrop = None;
    }

    fn can_open_popup(&self) -> bool {
        !self.zoomed && self.shown.is_some()
    }

    pub fn follow(&mut self, current: Option<AppId>, has_reviews: bool) {
        self.has_reviews = has_reviews;
        if current != self.shown {
            self.shown = current;
            self.item = 0;
            self.row_scroll = [None; 2];
            self.rewind_rows = true;
            self.trailers.arm(0);
            self.last_screenshot = None;
            self.browsing = false;
            self.zoomed = false;
            self.popups.close();
            self.loops.clear();
            self.spoilers.clear();
            self.expanded.clear();
            self.tags = current.and_then(|id| self.gamepalate.game_tags(id));
            self.originals.clear();
            self.autoplay();
        }
    }

    pub fn load(
        &mut self,
        upcoming: &[AppId],
        nearby: &[AppId],
        similar: &[AppId],
    ) -> Task<Message> {
        let rewind = self.rewind_rows();
        if upcoming.is_empty() {
            return rewind;
        }
        let details = self.load_details(upcoming, nearby);
        let images = self.load_images(upcoming, nearby, similar);
        self.descriptions.keep_only(nearby);
        self.reviews.keep_only(nearby);

        let ahead = self.load_first_thumbnails(upcoming, nearby);

        Task::batch([rewind, details, images, self.load_versions(), ahead])
    }

    /// Takes the rows of a Game left behind back to their start.
    fn rewind_rows(&mut self) -> Task<Message> {
        if !std::mem::take(&mut self.rewind_rows) {
            return Task::none();
        }
        Task::batch(
            self.row_ids
                .each_ref()
                .map(|id| operation::snap_to(id.clone(), RelativeOffset::START)),
        )
    }

    pub fn load_headers(&mut self, ids: &[AppId]) -> Task<Message> {
        let urls: Vec<String> = ids.iter().map(|&id| self.header_url(id)).collect();
        self.images.keep_only(&urls);
        let started = self.images.want(urls);

        self.download(started)
    }

    pub fn popup<'a>(&'a self, name: &'a str) -> Option<Element<'a, Message>> {
        let popup = self.popups.open?;
        let (bar, body) = match popup {
            Popup::About => (None, self.about(self.shown?)),
            Popup::Tags => (None, self.tags_body()),
            Popup::Reviews => (
                Some(self.review_filters(self.shown?)),
                self.reviews_body(self.shown?),
            ),
        };

        Some(sheet(
            popup.label(),
            name,
            self.popups.backdrop.as_ref(),
            bar,
            body,
            Message::ClosePopup,
        ))
    }

    /// The year, the developers and the tags, small, as a few lines under the name.
    pub fn facts<'a>(&'a self, id: AppId, tags: &[String]) -> Option<Element<'a, Message>> {
        self.pages.details(id).map(|details| facts(details, tags))
    }

    /// The keys of the sheets as one button, then the features; `compact` for a narrow column.
    pub fn more(&self, id: AppId, compact: bool) -> Option<Element<'_, Message>> {
        let open = self.features_open;
        let reviews = self.has_reviews && self.shown == Some(id);

        self.pages
            .details(id)
            .map(|details| more(id, details, open, reviews, compact))
    }

    fn load_details(&mut self, upcoming: &[AppId], nearby: &[AppId]) -> Task<Message> {
        let ahead = if self.current_is_loaded() {
            DETAILS_AHEAD
        } else {
            1
        };
        let wanted = &upcoming[..upcoming.len().min(ahead + 1)];

        self.pages.keep_only(nearby);
        let started = self.pages.want(wanted);

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

    pub fn is_open(&self) -> bool {
        self.zoomed || self.popups.is_shown_or_coming()
    }

    pub fn is_popup_open(&self) -> bool {
        self.popups.is_shown_or_coming()
    }
}

fn facts<'a>(details: &'a GameDetails, tags: &[String]) -> Element<'a, Message> {
    let store = &details.store;
    let byline = format::byline(
        format::year(store.release_date),
        format::developers(&store.developers),
    )
    .map(|byline| text(byline).size(font::NOTE_SIZE).style(style::muted));
    let early_access = store.is_early_access.then(|| {
        text("Early Access")
            .size(font::NOTE_SIZE)
            .style(style::accent)
    });
    // One piece per tag, so a long line breaks between two tags and never inside one.
    let tags = (!tags.is_empty()).then(|| {
        let last = tags.len() - 1;
        mouse_area(
            row(tags.iter().enumerate().map(|(index, tag)| {
                let piece = if index == last {
                    tag.clone()
                } else {
                    format!("{tag} ·")
                };
                text(piece).size(font::NOTE_SIZE).style(style::muted).into()
            }))
            .spacing(TAG_SPACING)
            .wrap()
            .vertical_spacing(LINE_SPACING),
        )
        .on_press(Message::ToggleTags)
        .interaction(mouse::Interaction::Pointer)
    });

    column![]
        .push(byline)
        .push(early_access)
        .push(tags)
        .spacing(LINE_SPACING)
        .into()
}

fn more(
    id: AppId,
    details: &GameDetails,
    open: bool,
    reviews: bool,
    compact: bool,
) -> Element<'_, Message> {
    let store = &details.store;
    let sheets = game::key_group(
        sheet_keys(reviews, compact)
            .into_iter()
            .map(|(key, label, message)| (key, label, Some(message)))
            .collect(),
        compact,
    );
    let header = button(
        row![
            text("Features").size(font::BAR_SIZE),
            text(if open { OPENED } else { CLOSED })
                .font(font::ICONS)
                .size(font::icon(10.0)),
        ]
        .spacing(8)
        .align_y(Center),
    )
    .padding(0)
    .style(style::list_row(false))
    .on_press(Message::ToggleFeatures);
    // The Steam Deck first, then the modes and the controller.
    let chips = open.then(|| {
        let features = format::features(&details.modes, store.controller)
            .into_iter()
            .map(|feature| chip(feature, style::tag));

        row(std::iter::once(deck::chip_of(id, details)).chain(features))
            .spacing(6)
            .wrap()
            .vertical_spacing(6)
    });
    let menu = column![header].push(chips).spacing(8);

    column![sheets].push(menu).spacing(MORE_SPACING).into()
}

/// One chip of Features.
fn chip<'a>(
    label: &'a str,
    style: impl Fn(&iced::Theme) -> container::Style + 'a,
) -> Element<'a, Message> {
    container(text(label).size(font::NOTE_SIZE))
        .padding([2, 8])
        .style(style)
        .into()
}

// The keys of the sheets with their labels, short in a narrow column.
fn sheet_keys(reviews: bool, compact: bool) -> Vec<(&'static str, &'static str, Message)> {
    let (about, tags) = if compact {
        ("About", "Tags")
    } else {
        ("About this game", "All tags")
    };

    [
        Some(("D", about, Message::ToggleAbout)),
        reviews.then_some(("R", "Reviews", Message::ToggleReviews)),
        Some(("T", tags, Message::ToggleTags)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(super) fn scroll_sheet<M: 'static>(y: f32) -> Task<M> {
    operation::scroll_by(SHEET, AbsoluteOffset { x: 0.0, y })
}

/// `backdrop` is the blurred page under the sheet, when there is one. A sheet with a `bar` takes
/// all the height there is: the list under the bar changes with it, and the sheet keeps its size.
pub(super) fn sheet<'a, M: Clone + 'a>(
    label: &'a str,
    name: &'a str,
    backdrop: Option<&'a image::Handle>,
    bar: Option<Element<'a, M>>,
    body: Element<'a, M>,
    close: M,
) -> Element<'a, M> {
    let close_button = tooltip(
        button(
            text(CLOSE)
                .font(font::ICONS)
                .size(font::icon(11.0))
                .width(Fill)
                .height(Fill)
                .center(),
        )
        .width(CLOSE_SIZE)
        .height(CLOSE_SIZE)
        .padding(0)
        .style(style::sheet_close)
        .on_press(close.clone()),
        container(text("Close (Esc)").size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Left,
    );
    let header = row![
        text(label).size(TITLE_SIZE).font(font::SEMIBOLD),
        text(name)
            .size(font::BAR_SIZE)
            .style(style::muted)
            .width(Fill),
        close_button,
    ]
    .spacing(10)
    .align_y(Center);
    let height = if bar.is_some() { Fill } else { Shrink };
    let sheet = column![
        container(header).padding(padding::top(10).bottom(10).left(SHEET_INSET).right(12)),
        style::separator(),
    ]
    .push(bar.map(|bar| {
        column![
            container(bar).padding(padding::top(10).bottom(10).left(SHEET_INSET).right(20)),
            style::separator(),
        ]
    }))
    .push(
        scrollable(
            container(body).padding(
                padding::top(20)
                    .bottom(SHEET_INSET)
                    .left(SHEET_INSET)
                    .right(SHEET_INSET),
            ),
        )
        .height(height)
        .id(SHEET),
    )
    .height(height);

    let picture = backdrop.map(|handle| {
        image(handle.clone())
            .width(Fill)
            .height(Fill)
            .content_fit(ContentFit::Fill)
    });
    let veil = if backdrop.is_some() {
        style::frosted_veil
    } else {
        style::veil
    };

    stack![]
        .push(picture)
        .push(opaque(
            mouse_area(container(space()).width(Fill).height(Fill).style(veil)).on_press(close),
        ))
        .push(
            container(opaque(
                container(sheet)
                    .max_width(MEASURE + 2.0 * SHEET_INSET)
                    .height(height)
                    .style(style::popup),
            ))
            .center(Fill)
            .padding(SHEET_MARGIN),
        )
        .into()
}

fn note<'a>(label: &'static str) -> Element<'a, Message> {
    text(label)
        .size(font::NOTE_SIZE)
        .style(style::muted)
        .center()
        .into()
}

fn sheet_note<'a, M: 'a>(label: &'static str) -> Element<'a, M> {
    text(label).size(BODY_SIZE).style(style::muted).into()
}

#[cfg(test)]
mod tests {

    use super::*;

    fn picture() -> image::Handle {
        image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255])
    }

    /// Opens `popup` the way the app does: the picture of the page comes, then the sheet.
    fn open(popups: &mut Popups, popup: Popup) {
        popups.toggle(popup, true);
        popups.pictured(Some(picture()));
    }

    #[test]
    fn the_sheet_keys_are_short_in_a_narrow_column_and_skip_reviews_without_any() {
        let labels = |reviews, compact| {
            sheet_keys(reviews, compact)
                .into_iter()
                .map(|(key, label, _)| format!("{key} {label}"))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            labels(true, false),
            ["D About this game", "R Reviews", "T All tags"]
        );
        assert_eq!(labels(true, true), ["D About", "R Reviews", "T Tags"]);
        assert_eq!(labels(false, true), ["D About", "T Tags"]);
    }

    #[test]
    fn about_keeps_its_header_in_the_shared_sheet() {
        assert_eq!(Popup::About.label(), "About this game");
        assert_eq!(Popup::Tags.label(), "Tags");
        assert_eq!(Popup::Reviews.label(), "Reviews");
    }

    #[test]
    fn r_opens_and_closes_the_reviews_only_when_there_are_some() {
        let mut popups = Popups::default();

        assert!(!popups.toggle(Popup::Reviews, false));
        assert!(!popups.is_shown_or_coming());

        open(&mut popups, Popup::Reviews);
        assert_eq!(popups.open, Some(Popup::Reviews));
        assert!(!popups.toggle(Popup::Reviews, true));
        assert_eq!(popups.open, None);
    }

    #[test]
    fn t_opens_and_closes_the_tags() {
        let mut popups = Popups::default();

        open(&mut popups, Popup::Tags);
        assert_eq!(popups.open, Some(Popup::Tags));

        popups.toggle(Popup::Tags, true);
        assert_eq!(popups.open, None);
    }

    #[test]
    fn a_sheet_waits_for_the_blurred_page_behind_it() {
        let mut popups = Popups::default();

        assert!(popups.toggle(Popup::About, true));
        assert_eq!(popups.open, None);
        assert!(popups.is_shown_or_coming());

        popups.pictured(Some(picture()));
        assert_eq!(popups.open, Some(Popup::About));
        assert!(popups.backdrop.is_some());
    }

    #[test]
    fn without_a_picture_the_sheet_opens_over_the_veil_alone() {
        let mut popups = Popups::default();

        popups.toggle(Popup::Tags, true);
        popups.pictured(None);

        assert_eq!(popups.open, Some(Popup::Tags));
        assert!(popups.backdrop.is_none());
    }

    #[test]
    fn a_second_key_while_waiting_asks_no_other_picture() {
        let mut popups = Popups::default();
        popups.toggle(Popup::Tags, true);

        assert!(!popups.toggle(Popup::About, true));
        popups.pictured(None);
        assert_eq!(popups.open, Some(Popup::About));
    }

    #[test]
    fn the_same_key_while_waiting_gives_up_the_sheet() {
        let mut popups = Popups::default();
        popups.toggle(Popup::Reviews, true);

        popups.toggle(Popup::Reviews, true);
        popups.pictured(Some(picture()));

        assert!(!popups.is_shown_or_coming());
        assert!(popups.backdrop.is_none());
    }

    #[test]
    fn opening_one_popup_closes_the_other_and_keeps_the_picture() {
        let mut popups = Popups::default();
        open(&mut popups, Popup::Tags);

        assert!(!popups.toggle(Popup::About, true));
        assert_eq!(popups.open, Some(Popup::About));
        assert!(!popups.is(Popup::Tags));
        assert!(popups.backdrop.is_some());

        popups.toggle(Popup::Tags, true);
        assert_eq!(popups.open, Some(Popup::Tags));
    }

    #[test]
    fn no_popup_opens_over_the_zoomed_screenshot() {
        let mut popups = Popups::default();

        popups.toggle(Popup::Tags, false);
        popups.toggle(Popup::About, false);

        assert!(!popups.is_shown_or_coming());
    }

    #[test]
    fn a_late_picture_after_closing_opens_nothing() {
        let mut popups = Popups::default();
        popups.toggle(Popup::About, true);

        popups.close();
        popups.pictured(Some(picture()));

        assert_eq!(popups.open, None);
    }

    #[test]
    fn changing_game_closes_the_popup_and_keeps_all_the_tags_shown() {
        let mut popups = Popups {
            all_tags: true,
            ..Popups::default()
        };
        open(&mut popups, Popup::Tags);

        popups.close();

        assert_eq!(popups.open, None);
        assert!(popups.backdrop.is_none());
        assert!(popups.all_tags);
    }
}
