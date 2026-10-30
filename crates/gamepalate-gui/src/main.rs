mod app;
mod screen;

use std::sync::Arc;

use gamepalate_app::Gamepalate;

use crate::app::App;

fn main() -> anyhow::Result<()> {
    let gamepalate = Arc::new(Gamepalate::open(None)?);

    iced::application(
        move || App::new(Arc::clone(&gamepalate)),
        App::update,
        App::view,
    )
    .theme(App::theme)
    .run()?;

    Ok(())
}
