use std::ops::Range;

use gamepalate_app::{GameCorrection, HoursSource, LibraryGameView};
use iced::{
    Center, ContentFit, Element, Fill, Point, mouse,
    widget::{
        column, container, image, mouse_area, progress_bar, row, scrollable, space, stack, text,
        tooltip,
    },
};

use super::{GRID, Library, Message, Move, hours, hours_text, multiplier};
use crate::{font, screen::showcase::images::Picture, style};

pub(super) const TILE_WIDTH: f32 = 150.0;
const TILE_HEIGHT: f32 = 225.0;
const GAP: f32 = 12.0;
pub(super) const GRID_PADDING: f32 = 24.0;
const DIVIDER_HEIGHT: f32 = 56.0;
const DIMMED: f32 = 0.45;
const SCREENS_AHEAD: f32 = 2.0;
const IMAGE_RADIUS: f32 = 6.0;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Row {
    Games(Range<usize>),
    Divider,
}

pub(super) fn columns(width: f32) -> usize {
    let inner = width - 2.0 * GRID_PADDING;
    (((inner + GAP) / (TILE_WIDTH + GAP)).floor() as usize).max(1)
}

fn grid_width(columns: usize) -> f32 {
    columns as f32 * (TILE_WIDTH + GAP) - GAP
}

pub(super) fn rows(counting: usize, total: usize, columns: usize, divided: bool) -> Vec<Row> {
    let chunks = |range: Range<usize>| {
        range
            .clone()
            .step_by(columns)
            .map(move |start| Row::Games(start..(start + columns).min(range.end)))
    };

    if divided && counting < total {
        chunks(0..counting)
            .chain([Row::Divider])
            .chain(chunks(counting..total))
            .collect()
    } else {
        chunks(0..total).collect()
    }
}

fn row_height(row: &Row) -> f32 {
    match row {
        Row::Games(_) => TILE_HEIGHT + GAP,
        Row::Divider => DIVIDER_HEIGHT,
    }
}

pub(super) fn wanted_rows(rows: &[Row], offset: f32, height: f32, upward: bool) -> Vec<usize> {
    let visible = visible_rows(rows, offset, height);
    let ahead = height * SCREENS_AHEAD;

    if upward {
        visible
            .chain(visible_rows(rows, offset - ahead, ahead).rev())
            .collect()
    } else {
        visible
            .chain(visible_rows(rows, offset + height, ahead))
            .collect()
    }
}

pub(super) fn visible_rows(rows: &[Row], offset: f32, height: f32) -> Range<usize> {
    let top = offset - GRID_PADDING;
    let bottom = top + height;
    let mut start = rows.len();
    let mut end = rows.len();
    let mut y = 0.0;

    for (index, row) in rows.iter().enumerate() {
        let next = y + row_height(row);
        if start == rows.len() && next > top {
            start = index;
        }
        if y >= bottom {
            end = index;
            break;
        }
        y = next;
    }

    start.min(end)..end
}

pub(super) fn reveal_offset(
    rows: &[Row],
    position: usize,
    offset: f32,
    height: f32,
) -> Option<f32> {
    let mut top = GRID_PADDING;
    for row in rows {
        if let Row::Games(games) = row
            && games.contains(&position)
        {
            let bottom = top + TILE_HEIGHT;
            return if top < offset {
                Some(top - GRID_PADDING)
            } else if bottom > offset + height {
                Some((bottom + GRID_PADDING - height).max(0.0))
            } else {
                None
            };
        }
        top += row_height(row);
    }
    None
}

pub(super) fn tile_origin(rows: &[Row], position: usize, width: f32, offset: f32) -> Option<Point> {
    let free = width - 2.0 * GRID_PADDING - grid_width(columns(width));
    let left = GRID_PADDING + (free / 2.0).max(0.0);
    let mut top = GRID_PADDING - offset;
    for row in rows {
        if let Row::Games(games) = row
            && games.contains(&position)
        {
            let column = (position - games.start) as f32;
            return Some(Point::new(left + column * (TILE_WIDTH + GAP), top));
        }
        top += row_height(row);
    }
    None
}

