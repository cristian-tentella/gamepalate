pub(super) mod images;
mod pages;

use std::{collections::HashMap, sync::Arc};

use ::image::imageops::FilterType;
use gamepalate_app::{
    AppId, GameDetails, Gamepalate, ImageKind, original_screenshot, thumbnail_screenshot,
};
use iced::{
    Center, ContentFit, Element, Fill, Task, mouse, padding,
    widget::{
        Column, button, column, container, image, keyed_column, mouse_area, opaque, row, space,
        stack, text,
    },
};

use self::{
    images::{Images, Picture},
    pages::{Page, Pages},
};
use crate::{font, format, style};

pub const AHEAD: usize = 5;
pub const BEHIND: usize = 5;
const DETAILS_AHEAD: usize = 3;
const SCREENSHOTS_AHEAD: usize = 2;
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
const CLOSE: char = '\u{E8BB}';
const CLOSED: char = '\u{E76C}';
const OPENED: char = '\u{E70D}';
const LEFT: char = '\u{E76B}';
const RIGHT: char = '\u{E76C}';
const ARROW_INSET: f32 = 12.0;
const BLURRED_WIDTH: u32 = 320;
const BLUR_SIGMA: f32 = 5.0;

pub struct Showcase {
    gamepalate: Arc<Gamepalate>,
    images: Images,
    pages: Pages,
    screenshot: usize,
    last_screenshot: Option<String>,
    shown: Option<AppId>,
    browsing: bool,
    zoomed: bool,
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
    PreviousScreenshot,
    NextScreenshot,
    ShowScreenshot(usize),
    RetryDetails,
    Zoom,
    CloseZoom,
    ToggleFeatures,
    Hover(bool),
}

impl Showcase {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> Showcase {
        Showcase {
            gamepalate: Arc::clone(gamepalate),
            images: Images::default(),
            pages: Pages::default(),
            screenshot: 0,
            last_screenshot: None,
            shown: None,
            browsing: false,
            zoomed: false,
            features_open: false,
            hovered: false,
            originals: HashMap::new(),
            thumbnails: HashMap::new(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
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
            Message::ToggleFeatures => self.features_open = !self.features_open,
            Message::Hover(hovered) => self.hovered = hovered,
        }

        self.load_versions()
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

    pub fn is_zoomed(&self) -> bool {
        self.zoomed
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
        let close = button(text(CLOSE).font(font::ICONS).size(12))
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
        text(tags.join(" · "))
            .size(font::NOTE_SIZE)
            .style(style::muted)
    });
    let features = format::features(store.steam_deck, &details.modes, store.controller);
    let menu = (!features.is_empty()).then(|| {
        let header = button(
            row![
                text("Features").size(font::BAR_SIZE),
                text(if open { OPENED } else { CLOSED })
                    .font(font::ICONS)
                    .size(10),
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
    button(text(icon).font(font::ICONS).size(14))
        .padding(10)
        .style(style::arrow)
        .on_press(message)
        .into()
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
    use super::*;

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
    fn stops_at_the_last_screenshot() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(2, 3), 2);
        assert_eq!(next_index(0, 0), 0);
    }
}
