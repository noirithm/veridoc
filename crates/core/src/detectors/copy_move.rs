use std::collections::BTreeMap;

use image::{GrayImage, RgbImage};

use super::blocks::find_clusters;
use super::Detector;
use crate::{Confidence, DetectorKind, DocumentId, Finding, Rect, Result, Status};

/// Each block is summarised as a GRID x GRID table of cell averages.
const GRID: usize = 4;
const FEATURES: usize = GRID * GRID;
/// Feature values are rounded to this step when sorting candidates.
const QUANT_STEP: f32 = 4.0;

#[derive(Debug, Clone)]
pub struct CopyMoveConfig {
    /// Side length of the compared blocks, in pixels (rounded to a multiple of 4).
    pub block: u32,
    /// Distance between neighbouring block origins, in pixels.
    pub stride: u32,
    /// Blocks flatter than this are ignored, since blank areas always match.
    pub min_contrast: f32,
    /// Largest mean difference between two block summaries to call them a match.
    pub max_distance: f32,
    /// Matches closer together than this are treated as repeating patterns.
    pub min_shift: f32,
    /// How many following candidates each block is compared against.
    pub window: usize,
    /// Smallest group of matching blocks worth reporting.
    pub min_blocks: usize,
}

impl Default for CopyMoveConfig {
    fn default() -> Self {
        Self {
            block: 16,
            stride: 4,
            min_contrast: 4.0,
            max_distance: 2.5,
            min_shift: 32.0,
            window: 6,
            min_blocks: 40,
        }
    }
}

pub struct CopyMoveDetector {
    config: CopyMoveConfig,
}

struct Candidate {
    index: usize,
    key: [i16; FEATURES],
    values: [f32; FEATURES],
}

/// Summed-area table: lets us total any rectangle of pixels in constant time.
struct IntegralImage {
    sums: Vec<u64>,
    row_len: usize,
}

impl IntegralImage {
    fn new(gray: &GrayImage) -> Self {
        let (w, h) = (gray.width() as usize, gray.height() as usize);
        let row_len = w + 1;
        let mut sums = vec![0u64; row_len * (h + 1)];

        for y in 0..h {
            let mut running = 0u64;
            for x in 0..w {
                running += u64::from(gray.get_pixel(x as u32, y as u32)[0]);
                sums[(y + 1) * row_len + x + 1] = sums[y * row_len + x + 1] + running;
            }
        }
        Self { sums, row_len }
    }

    fn sum(&self, x: usize, y: usize, w: usize, h: usize) -> u64 {
        let at = |cx: usize, cy: usize| self.sums[cy * self.row_len + cx];
        at(x + w, y + h) + at(x, y) - at(x + w, y) - at(x, y + h)
    }
}

impl CopyMoveDetector {
    pub fn new(mut config: CopyMoveConfig) -> Self {
        let grid = GRID as u32;
        config.block = (config.block / grid).max(1) * grid;
        config.stride = config.stride.max(1);
        Self { config }
    }

    /// Brightness-independent summary of one block, or `None` if it is too flat.
    fn block_values(&self, table: &IntegralImage, x: usize, y: usize) -> Option<[f32; FEATURES]> {
        let cell = self.config.block as usize / GRID;
        let area = (cell * cell) as f32;

        let mut values = [0f32; FEATURES];
        for gy in 0..GRID {
            for gx in 0..GRID {
                let total = table.sum(x + gx * cell, y + gy * cell, cell, cell);
                values[gy * GRID + gx] = total as f32 / area;
            }
        }

        let mean = values.iter().sum::<f32>() / FEATURES as f32;
        for v in &mut values {
            *v -= mean;
        }
        let contrast = (values.iter().map(|v| v * v).sum::<f32>() / FEATURES as f32).sqrt();

        (contrast >= self.config.min_contrast).then_some(values)
    }

    fn candidates(&self, gray: &GrayImage, cols: usize, rows: usize) -> Vec<Candidate> {
        let table = IntegralImage::new(gray);
        let stride = self.config.stride as usize;
        let mut found = Vec::new();

        for gy in 0..rows {
            for gx in 0..cols {
                if let Some(values) = self.block_values(&table, gx * stride, gy * stride) {
                    found.push(Candidate {
                        index: gy * cols + gx,
                        key: quantize(&values),
                        values,
                    });
                }
            }
        }

        found.sort_unstable_by_key(|c| c.key);
        found
    }
}

impl Default for CopyMoveDetector {
    fn default() -> Self {
        Self::new(CopyMoveConfig::default())
    }
}

impl Detector for CopyMoveDetector {
    fn kind(&self) -> DetectorKind {
        DetectorKind::CopyMove
    }

