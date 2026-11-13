//! The blurred picture of the page behind the game search: a screenshot of the window without
//! the top bar, shrunk, blurred, and stretched back over the page.

use std::time::{Duration, Instant};

use iced::{Rectangle, widget::image::Handle, window::Screenshot};
use image::{RgbaImage, imageops};

/// The screenshot is shrunk by this much before the blur: the blur is cheaper, and the picture
/// is stretched back with a smooth filter.
const SHRINK: u32 = 4;
/// The blur radius on the shrunk picture: about 24 pixels on the window.
const SIGMA: f32 = 6.0;
/// The blurred picture fades in, so its late arrival looks like part of the opening.
pub const FADE: Duration = Duration::from_millis(120);

pub struct Blurred {
    pub handle: Handle,
}

/// The part of the window below `top`, in logical pixels, shrunk and blurred.
pub fn blur(screenshot: &Screenshot, top: f32) -> Blurred {
    let below = below_the_bar(screenshot, top);
    let start = below.y as usize * below.width as usize * 4;
    let (small, small_width, small_height) =
        shrink(&screenshot.rgba[start..], below.width, below.height, SHRINK);
    let small = RgbaImage::from_raw(small_width, small_height, small)
        .expect("the shrunk picture has the size it says");
    let blurred = imageops::fast_blur(&small, SIGMA);

    Blurred {
        handle: Handle::from_rgba(small_width, small_height, blurred.into_raw()),
    }
}

/// How far the fade has gone, from 0 to 1.
pub fn faded_in(since: Instant, now: Instant) -> f32 {
    (now.saturating_duration_since(since).as_secs_f32() / FADE.as_secs_f32()).min(1.0)
}

fn below_the_bar(screenshot: &Screenshot, top: f32) -> Rectangle<u32> {
    let size = screenshot.size;
    let top = ((top * screenshot.scale_factor).round() as u32).min(size.height.saturating_sub(1));

    Rectangle {
        x: 0,
        y: top,
        width: size.width,
        height: size.height - top,
    }
}

/// Each `factor`×`factor` block of pixels becomes their average. The pixels left over at the
/// right and bottom edges are dropped.
fn shrink(rgba: &[u8], width: u32, height: u32, factor: u32) -> (Vec<u8>, u32, u32) {
    let small_width = (width / factor).max(1);
    let small_height = (height / factor).max(1);
    let (factor_x, factor_y) = (factor.min(width), factor.min(height));
    let count = factor_x * factor_y;
    let stride = width as usize * 4;
    let mut small = Vec::with_capacity(small_width as usize * small_height as usize * 4);

    for y in 0..small_height {
        for x in 0..small_width {
            let mut sum = [0u32; 4];
            for dy in 0..factor_y {
                let row = (y * factor_y + dy) as usize * stride;
                for dx in 0..factor_x {
                    let at = row + (x * factor_x + dx) as usize * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(rgba[at + channel]);
                    }
                }
            }
            small.extend(sum.map(|total| (total / count) as u8));
        }
    }

    (small, small_width, small_height)
}

#[cfg(test)]
mod tests {
    use iced::Size;

    use super::*;

    fn screenshot(width: u32, height: u32, scale_factor: f32) -> Screenshot {
        Screenshot::new(
            vec![0; (width * height * 4) as usize],
            Size::new(width, height),
            scale_factor,
        )
    }

    #[test]
    fn the_bar_is_cut_away_in_physical_pixels() {
        let region = below_the_bar(&screenshot(200, 100, 1.5), 20.0);

        assert_eq!(
            region,
            Rectangle {
                x: 0,
                y: 30,
                width: 200,
                height: 70
            }
        );
    }

    #[test]
    fn a_bar_taller_than_the_window_leaves_one_row() {
        let region = below_the_bar(&screenshot(200, 100, 1.0), 500.0);

        assert_eq!(region.height, 1);
    }

    #[test]
    fn shrinking_averages_each_block() {
        let pixels: Vec<u8> = [[0, 0, 0, 255], [100, 200, 40, 255]]
            .into_iter()
            .cycle()
            .take(4 * 2)
            .flatten()
            .collect();

        let (small, width, height) = shrink(&pixels, 4, 2, 2);

        assert_eq!((width, height), (2, 1));
        assert_eq!(small, [50, 100, 20, 255, 50, 100, 20, 255]);
    }

    #[test]
    fn a_picture_smaller_than_the_factor_keeps_one_pixel() {
        let (small, width, height) = shrink(&[10, 20, 30, 255], 1, 1, 4);

        assert_eq!((width, height), (1, 1));
        assert_eq!(small, [10, 20, 30, 255]);
    }

    #[test]
    fn the_fade_ends_after_its_time() {
        let start = Instant::now();

        assert_eq!(faded_in(start, start), 0.0);
        assert_eq!(faded_in(start, start + FADE / 2), 0.5);
        assert_eq!(faded_in(start, start + FADE * 2), 1.0);
    }

    #[test]
    fn blurring_keeps_the_part_below_the_bar() {
        let blurred = blur(&screenshot(80, 60, 1.0), 20.0);

        let Handle::Rgba { width, height, .. } = blurred.handle else {
            panic!("the blurred picture should be raw pixels");
        };
        assert_eq!((width, height), (20, 10));
    }
}
