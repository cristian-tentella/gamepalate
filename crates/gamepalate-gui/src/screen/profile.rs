use std::sync::Arc;

use gamepalate_app::{
    AppId, Ban, Gamepalate, HoursGame, HoursSource, ProfileView, SignalsConfig, TagView, TasteView,
};
use iced::{
    Center, Color, Element, Fill, Length, Task, Theme, padding,
    widget::{button, column, container, row, scrollable, space, text, tooltip},
};

use super::showcase::{self, Showcase};
use crate::{font, style};

const SOURCES: [HoursSource; 4] = [
    HoursSource::Played,
    HoursSource::Wishlist,
    HoursSource::Followed,
    HoursSource::WouldTry,
];
const FIRST_TAGS: usize = 12;
const COLUMN_WIDTH: f32 = 760.0;
const HEADER_WIDTH: f32 = 140.0;
const HEADER_HEIGHT: f32 = HEADER_WIDTH * 215.0 / 460.0;
const TAG_NAME_WIDTH: f32 = 150.0;
const NOTE_WIDTH: f32 = 110.0;
const BAR_HEIGHT: f32 = 8.0;
const PORTIONS: u16 = 1000;
const WARNING: char = '\u{E7BA}';
const CLOSED: char = '\u{E76C}';
const OPENED: char = '\u{E70D}';

pub struct Profile {
    view: ProfileView,
    open_tag: Option<usize>,
    all_tags: bool,
    showcase: Showcase,
}

#[derive(Clone, Debug)]
pub enum Message {
    ToggleTag(usize),
    ToggleAllTags,
    Showcase(showcase::Message),
}

impl Profile {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Profile, Task<Message>) {
        let mut profile = Profile {
            view: gamepalate.profile_view(),
            open_tag: None,
            all_tags: false,
            showcase: Showcase::new(gamepalate),
        };
        let task = profile.load();

        (profile, task)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);

        Task::batch([task, self.load()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleTag(index) => {
                self.open_tag = (self.open_tag != Some(index)).then_some(index);
            }
            Message::ToggleAllTags => {
                self.all_tags = !self.all_tags;
                if self.open_tag.is_some_and(|index| index >= FIRST_TAGS) {
                    self.open_tag = None;
                }
            }
            Message::Showcase(message) => {
                return self.showcase.update(message).map(Message::Showcase);
            }
        }

        Task::none()
    }

    fn load(&mut self) -> Task<Message> {
        let ids: Vec<AppId> = self
            .view
            .taste
            .iter()
            .flat_map(|taste| &taste.shaping_games)
            .map(|game| game.id)
            .collect();

        self.showcase.load_headers(&ids).map(Message::Showcase)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let taste = match &self.view.taste {
            Some(taste) => self.taste(taste),
            None => text(no_taste_text(self.view.hours.min_hours))
                .size(font::BAR_SIZE)
                .into(),
        };

        scrollable(
            container(
                column![section_title("Your taste"), taste]
                    .spacing(14)
                    .max_width(COLUMN_WIDTH),
            )
            .padding(24)
            .width(Fill),
        )
        .height(Fill)
        .into()
    }

    fn taste<'a>(&'a self, taste: &'a TasteView) -> Element<'a, Message> {
        let shown = if self.all_tags {
            taste.tags.len()
        } else {
            taste.tags.len().min(FIRST_TAGS)
        };
        let top = top_weight(&taste.tags);
        let tags = column(
            taste.tags[..shown]
                .iter()
                .enumerate()
                .map(|(index, tag)| tag_row(index, tag, top, self.open_tag == Some(index))),
        )
        .spacing(2);
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
        .push(tags)
        .push(more)
        .spacing(10)
        .into()
    }
}

