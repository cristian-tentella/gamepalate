use std::sync::Arc;

use gamepalate_app::{AppId, Block, Span};
use iced::{
    Center, ContentFit, Element, Task, padding,
    widget::{button, column, container, image, rich_text, row, space, span, text},
};

use super::{
    BODY_SIZE, MEASURE, Message, PLAY, Popup, Showcase, descriptions::Description, images::Picture,
    sheet_note,
};
use crate::{font, style};

const BODY_LINE_HEIGHT: f32 = 1.5;
const HEADING_SIZE: f32 = 20.0;
const HEADING_SPACE: f32 = 12.0;
const BLOCK_SPACING: f32 = 16.0;
const ITEM_SPACING: f32 = 8.0;
const BULLET_WIDTH: f32 = 18.0;
const BULLET: &str = "•";
const EXTRA_RADIUS: f32 = 4.0;
const DESCRIPTION_WIDTH: f32 = 760.0;

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

fn extra_size(width: u32, height: u32, column: f32) -> Option<(f32, f32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let shown = (width as f32).min(column);
    Some((shown, height as f32 * shown / width as f32))
}

impl Showcase {
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

    pub(super) fn about(&self, id: AppId) -> Element<'_, Message> {
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

    pub(super) fn load_description(&mut self) -> Task<Message> {
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

    pub(super) fn load_extras(&mut self) -> Task<Message> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_image_keeps_its_size_and_only_shrinks() {
        assert_eq!(extra_size(600, 105, 760.0), Some((600.0, 105.0)));
        assert_eq!(extra_size(600, 300, 480.0), Some((480.0, 240.0)));
        assert_eq!(extra_size(0, 0, 760.0), None);
        assert_eq!(extra_size(600, 0, 760.0), None);
    }
}
