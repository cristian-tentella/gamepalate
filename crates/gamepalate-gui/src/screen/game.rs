use std::process::Command;

use gamepalate_app::{Currency, LATER_DAYS, Price, Reviews, Vote, VoteKind};
use iced::{
    Center, Element, Fill, Left, Length, never, padding,
    widget::{
        Container, Row, button, column, container, rich_text, row, rule, space, span, stack, text,
        tooltip,
    },
};

use crate::{font, format, style};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Vote(VoteKind),
    NotThisOne,
    Played,
}

impl Pick {
    pub fn of(vote: &Vote) -> Pick {
        if vote.genre_ok {
            Pick::NotThisOne
        } else {
            Pick::Vote(vote.kind)
        }
    }

    /// The kind of vote the pick saves, and whether the genre is fine.
    pub fn ballot(self) -> Option<(VoteKind, bool)> {
        match self {
            Pick::Vote(kind) => Some((kind, false)),
            Pick::NotThisOne => Some((VoteKind::NotForMe, true)),
            Pick::Played => None,
        }
    }
}

pub const PICKS: [(&str, Pick); 4] = [
    ("1", Pick::Vote(VoteKind::WouldTry)),
    ("2", Pick::NotThisOne),
    ("3", Pick::Vote(VoteKind::NotForMe)),
    ("4", Pick::Played),
];
pub const VOTE_HEIGHT: f32 = 36.0;
const VOTE_WIDTH: f32 = 172.0;
const VOTE_SPACING: f32 = 8.0;
const GROUP_SPACING: f32 = 12.0;
pub const LINKS_WIDTH: f32 = 340.0;
pub const ACTION_SPACING: f32 = 16.0;
const KEYCAP_SIZE: f32 = 18.0;
const KEYCAP_PADDING: f32 = 5.0;
const KEY_SPACING: f32 = 6.0;
const TITLE_LINE_HEIGHT: f32 = 1.1;
const BET_INSET: f32 = 8.0;
const BADGE_PADDING: f32 = 2.0;
pub const PRICE_GAP: f32 = 4.0;
const COMPACT_VOTE_SIZE: f32 = 14.0;
const VOTE_INSET: f32 = 16.0;
const CHECK: char = '\u{E668}';

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Link {
    Steam,
    Eneba,
    Hltb,
}

pub fn pick_key(key: &str) -> Option<Pick> {
    PICKS
        .iter()
        .find(|(pick_key, _)| *pick_key == key)
        .map(|(_, pick)| *pick)
}

