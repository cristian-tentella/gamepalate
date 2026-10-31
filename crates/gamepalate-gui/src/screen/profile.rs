use std::{collections::BTreeMap, path::PathBuf, process::Command, sync::Arc};

use gamepalate_app::{
    AppId, Ban, CorrectedGameView, CorrectionsView, FamilyView, Gamepalate, HoursGame, HoursSource,
    ProfileView, SignalsConfig, TagView, TasteView, Timestamp, VoteKind, VotedGame, VotesView,
};
use iced::{
    Center, Color, Element, Fill, Length, Task, Theme, padding,
    widget::{button, column, container, progress_bar, row, scrollable, space, text, tooltip},
};

use super::{
    showcase::{self, Showcase},
    votes::{Filter, bets_summary, by_id},
};
use crate::{font, format, style};

const SOURCES: [HoursSource; 4] = [
    HoursSource::Played,
    HoursSource::Wishlist,
    HoursSource::Followed,
    HoursSource::WouldTry,
];
const FIRST_TAGS: usize = 12;
const VOTE_KINDS: [VoteKind; 4] = [
    VoteKind::WouldTry,
    VoteKind::Maybe,
    VoteKind::NotForMe,
    VoteKind::Known,
];
const NO_VOTES: &str = "No votes yet. Vote in Discover with 1-4.";
const OPINION_ON: &str = "Your votes shape the list ✓";
const LABEL_BAR_WIDTH: f32 = 120.0;
const NO_FAMILIES: &str =
    "No families yet. A family is a group of games that stands for one of your tastes.";
const COLUMN_WIDTH: f32 = 760.0;
const HEADER_WIDTH: f32 = 140.0;
const HEADER_HEIGHT: f32 = HEADER_WIDTH * 215.0 / 460.0;
const SIDE_WIDTH: f32 = 380.0;
const FAMILY_HEADERS: usize = 4;
const FAMILY_HEADER_WIDTH: f32 = 80.0;
const FAMILY_HEADER_HEIGHT: f32 = FAMILY_HEADER_WIDTH * 215.0 / 460.0;
const FAMILY_TAGS: usize = 3;
const HELD_OUT_TIP: &str =
    "Games you like but kept out of every family on purpose, to test gamepalate.";
const TAG_NAME_WIDTH: f32 = 150.0;
const NOTE_WIDTH: f32 = 110.0;
const BAR_HEIGHT: f32 = 8.0;
const PORTIONS: u16 = 1000;
const WARNING: char = '\u{E7BA}';
const CLOSED: char = '\u{E76C}';
const OPENED: char = '\u{E70D}';

pub struct Profile {
    data_dir: PathBuf,
    view: ProfileView,
    voted: BTreeMap<AppId, VotedGame>,
    open_correction: Option<Correction>,
    open_tag: Option<usize>,
    all_tags: bool,
    showcase: Showcase,
}

