mod images;
mod pages;

use std::sync::Arc;

use gamepalate_app::{AppId, GameDetails, Gamepalate};
use iced::{
    Center, ContentFit, Element, Task, padding,
    widget::{button, column, container, image, row, space, text},
};

use self::{
    images::{Images, Picture},
    pages::{Page, Pages},
};
use crate::{font, format, style};

pub const AHEAD: usize = 5;
pub const BEHIND: usize = 5;
const DETAILS_AHEAD: usize = 2;
const SCREENSHOTS_AHEAD: usize = 2;
const DESCRIPTION_WIDTH: f32 = 600.0;
const DOT_SIZE: f32 = 8.0;
const IMAGE_RADIUS: f32 = 8.0;

pub struct Showcase {
    gamepalate: Arc<Gamepalate>,
    images: Images,
    pages: Pages,
    screenshot: usize,
    last_screenshot: Option<String>,
    shown: Option<AppId>,
}

#[derive(Clone, Debug)]
pub enum Message {
    ImageLoaded(String, Option<image::Handle>),
    ImageAllocated(String, Option<image::Allocation>),
    DetailsLoaded(AppId, Result<Option<Arc<GameDetails>>, String>),
    PreviousScreenshot,
    NextScreenshot,
    ShowScreenshot(usize),
    RetryDetails,
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
        }

        Task::none()
    }

    pub fn follow(&mut self, current: Option<AppId>) {
        if current != self.shown {
            self.shown = current;
            self.screenshot = 0;
            self.last_screenshot = None;
        }
    }

    pub fn load(&mut self, upcoming: &[AppId], nearby: &[AppId]) -> Task<Message> {
        if upcoming.is_empty() {
            return Task::none();
        }
        let details = self.load_details(upcoming, nearby);

        Task::batch([details, self.load_images(upcoming, nearby)])
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

    pub fn media<'a>(&'a self, id: AppId, width: f32, height: f32) -> Element<'a, Message> {
        let details = self.pages.details(id);

        column![self.screenshot(id, width, height)]
            .push(details.and_then(|details| self.dots(details.store.screenshot_urls.len())))
            .push(
                details
                    .and_then(|details| details.store.short_description.as_deref())
                    .map(|description| {
                        container(text(description).size(font::BAR_SIZE))
                            .max_width(DESCRIPTION_WIDTH)
                    }),
            )
            .spacing(14)
            .width(width)
            .into()
    }

    pub fn facts<'a>(&'a self, id: AppId) -> Option<Element<'a, Message>> {
        self.pages.details(id).map(facts)
    }

    fn show_screenshot(&mut self, index: usize) {
        if index == self.screenshot {
            return;
        }
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

    fn load_images(&mut self, upcoming: &[AppId], nearby: &[AppId]) -> Task<Message> {
        let Some((&current, next)) = upcoming.split_first() else {
            return Task::none();
        };

        let (soon, later) = screenshot_order(self.screenshots(current), self.screenshot);
        let wanted = if self.current_is_loaded() {
            let next = next
                .iter()
                .map(|&id| (self.header_url(id), self.first_screenshot(id)))
                .collect();
            download_order(self.header_url(current), soon, next, later)
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

        self.images.keep_only(&kept);
        let started = self.images.want(wanted);

        self.download(started)
    }

    fn download(&self, started: Vec<String>) -> Task<Message> {
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
        let content: Element<'a, Message> = match self.images.get(url) {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
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

    fn screenshot<'a>(&self, id: AppId, width: f32, height: f32) -> Element<'a, Message> {
        let content: Element<'a, Message> = match self.pages.get(id) {
            Some(Page::Ready(details)) => {
                match details.store.screenshot_urls.get(self.screenshot) {
                    Some(url) => {
                        let url =
                            shown_screenshot(url, &self.images, self.last_screenshot.as_ref());
                        return self.picture(url, width, height);
                    }
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
    next: Vec<(String, Option<String>)>,
    later: Vec<String>,
) -> Vec<String> {
    let mut wanted = vec![header];
    wanted.extend(soon.iter().cloned());
    for (header, screenshot) in next {
        wanted.push(header);
        wanted.extend(screenshot);
    }
    wanted.extend(later);
    wanted
}

fn screenshot_order(screenshots: &[String], index: usize) -> (&[String], Vec<String>) {
    let start = index.min(screenshots.len());
    let end = (index + 1 + SCREENSHOTS_AHEAD).min(screenshots.len());
    let later = screenshots[end..]
        .iter()
        .chain(screenshots[..start].iter().rev())
        .cloned()
        .collect();

    (&screenshots[start..end], later)
}

fn next_index(index: usize, count: usize) -> usize {
    if index + 1 < count { index + 1 } else { index }
}

fn shown_screenshot<'a, T>(
    url: &'a String,
    images: &Images<T>,
    last: Option<&'a String>,
) -> &'a String {
    match (images.get(url), last) {
        (Some(Picture::Loading) | None, Some(last)) => last,
        _ => url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_next_screenshots_first_then_the_rest() {
        let shots: Vec<String> = (0..6).map(|index| format!("ss_{index}.jpg")).collect();

        let (soon, later) = screenshot_order(&shots, 2);

        assert_eq!(soon, ["ss_2.jpg", "ss_3.jpg", "ss_4.jpg"]);
        assert_eq!(later, ["ss_5.jpg", "ss_1.jpg", "ss_0.jpg"]);
    }

    #[test]
    fn the_game_is_loaded_with_its_details_header_and_screenshot() {
        let ready = Some(&Picture::Ready(0));
        let failed = Some(&Picture::<u8>::Failed);
        let loading = Some(&Picture::<u8>::Loading);

        assert!(is_loaded(Some(&Page::Missing), ready, None));
        assert!(is_loaded(Some(&Page::Failed), failed, None));
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
            vec![
                ("next header".to_string(), Some("next shot".to_string())),
                ("last header".to_string(), None),
            ],
            urls(&["shot 3"]),
        );

        assert_eq!(
            order,
            urls(&[
                "header",
                "shot 1",
                "shot 2",
                "next header",
                "next shot",
                "last header",
                "shot 3"
            ])
        );
    }

    #[test]
    fn the_screenshot_order_stops_at_the_ends() {
        let shots: Vec<String> = (0..2).map(|index| format!("ss_{index}.jpg")).collect();

        assert_eq!(
            screenshot_order(&shots, 1),
            (&shots[1..], vec![shots[0].clone()])
        );
        assert_eq!(screenshot_order(&[], 0), (&[][..], Vec::new()));
    }

    #[test]
    fn shows_the_last_screenshot_until_the_next_is_ready() {
        let (last, next) = ("last".to_string(), "next".to_string());
        let mut images = Images::<u8>::default();
        images.want(vec![last.clone(), next.clone()]);
        images.loaded(last.clone(), Some(0));

        assert_eq!(shown_screenshot(&next, &images, Some(&last)), &last);
        assert_eq!(shown_screenshot(&next, &images, None), &next);

        images.loaded(next.clone(), Some(0));

        assert_eq!(shown_screenshot(&next, &images, Some(&last)), &next);
    }

    #[test]
    fn a_failed_screenshot_is_shown_as_failed() {
        let (last, next) = ("last".to_string(), "next".to_string());
        let mut images = Images::<u8>::default();
        images.want(vec![next.clone()]);
        images.loaded(next.clone(), None);

        assert_eq!(shown_screenshot(&next, &images, Some(&last)), &next);
    }

    #[test]
    fn stops_at_the_last_screenshot() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(2, 3), 2);
        assert_eq!(next_index(0, 0), 0);
    }
}
