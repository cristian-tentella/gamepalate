use gamepalate_core::{ids::AppId, timestamp::Timestamp};
use serde::Deserialize;
use serde_json::Value;

use crate::transport::{Get, SteamError, parse};

const REVIEWS_ENDPOINT: &str = "IUserReviewsService/GetAppReviews/v1";
const PER_PAGE: &str = "10";
const HELPFUL_DAYS: &str = "365";
const FIRST_CURSOR: &str = "*";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Order {
    #[default]
    Helpful,
    Recent,
    Funny,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Verdict {
    #[default]
    All,
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Languages {
    #[default]
    EnglishAndItalian,
    All,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ReviewQuery {
    pub order: Order,
    pub verdict: Verdict,
    pub languages: Languages,
    pub min_hours: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewPage {
    pub reviews: Vec<Review>,
    pub next: Option<String>,
    pub total_matching: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub id: String,
    pub recommended: bool,
    pub text: Vec<Block>,
    pub hours_at_review: f64,
    pub hours_now: f64,
    pub written: Timestamp,
    pub helpful: u32,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading(String),
    Item(Vec<Span>),
    Quote(Vec<Span>),
    Spoiler(Vec<Span>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
}

#[derive(Deserialize)]
struct Response {
    response: Reply,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    reviews: Vec<RawReview>,
    cursor: Option<String>,
    total_matching: Option<u32>,
}

#[derive(Deserialize)]
struct RawReview {
    recommendationid: String,
    author: Author,
    language: String,
    review: String,
    timestamp_created: i64,
    voted_up: bool,
    votes_up: u32,
}

#[derive(Deserialize)]
struct Author {
    #[serde(default)]
    playtime_forever: u32,
    #[serde(default)]
    playtime_at_review: u32,
}

impl ReviewQuery {
    fn params(&self) -> Vec<(&'static str, String)> {
        let filter = match self.order {
            Order::Helpful => "0",
            Order::Recent => "1",
            Order::Funny => "3",
        };
        let review_type = match self.verdict {
            Verdict::All => "0",
            Verdict::Positive => "1",
            Verdict::Negative => "2",
        };
        let mut params = vec![
            ("filter", filter.to_string()),
            ("review_type", review_type.to_string()),
        ];
        if self.order == Order::Helpful {
            params.push(("day_range", HELPFUL_DAYS.to_string()));
        }
        match self.languages {
            Languages::EnglishAndItalian => {
                params.push(("languages[0]", "english".to_string()));
                params.push(("languages[1]", "italian".to_string()));
            }
            Languages::All => params.push(("languages[0]", "all".to_string())),
        }
        if let Some(hours) = self.min_hours {
            params.push(("playtime_min_hours", hours.to_string()));
        }
        params
    }
}

pub async fn get_reviews<S: Get>(
    transport: &S,
    id: AppId,
    query: &ReviewQuery,
    cursor: Option<&str>,
) -> Result<ReviewPage, SteamError> {
    let appid = id.0.to_string();
    let cursor = cursor.unwrap_or(FIRST_CURSOR);
    let query = query.params();
    let mut params: Vec<(&str, &str)> = vec![("appid", &appid)];
    params.extend(query.iter().map(|(name, value)| (*name, value.as_str())));
    params.extend([("num_per_page", PER_PAGE), ("cursor", cursor)]);

    let value = transport.get(REVIEWS_ENDPOINT, &params, false).await?;
    parse_page(value, cursor)
}

fn parse_page(value: Value, asked: &str) -> Result<ReviewPage, SteamError> {
    let Response { response } = parse(REVIEWS_ENDPOINT, value)?;
    let next = response
        .cursor
        .filter(|cursor| !response.reviews.is_empty() && cursor != asked);

    Ok(ReviewPage {
        reviews: response.reviews.into_iter().map(review).collect(),
        next,
        total_matching: response.total_matching,
    })
}

fn review(raw: RawReview) -> Review {
    let hours = |minutes: u32| f64::from(minutes) / 60.0;

    Review {
        id: raw.recommendationid,
        recommended: raw.voted_up,
        text: blocks(&raw.review),
        hours_at_review: hours(raw.author.playtime_at_review),
        hours_now: hours(raw.author.playtime_forever),
        written: Timestamp(raw.timestamp_created * 1000),
        helpful: raw.votes_up,
        language: raw.language,
    }
}

pub fn blocks(bbcode: &str) -> Vec<Block> {
    let mut reader = Reader::default();
    for token in tokens(bbcode) {
        reader.read(token);
    }
    reader.finish()
}

const KNOWN_TAGS: [&str; 19] = [
    "h1", "h2", "h3", "b", "i", "u", "strike", "spoiler", "url", "list", "olist", "*", "quote",
    "code", "hr", "table", "tr", "th", "td",
];

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Text(&'a str),
    Open(String),
    Close(String),
}

fn tokens(bbcode: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut rest = bbcode;
    while !rest.is_empty() {
        if let Some((tag, after)) = rest.strip_prefix('[').and_then(|tag| tag.split_once(']'))
            && let Some(token) = tag_token(tag)
        {
            tokens.push(token);
            rest = after;
            continue;
        }
        let skip = usize::from(rest.starts_with('['));
        let end = rest[skip..]
            .find('[')
            .map_or(rest.len(), |index| index + skip);
        tokens.push(Token::Text(&rest[..end]));
        rest = &rest[end..];
    }
    tokens
}

fn tag_token(tag: &str) -> Option<Token<'static>> {
    let known = |name: &str| {
        let name = name.trim().to_ascii_lowercase();
        KNOWN_TAGS.contains(&name.as_str()).then_some(name)
    };
    match tag.strip_prefix('/') {
        Some(name) => known(name).map(Token::Close),
        None => known(tag.split_once('=').map_or(tag, |(name, _)| name)).map(Token::Open),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Quote,
    Spoiler,
}

#[derive(Default)]
struct Reader {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    breaks: usize,
    bold: usize,
    heading: Option<String>,
    lists: usize,
    in_item: bool,
    containers: Vec<Container>,
    hidden: bool,
}

impl Reader {
    fn read(&mut self, token: Token) {
        match token {
            Token::Text(text) => self.text(text),
            Token::Open(name) => self.open(&name),
            Token::Close(name) => self.close(&name),
        }
    }

    fn text(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.breaks += 1;
            }
            self.line(line);
        }
    }

    fn line(&mut self, line: &str) {
        if let Some(heading) = &mut self.heading {
            if self.breaks > 0 {
                heading.push(' ');
            }
            heading.push_str(line);
            self.breaks = 0;
            return;
        }
        if line.trim().is_empty() {
            if self.breaks == 0 && !self.spans.is_empty() {
                self.push(line);
            }
            return;
        }
        if self.breaks >= 2 && self.plain() {
            self.flush();
        } else if self.breaks >= 1 && !self.spans.is_empty() {
            self.trim_end();
            self.push("\n");
        }
        let line = if self.breaks > 0 || self.spans.is_empty() {
            line.trim_start()
        } else {
            line
        };
        self.breaks = 0;
        self.push(line);
    }

    fn open(&mut self, name: &str) {
        match name {
            "b" => self.bold += 1,
            "h1" | "h2" | "h3" if self.containers.is_empty() && !self.in_item => {
                self.end_heading();
                self.end_paragraph();
                self.heading = Some(String::new());
            }
            "list" | "olist" if self.containers.is_empty() => {
                self.end_heading();
                self.end_item();
                self.end_paragraph();
                self.lists += 1;
            }
            "*" if self.lists > 0 && self.containers.is_empty() => {
                self.end_heading();
                self.end_item();
                self.end_paragraph();
                self.in_item = true;
            }
            "quote" | "spoiler" => {
                self.end_heading();
                if self.containers.is_empty() {
                    self.flush();
                    self.in_item = false;
                } else {
                    self.end_paragraph();
                }
                self.hidden |= name == "spoiler";
                self.containers.push(if name == "spoiler" {
                    Container::Spoiler
                } else {
                    Container::Quote
                });
            }
            "h1" | "h2" | "h3" | "list" | "olist" | "*" | "code" | "hr" | "table" | "tr" | "th"
            | "td" => {
                self.end_heading();
                self.end_paragraph();
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        match name {
            "b" => self.bold = self.bold.saturating_sub(1),
            "h1" | "h2" | "h3" if self.heading.is_some() => self.end_heading(),
            "list" | "olist" if self.lists > 0 && self.containers.is_empty() => {
                self.end_item();
                self.lists -= 1;
            }
            "quote" | "spoiler" => {
                let wanted = if name == "spoiler" {
                    Container::Spoiler
                } else {
                    Container::Quote
                };
                if let Some(index) = self.containers.iter().rposition(|&kind| kind == wanted) {
                    if self.containers.len() == 1 {
                        self.flush();
                    } else {
                        self.end_paragraph();
                    }
                    self.containers.remove(index);
                    self.hidden &= !self.containers.is_empty();
                }
            }
            "h1" | "h2" | "h3" | "list" | "olist" | "code" | "hr" | "table" | "tr" | "th"
            | "td" => self.end_paragraph(),
            _ => {}
        }
    }

    fn plain(&self) -> bool {
        self.containers.is_empty() && !self.in_item
    }

    fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let bold = self.bold > 0;
        match self.spans.last_mut() {
            Some(last) if last.bold == bold => last.text.push_str(text),
            _ => self.spans.push(Span {
                text: text.to_string(),
                bold,
            }),
        }
    }

    fn trim_end(&mut self) {
        while let Some(last) = self.spans.last_mut() {
            last.text.truncate(last.text.trim_end().len());
            if !last.text.is_empty() {
                break;
            }
            self.spans.pop();
        }
    }

    fn end_paragraph(&mut self) {
        if self.plain() {
            self.flush();
        } else {
            self.breaks = self.breaks.max(1);
        }
    }

    fn end_heading(&mut self) {
        if let Some(heading) = self.heading.take() {
            let heading = heading.split_whitespace().collect::<Vec<_>>().join(" ");
            if !heading.is_empty() {
                self.blocks.push(Block::Heading(heading));
            }
            self.breaks = 0;
        }
    }

    fn end_item(&mut self) {
        if self.in_item {
            self.flush();
            self.in_item = false;
        }
    }

    fn flush(&mut self) {
        self.trim_end();
        let spans = std::mem::take(&mut self.spans);
        self.breaks = 0;
        if spans.is_empty() {
            return;
        }
        let block = if self.hidden {
            Block::Spoiler(spans)
        } else if !self.containers.is_empty() {
            Block::Quote(spans)
        } else if self.in_item {
            Block::Item(spans)
        } else {
            Block::Paragraph(spans)
        };
        self.blocks.push(block);
    }

    fn finish(mut self) -> Vec<Block> {
        self.end_heading();
        self.flush();
        self.blocks
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    type Params = Vec<(String, String)>;

    struct FakeSteam {
        reply: Value,
        asked: Mutex<Vec<(String, Params)>>,
    }

    impl FakeSteam {
        fn new(reply: Value) -> FakeSteam {
            FakeSteam {
                reply,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn params(&self) -> Params {
            self.asked.lock().unwrap().last().unwrap().1.clone()
        }
    }

    impl Get for FakeSteam {
        fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> impl Future<Output = Result<Value, SteamError>> + Send {
            self.asked.lock().unwrap().push((
                endpoint.to_string(),
                params
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
            ));
            std::future::ready(Ok(self.reply.clone()))
        }
    }

    // Portal 2, most helpful in English and Italian, 13 November 2026: shortened to three
    // reviews, with made-up Steam IDs.
    fn portal_page() -> Value {
        json!({ "response": {
            "query_summary": {
                "num_reviews": 10, "review_score": 9,
                "review_score_desc": "Overwhelmingly Positive",
                "total_positive": 175110, "total_negative": 2009, "total_reviews": 177119
            },
            "reviews": [
                {
                    "recommendationid": "212304207",
                    "author": {
                        "steamid": "76561190000000001", "num_reviews": 1,
                        "playtime_forever": 273, "playtime_last_two_weeks": 0,
                        "playtime_at_review": 273, "last_played": 1764601138
                    },
                    "language": "english",
                    "review": "[i]Happiness consists in frequent repetition of pleasure[/i]\n\nSuch as when GlaDoS is constantly experimenting on me",
                    "timestamp_created": 1764595557, "timestamp_updated": 1764595816,
                    "voted_up": true, "votes_up": 112, "votes_funny": 7,
                    "weighted_vote_score": 0.9156509637832642, "comment_count": 0,
                    "steam_purchase": true, "received_for_free": false,
                    "written_during_early_access": false, "primarily_steam_deck": false,
                    "refunded": false
                },
                {
                    "recommendationid": "215431128",
                    "author": {
                        "steamid": "76561190000000002", "num_reviews": 3,
                        "playtime_forever": 1038, "playtime_last_two_weeks": 0,
                        "playtime_at_review": 953, "last_played": 1773840989
                    },
                    "language": "english",
                    "review": "The single player campain holds up very well 15 years later.\nIt's atmospheric, challenging and funny :) <3",
                    "timestamp_created": 1767819249, "timestamp_updated": 1767819299,
                    "voted_up": true, "votes_up": 30, "votes_funny": 0,
                    "weighted_vote_score": 0.7815333008766174, "comment_count": 0,
                    "steam_purchase": true, "received_for_free": false,
                    "written_during_early_access": false, "primarily_steam_deck": false,
                    "refunded": false
                },
                {
                    "recommendationid": "224330162",
                    "author": {
                        "steamid": "76561190000000003", "num_reviews": 150,
                        "playtime_forever": 2980, "playtime_last_two_weeks": 0,
                        "playtime_at_review": 1334, "deck_playtime_at_review": 1334,
                        "last_played": 1778491325
                    },
                    "language": "english",
                    "review": "[h1]Portal 2: A Masterclass in Physics, Wit, and Passive-Aggressive Robots[/h1]\r\n[b]It is rare for a sequel to not only live up to its predecessor.[/b]\r\n\r\n[h1]A Masterclass in Narrative Comedy[/h1]\r\n[b]The writing in Portal 2 is, quite frankly, peerless.[/b]",
                    "timestamp_created": 1777431373, "timestamp_updated": 1777431373,
                    "voted_up": true, "votes_up": 61, "votes_funny": 1,
                    "weighted_vote_score": 0.7785949110984802, "comment_count": 0,
                    "steam_purchase": true, "received_for_free": false,
                    "written_during_early_access": false, "primarily_steam_deck": true,
                    "refunded": false,
                    "hardware": { "os": "SteamOS Holo", "system_ram": "14809", "vram_size": 1024 }
                }
            ],
            "cursor": "AoMFQFUbZgAAAAAFP/IOuaAAAABym+DXBg==",
            "day_range_used": 365,
            "total_matching": 13380
        } })
    }

    // The page after the last one, as Steam sends it: no reviews and the same cursor.
    fn past_the_end(cursor: &str) -> Value {
        json!({ "response": {
            "query_summary": { "num_reviews": 0 },
            "cursor": cursor,
            "total_matching": 1
        } })
    }

    fn plain(text: &str) -> Span {
        Span {
            text: text.to_string(),
            bold: false,
        }
    }

    fn bold(text: &str) -> Span {
        Span {
            text: text.to_string(),
            bold: true,
        }
    }

    fn paragraph(text: &str) -> Block {
        Block::Paragraph(vec![plain(text)])
    }

    fn pairs(pairs: &[(&str, &str)]) -> Params {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    async fn asked(query: ReviewQuery) -> Params {
        let steam = FakeSteam::new(portal_page());
        get_reviews(&steam, AppId(620), &query, None).await.unwrap();
        steam.params()
    }

    #[tokio::test]
    async fn the_default_asks_the_most_helpful_of_the_year_in_english_and_italian() {
        let steam = FakeSteam::new(portal_page());

        get_reviews(&steam, AppId(620), &ReviewQuery::default(), None)
            .await
            .unwrap();

        let asked = steam.asked.lock().unwrap();
        assert_eq!(asked[0].0, REVIEWS_ENDPOINT);
        assert_eq!(
            asked[0].1,
            pairs(&[
                ("appid", "620"),
                ("filter", "0"),
                ("review_type", "0"),
                ("day_range", "365"),
                ("languages[0]", "english"),
                ("languages[1]", "italian"),
                ("num_per_page", "10"),
                ("cursor", "*"),
            ])
        );
    }

    #[tokio::test]
    async fn every_order_has_its_number_and_only_helpful_has_a_day_range() {
        for (order, filter) in [
            (Order::Helpful, "0"),
            (Order::Recent, "1"),
            (Order::Funny, "3"),
        ] {
            let params = asked(ReviewQuery {
                order,
                ..ReviewQuery::default()
            })
            .await;

            assert!(params.contains(&("filter".to_string(), filter.to_string())));
            assert_eq!(
                params.iter().any(|(name, _)| name == "day_range"),
                order == Order::Helpful,
                "{order:?}"
            );
        }
    }

    #[tokio::test]
    async fn every_verdict_has_its_number() {
        for (verdict, review_type) in [
            (Verdict::All, "0"),
            (Verdict::Positive, "1"),
            (Verdict::Negative, "2"),
        ] {
            let params = asked(ReviewQuery {
                verdict,
                ..ReviewQuery::default()
            })
            .await;

            assert!(params.contains(&("review_type".to_string(), review_type.to_string())));
        }
    }

    #[tokio::test]
    async fn all_languages_is_one_parameter() {
        let params = asked(ReviewQuery {
            languages: Languages::All,
            ..ReviewQuery::default()
        })
        .await;

        let languages: Vec<_> = params
            .iter()
            .filter(|(name, _)| name.starts_with("languages"))
            .collect();
        assert_eq!(
            languages,
            [&("languages[0]".to_string(), "all".to_string())]
        );
    }

    #[tokio::test]
    async fn the_hours_are_asked_only_when_there_is_a_minimum() {
        let none = asked(ReviewQuery::default()).await;
        let ten = asked(ReviewQuery {
            min_hours: Some(10),
            ..ReviewQuery::default()
        })
        .await;

        assert!(!none.iter().any(|(name, _)| name == "playtime_min_hours"));
        assert!(ten.contains(&("playtime_min_hours".to_string(), "10".to_string())));
    }

    #[tokio::test]
    async fn the_next_page_is_asked_with_the_cursor_of_the_first() {
        let steam = FakeSteam::new(portal_page());

        let first = get_reviews(&steam, AppId(620), &ReviewQuery::default(), None)
            .await
            .unwrap();
        get_reviews(
            &steam,
            AppId(620),
            &ReviewQuery::default(),
            first.next.as_deref(),
        )
        .await
        .unwrap();

        assert_eq!(
            first.next.as_deref(),
            Some("AoMFQFUbZgAAAAAFP/IOuaAAAABym+DXBg==")
        );
        assert!(steam.params().contains(&(
            "cursor".to_string(),
            "AoMFQFUbZgAAAAAFP/IOuaAAAABym+DXBg==".to_string()
        )));
    }

    #[tokio::test]
    async fn reads_a_page_of_reviews() {
        let steam = FakeSteam::new(portal_page());

        let page = get_reviews(&steam, AppId(620), &ReviewQuery::default(), None)
            .await
            .unwrap();

        assert_eq!(page.total_matching, Some(13380));
        assert_eq!(page.reviews.len(), 3);
        assert_eq!(
            page.reviews[0],
            Review {
                id: "212304207".to_string(),
                recommended: true,
                text: vec![
                    paragraph("Happiness consists in frequent repetition of pleasure"),
                    paragraph("Such as when GlaDoS is constantly experimenting on me"),
                ],
                hours_at_review: 273.0 / 60.0,
                hours_now: 273.0 / 60.0,
                written: Timestamp(1_764_595_557_000),
                helpful: 112,
                language: "english".to_string(),
            }
        );
        assert_eq!(
            page.reviews[1].text,
            [paragraph(
                "The single player campain holds up very well 15 years later.\nIt's atmospheric, challenging and funny :) <3"
            )]
        );
        assert_eq!(
            page.reviews[2].text,
            [
                Block::Heading(
                    "Portal 2: A Masterclass in Physics, Wit, and Passive-Aggressive Robots"
                        .to_string()
                ),
                Block::Paragraph(vec![bold(
                    "It is rare for a sequel to not only live up to its predecessor."
                )]),
                Block::Heading("A Masterclass in Narrative Comedy".to_string()),
                Block::Paragraph(vec![bold(
                    "The writing in Portal 2 is, quite frankly, peerless."
                )]),
            ]
        );
    }

    #[test]
    fn the_hours_come_from_the_minutes() {
        let page = parse_page(portal_page(), "*").unwrap();

        assert_eq!(page.reviews[1].hours_at_review, 953.0 / 60.0);
        assert_eq!(page.reviews[1].hours_now, 1038.0 / 60.0);
        assert_eq!(page.reviews[1].written, Timestamp(1_767_819_249_000));
    }

    #[test]
    fn the_page_after_the_last_is_empty_and_has_no_next() {
        let page =
            parse_page(past_the_end("AoJwjPXD/vQCfu2WpgI="), "AoJwjPXD/vQCfu2WpgI=").unwrap();

        assert_eq!(
            page,
            ReviewPage {
                reviews: Vec::new(),
                next: None,
                total_matching: Some(1),
            }
        );
    }

    #[test]
    fn a_page_with_the_same_cursor_has_no_next() {
        let page = parse_page(portal_page(), "AoMFQFUbZgAAAAAFP/IOuaAAAABym+DXBg==").unwrap();

        assert_eq!(page.reviews.len(), 3);
        assert_eq!(page.next, None);
    }

    #[test]
    fn an_empty_page_with_a_new_cursor_has_no_next() {
        let page = parse_page(past_the_end("new"), "*").unwrap();

        assert_eq!(page.next, None);
    }

    #[test]
    fn an_unexpected_shape_is_an_error() {
        assert!(parse_page(json!([]), "*").is_err());
        assert!(parse_page(json!({ "response": { "reviews": [{}] } }), "*").is_err());
    }

    #[test]
    fn text_without_tags_is_one_paragraph() {
        assert_eq!(
            blocks("Great game, 10/10"),
            [paragraph("Great game, 10/10")]
        );
    }

    #[test]
    fn an_empty_review_has_no_blocks() {
        assert_eq!(blocks(""), []);
        assert_eq!(blocks("\r\n \r\n[b][/b]"), []);
    }

    #[test]
    fn double_newlines_split_paragraphs_and_extra_ones_are_dropped() {
        assert_eq!(
            blocks("\r\nFirst line\r\nsecond line  \r\n\r\n\r\n\r\n  Next one\n\n\nLast\r\n\r\n"),
            [
                paragraph("First line\nsecond line"),
                paragraph("Next one"),
                paragraph("Last"),
            ]
        );
    }

    #[test]
    fn bold_stays_and_the_other_styles_leave_their_text() {
        assert_eq!(
            blocks("A [b]great[/b] [i]and[/i] [u]very[/u] [strike]bad[/strike] game"),
            [Block::Paragraph(vec![
                plain("A "),
                bold("great"),
                plain(" and very bad game"),
            ])]
        );
    }

    #[test]
    fn a_link_keeps_only_its_text() {
        assert_eq!(
            blocks("See [url=https://example.com/guide]the guide[/url] or [url]https://x.y[/url]"),
            [paragraph("See the guide or https://x.y")]
        );
    }

    #[test]
    fn headings_are_their_own_blocks() {
        assert_eq!(
            blocks("[h1] The  story [/h1]\r\nIt is short.[h2]Sound[/h2][h3]Art[/h3]"),
            [
                Block::Heading("The story".to_string()),
                paragraph("It is short."),
                Block::Heading("Sound".to_string()),
                Block::Heading("Art".to_string()),
            ]
        );
    }

    #[test]
    fn list_items_are_blocks() {
        assert_eq!(
            blocks(
                "Pros:\r\n[list]\r\n[*]Puzzles\r\n[*][b]Humor[/b], lots\r\n[/list]\r\nCons:[olist][*]Short[/olist]"
            ),
            [
                paragraph("Pros:"),
                Block::Item(vec![plain("Puzzles")]),
                Block::Item(vec![bold("Humor"), plain(", lots")]),
                paragraph("Cons:"),
                Block::Item(vec![plain("Short")]),
            ]
        );
    }

    #[test]
    fn a_quote_and_a_spoiler_are_blocks_with_their_lines() {
        assert_eq!(
            blocks(
                "He said:[quote=GLaDOS]You monster.\r\n\r\nReally.[/quote]The end: [spoiler]it's [b]a cake[/b][/spoiler] Yes."
            ),
            [
                paragraph("He said:"),
                Block::Quote(vec![plain("You monster.\nReally.")]),
                paragraph("The end:"),
                Block::Spoiler(vec![plain("it's "), bold("a cake")]),
                paragraph("Yes."),
            ]
        );
    }

    #[test]
    fn a_spoiler_inside_a_quote_hides_the_whole_quote() {
        assert_eq!(
            blocks("[quote]Did you know [spoiler]the cake[/spoiler] is a lie[/quote]"),
            [Block::Spoiler(vec![plain(
                "Did you know\nthe cake\nis a lie"
            )])]
        );
    }

    #[test]
    fn nested_tags_inside_a_quote_become_lines() {
        assert_eq!(
            blocks("[quote][h1]Title[/h1][list][*]one[*]two[/list][/quote]"),
            [Block::Quote(vec![plain("Title\none\ntwo")])]
        );
    }

    #[test]
    fn code_rules_and_tables_become_paragraphs() {
        assert_eq!(
            blocks(
                "Before[hr][/hr]After[code]fps_max 60[/code][table][tr][th]CPU[/th][td]i5[/td][/tr][/table]"
            ),
            [
                paragraph("Before"),
                paragraph("After"),
                paragraph("fps_max 60"),
                paragraph("CPU"),
                paragraph("i5"),
            ]
        );
    }

    #[test]
    fn a_tag_left_open_lasts_to_the_end() {
        assert_eq!(
            blocks("Fine [b]until here\r\n\r\nand here"),
            [
                Block::Paragraph(vec![plain("Fine "), bold("until here")]),
                Block::Paragraph(vec![bold("and here")]),
            ]
        );
        assert_eq!(
            blocks("Spoiler: [spoiler]never closed"),
            [
                paragraph("Spoiler:"),
                Block::Spoiler(vec![plain("never closed")])
            ]
        );
        assert_eq!(
            blocks("[h1]Open heading"),
            [Block::Heading("Open heading".to_string())]
        );
    }

    #[test]
    fn a_stray_closing_tag_is_dropped() {
        assert_eq!(
            blocks("Odd[/b] text[/spoiler][/list]"),
            [paragraph("Odd text")]
        );
    }

    #[test]
    fn unknown_tags_and_brackets_stay_as_they_are() {
        assert_eq!(
            blocks("[img]x.png[/img] Score [9/10] [ ] [unclosed"),
            [paragraph("[img]x.png[/img] Score [9/10] [ ] [unclosed")]
        );
    }

    #[test]
    fn tag_names_ignore_case() {
        assert_eq!(
            blocks("[B]Loud[/B] [URL=x]link[/Url]"),
            [Block::Paragraph(vec![bold("Loud"), plain(" link")])]
        );
    }
}
