use std::sync::Arc;

use gamepalate_app::{
    FamilyChange, FamilyGame, FamilyView, MAX_FAMILIES, RemadeFamilies, RemakeChanges,
};
use iced::{
    Center, Element, Fill, Task,
    widget::{
        button, column, container, mouse_area, opaque, row, scrollable, space, stack, text,
        text_input, tooltip,
    },
};

use super::{Editing, HELD_OUT_TIP, Message, Profile, Remake, section_title};
use crate::{
    font, format,
    screen::{library, sentence},
    style,
};

const NO_FAMILIES: &str = "No families yet. Add games to one from Library.";
pub(super) const FAMILY_HEADERS: usize = 3;
const FAMILY_HEADER_WIDTH: f32 = 80.0;
const FAMILY_HEADER_HEIGHT: f32 = FAMILY_HEADER_WIDTH * 215.0 / 460.0;
const FAMILY_COVER_GAP: f32 = 6.0;
const FAMILY_COVER_MAX: f32 = 180.0;
const CARD_PADDING: f32 = 12.0;
const FAMILY_TAGS: usize = 3;
const EDITOR_WIDTH: f32 = 460.0;
const EDITOR_GAMES_HEIGHT: f32 = 320.0;
const REMOVE: char = '\u{E5CD}';
const UPDATE: char = '\u{E5D5}';
const UPDATE_TIP: &str = "Make the automatic families again from the games you play";
const EDITOR_HEIGHT: f32 = 560.0;
pub(super) const NO_NEW_FAMILIES: &str =
    "No new families: your families already cover the games you play";

pub(super) fn families_text(
    change: &FamilyChange,
    games: &[FamilyGame],
    gone: &[String],
) -> String {
    match change {
        FamilyChange::Rename { to, .. } => format!("Renamed to {}", to.trim()),
        FamilyChange::Delete { family } => format!("Deleted {family}"),
        FamilyChange::RemoveGame { family, game } => {
            let name = games
                .iter()
                .find(|found| found.id == *game)
                .map_or_else(|| format!("#{}", game.0), |found| found.name.clone());
            if gone.contains(family) {
                format!("Removed {name} from {family}, which is now gone")
            } else {
                format!("Removed {name} from {family}")
            }
        }
        FamilyChange::AddGame { .. }
        | FamilyChange::HoldOut { .. }
        | FamilyChange::Release { .. } => String::new(),
    }
}

pub(super) fn remake_dialog(changes: &RemakeChanges) -> Element<'_, Message> {
    let (title, confirm) = remake_title(changes);
    let sections = remake_sections(changes).into_iter().map(|(label, names)| {
        column![text(label).size(font::NOTE_SIZE).style(style::muted)]
            .extend(
                names
                    .iter()
                    .map(|name| text(name).size(font::BAR_SIZE).into()),
            )
            .spacing(2)
            .into()
    });

    let content = column![text(title).size(font::INFO_SIZE).font(font::SEMIBOLD)]
        .extend(sections)
        .push(
            row![
                space().width(Fill),
                button(text("Cancel").size(font::BAR_SIZE))
                    .padding([6, 16])
                    .style(style::secondary)
                    .on_press(Message::CancelRemake),
                button(text(confirm).size(font::BAR_SIZE))
                    .padding([6, 16])
                    .style(style::primary)
                    .on_press(Message::ConfirmRemake),
            ]
            .spacing(10)
            .align_y(Center),
        )
        .spacing(14)
        .padding(20);

    stack![
        opaque(
            mouse_area(
                container(space())
                    .width(Fill)
                    .height(Fill)
                    .style(style::veil)
            )
            .on_press(Message::CancelRemake)
        ),
        container(opaque(
            container(scrollable(content))
                .width(EDITOR_WIDTH)
                .max_height(EDITOR_HEIGHT)
                .style(style::popup)
        ))
        .center(Fill),
    ]
    .into()
}

// Only adding needs no word on what goes away.
fn remake_title(changes: &RemakeChanges) -> (&'static str, &'static str) {
    if changes.made_again.is_empty() && changes.removed.is_empty() {
        ("Add families?", "Add")
    } else {
        ("Update families?", "Update")
    }
}

fn remake_sections(changes: &RemakeChanges) -> Vec<(&'static str, &[String])> {
    [
        ("New, from the games you play", changes.added.as_slice()),
        (
            "Updated from the games you play",
            changes.made_again.as_slice(),
        ),
        ("Removed", changes.removed.as_slice()),
        ("Stay as they are", changes.kept.as_slice()),
    ]
    .into_iter()
    .filter(|(_, names)| !names.is_empty())
    .collect()
}

