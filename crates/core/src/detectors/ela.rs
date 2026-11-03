use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageFormat, RgbImage};

use super::Detector;
use crate::{Confidence, DetectorKind, DocumentId, Finding, Rect, Result, Status};

#[derive(Debug, Clone)]
pub struct ElaConfig {
    /// JPEG quality used for the comparison re-save.
    pub quality: u8,
    /// Side length of the analysis blocks, in pixels.
    pub block: u32,
    /// How many standard deviations above the page average a block must be.
    pub z_threshold: f32,
    /// Absolute floor for the mean error, so flat pages produce nothing.
    pub min_error: f32,
    /// Smallest cluster of flagged blocks worth reporting.
    pub min_blocks: usize,
}

impl Default for ElaConfig {
    fn default() -> Self {
        Self {
            quality: 90,
            block: 16,
            z_threshold: 2.5,
            min_error: 1.5,
            min_blocks: 2,
        }
    }
}

pub struct ElaDetector {
    config: ElaConfig,
}

impl ElaDetector {
    pub fn new(config: ElaConfig) -> Self {
        Self { config }
    }

    fn recompress(&self, image: &RgbImage) -> Result<RgbImage> {
        let mut buf = Vec::new();
        JpegEncoder::new_with_quality(&mut buf, self.config.quality).encode(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgb8,
        )?;
        Ok(image::load_from_memory_with_format(&buf, ImageFormat::Jpeg)?.to_rgb8())
    }

    /// Mean absolute difference per block, in row-major order.
    fn block_errors(&self, original: &RgbImage, resaved: &RgbImage) -> (Vec<f32>, usize, usize) {
        let block = self.config.block;
        let cols = original.width().div_ceil(block) as usize;
        let rows = original.height().div_ceil(block) as usize;
        let mut sums = vec![0f64; cols * rows];
        let mut counts = vec![0u32; cols * rows];

        for (x, y, a) in original.enumerate_pixels() {
            let b = resaved.get_pixel(x, y);
            let diff: u32 =
                a.0.iter()
                    .zip(b.0.iter())
                    .map(|(p, q)| p.abs_diff(*q) as u32)
                    .sum();
            let idx = (y / block) as usize * cols + (x / block) as usize;
            sums[idx] += diff as f64 / 3.0;
            counts[idx] += 1;
        }

        let means = sums
            .iter()
            .zip(&counts)
            .map(|(s, c)| {
                if *c == 0 {
                    0.0
                } else {
                    (*s / *c as f64) as f32
                }
            })
            .collect();
        (means, cols, rows)
    }
}

impl Default for ElaDetector {
    fn default() -> Self {
        Self::new(ElaConfig::default())
    }
}

impl Detector for ElaDetector {
    fn kind(&self) -> DetectorKind {
        DetectorKind::ErrorLevel
    }

