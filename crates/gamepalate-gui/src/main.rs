mod app;
mod font;
mod format;
mod progress;
mod screen;
mod smooth;
mod style;

use gamepalate_app::{log::start_log, setup::saved_gui_settings};
use iced::{Pixels, Settings};

use crate::app::{App, window_settings};

fn main() -> anyhow::Result<()> {
    let _ = start_log();
    tracing::info!("gamepalate {} started", env!("CARGO_PKG_VERSION"));
    let saved = saved_gui_settings().window;

    iced::application(move || App::new(saved), App::update, App::view)
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window_settings(saved))
        .settings(Settings {
            fonts: vec![
                font::REGULAR_BYTES.into(),
                font::SEMIBOLD_BYTES.into(),
                font::BOLD_BYTES.into(),
            ],
            default_font: font::REGULAR,
            default_text_size: Pixels(14.0),
            ..Settings::default()
        })
        .run()
        .inspect_err(|error| tracing::error!("the window stopped: {error}"))?;

    Ok(())
}