pub(super) fn already_many() -> String {
    format!("You already have {MAX_FAMILIES} families")
}

pub(super) fn remade_text(outcome: &RemadeFamilies) -> String {
    let families = |count: usize| {
        if count == 1 {
            "1 family".to_string()
        } else {
            format!("{count} families")
        }
    };
    match outcome {
        RemadeFamilies::Remade(remade) => match (remade.added.len(), remade.removed.len()) {
            (0, removed) => format!("Removed {}", families(removed)),
            (added, 0) => format!("Added {}", families(added)),
            (added, _) => format!("Updated {}", families(added)),
        },
        RemadeFamilies::NoNewFamilies => NO_NEW_FAMILIES.to_string(),
        RemadeFamilies::AlreadyMany => already_many(),
    }
}

fn new_games_text(new: usize) -> String {
    format!("{new} new games since your families were made")
}

// Three covers as wide as the card allows, not wider than a header looks good.
pub(super) fn cover_width(side: f32) -> f32 {
    let inside = side - 2.0 * CARD_PADDING - 2.0;
    ((inside - 2.0 * FAMILY_COVER_GAP) / 3.0)
        .min(FAMILY_COVER_MAX)
        .floor()
}

fn more_games(games: usize) -> Option<String> {
    (games > FAMILY_HEADERS).then(|| format!("+{}", games - FAMILY_HEADERS))
}

pub(super) fn games_count(games: usize) -> String {
    if games == 1 {
        "1 game".to_string()
    } else {
        format!("{games} games")
    }
}

fn held_out_text(held_out: usize) -> Option<String> {
    match held_out {
        0 => None,
        1 => Some("1 held-out game".to_string()),
        n => Some(format!("{n} held-out games")),
    }
}

