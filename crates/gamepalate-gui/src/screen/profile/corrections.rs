use std::collections::BTreeMap;

use gamepalate_app::{Ban, CorrectedGameView, CorrectionsView};
use iced::{
    Center, Element, padding,
    widget::{button, column, container, row, text},
};

use super::{CLOSED, Correction, Message, OPENED, Profile, section_title};
use crate::{font, style};

fn tag_multiplier_text(name: &str, multiplier: f64) -> String {
    match multiplier {
        0.5 => format!("{name} counts half"),
        2.0 => format!("{name} counts double"),
        m => format!("{name} counts ×{m}"),
    }
}

fn banned_texts(corrections: &CorrectionsView) -> Vec<String> {
    let mut groups: BTreeMap<Option<u32>, Vec<&str>> = BTreeMap::new();
    for (name, ban) in &corrections.banned_tags {
        let limit = match ban {
            Ban::Everywhere => None,
            Ban::TopTags(n) => Some(*n),
        };
        groups.entry(limit).or_default().push(name);
    }

    groups
        .into_iter()
        .map(|(limit, names)| match limit {
            None => format!("{} banned", names.join(", ")),
            Some(n) => format!("{} banned in the top {n} tags", names.join(", ")),
        })
        .collect()
}

fn games_multiplier_text(games: &[CorrectedGameView]) -> Option<String> {
    let less = games.iter().filter(|game| game.multiplier < 1.0).count();
    let more = games.len() - less;
    let part = |count: usize, how: &str| match count {
        0 => None,
        1 => Some(format!("1 game counts {how}")),
        n => Some(format!("{n} games count {how}")),
    };
    let parts: Vec<String> = [part(less, "less"), part(more, "more")]
        .into_iter()
        .flatten()
        .collect();

    (!parts.is_empty()).then(|| parts.join(", "))
}

fn corrected_game_text(game: &CorrectedGameView) -> String {
    let effect = if game.in_effect { "" } else { " · no effect" };
    format!("{} ×{}{effect}", game.name, game.multiplier)
}

fn hidden_text(hidden: usize) -> String {
    if hidden == 1 {
        "1 hidden game".to_string()
    } else {
        format!("{hidden} hidden games")
    }
}

impl Profile {
    pub(super) fn corrections(&self) -> Element<'_, Message> {
        let corrections = &self.view.corrections;
        let line = |line: String| text(line).size(font::BAR_SIZE).into();
        let mut lines: Vec<Element<'_, Message>> = corrections
            .tag_multipliers
            .iter()
            .map(|(name, multiplier)| line(tag_multiplier_text(name, *multiplier)))
            .chain(banned_texts(corrections).into_iter().map(line))
            .collect();

        if let Some(summary) = games_multiplier_text(&corrections.game_multipliers) {
            lines.push(
                self.expandable(
                    summary,
                    Correction::Games,
                    corrections
                        .game_multipliers
                        .iter()
                        .map(|game| {
                            (
                                corrected_game_text(game),
                                Some(Message::OpenInLibrary(game.id)),
                            )
                        })
                        .collect(),
                ),
            );
        }
        if !corrections.hidden_games.is_empty() {
            lines.push(
                self.expandable(
                    hidden_text(corrections.hidden_games.len()),
                    Correction::Hidden,
                    corrections
                        .hidden_games
                        .iter()
                        .map(|name| (name.clone(), None))
                        .collect(),
                ),
            );
        }

        column![
            column![
                section_title("Corrections"),
                text("Correct games in Library and tags with the CLI")
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .spacing(4),
            column(lines).spacing(8),
        ]
        .spacing(14)
        .into()
    }

    fn expandable(
        &self,
        summary: String,
        correction: Correction,
        items: Vec<(String, Option<Message>)>,
    ) -> Element<'_, Message> {
        let open = self.open_correction == Some(correction);
        let header = button(
            row![
                text(summary).size(font::BAR_SIZE),
                text(if open { OPENED } else { CLOSED })
                    .font(font::ICONS)
                    .size(font::icon(10.0)),
            ]
            .spacing(8)
            .align_y(Center),
        )
        .padding(0)
        .style(style::list_row(false))
        .on_press(Message::ToggleCorrection(correction));

        column![header]
            .push(open.then(|| {
                container(
                    column(items.into_iter().map(|(item, message)| {
                        let item = text(item).size(font::NOTE_SIZE);
                        match message {
                            Some(message) => button(item)
                                .padding(0)
                                .style(style::link)
                                .on_press(message)
                                .into(),
                            None => item.style(style::muted).into(),
                        }
                    }))
                    .spacing(2),
                )
                .padding(padding::left(12))
            }))
            .spacing(6)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::AppId;

    use super::*;

    #[test]
    fn a_tag_multiplier_reads_as_a_sentence() {
        assert_eq!(
            tag_multiplier_text("Souls-like", 0.5),
            "Souls-like counts half"
        );
        assert_eq!(
            tag_multiplier_text("Roguelike", 2.0),
            "Roguelike counts double"
        );
        assert_eq!(
            tag_multiplier_text("Open World", 0.7),
            "Open World counts ×0.7"
        );
    }

    #[test]
    fn the_banned_tags_come_together_by_kind_of_ban() {
        let corrections = CorrectionsView {
            banned_tags: vec![
                ("Anime".to_string(), Ban::TopTags(8)),
                ("Horror".to_string(), Ban::Everywhere),
                ("JRPG".to_string(), Ban::TopTags(8)),
            ],
            ..CorrectionsView::default()
        };

        assert_eq!(
            banned_texts(&corrections),
            ["Horror banned", "Anime, JRPG banned in the top 8 tags"]
        );
    }

    fn corrected(name: &str, multiplier: f64, in_effect: bool) -> CorrectedGameView {
        CorrectedGameView {
            id: AppId(431730),
            name: name.to_string(),
            multiplier,
            in_effect,
        }
    }

    #[test]
    fn the_game_multipliers_count_the_games_that_count_less_or_more() {
        let games = [
            corrected("Aseprite", 0.5, true),
            corrected("Call of Duty", 0.3, true),
            corrected("Outer Wilds", 2.0, false),
        ];

        assert_eq!(
            games_multiplier_text(&games).as_deref(),
            Some("2 games count less, 1 game counts more")
        );
        assert_eq!(games_multiplier_text(&[]), None);
        assert_eq!(corrected_game_text(&games[0]), "Aseprite ×0.5");
        assert_eq!(corrected_game_text(&games[2]), "Outer Wilds ×2 · no effect");
    }

    #[test]
    fn the_hidden_games_are_counted() {
        assert_eq!(hidden_text(1), "1 hidden game");
        assert_eq!(hidden_text(94), "94 hidden games");
    }
}
