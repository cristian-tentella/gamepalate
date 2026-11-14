use gamepalate_app::{AppId, CheckResult, DeckCheck, GameDetails, ProtonDb, SteamDeck};
use iced::{
    Element, Theme, mouse,
    widget::{column, container, mouse_area, row, text, text::Style as TextStyle, tooltip},
};

use super::{LINE_SPACING, Message, TAG_SPACING};
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

type Tone = fn(&Theme) -> TextStyle;

fn deck_look(deck: SteamDeck) -> (char, Tone) {
    match deck {
        SteamDeck::Verified => (CHECK_CIRCLE, style::verified),
        SteamDeck::Playable => (INFO, style::playable),
        SteamDeck::Unsupported => (BLOCK, style::muted),
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

/// What Valve and ProtonDB say about the game on Steam Deck, always in sight. The tooltip has
/// the reasons and a click opens the game on ProtonDB. `None` when neither says anything.
pub(super) fn badge(id: AppId, details: &GameDetails) -> Option<Element<'_, Message>> {
    let deck = details.store.steam_deck;
    let parts = format::deck_badge(deck, details.protondb)?;

    let mut pieces: Vec<Element<'_, Message>> = Vec::new();
    if let (Some(deck), Some(label)) = (deck, parts.deck) {
        let (symbol, tone) = deck_look(deck);
        pieces.push(
            row![
                icon(symbol, tone),
                text(label).size(font::NOTE_SIZE).style(tone)
            ]
            .spacing(TAG_SPACING)
            .into(),
        );
    }
    if let Some(label) = parts.protondb {
        let label = if deck.is_some() {
            format!("· {label}")
        } else {
            label
        };
        pieces.push(text(label).size(font::NOTE_SIZE).style(style::muted).into());
    }

    let line = mouse_area(
        row(pieces)
            .spacing(TAG_SPACING)
            .align_y(iced::Center)
            .wrap()
            .vertical_spacing(LINE_SPACING),
    )
    .on_press(Message::OpenLink(format::protondb_url(id)))
    .interaction(mouse::Interaction::Pointer);

    Some(
        tooltip(
            line,
            tip(&details.deck, details.protondb),
            tooltip::Position::Bottom,
        )
        .into(),
    )
}

fn tip(checks: &[DeckCheck], protondb: Option<ProtonDb>) -> Element<'_, Message> {
    let reasons = format::deck_order(checks).into_iter().map(|check| {
        let (symbol, tone) = check_look(check.result);

        row![icon(symbol, tone), text(check.text).size(font::NOTE_SIZE)]
            .spacing(ICON_GAP)
            .into()
    });
    let protondb = text(format::protondb_line(protondb))
        .size(font::NOTE_SIZE)
        .style(style::muted);

    container(column(reasons).push(protondb).spacing(TIP_SPACING))
        .padding([6, 10])
        .max_width(TIP_WIDTH)
        .style(style::popup)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_category_has_its_symbol() {
        assert_eq!(deck_look(SteamDeck::Verified).0, CHECK_CIRCLE);
        assert_eq!(deck_look(SteamDeck::Playable).0, INFO);
        assert_eq!(deck_look(SteamDeck::Unsupported).0, BLOCK);
    }

    #[test]
    fn each_check_has_its_symbol() {
        assert_eq!(check_look(CheckResult::Passed).0, CHECK_CIRCLE);
        assert_eq!(check_look(CheckResult::Warning).0, WARNING);
        assert_eq!(check_look(CheckResult::Failed).0, BLOCK);
        assert_eq!(check_look(CheckResult::Note).0, INFO);
    }
}
