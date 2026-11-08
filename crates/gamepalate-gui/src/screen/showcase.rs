mod descriptions;
pub(super) mod images;
mod pages;

use std::{collections::HashMap, sync::Arc};

use ::image::imageops::FilterType;
use gamepalate_app::{
    AppId, Block, GameDetails, GameTagsView, Gamepalate, ImageKind, Span, TagOfGame, UsedTag,
    original_screenshot, thumbnail_screenshot,
};
use iced::{
    Center, ContentFit, Element, Fill, Task, mouse, padding,
    widget::{
        Column, Id, button, column, container, image, keyed_column, mouse_area, opaque, operation,
        progress_bar, rich_text, row, scrollable, scrollable::AbsoluteOffset, space, span, stack,
        text,
    },
};

use self::{
    descriptions::{Description, Descriptions},
    images::{Images, Picture},
    pages::{Page, Pages},
};
use super::game;
use crate::{font, format, style};

pub const AHEAD: usize = 5;
pub const BEHIND: usize = 5;
const DETAILS_AHEAD: usize = 3;
const SCREENSHOTS_AHEAD: usize = 2;
const SHEET: Id = Id::new("showcase-sheet");
const FIRST_TAGS: usize = 10;
const TAG_NAME_WIDTH: f32 = 200.0;
const TAG_BAR_WIDTH: f32 = 360.0;
const TAG_BAR_HEIGHT: f32 = 6.0;
const TAG_PERCENT_WIDTH: f32 = 64.0;
const TAG_SPACING: f32 = 10.0;
const TAGS_SECTION_SPACING: f32 = 28.0;
const USED_REASON: &str = "These tags add the most to this game's score. You play many games with them, and few games on Steam have them.";
const SCROLL_STEP: f32 = 60.0;
const MEASURE: f32 = 680.0;
const SHEET_INSET: f32 = 40.0;
const SHEET_MARGIN: f32 = 32.0;
const BODY_SIZE: f32 = 16.0;
const BODY_LINE_HEIGHT: f32 = 1.5;
const HEADING_SIZE: f32 = 20.0;
const HEADING_SPACE: f32 = 12.0;
const BLOCK_SPACING: f32 = 16.0;
const ITEM_SPACING: f32 = 8.0;
const BULLET_WIDTH: f32 = 18.0;
const EXTRA_RADIUS: f32 = 4.0;
const DESCRIPTION_WIDTH: f32 = 760.0;
const DOT_SIZE: f32 = 8.0;
const MEDIA_SPACING: f32 = 14.0;
const DOTS_HEIGHT: f32 = 20.0;
const SCREENSHOT_WIDTH: f32 = 820.0;
const SOURCE_WIDTH: f32 = 1920.0;
const IMAGE_RADIUS: f32 = 8.0;
const WAITING_OPACITY: f32 = 0.5;
const MAX_ZOOM: f32 = 4.0;
const ZOOM_STEP: f32 = 0.25;
const CLOSE: char = '\u{E5CD}';
const CLOSED: char = '\u{E5CC}';
const OPENED: char = '\u{E5CF}';
const LEFT: char = '\u{E5CB}';
const RIGHT: char = '\u{E5CC}';
const PLAY: char = '\u{E037}';
const BULLET: &str = "•";
const ARROW_INSET: f32 = 12.0;
const BLURRED_WIDTH: u32 = 320;
const BLUR_SIGMA: f32 = 5.0;

pub struct Showcase {
    gamepalate: Arc<Gamepalate>,
    images: Images,
    pages: Pages,
    descriptions: Descriptions,
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
}

