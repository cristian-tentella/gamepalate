mod app;
mod font;
mod format;
mod screen;
mod style;

use std::sync::Arc;

use gamepalate_app::Gamepalate;
use iced::{Pixels, Settings, Size, window};

use crate::app::App;

fn main() -> anyhow::Result<()> {
    let gamepalate = Arc::new(Gamepalate::open(None)?);

    iced::application(
        move || App::new(Arc::clone(&gamepalate)),
        App::update,
        App::view,
    )
    .theme(App::theme)
    .window(window::Settings {
        size: Size::new(1280.0, 800.0),
        min_size: Some(Size::new(960.0, 600.0)),
        ..window::Settings::default()
    })
    .settings(Settings {
        fonts: vec![font::REGULAR_BYTES.into(), font::BOLD_BYTES.into()],
        default_font: font::REGULAR,
        default_text_size: Pixels(14.0),
        ..Settings::default()
    })
    .run()?;

    Ok(())
}
