use std::collections::HashMap;

use gamepalate_core::ids::AppId;
use serde::Deserialize;
use serde_json::Value;

use crate::transport::{Get, SteamError, parse};

const APP_DETAILS_ENDPOINT: &str = "https://store.steampowered.com/api/appdetails";
const ABOUT_HEADINGS: [&str; 2] = ["about the game", "about this game"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Paragraph(Vec<Span>),
    List(Vec<Vec<Span>>),
    Image {
        url: String,
        width: u32,
        height: u32,
    },
    Video {
        url: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub link: Option<String>,
}

#[derive(Deserialize)]
struct AppDetails {
    success: bool,
    data: Option<AppData>,
}

#[derive(Deserialize)]
struct AppData {
    about_the_game: Option<String>,
}

pub async fn get_description<S: Get>(
    transport: &S,
    id: AppId,
    language: &str,
) -> Result<Option<Vec<Block>>, SteamError> {
    let appids = id.0.to_string();
    let value = transport
        .get(
            APP_DETAILS_ENDPOINT,
            &[("appids", &appids), ("l", language), ("filters", "basic")],
            false,
        )
        .await?;
    parse_description(id, value)
}

fn parse_description(id: AppId, value: Value) -> Result<Option<Vec<Block>>, SteamError> {
    let mut apps: HashMap<String, AppDetails> = parse(APP_DETAILS_ENDPOINT, value)?;

    Ok(apps
        .remove(&id.0.to_string())
        .filter(|app| app.success)
        .and_then(|app| app.data?.about_the_game)
        .map(|html| blocks(&html)))
}

pub fn blocks(html: &str) -> Vec<Block> {
    let mut reader = Reader::default();
    for token in tokens(html) {
        reader.read(token);
    }
    reader.finish()
}

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Open {
        name: String,
        attributes: Vec<(String, String)>,
    },
    Close(String),
}

fn tokens(html: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.split_once("-->").map_or("", |(_, after)| after);
            continue;
        }
        if let Some((tag, after)) = rest.strip_prefix('<').and_then(|tag| tag.split_once('>')) {
            tokens.extend(tag_token(tag));
            rest = after;
            continue;
        }
        let first = rest.chars().next().map_or(0, char::len_utf8);
        let end = rest[first..]
            .find('<')
            .map_or(rest.len(), |index| index + first);
        tokens.push(Token::Text(decode(&rest[..end])));
        rest = &rest[end..];
    }
    tokens
}

fn tag_token(tag: &str) -> Option<Token> {
    let tag = tag.trim().trim_end_matches('/').trim_end();
    if let Some(name) = tag.strip_prefix('/') {
        return Some(Token::Close(name.trim().to_ascii_lowercase()));
    }
    let (name, rest) = tag
        .split_once(|c: char| c.is_ascii_whitespace())
        .unwrap_or((tag, ""));
    if name.is_empty() || name.starts_with('!') {
        return None;
    }

    Some(Token::Open {
        name: name.to_ascii_lowercase(),
        attributes: attributes(rest),
    })
}

fn attributes(mut rest: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return found;
        }
        let name_end = rest
            .find(|c: char| c == '=' || c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let value = match rest.strip_prefix('=') {
            Some(after) => {
                let after = after.trim_start();
                let (value, remaining) = match after.chars().next() {
                    Some(quote @ ('"' | '\'')) => {
                        after[1..].split_once(quote).unwrap_or((&after[1..], ""))
                    }
                    _ => after.split_at(
                        after
                            .find(|c: char| c.is_ascii_whitespace())
                            .unwrap_or(after.len()),
                    ),
                };
                rest = remaining;
                decode(value)
            }
            None => String::new(),
        };
        if !name.is_empty() {
            found.push((name, value));
        }
    }
}