#[derive(Clone, Debug)]
pub enum Message {
    ToggleTag(usize),
    ToggleAllTags,
    DiscoverFamily(String),
    OpenVotes(Filter),
    ToggleCorrection(Correction),
    OpenDataFolder,
    Showcase(showcase::Message),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Correction {
    Games,
    Hidden,
}

impl Profile {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Profile, Task<Message>) {
        let mut profile = Profile {
            data_dir: gamepalate.data_dir().to_path_buf(),
            view: gamepalate.profile_view(),
            voted: by_id(gamepalate.voted_games()),
            open_correction: None,
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
            Message::DiscoverFamily(_) | Message::OpenVotes(_) => {}
            Message::ToggleCorrection(correction) => {
                self.open_correction =
                    (self.open_correction != Some(correction)).then_some(correction);
            }
            Message::OpenDataFolder => {
                let _ = Command::new("explorer").arg(&self.data_dir).spawn();
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
            .chain(
                self.view
                    .families
                    .iter()
                    .flat_map(|family| family.games.iter().take(FAMILY_HEADERS).copied()),
            )
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

        let taste = column![section_title("Your taste"), taste]
            .spacing(14)
            .max_width(COLUMN_WIDTH);

        let side = column![self.families(), self.votes()]
            .push((!self.view.corrections.is_empty()).then(|| self.corrections()))
            .spacing(32);

        scrollable(
            column![
                container(
                    row![
                        container(taste).width(Fill),
                        container(side).width(SIDE_WIDTH),
                    ]
                    .spacing(32),
                )
                .padding(24)
                .width(Fill),
                style::separator(),
                container(self.data()).padding([12, 24]).width(Fill),
            ]
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

impl Profile {
    fn families(&self) -> Element<'_, Message> {
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
                button(text("Open families.toml").size(font::BAR_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenDataFolder),
            ]
            .spacing(8)
            .into()
        } else {
            column(view.families.iter().map(|family| self.family_card(family)))
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

        column![section_title("Families")]
            .push(problems)
            .push(body)
            .push(held_out)
            .spacing(14)
            .into()
    }

    fn family_card<'a>(&'a self, family: &'a FamilyView) -> Element<'a, Message> {
        let headers = row(family.games.iter().take(FAMILY_HEADERS).map(|&id| {
            self.showcase
                .header(id, FAMILY_HEADER_WIDTH, FAMILY_HEADER_HEIGHT)
                .map(Message::Showcase)
        }))
        .push(
            more_games(family.games.len())
                .map(|more| text(more).size(font::NOTE_SIZE).style(style::muted)),
        )
        .spacing(6)
        .align_y(Center);

        let tags = family
            .tags
            .iter()
            .take(FAMILY_TAGS)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" · ");

        container(
            column![
                headers,
                row![
                    text(&family.name).size(font::BAR_SIZE).font(font::SEMIBOLD),
                    text(games_count(family.games.len()))
                        .size(font::NOTE_SIZE)
                        .style(style::muted),
                ]
                .spacing(8)
                .align_y(Center),
                text(tags).size(font::NOTE_SIZE).style(style::muted),
                container(
                    button(text("Discover from it ›").size(font::BAR_SIZE))
                        .padding(0)
                        .style(style::link)
                        .on_press(Message::DiscoverFamily(family.name.clone())),
                )
                .align_right(Fill),
            ]
            .spacing(6),
        )
        .padding([10, 12])
        .width(Fill)
        .style(style::card)
        .into()
    }
}

impl Profile {
    fn votes(&self) -> Element<'_, Message> {
        let votes = &self.view.votes;
        let total = votes.would_try + votes.maybe + votes.not_for_me + votes.known;
        let title = section_title("Votes");
        if total == 0 {
            return column![title, text(NO_VOTES).size(font::BAR_SIZE)]
                .spacing(14)
                .into();
        }

        let counts = row(VOTE_KINDS.map(|kind| {
            button(text!("{} {}", vote_count(votes, kind), format::vote(kind)).size(font::BAR_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::OpenVotes(Filter::Kind(kind)))
                .into()
        }))
        .spacing(14)
        .wrap()
        .vertical_spacing(6);

        let opinion: Element<'_, Message> = if votes.labels.enough() {
            text(OPINION_ON).size(font::BAR_SIZE).into()
        } else {
            let needed = votes.min_labels;
            column![
                text(opinion_progress(votes))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                label_bar("Would try", votes.labels.would_try, needed),
                label_bar("Not for me", votes.labels.not_for_me, needed),
            ]
            .spacing(6)
            .into()
        };

        let bets = bets_summary(&self.voted).map(|(would_try, bets, others)| {
            button(text(format::bets_summary(would_try, bets, others)).size(font::NOTE_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::OpenVotes(Filter::Bets))
        });

        column![title, counts, opinion]
            .push(bets)
            .spacing(14)
            .into()
    }

    fn corrections(&self) -> Element<'_, Message> {
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
                        .map(corrected_game_text)
                        .collect(),
                ),
            );
        }
        if !corrections.hidden_games.is_empty() {
            lines.push(self.expandable(
                hidden_text(corrections.hidden_games.len()),
                Correction::Hidden,
                corrections.hidden_games.clone(),
            ));
        }

        column![
            row![
                section_title("Corrections"),
                text("edit with the gamepalate CLI")
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .spacing(10)
            .align_y(Center),
            column(lines).spacing(8),
        ]
        .spacing(14)
        .into()
    }

