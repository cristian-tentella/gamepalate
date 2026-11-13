use gamepalate_app::{
    AppId, FamilyChange, GameCorrection, GameTagsView, HoursSource, LibraryGameView, LibraryView,
    names::folded,
};
use iced::{
    Center, Element, Fill, Padding, Point, Size, Task,
    widget::{button, checkbox, column, container, opaque, row, space, text, text_input, tooltip},
};

use super::{
    Library, Message, NEW_FAMILY,
    grid::{TILE_WIDTH, tile_origin},
    hours, hours_text, multiplier,
};
use crate::{
    font, format,
    screen::profile::HELD_OUT_TIP,
    style::{self, Tone},
};

const POPUP_WIDTH: f32 = 320.0;
const POPUP_HEIGHT: f32 = 360.0;
const POPUP_GAP: f32 = 8.0;
const POPUP_MARGIN: f32 = 8.0;
const PICKS: [Choice; 3] = [Choice::Fully, Choice::Half, Choice::Not];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Choice {
    Fully,
    Half,
    Not,
    Custom(f64),
}

fn popup_origin(tile: Point, area: Size) -> Point {
    let right = tile.x + TILE_WIDTH + POPUP_GAP;
    let x = if right + POPUP_WIDTH + POPUP_MARGIN <= area.width {
        right
    } else {
        tile.x - POPUP_GAP - POPUP_WIDTH
    };
    let y = tile.y.min(area.height - POPUP_HEIGHT - POPUP_MARGIN);

    Point::new(x.max(POPUP_MARGIN), y.max(POPUP_MARGIN))
}

fn selected_choice(multiplier: Option<f64>, dampen: f64) -> Choice {
    match multiplier {
        None => Choice::Fully,
        Some(1.0) => Choice::Fully,
        Some(factor) if factor == dampen => Choice::Half,
        Some(0.0) => Choice::Not,
        Some(factor) => Choice::Custom(factor),
    }
}

fn choices(selected: Choice) -> Vec<Choice> {
    let mut choices = vec![Choice::Fully, Choice::Half, Choice::Not];
    if let Choice::Custom(_) = selected {
        choices.push(selected);
    }
    choices
}

fn choice_multiplier(choice: Choice, dampen: f64) -> Option<f64> {
    match choice {
        Choice::Fully => None,
        Choice::Half => Some(dampen),
        Choice::Not => Some(0.0),
        Choice::Custom(factor) => Some(factor),
    }
}

fn choice_factor(choice: Choice, dampen: f64) -> String {
    multiplier(choice_multiplier(choice, dampen).unwrap_or(1.0))
}

fn choice_label(choice: Choice) -> &'static str {
    match choice {
        Choice::Fully => "Fully",
        Choice::Half => "Half",
        Choice::Not => "Not",
        Choice::Custom(_) => "Custom",
    }
}

fn choice_tone(choice: Choice) -> Tone {
    match choice {
        Choice::Fully => Tone::Success,
        Choice::Half => Tone::Warning,
        Choice::Not => Tone::Danger,
        Choice::Custom(_) => Tone::Primary,
    }
}

fn held_out(game: &LibraryGameView) -> Element<'_, Message> {
    let id = game.id;
    tooltip(
        checkbox(game.held_out)
            .label("Held out")
            .text_size(font::BAR_SIZE)
            .on_toggle(move |held_out| {
                Message::ChangeFamilies(if held_out {
                    FamilyChange::HoldOut { game: id }
                } else {
                    FamilyChange::Release { game: id }
                })
            }),
        container(text(HELD_OUT_TIP).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Bottom,
    )
    .into()
}

fn chip_change(family: &str, game: AppId, inside: bool) -> FamilyChange {
    let family = family.to_string();
    if inside {
        FamilyChange::RemoveGame { family, game }
    } else {
        FamilyChange::AddGame { family, game }
    }
}

pub(super) fn changed_game(change: &FamilyChange) -> Option<AppId> {
    match change {
        FamilyChange::AddGame { game, .. }
        | FamilyChange::RemoveGame { game, .. }
        | FamilyChange::HoldOut { game }
        | FamilyChange::Release { game } => Some(*game),
        FamilyChange::Rename { .. } | FamilyChange::Delete { .. } => None,
    }
}

