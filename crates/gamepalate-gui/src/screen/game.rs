use std::process::Command;

use gamepalate_app::{Currency, Price, Reviews, VoteKind};
use iced::{
    Center, Element, Fill, Length, never, padding,
    widget::{
        Container, Row, button, column, container, rich_text, row, span, stack, text, tooltip,
    },
};

use crate::{font, format, style};

pub const VOTES: [(&str, VoteKind); 4] = [
    ("1", VoteKind::WouldTry),
    ("2", VoteKind::Maybe),
    ("3", VoteKind::NotForMe),
    ("4", VoteKind::Known),
];
pub const VOTE_HEIGHT: f32 = 36.0;
const VOTE_WIDTH: f32 = 172.0;
const VOTE_SPACING: f32 = 8.0;
pub const LINKS_WIDTH: f32 = 380.0;
const KEYCAP_SIZE: f32 = 18.0;
const KEYCAP_PADDING: f32 = 5.0;
const KEY_SPACING: f32 = 6.0;
const TITLE_LINE_HEIGHT: f32 = 1.1;
const BET_INSET: f32 = 8.0;
const BADGE_PADDING: f32 = 2.0;
pub const PRICE_GAP: f32 = 4.0;
const COMPACT_VOTE_SIZE: f32 = 14.0;
const CHECK: char = '\u{E73E}';

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Link {
    Steam,
    Eneba,
    Hltb,
}

pub fn vote_key(key: &str) -> Option<VoteKind> {
    VOTES
        .iter()
        .find(|(vote_key, _)| *vote_key == key)
        .map(|(_, kind)| *kind)
}

pub fn link_key(key: &str) -> Option<Link> {
    match key {
        "5" => Some(Link::Steam),
        "6" => Some(Link::Eneba),
        "7" => Some(Link::Hltb),
        _ => None,
    }
}

pub fn open(link: Link, name: &str, store_url: &str) {
    let url = match link {
        Link::Steam => store_url.to_string(),
        Link::Eneba => format::eneba_url(name),
        Link::Hltb => format::hltb_url(name),
    };
    open_url(&url);
}

pub fn open_url(url: &str) {
    let _ = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
}

pub fn title<'a, Message: 'a>(name: &'a str) -> Element<'a, Message> {
    text(name)
        .size(font::TITLE_SIZE)
        .font(font::SEMIBOLD)
        .line_height(TITLE_LINE_HEIGHT)
        .into()
}

pub fn with_bet<'a, Message: 'a>(
    header: Element<'a, Message>,
    bet_every: Option<usize>,
) -> Element<'a, Message> {
    match bet_every {
        Some(every) => {
            let label = tooltip(
                bet_label(),
                container(text(format::bet(every)).size(font::NOTE_SIZE))
                    .padding([6, 10])
                    .style(style::popup),
                tooltip::Position::Bottom,
            );

            stack![header, container(label).padding(BET_INSET)].into()
        }
        None => header,
    }
}

pub fn beside_reviews() -> (f32, f32) {
    let shift = font::baseline(font::STAT_SIZE, font::STAT_LINE_HEIGHT)
        - font::baseline(font::AMOUNT_SIZE, font::PRICE_LINE_HEIGHT);
    let gap = font::STAT_SIZE * font::STAT_LINE_HEIGHT
        - shift
        - font::AMOUNT_SIZE * font::PRICE_LINE_HEIGHT;

    (shift, gap)
}

pub fn price_row<'a, Message: 'a>(
    price: &Price,
    currency: &Currency,
    gap: f32,
) -> Element<'a, Message> {
    let discount = price.discount;
    let main = row![
        text(format::price(price, currency))
            .size(font::AMOUNT_SIZE)
            .font(font::SEMIBOLD)
            .line_height(font::PRICE_LINE_HEIGHT)
    ]
    .push(discount.map(|discount| discount_badge(discount.percent)))
    .spacing(12)
    .align_y(Center);
    let context = discount.map(|discount| {
        let ends = discount.end.and_then(format::ends).map(|ends| {
            row![
                text("·").size(font::NOTE_SIZE).style(style::muted),
                text(ends).size(font::NOTE_SIZE).style(style::muted),
            ]
            .spacing(6)
        });

        row![
            rich_text![span(format::original_price(price, currency)).strikethrough(true)]
                .size(font::NOTE_SIZE)
                .style(style::muted)
                .on_link_click(never)
        ]
        .push(ends)
        .spacing(6)
    });

    column![main].push(context).spacing(gap).into()
}

fn discount_badge<'a, Message: 'a>(percent: u8) -> Element<'a, Message> {
    container(
        text(format::discount(percent))
            .size(font::NOTE_SIZE)
            .font(font::SEMIBOLD),
    )
    .padding([BADGE_PADDING, 8.0])
    .style(style::discount_badge)
    .into()
}

pub fn bet_label<'a, Message: 'a>() -> Element<'a, Message> {
    container(text("Bet").size(font::NOTE_SIZE).font(font::SEMIBOLD))
        .padding([2, 8])
        .style(style::bet)
        .into()
}

pub fn reviews<'a, Message: 'a>(reviews: &Reviews) -> Element<'a, Message> {
    match format::reviews(reviews) {
        (percent, label) if !percent.is_empty() => column![
            text(percent)
                .size(font::STAT_SIZE)
                .font(font::BOLD)
                .line_height(font::STAT_LINE_HEIGHT)
                .style(style::accent),
            text(label).size(font::NOTE_SIZE).style(style::muted),
        ]
        .into(),
        (_, label) => text(label).size(font::INFO_SIZE).style(style::muted).into(),
    }
}

