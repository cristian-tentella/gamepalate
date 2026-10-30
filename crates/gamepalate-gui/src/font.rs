use iced::{Font, font::Weight};

pub const REGULAR_BYTES: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
pub const SEMIBOLD_BYTES: &[u8] = include_bytes!("../fonts/Inter-SemiBold.ttf");
pub const BOLD_BYTES: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");

pub const BAR_SIZE: f32 = 14.0;
pub const NOTE_SIZE: f32 = 12.0;
pub const VOTE_SIZE: f32 = 15.0;

pub const ICONS: Font = Font::with_name("Segoe Fluent Icons");

pub const REGULAR: Font = Font::with_name("Inter");
pub const SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..REGULAR
};
pub const BOLD: Font = Font {
    weight: Weight::Bold,
    ..REGULAR
};
