use iced::{
    Element, Theme,
    widget::{container, rule},
};

pub fn bar(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.weakest.color.into()),
        text_color: Some(palette.background.weakest.text),
        ..container::Style::default()
    }
}

pub fn separator<'a, Message: 'a>() -> Element<'a, Message> {
    rule::horizontal(1).style(rule::weak).into()
}