pub fn vote_button<'a, Message: Clone + 'a>(
    key: &'a str,
    kind: VoteKind,
    compact: bool,
    width: Length,
    on_press: Option<Message>,
    flashed: bool,
) -> Element<'a, Message> {
    let keycap = if flashed {
        container(text(CHECK).font(font::ICONS).size(10))
            .center_x(KEYCAP_SIZE)
            .center_y(KEYCAP_SIZE)
            .style(style::keycap(true))
    } else {
        keycap(key)
    };
    let label = text(format::vote(kind))
        .size(if compact {
            COMPACT_VOTE_SIZE
        } else {
            font::VOTE_SIZE
        })
        .font(font::SEMIBOLD);

    container(
        button(
            container(row![keycap, label].spacing(10).align_y(Center))
                .center_x(Fill)
                .center_y(Fill),
        )
        .width(Fill)
        .height(VOTE_HEIGHT)
        .padding(0)
        .style(style::vote(flashed))
        .on_press_maybe(on_press),
    )
    .width(width)
    .into()
}

pub fn keycap<'a, Message: 'a>(key: &'a str) -> Container<'a, Message> {
    let keycap = container(text(key).size(font::NOTE_SIZE))
        .center_y(KEYCAP_SIZE)
        .style(style::keycap(false));

    if key.chars().count() == 1 {
        keycap.center_x(KEYCAP_SIZE)
    } else {
        keycap.padding(padding::left(KEYCAP_PADDING).right(KEYCAP_PADDING))
    }
}

pub fn hint<'a, Message: 'a>(key: &'a str, label: &'a str) -> Element<'a, Message> {
    row![
        keycap(key),
        text(label).size(font::NOTE_SIZE).style(style::muted)
    ]
    .spacing(KEY_SPACING)
    .align_y(Center)
    .into()
}

pub fn vote_buttons<'a, Message: 'a>(
    button: impl Fn(&'a str, VoteKind) -> Element<'a, Message>,
) -> Row<'a, Message> {
    let [would_try, maybe, not_for_me, known] = VOTES.map(|(key, kind)| button(key, kind));

    row![would_try, maybe, not_for_me, style::divider(), known]
        .spacing(VOTE_SPACING)
        .align_y(Center)
}

pub fn vote_width(available: f32, beside: f32) -> Length {
    let divider = 1.0 + 2.0 * style::DIVIDER_INSET + VOTE_SPACING;
    let needed = 4.0 * (VOTE_WIDTH + VOTE_SPACING) + divider + beside;

    if available >= needed {
        Length::Fixed(VOTE_WIDTH)
    } else {
        Length::Fill
    }
}

pub fn link_button<'a, Message: Clone + 'a>(
    key: &'a str,
    label: &'a str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    button(
        row![keycap(key), text(label).size(font::BAR_SIZE)]
            .spacing(KEY_SPACING)
            .align_y(Center),
    )
    .padding(0)
    .style(style::link)
    .on_press_maybe(on_press)
    .into()
}

pub fn links<'a, Message: Clone + 'a>(
    on_press: impl Fn(Link) -> Option<Message>,
    about: Option<Message>,
) -> Element<'a, Message> {
    row![
        link_button("D", "About", about),
        link_button("5", "Steam ↗", on_press(Link::Steam)),
        link_button("6", "Eneba ↗", on_press(Link::Eneba)),
        link_button("7", "HLTB ↗", on_press(Link::Hltb)),
    ]
    .spacing(20)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beside_the_reviews_the_price_lines_up_with_them() {
        let (shift, gap) = beside_reviews();
        let price_height = font::AMOUNT_SIZE * font::PRICE_LINE_HEIGHT;
        let price_baseline = shift + font::baseline(font::AMOUNT_SIZE, font::PRICE_LINE_HEIGHT);

        assert!(
            (price_baseline - font::baseline(font::STAT_SIZE, font::STAT_LINE_HEIGHT)).abs() < 1e-4
        );
        assert!(
            (shift + price_height + gap - font::STAT_SIZE * font::STAT_LINE_HEIGHT).abs() < 1e-4
        );
        assert!(gap >= 0.0);
        assert!(font::NOTE_SIZE * 1.3 + 2.0 * BADGE_PADDING <= price_height);
    }

    #[test]
    fn the_vote_buttons_stop_growing_once_there_is_room() {
        assert_eq!(vote_width(760.0, 0.0), Length::Fixed(VOTE_WIDTH));
        assert_eq!(vote_width(720.0, 0.0), Length::Fill);
        assert_eq!(vote_width(956.0, 180.0), Length::Fixed(VOTE_WIDTH));
        assert_eq!(vote_width(900.0, 180.0), Length::Fill);
    }

    #[test]
    fn the_keys_from_one_to_four_are_the_votes() {
        assert_eq!(vote_key("1"), Some(VoteKind::WouldTry));
        assert_eq!(vote_key("2"), Some(VoteKind::Maybe));
        assert_eq!(vote_key("3"), Some(VoteKind::NotForMe));
        assert_eq!(vote_key("4"), Some(VoteKind::Known));
        assert_eq!(vote_key("5"), None);
    }

    #[test]
    fn the_keys_from_five_to_seven_are_the_links() {
        assert_eq!(link_key("5"), Some(Link::Steam));
        assert_eq!(link_key("6"), Some(Link::Eneba));
        assert_eq!(link_key("7"), Some(Link::Hltb));
        assert_eq!(link_key("4"), None);
    }
}
