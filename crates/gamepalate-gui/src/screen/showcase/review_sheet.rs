use std::{fmt, sync::Arc};

use gamepalate_app::{
    AppId, Timestamp,
    reviews::{Block, Language, Languages, Order, Review, ReviewQuery, Span, Verdict},
};
use iced::{
    Center, Element, Fill, Task, never,
    widget::{
        button, column, container, pick_list, rich_text, row, sensor, span, text, text_input,
    },
};

use super::{
    BODY_SIZE, Message, Popup, Showcase,
    reviews::{Fetch, Request},
    sheet_note,
};
use crate::{font, format, style};

const LINE_HEIGHT: f32 = 1.6;
const REVIEW_SPACING: f32 = 16.0;
const BLOCK_SPACING: f32 = 14.0;
const CARD_PADDING: f32 = 20.0;
const BULLET_WIDTH: f32 = 18.0;
const BULLET: &str = "•";
const QUOTE_PADDING: [u16; 2] = [10, 14];
const LOAD_AHEAD: f32 = 600.0;
const FILTER_SPACING: f32 = 16.0;
const THUMB_UP: char = '\u{E8DC}';
const THUMB_DOWN: char = '\u{E8DB}';
const THUMB_SIZE: f32 = 11.0;
/// About seven lines of the sheet: longer reviews are cut there until Read more.
const SHORT_CHARS: usize = 600;
/// What a block costs beyond its letters: each starts a line of its own.
const BLOCK_CHARS: usize = 40;
const CUT: &str = "…";
const ORDERS: [Sorting; 3] = [
    Sorting(Order::Helpful),
    Sorting(Order::Recent),
    Sorting(Order::Funny),
];
const VERDICTS: [Verdict; 3] = [Verdict::All, Verdict::Positive, Verdict::Negative];
/// The languages to choose from: a new one goes here, before All languages.
const LANGUAGES: [Speaking; 2] = [
    Speaking(Languages::Only(Language::English)),
    Speaking(Languages::All),
];
const HOURS_WIDTH: f32 = 56.0;
const HOURS_DIGITS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sorting(Order);

impl fmt::Display for Sorting {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.0 {
            Order::Helpful => "Most helpful",
            Order::Recent => "Most recent",
            Order::Funny => "Funniest",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Speaking(Languages);

impl fmt::Display for Speaking {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.0 {
            Languages::Only(Language::English) => "English",
            Languages::All => "All languages",
        })
    }
}

/// What the hours field keeps of what is typed: digits only, a few of them.
pub(super) fn hours_typed(typed: &str) -> String {
    typed
        .chars()
        .filter(char::is_ascii_digit)
        .take(HOURS_DIGITS)
        .collect()
}

/// The minimum the hours field asks for: nothing, or 0, is any.
pub(super) fn min_hours(typed: &str) -> Option<u32> {
    typed.parse().ok().filter(|&hours| hours > 0)
}

fn verdict_label(verdict: Verdict, matching: Option<u32>) -> String {
    let label = match verdict {
        Verdict::All => "All",
        Verdict::Positive => "Positive",
        Verdict::Negative => "Negative",
    };
    match matching {
        Some(matching) => format!("{label} {}", format::count(matching)),
        None => label.to_string(),
    }
}

