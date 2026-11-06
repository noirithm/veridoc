use image::{GrayImage, RgbImage};

use super::blocks::{block_activity, find_clusters, peer_scores};
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
    /// Blocks on each side, in activity order, that count as comparable.
    pub peer_window: usize,
    /// Smallest cluster of flagged blocks worth reporting.
    pub min_blocks: usize,
    /// A pixel only counts as flat when no neighbour differs from it by more than this.
    pub flat_limit: u8,
    /// Blocks with fewer flat pixels than this are not measured.
    pub min_flat_pixels: u32,
    /// Smallest difference from comparable blocks worth reporting.
    pub min_difference: f32,
}

impl Default for NoiseConfig {
    fn default() -> Self {
        Self {
            block: 16,
            z_threshold: 5.0,
            min_spread: 2.0,
            peer_window: 75,
            min_blocks: 4,
            flat_limit: 20,
            min_flat_pixels: 64,
            min_difference: 8.0,
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

    /// Noise level per block, row by row, or `None` when a block has too few
    /// flat pixels to measure. Only flat pixels are used, because printed
    /// edges would otherwise be mistaken for noise. A pixel's residual is how
    /// far it sits from the average of its 8 neighbours.
    fn block_levels(&self, gray: &GrayImage) -> (Vec<Option<f32>>, usize, usize) {
        let (w, h) = (gray.width() as usize, gray.height() as usize);
        let block = self.config.block as usize;
        let cols = w.div_ceil(block);
        let rows = h.div_ceil(block);
        let mut sums = vec![0f32; cols * rows];
        let mut counts = vec![0u32; cols * rows];

        let pixels = gray.as_raw();
        let flat_limit = i32::from(self.config.flat_limit);

        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let centre = i32::from(pixels[y * w + x]);
                let mut around = 0i32;
                let mut steepest = 0i32;

                for ny in y - 1..=y + 1 {
                    for nx in x - 1..=x + 1 {
                        if nx != x || ny != y {
                            let value = i32::from(pixels[ny * w + nx]);
                            around += value;
                            steepest = steepest.max((value - centre).abs());
                        }
                    }
                }
                if steepest > flat_limit {
                    continue;
                }

                let residual = centre as f32 - around as f32 / 8.0;
                let index = (y / block) * cols + x / block;
                sums[index] += residual * residual;
                counts[index] += 1;
            }
        }

        let enough = self.config.min_flat_pixels;
        let levels = sums
            .iter()
            .zip(&counts)
            .map(|(sum, &count)| {
                if count >= enough {
                    Some(sum / count as f32)
                } else {
                    None
                }
            })
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

        let gray = image::imageops::grayscale(image);
        let (levels, cols, rows) = self.block_levels(&gray);
        let activity = block_activity(&gray, self.config.block);

        // Blocks that could not be measured take no part in the comparison.
        let measured: Vec<(usize, f32)> = levels
            .iter()
            .enumerate()
            .filter_map(|(i, level)| level.map(|l| (i, l)))
            .collect();
        let values: Vec<f32> = measured.iter().map(|&(_, level)| level).collect();
        let detail: Vec<f32> = measured.iter().map(|&(i, _)| activity[i]).collect();

        // Each block is judged against blocks with a similar amount of
        // detail, never against the whole page.
        let peers = peer_scores(
            &values,
            &detail,
            self.config.peer_window,
            self.config.min_spread,
        );

        let mut level = vec![0f32; levels.len()];
        let mut typical = vec![0f32; levels.len()];
        let mut strength = vec![0f32; levels.len()];
        let mut flagged = vec![false; levels.len()];

        for (&(i, value), peer) in measured.iter().zip(&peers) {
            level[i] = value;
            typical[i] = peer.typical;
            strength[i] = peer.z.abs();
            flagged[i] = peer.z.abs() >= self.config.z_threshold
                && (value - peer.typical).abs() >= self.config.min_difference;
        }

        let mut findings = Vec::new();
        for cluster in find_clusters(&flagged, cols, rows) {
            let count = cluster.members.len();
            if count < self.config.min_blocks {
                continue;
            }

            let region = cluster.region(self.config.block, image.width(), image.height());
            let mean_level = cluster.members.iter().map(|&i| level[i]).sum::<f32>() / count as f32;
            let mean_typical =
                cluster.members.iter().map(|&i| typical[i]).sum::<f32>() / count as f32;
            let strongest = cluster
                .members
                .iter()
                .map(|&i| strength[i])
                .fold(0.0, f32::max);

            // Noise statistics are easily affected by page content, so this
            // check stays weak on its own and needs another to agree.
            let confidence = (0.2 + 0.03 * (strongest - self.config.z_threshold)).clamp(0.2, 0.4);
            let direction = if mean_level < mean_typical {
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
                    "Pixel noise is {direction} than in comparable areas of the page \
                     ({mean_level:.1} against a typical {mean_typical:.1}) across {count} blocks, \
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
    use crate::detectors::fixtures::text_page;
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
    fn printed_text_on_blank_paper_is_not_flagged() {
        let findings = NoiseDetector::default()
            .analyze(&id(), 0, &text_page())
            .unwrap();
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
