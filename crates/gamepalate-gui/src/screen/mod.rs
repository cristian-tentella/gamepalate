pub mod discover;

pub use discover::Discover;

pub enum Screen {
    Discover(Discover),
    Votes,
    Profile,
}
