mod ela;

pub use ela::{ElaConfig, ElaDetector};

use image::RgbImage;

use crate::{DetectorKind, DocumentId, Finding, Result};

/// A single forensic check that inspects one page image.
pub trait Detector {
    fn kind(&self) -> DetectorKind;

    fn analyze(&self, document: &DocumentId, page: u32, image: &RgbImage) -> Result<Vec<Finding>>;
}