pub mod discover;
mod game;
pub mod setup;
mod showcase;
pub mod votes;

pub use discover::Discover;
pub use setup::Setup;
pub use votes::Votes;

pub enum Screen {
    Discover(Box<Discover>),
    Votes(Box<Votes>),
    Setup(Box<Setup>),
    Profile,
}
