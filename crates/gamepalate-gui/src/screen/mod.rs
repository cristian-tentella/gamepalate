pub mod discover;

pub use discover::Discover;

pub enum Screen {
    Discover(Box<Discover>),
    Votes,
    Profile,
}
