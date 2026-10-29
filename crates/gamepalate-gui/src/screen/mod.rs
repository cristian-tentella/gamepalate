pub mod recommendations;

pub use recommendations::Recommendations;

pub enum Screen {
    Recommendations(Recommendations),
}
