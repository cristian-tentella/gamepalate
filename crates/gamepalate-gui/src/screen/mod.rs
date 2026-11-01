use std::{fs::OpenOptions, io, path::Path, process::Command};

pub mod discover;
mod game;
pub mod library;
pub mod profile;
pub mod settings;
pub mod setup;
mod showcase;
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

fn create_missing(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_created_empty_and_an_existing_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");

        create_missing(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");

        std::fs::write(&path, "[families.cozy]\n").unwrap();
        create_missing(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[families.cozy]\n");
    }
}