    fn analyze(&self, document: &DocumentId, page: u32, image: &RgbImage) -> Result<Vec<Finding>> {
        let block = self.config.block;
        if image.width() < block || image.height() < block {
            return Ok(Vec::new());
        }

        let stride = self.config.stride as usize;
        let cols = ((image.width() - block) / self.config.stride + 1) as usize;
        let rows = ((image.height() - block) / self.config.stride + 1) as usize;

        let gray = image::imageops::grayscale(image);
        let candidates = self.candidates(&gray, cols, rows);

        // Each matching pair votes for the shift between its two blocks.
        // Votes are stored as the grid index of the block at the origin side.
        let step = self.config.stride as i32;
        let mut votes: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();

        for (pos, first) in candidates.iter().enumerate() {
            for second in candidates.iter().skip(pos + 1).take(self.config.window) {
                if mean_abs_diff(&first.values, &second.values) > self.config.max_distance {
                    continue;
                }

                let dx = ((second.index % cols) as i32 - (first.index % cols) as i32) * step;
                let dy = ((second.index / cols) as i32 - (first.index / cols) as i32) * step;
                if ((dx * dx + dy * dy) as f32).sqrt() < self.config.min_shift {
                    continue;
                }

                // Store every pair with the same orientation so that
                // A -> B and B -> A count as the same shift.
                let (source, dx, dy) = if dy < 0 || (dy == 0 && dx < 0) {
                    (second.index, -dx, -dy)
                } else {
                    (first.index, dx, dy)
                };
                votes.entry((dx, dy)).or_default().push(source);
            }
        }

        let mut findings = Vec::new();
        for ((dx, dy), sources) in votes {
            if sources.len() < self.config.min_blocks {
                continue;
            }

            let mut flagged = vec![false; cols * rows];
            for &i in &sources {
                flagged[i] = true;
            }

            for cluster in find_clusters(&flagged, cols, rows) {
                let count = cluster.members.len();
                if count < self.config.min_blocks {
                    continue;
                }

                let left = (cluster.min_col * stride) as f32;
                let top = (cluster.min_row * stride) as f32;
                let right = (cluster.max_col * stride) as f32 + block as f32;
                let bottom = (cluster.max_row * stride) as f32 + block as f32;

                let source = Rect::new(left, top, right - left, bottom - top);
                let copy = Rect::new(
                    left + dx as f32,
                    top + dy as f32,
                    source.width,
                    source.height,
                );

                // Overlapping regions point to repeating patterns, not a paste.
                if source.intersection(&copy).is_some() {
                    continue;
                }

                // Identical content in two places is strong evidence, but
                // stamps and logos can repeat legitimately, so it stays capped.
                let score = (0.45 + 0.4 * (count as f32 / 300.0).min(1.0)).clamp(0.45, 0.85);
                let confidence = Confidence::new(score)?;

                for (region, other) in [(source, copy), (copy, source)] {
                    findings.push(Finding::new(
                        document.clone(),
                        page,
                        region,
                        DetectorKind::CopyMove,
                        Status::PotentialManipulation,
                        confidence,
                        format!(
                            "This region closely matches the region at ({:.0}, {:.0}) \
                             across {} overlapping blocks, which can indicate content \
                             copied from one part of the page to another",
                            other.x, other.y, count
                        ),
                    ));
                }
            }
        }

        Ok(findings)
    }
}

fn quantize(values: &[f32; FEATURES]) -> [i16; FEATURES] {
    let mut key = [0i16; FEATURES];
    for (slot, v) in key.iter_mut().zip(values) {
        *slot = (v / QUANT_STEP).round() as i16;
    }
    key
}

fn mean_abs_diff(a: &[f32; FEATURES], b: &[f32; FEATURES]) -> f32 {
    a.iter().zip(b).map(|(p, q)| (p - q).abs()).sum::<f32>() / FEATURES as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn id() -> DocumentId {
        DocumentId("test-doc".into())
    }

    /// Deterministic blotchy texture: random brightness per 6px square.
    fn texture(x: u32, y: u32) -> u8 {
        let (cx, cy) = (x / 6, y / 6);
        let mut h = cx
            .wrapping_mul(374_761_393)
            .wrapping_add(cy.wrapping_mul(668_265_263));
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        (h ^ (h >> 16)) as u8
    }

    fn textured_page() -> RgbImage {
        RgbImage::from_fn(256, 256, |x, y| Rgb([texture(x, y); 3]))
    }

    #[test]
    fn flat_page_has_no_findings() {
        let page = RgbImage::from_pixel(128, 128, Rgb([220, 220, 220]));
        let findings = CopyMoveDetector::default()
            .analyze(&id(), 0, &page)
            .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn unique_texture_has_no_findings() {
        let findings = CopyMoveDetector::default()
            .analyze(&id(), 0, &textured_page())
            .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn pasted_patch_is_found_at_both_locations() {
        let mut page = textured_page();
        let original = page.clone();
        for y in 0..64 {
            for x in 0..64 {
                page.put_pixel(160 + x, 128 + y, *original.get_pixel(32 + x, 32 + y));
            }
        }

        let findings = CopyMoveDetector::default()
            .analyze(&id(), 0, &page)
            .unwrap();
        assert!(!findings.is_empty());

        let source = Rect::new(32.0, 32.0, 64.0, 64.0);
        let copy = Rect::new(160.0, 128.0, 64.0, 64.0);
        assert!(findings.iter().any(|f| f.region.iou(&source) > 0.5));
        assert!(findings.iter().any(|f| f.region.iou(&copy) > 0.5));
        assert!(findings
            .iter()
            .all(|f| f.detector == DetectorKind::CopyMove));
    }
}