fn decode(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        let found = rest[1..]
            .find(';')
            .filter(|&end| end <= 10)
            .and_then(|end| Some((entity(&rest[1..1 + end])?, end + 2)));
        match found {
            Some((c, length)) => {
                decoded.push(c);
                rest = &rest[length..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

fn collapse(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_ascii_whitespace() {
            space = true;
        } else {
            if space {
                collapsed.push(' ');
            }
            space = false;
            collapsed.push(c);
        }
    }
    if space {
        collapsed.push(' ');
    }
    collapsed
}

#[derive(Default)]
struct Reader {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    breaks: usize,
    bold: usize,
    italic: usize,
    links: Vec<Option<String>>,
    heading: Option<String>,
    list: Option<Vec<Vec<Span>>>,
    in_item: bool,
    video: Option<Video>,
}

struct Video {
    mp4: Option<String>,
    other: Option<String>,
}

impl Reader {
    fn read(&mut self, token: Token) {
        match token {
            Token::Text(text) => self.text(&text),
            Token::Open { name, attributes } => self.open(&name, &attributes),
            Token::Close(name) => self.close(&name),
        }
    }

    fn text(&mut self, text: &str) {
        if self.video.is_some() {
            return;
        }
        if let Some(heading) = &mut self.heading {
            heading.push_str(text);
            return;
        }
        let text = collapse(text);
        if text.trim_matches(' ').is_empty() {
            if self.breaks == 0 && !self.spans.is_empty() {
                self.push(" ");
            }
            return;
        }
        if self.breaks >= 2 && !self.in_item {
            self.end_paragraph();
        } else if self.breaks >= 1 && !self.spans.is_empty() {
            self.push("\n");
        }
        self.breaks = 0;
        self.push(&text);
    }

    fn open(&mut self, name: &str, attributes: &[(String, String)]) {
        let attribute = |wanted: &str| {
            attributes
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.clone())
        };
        match name {
            "br" if self.heading.is_none() => self.breaks += 1,
            "p" | "div" => self.end_paragraph(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.end_paragraph();
                self.heading = Some(String::new());
            }
            "strong" | "b" => self.bold += 1,
            "i" | "em" => self.italic += 1,
            "a" => self
                .links
                .push(attribute("href").filter(|href| !href.is_empty())),
            "ul" | "ol" if self.list.is_none() => {
                self.end_paragraph();
                self.list = Some(Vec::new());
            }
            "li" if self.list.is_some() => {
                self.end_item();
                self.in_item = true;
            }
            "img" => {
                if let Some(url) = attribute("src").filter(|src| !src.is_empty()) {
                    let size = |name| attribute(name).and_then(|value| value.parse().ok());
                    self.end_paragraph();
                    self.blocks.push(Block::Image {
                        url,
                        width: size("width").unwrap_or(0),
                        height: size("height").unwrap_or(0),
                    });
                }
            }
            "video" => {
                self.end_paragraph();
                self.video = Some(Video {
                    mp4: None,
                    other: attribute("src").filter(|src| !src.is_empty()),
                });
            }
            "source" => {
                if let (Some(video), Some(url)) = (&mut self.video, attribute("src")) {
                    let is_mp4 = attribute("type")
                        .is_some_and(|kind| kind.starts_with("video/mp4"))
                        || url
                            .split('?')
                            .next()
                            .is_some_and(|path| path.ends_with(".mp4"));
                    if is_mp4 {
                        video.mp4.get_or_insert(url);
                    } else {
                        video.other.get_or_insert(url);
                    }
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        match name {
            "strong" | "b" => self.bold = self.bold.saturating_sub(1),
            "i" | "em" => self.italic = self.italic.saturating_sub(1),
            "a" => {
                self.links.pop();
            }
            "p" | "div" => self.end_paragraph(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if let Some(heading) = self.heading.take() {
                    let heading = collapse(&heading).trim().to_string();
                    if !heading.is_empty() {
                        self.blocks.push(Block::Heading(heading));
                    }
                }
                self.breaks = 0;
            }
            "li" => self.end_item(),
            "ul" | "ol" => self.end_list(),
            "video" => {
                if let Some(video) = self.video.take()
                    && let Some(url) = video.mp4.or(video.other)
                {
                    self.blocks.push(Block::Video { url });
                }
                self.breaks = 0;
            }
            _ => {}
        }
    }

    fn push(&mut self, text: &str) {
        let bold = self.bold > 0;
        let italic = self.italic > 0;
        let link = self.links.last().cloned().flatten();
        match self.spans.last_mut() {
            Some(last) if last.bold == bold && last.italic == italic && last.link == link => {
                last.text.push_str(text);
            }
            _ => self.spans.push(Span {
                text: text.to_string(),
                bold,
                italic,
                link,
            }),
        }
    }

    fn end_paragraph(&mut self) {
        if self.in_item {
            return;
        }
        let spans = tidy(std::mem::take(&mut self.spans));
        if !spans.is_empty() {
            self.blocks.push(Block::Paragraph(spans));
        }
        self.breaks = 0;
    }

    fn end_item(&mut self) {
        if self.in_item {
            self.in_item = false;
            let spans = tidy(std::mem::take(&mut self.spans));
            if let Some(list) = &mut self.list
                && !spans.is_empty()
            {
                list.push(spans);
            }
        }
        self.breaks = 0;
    }

    fn end_list(&mut self) {
        self.end_item();
        self.end_paragraph();
        if let Some(items) = self.list.take()
            && !items.is_empty()
        {
            self.blocks.push(Block::List(items));
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.end_list();
        self.end_paragraph();
        if let Some(Block::Heading(heading)) = self.blocks.first()
            && ABOUT_HEADINGS.contains(&heading.to_lowercase().as_str())
        {
            self.blocks.remove(0);
        }
        self.blocks
    }
}

fn tidy(spans: Vec<Span>) -> Vec<Span> {
    let mut tidy: Vec<Span> = Vec::new();
    for mut span in spans {
        span.text = span.text.replace(" \n", "\n").replace("\n ", "\n");
        if tidy.is_empty() {
            span.text = span.text.trim_start_matches([' ', '\n']).to_string();
        } else if tidy
            .last()
            .is_some_and(|last| last.text.ends_with([' ', '\n']))
        {
            span.text = span.text.trim_start_matches(' ').to_string();
        }
        if span.text.starts_with('\n')
            && let Some(last) = tidy.last_mut()
        {
            last.text.truncate(last.text.trim_end_matches(' ').len());
        }
        if !span.text.is_empty() {
            tidy.push(span);
        }
    }
    while let Some(last) = tidy.last_mut() {
        last.text
            .truncate(last.text.trim_end_matches([' ', '\n']).len());
        if !last.text.is_empty() {
            break;
        }
        tidy.pop();
    }
    tidy.retain(|span| !span.text.is_empty());
    tidy
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn plain(text: &str) -> Span {
        Span {
            text: text.to_string(),
            ..Span::default()
        }
    }

    fn bold(text: &str) -> Span {
        Span {
            bold: true,
            ..plain(text)
        }
    }

    fn paragraph(text: &str) -> Block {
        Block::Paragraph(vec![plain(text)])
    }

    #[test]
    fn double_breaks_split_paragraphs_and_single_ones_break_lines() {
        assert_eq!(
            blocks("A farm game! <br><br>You inherit\na farm.<br>It needs work.<br><br>"),
            [
                paragraph("A farm game!"),
                paragraph("You inherit a farm.\nIt needs work."),
            ]
        );
    }

    #[test]
    fn reads_headings_and_lists_with_bold_inside() {
        let html = r#"<strong>Gameplay</strong><br>Runs are random.<h2 class="bb_tag" >Key Features:</h2><ul class="bb_ul"><li><strong>8 players.</strong> Invite friends.<br><br></li><li>Over 100 seeds<br></li></ul><br>The end."#;

        assert_eq!(
            blocks(html),
            [
                Block::Paragraph(vec![bold("Gameplay"), plain("\nRuns are random.")]),
                Block::Heading("Key Features:".to_string()),
                Block::List(vec![
                    vec![bold("8 players."), plain(" Invite friends.")],
                    vec![plain("Over 100 seeds")],
                ]),
                paragraph("The end."),
            ]
        );
    }

    #[test]
    fn reads_an_image_with_its_size_between_paragraphs() {
        let html = r#"Before. <br><br> <span class="bb_img_ctn"><img class="bb_img" src="https://cdn/extras/a.avif?t=1" width=600 height=105 /></span><br><br>After."#;

        assert_eq!(
            blocks(html),
            [
                paragraph("Before."),
                Block::Image {
                    url: "https://cdn/extras/a.avif?t=1".to_string(),
                    width: 600,
                    height: 105,
                },
                paragraph("After."),
            ]
        );
    }

    #[test]
    fn an_image_without_a_size_is_zero_by_zero() {
        assert_eq!(
            blocks(r#"<img src="https://cdn/b.webp">"#),
            [Block::Image {
                url: "https://cdn/b.webp".to_string(),
                width: 0,
                height: 0,
            }]
        );
    }

    #[test]
    fn a_video_gives_its_mp4() {
        let html = r#"<span class="bb_img_ctn"><video class="bb_img" autoplay muted loop playsinline poster="https://cdn/v.poster.avif?t=1" width=600 height=338 ><source src="https://cdn/v.webm?t=1" type="video/webm; codecs=vp9"><source src="https://cdn/v.mp4?t=1" type="video/mp4"></video></span><br><br>Text."#;

        assert_eq!(
            blocks(html),
            [
                Block::Video {
                    url: "https://cdn/v.mp4?t=1".to_string()
                },
                paragraph("Text."),
            ]
        );
    }

    #[test]
    fn a_video_with_only_webm_gives_the_webm() {
        assert_eq!(
            blocks(r#"<video><source src="https://cdn/v.webm" type="video/webm"></video>"#),
            [Block::Video {
                url: "https://cdn/v.webm".to_string()
            }]
        );
    }

    #[test]
    fn text_in_a_list_without_items_is_a_paragraph() {
        assert_eq!(
            blocks(
                r#"<img src="https://cdn/co-op.webp" width=616 height=27 /><br><ul class="bb_ul">allows you to team up. </ul><br>Next."#
            ),
            [
                Block::Image {
                    url: "https://cdn/co-op.webp".to_string(),
                    width: 616,
                    height: 27,
                },
                paragraph("allows you to team up."),
                paragraph("Next."),
            ]
        );
    }

    #[test]
    fn keeps_text_that_starts_with_a_wide_character() {
        assert_eq!(
            blocks(
                "<p class=\"bb_paragraph\" >\u{2022} Solve puzzles <br>\u{2022} Land\u{2019}s End</p>"
            ),
            [paragraph(
                "\u{2022} Solve puzzles\n\u{2022} Land\u{2019}s End"
            )]
        );
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            blocks(
                "Dungeons &amp; Dragons &quot;5e&quot; &#39;rules&#39; &#x2014; a&nbsp;b &unknown; & more"
            ),
            [paragraph(
                "Dungeons & Dragons \"5e\" 'rules' \u{2014} a\u{a0}b &unknown; & more"
            )]
        );
    }

    #[test]
    fn unknown_tags_keep_their_text() {
        assert_eq!(
            blocks("<span class=\"x\">Kept</span> <blink>also kept</blink><!-- not this -->"),
            [paragraph("Kept also kept")]
        );
    }

    #[test]
    fn italic_and_links_are_spans() {
        assert_eq!(
            blocks(r#"Read <i>this</i> on <a href="https://example.com">the site</a>."#),
            [Block::Paragraph(vec![
                plain("Read "),
                Span {
                    italic: true,
                    ..plain("this")
                },
                plain(" on "),
                Span {
                    link: Some("https://example.com".to_string()),
                    ..plain("the site")
                },
                plain("."),
            ])]
        );
    }

    #[test]
    fn the_about_heading_at_the_top_is_dropped() {
        assert_eq!(
            blocks("<h1>About the Game</h1>A roguelike.<h2>About the Game</h2>"),
            [
                paragraph("A roguelike."),
                Block::Heading("About the Game".to_string()),
            ]
        );
    }

    #[test]
    fn an_empty_description_has_no_blocks() {
        assert_eq!(blocks(""), []);
        assert_eq!(blocks("<br><br> <p></p>"), []);
    }

    #[test]
    fn reads_the_description_of_the_asked_game() {
        let value = json!({ "620": { "success": true, "data": {
            "about_the_game": "Portal 2 <strong>co-op</strong>",
            "detailed_description": "<h1>New DLC Available</h1>",
        } } });

        assert_eq!(
            parse_description(AppId(620), value).unwrap(),
            Some(vec![Block::Paragraph(vec![
                plain("Portal 2 "),
                bold("co-op")
            ])])
        );
    }

    #[test]
    fn a_game_without_a_page_or_description_is_none() {
        let missing = json!({ "999": { "success": false } });
        let empty = json!({ "620": { "success": true, "data": {} } });

        assert_eq!(parse_description(AppId(999), missing).unwrap(), None);
        assert_eq!(parse_description(AppId(620), empty).unwrap(), None);
    }

    #[test]
    fn an_unexpected_shape_is_an_error() {
        assert!(parse_description(AppId(620), json!([])).is_err());
    }
}
