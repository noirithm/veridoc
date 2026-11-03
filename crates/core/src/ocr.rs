use image::RgbImage;
use serde::{Deserialize, Serialize};

use crate::{Confidence, Rect, Result};

/// A single recognised word and where it sits on the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrWord {
    pub text: String,
    pub region: Rect,
    pub confidence: Confidence,
}

/// Words that were read along the same line of text, left to right.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OcrLine {
    pub words: Vec<OcrWord>,
}

/// Anything that can read text from a page image.
pub trait OcrEngine {
    fn recognize(&self, image: &RgbImage) -> Result<Vec<OcrLine>>;
}
