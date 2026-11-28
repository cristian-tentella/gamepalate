use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ::image::imageops::FilterType;
use gamepalate_app::{
    AppId, GameDetails, ImageKind, Trailer, original_screenshot, thumbnail_screenshot,
};
use iced::{
    Center, Color, ContentFit, Element, Fill, Length, Rectangle, Task, Vector,
    advanced::widget::{self, Operation, operate, operation::Scrollable},
    mouse, padding,
    widget::{
        Column, button, column, container, image, keyed_column, mouse_area, opaque, row,
        scrollable, space, stack, text, tooltip,
    },
};

use super::{
    CLOSE, Message, PLAY, Showcase, Shown, Version,
    images::{Images, Picture},
    note,
    pages::Page,
};
use crate::{font, sideways::sideways, style, video};

const SCREENSHOTS_AHEAD: usize = 2;
const MEDIA_SPACING: f32 = 14.0;
const THUMBNAIL_HEIGHT: f32 = 65.0;
const THUMBNAIL_WIDTH: f32 = THUMBNAIL_HEIGHT * 16.0 / 9.0;
/// Around each thumbnail: the ring of the one shown, 2 px of accent 2 px away from it.
const RING_ROOM: f32 = 4.0;
const ROW_HEIGHT: f32 = THUMBNAIL_HEIGHT + 2.0 * RING_ROOM;
/// Between the screenshot and the row: with the room of the ring, 12 px to the thumbnails.
const ROW_GAP: f32 = 8.0;
/// The fade at an edge of the row with thumbnails hidden past it.
const FADE_WIDTH: f32 = 40.0;
/// How far the thumbnail shown stays from the edges of the row.
const ROW_MARGIN: f32 = 8.0;
const THUMBNAIL_PLAY: f32 = 27.0;
const THUMBNAIL_PLAY_SHADOW: f32 = 1.5;
pub const SCREENSHOT_WIDTH: f32 = 820.0;
const SOURCE_WIDTH: f32 = 1920.0;
const IMAGE_RADIUS: f32 = 8.0;
const WAITING_OPACITY: f32 = 0.5;
const MAX_ZOOM: f32 = 4.0;
const ZOOM_STEP: f32 = 0.25;
const LEFT: char = '\u{E5CB}';
const RIGHT: char = '\u{E5CC}';
const ARROW_INSET: f32 = 12.0;
const BLURRED_WIDTH: u32 = 320;
const BLUR_SIGMA: f32 = 5.0;
const PLAY_SIZE: f32 = 72.0;
const PLAY_HOVERED: f32 = 80.0;
const PLAY_SHADOW: f32 = 3.0;
const TRAILER_NAME_SIZE: f32 = 15.0;
const TRAILER_NAME_INSET: f32 = 16.0;

/// The two rows of thumbnails: under the gallery and under the zoom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Gallery,
    Zoom,
}

/// One item of the gallery: the trailers come first, then the screenshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Trailer(usize),
    Screenshot(usize),
}

fn item_at(trailers: usize, screenshots: usize, index: usize) -> Option<Item> {
    match index.checked_sub(trailers) {
        None => Some(Item::Trailer(index)),
        Some(screenshot) if screenshot < screenshots => Some(Item::Screenshot(screenshot)),
        Some(_) => None,
    }
}

pub(super) fn item_of(details: &GameDetails, index: usize) -> Option<Item> {
    let store = &details.store;
    item_at(store.trailers.len(), store.screenshot_urls.len(), index)
}

/// The trailer the gallery just moved to, which starts by itself.
fn arrived_on(item: Option<Item>) -> Option<usize> {
    match item? {
        Item::Trailer(index) => Some(index),
        Item::Screenshot(_) => None,
    }
}

fn item_count(details: &GameDetails) -> usize {
    details.store.trailers.len() + details.store.screenshot_urls.len()
}

/// The pictures of the gallery in its order: the posters of the trailers, then the screenshots.
fn pictures(details: &GameDetails) -> Vec<String> {
    let store = &details.store;
    store
        .trailers
        .iter()
        .filter_map(|trailer| trailer.poster_url.clone())
        .chain(store.screenshot_urls.iter().cloned())
        .collect()
}

