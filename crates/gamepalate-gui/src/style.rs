use std::sync::atomic::{AtomicBool, Ordering};

use iced::{
    Border, Color, Element, Shadow, Theme, Vector, border, gradient,
    overlay::menu,
    padding,
    theme::palette,
    widget::{button, container, pick_list, progress_bar, rule, slider, text, text_input, toggler},
};

/// Set once tiny-skia draws the window. iced_tiny_skia 0.14.1 paints the shadow of a quad without
/// the clip of a partial redraw (`Engine::draw_quad` passes no mask to `draw_pixmap`), over the
/// quad too: every redraw of a playing video or a hovered button under a sheet darkened the
/// whole sheet a little more. With tiny-skia nothing has a shadow.
static NO_SHADOWS: AtomicBool = AtomicBool::new(false);

/// Called when tiny-skia is chosen: from `ICED_BACKEND` at the start, or by the first widget
/// that sees the renderer.
pub fn drop_shadows() {
    NO_SHADOWS.store(true, Ordering::Relaxed);
}

/// Whether `ICED_BACKEND` makes iced start with tiny-skia: the first backend it names. Without
/// it iced tries wgpu first, and only a widget can tell.
pub fn picks_tiny_skia(iced_backend: Option<&str>) -> bool {
    iced_backend
        .and_then(|backends| backends.split(',').find(|backend| !backend.is_empty()))
        .is_some_and(|backend| matches!(backend, "tiny-skia" | "tiny_skia"))
}

fn shadow(shadow: Shadow) -> Shadow {
    match NO_SHADOWS.load(Ordering::Relaxed) {
        true => Shadow::default(),
        false => shadow,
    }
}

const BADGE_TINT: f32 = 0.14;
const KEYCAP_BORDER: f32 = 0.3;
const MUTED: f32 = 0.55;
const LIGHT_GREEN: Color = Color::from_rgb8(0x2e, 0x7d, 0x32);
const LIGHT_YELLOW: Color = Color::from_rgb8(0x94, 0x60, 0x00);
const DARK_GREEN: Color = Color::from_rgb8(0x7c, 0xc9, 0x8b);
const DARK_YELLOW: Color = Color::from_rgb8(0xf0, 0xc2, 0x4f);
const LIGHT_RED: Color = Color::from_rgb8(0xc6, 0x28, 0x28);
const DARK_RED: Color = Color::from_rgb8(0xef, 0x9a, 0x9a);
const DIVIDER_HEIGHT: f32 = 20.0;
pub const DIVIDER_INSET: f32 = 8.0;

pub fn top_bar(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background.base;

    container::Style {
        background: Some(palette::deviate(background.color, 0.015).into()),
        text_color: Some(background.text),
        ..container::Style::default()
    }
}

pub fn separator<'a, Message: 'a>() -> Element<'a, Message> {
    rule::horizontal(1).style(rule::weak).into()
}

pub fn divider<'a, Message: 'a>() -> Element<'a, Message> {
    container(rule::vertical(1).style(rule::weak))
        .height(DIVIDER_HEIGHT)
        .padding(padding::left(DIVIDER_INSET).right(DIVIDER_INSET))
        .into()
}

fn faded(theme: &Theme, alpha: f32) -> Color {
    let palette = theme.extended_palette();
    let alpha = if palette.is_dark {
        alpha * alpha
    } else {
        alpha
    };

    palette.background.base.text.scale_alpha(alpha)
}

pub fn tab(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let highlighted = current || matches!(status, button::Status::Hovered);

        button::Style {
            text_color: if highlighted {
                theme.extended_palette().background.base.text
            } else {
                faded(theme, 0.6)
            },
            ..button::Style::default()
        }
    }
}

pub fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(faded(theme, 0.7)),
    }
}

pub fn tab_underline(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().primary.base.color.into()),
        ..container::Style::default()
    }
}

pub fn link(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let color = if palette.is_dark {
        palette::mix(palette.primary.base.color, Color::WHITE, 0.4)
    } else {
        palette.primary.base.color
    };

    button::Style {
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => palette::deviate(color, 0.1),
            button::Status::Active | button::Status::Disabled => color,
        },
        ..button::Style::default()
    }
}

