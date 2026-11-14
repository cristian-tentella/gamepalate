use gamepalate_app::{AppId, CheckResult, DeckCheck, GameDetails, ProtonDb, SteamDeck};
use iced::{
    Element, Theme, mouse,
    widget::{column, container, mouse_area, row, text, text::Style as TextStyle, tooltip},
};

use super::{Message, chip};
use crate::{font, format, style};

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
const TIP_GAP: f32 = 6.0;

type Tone = fn(&Theme) -> TextStyle;

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

/// The Steam Deck chip of Features: green when the game runs well enough, red when it does not,
/// plain like the other chips when neither Valve nor ProtonDB says anything useful. The tooltip
/// says what Valve and ProtonDB say, then Valve's reasons; a click opens the game on ProtonDB.
pub(super) fn chip_of(id: AppId, details: &GameDetails) -> Element<'_, Message> {
    let deck = details.store.steam_deck;
    let runs = format::deck_rating(deck, details.protondb).map(format::DeckRating::runs);
    let label = mouse_area(chip("Steam Deck", style::deck_chip(runs)))
        .on_press(Message::OpenLink(format::protondb_url(id)))
        .interaction(mouse::Interaction::Pointer);

    tooltip(
        label,
        tip(deck, &details.deck, details.protondb),
        // Features closes the column, so below there is often no room and the tooltip,
        // pushed back into the window, would cover the chip.
        tooltip::Position::Top,
    )
    .gap(TIP_GAP)
    .into()
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