impl Profile {
    fn shaping<'a>(&'a self, taste: &'a TasteView) -> Element<'a, Message> {
        let games = row(taste.shaping_games.iter().map(|game| {
            column![
                self.showcase
                    .header(game.id, HEADER_WIDTH, HEADER_HEIGHT)
                    .map(Message::Showcase),
                text(&game.name).size(font::NOTE_SIZE),
                text(game_hours(game))
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
            text("Shaped most by")
                .size(font::BAR_SIZE)
                .font(font::SEMIBOLD),
            games,
        ]
        .spacing(8)
        .into()
    }
}

fn section_title(title: &str) -> Element<'_, Message> {
    text(title)
        .size(font::PRICE_SIZE)
        .font(font::SEMIBOLD)
        .into()
}

fn sources<'a>(taste: &TasteView, hours: &SignalsConfig) -> Element<'a, Message> {
    let shares: Vec<(HoursSource, f64)> = SOURCES
        .into_iter()
        .filter_map(|source| {
            let share = taste
                .sources
                .iter()
                .find(|&&(kind, _)| kind == source)
                .map_or(0.0, |&(_, share)| share);
            (share > 0.0).then_some((source, share))
        })
        .collect();

    let bar = row(shares.iter().map(|&(source, share)| {
        container(space())
            .width(Length::FillPortion(portion(share)))
            .height(BAR_HEIGHT)
            .style(move |theme: &Theme| filled(source_color(theme, source)))
            .into()
    }))
    .spacing(2)
    .width(Fill);

    let legend = row(shares.iter().map(|&(source, share)| {
        let label = row![
            container(space())
                .width(10)
                .height(10)
                .style(move |theme: &Theme| filled(source_color(theme, source))),
            text!("{} {}", source_label(source), percent(share)).size(font::NOTE_SIZE),
        ]
        .spacing(6)
        .align_y(Center);

        tooltip(
            label,
            container(text(source_tooltip(source, hours)).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    }))
    .spacing(16)
    .wrap()
    .vertical_spacing(6);

    column![bar, legend].spacing(8).into()
}

fn tag_row(index: usize, tag: &TagView, top: f64, open: bool) -> Element<'_, Message> {
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
        .width(Fill)
        .align_y(Center);

    let line = row![
        text(&tag.name).size(font::BAR_SIZE).width(TAG_NAME_WIDTH),
        bar,
        container(
            text(tag_note(tag))
                .size(font::NOTE_SIZE)
                .style(style::muted)
        )
        .align_right(NOTE_WIDTH),
        text(if open { OPENED } else { CLOSED })
            .font(font::ICONS)
            .size(10),
    ]
    .spacing(10)
    .align_y(Center);

    let header = button(line)
        .width(Fill)
        .padding([6, 8])
        .style(style::list_row(false))
        .on_press(Message::ToggleTag(index));

    let below = padding::left(TAG_NAME_WIDTH + 18.0);
    let dominant = tag.games.first().filter(|_| tag.dominant).map(|game| {
        let warning = row![
            text(WARNING)
                .font(font::ICONS)
                .size(font::NOTE_SIZE)
                .style(warning_text),
            text(format!("mostly from {}", game.name))
                .size(font::NOTE_SIZE)
                .style(warning_text),
        ]
        .spacing(6)
        .align_y(Center);

        container(tooltip(
            warning,
            container(text(dampen_hint(game)).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        ))
        .padding(below)
    });
    let games = open.then(|| {
        container(
            text(games_line(tag))
                .size(font::NOTE_SIZE)
                .style(style::muted),
        )
        .padding(below.right(8))
    });

    column![header].push(dominant).push(games).spacing(2).into()
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

fn source_color(theme: &Theme, source: HoursSource) -> Color {
    let palette = theme.extended_palette();
    match source {
        HoursSource::Played => palette.primary.base.color,
        HoursSource::Wishlist => palette.success.base.color,
        HoursSource::Followed => palette.warning.base.color,
        HoursSource::WouldTry => palette.secondary.strong.color,
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
    let games = if taste.game_count == 1 {
        "1 game".to_string()
    } else {
        format!("{} games", taste.game_count)
    };

    format!("Mostly {tags}, from {games}.")
}

fn source_label(source: HoursSource) -> &'static str {
    match source {
        HoursSource::Played => "Played",
        HoursSource::Wishlist => "Wishlist",
        HoursSource::Followed => "Followed",
        HoursSource::WouldTry => "Would try",
    }
}

fn source_tooltip(source: HoursSource, hours: &SignalsConfig) -> String {
    match source {
        HoursSource::Played => format!(
            "Games you played for at least {}, by the hours you played.",
            hours_text(hours.min_hours)
        ),
        HoursSource::Wishlist => format!(
            "A game in your wishlist counts as {} of play.",
            hours_text(hours.wishlist_hours)
        ),
        HoursSource::Followed => format!(
            "A game you follow counts as {} of play.",
            hours_text(hours.followed_hours)
        ),
        HoursSource::WouldTry => format!(
            "A game you voted Would try counts as {} of play.",
            hours_text(hours.would_try_hours)
        ),
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
        format!("1 game played in the last two weeks counts {times}.")
    } else {
        format!("{games} games played in the last two weeks count {times}.")
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

fn dampen_hint(game: &HoursGame) -> String {
    format!(
        "If this game shouldn't shape your taste: gamepalate dampen --game {}",
        game.id.0
    )
}

fn no_taste_text(min_hours: f64) -> String {
    format!(
        "No taste yet: none of your games has {} of play. Play one for a while, or add some to your wishlist.",
        hours_text(min_hours)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(name: &str, hours: f64, source: HoursSource) -> HoursGame {
        HoursGame {
            id: AppId(202990),
            name: name.to_string(),
            hours,
            source,
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

    fn taste(tags: &[&str], game_count: usize) -> TasteView {
        TasteView {
            game_count,
            recent_games: 0,
            sources: Vec::new(),
            shaping_games: Vec::new(),
            tags: tags.iter().map(|&name| tag(name)).collect(),
        }
    }

    #[test]
    fn the_headline_names_the_first_three_tags() {
        assert_eq!(
            headline(&taste(
                &["Story Rich", "Open World", "Exploration", "RPG"],
                354
            )),
            "Mostly Story Rich, Open World and Exploration, from 354 games."
        );
        assert_eq!(
            headline(&taste(&["Puzzle", "Mystery"], 2)),
            "Mostly Puzzle and Mystery, from 2 games."
        );
        assert_eq!(
            headline(&taste(&["Puzzle"], 1)),
            "Mostly Puzzle, from 1 game."
        );
    }

    #[test]
    fn the_sources_tell_how_many_hours_a_signal_is_worth() {
        let hours = SignalsConfig::default();

        assert_eq!(
            source_tooltip(HoursSource::Wishlist, &hours),
            "A game in your wishlist counts as 5 hours of play."
        );
        assert_eq!(
            source_tooltip(HoursSource::Played, &hours),
            "Games you played for at least 2 hours, by the hours you played."
        );
    }

    #[test]
    fn the_recent_note_says_how_much_recent_games_count() {
        assert_eq!(
            recent_note(6, 3.0).unwrap(),
            "6 games played in the last two weeks count three times."
        );
        assert_eq!(
            recent_note(1, 2.0).unwrap(),
            "1 game played in the last two weeks counts twice."
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
    fn an_open_tag_lists_its_games_and_how_many_more() {
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
    fn the_dominant_game_comes_with_the_command_to_dampen_it() {
        assert_eq!(
            dampen_hint(&game("Call of Duty", 300.0, HoursSource::Played)),
            "If this game shouldn't shape your taste: gamepalate dampen --game 202990"
        );
    }

    #[test]
    fn the_more_tags_button_counts_the_hidden_ones() {
        assert_eq!(more_tags_label(25, false), "Show 13 more tags");
        assert_eq!(more_tags_label(25, true), "Show fewer tags");
    }

    #[test]
    fn without_a_taste_the_screen_says_how_to_get_one() {
        assert!(no_taste_text(2.0).starts_with("No taste yet: none of your games has 2 hours"));
    }
}