    fn analyze(&self, document: &DocumentId, page: u32, image: &RgbImage) -> Result<Vec<Finding>> {
        let resaved = self.recompress(image)?;
        let (errors, cols, rows) = self.block_errors(image, &resaved);

        let mean = errors.iter().sum::<f32>() / errors.len() as f32;
        let variance = errors.iter().map(|e| (e - mean).powi(2)).sum::<f32>() / errors.len() as f32;
        let std = variance.sqrt();
        if std < 1e-6 {
            return Ok(Vec::new());
        }

        let threshold = (mean + self.config.z_threshold * std).max(self.config.min_error);
        let flagged: Vec<bool> = errors.iter().map(|e| *e >= threshold).collect();

        let block = self.config.block;
        let mut seen = vec![false; flagged.len()];
        let mut findings = Vec::new();

        for start in 0..flagged.len() {
            if !flagged[start] || seen[start] {
                continue;
            }

            seen[start] = true;
            let mut stack = vec![start];
            let (mut min_c, mut max_c, mut min_r, mut max_r) = (cols, 0, rows, 0);
            let mut count = 0usize;
            let mut peak = 0f32;

            while let Some(i) = stack.pop() {
                let (c, r) = (i % cols, i / cols);
                min_c = min_c.min(c);
                max_c = max_c.max(c);
                min_r = min_r.min(r);
                max_r = max_r.max(r);
                count += 1;
                peak = peak.max(errors[i]);

                for n in neighbours(i, cols, rows).into_iter().flatten() {
                    if flagged[n] && !seen[n] {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }

            if count < self.config.min_blocks {
                continue;
            }

            let x = min_c as u32 * block;
            let y = min_r as u32 * block;
            let right = ((max_c as u32 + 1) * block).min(image.width());
            let bottom = ((max_r as u32 + 1) * block).min(image.height());
            let region = Rect::new(x as f32, y as f32, (right - x) as f32, (bottom - y) as f32);

            // Heuristic only: ELA is a weak signal, so confidence stays low
            // and rises slowly with how far the region sits above the page.
            let z = (peak - mean) / std;
            let score = (0.3 + 0.1 * (z - self.config.z_threshold)).clamp(0.3, 0.7);

            findings.push(Finding::new(
                document.clone(),
                page,
                region,
                DetectorKind::ErrorLevel,
                Status::PotentialManipulation,
                Confidence::new(score)?,
                format!(
                    "Compression error is {:.1}x the page average across {} blocks, \
                     which can indicate a region saved separately from its surroundings",
                    peak / mean.max(f32::EPSILON),
                    count
                ),
            ));
        }

        Ok(findings)
    }
}

fn neighbours(i: usize, cols: usize, rows: usize) -> [Option<usize>; 4] {
    let (c, r) = (i % cols, i / cols);
    [
        (c > 0).then(|| i - 1),
        (c + 1 < cols).then(|| i + 1),
        (r > 0).then(|| i - cols),
        (r + 1 < rows).then(|| i + cols),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn id() -> DocumentId {
        DocumentId("test-doc".into())
    }

    #[test]
    fn flat_page_has_no_findings() {
        let page = RgbImage::from_pixel(128, 128, Rgb([200, 200, 200]));
        let findings = ElaDetector::default().analyze(&id(), 0, &page).unwrap();
        assert!(findings.is_empty());
    }

    /// Smooth gradient with deterministic grain; `seed` changes the grain.
    fn noisy_page(seed: u32) -> RgbImage {
        RgbImage::from_fn(256, 256, |x, y| {
            let mut h = x
                .wrapping_mul(374_761_393)
                .wrapping_add(y.wrapping_mul(668_265_263))
                .wrapping_add(seed);
            h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
            let grain = ((h ^ (h >> 16)) % 25) as i32 - 12;
            let v = (x as i32 / 2 + 64 + grain).clamp(0, 255) as u8;
            Rgb([v, v, v])
        })
    }

    #[test]
    fn patch_with_a_different_save_history_is_flagged() {
        // The page was saved once at quality 60, then a fresh region was pasted in.
        let first_save = ElaDetector::new(ElaConfig {
            quality: 60,
            ..ElaConfig::default()
        });
        let mut page = first_save.recompress(&noisy_page(1)).unwrap();

        let fresh = noisy_page(2);
        for y in 96..160 {
            for x in 96..160 {
                page.put_pixel(x, y, *fresh.get_pixel(x, y));
            }
        }

        let detector = ElaDetector::new(ElaConfig {
            min_error: 0.5,
            ..ElaConfig::default()
        });
        let findings = detector.analyze(&id(), 0, &page).unwrap();
        assert!(!findings.is_empty());

        let patch = Rect::new(96.0, 96.0, 64.0, 64.0);
        assert!(findings.iter().any(|f| f.region.iou(&patch) > 0.4));
        assert!(findings
            .iter()
            .all(|f| f.status == Status::PotentialManipulation));
    }
}
