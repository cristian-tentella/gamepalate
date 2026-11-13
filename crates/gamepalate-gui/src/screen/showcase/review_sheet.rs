use std::sync::Arc;

use gamepalate_app::{
    AppId, Timestamp,
    reviews::{Block, Review, Span},
};
use iced::{
    Center, Element, Task, never, padding,
    widget::{button, column, container, rich_text, row, sensor, span, text},
};

use super::{
    BODY_SIZE, Message, Popup, Showcase,
    reviews::{Fetch, Request},
    sheet_note,
};
use crate::{font, format, style};

const LINE_HEIGHT: f32 = 1.5;
const REVIEW_SPACING: f32 = 24.0;
const BLOCK_SPACING: f32 = 12.0;
const BULLET_WIDTH: f32 = 18.0;
const BULLET: &str = "•";
const QUOTE_INSET: f32 = 18.0;
const LOAD_AHEAD: f32 = 600.0;

fn paragraph<'a>(spans: &'a [Span], quiet: bool) -> Element<'a, Message> {
    let pieces: Vec<_> = spans
        .iter()
        .map(|part| {
            span(part.text.as_str()).font(if part.bold {
                font::SEMIBOLD
            } else {
                font::REGULAR
            })
        })
        .collect();
    let paragraph = rich_text(pieces)
        .size(BODY_SIZE)
        .line_height(LINE_HEIGHT)
        .on_link_click(never);

    if quiet {
        paragraph.style(style::muted).into()
    } else {
        paragraph.into()
    }
}

fn spoiler<'a>(review: &'a Review, index: usize) -> Element<'a, Message> {
    button(text("Spoiler").size(font::NOTE_SIZE))
        .padding([2, 10])
        .style(style::chip)
        .on_press(Message::ShowSpoiler(review.id.clone(), index))
        .into()
}

fn heading(review: &Review, now: Timestamp) -> Element<'_, Message> {
    let verdict = text(format::verdict(review.recommended))
        .size(font::NOTE_SIZE)
        .font(font::SEMIBOLD);
    let verdict = if review.recommended {
        verdict.style(style::accent)
    } else {
        verdict
    };

    row![
        verdict,
        text("·").size(font::NOTE_SIZE).style(style::muted),
        text(format::review_facts(review, now))
            .size(font::NOTE_SIZE)
            .style(style::muted),
    ]
    .spacing(6)
    .into()
}

impl Showcase {
    pub(super) fn reviews_body(&self, id: AppId) -> Element<'_, Message> {
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());
        let list = self.reviews.get(id, self.review_query);
        let reviews = list.map_or(&[][..], |list| list.reviews.as_slice());
        let mut body = column![].spacing(REVIEW_SPACING);
        for (index, review) in reviews.iter().enumerate() {
            if index > 0 {
                body = body.push(style::separator());
            }
            body = body.push(self.review(review, now));
        }

        body.push(self.reviews_end(id, list.map(|list| list.fetch), reviews.len()))
            .into()
    }

    fn review<'a>(&'a self, review: &'a Review, now: Timestamp) -> Element<'a, Message> {
        let blocks = review
            .text
            .iter()
            .enumerate()
            .map(|(index, block)| self.review_block(review, index, block));

        column![heading(review, now)]
            .push(column(blocks).spacing(BLOCK_SPACING))
            .spacing(BLOCK_SPACING)
            .into()
    }

    fn review_block<'a>(
        &self,
        review: &'a Review,
        index: usize,
        block: &'a Block,
    ) -> Element<'a, Message> {
        match block {
            Block::Paragraph(spans) => paragraph(spans, false),
            Block::Heading(heading) => text(heading)
                .size(BODY_SIZE)
                .font(font::SEMIBOLD)
                .line_height(LINE_HEIGHT)
                .into(),
            Block::Item(spans) => row![
                container(text(BULLET).size(BODY_SIZE).line_height(LINE_HEIGHT))
                    .width(BULLET_WIDTH),
                paragraph(spans, false),
            ]
            .into(),
            Block::Quote(spans) => container(paragraph(spans, true))
                .padding(padding::left(QUOTE_INSET))
                .into(),
            Block::Spoiler(spans) if self.spoilers.contains(&(review.id.clone(), index)) => {
                paragraph(spans, false)
            }
            Block::Spoiler(_) => spoiler(review, index),
        }
    }

    fn reviews_end<'a>(
        &self,
        id: AppId,
        fetch: Option<Fetch>,
        shown: usize,
    ) -> Element<'a, Message> {
        match fetch {
            Some(Fetch::Finished) if shown == 0 => sheet_note("No reviews match these filters"),
            Some(Fetch::Finished) => sheet_note("No more reviews"),
            Some(Fetch::Failed) => row![
                sheet_note("Couldn't load the reviews"),
                button(text("Retry").size(font::NOTE_SIZE))
                    .style(style::primary)
                    .padding([4, 12])
                    .on_press(Message::RetryReviews),
            ]
            .spacing(16)
            .align_y(Center)
            .into(),
            Some(Fetch::Idle | Fetch::Loading) | None => sensor(sheet_note("Loading…"))
                .key((id, self.review_query, shown))
                .anticipate(LOAD_AHEAD)
                .on_show(|_| Message::MoreReviews)
                .into(),
        }
    }

    fn open_reviews(&self) -> Option<AppId> {
        self.shown.filter(|_| self.popups.is(Popup::Reviews))
    }

    pub(super) fn load_reviews(&mut self) -> Task<Message> {
        let request = self
            .open_reviews()
            .and_then(|id| self.reviews.start(id, self.review_query));
        self.fetch_reviews(request)
    }

    pub(super) fn more_reviews(&mut self) -> Task<Message> {
        let request = self
            .open_reviews()
            .and_then(|id| self.reviews.more(id, self.review_query));
        self.fetch_reviews(request)
    }

    pub(super) fn retry_reviews(&mut self) -> Task<Message> {
        let request = self
            .open_reviews()
            .and_then(|id| self.reviews.retry(id, self.review_query));
        self.fetch_reviews(request)
    }

    fn fetch_reviews(&self, request: Option<Request>) -> Task<Message> {
        let Some(request) = request else {
            return Task::none();
        };
        let gamepalate = Arc::clone(&self.gamepalate);

        Task::perform(
            async move {
                let result = gamepalate
                    .reviews(request.id, &request.query, request.cursor.as_deref())
                    .await
                    .map(Arc::new)
                    .map_err(|error| error.to_string());
                (request, result)
            },
            |(request, result)| Message::ReviewsLoaded(request, result),
        )
    }
}
