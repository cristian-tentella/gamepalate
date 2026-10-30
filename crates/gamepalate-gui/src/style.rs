use iced::{
    Border, Color, Element, Shadow, Theme, Vector,
    theme::palette,
    widget::{button, container, progress_bar, rule, text},
};

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

pub fn tab(current: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let text = theme.extended_palette().background.base.text;
        let highlighted = current || matches!(status, button::Status::Hovered);

        button::Style {
            text_color: if highlighted {
                text
            } else {
                text.scale_alpha(0.6)
            },
            ..button::Style::default()
        }
    }
}

pub fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.6),
        ),
    }
}

pub fn tab_underline(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().primary.base.color.into()),
        ..container::Style::default()
    }
}

pub fn link(theme: &Theme, status: button::Status) -> button::Style {
    let primary = theme.extended_palette().primary;

    button::Style {
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => primary.strong.color,
            button::Status::Active | button::Status::Disabled => primary.base.color,
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
            text_color: flashed.then_some(palette.primary.base.text),
            border: Border {
                color: if flashed {
                    palette.primary.base.text.scale_alpha(0.6)
                } else {
                    palette.background.strong.color
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
        background: Some(background.text.scale_alpha(0.9).into()),
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

pub fn discount(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().success.base.color),
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
            palette.background.base.text.scale_alpha(0.4)
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