impl Popup {
    fn label(self) -> &'static str {
        match self {
            Popup::About => "About this game",
            Popup::Tags => "Tags",
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
            Message::ExtraLoaded(url, None) => self.extras.loaded(url, None),
            Message::ExtraLoaded(url, Some(handle)) => {
                return image::allocate(handle)
                    .map(move |allocation| Message::ExtraAllocated(url.clone(), allocation.ok()));
            }
            Message::ExtraAllocated(url, allocation) => self.extras.loaded(url, allocation),
            Message::OpenLink(url) => game::open_url(&url),
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
        ])
    }

    fn can_open_popup(&self) -> bool {
        !self.zoomed && self.shown.is_some()
    }

    fn load_versions(&mut self) -> Task<Message> {
        let screenshots = self.shown.map_or(&[][..], |id| self.screenshots(id));
        let waiting = self
            .current_screenshot()
            .is_some_and(|url| matches!(self.images.get(url), None | Some(Picture::Loading)));
        let wanted = wanted_versions(
            screenshots,
            self.screenshot,
            self.zoomed,
            self.browsing,
            waiting,
        );

        Task::batch(
            wanted
                .into_iter()
                .map(|(version, url)| self.load_version(version, url)),
        )
    }

    fn load_version(&mut self, version: Version, url: String) -> Task<Message> {
        let address = match version {
            Version::Original => original_screenshot(&url),
            Version::Thumbnail => thumbnail_screenshot(&url),
        };
        let Some(address) = address else {
            return Task::none();
        };
        if self.versions(version).contains_key(&url) {
            return Task::none();
        }
        self.versions(version).insert(url.clone(), Picture::Loading);

        let gamepalate = Arc::clone(&self.gamepalate);
        Task::perform(
            async move {
                let bytes = gamepalate.image_without_cache(&address).await.ok();
                let handle = bytes.and_then(|bytes| match version {
                    Version::Original => Some(image::Handle::from_bytes(bytes)),
                    Version::Thumbnail => blurred(&bytes),
                });
                (url, handle)
            },
            move |(url, handle)| Message::VersionLoaded(version, url, handle),
        )
    }

    fn versions(&mut self, version: Version) -> &mut HashMap<String, Picture> {
        match version {
            Version::Original => &mut self.originals,
            Version::Thumbnail => &mut self.thumbnails,
        }
    }

    fn version_loaded(
        &mut self,
        version: Version,
        url: &str,
        allocation: Option<image::Allocation>,
    ) {
        if let Some(picture) = self.versions(version).get_mut(url) {
            *picture = allocation.map_or(Picture::Failed, Picture::Ready);
        }
    }

    pub fn follow(&mut self, current: Option<AppId>) {
        if current != self.shown {
            self.shown = current;
            self.screenshot = 0;
            self.last_screenshot = None;
            self.browsing = false;
            self.zoomed = false;
            self.popups.close();
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

        Task::batch([details, images, self.load_versions()])
    }

    pub fn load_headers(&mut self, ids: &[AppId]) -> Task<Message> {
        let urls: Vec<String> = ids.iter().map(|&id| self.header_url(id)).collect();
        self.images.keep_only(&urls);
        let started = self.images.want(urls);

        self.download(started)
    }

    pub fn header<'a>(&self, id: AppId, width: f32, height: f32) -> Element<'a, Message> {
        self.picture(&self.header_url(id), width, height)
    }

    pub fn thumbnail<'a>(&self, id: AppId, width: f32, height: f32) -> Element<'a, Message> {
        match self.images.get(&self.gamepalate.header_url(id)) {
            Some(Picture::Ready(_)) => self.header(id, width, height),
            _ => container(space())
                .width(width)
                .height(height)
                .style(style::picture)
                .into(),
        }
    }

    pub fn media<'a>(&'a self, id: AppId, width: f32, height: f32) -> Element<'a, Message> {
        self.gallery_column(id, width, height)
            .push(self.description(id))
            .into()
    }

    pub fn gallery<'a>(&'a self, id: AppId, width: f32, height: f32) -> Element<'a, Message> {
        self.gallery_column(id, width, height).into()
    }

    pub fn description<'a>(&'a self, id: AppId) -> Option<Element<'a, Message>> {
        self.pages
            .details(id)?
            .store
            .short_description
            .as_deref()
            .map(|description| {
                container(text(description).size(font::BAR_SIZE))
                    .max_width(DESCRIPTION_WIDTH)
                    .into()
            })
    }

    pub fn popup<'a>(&'a self, name: &'a str) -> Option<Element<'a, Message>> {
        let popup = self.popups.open?;
        let body = match popup {
            Popup::About => self.about(self.shown?),
            Popup::Tags => self.tags_body(),
        };

        Some(sheet(popup.label(), name, body))
    }

    pub fn used_tags(&self) -> Vec<String> {
        used_names(self.tags.as_ref())
    }

    fn about(&self, id: AppId) -> Element<'_, Message> {
        match self.descriptions.get(id) {
            Some(Description::Ready(blocks)) => column(
                blocks
                    .iter()
                    .enumerate()
                    .map(|(index, block)| self.block(block, index == 0)),
            )
            .spacing(BLOCK_SPACING)
            .into(),
            Some(Description::Missing) => {
                sheet_note("The Steam store has no description for this game.")
            }
            Some(Description::Failed) => sheet_note("Couldn't load the description."),
            Some(Description::Loading) | None => sheet_note("Loading…"),
        }
    }

    fn tags_body(&self) -> Element<'_, Message> {
        let Some(view) = self.tags.as_ref().filter(|view| !view.tags.is_empty()) else {
            return sheet_note("No tags for this game");
        };
        let all = self.popups.all_tags;
        let shown = shown_tags(view.tags.len(), all);
        let list = column(
            view.tags[..shown]
                .iter()
                .map(|tag| tag_line(tag, view.weighted)),
        )
        .spacing(TAG_SPACING);
        let more = show_all_label(view.tags.len(), all).map(|label| {
            button(text(label).size(font::BAR_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::ToggleAllTags)
        });

        column![used_block(&view.used), column![list].push(more).spacing(16)]
            .spacing(TAGS_SECTION_SPACING)
            .into()
    }

    fn block<'a>(&'a self, block: &'a Block, first: bool) -> Element<'a, Message> {
        match block {
            Block::Heading(heading) => container(
                text(heading)
                    .size(HEADING_SIZE)
                    .font(font::SEMIBOLD)
                    .line_height(1.3),
            )
            .padding(padding::top(if first { 0.0 } else { HEADING_SPACE }))
            .into(),
            Block::Paragraph(spans) => paragraph(spans),
            Block::List(items) => column(items.iter().map(|item| {
                row![
                    container(text(BULLET).size(BODY_SIZE).line_height(BODY_LINE_HEIGHT))
                        .width(BULLET_WIDTH),
                    paragraph(item),
                ]
                .into()
            }))
            .spacing(ITEM_SPACING)
            .into(),
            Block::Image { url, width, height } => {
                self.extra(url, extra_size(*width, *height, MEASURE))
            }
            Block::Video { url } => button(
                row![
                    text(PLAY).font(font::ICONS).size(font::icon(12.0)),
                    text("Play video").size(BODY_SIZE),
                ]
                .spacing(8)
                .align_y(Center),
            )
            .padding(0)
            .style(style::link)
            .on_press(Message::OpenLink(url.clone()))
            .into(),
        }
    }

    fn extra<'a>(&self, url: &str, size: Option<(f32, f32)>) -> Element<'a, Message> {
        let picture = self.extras.get(url);
        match (picture, size) {
            (Some(Picture::Ready(allocation)), Some((width, height))) => {
                image(allocation.handle().clone())
                    .width(width)
                    .height(height)
                    .border_radius(EXTRA_RADIUS)
                    .into()
            }
            (Some(Picture::Ready(allocation)), None) => container(
                image(allocation.handle().clone())
                    .content_fit(ContentFit::ScaleDown)
                    .border_radius(EXTRA_RADIUS),
            )
            .max_width(MEASURE)
            .into(),
            (_, Some((width, height))) => container(space())
                .width(width)
                .height(height)
                .style(style::picture)
                .into(),
            (_, None) => space().into(),
        }
    }

    fn gallery_column<'a>(&'a self, id: AppId, width: f32, height: f32) -> Column<'a, Message> {
        column![self.screenshot(id, width, height)]
            .push(
                self.pages
                    .details(id)
                    .and_then(|details| self.dots(details.store.screenshot_urls.len())),
            )
            .spacing(MEDIA_SPACING)
            .width(width)
    }

    pub fn facts<'a>(&'a self, id: AppId, tags: &[String]) -> Option<Element<'a, Message>> {
        let open = self.features_open;

        self.pages
            .details(id)
            .map(|details| facts(details, tags, open))
    }

    fn show_screenshot(&mut self, index: usize) {
        if index == self.screenshot {
            return;
        }
        self.browsing = true;
        let shown = self.current_screenshot();
        if shown.is_some_and(|url| matches!(self.images.get(url), Some(Picture::Ready(_)))) {
            self.last_screenshot = shown.cloned();
        }
        self.screenshot = index;
    }

    fn current_is_loaded(&self) -> bool {
        let Some(id) = self.shown else {
            return true;
        };
        let header = self.images.get(&self.header_url(id));
        let screenshot = self.current_screenshot().map(|url| self.images.get(url));
        is_loaded(self.pages.get(id), header, screenshot)
    }

    fn current_screenshot(&self) -> Option<&String> {
        self.shown
            .and_then(|id| self.screenshots(id).get(self.screenshot))
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

    fn load_description(&mut self) -> Task<Message> {
        let Some(id) = self.shown.filter(|_| self.popups.is(Popup::About)) else {
            return Task::none();
        };

        Task::batch(self.descriptions.want(&[id]).into_iter().map(|id| {
            let gamepalate = Arc::clone(&self.gamepalate);
            Task::perform(
                async move {
                    gamepalate
                        .description(id)
                        .await
                        .map(|blocks| blocks.map(Arc::new))
                        .map_err(|error| error.to_string())
                },
                move |result| Message::DescriptionLoaded(id, result),
            )
        }))
    }

    fn load_extras(&mut self) -> Task<Message> {
        let Some(id) = self.shown.filter(|_| self.popups.is(Popup::About)) else {
            return Task::none();
        };
        let urls = self.descriptions.image_urls(id);
        self.extras.keep_only(&urls);
        let started = self.extras.want(urls);

        Task::batch(started.into_iter().map(|url| {
            let gamepalate = Arc::clone(&self.gamepalate);
            Task::perform(
                async move {
                    let handle = gamepalate.extra_image(&url).await.ok().map(|image| {
                        image::Handle::from_rgba(image.width, image.height, image.rgba)
                    });
                    (url, handle)
                },
                |(url, handle)| Message::ExtraLoaded(url, handle),
            )
        }))
    }

    fn load_images(
        &mut self,
        upcoming: &[AppId],
        nearby: &[AppId],
        similar: &[AppId],
    ) -> Task<Message> {
        let Some((&current, next)) = upcoming.split_first() else {
            return Task::none();
        };

        let screenshots = self.screenshots(current);
        let soon = next_screenshots(screenshots, self.screenshot);
        let similar: Vec<String> = similar
            .iter()
            .map(|&id| self.gamepalate.header_url(id))
            .collect();
        let mut wanted = if self.current_is_loaded() {
            let later = if self.browsing {
                other_screenshots(screenshots, self.screenshot)
            } else {
                Vec::new()
            };
            let next = next
                .iter()
                .map(|&id| (self.header_url(id), self.first_screenshot(id)))
                .collect();
            download_order(self.header_url(current), soon, later, next)
        } else {
            download_order(
                self.header_url(current),
                &soon[..soon.len().min(1)],
                Vec::new(),
                Vec::new(),
            )
        };

        let mut kept = Vec::new();
        for &id in nearby {
            kept.push(self.gamepalate.header_url(id));
            kept.extend(
                self.pages
                    .details(id)
                    .and_then(|details| details.store.header_url.clone()),
            );
            kept.extend(self.first_screenshot(id));
        }
        kept.extend(self.screenshots(current).iter().cloned());
        kept.extend(similar.iter().cloned());
        let mut similar = similar.into_iter();
        wanted.splice(1..1, similar.next());
        wanted.extend(similar);

        self.images.keep_only(&kept);
        let started = self.images.want(wanted);

        self.download(started)
    }

    fn download(&self, started: Vec<String>) -> Task<Message> {
        Task::batch(started.into_iter().map(|url| {
            let gamepalate = Arc::clone(&self.gamepalate);
            let kind = if self.pages.is_screenshot(&url) {
                ImageKind::Screenshot
            } else {
                ImageKind::Cover
            };
            Task::perform(
                async move {
                    let handle = gamepalate
                        .image(&url, kind)
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

    fn first_screenshot(&self, id: AppId) -> Option<String> {
        self.screenshots(id).first().cloned()
    }

    fn screenshot_count(&self) -> usize {
        self.shown.map_or(0, |id| self.screenshots(id).len())
    }

    fn header_url(&self, id: AppId) -> String {
        let url = self.gamepalate.header_url(id);
        let hashed = self
            .pages
            .details(id)
            .and_then(|details| details.store.header_url.clone());

        match (self.images.get(&url), hashed) {
            (Some(Picture::Failed), Some(hashed)) => hashed,
            _ => url,
        }
    }

    fn picture<'a>(&self, url: &str, width: f32, height: f32) -> Element<'a, Message> {
        framed(self.images.get(url), width, height, 1.0)
    }

    fn shown(&self, wanted: &String) -> Shown<'_> {
        shown(
            wanted,
            &self.images,
            self.thumbnails.get(wanted),
            self.last_screenshot.as_ref(),
        )
    }

    fn best(&self, url: &str) -> Option<&Picture> {
        match self.originals.get(url) {
            Some(original) if matches!(original, Picture::Ready(_)) => Some(original),
            _ => self.images.get(url),
        }
    }

    fn screenshot<'a>(&self, id: AppId, width: f32, height: f32) -> Element<'a, Message> {
        let content: Element<'a, Message> = match self.pages.get(id) {
            Some(Page::Ready(details)) => {
                match details.store.screenshot_urls.get(self.screenshot) {
                    Some(wanted) => {
                        let (picture, opacity) = match self.shown(wanted) {
                            Shown::Wanted => (self.images.get(wanted), 1.0),
                            Shown::Thumbnail => (self.thumbnails.get(wanted), 1.0),
                            Shown::Last(last) => (self.images.get(last), WAITING_OPACITY),
                        };
                        let ready = matches!(picture, Some(Picture::Ready(_)));
                        let picture = framed(picture, width, height, opacity);
                        let picture = if ready {
                            mouse_area(picture)
                                .on_press(Message::Zoom)
                                .interaction(mouse::Interaction::ZoomIn)
                                .into()
                        } else {
                            picture
                        };
                        let count = details.store.screenshot_urls.len();

                        return mouse_area(stack![picture].push(self.arrows(count)))
                            .on_enter(Message::Hover(true))
                            .on_exit(Message::Hover(false))
                            .into();
                    }
                    None => note("No screenshots"),
                }
            }
            Some(Page::Missing) => note("Not on the Steam store"),
            Some(Page::Failed(reason)) => column![
                note("The Steam store didn't answer, so only catalog data is shown"),
                text(reason.clone())
                    .size(font::NOTE_SIZE)
                    .style(style::muted)
                    .center(),
                button(text("Retry").size(font::NOTE_SIZE))
                    .style(style::primary)
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

    fn arrows<'a>(&self, count: usize) -> Option<Element<'a, Message>> {
        if !self.hovered || count < 2 {
            return None;
        }
        let previous = (self.screenshot > 0).then(|| arrow(LEFT, Message::PreviousScreenshot));
        let next = (self.screenshot + 1 < count).then(|| arrow(RIGHT, Message::NextScreenshot));

        Some(
            container(
                row![]
                    .push(previous)
                    .push(space::horizontal())
                    .push(next)
                    .align_y(Center),
            )
            .padding(ARROW_INSET)
            .center_y(Fill)
            .width(Fill)
            .into(),
        )
    }

    pub fn is_open(&self) -> bool {
        self.zoomed || self.popups.open.is_some()
    }

    pub fn is_popup_open(&self) -> bool {
        self.popups.open.is_some()
    }

    pub fn zoom(&self) -> Option<Element<'_, Message>> {
        if !self.zoomed {
            return None;
        }
        let wanted = self.current_screenshot()?;
        let picture = match (self.originals.get(wanted), self.shown(wanted)) {
            (Some(original), _) if matches!(original, Picture::Ready(_)) => Some(original),
            (_, Shown::Thumbnail) => self.thumbnails.get(wanted),
            (_, Shown::Last(last)) => self.best(last),
            (_, Shown::Wanted) => self.best(wanted),
        };
        let screenshot: Element<'_, Message> = match picture {
            Some(Picture::Ready(allocation)) => keyed_column([(
                self.screenshot,
                image::viewer(allocation.handle().clone())
                    .width(Fill)
                    .height(Fill)
                    .min_scale(1.0)
                    .max_scale(MAX_ZOOM)
                    .scale_step(ZOOM_STEP)
                    .into(),
            )])
            .height(Fill)
            .into(),
            Some(Picture::Failed) => container(note("Image unavailable")).center(Fill).into(),
            Some(Picture::Loading) | None => space().height(Fill).into(),
        };
        let close = button(text(CLOSE).font(font::ICONS).size(font::icon(12.0)))
            .padding([8, 12])
            .style(style::secondary)
            .on_press(Message::CloseZoom);

        Some(opaque(
            container(
                column![stack![
                    screenshot,
                    container(close).align_right(Fill).padding(8)
                ]]
                .push(
                    self.dots(self.screenshot_count())
                        .map(|dots| container(dots).center_x(Fill)),
                )
                .spacing(8),
            )
            .padding(16)
            .width(Fill)
            .height(Fill)
            .style(style::zoom),
        ))
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
}

pub fn max_screenshot_width(scale: f32) -> f32 {
    (SOURCE_WIDTH / scale).max(SCREENSHOT_WIDTH)
}

pub fn screenshot_width(width: f32, height: f32, max: f32) -> f32 {
    let fitting = (height - MEDIA_SPACING - DOTS_HEIGHT) * 16.0 / 9.0;

    width.min(fitting.clamp(SCREENSHOT_WIDTH, max))
}

fn framed<'a>(
    picture: Option<&Picture>,
    width: f32,
    height: f32,
    opacity: f32,
) -> Element<'a, Message> {
    let content: Element<'a, Message> = match picture {
        Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
            .width(width)
            .height(height)
            .content_fit(ContentFit::Cover)
            .border_radius(IMAGE_RADIUS)
            .opacity(opacity)
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

fn wanted_versions(
    screenshots: &[String],
    index: usize,
    zoomed: bool,
    browsing: bool,
    waiting: bool,
) -> Vec<(Version, String)> {
    let current = screenshots.get(index);
    let mut wanted: Vec<(Version, String)> = current
        .filter(|_| zoomed)
        .map(|url| (Version::Original, url.clone()))
        .into_iter()
        .collect();
    if browsing {
        wanted.extend(
            screenshots
                .iter()
                .map(|url| (Version::Thumbnail, url.clone())),
        );
    } else if waiting {
        wanted.extend(current.map(|url| (Version::Thumbnail, url.clone())));
    }

    wanted
}

fn blurred(bytes: &[u8]) -> Option<image::Handle> {
    let thumbnail = ::image::load_from_memory(bytes).ok()?;
    let height = BLURRED_WIDTH * thumbnail.height() / thumbnail.width().max(1);
    let blurred = thumbnail
        .resize_exact(BLURRED_WIDTH, height.max(1), FilterType::Triangle)
        .fast_blur(BLUR_SIGMA)
        .into_rgba8();

    Some(image::Handle::from_rgba(
        blurred.width(),
        blurred.height(),
        blurred.into_raw(),
    ))
}

fn facts<'a>(details: &'a GameDetails, tags: &[String], open: bool) -> Element<'a, Message> {
    let store = &details.store;
    let byline = format::byline(
        format::year(store.release_date),
        format::developers(&store.developers),
    )
    .map(|byline| text(byline).size(font::NOTE_SIZE).style(style::muted));
    let tags = (!tags.is_empty()).then(|| {
        mouse_area(
            text(tags.join(" · "))
                .size(font::NOTE_SIZE)
                .style(style::muted),
        )
        .on_press(Message::ToggleTags)
        .interaction(mouse::Interaction::Pointer)
    });
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

    column![]
        .push(byline)
        .push(tags)
        .push(menu)
        .spacing(8)
        .into()
}

fn arrow<'a>(icon: char, message: Message) -> Element<'a, Message> {
    button(text(icon).font(font::ICONS).size(font::icon(14.0)))
        .padding(10)
        .style(style::arrow)
        .on_press(message)
        .into()
}

fn paragraph(spans: &[Span]) -> Element<'_, Message> {
    let pieces: Vec<_> = spans
        .iter()
        .map(|part| {
            span(part.text.as_str())
                .font(if part.bold {
                    font::SEMIBOLD
                } else {
                    font::REGULAR
                })
                .underline(part.link.is_some())
                .link_maybe(part.link.clone())
        })
        .collect();

    rich_text(pieces)
        .size(BODY_SIZE)
        .line_height(BODY_LINE_HEIGHT)
        .on_link_click(Message::OpenLink)
        .into()
}

fn scroll_sheet(y: f32) -> Task<Message> {
    operation::scroll_by(SHEET, AbsoluteOffset { x: 0.0, y })
}

fn sheet<'a>(label: &'a str, name: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    let close = button(text(CLOSE).font(font::ICONS).size(font::icon(12.0)))
        .padding([8, 12])
        .style(style::secondary)
        .on_press(Message::ClosePopup);
    let header = row![
        column![
            text(label).size(font::NOTE_SIZE).style(style::muted),
            text(name).size(font::PRICE_SIZE).font(font::SEMIBOLD),
        ]
        .spacing(4)
        .width(Fill),
        close,
    ];
    let sheet = column![
        container(header).padding(padding::top(24).bottom(20).left(SHEET_INSET).right(20)),
        style::separator(),
        scrollable(
            container(body).padding(
                padding::top(24)
                    .bottom(SHEET_INSET)
                    .left(SHEET_INSET)
                    .right(SHEET_INSET)
            )
        )
        .id(SHEET),
    ];

    stack![
        opaque(
            mouse_area(
                container(space())
                    .width(Fill)
                    .height(Fill)
                    .style(style::veil)
            )
            .on_press(Message::ClosePopup)
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

fn used_block<'a>(used: &[UsedTag]) -> Element<'a, Message> {
    if used.is_empty() {
        return sheet_note("None of these tags are in your taste");
    }

    column![
        text("Used for your match")
            .size(font::INFO_SIZE)
            .font(font::SEMIBOLD),
        text(USED_REASON).size(font::BAR_SIZE).style(style::muted),
        column(
            used.iter()
                .map(|tag| text(used_line(tag)).size(font::BAR_SIZE).into())
        )
        .spacing(4),
    ]
    .spacing(8)
    .into()
}

fn tag_line(tag: &TagOfGame, weighted: bool) -> Element<'_, Message> {
    let counts = tag.used && !tag.banned;
    let name = text(&tag.name)
        .size(font::BAR_SIZE)
        .font(if counts {
            font::SEMIBOLD
        } else {
            font::REGULAR
        })
        .width(TAG_NAME_WIDTH);
    let name = if tag.banned {
        name.style(style::muted)
    } else {
        name
    };
    let bar = weighted.then(|| {
        progress_bar(0.0..=1.0, tag.relative as f32)
            .girth(TAG_BAR_HEIGHT)
            .length(TAG_BAR_WIDTH)
            .style(if counts {
                style::meter
            } else {
                style::quiet_meter
            })
    });
    let note = tag_note(tag, weighted).map(|note| {
        container(text(note).size(font::NOTE_SIZE).style(style::muted))
            .align_right(TAG_PERCENT_WIDTH)
    });

    row![name]
        .push(bar)
        .push(note)
        .spacing(12)
        .align_y(Center)
        .into()
}

fn tag_note(tag: &TagOfGame, weighted: bool) -> Option<String> {
    if tag.banned {
        Some("Banned".to_string())
    } else {
        weighted.then(|| format!("{:.0}%", tag.weight * 100.0))
    }
}

fn used_line(tag: &UsedTag) -> String {
    format!(
        "{} · in {} of your games · {} of Steam games",
        tag.name,
        tag.profile_games,
        catalog_share(tag.catalog_share)
    )
}

fn catalog_share(share: f64) -> String {
    if share < 0.01 {
        "<1%".to_string()
    } else {
        format!("{:.0}%", share * 100.0)
    }
}

fn shown_tags(count: usize, all: bool) -> usize {
    if all { count } else { count.min(FIRST_TAGS) }
}

fn show_all_label(count: usize, all: bool) -> Option<String> {
    (count > FIRST_TAGS).then(|| {
        if all {
            "Show fewer".to_string()
        } else {
            format!("Show all {count}")
        }
    })
}

fn used_names(view: Option<&GameTagsView>) -> Vec<String> {
    view.map_or_else(Vec::new, |view| {
        view.used.iter().map(|tag| tag.name.clone()).collect()
    })
}

fn sheet_note<'a>(label: &'static str) -> Element<'a, Message> {
    text(label).size(BODY_SIZE).style(style::muted).into()
}

fn extra_size(width: u32, height: u32, column: f32) -> Option<(f32, f32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let shown = (width as f32).min(column);
    Some((shown, height as f32 * shown / width as f32))
}

fn note<'a>(label: &'static str) -> Element<'a, Message> {
    text(label)
        .size(font::NOTE_SIZE)
        .style(style::muted)
        .center()
        .into()
}

