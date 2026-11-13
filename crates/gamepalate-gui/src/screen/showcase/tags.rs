use gamepalate_app::{GameTagsView, TagOfGame, UsedTag};
use iced::{
    Center, Element,
    widget::{button, column, container, progress_bar, row, text},
};

use super::{Message, Showcase, sheet_note};
use crate::{font, style};

const FIRST_TAGS: usize = 10;
const TAG_NAME_WIDTH: f32 = 200.0;
const TAG_BAR_WIDTH: f32 = 360.0;
const TAG_BAR_HEIGHT: f32 = 6.0;
const TAG_PERCENT_WIDTH: f32 = 64.0;
const TAG_SPACING: f32 = 10.0;
const TAGS_SECTION_SPACING: f32 = 28.0;
const USED_REASON: &str = "Tags like Indie or Singleplayer are on more than half of Steam games, so they don't tell games apart. Here each tag is weighed by how rare it is on Steam and how often it appears in the games you play. These weigh the most for this game.";
const ALL_TAGS_ORDER: &str = "Ordered by how many Steam players picked each one";

fn used_block<'a, M: 'a>(used: &'a [UsedTag]) -> Element<'a, M> {
    if used.is_empty() {
        return sheet_note("None of these tags are in your taste");
    }

    column![
        section_title("Used for your match"),
        text(USED_REASON).size(font::BAR_SIZE).style(style::muted),
        column(used.iter().map(|tag| {
            row![
                text(&tag.name)
                    .size(font::BAR_SIZE)
                    .font(font::SEMIBOLD)
                    .style(style::accent),
                text(used_line(tag))
                    .size(font::BAR_SIZE)
                    .style(style::muted),
            ]
            .spacing(8)
            .into()
        }))
        .spacing(4),
    ]
    .spacing(8)
    .into()
}

fn section_title<'a, M: 'a>(label: &'static str) -> Element<'a, M> {
    text(label)
        .size(font::INFO_SIZE)
        .font(font::SEMIBOLD)
        .into()
}

fn tag_line<'a, M: 'a>(tag: &'a TagOfGame, weighted: bool) -> Element<'a, M> {
    let counts = tag.used && !tag.banned;
    let name = text(&tag.name)
        .size(font::BAR_SIZE)
        .font(if counts {
            font::SEMIBOLD
        } else {
            font::REGULAR
        })
        .width(TAG_NAME_WIDTH);
    let name = if tag.banned {
        name.style(style::muted)
    } else {
        name
    };
    let bar = weighted.then(|| {
        progress_bar(0.0..=1.0, tag.relative as f32)
            .girth(TAG_BAR_HEIGHT)
            .length(TAG_BAR_WIDTH)
            .style(if tag.banned {
                style::quiet_meter
            } else if counts {
                style::accent_meter
            } else {
                style::meter
            })
    });
    let note = tag_note(tag, weighted).map(|note| {
        container(text(note).size(font::NOTE_SIZE).style(style::muted))
            .align_right(TAG_PERCENT_WIDTH)
    });

    row![name]
        .push(bar)
        .push(note)
        .spacing(12)
        .align_y(Center)
        .into()
}

fn tag_note(tag: &TagOfGame, weighted: bool) -> Option<String> {
    if tag.banned {
        Some("Banned".to_string())
    } else {
        weighted.then(|| format!("{:.0}%", tag.weight * 100.0))
    }
}

fn used_line(tag: &UsedTag) -> String {
    format!(
        "in {} of your games · {} of Steam games",
        tag.profile_games,
        catalog_share(tag.catalog_share)
    )
}

fn catalog_share(share: f64) -> String {
    if share < 0.01 {
        "<1%".to_string()
    } else {
        format!("{:.0}%", share * 100.0)
    }
}

fn shown_tags(count: usize, all: bool) -> usize {
    if all { count } else { count.min(FIRST_TAGS) }
}

fn show_all_label(count: usize, all: bool) -> Option<String> {
    (count > FIRST_TAGS).then(|| {
        if all {
            "Show fewer".to_string()
        } else {
            format!("Show all {count}")
        }
    })
}

pub(in crate::screen) fn tags_body<'a, M: Clone + 'a>(
    view: Option<&'a GameTagsView>,
    all: bool,
    toggle_all: M,
) -> Element<'a, M> {
    let Some(view) = view.filter(|view| !view.tags.is_empty()) else {
        return sheet_note("No tags for this game");
    };
    let shown = shown_tags(view.tags.len(), all);
    let list = column(
        view.tags[..shown]
            .iter()
            .map(|tag| tag_line(tag, view.weighted)),
    )
    .spacing(TAG_SPACING);
    let more = show_all_label(view.tags.len(), all).map(|label| {
        button(text(label).size(font::BAR_SIZE))
            .padding(0)
            .style(style::link)
            .on_press(toggle_all)
    });

    let all_tags = column![
        column![
            section_title("All tags"),
            text(ALL_TAGS_ORDER)
                .size(font::BAR_SIZE)
                .style(style::muted),
        ]
        .spacing(8),
        list,
    ]
    .push(more)
    .spacing(16);

    column![used_block(&view.used), all_tags]
        .spacing(TAGS_SECTION_SPACING)
        .into()
}

fn used_names(view: Option<&GameTagsView>) -> Vec<String> {
    view.map_or_else(Vec::new, |view| {
        view.used.iter().map(|tag| tag.name.clone()).collect()
    })
}

impl Showcase {
    pub fn used_tags(&self) -> Vec<String> {
        used_names(self.tags.as_ref())
    }

    pub(super) fn tags_body(&self) -> Element<'_, Message> {
        tags_body(
            self.tags.as_ref(),
            self.popups.all_tags,
            Message::ToggleAllTags,
        )
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::TagId;

    use super::*;

    #[test]
    fn shows_the_first_ten_tags_and_offers_the_rest() {
        assert_eq!(shown_tags(18, false), 10);
        assert_eq!(shown_tags(18, true), 18);
        assert_eq!(shown_tags(7, false), 7);
        assert_eq!(show_all_label(18, false).as_deref(), Some("Show all 18"));
        assert_eq!(show_all_label(18, true).as_deref(), Some("Show fewer"));
        assert_eq!(show_all_label(10, false), None);
        assert_eq!(show_all_label(3, true), None);
    }

    #[test]
    fn a_used_tag_says_how_many_games_have_it() {
        let used = |profile_games, catalog_share| UsedTag {
            id: TagId(1),
            name: "Roguelite".to_string(),
            profile_games,
            catalog_share,
        };

        assert_eq!(
            used_line(&used(14, 0.03)),
            "in 14 of your games · 3% of Steam games"
        );
        assert_eq!(
            used_line(&used(1, 0.004)),
            "in 1 of your games · <1% of Steam games"
        );
        assert_eq!(catalog_share(0.01), "1%");
        assert_eq!(catalog_share(0.0099), "<1%");
    }

    #[test]
    fn a_banned_tag_says_banned_instead_of_its_weight() {
        let tag = |banned| TagOfGame {
            id: TagId(1),
            name: "Horror".to_string(),
            weight: 0.08,
            relative: 1.0,
            used: false,
            banned,
        };

        assert_eq!(tag_note(&tag(false), true).as_deref(), Some("8%"));
        assert_eq!(tag_note(&tag(true), true).as_deref(), Some("Banned"));
        assert_eq!(tag_note(&tag(false), false), None);
        assert_eq!(tag_note(&tag(true), false).as_deref(), Some("Banned"));
    }

    #[test]
    fn the_facts_get_the_names_of_the_used_tags() {
        let view = GameTagsView {
            tags: Vec::new(),
            weighted: false,
            used: ["Roguelite", "Deckbuilder"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| UsedTag {
                    id: TagId(index as u32),
                    name: name.to_string(),
                    profile_games: 1,
                    catalog_share: 0.1,
                })
                .collect(),
        };

        assert_eq!(used_names(Some(&view)), ["Roguelite", "Deckbuilder"]);
        assert!(used_names(None).is_empty());
    }
}
