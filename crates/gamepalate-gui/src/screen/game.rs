use std::process::Command;

use gamepalate_app::{Currency, Price, Reviews, VoteKind};
use iced::{
    Center, Element, Fill, Length,
    widget::{button, column, container, row, text},
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
const LINKS_WIDTH: f32 = 240.0;
const STATS_SPACING: f32 = 32.0;
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
    let _ = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn();
}

pub fn price_row<'a, Message: 'a>(
    price: &Price,
    bet: bool,
    currency: &Currency,
) -> Element<'a, Message> {
    let discount = price.discount;

    row![
        text(format::price(price, currency))
            .size(font::PRICE_SIZE)
            .font(font::SEMIBOLD)
    ]
    .push(discount.map(|discount| {
        text(format::discount(discount.percent))
            .size(font::INFO_SIZE)
            .font(font::SEMIBOLD)
            .style(style::discount)
    }))
    .push(discount.and_then(|discount| discount.end).map(|end| {
        text(format::until(end))
            .size(font::NOTE_SIZE)
            .style(style::muted)
    }))
    .push(bet.then(bet_label))
    .spacing(12)
    .align_y(Center)
    .into()
}

pub fn bet_label<'a, Message: 'a>() -> Element<'a, Message> {
    container(text("Bet").size(font::NOTE_SIZE).font(font::SEMIBOLD))
        .padding([2, 8])
        .style(style::bet)
        .into()
}

pub fn stats<'a, Message: 'a>(
    reviews: Option<&Reviews>,
    year: Option<String>,
) -> Element<'a, Message> {
    let reviews = reviews.map(|reviews| match format::reviews(reviews) {
        (percent, label) if !percent.is_empty() => stat(percent, label),
        (_, label) => text(label).size(font::INFO_SIZE).style(style::muted).into(),
    });

    row![]
        .push(reviews)
        .push(year.map(|year| stat(year, "released".to_string())))
        .spacing(STATS_SPACING)
        .align_y(Center)
        .into()
}

fn stat<'a, Message: 'a>(value: String, label: String) -> Element<'a, Message> {
    column![
        text(value).size(font::PRICE_SIZE).font(font::SEMIBOLD),
        text(label).size(font::NOTE_SIZE).style(style::muted),
    ]
    .spacing(2)
    .into()
}

pub fn vote_button<'a, Message: Clone + 'a>(
    key: &'a str,
    kind: VoteKind,
    compact: bool,
    width: Length,
    on_press: Option<Message>,
    flashed: bool,
) -> Element<'a, Message> {
    let mark = if flashed {
        text(CHECK).font(font::ICONS).size(10)
    } else {
        text(key).size(font::NOTE_SIZE).style(style::muted)
    };
    let keycap = container(mark)
        .center_x(18)
        .center_y(18)
        .style(style::keycap(flashed));
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

pub fn vote_width(row_width: f32, beside: f32) -> Length {
    let needed = 4.0 * (VOTE_WIDTH + VOTE_SPACING) + LINKS_WIDTH + beside;

    if row_width >= needed {
        Length::Fixed(VOTE_WIDTH)
    } else {
        Length::Fill
    }
}

pub fn link_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    button(text(label).size(font::BAR_SIZE))
        .padding(0)
        .style(style::link)
        .on_press_maybe(on_press)
        .into()
}

pub fn links<'a, Message: Clone + 'a>(
    on_press: impl Fn(Link) -> Option<Message>,
) -> Element<'a, Message> {
    row![
        link_button("5 Steam ↗", on_press(Link::Steam)),
        link_button("6 Eneba ↗", on_press(Link::Eneba)),
        link_button("7 HLTB ↗", on_press(Link::Hltb)),
    ]
    .spacing(20)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vote_buttons_stop_growing_once_there_is_room() {
        assert_eq!(vote_width(960.0, 0.0), Length::Fixed(VOTE_WIDTH));
        assert_eq!(vote_width(900.0, 0.0), Length::Fill);
        assert_eq!(vote_width(1200.0, 240.0), Length::Fixed(VOTE_WIDTH));
        assert_eq!(vote_width(1100.0, 240.0), Length::Fill);
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