fn is_loaded<T>(
    page: Option<&Page>,
    header: Option<&Picture<T>>,
    screenshot: Option<Option<&Picture<T>>>,
) -> bool {
    let done =
        |picture: Option<&Picture<T>>| matches!(picture, Some(Picture::Ready(_) | Picture::Failed));
    !matches!(page, Some(Page::Loading) | None) && done(header) && screenshot.is_none_or(done)
}

fn download_order(
    header: String,
    soon: &[String],
    later: Vec<String>,
    next: Vec<(String, Option<String>)>,
) -> Vec<String> {
    let mut wanted = vec![header];
    wanted.extend(soon.iter().cloned());
    wanted.extend(later);
    for (header, screenshot) in next {
        wanted.push(header);
        wanted.extend(screenshot);
    }
    wanted
}

fn next_screenshots(screenshots: &[String], index: usize) -> &[String] {
    let start = index.min(screenshots.len());
    let end = (index + 1 + SCREENSHOTS_AHEAD).min(screenshots.len());

    &screenshots[start..end]
}

fn other_screenshots(screenshots: &[String], index: usize) -> Vec<String> {
    let start = index.min(screenshots.len());
    let end = (index + 1 + SCREENSHOTS_AHEAD).min(screenshots.len());

    screenshots[end..]
        .iter()
        .chain(screenshots[..start].iter().rev())
        .cloned()
        .collect()
}