fn dropdown<'a, T: fmt::Display + Clone + PartialEq + 'a>(
    choices: &'a [T],
    chosen: T,
    choose: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message> {
    pick_list(choices, Some(chosen), choose)
        .text_size(font::NOTE_SIZE)
        .padding([4, 8])
        .style(style::dropdown)
        .menu_style(style::menu)
        .into()
}

/// The start of a review, cut at a word near `limit` characters, or `None` when it is not
/// longer. A spoiler is never cut: it stays closed behind its button.
pub(super) fn shortened(blocks: &[Block], limit: usize) -> Option<Vec<Block>> {
    let mut left = limit;
    let mut kept = Vec::new();
    for block in blocks {
        let length = block_chars(block);
        if length <= left {
            kept.push(block.clone());
            left -= length;
            continue;
        }
        let cut = match block {
            Block::Spoiler(_) => None,
            _ if left < 2 * BLOCK_CHARS => None,
            Block::Heading(heading) => Some(Block::Heading(cut_text(heading, left - BLOCK_CHARS))),
            Block::Paragraph(spans) => Some(Block::Paragraph(cut_spans(spans, left - BLOCK_CHARS))),
            Block::Item(spans) => Some(Block::Item(cut_spans(spans, left - BLOCK_CHARS))),
            Block::Quote(spans) => Some(Block::Quote(cut_spans(spans, left - BLOCK_CHARS))),
        };
        kept.extend(cut);
        if kept.is_empty() {
            kept.push(block.clone());
        }
        return Some(kept);
    }
    None
}

fn block_chars(block: &Block) -> usize {
    let letters = match block {
        Block::Heading(heading) => heading.chars().count(),
        Block::Spoiler(_) => 0,
        Block::Paragraph(spans) | Block::Item(spans) | Block::Quote(spans) => {
            spans.iter().map(|part| part.text.chars().count()).sum()
        }
    };
    letters + BLOCK_CHARS
}

fn cut_spans(spans: &[Span], mut left: usize) -> Vec<Span> {
    let mut kept = Vec::new();
    for part in spans {
        let length = part.text.chars().count();
        if length < left {
            kept.push(part.clone());
            left -= length;
        } else {
            kept.push(Span {
                text: cut_text(&part.text, left),
                bold: part.bold,
            });
            break;
        }
    }
    kept
}

/// The first `limit` characters, back to the last space when there is one, then an ellipsis.
fn cut_text(text: &str, limit: usize) -> String {
    let start: String = text.chars().take(limit).collect();
    let start = match start.rfind(char::is_whitespace) {
        Some(space) if space > start.len() / 2 => &start[..space],
        _ => start.as_str(),
    };
    format!("{}{CUT}", start.trim_end())
}

fn paragraph<'a>(spans: &[Span], quiet: bool) -> Element<'a, Message> {
    let pieces: Vec<_> = spans
        .iter()
        .map(|part| {
            span(part.text.clone()).font(if part.bold {
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

fn spoiler<'a>(review: &Review, index: usize) -> Element<'a, Message> {
    button(text("Spoiler").size(font::NOTE_SIZE))
        .padding([2, 10])
        .style(style::chip)
        .on_press(Message::ShowSpoiler(review.id.clone(), index))
        .into()
}

fn thumb(recommended: bool) -> char {
    if recommended { THUMB_UP } else { THUMB_DOWN }
}

fn heading(review: &Review, now: Timestamp) -> Element<'_, Message> {
    let thumb = text(thumb(review.recommended))
        .font(font::ICONS)
        .size(font::icon(THUMB_SIZE));
    let verdict = text(format::verdict(review.recommended))
        .size(font::NOTE_SIZE)
        .font(font::SEMIBOLD);
    let verdict = container(row![thumb, verdict].spacing(6).align_y(Center))
        .padding([3, 10])
        .style(style::review_verdict(review.recommended));

    row![
        verdict,
        text(format::review_facts(review, now))
            .size(font::NOTE_SIZE)
            .style(style::muted),
    ]
    .spacing(12)
    .align_y(Center)
    .wrap()
    .vertical_spacing(6)
    .into()
}

impl Showcase {
    pub(super) fn review_filters(&self, id: AppId) -> Element<'_, Message> {
        let query = self.reviews.query();
        let matching = self.reviews.get(id).and_then(|list| list.matching);
        let verdicts = row(VERDICTS.map(|verdict| {
            let chosen = verdict == query.verdict;
            button(text(verdict_label(verdict, matching.filter(|_| chosen))).size(font::NOTE_SIZE))
                .padding([3, 10])
                .style(style::choice(chosen))
                .on_press(Message::FilterReviews(ReviewQuery { verdict, ..query }))
                .into()
        }))
        .spacing(6);
        let played = row![
            text("Played at least")
                .size(font::NOTE_SIZE)
                .style(style::muted),
            text_input("Any", &self.hours_typed)
                .on_input(Message::TypeHours)
                .on_submit(Message::ApplyHours(self.hours_typed.clone()))
                .style(style::field)
                .size(font::NOTE_SIZE)
                .padding([4, 8])
                .width(HOURS_WIDTH),
            text("h").size(font::NOTE_SIZE).style(style::muted),
        ]
        .spacing(8)
        .align_y(Center);

        row![
            dropdown(&ORDERS, Sorting(query.order), move |Sorting(order)| {
                Message::FilterReviews(ReviewQuery { order, ..query })
            }),
            verdicts,
            dropdown(
                &LANGUAGES,
                Speaking(query.languages),
                move |Speaking(languages)| {
                    Message::FilterReviews(ReviewQuery { languages, ..query })
                }
            ),
            played,
        ]
        .spacing(FILTER_SPACING)
        .align_y(Center)
        .wrap()
        .vertical_spacing(8)
        .into()
    }

    pub(super) fn reviews_body(&self, id: AppId) -> Element<'_, Message> {
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());
        let list = self.reviews.get(id);
        let reviews = list.map_or(&[][..], |list| list.reviews.as_slice());
        let fell_back = self.reviews.fell_back(id) && !list.is_some_and(|list| list.found_none());
        let mut body = column![]
            .push(fell_back.then(|| {
                sheet_note("No helpful reviews in the past year, so these are the most recent")
            }))
            .spacing(REVIEW_SPACING);
        for review in reviews {
            body = body.push(self.review(review, now));
        }

        body.push(self.reviews_end(id, list.map(|list| list.fetch), reviews.len()))
            .into()
    }

    fn review<'a>(&'a self, review: &'a Review, now: Timestamp) -> Element<'a, Message> {
        let short = if self.expanded.contains(&review.id) {
            None
        } else {
            shortened(&review.text, SHORT_CHARS)
        };
        let blocks = short
            .as_deref()
            .unwrap_or(&review.text)
            .iter()
            .enumerate()
            .map(|(index, block)| self.review_block(review, index, block));
        let more = short.is_some().then(|| {
            button(text("Read more").size(font::NOTE_SIZE))
                .padding([2, 6])
                .style(style::text_button)
                .on_press(Message::ExpandReview(review.id.clone()))
        });

        container(
            column![heading(review, now)]
                .push(column(blocks).spacing(BLOCK_SPACING))
                .push(more)
                .spacing(BLOCK_SPACING),
        )
        .padding(CARD_PADDING)
        .width(Fill)
        .style(style::review_card)
        .into()
    }

    fn review_block<'a>(
        &self,
        review: &Review,
        index: usize,
        block: &Block,
    ) -> Element<'a, Message> {
        match block {
            Block::Paragraph(spans) => paragraph(spans, false),
            Block::Heading(heading) => text(heading.clone())
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
                .padding(QUOTE_PADDING)
                .width(Fill)
                .style(style::review_quote)
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
                .key((id, self.reviews.shown(id), shown))
                .anticipate(LOAD_AHEAD)
                .on_show(|_| Message::MoreReviews)
                .into(),
        }
    }

    fn open_reviews(&self) -> Option<AppId> {
        self.shown.filter(|_| self.popups.is(Popup::Reviews))
    }

    pub(super) fn load_reviews(&mut self) -> Task<Message> {
        let request = self.open_reviews().and_then(|id| self.reviews.start(id));
        self.fetch_reviews(request)
    }

    pub(super) fn more_reviews(&mut self) -> Task<Message> {
        let request = self.open_reviews().and_then(|id| self.reviews.more(id));
        self.fetch_reviews(request)
    }

    pub(super) fn retry_reviews(&mut self) -> Task<Message> {
        let request = self.open_reviews().and_then(|id| self.reviews.retry(id));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filters_read_as_in_the_sheet() {
        assert_eq!(
            ORDERS.map(|order| order.to_string()),
            ["Most helpful", "Most recent", "Funniest"]
        );
        assert_eq!(
            LANGUAGES.map(|language| language.to_string()),
            ["English", "All languages"]
        );
        assert_eq!(LANGUAGES[0].0, Languages::default());
    }

    #[test]
    fn the_hours_field_keeps_a_few_digits() {
        assert_eq!(hours_typed("12a3"), "123");
        assert_eq!(hours_typed("1234567"), "12345");
        assert_eq!(hours_typed("-5"), "5");
    }

    #[test]
    fn any_number_of_hours_is_a_minimum_and_nothing_or_0_is_any() {
        assert_eq!(min_hours("7"), Some(7));
        assert_eq!(min_hours("250"), Some(250));
        assert_eq!(min_hours(""), None);
        assert_eq!(min_hours("0"), None);
    }

    fn words(text: &str) -> Block {
        Block::Paragraph(vec![Span {
            text: text.to_string(),
            bold: false,
        }])
    }

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|block| match block {
                Block::Heading(heading) => heading.clone(),
                Block::Paragraph(spans)
                | Block::Item(spans)
                | Block::Quote(spans)
                | Block::Spoiler(spans) => spans.iter().map(|part| part.text.as_str()).collect(),
            })
            .collect()
    }

    #[test]
    fn a_short_review_is_not_cut() {
        assert_eq!(
            shortened(&[words("Great game"), words("Buy it")], 200),
            None
        );
    }

    #[test]
    fn a_long_review_is_cut_at_a_word_with_an_ellipsis() {
        let long = "word ".repeat(100);

        let short = shortened(&[words("Intro"), words(&long)], 200).unwrap();

        let texts = texts(&short);
        assert_eq!(texts[0], "Intro");
        assert!(texts[1].ends_with("word…"), "{}", texts[1]);
        assert!(texts[1].chars().count() <= 200 - 2 * BLOCK_CHARS);
    }

    #[test]
    fn many_short_blocks_count_as_lines() {
        let items: Vec<_> = (0..30).map(|_| words("ok")).collect();

        let short = shortened(&items, 200).unwrap();

        assert_eq!(short.len(), 200 / (2 + BLOCK_CHARS));
    }

    #[test]
    fn a_spoiler_is_never_cut() {
        let secret = Block::Spoiler(vec![Span {
            text: "secret ".repeat(100),
            bold: false,
        }]);
        let long = "word ".repeat(100);

        let short = shortened(&[words("Intro"), secret.clone(), words(&long)], 200).unwrap();

        assert_eq!(short[1], secret);
    }

    #[test]
    fn a_long_word_is_cut_inside() {
        assert_eq!(cut_text("abcdefghij", 4), "abcd…");
        assert_eq!(cut_text("one two three", 9), "one two…");
    }

    #[test]
    fn the_thumb_follows_the_verdict() {
        assert_eq!(thumb(true), THUMB_UP);
        assert_eq!(thumb(false), THUMB_DOWN);
    }

    #[test]
    fn the_chosen_verdict_shows_how_many_reviews_match() {
        assert_eq!(verdict_label(Verdict::All, Some(2287)), "All 2,287");
        assert_eq!(verdict_label(Verdict::Negative, Some(0)), "Negative 0");
        assert_eq!(verdict_label(Verdict::Positive, None), "Positive");
    }
}