// An action in the accent color that shows its edges only under the pointer.
pub fn quiet_action(theme: &Theme, status: button::Status) -> button::Style {
    let link = link(theme, status);
    let tint = |alpha: f32| Some(link.text_color.scale_alpha(alpha).into());

    button::Style {
        background: match status {
            button::Status::Hovered => tint(0.12),
            button::Status::Pressed => tint(0.2),
            button::Status::Active | button::Status::Disabled => None,
        },
        text_color: match status {
            button::Status::Disabled => link.text_color.scale_alpha(0.5),
            _ => link.text_color,
        },
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..link
    }
}

pub fn key_action(theme: &Theme, status: button::Status) -> button::Style {
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => {
            theme.extended_palette().background.base.text
        }
        button::Status::Active | button::Status::Disabled => faded(theme, 0.7),
    };

    button::Style {
        text_color,
        ..button::Style::default()
    }
}

pub fn muted_keycap(theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(faded(theme, MUTED)),
        border: Border {
            color: theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(KEYCAP_BORDER * MUTED),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn window_button(theme: &Theme, status: button::Status) -> button::Style {
    let background = theme.extended_palette().background;

    button::Style {
        background: match status {
            button::Status::Hovered => Some(background.weak.color.into()),
            button::Status::Pressed => Some(background.strong.color.into()),
            button::Status::Active | button::Status::Disabled => None,
        },
        text_color: background.base.text,
        ..button::Style::default()
    }
}

pub fn open_bar_icon(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let style = window_button(theme, status);

    button::Style {
        background: style
            .background
            .or_else(|| Some(palette.background.weak.color.into())),
        text_color: palette.primary.base.color,
        ..style
    }
}

/// The round close button of a sheet: only the cross until the pointer is on it.
pub fn sheet_close(theme: &Theme, status: button::Status) -> button::Style {
    let background = theme.extended_palette().background;

    button::Style {
        background: match status {
            button::Status::Hovered => Some(background.weak.color.into()),
            button::Status::Pressed => Some(background.strong.color.into()),
            button::Status::Active | button::Status::Disabled => None,
        },
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => background.base.text,
            button::Status::Active | button::Status::Disabled => {
                muted(theme).color.unwrap_or(background.base.text)
            }
        },
        border: Border {
            radius: f32::MAX.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

pub fn close_button(theme: &Theme, status: button::Status) -> button::Style {
    let red = Color::from_rgb8(0xc4, 0x2b, 0x1c);

    match status {
        button::Status::Hovered => button::Style {
            background: Some(red.into()),
            text_color: Color::WHITE,
            ..button::Style::default()
        },
        button::Status::Pressed => button::Style {
            background: Some(red.scale_alpha(0.8).into()),
            text_color: Color::WHITE,
            ..button::Style::default()
        },
        button::Status::Active | button::Status::Disabled => window_button(theme, status),
    }
}

pub const VOTE_RADIUS: f32 = 6.0;

/// Where a vote button sits: alone, or as a segment of the group of verdicts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Alone,
    First,
    Middle,
    Last,
    // The four corners of a grid of segments.
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Segment {
    // A segment sits inside the group's border, so its outer corners follow the inner edge.
    fn radius(self) -> border::Radius {
        let inner = VOTE_RADIUS - 1.0;
        match self {
            Segment::Alone => VOTE_RADIUS.into(),
            Segment::First => border::Radius::default().left(inner),
            Segment::Middle => border::Radius::default(),
            Segment::Last => border::Radius::default().right(inner),
            Segment::TopLeft => border::Radius::default().top_left(inner),
            Segment::TopRight => border::Radius::default().top_right(inner),
            Segment::BottomLeft => border::Radius::default().bottom_left(inner),
            Segment::BottomRight => border::Radius::default().bottom_right(inner),
        }
    }
}

pub fn vote(flashed: bool, segment: Segment) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let background = palette.background;
        let deviation = match status {
            button::Status::Hovered => 0.04,
            button::Status::Pressed => 0.07,
            button::Status::Active | button::Status::Disabled => 0.015,
        };
        let width = if segment == Segment::Alone { 1.0 } else { 0.0 };

        if flashed {
            return button::Style {
                background: Some(palette.primary.base.color.into()),
                text_color: palette.primary.base.text,
                border: Border {
                    color: palette.primary.base.color,
                    width,
                    radius: segment.radius(),
                },
                ..button::Style::default()
            };
        }

        button::Style {
            background: Some(palette::deviate(background.base.color, deviation).into()),
            text_color: background.base.text,
            border: Border {
                color: background.strong.color,
                width,
                radius: segment.radius(),
            },
            ..button::Style::default()
        }
    }
}

/// The handle of a long list: a thin rail with a wide, short handle that is easy to grab.
pub fn scrubber(theme: &Theme, status: slider::Status) -> slider::Style {
    let palette = theme.extended_palette();
    let handle = match status {
        slider::Status::Active => palette.background.strong.color,
        slider::Status::Hovered => palette.primary.weak.color,
        slider::Status::Dragged => palette.primary.base.color,
    };

    slider::Style {
        rail: slider::Rail {
            backgrounds: (
                palette.background.weak.color.into(),
                palette.background.weak.color.into(),
            ),
            width: 4.0,
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Rectangle {
                width: 28,
                border_radius: 4.0.into(),
            },
            background: handle.into(),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// What a filter chip does to the list: nothing, shows only its games, or hides them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Off,
    Only,
    Hide,
}

/// A pill for a filter: an outline while it does nothing, tinted when it shows only its games
/// (the accent) or hides them (red).
pub fn filter_chip(state: Chip) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (background, text_color, border_color) = match state {
            Chip::Off => (
                hovered.then_some(palette.background.weak.color),
                palette.background.base.text,
                palette.background.strong.color,
            ),
            Chip::Only => (
                Some(palette.primary.weak.color),
                palette.primary.weak.text,
                palette.primary.base.color,
            ),
            Chip::Hide => (
                Some(palette.danger.weak.color),
                palette.danger.weak.text,
                palette.danger.base.color,
            ),
        };

        button::Style {
            background: background.map(Into::into),
            text_color,
            border: Border {
                color: border_color,
                width: 1.0,
                radius: 999.0.into(),
            },
            ..button::Style::default()
        }
    }
}

/// A segment of a choice, like the ones at the top of Votes: the chosen one is filled.
pub fn segment(
    current: bool,
    segment: Segment,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let (background, text_color) = if current {
            (palette.primary.base.color, palette.primary.base.text)
        } else {
            let deviation = match status {
                button::Status::Hovered => 0.04,
                button::Status::Pressed => 0.07,
                button::Status::Active | button::Status::Disabled => 0.015,
            };
            (
                palette::deviate(palette.background.base.color, deviation),
                palette.background.base.text,
            )
        };

        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border {
                radius: segment.radius(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

/// The border around the segments of the verdicts.
pub fn vote_group(theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: theme.extended_palette().background.strong.color,
            width: 1.0,
            radius: VOTE_RADIUS.into(),
        },
        ..container::Style::default()
    }
}

/// The line between two segments, as strong as the group's border.
pub fn vote_seam(theme: &Theme) -> rule::Style {
    rule::Style {
        color: theme.extended_palette().background.strong.color,
        ..rule::default(theme)
    }
}

pub fn keycap(flashed: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();

        container::Style {
            text_color: Some(if flashed {
                palette.primary.base.text
            } else {
                palette.background.base.text
            }),
            border: Border {
                color: if flashed {
                    palette.primary.base.text.scale_alpha(0.6)
                } else {
                    palette.background.base.text.scale_alpha(KEYCAP_BORDER)
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }
    }
}

pub fn toast(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background.base;

    container::Style {
        background: Some(faded(theme, 0.9).into()),
        text_color: Some(background.color),
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        shadow: shadow(Shadow {
            color: Color::BLACK.scale_alpha(0.2),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        }),
        ..container::Style::default()
    }
}

pub fn accent(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().success.base.color),
    }
}

fn green(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        DARK_GREEN
    } else {
        LIGHT_GREEN
    }
}

fn red(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        DARK_RED
    } else {
        LIGHT_RED
    }
}

