pub mod discover;
mod game;
mod showcase;

pub use discover::Discover;

pub enum Screen {
    Discover(Box<Discover>),
    Votes,
    Profile,
}
