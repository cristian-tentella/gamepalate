use std::process::Command;

use gamepalate_app::{Currency, Price, Reviews, VoteKind};
use iced::{
    Center, Element, Fill, never,
    widget::{button, container, rich_text, row, span, text},
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

pub fn reviews_line<'a, Message: 'a>(reviews: &Reviews) -> Element<'a, Message> {
    let (percent, rest) = format::reviews(reviews);

    rich_text![span(percent).font(font::SEMIBOLD), span(rest)]
        .size(font::INFO_SIZE)
        .on_link_click(never)
        .into()
}

pub fn vote_button<'a, Message: Clone + 'a>(
    key: &'a str,
    kind: VoteKind,
    compact: bool,
    on_press: Option<Message>,
    flashed: bool,
) -> Element<'a, Message> {
    let mark = if flashed {
        text(CHECK).font(font::ICONS).size(10)
    } else {
        text(key).size(11).style(style::muted)
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
    .width(Fill)
    .max_width(VOTE_WIDTH)
    .into()
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