/// Green for a game that runs well on Steam Deck: the theme has no green of its own.
pub fn verified(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(green(theme)),
    }
}

/// The Steam Deck chip of Features: green when the game `runs`, red when it does not, a plain
/// chip when nobody knows.
pub fn deck_chip(runs: Option<bool>) -> impl Fn(&Theme) -> container::Style {
    move |theme| match runs {
        Some(runs) => {
            let color = if runs { green(theme) } else { red(theme) };

            container::Style {
                text_color: Some(color),
                background: Some(color.scale_alpha(BADGE_TINT).into()),
                border: Border {
                    color,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..container::Style::default()
            }
        }
        None => tag(theme),
    }
}

pub fn playable(theme: &Theme) -> text::Style {
    let color = if theme.extended_palette().is_dark {
        DARK_YELLOW
    } else {
        LIGHT_YELLOW
    };

    text::Style { color: Some(color) }
}

pub fn primary_badge(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().primary.base.color.into()),
        ..badge(theme)
    }
}

/// A line on the weak background, such as the one over Browse while the catalog downloads.
pub fn weak_band(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        ..container::Style::default()
    }
}

pub fn badge(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().success.base.color.into()),
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn tag(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background;

    container::Style {
        text_color: Some(background.base.text),
        border: Border {
            color: background.strong.color,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn discount_badge(theme: &Theme) -> container::Style {
    let accent = theme.extended_palette().success.base.color;

    container::Style {
        background: Some(accent.scale_alpha(BADGE_TINT).into()),
        text_color: Some(accent),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn bet(theme: &Theme) -> container::Style {
    let warning = theme.extended_palette().warning;

    container::Style {
        background: Some(warning.weak.color.into()),
        text_color: Some(warning.weak.text),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn chip(theme: &Theme, status: button::Status) -> button::Style {
    let background = theme.extended_palette().background;

    button::Style {
        background: Some(
            match status {
                button::Status::Hovered | button::Status::Pressed => background.strong.color,
                button::Status::Active | button::Status::Disabled => background.weak.color,
            }
            .into(),
        ),
        text_color: background.base.text,
        border: Border {
            radius: 10.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

pub fn choice(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let primary = theme.extended_palette().primary;

        if current {
            button::Style {
                background: Some(primary.base.color.into()),
                text_color: primary.base.text,
                border: Border {
                    radius: 10.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        } else {
            chip(theme, status)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Primary,
    Success,
    Warning,
    Danger,
}

pub fn tinted(tone: Tone, current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let (base, weak) = match tone {
            Tone::Primary => (palette.primary.base, palette.primary.weak),
            Tone::Success => (palette.success.base, palette.success.weak),
            Tone::Warning => (palette.warning.base, palette.warning.weak),
            Tone::Danger => (palette.danger.base, palette.danger.weak),
        };
        let (background, text_color) = if current {
            (base.color, base.text)
        } else {
            match status {
                button::Status::Hovered | button::Status::Pressed => (weak.color, weak.text),
                button::Status::Active => {
                    (palette.background.weak.color, palette.background.base.text)
                }
                button::Status::Disabled => (palette.background.weak.color, faded(theme, 0.5)),
            }
        };

        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

pub fn list_row(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let background = theme.extended_palette().background;
        let color = if current {
            Some(background.strong.color)
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(background.weak.color)
        } else {
            None
        };

        button::Style {
            background: color.map(Into::into),
            text_color: background.base.text,
            ..button::Style::default()
        }
    }
}

/// A grey block where a game's page will be, while it loads.
pub fn skeleton(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn veil(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(
            theme
                .extended_palette()
                .background
                .base
                .color
                .scale_alpha(0.7)
                .into(),
        ),
        ..container::Style::default()
    }
}

pub fn arrow(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let alpha = match status {
        button::Status::Hovered | button::Status::Pressed => 1.0,
        button::Status::Active | button::Status::Disabled => 0.8,
    };

    button::Style {
        background: Some(palette.background.base.color.scale_alpha(alpha).into()),
        text_color: palette.background.base.text,
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// The control bar of a trailer: a pill like the gallery arrows, over the video.
pub fn video_bar(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background.base;

    container::Style {
        background: Some(background.color.scale_alpha(0.85).into()),
        text_color: Some(background.text),
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The position of a trailer: the part already seen has the accent.
pub fn video_position(theme: &Theme, status: slider::Status) -> slider::Style {
    let palette = theme.extended_palette();
    let handle = match status {
        slider::Status::Active => palette.primary.base.color,
        slider::Status::Hovered | slider::Status::Dragged => palette.primary.strong.color,
    };

    slider::Style {
        rail: slider::Rail {
            backgrounds: (
                palette.primary.base.color.into(),
                palette.background.strong.color.into(),
            ),
            width: 4.0,
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 6.0 },
            background: handle.into(),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

pub fn popup(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background;

    container::Style {
        background: Some(background.base.color.into()),
        text_color: Some(background.base.text),
        border: Border {
            color: background.strong.color,
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: shadow(Shadow {
            color: Color::BLACK.scale_alpha(0.25),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 16.0,
        }),
        ..container::Style::default()
    }
}

/// One review in the reviews sheet: a little apart from the sheet, with a soft edge.
pub fn review_card(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background;

    container::Style {
        background: Some(palette::deviate(background.base.color, 0.03).into()),
        border: Border {
            color: background.weak.color,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

/// A quote inside a review.
pub fn review_quote(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background;

    container::Style {
        background: Some(palette::deviate(background.base.color, 0.07).into()),
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The verdict of a review: in the accent when recommended, grey when not. The theme's danger
/// is too close to the accent to tell them apart.
pub fn review_verdict(recommended: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let (background, text_color) = if recommended {
            let accent = palette.success.base.color;
            (accent.scale_alpha(0.16), accent)
        } else {
            (
                palette.background.strong.color,
                palette.background.base.text,
            )
        };

        container::Style {
            background: Some(background.into()),
            text_color: Some(text_color),
            border: Border {
                radius: 999.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        }
    }
}

pub fn card(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background;

    container::Style {
        background: Some(palette::deviate(background.base.color, 0.015).into()),
        border: Border {
            color: background.strong.color,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn drop_zone(hovering: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let (color, width) = if hovering {
            (palette.primary.base.color, 2.0)
        } else {
            (palette.background.strong.color, 1.0)
        };

        container::Style {
            border: Border {
                color,
                width,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        }
    }
}

pub fn zoom(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background.base;

    container::Style {
        background: Some(background.color.into()),
        text_color: Some(background.text),
        ..container::Style::default()
    }
}

pub fn picture(theme: &Theme) -> container::Style {
    let background = theme.extended_palette().background;

    container::Style {
        background: Some(palette::deviate(background.base.color, 0.04).into()),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

fn dot_color(theme: &Theme, current: bool, status: button::Status) -> Color {
    let palette = theme.extended_palette();
    if current {
        palette.primary.base.color
    } else if matches!(status, button::Status::Hovered) {
        faded(theme, 0.4)
    } else {
        palette.background.strong.color
    }
}

pub fn dot(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| button::Style {
        background: Some(dot_color(theme, current, status).into()),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// The dot of a trailer: a small triangle drawn by the play glyph, in the colors of the dots.
pub fn trailer_dot(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| button::Style {
        text_color: dot_color(theme, current, status),
        ..button::Style::default()
    }
}

/// The shade at the bottom of a trailer's poster, the same in both themes: it lies on the picture.
pub fn poster_shade(_theme: &Theme) -> container::Style {
    let shade = gradient::Linear::new(0.0)
        .add_stop(0.0, Color::BLACK.scale_alpha(0.6))
        .add_stop(0.45, Color::TRANSPARENT);

    container::Style {
        background: Some(shade.into()),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn step(done: bool, current: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let accent = palette.primary.base.color;
        let (background, border, width) = if done {
            (Some(accent), accent, 0.0)
        } else if current {
            (None, accent, 2.0)
        } else {
            (None, palette.background.strong.color, 1.5)
        };

        container::Style {
            background: background.map(Into::into),
            border: Border {
                color: border,
                width,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        }
    }
}

pub fn step_line(reached: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let color = if reached {
            palette.primary.base.color
        } else {
            palette.background.strong.color
        };

        container::Style {
            background: Some(color.into()),
            ..container::Style::default()
        }
    }
}

pub fn field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let style = text_input::default(theme, status);

    text_input::Style {
        border: Border {
            radius: 6.0.into(),
            ..style.border
        },
        ..style
    }
}

/// A field with no box of its own, inside a panel that draws one.
pub fn bare_field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: Border::default(),
        ..text_input::default(theme, status)
    }
}

/// The veil over the page under the game search and a game's sheet: the blurred page shows
/// through it.
pub fn frosted_veil(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let alpha = if palette.is_dark { 0.55 } else { 0.45 };

    container::Style {
        background: Some(palette.background.base.color.scale_alpha(alpha).into()),
        ..container::Style::default()
    }
}

pub fn wrong_field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let style = field(theme, status);

    text_input::Style {
        border: Border {
            color: theme.extended_palette().danger.base.color,
            ..style.border
        },
        ..style
    }
}

pub fn dropdown(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let palette = theme.extended_palette();
    let border = match status {
        pick_list::Status::Active => palette.background.strong.color,
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => {
            palette.primary.strong.color
        }
    };

    pick_list::Style {
        text_color: palette.background.base.text,
        placeholder_color: palette.secondary.base.color,
        handle_color: faded(theme, 0.7),
        background: palette.background.base.color.into(),
        border: Border {
            color: border,
            width: 1.0,
            radius: 6.0.into(),
        },
    }
}

pub fn menu(theme: &Theme) -> menu::Style {
    let palette = theme.extended_palette();

    menu::Style {
        background: palette.background.base.color.into(),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 6.0.into(),
        },
        text_color: palette.background.base.text,
        selected_text_color: palette.background.base.text,
        selected_background: palette
            .primary
            .base
            .color
            .scale_alpha(if palette.is_dark { 0.3 } else { 0.12 })
            .into(),
        shadow: shadow(Shadow {
            color: Color::BLACK.scale_alpha(0.2),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        }),
    }
}

pub fn text_button(theme: &Theme, status: button::Status) -> button::Style {
    let background = theme.extended_palette().background;

    button::Style {
        background: match status {
            button::Status::Hovered => Some(background.weak.color.into()),
            button::Status::Pressed => Some(background.strong.color.into()),
            button::Status::Active | button::Status::Disabled => None,
        },
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..link(theme, status)
    }
}

/// A column heading of a table: the hover fills the whole cell, square.
pub fn column_heading(theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        border: Border::default(),
        ..text_button(theme, status)
    }
}

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    rounded(button::primary(theme, status))
}

pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    rounded(button::danger(theme, status))
}

pub fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    rounded(button::secondary(theme, status))
}

fn rounded(style: button::Style) -> button::Style {
    button::Style {
        border: Border {
            radius: 6.0.into(),
            ..style.border
        },
        ..style
    }
}

/// A switch that is tinted with the accent when it is on, like a chip of Browse.
pub fn switch(theme: &Theme, status: toggler::Status) -> toggler::Style {
    let palette = theme.extended_palette();
    let (on, hovered) = match status {
        toggler::Status::Active { is_toggled } | toggler::Status::Disabled { is_toggled } => {
            (is_toggled, false)
        }
        toggler::Status::Hovered { is_toggled } => (is_toggled, true),
    };
    let (background, border, knob) = if on {
        let track = if hovered {
            palette.primary.strong.color
        } else {
            palette.primary.base.color
        };
        (track, track, Color::WHITE)
    } else {
        (
            if hovered {
                palette.background.weak.color
            } else {
                palette.background.base.color
            },
            palette.background.strong.color,
            palette.background.strong.color,
        )
    };

    toggler::Style {
        background: background.into(),
        background_border_width: 1.0,
        background_border_color: border,
        foreground: knob.into(),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: None,
        border_radius: None,
        padding_ratio: 0.25,
    }
}

pub fn meter(theme: &Theme) -> progress_bar::Style {
    let palette = theme.extended_palette();

    progress_bar::Style {
        background: palette.background.weak.color.into(),
        bar: palette.primary.base.color.into(),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
    }
}

pub fn accent_meter(theme: &Theme) -> progress_bar::Style {
    let palette = theme.extended_palette();

    progress_bar::Style {
        background: palette.background.weak.color.into(),
        bar: palette.success.base.color.into(),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
    }
}

pub fn quiet_meter(theme: &Theme) -> progress_bar::Style {
    let background = theme.extended_palette().background;

    progress_bar::Style {
        background: background.weak.color.into(),
        bar: background.strong.color.into(),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
    }
}

pub fn weight_bar(theme: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: Color::WHITE.scale_alpha(0.15).into(),
        bar: theme.extended_palette().primary.base.color.into(),
        border: Border {
            radius: 1.5.into(),
            ..Border::default()
        },
    }
}

pub fn capsule_strip(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::BLACK.scale_alpha(0.7).into()),
        text_color: Some(Color::WHITE),
        border: Border {
            radius: iced::border::Radius::default().bottom(6.0),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn cover_count(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::BLACK.scale_alpha(0.55).into()),
        text_color: Some(Color::WHITE),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn capsule_badge(theme: &Theme) -> container::Style {
    let primary = theme.extended_palette().primary.base;

    container::Style {
        background: Some(primary.color.into()),
        text_color: Some(primary.text),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn toast_link(theme: &Theme, status: button::Status) -> button::Style {
    let background = theme.extended_palette().background.base;
    let color = match status {
        button::Status::Hovered | button::Status::Pressed => background.color,
        _ => background.color.scale_alpha(0.85),
    };

    button::Style {
        text_color: color,
        ..button::Style::default()
    }
}

pub fn capsule_selected(theme: &Theme) -> container::Style {
    let primary = theme.extended_palette().primary.base;

    container::Style {
        border: Border {
            color: primary.color,
            width: 3.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iced_starts_with_tiny_skia_when_it_is_the_first_backend_named() {
        assert!(picks_tiny_skia(Some("tiny-skia")));
        assert!(picks_tiny_skia(Some("tiny_skia")));
        assert!(picks_tiny_skia(Some(",tiny-skia,wgpu")));
        assert!(!picks_tiny_skia(Some("wgpu,tiny-skia")));
        assert!(!picks_tiny_skia(Some("")));
        assert!(!picks_tiny_skia(None));
    }

    #[test]
    fn faded_text_keeps_its_alpha_in_the_light_and_fades_more_in_the_dark() {
        let light = Theme::Light.extended_palette().background.base.text;
        let dark = Theme::Dark.extended_palette().background.base.text;

        assert_eq!(faded(&Theme::Light, 0.7), light.scale_alpha(0.7));
        assert_eq!(faded(&Theme::Dark, 0.7), dark.scale_alpha(0.7 * 0.7));
    }

    #[test]
    fn the_deck_colours_read_on_both_backgrounds() {
        for theme in [Theme::Light, Theme::Dark] {
            let background = theme.palette().background;

            for style in [verified(&theme), playable(&theme)] {
                assert!(style.color.unwrap().relative_contrast(background) >= 4.5);
            }
            assert!(red(&theme).relative_contrast(background) >= 4.5);
        }
    }

    #[test]
    fn a_tinted_choice_is_coloured_only_when_current_or_under_the_mouse() {
        let palette = Theme::Light.extended_palette();
        let style =
            |current, status| tinted(Tone::Success, current)(&Theme::Light, status).background;

        assert_eq!(
            style(true, button::Status::Active),
            Some(palette.success.base.color.into())
        );
        assert_eq!(
            style(false, button::Status::Active),
            Some(palette.background.weak.color.into())
        );
        assert_eq!(
            style(false, button::Status::Hovered),
            Some(palette.success.weak.color.into())
        );
    }
}
