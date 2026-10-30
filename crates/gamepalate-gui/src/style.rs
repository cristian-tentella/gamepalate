use iced::{
    Border, Color, Element, Theme,
    theme::palette,
    widget::{button, container, rule, text},
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

pub fn vote(theme: &Theme, status: button::Status) -> button::Style {
    let background = theme.extended_palette().background;
    let deviation = match status {
        button::Status::Hovered => 0.04,
        button::Status::Pressed => 0.07,
        button::Status::Active | button::Status::Disabled => 0.015,
    };

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

pub fn keycap(theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: theme.extended_palette().background.strong.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}
