mod app;
mod font;
mod screen;

use std::sync::Arc;

use gamepalate_app::Gamepalate;
use iced::{Pixels, Settings};

use crate::app::App;

fn main() -> anyhow::Result<()> {
    let gamepalate = Arc::new(Gamepalate::open(None)?);

    iced::application(
        move || App::new(Arc::clone(&gamepalate)),
        App::update,
        App::view,
    )
    .theme(App::theme)
    .settings(Settings {
        fonts: vec![font::REGULAR_BYTES.into(), font::BOLD_BYTES.into()],
        default_font: font::REGULAR,
        default_text_size: Pixels(14.0),
        ..Settings::default()
    })
    .run()?;

    Ok(())
}
