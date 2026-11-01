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
