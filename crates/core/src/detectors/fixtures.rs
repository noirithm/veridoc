//! Sample pages shared by the detector tests.

use image::{Rgb, RgbImage};

/// A white page carrying six lines of dark vertical strokes, a stand-in for
/// printed text. Nothing on it has been edited.
pub(crate) fn text_page() -> RgbImage {
    let mut page = RgbImage::from_pixel(320, 256, Rgb([255, 255, 255]));
    for line in 0..6 {
        let top = 20 + line * 38;
        for stroke in 0..30 {
            let left = 10 + stroke * 10;
            for y in top..top + 14 {
                for x in left..left + 2 {
                    page.put_pixel(x, y, Rgb([0, 0, 0]));
                }
            }
        }
    }
    page
}
