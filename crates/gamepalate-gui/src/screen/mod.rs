pub mod discover;
mod game;
mod showcase;
pub mod votes;

pub use discover::Discover;
pub use votes::Votes;

pub enum Screen {
    Discover(Box<Discover>),
    Votes(Box<Votes>),
    Profile,
}