pub(super) fn moved(rows: &[Row], position: Option<usize>, direction: Move) -> Option<usize> {
    let games: Vec<&Range<usize>> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Games(games) => Some(games),
            Row::Divider => None,
        })
        .collect();
    let last = games.last()?.end.checked_sub(1)?;
    let Some(position) = position else {
        return Some(0);
    };
    let line = games.iter().position(|games| games.contains(&position))?;
    let column = position - games[line].start;
    let at = |line: usize| {
        let games = games[line];
        games.start + column.min(games.len() - 1)
    };

    Some(match direction {
        Move::Left => position.saturating_sub(1),
        Move::Right => (position + 1).min(last),
        Move::Up if line > 0 => at(line - 1),
        Move::Down if line + 1 < games.len() => at(line + 1),
        Move::Up | Move::Down => position,
    })
}

pub(super) fn tile_label(game: &LibraryGameView) -> String {
    match game.source {
        HoursSource::Played => hours(game.owned_hours.unwrap_or(game.hours)),
        HoursSource::Wishlist => "wishlist".to_string(),
        HoursSource::Followed => "followed".to_string(),
        HoursSource::WouldTry => "would try".to_string(),
        HoursSource::Elsewhere => "elsewhere".to_string(),
    }
}

fn weight_fraction(share: f64, heaviest: f64) -> Option<f32> {
    (share > 0.0 && heaviest > 0.0).then(|| (share / heaviest) as f32)
}