/// The small pictures of the row under the gallery, in its order: the thumbnail of each trailer,
/// then the small size of each screenshot.
fn row_pictures(details: &GameDetails) -> Vec<Option<String>> {
    let store = &details.store;
    store
        .trailers
        .iter()
        .map(|trailer| trailer.thumbnail_url.clone())
        .chain(
            store
                .screenshot_urls
                .iter()
                .map(|url| thumbnail_screenshot(url)),
        )
        .collect()
}

/// Where the row scrolls to keep the thumbnail at `left` in sight, `ROW_MARGIN` from its edges:
/// as little as it takes, and not at all if it is already there.
fn row_offset(left: f32, viewport: f32, content: f32, offset: f32) -> f32 {
    let lowest = left + THUMBNAIL_WIDTH + ROW_MARGIN - viewport;
    let highest = left - ROW_MARGIN;

    offset
        .max(lowest)
        .min(highest)
        .clamp(0.0, (content - viewport).max(0.0))
}

fn thumbnail_left(index: usize) -> f32 {
    RING_ROOM + index as f32 * (THUMBNAIL_WIDTH + 2.0 * RING_ROOM)
}

fn row_width(count: usize) -> f32 {
    count as f32 * (THUMBNAIL_WIDTH + 2.0 * RING_ROOM)
}

/// Whether the row has more thumbnails than it shows, so the wheel over it scrolls it.
fn row_overflows(viewport: f32, content: f32) -> bool {
    content > viewport + 0.5
}

/// Whether the row fades at its left and at its right edge: only where thumbnails are hidden.
fn row_fades(offset: f32, viewport: f32, content: f32) -> (bool, bool) {
    (offset > 0.5, offset + viewport < content - 0.5)
}

/// Scrolls the row with the given id to keep the thumbnail at `left` in sight.
struct Reveal {
    target: widget::Id,
    left: f32,
}

impl<T> Operation<T> for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        if id == Some(&self.target) {
            let x = row_offset(self.left, bounds.width, content_bounds.width, translation.x);
            state.scroll_to(scrollable::AbsoluteOffset {
                x: Some(x),
                y: None,
            });
        }
    }
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

/// The trailer in the place of the screenshot, with the poster under it while it loads.
fn playing<'a>(
    player: &'a video::Player,
    poster: Element<'a, video::Message>,
    width: f32,
    height: f32,
) -> Element<'a, Message> {
    container(player.view(poster).map(Message::Trailer))
        .width(width)
        .height(height)
        .style(style::picture)
        .into()
}

