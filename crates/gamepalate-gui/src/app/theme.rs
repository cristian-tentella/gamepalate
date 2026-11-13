use gamepalate_app::ThemeChoice;
use iced::{Color, Element, Task, Theme, theme::Palette};

use super::{App, Message, bar_icon};

const SUN: char = '\u{E518}';
const MOON: char = '\u{E51C}';
const LIGHT_BACKGROUND: Color = Color::from_rgb8(0xff, 0xff, 0xff);
const LIGHT_TEXT: Color = Color::from_rgb8(0x02, 0x31, 0x3d);
const LIGHT_PRIMARY: Color = Color::from_rgb8(0x04, 0x77, 0x91);
const LIGHT_ACCENT: Color = Color::from_rgb8(0x91, 0x33, 0x78);
const LIGHT_DANGER: Color = Color::from_rgb8(0xa1, 0x59, 0x5a);
const DARK_BACKGROUND: Color = Color::from_rgb8(0x08, 0x0f, 0x11);
const DARK_TEXT: Color = Color::from_rgb8(0xda, 0xeb, 0xf1);
const DARK_PRIMARY: Color = Color::from_rgb8(0x8d, 0xd6, 0xed);
const DARK_ACCENT: Color = Color::from_rgb8(0xd8, 0x9b, 0xc3);
const DARK_DANGER: Color = Color::from_rgb8(0xff, 0xba, 0xb9);

pub(super) fn light() -> Theme {
    Theme::custom(
        "Light",
        Palette {
            background: LIGHT_BACKGROUND,
            text: LIGHT_TEXT,
            primary: LIGHT_PRIMARY,
            success: LIGHT_ACCENT,
            warning: LIGHT_ACCENT,
            danger: LIGHT_DANGER,
        },
    )
}

fn dark() -> Theme {
    Theme::custom(
        "Dark",
        Palette {
            background: DARK_BACKGROUND,
            text: DARK_TEXT,
            primary: DARK_PRIMARY,
            success: DARK_ACCENT,
            warning: DARK_ACCENT,
            danger: DARK_DANGER,
        },
    )
}

fn is_dark(theme: &Theme) -> bool {
    theme.extended_palette().is_dark
}

pub(super) fn toggled(theme: &Theme) -> Theme {
    if is_dark(theme) { light() } else { dark() }
}

pub(super) fn theme(choice: ThemeChoice) -> Theme {
    match choice {
        ThemeChoice::Light => light(),
        ThemeChoice::Dark => dark(),
    }
}

pub(super) fn choice(theme: &Theme) -> ThemeChoice {
    if is_dark(theme) {
        ThemeChoice::Dark
    } else {
        ThemeChoice::Light
    }
}

fn theme_icon(theme: &Theme) -> char {
    if is_dark(theme) { SUN } else { MOON }
}

pub(super) fn theme_button<'a>(theme: &Theme) -> Element<'a, Message> {
    bar_icon(theme_icon(theme), Some(Message::ToggleTheme)).into()
}

impl App {
    pub(super) fn set_theme(&mut self, choice: ThemeChoice) -> Task<Message> {
        self.theme = theme(choice);
        self.change_gui_settings(move |settings| settings.theme = choice)
    }
}

#[cfg(test)]
mod tests {
    use iced::widget::button;

    use super::*;
    use crate::style;

    #[test]
    fn the_theme_goes_from_light_to_dark_and_back() {
        assert_eq!(toggled(&light()), dark());
        assert_eq!(toggled(&dark()), light());
    }

    #[test]
    fn the_dark_theme_has_a_darker_background_than_the_default() {
        let background = dark().palette().background;

        assert_eq!(background, DARK_BACKGROUND);
        assert!(background.relative_luminance() < Palette::DARK.background.relative_luminance());
    }

    #[test]
    fn links_and_discounts_stand_out_in_the_dark_as_in_the_light() {
        let contrast = |theme: &Theme| {
            let background = theme.palette().background;
            let link = style::link(theme, button::Status::Active).text_color;
            let discount = style::accent(theme).color.unwrap();

            (
                link.relative_contrast(background),
                discount.relative_contrast(background),
            )
        };
        let (light_link, light_discount) = contrast(&light());
        let (dark_link, dark_discount) = contrast(&dark());

        assert!(dark_link >= light_link);
        assert!(dark_discount >= light_discount);
    }

    #[test]
    fn the_saved_choice_is_the_theme() {
        for choice_saved in [ThemeChoice::Light, ThemeChoice::Dark] {
            assert_eq!(choice(&theme(choice_saved)), choice_saved);
        }
    }

    #[test]
    fn the_button_shows_the_theme_it_switches_to() {
        assert_eq!(theme_icon(&light()), MOON);
        assert_eq!(theme_icon(&dark()), SUN);
    }
}
