mod about;
mod descriptions;
pub(crate) mod images;
mod media;
mod pages;
mod review_sheet;
mod reviews;
mod tags;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gamepalate_app::{
    AppId, Block, GameDetails, GameTagsView, Gamepalate,
    reviews::{ReviewPage, ReviewQuery},
};
use iced::{
    Center, Element, Fill, Task, mouse, padding,
    widget::{
        Id, button, column, container, image, mouse_area, opaque, operation, row, scrollable,
        scrollable::{AbsoluteOffset, RelativeOffset},
        space, stack, text,
    },
};

use self::{
    descriptions::Descriptions,
    images::{Images, Picture},
    pages::Pages,
    reviews::{Request, ReviewLists},
};
use super::game;
use crate::{font, format, style};
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
const CLOSE: char = '\u{E5CD}';
const CLOSED: char = '\u{E5CC}';
const OPENED: char = '\u{E5CF}';

pub struct Showcase {
    gamepalate: Arc<Gamepalate>,
    images: Images,
    pages: Pages,
    descriptions: Descriptions,
    reviews: ReviewLists,
    spoilers: HashSet<(String, usize)>,
    has_reviews: bool,
    extras: Images,
    screenshot: usize,
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
    all_tags: bool,
}

impl Popups {
    fn toggle(&mut self, popup: Popup, allowed: bool) {
        if allowed {
            self.open = (self.open != Some(popup)).then_some(popup);
        }
    }

    fn close(&mut self) {
        self.open = None;
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
    RetryReviews,
    ShowSpoiler(String, usize),
    ClosePopup,
    ScrollUp,
    ScrollDown,
    Close,
    ToggleFeatures,
    Hover(bool),
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
            has_reviews: false,
            extras: Images::default(),
            screenshot: 0,
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
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
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
            Message::ExtraLoaded(url, None) => self.extras.loaded(url, None),
            Message::ExtraLoaded(url, Some(handle)) => {
                return image::allocate(handle)
                    .map(move |allocation| Message::ExtraAllocated(url.clone(), allocation.ok()));
            }
            Message::ExtraAllocated(url, allocation) => self.extras.loaded(url, allocation),
            Message::OpenLink(url) => super::open(url),
            Message::PreviousScreenshot | Message::NextScreenshot if self.popups.open.is_some() => {
            }
            Message::PreviousScreenshot => self.show_screenshot(self.screenshot.saturating_sub(1)),
            Message::NextScreenshot => {
                self.show_screenshot(next_index(self.screenshot, self.screenshot_count()));
            }
            Message::ShowScreenshot(index) => self.show_screenshot(index),
            Message::RetryDetails => {
                if let Some(id) = self.shown {
                    self.pages.retry(id);
                }
            }
            Message::Zoom => self.zoomed = self.current_screenshot().is_some(),
            Message::CloseZoom => self.zoomed = false,
            Message::ToggleAbout => self.popups.toggle(Popup::About, self.can_open_popup()),
            Message::ToggleTags => self.popups.toggle(Popup::Tags, self.can_open_popup()),
            Message::ToggleAllTags => self.popups.all_tags = !self.popups.all_tags,
            Message::ToggleReviews => self
                .popups
                .toggle(Popup::Reviews, self.can_open_popup() && self.has_reviews),
            Message::MoreReviews => return self.more_reviews(),
            Message::RetryReviews => return self.retry_reviews(),
            Message::FilterReviews(query) => {
                if self.reviews.choose(query) {
                    return Task::batch([
                        operation::snap_to(SHEET, RelativeOffset::START),
                        self.load_reviews(),
                    ]);
                }
            }
            Message::ShowSpoiler(id, index) => {
                self.spoilers.insert((id, index));
            }
            Message::ClosePopup => self.popups.close(),
            Message::Close => {
                self.zoomed = false;
                self.popups.close();
            }
            Message::ToggleFeatures => self.features_open = !self.features_open,
            Message::Hover(hovered) => self.hovered = hovered,
        }

        Task::batch([
            self.load_versions(),
            self.load_description(),
            self.load_extras(),
            self.load_reviews(),
        ])
    }

    fn can_open_popup(&self) -> bool {
        !self.zoomed && self.shown.is_some()
    }

    pub fn follow(&mut self, current: Option<AppId>, has_reviews: bool) {
        self.has_reviews = has_reviews;
        if current != self.shown {
            self.shown = current;
            self.screenshot = 0;
            self.last_screenshot = None;
            self.browsing = false;
            self.zoomed = false;
            self.popups.close();
            self.spoilers.clear();
            self.tags = current.and_then(|id| self.gamepalate.game_tags(id));
            self.originals.clear();
            self.thumbnails.clear();
        }
    }

    pub fn load(
        &mut self,
        upcoming: &[AppId],
        nearby: &[AppId],
        similar: &[AppId],
    ) -> Task<Message> {
        if upcoming.is_empty() {
            return Task::none();
        }
        let details = self.load_details(upcoming, nearby);
        let images = self.load_images(upcoming, nearby, similar);
        self.descriptions.keep_only(nearby);
        self.reviews.keep_only(nearby);

        Task::batch([details, images, self.load_versions()])
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

        Some(sheet(popup.label(), name, bar, body, Message::ClosePopup))
    }

    pub fn facts<'a>(&'a self, id: AppId, tags: &[String]) -> Option<Element<'a, Message>> {
        let open = self.features_open;

        self.pages
            .details(id)
            .map(|details| facts(details, tags, open))
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
        self.zoomed || self.popups.open.is_some()
    }

    pub fn is_popup_open(&self) -> bool {
        self.popups.open.is_some()
    }
}