fn wanted_versions(
    screenshots: &[String],
    index: Option<usize>,
    zoomed: bool,
    browsing: bool,
    waiting: bool,
) -> Vec<(Version, String)> {
    let current = index.and_then(|index| screenshots.get(index));
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

fn arrow<'a>(icon: char, message: Message) -> Element<'a, Message> {
    button(text(icon).font(font::ICONS).size(font::icon(14.0)))
        .padding(10)
        .style(style::arrow)
        .on_press(message)
        .into()
}

pub fn max_screenshot_width(scale: f32) -> f32 {
    (SOURCE_WIDTH / scale).max(SCREENSHOT_WIDTH)
}

pub fn screenshot_width(width: f32, height: f32, min: f32, max: f32) -> f32 {
    let fitting = (height - ROW_GAP - ROW_HEIGHT) * 16.0 / 9.0;

    width.min(fitting.clamp(min, max))
}

fn is_loaded<T>(
    page: Option<&Page>,
    header: Option<&Picture<T>>,
    screenshot: Option<Option<&Picture<T>>>,
) -> bool {
    let done =
        |picture: Option<&Picture<T>>| matches!(picture, Some(Picture::Ready(_) | Picture::Failed));
    !matches!(page, Some(Page::Loading(_)) | None) && done(header) && screenshot.is_none_or(done)
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

pub(super) fn next_index(index: usize, count: usize) -> usize {
    if index + 1 < count { index + 1 } else { index }
}

pub(super) fn shown<'a, T, U>(
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

impl Showcase {
    pub(super) fn load_versions(&mut self) -> Task<Message> {
        let screenshots = self.shown.map_or(&[][..], |id| self.screenshots(id));
        let waiting = self
            .current_screenshot()
            .is_some_and(|url| matches!(self.images.get(url), None | Some(Picture::Loading)));
        let index = match self.current_item() {
            Some(Item::Screenshot(index)) => Some(index),
            Some(Item::Trailer(_)) | None => None,
        };
        let wanted = wanted_versions(screenshots, index, self.zoomed, self.browsing, waiting);

        Task::batch(
            wanted
                .into_iter()
                .map(|(version, url)| self.load_version(version, url)),
        )
    }

    /// The blurred first screenshot of the games coming up, so it is already there when one of
    /// them is shown. The blurred ones of games no longer nearby are dropped.
    pub(super) fn load_first_thumbnails(
        &mut self,
        upcoming: &[AppId],
        nearby: &[AppId],
    ) -> Task<Message> {
        let mut kept: Vec<&String> = Vec::new();
        for &id in upcoming.iter().chain(nearby) {
            kept.extend(self.screenshots(id));
        }
        let kept: HashSet<String> = kept.into_iter().cloned().collect();
        self.thumbnails.retain(|url, _| kept.contains(url));

        let first: Vec<String> = upcoming
            .iter()
            .filter_map(|&id| self.first_picture(id))
            .collect();

        Task::batch(
            first
                .into_iter()
                .map(|url| self.load_version(Version::Thumbnail, url)),
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

    pub(super) fn version_loaded(
        &mut self,
        version: Version,
        url: &str,
        allocation: Option<image::Allocation>,
    ) {
        if let Some(picture) = self.versions(version).get_mut(url) {
            *picture = allocation.map_or(Picture::Failed, Picture::Ready);
        }
    }

    /// `scale` measures the poster in physical pixels, to choose the rendition of a trailer.
    pub fn media<'a>(
        &'a self,
        id: AppId,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Element<'a, Message> {
        self.gallery_column(id, width, height, scale)
            .push(self.description(id))
            .into()
    }

    pub fn gallery<'a>(
        &'a self,
        id: AppId,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Element<'a, Message> {
        self.gallery_column(id, width, height, scale).into()
    }

    fn gallery_column<'a>(
        &'a self,
        id: AppId,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Column<'a, Message> {
        let gallery = column![self.screenshot(id, width, height, scale)]
            .push(
                self.pages
                    .details(id)
                    .and_then(|details| self.row(id, details, Row::Gallery, Some(width))),
            )
            .spacing(ROW_GAP);

        column![gallery].spacing(MEDIA_SPACING).width(width)
    }

    /// Another item stops the trailer; the zoom is only for screenshots.
    pub(super) fn show_screenshot(&mut self, index: usize) {
        if index == self.item {
            return;
        }
        self.trailers.stop();
        self.browsing = true;
        let shown = self.current_picture();
        if shown.is_some_and(|url| matches!(self.images.get(url), Some(Picture::Ready(_)))) {
            self.last_screenshot = shown.cloned();
        }
        self.item = index;
        if !matches!(self.current_item(), Some(Item::Screenshot(_))) {
            self.zoomed = false;
        }
        if let Some(trailer) = arrived_on(self.current_item()) {
            self.trailers.arm(trailer);
        }
    }

    pub(super) fn current_is_loaded(&self) -> bool {
        let Some(id) = self.shown else {
            return true;
        };
        let header = self.images.get(&self.header_url(id));
        let picture = self.current_picture().map(|url| self.images.get(url));
        is_loaded(self.pages.get(id), header, picture)
    }

    pub(super) fn current_item(&self) -> Option<Item> {
        item_of(self.pages.details(self.shown?)?, self.item)
    }

    pub(super) fn current_screenshot(&self) -> Option<&String> {
        match self.current_item()? {
            Item::Screenshot(index) => self.screenshots(self.shown?).get(index),
            Item::Trailer(_) => None,
        }
    }

    /// The poster of a trailer or the screenshot.
    fn current_picture(&self) -> Option<&String> {
        match self.current_item()? {
            Item::Screenshot(_) => self.current_screenshot(),
            Item::Trailer(index) => self.trailer(index)?.poster_url.as_ref(),
        }
    }

    pub(super) fn trailer(&self, index: usize) -> Option<&Trailer> {
        self.pages.details(self.shown?)?.store.trailers.get(index)
    }

    pub(super) fn load_images(
        &mut self,
        upcoming: &[AppId],
        nearby: &[AppId],
        similar: &[AppId],
    ) -> Task<Message> {
        let Some((&current, next)) = upcoming.split_first() else {
            return Task::none();
        };

        let pictures = self.pictures(current);
        let thumbnails: Vec<String> = self
            .pages
            .details(current)
            .map_or_else(Vec::new, row_pictures)
            .into_iter()
            .flatten()
            .collect();
        let soon = next_screenshots(&pictures, self.item);
        let similar: Vec<String> = similar
            .iter()
            .map(|&id| self.gamepalate.header_url(id))
            .collect();
        let loaded = self.current_is_loaded();
        let mut wanted = if loaded {
            let later = if self.browsing {
                other_screenshots(&pictures, self.item)
            } else {
                Vec::new()
            };
            let next = next
                .iter()
                .map(|&id| (self.header_url(id), self.first_picture(id)))
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
            kept.extend(self.first_picture(id));
        }
        kept.extend(pictures.iter().cloned());
        kept.extend(thumbnails.iter().cloned());
        kept.extend(similar.iter().cloned());
        let mut similar = similar.into_iter();
        wanted.splice(1..1, similar.next());
        wanted.extend(similar);
        if loaded {
            wanted.extend(thumbnails);
        }

        self.images.keep_only(&kept);
        let started = self.images.want(wanted);

        self.download(started)
    }

    pub(super) fn download(&self, started: Vec<String>) -> Task<Message> {
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

    fn pictures(&self, id: AppId) -> Vec<String> {
        self.pages.details(id).map_or_else(Vec::new, pictures)
    }

    /// The poster of the first trailer, or the first screenshot.
    fn first_picture(&self, id: AppId) -> Option<String> {
        self.pictures(id).into_iter().next()
    }

    pub(super) fn item_count(&self) -> usize {
        self.shown
            .and_then(|id| self.pages.details(id))
            .map_or(0, item_count)
    }

    pub(super) fn header_url(&self, id: AppId) -> String {
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

    fn picture<'a>(&self, url: &str, width: f32, height: f32) -> Element<'a, Message> {
        framed(self.images.get(url), width, height, 1.0)
    }

    pub(super) fn shown(&self, wanted: &String) -> Shown<'_> {
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

    fn screenshot<'a>(
        &'a self,
        id: AppId,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Element<'a, Message> {
        let content: Element<'a, Message> = match self.pages.get(id) {
            Some(Page::Ready(details)) => {
                let picture = match item_of(details, self.item) {
                    Some(Item::Trailer(index)) => {
                        let trailer = &details.store.trailers[index];
                        match self.trailers.player(index) {
                            Some(player) => {
                                playing(player, self.poster_image(trailer), width, height)
                            }
                            None => self.poster(trailer, width, height, scale),
                        }
                    }
                    Some(Item::Screenshot(index)) => {
                        self.zoomable(&details.store.screenshot_urls[index], width, height)
                    }
                    None => note("No screenshots"),
                };
                if item_count(details) == 0 {
                    picture
                } else {
                    return mouse_area(stack![picture].push(self.arrows(item_count(details))))
                        .on_enter(Message::Hover(true))
                        .on_exit(Message::Hover(false))
                        .into();
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
            Some(Page::Loading(since)) => container(video::spinner(*since)).center(Fill).into(),
            None => space().into(),
        };

        container(content)
            .center_x(width)
            .center_y(height)
            .style(style::picture)
            .into()
    }

    /// A screenshot that opens in the zoom once it is there.
    fn zoomable<'a>(&self, wanted: &String, width: f32, height: f32) -> Element<'a, Message> {
        let (picture, opacity) = self.picture_shown(wanted);
        let ready = matches!(picture, Some(Picture::Ready(_)));
        let picture = framed(picture, width, height, opacity);
        if ready {
            mouse_area(picture)
                .on_press(Message::Zoom)
                .interaction(mouse::Interaction::ZoomIn)
                .into()
        } else {
            picture
        }
    }

    fn picture_shown(&self, wanted: &String) -> (Option<&Picture>, f32) {
        match self.shown(wanted) {
            Shown::Wanted => (self.images.get(wanted), 1.0),
            Shown::Thumbnail => (self.thumbnails.get(wanted), 1.0),
            Shown::Last(last) => (self.images.get(last), WAITING_OPACITY),
        }
    }

    /// The poster of a trailer, as Steam shows it: a shade from the bottom, a large play icon and
    /// the name. A click anywhere plays it.
    fn poster<'a>(
        &self,
        trailer: &Trailer,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Element<'a, Message> {
        let picture = match &trailer.poster_url {
            Some(url) => {
                let (picture, opacity) = self.picture_shown(url);
                framed(picture, width, height, opacity)
            }
            None => framed(None, width, height, 1.0),
        };
        let size = if self.hovered {
            PLAY_HOVERED
        } else {
            PLAY_SIZE
        };
        let icon = |color: Color| text(PLAY).font(font::ICONS).size(size).color(color);
        let play = tooltip(
            stack![
                container(icon(Color::BLACK.scale_alpha(0.4))).padding(padding::top(PLAY_SHADOW)),
                icon(Color::WHITE),
            ],
            container(text("Play trailer").size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        );
        let name = container(
            text(trailer.name.clone())
                .size(TRAILER_NAME_SIZE)
                .font(font::SEMIBOLD)
                .color(Color::WHITE),
        )
        .padding(TRAILER_NAME_INSET)
        .align_bottom(Fill)
        .width(Fill);

        mouse_area(stack![
            picture,
            container(space())
                .width(Fill)
                .height(Fill)
                .style(style::poster_shade),
            container(play).center(Fill),
            name,
            video::probe(scale, Message::Measured),
        ])
        .on_press(Message::PlayTrailer)
        .interaction(mouse::Interaction::Pointer)
        .into()
    }

    /// The poster under the trailer while it loads, half seen like a screenshot that comes.
    fn poster_image<'a>(&self, trailer: &Trailer) -> Element<'a, video::Message> {
        let picture = trailer
            .poster_url
            .as_ref()
            .and_then(|url| self.images.get(url));
        match picture {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .border_radius(IMAGE_RADIUS)
                .opacity(WAITING_OPACITY)
                .into(),
            _ => space().into(),
        }
    }

    fn arrows<'a>(&self, count: usize) -> Option<Element<'a, Message>> {
        if !self.hovered || count < 2 {
            return None;
        }
        let previous = (self.item > 0).then(|| arrow(LEFT, Message::PreviousScreenshot));
        let next = (self.item + 1 < count).then(|| arrow(RIGHT, Message::NextScreenshot));

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
                self.item,
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
                    self.shown
                        .and_then(|id| Some((id, self.pages.details(id)?)))
                        .and_then(|(id, details)| self.row(id, details, Row::Zoom, None))
                        .map(|row| container(row).center_x(Fill)),
                )
                .spacing(ROW_GAP),
            )
            .padding(16)
            .width(Fill)
            .height(Fill)
            .style(style::zoom),
        ))
    }

    /// A thumbnail for each item, under the gallery and under the zoom. The row is new for each
    /// game, so it starts from the left. `width` is the gallery's; the zoom's row is as wide as
    /// its thumbnails, as far as the window lets it, and only its scrolling tells how wide it is.
    fn row<'a>(
        &self,
        id: AppId,
        details: &GameDetails,
        which: Row,
        width: Option<f32>,
    ) -> Option<Element<'a, Message>> {
        if item_count(details) < 2 {
            return None;
        }
        let thumbnails = row_pictures(details)
            .into_iter()
            .enumerate()
            .map(|(index, url)| {
                let trailer = match item_of(details, index) {
                    Some(Item::Trailer(trailer)) => details.store.trailers.get(trailer),
                    _ => None,
                };
                self.row_thumbnail(index, url.as_deref(), trailer)
            });
        let scrolled = self.row_scroll[which as usize];
        let measures = match (width, scrolled) {
            (Some(width), _) => Some((
                scrolled.map_or(0.0, |viewport| viewport.absolute_offset().x),
                width,
                row_width(item_count(details)),
            )),
            (None, Some(viewport)) => Some((
                viewport.absolute_offset().x,
                viewport.bounds().width,
                viewport.content_bounds().width,
            )),
            (None, None) => None,
        };
        let (left, right) = measures.map_or((false, false), |(offset, viewport, content)| {
            row_fades(offset, viewport, content)
        });
        let overflows =
            measures.is_some_and(|(_, viewport, content)| row_overflows(viewport, content));
        let row = scrollable(row(thumbnails))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::hidden(),
            ))
            .id(self.row_ids[which as usize].clone())
            .on_scroll(move |viewport| Message::RowScrolled(which, viewport))
            .width(width.map_or(Length::Shrink, Length::Fixed));
        let fade = |shown: bool, left: bool| {
            shown.then(|| {
                container(space())
                    .width(FADE_WIDTH)
                    .height(ROW_HEIGHT)
                    .style(style::row_fade(left))
            })
        };

        let row = stack![
            keyed_column([(id, row.into())]),
            row![]
                .push(fade(left, true))
                .push(space::horizontal())
                .push(fade(right, false))
                .width(Fill),
        ]
        .width(width.map_or(Length::Shrink, Length::Fixed));

        // Over a row with hidden thumbnails the vertical wheel scrolls it sideways, like
        // Shift+wheel; a row that fits leaves the wheel to the page.
        Some(if overflows {
            sideways(row).into()
        } else {
            row.into()
        })
    }

    fn row_thumbnail<'a>(
        &self,
        index: usize,
        url: Option<&str>,
        trailer: Option<&Trailer>,
    ) -> Element<'a, Message> {
        let picture: Element<'a, Message> = match url.and_then(|url| self.images.get(url)) {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
                .width(THUMBNAIL_WIDTH)
                .height(THUMBNAIL_HEIGHT)
                .content_fit(ContentFit::Cover)
                .border_radius(style::THUMBNAIL_RADIUS)
                .into(),
            _ => container(space())
                .width(THUMBNAIL_WIDTH)
                .height(THUMBNAIL_HEIGHT)
                .style(style::thumbnail)
                .into(),
        };
        let thumbnail: Element<'a, Message> = match trailer {
            Some(trailer) => {
                let icon = |color: Color| {
                    text(PLAY)
                        .font(font::ICONS)
                        .size(THUMBNAIL_PLAY)
                        .color(color)
                };
                let thumbnail = stack![
                    picture,
                    container(space())
                        .width(Fill)
                        .height(Fill)
                        .style(style::thumbnail_shade),
                    container(stack![
                        container(icon(Color::BLACK.scale_alpha(0.4)))
                            .padding(padding::top(THUMBNAIL_PLAY_SHADOW)),
                        icon(Color::WHITE),
                    ])
                    .center(Fill),
                ];
                tooltip(
                    thumbnail,
                    container(text(trailer.name.clone()).size(font::NOTE_SIZE))
                        .padding([6, 10])
                        .style(style::popup),
                    tooltip::Position::Top,
                )
                .into()
            }
            None => picture,
        };

        mouse_area(
            container(thumbnail)
                .padding(RING_ROOM)
                .style(style::thumbnail_ring(index == self.item)),
        )
        .on_press(Message::ShowScreenshot(index))
        .interaction(mouse::Interaction::Pointer)
        .into()
    }

    /// Scrolls the rows to keep the thumbnail shown in sight.
    pub(super) fn reveal(&self) -> Task<Message> {
        let left = thumbnail_left(self.item);
        Task::batch(self.row_ids.each_ref().map(|target| {
            operate(Reveal {
                target: target.clone(),
                left,
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_THUMBNAIL: Option<&Picture<u8>> = None;

    #[test]
    fn moving_to_a_trailer_arms_it_and_to_a_screenshot_does_not() {
        assert_eq!(arrived_on(item_at(3, 5, 2)), Some(2));
        assert_eq!(arrived_on(item_at(3, 5, 3)), None);
        assert_eq!(arrived_on(None), None);
    }

    #[test]
    fn the_screenshot_grows_only_while_it_fits_in_the_height() {
        assert_eq!(
            screenshot_width(600.0, 300.0, SCREENSHOT_WIDTH, 1280.0),
            600.0
        );
        assert_eq!(
            screenshot_width(2000.0, 300.0, SCREENSHOT_WIDTH, 1280.0),
            SCREENSHOT_WIDTH
        );
        assert_eq!(
            screenshot_width(2000.0, 643.5, SCREENSHOT_WIDTH, 1280.0),
            1000.0
        );
        assert_eq!(
            screenshot_width(2000.0, 1200.0, SCREENSHOT_WIDTH, 1280.0),
            1280.0
        );
        assert_eq!(
            screenshot_width(2000.0, 1200.0, SCREENSHOT_WIDTH, 1920.0),
            1920.0
        );
        assert_eq!(screenshot_width(2000.0, 441.0, 400.0, 1280.0), 640.0);
        assert_eq!(
            screenshot_width(2000.0, 441.0, SCREENSHOT_WIDTH, 1280.0),
            SCREENSHOT_WIDTH
        );
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
        assert!(!is_loaded(
            Some(&Page::Loading(std::time::Instant::now())),
            ready,
            None
        ));
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
            wanted_versions(&shots, Some(0), false, false, true),
            [thumbnail("ss_0.jpg")]
        );
        assert!(wanted_versions(&shots, Some(0), false, false, false).is_empty());
        assert_eq!(
            wanted_versions(&shots, Some(1), true, true, true),
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

    fn details(trailers: &[Option<&str>], screenshots: &[&str]) -> GameDetails {
        GameDetails {
            store: gamepalate_app::ItemDetails {
                short_description: None,
                developers: Vec::new(),
                release_date: None,
                header_url: None,
                screenshot_urls: screenshots.iter().map(|url| url.to_string()).collect(),
                trailers: trailers
                    .iter()
                    .enumerate()
                    .map(|(index, poster)| Trailer {
                        name: format!("Trailer {index}"),
                        poster_url: poster.map(str::to_string),
                        thumbnail_url: poster.map(|poster| format!("{poster} small")),
                        manifest_url: format!("https://video/{index}/dash_av1.mpd"),
                    })
                    .collect(),
                player_categories: Vec::new(),
                controller: None,
                italian: None,
                steam_deck: None,
                is_early_access: false,
            },
            modes: Vec::new(),
            deck: Vec::new(),
            protondb: None,
        }
    }

    #[test]
    fn the_trailers_come_before_the_screenshots() {
        let game = details(&[Some("poster 0"), Some("poster 1")], &["ss_0", "ss_1"]);

        assert_eq!(item_of(&game, 0), Some(Item::Trailer(0)));
        assert_eq!(item_of(&game, 1), Some(Item::Trailer(1)));
        assert_eq!(item_of(&game, 2), Some(Item::Screenshot(0)));
        assert_eq!(item_of(&game, 3), Some(Item::Screenshot(1)));
        assert_eq!(item_of(&game, 4), None);
        assert_eq!(item_at(0, 1, 0), Some(Item::Screenshot(0)));
        assert_eq!(item_at(0, 0, 0), None);
    }

    #[test]
    fn the_row_has_a_thumbnail_for_each_trailer_and_screenshot() {
        let game = details(
            &[Some("poster 0"), None],
            &[
                "https://example.com/ss_0.1920x1080.jpg?t=1",
                "https://example.com/ss_1.1920x1080.jpg",
            ],
        );

        assert_eq!(item_count(&game), 4);
        assert_eq!(
            row_pictures(&game),
            [
                Some("poster 0 small".to_string()),
                None,
                Some("https://example.com/ss_0.116x65.jpg?t=1".to_string()),
                Some("https://example.com/ss_1.116x65.jpg".to_string()),
            ]
        );
        assert_eq!(item_count(&details(&[], &["ss_0"])), 1);
        assert_eq!(next_index(2, 3), 2);
    }

    #[test]
    fn the_row_is_as_high_as_a_thumbnail_with_its_ring() {
        assert_eq!(ROW_HEIGHT, 73.0);
        assert_eq!(thumbnail_left(0), RING_ROOM);
        assert_eq!(
            thumbnail_left(2) - thumbnail_left(1) - THUMBNAIL_WIDTH,
            ROW_MARGIN
        );
    }

    /// A row of 20 thumbnails, 600 px wide.
    fn offset(index: usize, offset: f32) -> f32 {
        row_offset(
            thumbnail_left(index),
            600.0,
            thumbnail_left(20) - RING_ROOM,
            offset,
        )
    }

    #[test]
    fn the_row_fades_only_where_thumbnails_are_hidden() {
        assert_eq!(row_fades(0.0, 600.0, 400.0), (false, false));
        assert_eq!(row_fades(0.0, 600.0, 600.0), (false, false));
        assert_eq!(row_fades(0.0, 600.0, 1000.0), (false, true));
        assert_eq!(row_fades(200.0, 600.0, 1000.0), (true, true));
        assert_eq!(row_fades(400.0, 600.0, 1000.0), (true, false));
        assert_eq!(row_fades(399.8, 600.0, 1000.0), (true, false));
        assert_eq!(row_width(20), thumbnail_left(20) - RING_ROOM);
    }

    #[test]
    fn the_wheel_scrolls_the_row_only_when_it_overflows() {
        assert!(row_overflows(600.0, 1000.0));
        assert!(!row_overflows(600.0, 600.0));
        assert!(!row_overflows(600.0, 400.0));
    }

    #[test]
    fn the_row_scrolls_to_the_first_and_to_the_last() {
        let content = thumbnail_left(20) - RING_ROOM;

        assert_eq!(offset(0, 500.0), 0.0);
        assert_eq!(offset(19, 0.0), content - 600.0);
    }

    #[test]
    fn a_thumbnail_in_sight_does_not_move_the_row() {
        assert_eq!(offset(1, 0.0), 0.0);
        assert_eq!(offset(5, 300.0), 300.0);
    }

    #[test]
    fn the_row_scrolls_as_little_as_it_takes() {
        let left = thumbnail_left(8);

        assert_eq!(offset(8, 0.0), left + THUMBNAIL_WIDTH + ROW_MARGIN - 600.0);
        assert_eq!(offset(8, 1500.0), left - ROW_MARGIN);
    }

    #[test]
    fn a_row_shorter_than_its_width_does_not_scroll() {
        let content = thumbnail_left(3) - RING_ROOM;

        assert_eq!(row_offset(thumbnail_left(2), 600.0, content, 0.0), 0.0);
        assert_eq!(row_offset(thumbnail_left(2), 600.0, content, 40.0), 0.0);
    }

    #[test]
    fn the_poster_takes_the_place_of_the_first_screenshot() {
        let game = details(&[Some("poster 0"), None], &["ss_0", "ss_1"]);
        let pictures = pictures(&game);

        assert_eq!(pictures, ["poster 0", "ss_0", "ss_1"]);
        assert_eq!(
            download_order(
                "next header".to_string(),
                &[],
                Vec::new(),
                vec![("header".to_string(), pictures.first().cloned())],
            ),
            ["next header", "header", "poster 0"]
        );
        assert_eq!(next_screenshots(&pictures, 0), ["poster 0", "ss_0", "ss_1"]);
    }

    #[test]
    fn a_poster_has_no_thumbnail_or_original() {
        assert!(thumbnail_screenshot("https://shared/store_trailers/1/poster.jpg").is_none());
        assert!(original_screenshot("https://shared/store_trailers/1/poster.jpg").is_none());
    }

    #[test]
    fn stops_at_the_last_screenshot() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(2, 3), 2);
        assert_eq!(next_index(0, 0), 0);
    }
}