pub(super) fn families_text(
    change: &FamilyChange,
    game: &LibraryGameView,
    gone: &[String],
) -> String {
    match change {
        FamilyChange::AddGame { family, .. } if game.held_out => {
            format!("Added to {}, no longer held out", family.trim())
        }
        FamilyChange::AddGame { family, .. } => format!("Added to {}", family.trim()),
        FamilyChange::RemoveGame { family, .. } if gone.contains(family) => {
            format!("Removed from {family}, which is now gone")
        }
        FamilyChange::RemoveGame { family, .. } => format!("Removed from {family}"),
        FamilyChange::HoldOut { .. } if game.families.is_empty() => "Held out".to_string(),
        FamilyChange::HoldOut { .. } => {
            format!("Held out, removed from {}", game.families.join(", "))
        }
        FamilyChange::Release { .. } => "No longer held out".to_string(),
        FamilyChange::Rename { .. } | FamilyChange::Delete { .. } => String::new(),
    }
}

fn tags_button<'a>(view: Option<&GameTagsView>) -> Option<Element<'a, Message>> {
    view.map(|_| {
        button(text("Tags").size(font::NOTE_SIZE))
            .padding(0)
            .style(style::link)
            .on_press(Message::ToggleTags)
            .into()
    })
}

fn removal(game: &LibraryGameView) -> Option<Message> {
    game.elsewhere_only
        .then(|| Message::PlayElsewhere(game.id, game.name.clone(), false))
}

fn counts_as(hours: f64) -> String {
    format!("counts as {}", hours_text(hours))
}

fn source_line(game: &LibraryGameView, view: &LibraryView) -> String {
    let signals: Vec<&str> = [
        (game.owned_hours.is_some(), "Owned"),
        (game.borrowed, "Not owned"),
        (game.wishlist, "Wishlist"),
        (game.followed, "Followed"),
        (game.elsewhere, "Played elsewhere"),
    ]
    .into_iter()
    .filter(|&(is_in, _)| is_in)
    .map(|(_, name)| name)
    .collect();
    let mut parts = vec![match signals.as_slice() {
        [] => "Played".to_string(),
        [one] => one.to_string(),
        [first, rest @ ..] => format!("{first} and {}", rest.join(" and ").to_lowercase()),
    }];

    parts.push(match (game.source, game.owned_hours) {
        (HoursSource::Played, Some(owned)) => hours(owned),
        (HoursSource::Played, None) => hours(game.hours),
        (HoursSource::Wishlist, _) => counts_as(view.wishlist_hours),
        (HoursSource::Followed, _) => counts_as(view.followed_hours),
        (HoursSource::WouldTry, _) => counts_as(game.hours),
        (HoursSource::Elsewhere, _) => counts_as(view.elsewhere_hours),
    });
    if game.recent {
        parts.push("played in the last two weeks".to_string());
    }
    parts.join(" · ")
}

