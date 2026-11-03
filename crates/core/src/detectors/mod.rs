mod blocks;
mod copy_move;
mod ela;
mod noise;
mod stats;
mod typography;

pub use copy_move::{CopyMoveConfig, CopyMoveDetector};
pub use ela::{ElaConfig, ElaDetector};
pub use noise::{NoiseConfig, NoiseDetector};
pub use typography::{TypographyConfig, TypographyDetector};

use image::RgbImage;

use crate::{DetectorKind, DocumentId, Finding, Result};

/// A single forensic check that inspects one page image.
pub trait Detector {
    fn kind(&self) -> DetectorKind;

    fn analyze(&self, document: &DocumentId, page: u32, image: &RgbImage) -> Result<Vec<Finding>>;
}
