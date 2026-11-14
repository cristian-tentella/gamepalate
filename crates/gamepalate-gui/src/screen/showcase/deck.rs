use gamepalate_app::{AppId, CheckResult, DeckCheck, GameDetails, ProtonDb, SteamDeck};
use iced::{
    Element, Theme, mouse,
    widget::{column, container, mouse_area, row, text, text::Style as TextStyle, tooltip},
};

use super::Message;
use crate::{
    font,
    format::{self, DeckRating},
    style,
};

const HANDHELD: char = '\u{F4C6}';
const CHECK_CIRCLE: char = '\u{E86C}';
const WARNING: char = '\u{E002}';
const BLOCK: char = '\u{E14B}';
const INFO: char = '\u{E88E}';
const ICON_SIZE: f32 = 11.0;
const ICON_GAP: f32 = 6.0;
// The right column of a game is never narrower than this.
const TIP_WIDTH: f32 = 260.0;
const TIP_SPACING: f32 = 4.0;
const TIP_GROUP_SPACING: f32 = 8.0;

type Tone = fn(&Theme) -> TextStyle;

/// The tone of the icon and of the word. Good keeps the green on the icon alone, so it stays
/// apart from Great.
fn rating_look(rating: DeckRating) -> (Tone, Tone) {
    match rating {
        DeckRating::Great => (style::verified, style::verified),
        DeckRating::Good => (style::verified, style::muted),
        DeckRating::Rough => (style::playable, style::playable),
        DeckRating::Broken => (style::muted, style::muted),
    }
}

fn check_look(result: CheckResult) -> (char, Tone) {
    match result {
        CheckResult::Passed => (CHECK_CIRCLE, style::verified),
        CheckResult::Warning => (WARNING, style::playable),
        CheckResult::Failed => (BLOCK, style::muted),
        CheckResult::Note => (INFO, style::muted),
    }
}

fn icon<'a>(symbol: char, tone: Tone) -> Element<'a, Message> {
    text(symbol)
        .font(font::ICONS)
        .size(font::icon(ICON_SIZE))
        .style(tone)
        .into()
}

/// How well the game runs on Steam Deck, in one word. The tooltip says what Valve and ProtonDB
/// say, then Valve's reasons; a click opens the game on ProtonDB. `None` when neither says
/// anything useful.
pub(super) fn badge(id: AppId, details: &GameDetails) -> Option<Element<'_, Message>> {
    let deck = details.store.steam_deck;
    let rating = format::deck_rating(deck, details.protondb)?;
    let (icon_tone, word_tone) = rating_look(rating);

    let line = mouse_area(
        row![
            icon(HANDHELD, icon_tone),
            text(format::deck_rating_label(rating))
                .size(font::NOTE_SIZE)
                .style(word_tone),
        ]
        .spacing(ICON_GAP)
        .align_y(iced::Center),
    )
    .on_press(Message::OpenLink(format::protondb_url(id)))
    .interaction(mouse::Interaction::Pointer);

    Some(
        tooltip(
            line,
            tip(deck, &details.deck, details.protondb),
            tooltip::Position::Bottom,
        )
        .into(),
    )
}

fn tip(
    deck: Option<SteamDeck>,
    checks: &[DeckCheck],
    protondb: Option<ProtonDb>,
) -> Element<'_, Message> {
    let sources = column![
        text(format::steam_deck_line(deck))
            .size(font::NOTE_SIZE)
            .font(font::SEMIBOLD),
        text(format::protondb_line(protondb))
            .size(font::NOTE_SIZE)
            .style(style::muted),
    ]
    .spacing(TIP_SPACING);
    let reasons = format::deck_order(checks).into_iter().map(|check| {
        let (symbol, tone) = check_look(check.result);

        row![icon(symbol, tone), text(check.text).size(font::NOTE_SIZE)]
            .spacing(ICON_GAP)
            .into()
    });
    let reasons = (!checks.is_empty()).then(|| column(reasons).spacing(TIP_SPACING));

    container(column![sources].push(reasons).spacing(TIP_GROUP_SPACING))
        .padding([6, 10])
        .max_width(TIP_WIDTH)
        .style(style::popup)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_check_has_its_symbol() {
        assert_eq!(check_look(CheckResult::Passed).0, CHECK_CIRCLE);
        assert_eq!(check_look(CheckResult::Warning).0, WARNING);
        assert_eq!(check_look(CheckResult::Failed).0, BLOCK);
        assert_eq!(check_look(CheckResult::Note).0, INFO);
    }
}
