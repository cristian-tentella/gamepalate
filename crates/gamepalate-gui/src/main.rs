#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod font;
mod format;
mod new_version;
mod progress;
mod screen;
mod smooth;
mod style;
mod window_icon;

use gamepalate_app::{log::start_log, setup::saved_gui_settings};
use iced::{Pixels, Settings};

use crate::app::{App, window_settings};

fn main() -> anyhow::Result<()> {
    let _ = start_log();
    tracing::info!("gamepalate {} started", env!("CARGO_PKG_VERSION"));
    let gui = saved_gui_settings();
    let saved = gui.window;
    let check = gui.check_for_new_versions;

    iced::application(move || App::new(saved, check), App::update, App::view)
        .title("gamepalate")
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window_settings(saved))
        .settings(Settings {
            fonts: vec![
                font::REGULAR_BYTES.into(),
                font::SEMIBOLD_BYTES.into(),
                font::BOLD_BYTES.into(),
                font::ICONS_BYTES.into(),
            ],
            default_font: font::REGULAR,
            default_text_size: Pixels(14.0),
            ..Settings::default()
        })
        .run()
        .inspect_err(|error| tracing::error!("the window stopped: {error}"))?;

    Ok(())
}