impl Profile {
    pub(super) fn change_families(&self, change: FamilyChange) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        Task::perform(
            async move {
                let result = gamepalate
                    .change_families(&change)
                    .map_err(|error| sentence(&error.to_string()));
                (change, result)
            },
            |(change, result)| Message::FamiliesSaved(change, result),
        )
    }

    pub(super) fn remake_families(&self) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        Task::perform(
            async move {
                gamepalate
                    .remake_families()
                    .map_err(|error| sentence(&error.to_string()))
            },
            Message::Remade,
        )
    }

    pub(super) fn preview_remake(&self) -> Task<Message> {
        let gamepalate = Arc::clone(&self.gamepalate);
        Task::perform(async move { gamepalate.remake_preview() }, Message::Planned)
    }

    pub(super) fn rename(&self) -> Task<Message> {
        match &self.editing {
            Some(editing) if editing.name.trim() != editing.family => {
                self.change_families(FamilyChange::Rename {
                    from: editing.family.clone(),
                    to: editing.name.clone(),
                })
            }
            _ => Task::none(),
        }
    }

    pub(super) fn editor<'a>(&'a self, editing: &'a Editing) -> Element<'a, Message> {
        let games = column(editing.games.iter().map(|game| {
            let remove = FamilyChange::RemoveGame {
                family: editing.family.clone(),
                game: game.id,
            };
            row![
                self.showcase
                    .header(game.id, FAMILY_HEADER_WIDTH, FAMILY_HEADER_HEIGHT)
                    .map(Message::Showcase),
                text(&game.name).size(font::BAR_SIZE).width(Fill),
                button(
                    text(REMOVE)
                        .font(font::ICONS)
                        .size(font::icon(font::NOTE_SIZE))
                )
                .padding([6, 8])
                .style(style::window_button)
                .on_press(Message::ChangeFamilies(remove)),
            ]
            .spacing(10)
            .align_y(Center)
            .into()
        }))
        .spacing(8);

        let content = column![
            text("Edit family")
                .size(font::INFO_SIZE)
                .font(font::SEMIBOLD),
            row![
                text("Name").size(font::BAR_SIZE),
                text_input("Family name", &editing.name)
                    .style(style::field)
                    .on_input(Message::EditName)
                    .on_submit(Message::RenameFamily)
                    .size(font::BAR_SIZE)
                    .padding([5, 8]),
            ]
            .spacing(10)
            .align_y(Center),
            container(scrollable(games).spacing(8)).max_height(EDITOR_GAMES_HEIGHT),
            row![
                button(text("Delete family").size(font::BAR_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::ChangeFamilies(FamilyChange::Delete {
                        family: editing.family.clone(),
                    })),
                space().width(Fill),
                button(text("Done").size(font::BAR_SIZE))
                    .padding([6, 16])
                    .style(style::primary)
                    .on_press(Message::DoneEditing),
            ]
            .align_y(Center),
        ]
        .spacing(14)
        .padding(20);

        stack![
            opaque(
                mouse_area(
                    container(space())
                        .width(Fill)
                        .height(Fill)
                        .style(style::veil)
                )
                .on_press(Message::CloseEditor)
            ),
            container(opaque(
                container(content).width(EDITOR_WIDTH).style(style::popup)
            ))
            .center(Fill),
        ]
        .into()
    }

    pub(super) fn families(&self, cover: f32) -> Element<'_, Message> {
        let view = &self.view;
        let problems = (!view.family_problems.is_empty()).then(|| {
            container(
                column![text("Problems in families.toml").font(font::SEMIBOLD)]
                    .extend(
                        view.family_problems
                            .iter()
                            .map(|problem| text(problem).size(font::NOTE_SIZE).into()),
                    )
                    .spacing(4),
            )
            .padding([8, 12])
            .width(Fill)
            .style(style::bet)
        });

        let body: Element<'_, Message> = if view.families.is_empty() {
            column![
                text(NO_FAMILIES).size(font::BAR_SIZE),
                button(text("Open Library ›").size(font::BAR_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenLibrary(
                        library::Filter::All,
                        library::Sort::TasteWeight,
                    )),
                button(text("Open families.toml").size(font::NOTE_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenFamilies),
            ]
            .spacing(8)
            .into()
        } else {
            column(
                view.families
                    .iter()
                    .map(|family| self.family_card(family, cover)),
            )
            .spacing(10)
            .into()
        };

        let held_out = held_out_text(view.held_out).map(|held_out| {
            tooltip(
                text(held_out).size(font::BAR_SIZE).style(style::muted),
                container(text(HELD_OUT_TIP).size(font::NOTE_SIZE))
                    .padding([6, 10])
                    .style(style::popup),
                tooltip::Position::Bottom,
            )
        });

        column![self.families_title()]
            .push(problems)
            .push(body)
            .push(held_out)
            .spacing(14)
            .into()
    }

    fn families_title(&self) -> Element<'_, Message> {
        let working = self.remake != Remake::Idle;
        let update = button(
            row![
                text(UPDATE)
                    .font(font::ICONS)
                    .size(font::icon(font::BAR_SIZE)),
                text("Update").size(font::BAR_SIZE),
            ]
            .spacing(4)
            .align_y(Center),
        )
        .padding([4, 8])
        .style(style::quiet_action)
        .on_press_maybe((!working).then_some(Message::AskRemake));

        column![
            row![
                section_title("Families"),
                space().width(Fill),
                tooltip(
                    update,
                    container(text(UPDATE_TIP).size(font::NOTE_SIZE))
                        .padding([6, 10])
                        .style(style::popup),
                    tooltip::Position::Bottom,
                ),
            ]
            .align_y(Center),
        ]
        .push(self.new_games.map(|new| {
            text(new_games_text(new))
                .size(font::NOTE_SIZE)
                .style(style::muted)
        }))
        .spacing(4)
        .into()
    }

    fn family_card<'a>(&'a self, family: &'a FamilyView, cover: f32) -> Element<'a, Message> {
        let height = cover * 215.0 / 460.0;
        let more = more_games(family.games.len());
        let shown = family.games.iter().take(FAMILY_HEADERS).count();
        let covers =
            row(family
                .games
                .iter()
                .take(FAMILY_HEADERS)
                .enumerate()
                .map(move |(i, &id)| {
                    let header = self
                        .showcase
                        .header(id, cover, height)
                        .map(Message::Showcase);
                    match more.clone() {
                        Some(more) if i + 1 == shown => stack![
                            header,
                            container(text(more).size(font::BAR_SIZE).font(font::SEMIBOLD))
                                .center(Fill)
                                .style(style::cover_count),
                        ]
                        .width(cover)
                        .height(height)
                        .into(),
                        _ => header,
                    }
                }))
            .spacing(FAMILY_COVER_GAP);

        let details = std::iter::once(games_count(family.games.len()))
            .chain(family.tags.iter().take(FAMILY_TAGS).cloned())
            .collect::<Vec<_>>()
            .join(" · ");

        container(
            column![
                covers,
                text(&family.name).size(font::BAR_SIZE).font(font::SEMIBOLD),
                text(details).size(font::NOTE_SIZE).style(style::muted),
                family.automatic.as_deref().map(|date| {
                    row![
                        crate::screen::automatic_mark(),
                        text(format::automatic_note(date, jiff::Zoned::now().date()))
                            .size(font::NOTE_SIZE)
                            .style(style::muted)
                            .width(Fill),
                    ]
                    .spacing(8)
                    .align_y(Center)
                }),
                row![
                    button(text("Edit").size(font::BAR_SIZE))
                        .padding(0)
                        .style(style::link)
                        .on_press(Message::EditFamily(family.name.clone())),
                    space().width(Fill),
                    button(text("Discover from it ›").size(font::BAR_SIZE))
                        .padding(0)
                        .style(style::link)
                        .on_press(Message::DiscoverFamily(family.name.clone())),
                ]
                .align_y(Center),
            ]
            .spacing(6),
        )
        .padding([10.0, CARD_PADDING])
        .width(Fill)
        .style(style::card)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::AppId;

    use super::super::{SIDE_WIDTH, fixtures::*};
    use super::*;

    #[test]
    fn the_toast_says_what_changed_in_a_family() {
        let games = [FamilyGame {
            id: AppId(1),
            name: "Celeste".to_string(),
        }];
        let remove = |game| FamilyChange::RemoveGame {
            family: "cozy".to_string(),
            game: AppId(game),
        };

        assert_eq!(
            families_text(
                &FamilyChange::Rename {
                    from: "cozy".to_string(),
                    to: " calm ".to_string(),
                },
                &games,
                &[],
            ),
            "Renamed to calm"
        );
        assert_eq!(
            families_text(
                &FamilyChange::Delete {
                    family: "cozy".to_string(),
                },
                &games,
                &["cozy".to_string()],
            ),
            "Deleted cozy"
        );
        assert_eq!(
            families_text(&remove(1), &games, &[]),
            "Removed Celeste from cozy"
        );
        assert_eq!(
            families_text(&remove(1), &games, &["cozy".to_string()]),
            "Removed Celeste from cozy, which is now gone"
        );
        assert_eq!(
            families_text(&remove(2), &games, &[]),
            "Removed #2 from cozy"
        );
    }

    #[test]
    fn the_question_lists_what_comes_goes_and_stays() {
        let changes = changes();

        assert_eq!(remake_title(&changes), ("Update families?", "Update"));
        assert_eq!(
            remake_sections(&changes)
                .into_iter()
                .map(|(label, names)| (label, names.to_vec()))
                .collect::<Vec<_>>(),
            [
                (
                    "New, from the games you play",
                    vec!["Roguelike".to_string()]
                ),
                ("Removed", vec!["Puzzle · Mystery".to_string()]),
                (
                    "Stay as they are",
                    vec!["Mystery".to_string(), "JRPG".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn only_adding_asks_to_add() {
        let adding = RemakeChanges {
            removed: Vec::new(),
            ..changes()
        };

        assert_eq!(remake_title(&adding), ("Add families?", "Add"));
    }

    #[test]
    fn the_outcome_of_a_remake_says_what_happened() {
        assert_eq!(
            remade_text(&RemadeFamilies::NoNewFamilies),
            "No new families: your families already cover the games you play"
        );
        assert_eq!(
            remade_text(&RemadeFamilies::AlreadyMany),
            "You already have 4 families"
        );
    }

    #[test]
    fn the_hint_counts_the_new_games() {
        assert_eq!(
            new_games_text(12),
            "12 new games since your families were made"
        );
    }

    #[test]
    fn a_family_card_counts_the_games_beyond_its_headers() {
        assert_eq!(more_games(9).as_deref(), Some("+6"));
        assert_eq!(more_games(4).as_deref(), Some("+1"));
        assert_eq!(more_games(3), None);
        assert_eq!(games_count(1), "1 game");
        assert_eq!(games_count(9), "9 games");
    }

    #[test]
    fn three_covers_fill_the_card_up_to_a_limit() {
        assert_eq!(cover_width(SIDE_WIDTH), 114.0);
        assert_eq!(cover_width(1100.0), FAMILY_COVER_MAX);
    }

    #[test]
    fn the_held_out_games_show_only_when_there_are_some() {
        assert_eq!(held_out_text(0), None);
        assert_eq!(held_out_text(1).as_deref(), Some("1 held-out game"));
        assert_eq!(held_out_text(12).as_deref(), Some("12 held-out games"));
    }
}
