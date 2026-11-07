use iced::{
    Border, Color, Element, Shadow, Theme, Vector,
    overlay::menu,
    padding,
    theme::palette,
    widget::{button, container, pick_list, progress_bar, rule, text, text_input},
};

const BADGE_TINT: f32 = 0.14;
const KEYCAP_BORDER: f32 = 0.3;
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

pub fn vote(flashed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let background = palette.background;
        let deviation = match status {
            button::Status::Hovered => 0.04,
            button::Status::Pressed => 0.07,
            button::Status::Active | button::Status::Disabled => 0.015,
        };

        if flashed {
            return button::Style {
                background: Some(palette.primary.base.color.into()),
                text_color: palette.primary.base.text,
                border: Border {
                    color: palette.primary.base.color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..button::Style::default()
            };
        }

        button::Style {
            background: Some(palette::deviate(background.base.color, deviation).into()),
            text_color: background.base.text,
            border: Border {
                color: background.strong.color,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        }
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
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.2),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..container::Style::default()
    }
}

pub fn accent(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().success.base.color),
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
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.25),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 16.0,
        },
        ..container::Style::default()
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

pub fn dot(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let color = if current {
            palette.primary.base.color
        } else if matches!(status, button::Status::Hovered) {
            faded(theme, 0.4)
        } else {
            palette.background.strong.color
        };

        button::Style {
            background: Some(color.into()),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

pub fn step(done: bool, current: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();
        let (background, text, border) = if done {
            (
                Some(palette.success.base.color),
                palette.success.base.text,
                palette.success.base.color,
            )
        } else if current {
            (
                Some(palette.primary.base.color),
                palette.primary.base.text,
                palette.primary.base.color,
            )
        } else {
            (None, faded(theme, 0.5), palette.background.strong.color)
        };

        container::Style {
            background: background.map(Into::into),
            text_color: Some(text),
            border: Border {
                color: border,
                width: 1.5,
                radius: 14.0.into(),
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
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.2),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
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

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    rounded(button::primary(theme, status))
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
    fn faded_text_keeps_its_alpha_in_the_light_and_fades_more_in_the_dark() {
        let light = Theme::Light.extended_palette().background.base.text;
        let dark = Theme::Dark.extended_palette().background.base.text;

        assert_eq!(faded(&Theme::Light, 0.7), light.scale_alpha(0.7));
        assert_eq!(faded(&Theme::Dark, 0.7), dark.scale_alpha(0.7 * 0.7));
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
