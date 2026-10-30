use iced::{
    Element, Theme,
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