    fn expandable(
        &self,
        summary: String,
        correction: Correction,
        items: Vec<String>,
    ) -> Element<'_, Message> {
        let open = self.open_correction == Some(correction);
        let header = button(
            row![
                text(summary).size(font::BAR_SIZE),
                text(if open { OPENED } else { CLOSED })
                    .font(font::ICONS)
                    .size(10),
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
                    column(
                        items.into_iter().map(|item| {
                            text(item).size(font::NOTE_SIZE).style(style::muted).into()
                        }),
                    )
                    .spacing(2),
                )
                .padding(padding::left(12))
            }))
            .spacing(6)
            .into()
    }

    fn data(&self) -> Element<'_, Message> {
        let now = Timestamp(jiff::Timestamp::now().as_millisecond());

        row![
            text(data_text(&self.view, now))
                .size(font::NOTE_SIZE)
                .style(style::muted),
            button(text("Open data folder").size(font::NOTE_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(Message::OpenDataFolder),
        ]
        .spacing(12)
        .align_y(Center)
        .into()
    }
}

fn label_bar<'a>(label: &'a str, count: usize, needed: usize) -> Element<'a, Message> {
    row![
        text(label).size(font::NOTE_SIZE).width(80),
        progress_bar(0.0..=needed as f32, count.min(needed) as f32)
            .girth(4)
            .length(LABEL_BAR_WIDTH)
            .style(style::meter),
        text!("{count} of {needed}").size(font::NOTE_SIZE),
    ]
    .spacing(8)
    .align_y(Center)
    .into()
}

fn vote_count(votes: &VotesView, kind: VoteKind) -> usize {
    match kind {
        VoteKind::WouldTry => votes.would_try,
        VoteKind::Maybe => votes.maybe,
        VoteKind::NotForMe => votes.not_for_me,
        VoteKind::Known => votes.known,
    }
}

fn opinion_progress(votes: &VotesView) -> String {
    format!(
        "Your votes start shaping the list at {n} Would try and {n} Not for me: {} and {} so far.",
        votes.labels.would_try,
        votes.labels.not_for_me,
        n = votes.min_labels
    )
}

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

fn data_text(view: &ProfileView, now: Timestamp) -> String {
    format!(
        "Data · Catalog {} games · prices {} · tags {}",
        format::count(view.catalog_games as u32),
        format::when(view.prices_updated_at, now),
        format::when(view.tags_updated_at, now)
    )
}

fn more_games(games: usize) -> Option<String> {
    (games > FAMILY_HEADERS).then(|| format!("+{}", games - FAMILY_HEADERS))
}

fn games_count(games: usize) -> String {
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
    format!("Mostly {tags}, from {}.", games_count(taste.game_count))
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
    fn a_family_card_counts_the_games_beyond_its_headers() {
        assert_eq!(more_games(9).as_deref(), Some("+5"));
        assert_eq!(more_games(4), None);
        assert_eq!(games_count(1), "1 game");
        assert_eq!(games_count(9), "9 games");
    }

    #[test]
    fn the_held_out_games_show_only_when_there_are_some() {
        assert_eq!(held_out_text(0), None);
        assert_eq!(held_out_text(1).as_deref(), Some("1 held-out game"));
        assert_eq!(held_out_text(12).as_deref(), Some("12 held-out games"));
    }

    fn labels(would_try: usize, not_for_me: usize) -> VotesView {
        VotesView {
            would_try: 0,
            maybe: 0,
            not_for_me: 0,
            known: 0,
            labels: gamepalate_app::Labels {
                would_try,
                not_for_me,
            },
            min_labels: 10,
        }
    }

    #[test]
    fn the_opinion_says_how_many_votes_are_missing() {
        assert_eq!(
            opinion_progress(&labels(4, 7)),
            "Your votes start shaping the list at 10 Would try and 10 Not for me: 4 and 7 so far."
        );
    }

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

    #[test]
    fn without_a_taste_the_screen_says_how_to_get_one() {
        assert!(no_taste_text(2.0).starts_with("No taste yet: none of your games has 2 hours"));
    }
}
