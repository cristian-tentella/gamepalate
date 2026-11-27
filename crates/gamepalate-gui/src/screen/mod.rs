use std::{
    ffi::OsStr,
    fs::OpenOptions,
    io::{self, ErrorKind, Write},
    path::Path,
    time::Duration,
};

use gamepalate_app::NEW_FAMILIES_FILE;
use iced::{
    Center, Element, Fill, Task,
    widget::{button, container, row, text},
};

use crate::{font, style};

pub mod discover;
pub(crate) mod game;
pub mod library;
mod list_edit;
pub mod profile;
pub(crate) mod save_menu;
pub mod settings;
pub mod setup;
pub(crate) mod showcase;
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

impl Screen {
    /// The blurred pages under the sheets no longer fit the window.
    pub fn resized(&mut self) {
        match self {
            Screen::Discover(screen) => screen.resized(),
            Screen::Votes(screen) => screen.resized(),
            _ => {}
        }
    }
}

pub(crate) fn open_families(data_dir: &Path) {
    let path = data_dir.join("families.toml");
    if create_missing(&path).is_ok() {
        open(path);
    }
}

/// Opens a link, a folder or a file with what the system uses for it.
pub(crate) fn open(target: impl AsRef<OsStr>) {
    let target = target.as_ref();
    if let Err(error) = open::that_detached(target) {
        tracing::warn!("could not open {}: {error}", target.display());
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
    icon: Option<char>,
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
        self.show_toast(None, text, undo, hide)
    }

    pub(crate) fn show_with_icon<M: Send + 'static>(
        &mut self,
        icon: char,
        text: String,
        hide: fn(u64) -> M,
    ) -> Task<M> {
        self.show_toast(Some(icon), text, false, hide)
    }

    fn show_toast<M: Send + 'static>(
        &mut self,
        icon: Option<char>,
        text: String,
        undo: bool,
        hide: fn(u64) -> M,
    ) -> Task<M> {
        self.shown += 1;
        let id = self.shown;
        self.current = Some(Toast {
            id,
            icon,
            text,
            undo,
        });

        Task::perform(tokio::time::sleep(TOAST_DURATION), move |_| hide(id))
    }

    #[cfg(test)]
    pub(crate) fn shown(&self) -> Option<(Option<char>, &str)> {
        self.current
            .as_ref()
            .map(|toast| (toast.icon, toast.text.as_str()))
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
            let icon = toast.icon.map(|icon| {
                text(icon)
                    .font(font::ICONS)
                    .size(font::icon(font::NOTE_SIZE))
            });
            container(
                container(
                    row![]
                        .push(icon)
                        .push(text(&toast.text))
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

    #[tokio::test]
    async fn a_toast_can_have_an_icon_before_its_text() {
        let mut toasts = Toasts::default();

        let _ = toasts.show_with_icon(font::CHECK, "Catalog complete".to_string(), |id| id);
        assert_eq!(
            toasts.shown(),
            Some((Some(font::CHECK), "Catalog complete"))
        );

        let _ = toasts.show("Hades: saved".to_string(), true, |id| id);
        assert_eq!(toasts.shown(), Some((None, "Hades: saved")));
    }

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
