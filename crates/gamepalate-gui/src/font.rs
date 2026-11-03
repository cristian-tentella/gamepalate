use iced::{Font, font::Weight};

pub const REGULAR_BYTES: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf");
pub const SEMIBOLD_BYTES: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-SemiBold.ttf");
pub const BOLD_BYTES: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Bold.ttf");

pub const BAR_SIZE: f32 = 14.0;
pub const NOTE_SIZE: f32 = 13.0;
pub const VOTE_SIZE: f32 = 15.0;
pub const TITLE_SIZE: f32 = 28.0;
pub const INFO_SIZE: f32 = 16.0;
pub const PRICE_SIZE: f32 = 22.0;
pub const AMOUNT_SIZE: f32 = 20.0;
pub const STAT_SIZE: f32 = 32.0;
pub const PRICE_LINE_HEIGHT: f32 = 1.1;
pub const STAT_LINE_HEIGHT: f32 = 1.0;
const ASCENT: f32 = 0.984;
const DESCENT: f32 = 0.316;

pub const ICONS: Font = Font::with_name("Segoe Fluent Icons");

pub const REGULAR: Font = Font::with_name("Atkinson Hyperlegible Next");
pub const SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..REGULAR
};
pub const BOLD: Font = Font {
    weight: Weight::Bold,
    ..REGULAR
};

pub fn baseline(size: f32, line_height: f32) -> f32 {
    size * (line_height + ASCENT - DESCENT) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baseline_sits_lower_in_a_taller_line() {
        assert!((baseline(20.0, ASCENT + DESCENT) - 20.0 * ASCENT).abs() < 1e-4);
        assert!(baseline(20.0, 1.3) > baseline(20.0, 1.0));
    }
}