fn next_index(index: usize, count: usize) -> usize {
    if index + 1 < count { index + 1 } else { index }
}

fn shown<'a, T, U>(
    url: &String,
    images: &Images<T>,
    thumbnail: Option<&Picture<U>>,
    last: Option<&'a String>,
) -> Shown<'a> {
    match (images.get(url), thumbnail, last) {
        (Some(Picture::Ready(_) | Picture::Failed), _, _) => Shown::Wanted,
        (_, Some(Picture::Ready(_)), _) => Shown::Thumbnail,
        (_, _, Some(last)) => Shown::Last(last),
        _ => Shown::Wanted,
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::TagId;

    use super::*;

    #[test]
    fn a_description_image_keeps_its_size_and_only_shrinks() {
        assert_eq!(extra_size(600, 105, 760.0), Some((600.0, 105.0)));
        assert_eq!(extra_size(600, 300, 480.0), Some((480.0, 240.0)));
        assert_eq!(extra_size(0, 0, 760.0), None);
        assert_eq!(extra_size(600, 0, 760.0), None);
    }

    #[test]
    fn the_screenshot_grows_only_while_it_fits_in_the_height() {
        assert_eq!(screenshot_width(600.0, 300.0, 1280.0), 600.0);
        assert_eq!(screenshot_width(2000.0, 300.0, 1280.0), SCREENSHOT_WIDTH);
        assert_eq!(screenshot_width(2000.0, 596.5, 1280.0), 1000.0);
        assert_eq!(screenshot_width(2000.0, 1200.0, 1280.0), 1280.0);
        assert_eq!(screenshot_width(2000.0, 1200.0, 1920.0), 1920.0);
    }

    #[test]
    fn the_screenshot_stops_at_the_pixels_steam_gives() {
        assert_eq!(max_screenshot_width(1.0), 1920.0);
        assert_eq!(max_screenshot_width(1.25), 1536.0);
        assert_eq!(max_screenshot_width(1.5), 1280.0);
        assert_eq!(max_screenshot_width(3.0), SCREENSHOT_WIDTH);
    }

    #[test]
    fn loads_only_the_shown_screenshot_and_the_next_two() {
        let shots: Vec<String> = (0..6).map(|index| format!("ss_{index}.jpg")).collect();

        assert_eq!(
            next_screenshots(&shots, 2),
            ["ss_2.jpg", "ss_3.jpg", "ss_4.jpg"]
        );
    }

    #[test]
    fn the_game_is_loaded_with_its_details_header_and_screenshot() {
        let ready = Some(&Picture::Ready(0));
        let failed = Some(&Picture::<u8>::Failed);
        let loading = Some(&Picture::<u8>::Loading);

        assert!(is_loaded(Some(&Page::Missing), ready, None));
        assert!(is_loaded(Some(&Page::Failed(String::new())), failed, None));
        assert!(is_loaded(Some(&Page::Missing), ready, Some(failed)));
        assert!(!is_loaded(None, ready, None));
        assert!(!is_loaded(Some(&Page::Loading), ready, None));
        assert!(!is_loaded(Some(&Page::Missing), loading, None));
        assert!(!is_loaded::<u8>(Some(&Page::Missing), None, None));
        assert!(!is_loaded(Some(&Page::Missing), ready, Some(loading)));
        assert!(!is_loaded(Some(&Page::Missing), ready, Some(None)));
    }

    #[test]
    fn the_header_comes_first_then_the_screenshots() {
        let urls = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };

        let order = download_order(
            "header".to_string(),
            &urls(&["shot 1", "shot 2"]),
            urls(&["shot 3"]),
            vec![
                ("next header".to_string(), Some("next shot".to_string())),
                ("last header".to_string(), None),
            ],
        );

        assert_eq!(
            order,
            urls(&[
                "header",
                "shot 1",
                "shot 2",
                "shot 3",
                "next header",
                "next shot",
                "last header",
            ])
        );
    }

    #[test]
    fn a_game_being_browsed_wants_its_other_screenshots_after_the_next_ones() {
        let shots: Vec<String> = (0..6).map(|index| format!("ss_{index}.jpg")).collect();

        assert_eq!(
            other_screenshots(&shots, 2),
            ["ss_5.jpg", "ss_1.jpg", "ss_0.jpg"]
        );
        assert_eq!(other_screenshots(&shots[..2], 1), ["ss_0.jpg"]);
    }

    #[test]
    fn the_screenshot_order_stops_at_the_ends() {
        let shots: Vec<String> = (0..2).map(|index| format!("ss_{index}.jpg")).collect();

        assert_eq!(next_screenshots(&shots, 1), &shots[1..]);
        assert!(next_screenshots(&[], 0).is_empty());
    }

    const NO_THUMBNAIL: Option<&Picture<u8>> = None;

    #[test]
    fn shows_the_last_screenshot_until_the_next_is_ready() {
        let (last, next) = ("last".to_string(), "next".to_string());
        let mut images = Images::<u8>::default();
        images.want(vec![last.clone(), next.clone()]);
        images.loaded(last.clone(), Some(0));

        assert!(matches!(
            shown(&next, &images, NO_THUMBNAIL, Some(&last)),
            Shown::Last(url) if url == &last
        ));
        assert!(matches!(
            shown(&next, &images, NO_THUMBNAIL, None),
            Shown::Wanted
        ));

        images.loaded(next.clone(), Some(0));

        assert!(matches!(
            shown(&next, &images, NO_THUMBNAIL, Some(&last)),
            Shown::Wanted
        ));
    }

    #[test]
    fn shows_the_thumbnail_while_the_screenshot_loads() {
        let (last, next) = ("last".to_string(), "next".to_string());
        let mut images = Images::<u8>::default();
        images.want(vec![next.clone()]);

        assert!(matches!(
            shown(&next, &images, Some(&Picture::Ready(0u8)), Some(&last)),
            Shown::Thumbnail
        ));
        assert!(matches!(
            shown(&next, &images, Some(&Picture::<u8>::Loading), Some(&last)),
            Shown::Last(_)
        ));

        images.loaded(next.clone(), Some(0));

        assert!(matches!(
            shown(&next, &images, Some(&Picture::Ready(0u8)), Some(&last)),
            Shown::Wanted
        ));
    }

    #[test]
    fn a_failed_screenshot_is_shown_as_failed() {
        let (last, next) = ("last".to_string(), "next".to_string());
        let mut images = Images::<u8>::default();
        images.want(vec![next.clone()]);
        images.loaded(next.clone(), None);

        assert!(matches!(
            shown(&next, &images, Some(&Picture::Ready(0u8)), Some(&last)),
            Shown::Wanted
        ));
    }

    #[test]
    fn a_screenshot_still_loading_wants_its_thumbnail() {
        let shots: Vec<String> = (0..3).map(|index| format!("ss_{index}.jpg")).collect();
        let thumbnail = |url: &str| (Version::Thumbnail, url.to_string());

        assert_eq!(
            wanted_versions(&shots, 0, false, false, true),
            [thumbnail("ss_0.jpg")]
        );
        assert!(wanted_versions(&shots, 0, false, false, false).is_empty());
        assert_eq!(
            wanted_versions(&shots, 1, true, true, true),
            [
                (Version::Original, "ss_1.jpg".to_string()),
                thumbnail("ss_0.jpg"),
                thumbnail("ss_1.jpg"),
                thumbnail("ss_2.jpg"),
            ]
        );
    }

    #[test]
    fn the_thumbnail_is_enlarged_and_blurred() {
        let mut bytes = Vec::new();
        ::image::RgbImage::from_fn(116, 65, |x, _| ::image::Rgb([(x * 2) as u8, 0, 0]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                ::image::ImageFormat::Jpeg,
            )
            .unwrap();

        let Some(image::Handle::Rgba { width, height, .. }) = blurred(&bytes) else {
            panic!("the thumbnail is not an image");
        };

        assert_eq!((width, height), (BLURRED_WIDTH, 179));
        assert!(blurred(b"not an image").is_none());
    }

    #[test]
    fn about_keeps_its_header_in_the_shared_sheet() {
        assert_eq!(Popup::About.label(), "About this game");
        assert_eq!(Popup::Tags.label(), "Tags");
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

    #[test]
    fn shows_the_first_ten_tags_and_offers_the_rest() {
        assert_eq!(shown_tags(18, false), 10);
        assert_eq!(shown_tags(18, true), 18);
        assert_eq!(shown_tags(7, false), 7);
        assert_eq!(show_all_label(18, false).as_deref(), Some("Show all 18"));
        assert_eq!(show_all_label(18, true).as_deref(), Some("Show fewer"));
        assert_eq!(show_all_label(10, false), None);
        assert_eq!(show_all_label(3, true), None);
    }

    #[test]
    fn a_used_tag_says_how_many_games_have_it() {
        let used = |profile_games, catalog_share| UsedTag {
            id: TagId(1),
            name: "Roguelite".to_string(),
            profile_games,
            catalog_share,
        };

        assert_eq!(
            used_line(&used(14, 0.03)),
            "Roguelite · in 14 of your games · 3% of Steam games"
        );
        assert_eq!(
            used_line(&used(1, 0.004)),
            "Roguelite · in 1 of your games · <1% of Steam games"
        );
        assert_eq!(catalog_share(0.01), "1%");
        assert_eq!(catalog_share(0.0099), "<1%");
    }

    #[test]
    fn a_banned_tag_says_banned_instead_of_its_weight() {
        let tag = |banned| TagOfGame {
            id: TagId(1),
            name: "Horror".to_string(),
            weight: 0.08,
            relative: 1.0,
            used: false,
            banned,
        };

        assert_eq!(tag_note(&tag(false), true).as_deref(), Some("8%"));
        assert_eq!(tag_note(&tag(true), true).as_deref(), Some("Banned"));
        assert_eq!(tag_note(&tag(false), false), None);
        assert_eq!(tag_note(&tag(true), false).as_deref(), Some("Banned"));
    }

    #[test]
    fn the_facts_get_the_names_of_the_used_tags() {
        let view = GameTagsView {
            tags: Vec::new(),
            weighted: false,
            used: ["Roguelite", "Deckbuilder"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| UsedTag {
                    id: TagId(index as u32),
                    name: name.to_string(),
                    profile_games: 1,
                    catalog_share: 0.1,
                })
                .collect(),
        };

        assert_eq!(used_names(Some(&view)), ["Roguelite", "Deckbuilder"]);
        assert!(used_names(None).is_empty());
    }

    #[test]
    fn stops_at_the_last_screenshot() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(2, 3), 2);
        assert_eq!(next_index(0, 0), 0);
    }
}
