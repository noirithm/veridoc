use image::RgbImage;

use super::blocks::find_clusters;
use super::stats::median;
use super::Detector;
use crate::{Confidence, DetectorKind, DocumentId, Finding, Result, Status};

#[derive(Debug, Clone)]
pub struct NoiseConfig {
    /// Side length of the analysis blocks, in pixels.
    pub block: u32,
    /// Robust z-score a block must reach (in either direction) to be flagged.
    pub z_threshold: f32,
    /// Lower bound for the spread, so near-identical blocks don't inflate scores.
    pub min_spread: f32,
    /// Smallest cluster of flagged blocks worth reporting.
    pub min_blocks: usize,
}

impl Default for NoiseConfig {
    fn default() -> Self {
        Self {
            block: 16,
            z_threshold: 4.0,
            min_spread: 2.0,
            min_blocks: 2,
        }
    }
}

pub struct NoiseDetector {
    config: NoiseConfig,
}

impl NoiseDetector {
    pub fn new(config: NoiseConfig) -> Self {
        Self { config }
    }

    fn luma(image: &RgbImage) -> Vec<f32> {
        image
            .pixels()
            .map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
            .collect()
    }

    /// Mean squared high-pass residual per block, row by row. A pixel's
    /// residual is how far it sits from the average of its 8 neighbours.
    fn block_levels(&self, luma: &[f32], width: u32, height: u32) -> (Vec<f32>, usize, usize) {
        let (w, h) = (width as usize, height as usize);
        let block = self.config.block as usize;
        let cols = w.div_ceil(block);
        let rows = h.div_ceil(block);
        let mut sums = vec![0f32; cols * rows];
        let mut counts = vec![0u32; cols * rows];

        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let mut around = 0.0;
                for ny in y - 1..=y + 1 {
                    for nx in x - 1..=x + 1 {
                        if nx != x || ny != y {
                            around += luma[ny * w + nx];
                        }
                    }
                }
                let residual = luma[y * w + x] - around / 8.0;
                let idx = (y / block) * cols + x / block;
                sums[idx] += residual * residual;
                counts[idx] += 1;
            }
        }

        let levels = sums
            .iter()
            .zip(&counts)
            .map(|(s, c)| if *c == 0 { 0.0 } else { s / *c as f32 })
            .collect();
        (levels, cols, rows)
    }
}

impl Default for NoiseDetector {
    fn default() -> Self {
        Self::new(NoiseConfig::default())
    }
}

impl Detector for NoiseDetector {
    fn kind(&self) -> DetectorKind {
        DetectorKind::Noise
    }

    fn analyze(&self, document: &DocumentId, page: u32, image: &RgbImage) -> Result<Vec<Finding>> {
        if image.width() < 3 || image.height() < 3 {
            return Ok(Vec::new());
        }

        let luma = Self::luma(image);
        let (levels, cols, rows) = self.block_levels(&luma, image.width(), image.height());

        // Median and MAD are used instead of mean and standard deviation so
        // the tampered blocks themselves don't skew the page baseline.
        let centre = median(&mut levels.clone());
        let mut deviations: Vec<f32> = levels.iter().map(|l| (l - centre).abs()).collect();
        let spread = (1.4826 * median(&mut deviations)).max(self.config.min_spread);

        let scores: Vec<f32> = levels.iter().map(|l| (l - centre) / spread).collect();
        let flagged: Vec<bool> = scores
            .iter()
            .map(|z| z.abs() >= self.config.z_threshold)
            .collect();

        let mut findings = Vec::new();
        for cluster in find_clusters(&flagged, cols, rows) {
            if cluster.members.len() < self.config.min_blocks {
                continue;
            }

            let region = cluster.region(self.config.block, image.width(), image.height());
            let count = cluster.members.len();
            let mean_level = cluster.members.iter().map(|&i| levels[i]).sum::<f32>() / count as f32;
            let peak = cluster
                .members
                .iter()
                .map(|&i| scores[i].abs())
                .fold(0.0, f32::max);

            // Noise statistics are easily affected by page content, so the
            // confidence stays modest and grows slowly with the deviation.
            let confidence = (0.25 + 0.05 * (peak - self.config.z_threshold)).clamp(0.25, 0.6);
            let direction = if mean_level < centre {
                "lower"
            } else {
                "higher"
            };

            findings.push(Finding::new(
                document.clone(),
                page,
                region,
                DetectorKind::Noise,
                Status::PotentialManipulation,
                Confidence::new(confidence)?,
                format!(
                    "Pixel noise is {direction} than the rest of the page \
                     ({mean_level:.1} against a typical {centre:.1}) across {count} blocks, \
                     which can indicate content from a different source"
                ),
            ));
        }

        Ok(findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn id() -> DocumentId {
        DocumentId("test-doc".into())
    }

    /// Deterministic pseudo-random grain between -8 and 8.
    fn grain(x: u32, y: u32) -> f32 {
        let mut h = x
            .wrapping_mul(374_761_393)
            .wrapping_add(y.wrapping_mul(668_265_263));
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        h ^= h >> 16;
        (h % 17) as f32 - 8.0
    }

    #[test]
    fn flat_page_has_no_findings() {
        let page = RgbImage::from_pixel(128, 128, Rgb([200, 200, 200]));
        let findings = NoiseDetector::default().analyze(&id(), 0, &page).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn evenly_grainy_page_has_no_findings() {
        let page = RgbImage::from_fn(256, 256, |x, y| {
            let v = (128.0 + grain(x, y)) as u8;
            Rgb([v, v, v])
        });
        let findings = NoiseDetector::default().analyze(&id(), 0, &page).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn smooth_patch_on_grainy_page_is_flagged() {
        let page = RgbImage::from_fn(256, 256, |x, y| {
            let in_patch = (96..160).contains(&x) && (96..160).contains(&y);
            let v = if in_patch { 128.0 } else { 128.0 + grain(x, y) };
            Rgb([v as u8; 3])
        });

        let findings = NoiseDetector::default().analyze(&id(), 0, &page).unwrap();
        assert!(!findings.is_empty());

        let patch = crate::Rect::new(96.0, 96.0, 64.0, 64.0);
        assert!(findings.iter().any(|f| f.region.iou(&patch) > 0.4));
    }
}
