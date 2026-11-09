use std::{
    fs::OpenOptions,
    io::{self, ErrorKind, Write},
    path::Path,
    process::Command,
    time::Duration,
};

use gamepalate_app::NEW_FAMILIES_FILE;
use iced::{
    Center, Element, Fill, Task,
    widget::{button, container, row, text},
};

use crate::{font, style};

pub mod discover;
mod game;
pub mod library;
pub mod profile;
pub mod settings;
pub mod setup;
mod showcase;
mod steam_client;
pub mod votes;

pub use discover::Discover;
pub use library::Library;
pub use profile::Profile;
pub use settings::Settings;
pub use setup::Setup;
pub use votes::Votes;

pub enum Screen {
    Discover(Box<Discover>),
    Votes(Box<Votes>),
    Library(Box<Library>),
    Setup(Box<Setup>),
    Profile(Box<Profile>),
    Settings(Box<Settings>),
    Starting,
}

pub(crate) fn open_families(data_dir: &Path) {
    let path = data_dir.join("families.toml");
    if create_missing(&path).is_ok() {
        let _ = Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(path)
            .spawn();
    }
}

const TOAST_DURATION: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct Toasts {
    shown: u64,
    current: Option<Toast>,
}

struct Toast {
    id: u64,
    text: String,
    undo: bool,
}

impl Toasts {
    pub(crate) fn show<M: Send + 'static>(
        &mut self,
        text: String,
        undo: bool,
        hide: fn(u64) -> M,
    ) -> Task<M> {
        self.shown += 1;
        let id = self.shown;
        self.current = Some(Toast { id, text, undo });

        Task::perform(tokio::time::sleep(TOAST_DURATION), move |_| hide(id))
    }

    pub(crate) fn hide(&mut self, id: u64) {
        if self.current.as_ref().is_some_and(|toast| toast.id == id) {
            self.current = None;
        }
    }

    pub(crate) fn view<'a, M: Clone + 'a>(&'a self, undo: M) -> Option<Element<'a, M>> {
        self.current.as_ref().map(|toast| {
            let undo = toast.undo.then(|| {
                button(text("Undo").size(font::NOTE_SIZE).font(font::SEMIBOLD))
                    .padding(0)
                    .style(style::toast_link)
                    .on_press(undo)
            });
            container(
                container(
                    row![text(&toast.text)]
                        .push(undo)
                        .spacing(14)
                        .align_y(Center),
                )
                .padding([8, 14])
                .style(style::toast),
            )
            .center_x(Fill)
            .align_bottom(Fill)
            .padding(16)
            .into()
        })
    }
}

pub(crate) fn automatic_mark<'a, M: 'a>() -> Element<'a, M> {
    container(text("Automatic").size(font::NOTE_SIZE))
        .padding([1, 8])
        .style(style::tag)
        .into()
}

pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn create_missing(path: &Path) -> io::Result<()> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(NEW_FAMILIES_FILE.as_bytes()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_starts_with_a_capital() {
        assert_eq!(
            sentence("there is already a family named \"cozy\""),
            "There is already a family named \"cozy\""
        );
        assert_eq!(sentence(""), "");
    }

    #[test]
    fn a_missing_file_is_created_with_the_comments_and_an_existing_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");

        create_missing(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), NEW_FAMILIES_FILE);

        std::fs::write(&path, "[families.cozy]\n").unwrap();
        create_missing(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[families.cozy]\n");
    }
}
