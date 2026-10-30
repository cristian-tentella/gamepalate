use iced::{Font, font::Weight};

pub const REGULAR_BYTES: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
pub const BOLD_BYTES: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");

pub const REGULAR: Font = Font::with_name("Inter");
pub const BOLD: Font = Font {
    weight: Weight::Bold,
    ..REGULAR
};