pub fn link_key(key: &str) -> Option<Link> {
    match key {
        key if key.eq_ignore_ascii_case("q") => Some(Link::Steam),
        key if key.eq_ignore_ascii_case("w") => Some(Link::Eneba),
        key if key.eq_ignore_ascii_case("e") => Some(Link::Hltb),
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

pub fn tip(pick: Pick, would_try_hours: f64, elsewhere_hours: f64) -> String {
    match pick {
        Pick::Vote(VoteKind::WouldTry) => format::would_try_tip(would_try_hours),
        Pick::NotThisOne => format::NOT_THIS_ONE_TIP.to_string(),
        Pick::Vote(VoteKind::NotForMe) => format::NOT_FOR_ME_TIP.to_string(),
        Pick::Vote(VoteKind::Later) => format::later_tip(LATER_DAYS),
        Pick::Played => format::played_tip(elsewhere_hours),
    }
}

pub fn vote_button<'a, Message: Clone + 'a>(
    key: &'a str,
    pick: Pick,
    tip: String,
    compact: bool,
    width: Length,
    on_press: Option<Message>,
    flashed: bool,
) -> Element<'a, Message> {
    let keycap = if flashed {
        container(text(CHECK).font(font::ICONS).size(font::icon(10.0)))
            .center_x(KEYCAP_SIZE)
            .center_y(KEYCAP_SIZE)
            .style(style::keycap(true))
    } else {
        keycap(key)
    };
    let label = text(format::pick(pick))
        .size(if compact {
            COMPACT_VOTE_SIZE
        } else {
            font::VOTE_SIZE
        })
        .font(font::SEMIBOLD)
        .wrapping(text::Wrapping::None);
    // In a narrow row each button takes the width of its label, with room on both sides.
    let content = container(row![keycap, label].spacing(10).align_y(Center)).center_y(Fill);
    let (content, width) = if width == Length::Fill {
        (
            content.padding(padding::left(VOTE_INSET).right(VOTE_INSET)),
            Length::Shrink,
        )
    } else {
        (content.center_x(Fill), width)
    };
    let segment = segment(pick);
    let height = if segment == style::Segment::Alone {
        Length::Fixed(VOTE_HEIGHT)
    } else {
        Length::Fill
    };

    let button = button(content)
        .width(width)
        .height(height)
        .padding(0)
        .style(style::vote(flashed, segment))
        .on_press_maybe(on_press);

    tooltip(
        button,
        container(text(tip).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Top,
    )
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

pub fn key_button<'a, Message: Clone + 'a>(
    key: &'a str,
    label: &'a str,
    size: f32,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    button(
        row![keycap(key), text(label).size(size)]
            .spacing(KEY_SPACING)
            .align_y(Center),
    )
    .padding(0)
    .style(style::key_action)
    .on_press_maybe(on_press)
    .into()
}

pub fn moves<'a, Message: Clone + 'a>(
    later: Option<Message>,
    back: Option<Message>,
) -> Row<'a, Message> {
    let later = tooltip(
        key_button("Space", "Later", font::BAR_SIZE, later),
        container(text(format::later_tip(LATER_DAYS)).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Top,
    );

    row![later, key_button("⌫", "Back", font::BAR_SIZE, back)]
        .spacing(ACTION_SPACING)
        .align_y(Center)
}

fn segment(pick: Pick) -> style::Segment {
    match pick {
        Pick::Vote(VoteKind::WouldTry) => style::Segment::First,
        Pick::NotThisOne => style::Segment::Middle,
        Pick::Vote(VoteKind::NotForMe | VoteKind::Later) => style::Segment::Last,
        Pick::Played => style::Segment::Alone,
    }
}

/// The three verdicts as one segmented group, then Played on its own unless `played` is false.
pub fn vote_buttons<'a, Message: 'a>(
    button: impl Fn(&'a str, Pick) -> Element<'a, Message>,
    played: bool,
) -> Row<'a, Message> {
    let [would_try, not_this_one, not_for_me, played_button] =
        PICKS.map(|(key, pick)| button(key, pick));
    let seam = || rule::vertical(1).style(style::vote_seam);
    let verdicts =
        container(row![would_try, seam(), not_this_one, seam(), not_for_me].height(Fill))
            .height(VOTE_HEIGHT)
            .padding(1)
            .style(style::vote_group);

    row![verdicts]
        .push(played.then_some(played_button))
        .spacing(GROUP_SPACING)
        .align_y(Center)
}

pub fn vote_width(available: f32, beside: f32) -> Length {
    let verdicts = 3.0 * VOTE_WIDTH + 2.0 * 1.0 + 2.0 * 1.0;
    let needed = verdicts + GROUP_SPACING + VOTE_WIDTH + VOTE_SPACING + beside;

    if available >= needed {
        Length::Fixed(VOTE_WIDTH)
    } else {
        Length::Fill
    }
}

/// The links as a muted column, from the top to the bottom of the header image.
pub fn header_links<'a, Message: Clone + 'a>(
    height: f32,
    on_press: impl Fn(Link) -> Option<Message>,
) -> Element<'a, Message> {
    let link = |key: &'a str, label: &'a str, link: Link| {
        let keycap = keycap(key).style(style::muted_keycap);
        button(
            row![keycap, text(label).size(font::NOTE_SIZE)]
                .spacing(KEY_SPACING)
                .align_y(Center),
        )
        .padding(0)
        .style(style::muted_action)
        .on_press_maybe(on_press(link))
    };

    column![
        link("Q", "Steam ↗", Link::Steam),
        space().height(Fill),
        link("W", "Eneba ↗", Link::Eneba),
        space().height(Fill),
        link("E", "HLTB ↗", Link::Hltb),
    ]
    .height(height)
    .align_x(Left)
    .into()
}

pub fn links<'a, Message: Clone + 'a>(
    on_press: impl Fn(Link) -> Option<Message>,
) -> Element<'a, Message> {
    row![
        style::divider(),
        key_button("Q", "Steam ↗", font::BAR_SIZE, on_press(Link::Steam)),
        key_button("W", "Eneba ↗", font::BAR_SIZE, on_press(Link::Eneba)),
        key_button("E", "HLTB ↗", font::BAR_SIZE, on_press(Link::Hltb)),
    ]
    .spacing(ACTION_SPACING)
    .align_y(Center)
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
        assert_eq!(vote_width(712.0, 0.0), Length::Fixed(VOTE_WIDTH));
        assert_eq!(vote_width(711.0, 0.0), Length::Fill);
        assert_eq!(vote_width(892.0, 180.0), Length::Fixed(VOTE_WIDTH));
        assert_eq!(vote_width(891.0, 180.0), Length::Fill);
    }

    #[test]
    fn the_keys_one_to_four_are_the_votes_and_played() {
        assert_eq!(pick_key("1"), Some(Pick::Vote(VoteKind::WouldTry)));
        assert_eq!(pick_key("2"), Some(Pick::NotThisOne));
        assert_eq!(pick_key("3"), Some(Pick::Vote(VoteKind::NotForMe)));
        assert_eq!(pick_key("4"), Some(Pick::Played));
        assert_eq!(pick_key("5"), None);
    }

    #[test]
    fn q_w_and_e_are_the_links_in_either_case() {
        assert_eq!(link_key("q"), Some(Link::Steam));
        assert_eq!(link_key("W"), Some(Link::Eneba));
        assert_eq!(link_key("e"), Some(Link::Hltb));
        assert_eq!(link_key("E"), Some(Link::Hltb));
        for key in ["4", "5", "6", "7"] {
            assert_eq!(link_key(key), None);
        }
    }

    #[test]
    fn two_saves_a_not_for_me_with_the_genre_ok() {
        assert_eq!(Pick::NotThisOne.ballot(), Some((VoteKind::NotForMe, true)));
        assert_eq!(
            Pick::Vote(VoteKind::NotForMe).ballot(),
            Some((VoteKind::NotForMe, false))
        );
        assert_eq!(Pick::Played.ballot(), None);
    }

    #[test]
    fn a_not_for_me_with_the_genre_ok_is_shown_as_not_this_one() {
        let mut vote = Vote::new(VoteKind::NotForMe, "2026-11-13");
        assert_eq!(Pick::of(&vote), Pick::Vote(VoteKind::NotForMe));

        vote.genre_ok = true;

        assert_eq!(Pick::of(&vote), Pick::NotThisOne);
    }

    #[test]
    fn every_vote_has_a_tip_with_the_hours_of_the_config() {
        assert_eq!(
            tip(Pick::Vote(VoteKind::WouldTry), 5.0, 20.0),
            "Counts as 5 h of play in your taste. Stays in Votes for when it's on sale."
        );
        assert_eq!(
            tip(Pick::NotThisOne, 5.0, 20.0),
            "You like this kind of game, just not this one. It won't come back."
        );
        assert_eq!(
            tip(Pick::Vote(VoteKind::NotForMe), 5.0, 20.0),
            "Not your kind of game, or you played it and didn't like it. It won't come back."
        );
        assert_eq!(
            tip(Pick::Played, 5.0, 20.0),
            "Counts as 20 h of play, like a game in your Library"
        );
        assert_eq!(
            tip(Pick::Vote(VoteKind::WouldTry), 3.0, 20.0),
            "Counts as 3 h of play in your taste. Stays in Votes for when it's on sale."
        );
    }

    #[test]
    fn the_longest_label_fits_in_a_vote_button() {
        let longest = PICKS
            .iter()
            .map(|(_, pick)| format::pick(*pick).chars().count())
            .max();

        assert_eq!(longest, Some("Played, liked it".chars().count()));
    }
}
