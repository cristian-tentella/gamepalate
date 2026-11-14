use gamepalate_app::{Ban, HoursGame, HoursSource, SignalsConfig, SourceView, TagView, TasteView};
use iced::{
    Center, Color, Element, Fill, Length, Theme, mouse,
    widget::{button, column, container, mouse_area, progress_bar, row, space, text, tooltip},
};

use super::{CLOSED, Message, Profile, families::games_count};
use crate::{
    font,
    screen::{library, votes::Filter},
    style,
};

const FIRST_TAGS: usize = 12;
const HEADER_WIDTH: f32 = 140.0;
const HEADER_HEIGHT: f32 = HEADER_WIDTH * 215.0 / 460.0;
const TAG_NAME_WIDTH: f32 = 150.0;
const TAG_BAR_WIDTH: f32 = 140.0;

const SHAPING_NOTE: &str =
    "Hours count less as they grow, and games with more uncommon tags count more";

const BAR_HEIGHT: f32 = 8.0;
const PORTIONS: u16 = 1000;
const WARNING: char = '\u{F083}';
const SETTINGS_HINT: &str = "Change it in Settings, under Taste";
const SOURCE_BAR_WIDTH: f32 = 220.0;
const PERCENT_WIDTH: f32 = 40.0;

fn sources<'a>(taste: &TasteView, hours: &SignalsConfig) -> Element<'a, Message> {
    column(shown_sources(&taste.sources).into_iter().map(|source| {
        let line = row![
            text(source_label(source.source))
                .size(font::BAR_SIZE)
                .width(TAG_NAME_WIDTH),
            progress_bar(0.0..=1.0, source.share as f32)
                .girth(BAR_HEIGHT)
                .length(SOURCE_BAR_WIDTH)
                .style(style::meter),
            container(text(percent(source.share)).size(font::BAR_SIZE)).align_right(PERCENT_WIDTH),
            text(source_text(&source, hours))
                .size(font::NOTE_SIZE)
                .style(style::muted)
                .width(Fill),
            text(CLOSED).font(font::ICONS).size(font::icon(10.0)),
        ]
        .spacing(10)
        .align_y(Center);

        tooltip(
            button(line)
                .width(Fill)
                .padding([6, 8])
                .style(style::list_row(false))
                .on_press(source_message(source.source)),
            container(text(source_tooltip(source.source, hours)).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    }))
    .spacing(2)
    .into()
}

fn tag_row(tag: &TagView, top: f64) -> Element<'_, Message> {
    let (solid, ghost, rest) = bar_portions(tag.weight, tag.weight_before, top);
    let bar = row![]
        .push((solid > 0).then(|| {
            container(space())
                .width(Length::FillPortion(solid))
                .height(BAR_HEIGHT)
                .style(|theme: &Theme| filled(theme.extended_palette().primary.base.color))
        }))
        .push((ghost > 0).then(|| {
            container(space())
                .width(Length::FillPortion(ghost))
                .height(BAR_HEIGHT)
                .style(|theme: &Theme| {
                    filled(theme.extended_palette().primary.base.color.scale_alpha(0.3))
                })
        }))
        .push((rest > 0).then(|| space().width(Length::FillPortion(rest))))
        .width(TAG_BAR_WIDTH)
        .align_y(Center);

    let tone: fn(&Theme) -> text::Style = if tag.dominant {
        warning_text
    } else {
        style::muted
    };
    let games = tooltip(
        text(tag_games_text(tag)).size(font::NOTE_SIZE).style(tone),
        container(text(games_line(tag)).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Bottom,
    );
    let note = tag_note(tag);

    container(
        row![
            text(&tag.name).size(font::BAR_SIZE).width(TAG_NAME_WIDTH),
            bar,
            container(games).width(Fill),
        ]
        .push((!note.is_empty()).then(|| text(note).size(font::NOTE_SIZE).style(style::muted)))
        .spacing(10)
        .align_y(Center),
    )
    .padding([6, 8])
    .into()
}

fn dominant(taste: &TasteView) -> Option<Element<'_, Message>> {
    let tags = dominant_tags(taste);
    if tags.is_empty() {
        return None;
    }

    let title = row![
        text(WARNING)
            .font(font::ICONS)
            .size(font::icon(font::NOTE_SIZE))
            .style(warning_text),
        text("Mostly from one game")
            .size(font::BAR_SIZE)
            .font(font::SEMIBOLD)
            .style(warning_text),
    ]
    .spacing(8)
    .align_y(Center);

    let lines = tags.into_iter().map(|(tag, game)| {
        tooltip(
            button(text!("{} · {} ›", tag.name, game.name).size(font::BAR_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(dominant_message(game)),
            container(text(dampen_hint(game)).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    });

    Some(
        container(column![title].extend(lines).spacing(6))
            .padding([6, 8])
            .into(),
    )
}

fn filled(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        border: iced::Border {
            radius: 2.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

fn warning_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().warning.strong.color),
    }
}

fn top_weight(tags: &[TagView]) -> f64 {
    tags.iter()
        .map(|tag| tag.weight.max(tag.weight_before))
        .fold(0.0, f64::max)
}

fn portion(share: f64) -> u16 {
    (share.clamp(0.0, 1.0) * f64::from(PORTIONS)).round() as u16
}

fn bar_portions(weight: f64, weight_before: f64, top: f64) -> (u16, u16, u16) {
    if top <= 0.0 {
        return (0, 0, PORTIONS);
    }
    let solid = portion(weight.min(weight_before) / top);
    let ghost = portion(weight.max(weight_before) / top).saturating_sub(solid);

    (solid, ghost, PORTIONS - solid - ghost)
}

fn more_tags_label(tags: usize, all_tags: bool) -> String {
    if all_tags {
        "Show fewer tags".to_string()
    } else {
        format!("Show {} more tags", tags - FIRST_TAGS)
    }
}

fn headline(taste: &TasteView) -> String {
    let names: Vec<&str> = taste
        .tags
        .iter()
        .take(3)
        .map(|tag| tag.name.as_str())
        .collect();
    let tags = match names.as_slice() {
        [] => String::new(),
        [only] => only.to_string(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    };
    format!("Mostly {tags}, from {}", games_count(taste.game_count))
}

fn source_label(source: HoursSource) -> &'static str {
    match source {
        HoursSource::Played => "Played",
        HoursSource::Wishlist => "Wishlist",
        HoursSource::Followed => "Followed",
        HoursSource::WouldTry => "Would try",
        HoursSource::Elsewhere => "Played elsewhere",
    }
}

fn shown_sources(sources: &[SourceView]) -> Vec<SourceView> {
    let mut shown: Vec<SourceView> = sources
        .iter()
        .filter(|source| source.share > 0.0)
        .copied()
        .collect();
    shown.sort_by(|a, b| b.share.total_cmp(&a.share));
    shown
}

fn source_text(source: &SourceView, hours: &SignalsConfig) -> String {
    let each = |hours: f64| format!("{} each", hours_text(hours));
    match source.source {
        HoursSource::Played => format!("{}, by hours played", games_count(source.games)),
        HoursSource::Wishlist => {
            format!(
                "{}, {}",
                games_count(source.games),
                each(hours.wishlist_hours)
            )
        }
        HoursSource::Followed => {
            format!(
                "{}, {}",
                games_count(source.games),
                each(hours.followed_hours)
            )
        }
        HoursSource::WouldTry => {
            format!(
                "{}, {}",
                votes_count(source.games),
                each(hours.would_try_hours)
            )
        }
        HoursSource::Elsewhere => {
            format!(
                "{}, {}",
                games_count(source.games),
                each(hours.elsewhere_hours)
            )
        }
    }
}

fn source_tooltip(source: HoursSource, hours: &SignalsConfig) -> String {
    match source {
        HoursSource::Played => format!(
            "Games with at least {} of play. {SETTINGS_HINT}",
            hours_text(hours.min_hours)
        ),
        HoursSource::Wishlist
        | HoursSource::Followed
        | HoursSource::WouldTry
        | HoursSource::Elsewhere => SETTINGS_HINT.to_string(),
    }
}

fn source_message(source: HoursSource) -> Message {
    match source {
        HoursSource::Played => Message::OpenLibrary(library::Filter::All, library::Sort::Hours),
        HoursSource::Wishlist => {
            Message::OpenLibrary(library::Filter::Wishlist, library::Sort::TasteWeight)
        }
        HoursSource::Followed => {
            Message::OpenLibrary(library::Filter::Followed, library::Sort::TasteWeight)
        }
        HoursSource::WouldTry => Message::OpenVotes(Filter::WouldTry),
        HoursSource::Elsewhere => {
            Message::OpenLibrary(library::Filter::Elsewhere, library::Sort::TasteWeight)
        }
    }
}

fn votes_count(votes: usize) -> String {
    if votes == 1 {
        "1 vote".to_string()
    } else {
        format!("{votes} votes")
    }
}

fn hours_text(hours: f64) -> String {
    if hours == 1.0 {
        "1 hour".to_string()
    } else {
        format!("{hours} hours")
    }
}

fn recent_note(games: usize, multiplier: f64) -> Option<String> {
    if games == 0 || multiplier == 1.0 {
        return None;
    }
    let times = match multiplier {
        2.0 => "twice".to_string(),
        3.0 => "three times".to_string(),
        m => format!("{m} times"),
    };

    Some(if games == 1 {
        format!("1 game played in the last two weeks counts {times}")
    } else {
        format!("{games} games played in the last two weeks count {times}")
    })
}

fn percent(share: f64) -> String {
    format!("{:.0}%", share * 100.0)
}

fn tag_note(tag: &TagView) -> String {
    let banned = tag.banned.map(|ban| match ban {
        Ban::Everywhere => "banned".to_string(),
        Ban::TopTags(n) => format!("banned in top {n}"),
    });
    let multiplier = tag.multiplier.map(|m| format!("×{m}"));

    [multiplier, banned]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn game_hours(game: &HoursGame) -> String {
    match game.source {
        HoursSource::Played => format!("{:.0} h", game.hours),
        HoursSource::Wishlist => "wishlist".to_string(),
        HoursSource::Followed => "followed".to_string(),
        HoursSource::WouldTry => "would try".to_string(),
        HoursSource::Elsewhere => "elsewhere".to_string(),
    }
}

fn shaping_text(game: &HoursGame) -> String {
    if game.recent {
        format!("{} · recent", game_hours(game))
    } else {
        game_hours(game)
    }
}

fn games_line(tag: &TagView) -> String {
    let mut parts: Vec<String> = tag
        .games
        .iter()
        .map(|game| format!("{} {}", game.name, game_hours(game)))
        .collect();
    let others = tag.game_count.saturating_sub(tag.games.len());
    if others > 0 {
        parts.push(format!("+{others} more"));
    }

    parts.join(" · ")
}

fn short_name(name: &str) -> &str {
    let end = [": ", " - "]
        .into_iter()
        .filter_map(|separator| name.find(separator))
        .min()
        .unwrap_or(name.len());
    let short = name[..end].trim();

    if short.is_empty() { name } else { short }
}

fn tag_games_text(tag: &TagView) -> String {
    tag.games
        .iter()
        .map(|game| short_name(&game.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn dominant_tags(taste: &TasteView) -> Vec<(&TagView, &HoursGame)> {
    taste
        .tags
        .iter()
        .filter(|tag| tag.dominant)
        .filter_map(|tag| tag.games.first().map(|game| (tag, game)))
        .collect()
}

fn dominant_message(game: &HoursGame) -> Message {
    if in_library(game) {
        Message::OpenInLibrary(game.id)
    } else {
        Message::OpenVotes(Filter::WouldTry)
    }
}

fn in_library(game: &HoursGame) -> bool {
    game.source != HoursSource::WouldTry
}

fn dampen_hint(game: &HoursGame) -> String {
    if in_library(game) {
        "Open it in Library to make it count less".to_string()
    } else {
        format!("To make {} count less, change its vote in Votes", game.name)
    }
}

pub(super) fn no_taste_text(min_hours: f64) -> String {
    format!(
        "None of your games has {} of play yet. Play one for a while or add some to your wishlist.",
        hours_text(min_hours)
    )
}

impl Profile {
    fn shown_tags(&self) -> usize {
        let tags = self.view.taste.as_ref().map_or(0, |taste| taste.tags.len());
        if self.all_tags {
            tags
        } else {
            tags.min(FIRST_TAGS)
        }
    }

    pub(super) fn taste<'a>(&'a self, taste: &'a TasteView) -> Element<'a, Message> {
        let shown = self.shown_tags();
        let top = top_weight(&taste.tags);
        let tags = column(taste.tags[..shown].iter().map(|tag| tag_row(tag, top))).spacing(2);
        let more = (taste.tags.len() > FIRST_TAGS).then(|| {
            button(text(more_tags_label(taste.tags.len(), self.all_tags)).size(font::BAR_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::ToggleAllTags)
        });

        column![
            text(headline(taste)).size(font::INFO_SIZE),
            sources(taste, &self.view.hours),
        ]
        .push(
            recent_note(taste.recent_games, self.view.hours.recent_hours_multiplier)
                .map(|note| text(note).size(font::NOTE_SIZE).style(style::muted)),
        )
        .push(space().height(6))
        .push(self.shaping(taste))
        .push(space().height(6))
        .push(dominant(taste))
        .push(tags)
        .push(more)
        .spacing(10)
        .into()
    }

    fn shaping<'a>(&'a self, taste: &'a TasteView) -> Element<'a, Message> {
        let games = row(taste.shaping_games.iter().map(|game| {
            let header = self
                .showcase
                .header(game.id, HEADER_WIDTH, HEADER_HEIGHT)
                .map(Message::Showcase);
            let header: Element<'_, Message> = if in_library(game) {
                mouse_area(header)
                    .interaction(mouse::Interaction::Pointer)
                    .on_press(Message::OpenInLibrary(game.id))
                    .into()
            } else {
                header
            };
            column![
                header,
                text(&game.name).size(font::NOTE_SIZE),
                text(shaping_text(game))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .spacing(4)
            .width(HEADER_WIDTH)
            .into()
        }))
        .spacing(10)
        .wrap()
        .vertical_spacing(12);

        column![
            row![
                text("Counts most")
                    .size(font::BAR_SIZE)
                    .font(font::SEMIBOLD),
                space().width(Fill),
                button(text("See all in Library ›").size(font::BAR_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenLibrary(
                        library::Filter::All,
                        library::Sort::TasteWeight,
                    )),
            ]
            .align_y(Center),
            text(SHAPING_NOTE).size(font::NOTE_SIZE).style(style::muted),
            games,
        ]
        .spacing(8)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::AppId;

    use super::*;

    fn game(name: &str, hours: f64, source: HoursSource) -> HoursGame {
        HoursGame {
            id: AppId(202990),
            name: name.to_string(),
            hours,
            source,
            recent: false,
        }
    }

    fn tag(name: &str) -> TagView {
        TagView {
            name: name.to_string(),
            weight: 1.0,
            weight_before: 1.0,
            multiplier: None,
            banned: None,
            games: Vec::new(),
            game_count: 0,
            dominant: false,
        }
    }

    pub(super) fn taste(tags: &[&str], game_count: usize) -> TasteView {
        TasteView {
            game_count,
            recent_games: 0,
            sources: Vec::new(),
            shaping_games: Vec::new(),
            tags: tags.iter().map(|&name| tag(name)).collect(),
        }
    }

    fn source(source: HoursSource, share: f64, games: usize) -> SourceView {
        SourceView {
            source,
            share,
            games,
        }
    }

    #[test]
    fn the_headline_names_the_first_three_tags() {
        assert_eq!(
            headline(&taste(
                &["Story Rich", "Open World", "Exploration", "RPG"],
                354
            )),
            "Mostly Story Rich, Open World and Exploration, from 354 games"
        );
        assert_eq!(
            headline(&taste(&["Puzzle", "Mystery"], 2)),
            "Mostly Puzzle and Mystery, from 2 games"
        );
        assert_eq!(
            headline(&taste(&["Puzzle"], 1)),
            "Mostly Puzzle, from 1 game"
        );
    }

    #[test]
    fn the_sources_go_from_the_biggest_and_leave_out_the_empty_ones() {
        let shown = shown_sources(&[
            source(HoursSource::Played, 0.3, 161),
            source(HoursSource::Followed, 0.0, 0),
            source(HoursSource::Wishlist, 0.18, 135),
            source(HoursSource::WouldTry, 0.46, 407),
        ]);

        assert_eq!(
            shown.iter().map(|view| view.source).collect::<Vec<_>>(),
            [
                HoursSource::WouldTry,
                HoursSource::Played,
                HoursSource::Wishlist
            ]
        );
    }

    #[test]
    fn the_sources_tell_how_many_games_and_how_many_hours_each() {
        let hours = SignalsConfig::default();

        assert_eq!(
            source_text(&source(HoursSource::Played, 0.3, 161), &hours),
            "161 games, by hours played"
        );
        assert_eq!(
            source_text(&source(HoursSource::WouldTry, 0.46, 407), &hours),
            "407 votes, 5 hours each"
        );
        assert_eq!(
            source_text(&source(HoursSource::Followed, 0.06, 1), &hours),
            "1 game, 3 hours each"
        );
        assert_eq!(
            source_text(&source(HoursSource::Elsewhere, 0.04, 3), &hours),
            "3 games, 20 hours each"
        );
        assert_eq!(source_label(HoursSource::Elsewhere), "Played elsewhere");
    }

    #[test]
    fn a_source_says_where_to_change_what_it_counts() {
        let hours = SignalsConfig::default();

        assert_eq!(
            source_tooltip(HoursSource::Played, &hours),
            "Games with at least 2 hours of play. Change it in Settings, under Taste"
        );
        assert_eq!(
            source_tooltip(HoursSource::WouldTry, &hours),
            "Change it in Settings, under Taste"
        );
    }

    #[test]
    fn a_source_opens_where_its_games_are() {
        assert!(matches!(
            source_message(HoursSource::Played),
            Message::OpenLibrary(library::Filter::All, library::Sort::Hours)
        ));
        assert!(matches!(
            source_message(HoursSource::Wishlist),
            Message::OpenLibrary(library::Filter::Wishlist, library::Sort::TasteWeight)
        ));
        assert!(matches!(
            source_message(HoursSource::WouldTry),
            Message::OpenVotes(Filter::WouldTry)
        ));
        assert!(matches!(
            source_message(HoursSource::Elsewhere),
            Message::OpenLibrary(library::Filter::Elsewhere, library::Sort::TasteWeight)
        ));
    }

    #[test]
    fn the_recent_note_says_how_much_recent_games_count() {
        assert_eq!(
            recent_note(6, 3.0).unwrap(),
            "6 games played in the last two weeks count three times"
        );
        assert_eq!(
            recent_note(1, 2.0).unwrap(),
            "1 game played in the last two weeks counts twice"
        );
        assert_eq!(recent_note(0, 3.0), None);
        assert_eq!(recent_note(4, 1.0), None);
    }

    #[test]
    fn a_bar_is_as_long_as_its_weight_against_the_top_one() {
        assert_eq!(bar_portions(10.0, 10.0, 10.0), (1000, 0, 0));
        assert_eq!(bar_portions(5.0, 5.0, 10.0), (500, 0, 500));
        assert_eq!(bar_portions(0.0, 0.0, 0.0), (0, 0, 1000));
    }

    #[test]
    fn a_corrected_bar_shows_what_the_multiplier_changed() {
        assert_eq!(bar_portions(4.0, 8.0, 10.0), (400, 400, 200));
        assert_eq!(bar_portions(10.0, 5.0, 10.0), (500, 500, 0));
    }

    #[test]
    fn the_top_weight_counts_the_weight_before_a_boost() {
        let boosted = TagView {
            weight: 4.0,
            weight_before: 2.0,
            ..tag("Roguelike")
        };
        let dampened = TagView {
            weight: 3.0,
            weight_before: 6.0,
            ..tag("Souls-like")
        };

        assert_eq!(top_weight(&[boosted, dampened]), 6.0);
    }

    #[test]
    fn the_note_of_a_tag_shows_its_corrections() {
        let corrected = TagView {
            multiplier: Some(0.5),
            banned: Some(Ban::TopTags(8)),
            ..tag("JRPG")
        };
        let banned = TagView {
            banned: Some(Ban::Everywhere),
            ..tag("Horror")
        };

        assert_eq!(tag_note(&corrected), "×0.5 · banned in top 8");
        assert_eq!(tag_note(&banned), "banned");
        assert_eq!(tag_note(&tag("Puzzle")), "");
    }

    #[test]
    fn the_tooltip_of_a_tag_lists_its_games_and_how_many_more() {
        let open = TagView {
            games: vec![
                game("Outer Wilds", 64.4, HoursSource::Played),
                game("Blue Prince", 5.0, HoursSource::Wishlist),
            ],
            game_count: 60,
            ..tag("Exploration")
        };
        let all = TagView {
            game_count: 2,
            ..open.clone()
        };

        assert_eq!(
            games_line(&open),
            "Outer Wilds 64 h · Blue Prince wishlist · +58 more"
        );
        assert_eq!(games_line(&all), "Outer Wilds 64 h · Blue Prince wishlist");
    }

    #[test]
    fn a_game_shows_its_hours_or_the_signal_that_brought_it() {
        assert_eq!(
            game_hours(&game("Skyrim", 199.6, HoursSource::Played)),
            "200 h"
        );
        assert_eq!(
            game_hours(&game("Hades II", 5.0, HoursSource::Wishlist)),
            "wishlist"
        );
        assert_eq!(
            game_hours(&game("Hollow Knight: Silksong", 3.0, HoursSource::Followed)),
            "followed"
        );
        assert_eq!(
            game_hours(&game("Blue Prince", 5.0, HoursSource::WouldTry)),
            "would try"
        );
    }

    #[test]
    fn a_game_played_lately_says_so() {
        let recent = HoursGame {
            recent: true,
            ..game("Dwarf Fortress", 86.0, HoursSource::Played)
        };

        assert_eq!(shaping_text(&recent), "86 h · recent");
        assert_eq!(
            shaping_text(&game("ELDEN RING", 376.0, HoursSource::Played)),
            "376 h"
        );
    }

    #[test]
    fn the_dominant_game_opens_in_library() {
        assert_eq!(
            dampen_hint(&game("Call of Duty", 300.0, HoursSource::Played)),
            "Open it in Library to make it count less"
        );
        assert_eq!(
            dampen_hint(&game("Hades", 1.0, HoursSource::WouldTry)),
            "To make Hades count less, change its vote in Votes"
        );
        assert!(!in_library(&game("Hades", 1.0, HoursSource::WouldTry)));
    }

    #[test]
    fn the_more_tags_button_counts_the_hidden_ones() {
        assert_eq!(more_tags_label(25, false), "Show 13 more tags");
        assert_eq!(more_tags_label(25, true), "Show fewer tags");
    }

    #[test]
    fn a_short_name_stops_before_the_subtitle() {
        assert_eq!(short_name("Clair Obscur: Expedition 33"), "Clair Obscur");
        assert_eq!(short_name("Disco Elysium - The Final Cut"), "Disco Elysium");
        assert_eq!(
            short_name("Call of Duty: Black Ops II - Multiplayer"),
            "Call of Duty"
        );
        assert_eq!(short_name("OMORI"), "OMORI");
        assert_eq!(short_name(": odd"), ": odd");
    }

    #[test]
    fn a_tag_shows_the_short_names_of_its_games() {
        let story = TagView {
            games: vec![
                game("Clair Obscur: Expedition 33", 120.0, HoursSource::Played),
                game("Disco Elysium - The Final Cut", 40.0, HoursSource::Played),
                game("OMORI", 30.0, HoursSource::Played),
            ],
            ..tag("Story Rich")
        };

        assert_eq!(tag_games_text(&story), "Clair Obscur, Disco Elysium, OMORI");
    }

    #[test]
    fn the_dominant_tags_come_from_all_the_tags_not_only_the_shown_ones() {
        let mut taste = taste(
            &[
                "Story Rich",
                "RPG",
                "Open World",
                "Great Soundtrack",
                "Atmospheric",
                "Exploration",
                "Adventure",
                "Souls-like",
                "Action RPG",
                "Puzzle",
                "Difficult",
                "Metroidvania",
                "Action",
            ],
            717,
        );
        taste.tags[12].dominant = true;
        taste.tags[12].games = vec![game(
            "Call of Duty: Black Ops II - Multiplayer",
            300.0,
            HoursSource::Played,
        )];

        let dominant = dominant_tags(&taste);
        assert_eq!(dominant.len(), 1);
        assert_eq!(dominant[0].0.name, "Action");
        assert!(matches!(
            dominant_message(dominant[0].1),
            Message::OpenInLibrary(AppId(202990))
        ));
        assert!(matches!(
            dominant_message(&game("Hades", 5.0, HoursSource::WouldTry)),
            Message::OpenVotes(Filter::WouldTry)
        ));
    }

    #[test]
    fn without_a_taste_the_screen_says_how_to_get_one() {
        assert!(no_taste_text(2.0).starts_with("None of your games has 2 hours"));
    }
}