fn correction_badge(correction: GameCorrection) -> Option<String> {
    let parts: Vec<String> = correction
        .multiplier
        .map(multiplier)
        .into_iter()
        .chain(correction.hidden.then(|| "hidden".to_string()))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn divider_note(min_hours: f64) -> String {
    format!(
        "Under {} of play, set to ×0, or without tags",
        hours_text(min_hours)
    )
}

impl Library {
    pub(super) fn grid(&self, width: f32) -> Element<'_, Message> {
        let rows = self.rows(width);
        let shown = visible_rows(&rows, self.offset, self.area.height.max(1.0));
        let above: f32 = rows[..shown.start].iter().map(row_height).sum();
        let below: f32 = rows[shown.end..].iter().map(row_height).sum();

        let content = column![space().height(above)]
            .extend(rows[shown.clone()].iter().map(|line| self.line(line)))
            .push(space().height(below))
            .width(grid_width(columns(width)));

        scrollable(
            container(content)
                .padding([GRID_PADDING, GRID_PADDING])
                .center_x(Fill),
        )
        .id(GRID)
        .on_scroll(Message::Scrolled)
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn line<'a>(&'a self, line: &Row) -> Element<'a, Message> {
        match line {
            Row::Games(games) => row(games
                .clone()
                .map(|position| self.tile(&self.view.games[self.shown[position]])))
            .spacing(GAP)
            .height(TILE_HEIGHT + GAP)
            .into(),
            Row::Divider => container(
                column![
                    text("Doesn't count")
                        .size(font::BAR_SIZE)
                        .font(font::SEMIBOLD),
                    text(divider_note(self.view.min_hours))
                        .size(font::NOTE_SIZE)
                        .style(style::muted),
                ]
                .spacing(2),
            )
            .height(DIVIDER_HEIGHT)
            .align_y(Center)
            .into(),
        }
    }

    fn tile<'a>(&'a self, game: &'a LibraryGameView) -> Element<'a, Message> {
        let picture: Element<'a, Message> = match self.capsules.get(&game.id) {
            Some(Picture::Ready(allocation)) => image(allocation.handle().clone())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .content_fit(ContentFit::Cover)
                .border_radius(IMAGE_RADIUS)
                .opacity(if game.counts { 1.0 } else { DIMMED })
                .into(),
            Some(Picture::Failed) => container(
                text(&game.name)
                    .size(font::BAR_SIZE)
                    .center()
                    .style(if game.counts {
                        text::default
                    } else {
                        style::muted
                    }),
            )
            .padding(12)
            .center_x(TILE_WIDTH)
            .center_y(TILE_HEIGHT)
            .style(style::picture)
            .into(),
            Some(Picture::Loading) | None => container(space())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .style(style::picture)
                .into(),
        };
        let badge = correction_badge(game.correction).map(|label| {
            container(text(label).size(font::NOTE_SIZE))
                .padding([1, 6])
                .style(style::capsule_badge)
        });
        let bar = weight_fraction(game.share, self.heaviest).map(|fraction| {
            progress_bar(0.0..=1.0, fraction)
                .girth(3)
                .length(Fill)
                .style(style::weight_bar)
        });
        let strip = container(
            column![text(tile_label(game)).size(font::NOTE_SIZE)]
                .push(bar)
                .spacing(3),
        )
        .padding([3, 8])
        .width(Fill)
        .style(style::capsule_strip);
        let overlay = column![
            row![space().width(Fill)].push(badge).padding(6),
            space().height(Fill),
            strip,
        ];
        let frame = (self.selected == Some(game.id)).then(|| {
            container(space())
                .width(TILE_WIDTH)
                .height(TILE_HEIGHT)
                .style(style::capsule_selected)
        });

        tooltip(
            mouse_area(
                stack![picture, overlay]
                    .push(frame)
                    .width(TILE_WIDTH)
                    .height(TILE_HEIGHT),
            )
            .interaction(mouse::Interaction::Pointer)
            .on_press(Message::Open(game.id)),
            container(text(&game.name).size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::super::heaviest;
    use super::*;

    #[test]
    fn the_columns_fit_the_width() {
        assert_eq!(columns(1280.0), 7);
        assert_eq!(columns(1400.0), 8);
        assert_eq!(columns(100.0), 1);
    }

    #[test]
    fn the_grid_is_as_wide_as_its_columns() {
        assert_eq!(grid_width(1), TILE_WIDTH);
        assert_eq!(grid_width(7), 7.0 * TILE_WIDTH + 6.0 * GAP);
        assert!(grid_width(columns(2560.0)) <= 2560.0 - 2.0 * GRID_PADDING);
    }

    #[test]
    fn the_rows_break_at_the_divider() {
        assert_eq!(
            rows(3, 7, 2, true),
            [
                Row::Games(0..2),
                Row::Games(2..3),
                Row::Divider,
                Row::Games(3..5),
                Row::Games(5..7),
            ]
        );
        assert_eq!(rows(3, 7, 4, false), [Row::Games(0..4), Row::Games(4..7)]);
        assert_eq!(rows(5, 5, 4, true), [Row::Games(0..4), Row::Games(4..5)]);
        assert_eq!(rows(0, 2, 4, true), [Row::Divider, Row::Games(0..2)]);
    }

    #[test]
    fn only_the_rows_in_view_are_drawn() {
        let rows = rows(20, 40, 4, true);
        let row = TILE_HEIGHT + GAP;

        assert_eq!(visible_rows(&rows, 0.0, row), 0..1);
        assert_eq!(visible_rows(&rows, 0.0, row + GRID_PADDING + 1.0), 0..2);
        assert_eq!(visible_rows(&rows, GRID_PADDING + row * 5.0, 1.0), 5..6);
        assert_eq!(
            visible_rows(&rows, GRID_PADDING + row * 5.0, DIVIDER_HEIGHT + 1.0),
            5..7
        );
        assert_eq!(visible_rows(&rows, 100_000.0, 500.0), 11..11);
        assert_eq!(visible_rows(&rows, -5_000.0, 1.0), 0..0);
    }

    #[test]
    fn loads_the_rows_in_view_then_two_screens_the_way_it_scrolls() {
        let rows = rows(20, 40, 4, true);
        let row = TILE_HEIGHT + GAP;

        assert_eq!(
            wanted_rows(&rows, GRID_PADDING + row * 2.0, row, false),
            [2, 3, 4]
        );
        assert_eq!(
            wanted_rows(&rows, GRID_PADDING + row * 4.0, row, true),
            [4, 3, 2]
        );
    }

    #[test]
    fn a_tile_shows_its_hours_or_where_it_comes_from() {
        let games = games();

        assert_eq!(tile_label(&games[1]), "120 h");
        assert_eq!(tile_label(&games[2]), "1.0 h");
        assert_eq!(tile_label(&games[3]), "wishlist");
        assert_eq!(tile_label(&games[4]), "followed");
        assert_eq!(tile_label(&games[5]), "0 h");
        let borrowed = LibraryGameView {
            recent: true,
            hours: 6.0,
            ..game(7, "Drova")
        };
        assert_eq!(tile_label(&borrowed), "6.0 h");
    }

    #[test]
    fn a_tile_names_the_signal_its_hours_come_from() {
        let followed = LibraryGameView {
            followed: true,
            hours: 3.0,
            source: HoursSource::Followed,
            ..owned(7, "Paradise Killer", 0.0, 0.01)
        };
        let would_try = LibraryGameView {
            hours: 4.0,
            source: HoursSource::WouldTry,
            ..game(8, "Tunic")
        };

        assert_eq!(tile_label(&followed), "followed");
        assert_eq!(tile_label(&would_try), "would try");
        assert_eq!(tile_label(&owned(9, "Elden Ring", 376.0, 0.3)), "376 h");
    }

    #[test]
    fn the_weight_bar_is_as_long_as_the_share_of_the_heaviest_game() {
        let games = games();
        let heaviest = heaviest(&games);

        assert_eq!(heaviest, 0.5);
        assert_eq!(weight_fraction(games[1].share, heaviest), Some(1.0));
        assert_eq!(weight_fraction(games[0].share, heaviest), Some(0.4));
        assert_eq!(weight_fraction(games[2].share, heaviest), None);
        assert_eq!(weight_fraction(0.1, 0.0), None);
    }

    #[test]
    fn the_tile_corner_follows_the_column_the_divider_and_the_scroll() {
        let width = 2.0 * GRID_PADDING + grid_width(4);
        let rows = rows(5, 9, 4, true);

        assert_eq!(
            tile_origin(&rows, 0, width, 0.0),
            Some(Point::new(GRID_PADDING, GRID_PADDING))
        );
        assert_eq!(
            tile_origin(&rows, 6, width, 100.0),
            Some(Point::new(
                GRID_PADDING + TILE_WIDTH + GAP,
                GRID_PADDING + 2.0 * (TILE_HEIGHT + GAP) + DIVIDER_HEIGHT - 100.0
            ))
        );
        assert_eq!(
            tile_origin(&rows, 0, width + 100.0, 0.0),
            Some(Point::new(GRID_PADDING + 50.0, GRID_PADDING))
        );
        assert_eq!(tile_origin(&rows, 99, width, 0.0), None);
    }

    #[test]
    fn the_open_game_is_scrolled_into_view() {
        let rows = rows(20, 40, 4, true);
        let row = TILE_HEIGHT + GAP;

        assert_eq!(reveal_offset(&rows, 0, 0.0, 600.0), None);
        assert_eq!(reveal_offset(&rows, 0, 100.0, 600.0), Some(0.0));
        let below = reveal_offset(&rows, 24, 0.0, 600.0).unwrap();
        assert_eq!(
            below,
            2.0 * GRID_PADDING + row * 6.0 + DIVIDER_HEIGHT + TILE_HEIGHT - 600.0
        );
        assert_eq!(reveal_offset(&rows, 99, 0.0, 600.0), None);
    }

    #[test]
    fn the_arrows_walk_the_grid_across_the_divider() {
        let rows = rows(5, 9, 4, true);
        let to = |position, direction| moved(&rows, position, direction);

        assert_eq!(to(None, Move::Down), Some(0));
        assert_eq!(to(Some(0), Move::Left), Some(0));
        assert_eq!(to(Some(3), Move::Right), Some(4));
        assert_eq!(to(Some(8), Move::Right), Some(8));
        assert_eq!(to(Some(2), Move::Down), Some(4));
        assert_eq!(to(Some(4), Move::Down), Some(5));
        assert_eq!(to(Some(7), Move::Up), Some(4));
        assert_eq!(to(Some(5), Move::Up), Some(4));
        assert_eq!(to(Some(1), Move::Up), Some(1));
        assert_eq!(to(Some(6), Move::Down), Some(6));
        assert_eq!(moved(&[], None, Move::Down), None);
    }

    #[test]
    fn the_badge_names_the_correction() {
        let badge = |multiplier, hidden| correction_badge(GameCorrection { multiplier, hidden });

        assert_eq!(badge(None, false), None);
        assert_eq!(badge(Some(0.5), false).as_deref(), Some("×0.5"));
        assert_eq!(badge(Some(0.0), false).as_deref(), Some("×0"));
        assert_eq!(badge(None, true).as_deref(), Some("hidden"));
        assert_eq!(badge(Some(0.25), true).as_deref(), Some("×0.25 · hidden"));
    }

    #[test]
    fn the_divider_says_why_a_game_does_not_count() {
        assert_eq!(
            divider_note(2.0),
            "Under 2 hours of play, set to ×0, or without tags"
        );
        assert_eq!(
            divider_note(1.0),
            "Under 1 hour of play, set to ×0, or without tags"
        );
        assert_eq!(
            divider_note(0.5),
            "Under 0.5 hours of play, set to ×0, or without tags"
        );
    }
}