fn ordinal(number: usize) -> String {
    let suffix = match (number % 10, number % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

fn weight_line(game: &LibraryGameView, view: &LibraryView) -> String {
    if game.counts {
        let counting: Vec<&LibraryGameView> =
            view.games.iter().filter(|other| other.counts).collect();
        let name = folded(&game.name);
        let heavier = counting
            .iter()
            .filter(|other| {
                other.share > game.share
                    || (other.share == game.share && folded(&other.name) < name)
            })
            .count();
        format!(
            "{} of {} · {} the average game",
            ordinal(heavier + 1),
            counting.len(),
            format::affinity(game.share * counting.len() as f64)
        )
    } else if game.hours < view.min_hours {
        format!(
            "{}, under the {} that count",
            hours(game.hours),
            hours_text(view.min_hours)
        )
    } else if game.correction.multiplier == Some(0.0) {
        "Set to ×0, so it doesn't count".to_string()
    } else {
        "No tags, so it doesn't count".to_string()
    }
}

fn no_effect_text(game_hours: f64, min_hours: f64) -> String {
    format!(
        "It has {}, under {}, so a correction changes nothing",
        hours(game_hours),
        hours_text(min_hours)
    )
}

pub(super) fn corrected_text(
    name: &str,
    before: GameCorrection,
    after: GameCorrection,
    dampen: f64,
) -> String {
    if before.hidden != after.hidden {
        return if after.hidden {
            format!("{name} is hidden from Discover")
        } else {
            format!("{name} shows in Discover again")
        };
    }
    match selected_choice(after.multiplier, dampen) {
        Choice::Fully => format!("{name} counts fully"),
        Choice::Half => format!("{name} counts half"),
        Choice::Not => format!("{name} doesn't count"),
        Choice::Custom(factor) => format!("{name} counts {}", multiplier(factor)),
    }
}

impl Library {
    pub(super) fn pick(&mut self, index: usize) -> Task<Message> {
        let view = &self.view;
        let Some(game) = self.open.and_then(|id| self.game(id)) else {
            return Task::none();
        };
        let choice = PICKS[index.min(2)];
        if game.hours >= view.min_hours
            && choice != selected_choice(game.correction.multiplier, view.dampen)
        {
            let id = game.id;
            let correction = GameCorrection {
                multiplier: choice_multiplier(choice, view.dampen),
                ..game.correction
            };
            self.open = None;
            self.new_family = None;
            return self.update(Message::Correct(id, correction));
        }

        Task::none()
    }

    pub(super) fn placed_popup<'a>(
        &'a self,
        game: &'a LibraryGameView,
        size: Size,
    ) -> Element<'a, Message> {
        let rows = self.rows(size.width);
        let Some(tile) = self
            .shown
            .iter()
            .position(|&index| self.view.games[index].id == game.id)
            .and_then(|position| tile_origin(&rows, position, size.width, self.offset))
        else {
            return space().into();
        };
        let origin = popup_origin(tile, size);

        container(opaque(
            container(self.popup(game))
                .width(POPUP_WIDTH)
                .style(style::popup),
        ))
        .padding(Padding {
            top: origin.y,
            left: origin.x,
            ..Padding::ZERO
        })
        .into()
    }

    fn popup<'a>(&'a self, game: &'a LibraryGameView) -> Element<'a, Message> {
        let view = &self.view;
        let correctable = game.hours >= view.min_hours;
        let selected = selected_choice(game.correction.multiplier, view.dampen);
        let choices = row(choices(selected).into_iter().map(|choice| {
            let pick = PICKS.iter().position(|&pick| pick == choice);
            button(
                column![
                    text(choice_factor(choice, view.dampen))
                        .size(font::INFO_SIZE)
                        .font(font::SEMIBOLD),
                    text(choice_label(choice)).size(font::NOTE_SIZE),
                ]
                .spacing(2)
                .align_x(Center)
                .width(Fill),
            )
            .width(Fill)
            .padding([6, 4])
            .style(style::tinted(choice_tone(choice), choice == selected))
            .on_press_maybe(
                pick.filter(|_| correctable && choice != selected)
                    .map(Message::Pick),
            )
            .into()
        }))
        .spacing(6);
        let hide =
            (game.owned_hours.is_none() && !game.borrowed && !game.recent && !game.elsewhere).then(
                || {
                    let id = game.id;
                    let correction = game.correction;
                    checkbox(correction.hidden)
                        .label("Hide from Discover")
                        .text_size(font::BAR_SIZE)
                        .on_toggle(move |hidden| {
                            Message::Correct(
                                id,
                                GameCorrection {
                                    hidden,
                                    ..correction
                                },
                            )
                        })
                },
            );

        column![
            column![
                text(&game.name).size(font::INFO_SIZE).font(font::SEMIBOLD),
                text(source_line(game, view))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                text(weight_line(game, view)).size(font::BAR_SIZE),
            ]
            .push(
                (!game.tags.is_empty() || self.game_tags().is_some()).then(|| {
                    row![(!game.tags.is_empty()).then(|| {
                        text!("Brings {}", game.tags.join(", "))
                            .size(font::NOTE_SIZE)
                            .style(style::muted)
                            .width(Fill)
                    })]
                    .push(tags_button(self.game_tags()))
                    .spacing(12)
                    .align_y(Center)
                }),
            )
            .spacing(2),
            text("Counts for your taste")
                .size(font::NOTE_SIZE)
                .style(style::muted),
            choices,
        ]
        .push((!correctable).then(|| {
            text(no_effect_text(game.hours, view.min_hours))
                .size(font::NOTE_SIZE)
                .style(style::muted)
        }))
        .push(style::separator())
        .push(text("Families").size(font::NOTE_SIZE).style(style::muted))
        .push(self.family_chips(game))
        .push(row![held_out(game)].push(hide).spacing(16))
        .push(
            button(text("Open families.toml ›").size(font::NOTE_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::OpenFamilies),
        )
        .push(removal(game).map(|_| style::separator()))
        .push(removal(game).map(|message| {
            button(text("Remove from Library").size(font::NOTE_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(message)
        }))
        .spacing(10)
        .padding(16)
        .into()
    }

    fn family_chips<'a>(&'a self, game: &'a LibraryGameView) -> Element<'a, Message> {
        let chips = self.view.families.iter().map(|family| {
            let inside = game.families.contains(family);
            let automatic = self.view.automatic_families.contains(family);
            button(
                row![text(family).size(font::NOTE_SIZE)]
                    .push(automatic.then(|| {
                        text("Automatic")
                            .size(font::NOTE_SIZE * 0.85)
                            .style(style::muted)
                    }))
                    .spacing(6)
                    .align_y(Center),
            )
            .padding([4, 12])
            .style(style::choice(inside))
            .on_press(Message::ChangeFamilies(chip_change(
                family, game.id, inside,
            )))
            .into()
        });
        let new: Element<'a, Message> = match &self.new_family {
            Some(name) => text_input("Family name", name)
                .id(NEW_FAMILY)
                .style(style::field)
                .on_input(Message::NewFamilyName)
                .on_submit(Message::CreateFamily)
                .size(font::NOTE_SIZE)
                .padding([4, 10])
                .width(160)
                .into(),
            None => button(text("+ New family").size(font::NOTE_SIZE))
                .padding([4, 12])
                .style(style::chip)
                .on_press(Message::NewFamily)
                .into(),
        };

        row(chips)
            .push(new)
            .spacing(6)
            .wrap()
            .vertical_spacing(6)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::super::grid::{GRID_PADDING, tile_label};
    use super::*;

    #[test]
    fn the_selected_choice_follows_the_multiplier() {
        assert_eq!(selected_choice(None, 0.5), Choice::Fully);
        assert_eq!(selected_choice(Some(1.0), 0.5), Choice::Fully);
        assert_eq!(selected_choice(Some(0.5), 0.5), Choice::Half);
        assert_eq!(selected_choice(Some(0.0), 0.5), Choice::Not);
        assert_eq!(selected_choice(Some(0.25), 0.5), Choice::Custom(0.25));
        assert_eq!(selected_choice(Some(0.5), 0.6), Choice::Custom(0.5));
    }

    #[test]
    fn a_factor_written_by_hand_is_a_fourth_choice() {
        assert_eq!(
            choices(Choice::Half),
            [Choice::Fully, Choice::Half, Choice::Not]
        );
        assert_eq!(
            choices(Choice::Custom(0.25)),
            [
                Choice::Fully,
                Choice::Half,
                Choice::Not,
                Choice::Custom(0.25)
            ]
        );
        assert_eq!(choice_factor(Choice::Custom(0.25), 0.5), "×0.25");
        assert_eq!(choice_label(Choice::Custom(0.25)), "Custom");
    }

    #[test]
    fn each_choice_shows_its_factor_and_its_colour() {
        assert_eq!(choice_factor(Choice::Fully, 0.5), "×1");
        assert_eq!(choice_factor(Choice::Half, 0.5), "×0.5");
        assert_eq!(choice_factor(Choice::Half, 0.6), "×0.6");
        assert_eq!(choice_factor(Choice::Not, 0.5), "×0");
        assert_eq!(choice_tone(Choice::Fully), Tone::Success);
        assert_eq!(choice_tone(Choice::Half), Tone::Warning);
        assert_eq!(choice_tone(Choice::Not), Tone::Danger);
        assert_eq!(choice_tone(Choice::Custom(0.25)), Tone::Primary);
    }

    #[test]
    fn the_popup_opens_beside_the_tile_and_stays_in_the_grid() {
        let area = Size::new(1280.0, 700.0);
        let first = Point::new(GRID_PADDING, GRID_PADDING);
        let last = Point::new(area.width - GRID_PADDING - TILE_WIDTH, GRID_PADDING);

        assert_eq!(
            popup_origin(first, area),
            Point::new(first.x + TILE_WIDTH + POPUP_GAP, first.y)
        );
        assert_eq!(popup_origin(last, area).x, last.x - POPUP_GAP - POPUP_WIDTH);
        assert_eq!(
            popup_origin(Point::new(first.x, 600.0), area).y,
            area.height - POPUP_HEIGHT - POPUP_MARGIN
        );
        assert_eq!(
            popup_origin(Point::new(first.x, -100.0), area).y,
            POPUP_MARGIN
        );
    }

    #[test]
    fn fully_removes_the_multiplier() {
        assert_eq!(choice_multiplier(Choice::Fully, 0.5), None);
        assert_eq!(choice_multiplier(Choice::Half, 0.6), Some(0.6));
        assert_eq!(choice_multiplier(Choice::Not, 0.5), Some(0.0));
        assert_eq!(choice_multiplier(Choice::Custom(0.25), 0.5), Some(0.25));
    }

    #[test]
    fn the_source_line_says_every_signal_and_the_winning_hours() {
        let games = games();
        let view = view_of(Vec::new());
        let line = |game: &LibraryGameView| source_line(game, &view);

        assert_eq!(line(&games[1]), "Owned · 120 h");
        assert_eq!(line(&games[3]), "Wishlist · counts as 5 hours");
        assert_eq!(line(&games[4]), "Followed · counts as 3 hours");
        let both = LibraryGameView {
            followed: true,
            recent: true,
            ..games[0].clone()
        };
        assert_eq!(
            line(&both),
            "Owned and followed · 40 h · played in the last two weeks"
        );
        let borrowed = LibraryGameView {
            recent: true,
            hours: 6.0,
            ..game(7, "Drova")
        };
        assert_eq!(
            line(&borrowed),
            "Played · 6.0 h · played in the last two weeks"
        );
        let shared = LibraryGameView {
            borrowed: true,
            hours: 147.8,
            ..game(8, "Baldur's Gate 3")
        };
        assert_eq!(line(&shared), "Not owned · 148 h");
    }

    #[test]
    fn the_weight_line_says_how_much_or_why_not() {
        let games = games();
        let view = view_of(games.clone());
        let line = |game: &LibraryGameView| weight_line(game, &view);

        assert_eq!(line(&games[1]), "1st of 4 · 2× the average game");
        assert_eq!(line(&games[0]), "2nd of 4 · 0.8× the average game");
        assert_eq!(line(&games[4]), "4th of 4 · 0.2× the average game");
        assert_eq!(line(&games[2]), "1.0 h, under the 2 hours that count");
        let zero = LibraryGameView {
            counts: false,
            correction: GameCorrection {
                multiplier: Some(0.0),
                hidden: false,
            },
            ..games[1].clone()
        };
        assert_eq!(line(&zero), "Set to ×0, so it doesn't count");
        assert_eq!(
            no_effect_text(0.5, 2.0),
            "It has 0.5 h, under 2 hours, so a correction changes nothing"
        );
    }

    #[test]
    fn the_place_takes_its_english_ordinal() {
        let ordinals: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 111, 374]
            .into_iter()
            .map(ordinal)
            .collect();

        assert_eq!(
            ordinals,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st",
                "111th", "374th"
            ]
        );
    }

    #[test]
    fn the_toast_says_what_changed() {
        let none = GameCorrection::default();
        let with = |multiplier, hidden| GameCorrection { multiplier, hidden };

        assert_eq!(
            corrected_text("Black Ops II", none, with(Some(0.5), false), 0.5),
            "Black Ops II counts half"
        );
        assert_eq!(
            corrected_text("Hades", with(Some(0.5), false), none, 0.5),
            "Hades counts fully"
        );
        assert_eq!(
            corrected_text("Hades", none, with(Some(0.0), false), 0.5),
            "Hades doesn't count"
        );
        assert_eq!(
            corrected_text(
                "Hades",
                with(Some(0.0), false),
                with(Some(0.25), false),
                0.5
            ),
            "Hades counts ×0.25"
        );
        assert_eq!(
            corrected_text("Pentiment", none, with(None, true), 0.5),
            "Pentiment is hidden from Discover"
        );
        assert_eq!(
            corrected_text("Pentiment", with(None, true), none, 0.5),
            "Pentiment shows in Discover again"
        );
    }

    fn tags_view() -> GameTagsView {
        GameTagsView {
            tags: Vec::new(),
            weighted: false,
            used: Vec::new(),
        }
    }

    #[test]
    fn the_tags_button_is_there_only_for_a_game_in_the_catalog() {
        assert!(tags_button(Some(&tags_view())).is_some());
        assert!(tags_button(None).is_none());
    }

    #[test]
    fn a_chip_puts_the_game_in_or_takes_it_out() {
        assert_eq!(
            chip_change("cozy", AppId(1), false),
            FamilyChange::AddGame {
                family: "cozy".to_string(),
                game: AppId(1),
            }
        );
        assert_eq!(
            chip_change("cozy", AppId(1), true),
            FamilyChange::RemoveGame {
                family: "cozy".to_string(),
                game: AppId(1),
            }
        );
    }

    #[test]
    fn the_toast_says_what_changed_in_the_families() {
        let mut celeste = game(1, "Celeste");
        let add = FamilyChange::AddGame {
            family: " slow puzzles ".to_string(),
            game: AppId(1),
        };
        let remove = FamilyChange::RemoveGame {
            family: "cozy".to_string(),
            game: AppId(1),
        };
        let hold_out = FamilyChange::HoldOut { game: AppId(1) };

        assert_eq!(families_text(&add, &celeste, &[]), "Added to slow puzzles");
        assert_eq!(families_text(&hold_out, &celeste, &[]), "Held out");
        assert_eq!(
            families_text(&FamilyChange::Release { game: AppId(1) }, &celeste, &[]),
            "No longer held out"
        );
        assert_eq!(families_text(&remove, &celeste, &[]), "Removed from cozy");
        assert_eq!(
            families_text(&remove, &celeste, &["cozy".to_string()]),
            "Removed from cozy, which is now gone"
        );

        celeste.families = vec!["cozy".to_string(), "platformers".to_string()];
        assert_eq!(
            families_text(&hold_out, &celeste, &[]),
            "Held out, removed from cozy, platformers"
        );
        celeste.held_out = true;
        assert_eq!(
            families_text(&add, &celeste, &[]),
            "Added to slow puzzles, no longer held out"
        );
    }

    #[test]
    fn only_a_game_played_only_elsewhere_can_be_removed_from_the_library() {
        let only = elsewhere(7, "It Takes Two");
        let owned_too = LibraryGameView {
            elsewhere_only: false,
            ..owned(8, "Portal 2", 0.0, 0.1)
        };

        assert!(matches!(
            removal(&only),
            Some(Message::PlayElsewhere(AppId(7), _, false))
        ));
        assert!(removal(&owned_too).is_none());
        assert!(removal(&games()[0]).is_none());
    }

    #[test]
    fn a_game_played_elsewhere_says_so_on_its_tile_and_in_the_popup() {
        let view = view_of(Vec::new());
        let only = elsewhere(7, "It Takes Two");
        let owned_too = LibraryGameView {
            elsewhere: true,
            ..owned(8, "Portal 2", 40.0, 0.1)
        };

        assert_eq!(tile_label(&only), "elsewhere");
        assert_eq!(
            source_line(&only, &view),
            "Played elsewhere · counts as 20 hours"
        );
        assert_eq!(
            source_line(&owned_too, &view),
            "Owned and played elsewhere · 40 h"
        );
    }
}
