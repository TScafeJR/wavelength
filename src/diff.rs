//! Pixel comparison between two renders of the same page.

use crate::image::Image;

/// Differing pixels are drawn in this colour in the overlay.
const HIGHLIGHT: [u8; 4] = [255, 0, 255, 255];

/// The result of comparing two images.
#[derive(Debug, Clone)]
pub struct Diff {
    /// How many pixels differ. A pixel that exists in only one image counts as differing.
    pub differing: u64,
    /// How many pixels were compared.
    pub total: u64,
    /// The reference image, faded, with every differing pixel highlighted.
    pub overlay: Image,
}

impl Diff {
    /// The share of pixels that differ, from 0 to 100.
    pub fn percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.differing as f64 * 100.0 / self.total as f64
    }
}

/// Compares the engine's render against a reference, pixel for pixel.
pub fn compare(engine: &Image, reference: &Image) -> Diff {
    let width = engine.width().max(reference.width());
    let height = engine.height().max(reference.height());
    let mut overlay = Image::new(width, height, HIGHLIGHT);
    let mut differing = 0;
    for y in 0..height {
        for x in 0..width {
            match (engine.pixel(x, y), reference.pixel(x, y)) {
                (Some(ours), Some(theirs)) if ours == theirs => {
                    // Fade matching pixels towards white so the highlights stand out.
                    let fade = |channel: u8| 255 - (255 - channel) / 4;
                    let faded = [fade(theirs[0]), fade(theirs[1]), fade(theirs[2]), 255];
                    overlay.fill(x, y, x + 1, y + 1, faded);
                }
                _ => differing += 1,
            }
        }
    }
    Diff {
        differing,
        total: u64::from(width) * u64::from(height),
        overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_do_not_differ() {
        let image = Image::new(4, 3, [10, 20, 30, 255]);
        let diff = compare(&image, &image);
        assert_eq!((diff.differing, diff.total), (0, 12));
        assert_eq!(diff.percent(), 0.0);
        assert_eq!(diff.overlay.pixel(0, 0), Some([194, 197, 199, 255]));
    }

    #[test]
    fn changed_pixels_are_counted_and_highlighted() {
        let reference = Image::new(4, 4, [0, 0, 0, 255]);
        let mut engine = reference.clone();
        engine.fill(1, 1, 3, 2, [255, 255, 255, 255]);
        let diff = compare(&engine, &reference);
        assert_eq!(diff.differing, 2);
        assert_eq!(diff.percent(), 12.5);
        assert_eq!(diff.overlay.pixel(1, 1), Some(HIGHLIGHT));
        assert_ne!(diff.overlay.pixel(0, 0), Some(HIGHLIGHT));
    }

    #[test]
    fn a_size_mismatch_counts_the_missing_area() {
        let diff = compare(&Image::new(2, 2, [0; 4]), &Image::new(2, 3, [0; 4]));
        assert_eq!((diff.differing, diff.total), (2, 6));
    }
}