fn facts<'a>(details: &'a GameDetails, tags: &[String], open: bool) -> Element<'a, Message> {
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
    let about = game::key_button(
        "D",
        "About this game",
        font::NOTE_SIZE,
        Some(Message::ToggleAbout),
    );
    let used = (!tags.is_empty()).then(|| {
        mouse_area(
            text(tags.join(" · "))
                .size(font::NOTE_SIZE)
                .style(style::muted),
        )
        .on_press(Message::ToggleTags)
        .interaction(mouse::Interaction::Pointer)
    });
    let tags = row![]
        .push(used)
        .push(game::key_button(
            "T",
            "All tags",
            font::NOTE_SIZE,
            Some(Message::ToggleTags),
        ))
        .spacing(game::ACTION_SPACING)
        .align_y(Center);
    let features = format::features(store.steam_deck, &details.modes, store.controller);
    let menu = (!features.is_empty()).then(|| {
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
        let chips = open.then(|| {
            row(features.into_iter().map(|feature| {
                container(text(feature).size(font::NOTE_SIZE))
                    .padding([2, 8])
                    .style(style::tag)
                    .into()
            }))
            .spacing(6)
            .wrap()
            .vertical_spacing(6)
        });

        column![header].push(chips).spacing(8)
    });

    column![about]
        .push(byline)
        .push(early_access)
        .push(tags)
        .push(menu)
        .spacing(8)
        .into()
}

pub(super) fn scroll_sheet<M: 'static>(y: f32) -> Task<M> {
    operation::scroll_by(SHEET, AbsoluteOffset { x: 0.0, y })
}

pub(super) fn sheet<'a, M: Clone + 'a>(
    label: &'a str,
    name: &'a str,
    bar: Option<Element<'a, M>>,
    body: Element<'a, M>,
    close: M,
) -> Element<'a, M> {
    let close_button = button(text(CLOSE).font(font::ICONS).size(font::icon(12.0)))
        .padding([8, 12])
        .style(style::secondary)
        .on_press(close.clone());
    let header = row![
        column![
            text(label).size(font::NOTE_SIZE).style(style::muted),
            text(name).size(font::PRICE_SIZE).font(font::SEMIBOLD),
        ]
        .spacing(4)
        .width(Fill),
        close_button,
    ];
    let sheet = column![
        container(header).padding(padding::top(24).bottom(20).left(SHEET_INSET).right(20)),
        style::separator(),
    ]
    .push(bar.map(|bar| {
        column![
            container(bar).padding(padding::top(12).bottom(12).left(SHEET_INSET).right(20)),
            style::separator(),
        ]
    }))
    .push(
        scrollable(
            container(body).padding(
                padding::top(24)
                    .bottom(SHEET_INSET)
                    .left(SHEET_INSET)
                    .right(SHEET_INSET),
            ),
        )
        .id(SHEET),
    );

    stack![
        opaque(
            mouse_area(
                container(space())
                    .width(Fill)
                    .height(Fill)
                    .style(style::veil)
            )
            .on_press(close)
        ),
        container(opaque(
            container(sheet)
                .max_width(MEASURE + 2.0 * SHEET_INSET)
                .style(style::popup)
        ))
        .center(Fill)
        .padding(SHEET_MARGIN),
    ]
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

    #[test]
    fn about_keeps_its_header_in_the_shared_sheet() {
        assert_eq!(Popup::About.label(), "About this game");
        assert_eq!(Popup::Tags.label(), "Tags");
        assert_eq!(Popup::Reviews.label(), "Reviews");
    }

    #[test]
    fn r_opens_and_closes_the_reviews_only_when_there_are_some() {
        let mut popups = Popups::default();

        popups.toggle(Popup::Reviews, false);
        assert_eq!(popups.open, None);

        popups.toggle(Popup::Reviews, true);
        assert_eq!(popups.open, Some(Popup::Reviews));
        popups.toggle(Popup::Reviews, true);
        assert_eq!(popups.open, None);
    }

    #[test]
    fn t_opens_and_closes_the_tags() {
        let mut popups = Popups::default();

        popups.toggle(Popup::Tags, true);
        assert_eq!(popups.open, Some(Popup::Tags));

        popups.toggle(Popup::Tags, true);
        assert_eq!(popups.open, None);
    }

    #[test]
    fn opening_one_popup_closes_the_other() {
        let mut popups = Popups::default();

        popups.toggle(Popup::Tags, true);
        popups.toggle(Popup::About, true);
        assert_eq!(popups.open, Some(Popup::About));
        assert!(!popups.is(Popup::Tags));

        popups.toggle(Popup::Tags, true);
        assert_eq!(popups.open, Some(Popup::Tags));
    }

    #[test]
    fn no_popup_opens_over_the_zoomed_screenshot() {
        let mut popups = Popups::default();

        popups.toggle(Popup::Tags, false);
        popups.toggle(Popup::About, false);

        assert_eq!(popups.open, None);
    }

    #[test]
    fn changing_game_closes_the_popup_and_keeps_all_the_tags_shown() {
        let mut popups = Popups {
            open: Some(Popup::Tags),
            all_tags: true,
        };

        popups.close();

        assert_eq!(popups.open, None);
        assert!(popups.all_tags);
    }
}
