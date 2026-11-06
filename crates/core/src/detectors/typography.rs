use image::{GrayImage, RgbImage};

use super::stats::median;
use super::Detector;
use crate::{Confidence, DetectorKind, DocumentId, Finding, Rect, Result, Status};

#[derive(Debug, Clone)]
pub struct TypographyConfig {
    /// Shortest blob treated as a character, in pixels. Skips dots and specks.
    pub min_glyph_height: u32,
    /// Tallest blob treated as a character, in pixels. Skips photos and boxes.
    pub max_glyph_height: u32,
    /// Lines with fewer words have nothing to be compared against.
    pub min_words: usize,
    /// Words with fewer characters are too unreliable to measure.
    pub min_word_glyphs: usize,
    /// How many times thicker or thinner than its line a word must be.
    pub thickness_ratio: f32,
    /// Smallest baseline offset worth reporting, in pixels.
    pub min_baseline_shift: f32,
    /// Pages with less brightness range than this are treated as blank.
    pub min_contrast: u8,
}

impl Default for TypographyConfig {
    fn default() -> Self {
        Self {
            min_glyph_height: 6,
            max_glyph_height: 80,
            min_words: 3,
            min_word_glyphs: 2,
            thickness_ratio: 1.5,
            min_baseline_shift: 2.0,
            min_contrast: 60,
        }
    }
}

pub struct TypographyDetector {
    config: TypographyConfig,
}

/// One connected blob of ink. Right and bottom are exclusive.
#[derive(Debug, Clone, Copy)]
struct Glyph {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
    thickness: f32,
}

impl Glyph {
    fn width(&self) -> u32 {
        self.right - self.left
    }

    fn height(&self) -> u32 {
        self.bottom - self.top
    }
}

struct Line {
    top: u32,
    bottom: u32,
    glyphs: Vec<Glyph>,
}

impl Line {
    /// True when the glyph shares most of its vertical extent with this line.
    fn overlaps(&self, glyph: &Glyph) -> bool {
        let overlap = self
            .bottom
            .min(glyph.bottom)
            .saturating_sub(self.top.max(glyph.top));
        let shorter = (self.bottom - self.top).min(glyph.height());
        shorter > 0 && overlap as f32 / shorter as f32 >= 0.5
    }
}

struct Word {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
    thickness: f32,
    baseline: f32,
}

impl Word {
    fn from_glyphs(glyphs: Vec<Glyph>) -> Self {
        let mut thicknesses: Vec<f32> = glyphs.iter().map(|g| g.thickness).collect();
        let tallest = glyphs.iter().map(Glyph::height).max().unwrap_or(0);

        // Tails (g, p, y) and punctuation sit below or above the baseline, so
        // it is taken from the highest bottom among full-size glyphs.
        let lowest_flat = glyphs
            .iter()
            .filter(|g| g.height() * 2 >= tallest)
            .map(|g| g.bottom as f32)
            .fold(f32::INFINITY, f32::min);
        let baseline = if lowest_flat.is_finite() {
            lowest_flat
        } else {
            0.0
        };

        Self {
            left: glyphs.iter().map(|g| g.left).min().unwrap_or(0),
            top: glyphs.iter().map(|g| g.top).min().unwrap_or(0),
            right: glyphs.iter().map(|g| g.right).max().unwrap_or(0),
            bottom: glyphs.iter().map(|g| g.bottom).max().unwrap_or(0),
            thickness: median(&mut thicknesses),
            baseline,
        }
    }
}

impl TypographyDetector {
    pub fn new(config: TypographyConfig) -> Self {
        Self { config }
    }

    fn accepts(&self, glyph: &Glyph) -> bool {
        let (w, h) = (glyph.width(), glyph.height());
        (self.config.min_glyph_height..=self.config.max_glyph_height).contains(&h)
            && w <= self.config.max_glyph_height
            && w <= 3 * h
    }

    /// Finds connected ink blobs and estimates each one's stroke thickness
    /// as twice its area divided by its outline length.
    fn find_glyphs(&self, ink: &[bool], width: usize, height: usize) -> Vec<Glyph> {
        let mut seen = vec![false; ink.len()];
        let mut glyphs = Vec::new();

        for start in 0..ink.len() {
            if !ink[start] || seen[start] {
                continue;
            }

            seen[start] = true;
            let mut stack = vec![start];
            let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
            let (mut area, mut edge) = (0usize, 0usize);

            while let Some(i) = stack.pop() {
                let (x, y) = (i % width, i / width);
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
                area += 1;

                let mut on_edge = false;
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let (nx, ny) = (x as isize + dx, y as isize + dy);
                        if !is_ink(ink, width, height, nx, ny) {
                            // Only the four direct neighbours define the outline.
                            on_edge |= dx == 0 || dy == 0;
                            continue;
                        }
                        let n = ny as usize * width + nx as usize;
                        if !seen[n] {
                            seen[n] = true;
                            stack.push(n);
                        }
                    }
                }
                if on_edge {
                    edge += 1;
                }
            }

            let glyph = Glyph {
                left: left as u32,
                top: top as u32,
                right: right as u32,
                bottom: bottom as u32,
                thickness: 2.0 * area as f32 / edge.max(1) as f32,
            };
            if self.accepts(&glyph) {
                glyphs.push(glyph);
            }
        }
        glyphs
    }

    fn split_words(&self, glyphs: &[Glyph], line_height: f32) -> Vec<Word> {
        let mut ordered = glyphs.to_vec();
        ordered.sort_by_key(|g| g.left);
        let gap_limit = line_height * 0.5;

        let mut groups: Vec<Vec<Glyph>> = Vec::new();
        let mut reach = 0f32;
        for glyph in ordered {
            let starts_new = groups.is_empty() || glyph.left as f32 - reach > gap_limit;
            if starts_new {
                groups.push(Vec::new());
                reach = glyph.right as f32;
            } else {
                reach = reach.max(glyph.right as f32);
            }
            if let Some(group) = groups.last_mut() {
                group.push(glyph);
            }
        }

        groups
            .into_iter()
            .filter(|g| g.len() >= self.config.min_word_glyphs)
            .map(Word::from_glyphs)
            .collect()
    }

    fn check_line(
        &self,
        document: &DocumentId,
        page: u32,
        glyphs: &[Glyph],
        width: u32,
        height: u32,
    ) -> Result<Vec<Finding>> {
        let mut heights: Vec<f32> = glyphs.iter().map(|g| g.height() as f32).collect();
        let line_height = median(&mut heights);

        let words = self.split_words(glyphs, line_height);
        if words.len() < self.config.min_words {
            return Ok(Vec::new());
        }

        let mut thicknesses: Vec<f32> = words.iter().map(|w| w.thickness).collect();
        let mut baselines: Vec<f32> = words.iter().map(|w| w.baseline).collect();
        let typical_thickness = median(&mut thicknesses);
        let typical_baseline = median(&mut baselines);
        let baseline_limit = (line_height * 0.25).max(self.config.min_baseline_shift);

        let mut findings = Vec::new();
        for word in &words {
            let mut reasons = Vec::new();

            if typical_thickness > 0.0 {
                let ratio = word.thickness / typical_thickness;
                let change = (word.thickness - typical_thickness).abs();
                let limit = self.config.thickness_ratio;
                if change >= 0.75 && (ratio >= limit || ratio <= 1.0 / limit) {
                    reasons.push(format!(
                        "stroke thickness is {ratio:.1}x that of the rest of the line"
                    ));
                }
            }

            let shift = word.baseline - typical_baseline;
            if shift.abs() >= baseline_limit {
                let direction = if shift > 0.0 { "lower" } else { "higher" };
                reasons.push(format!(
                    "the baseline sits {:.1} px {direction} than the rest of the line",
                    shift.abs()
                ));
            }

            if reasons.is_empty() {
                continue;
            }

            // Two independent differences are stronger evidence than one.
            let score = if reasons.len() > 1 { 0.55 } else { 0.35 };

            let pad = 2u32;
            let x = word.left.saturating_sub(pad);
            let y = word.top.saturating_sub(pad);
            let right = (word.right + pad).min(width);
            let bottom = (word.bottom + pad).min(height);
            let region = Rect::new(x as f32, y as f32, (right - x) as f32, (bottom - y) as f32);

            findings.push(Finding::new(
                document.clone(),
                page,
                region,
                DetectorKind::Typography,
                Status::PotentialManipulation,
                Confidence::new(score)?,
                format!(
                    "Text in this word differs from the rest of its line: {}. \
                     This can indicate characters added or replaced after the original was printed",
                    reasons.join("; ")
                ),
            ));
        }
        Ok(findings)
    }
}

impl Default for TypographyDetector {
    fn default() -> Self {
        Self::new(TypographyConfig::default())
    }
}

impl Detector for TypographyDetector {
    fn kind(&self) -> DetectorKind {
        DetectorKind::Typography
    }

    fn analyze(&self, document: &DocumentId, page: u32, image: &RgbImage) -> Result<Vec<Finding>> {
        let gray = image::imageops::grayscale(image);
        let (lowest, highest) = gray
            .pixels()
            .fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
        if highest.saturating_sub(lowest) < self.config.min_contrast {
            return Ok(Vec::new());
        }

        let cutoff = otsu_threshold(&gray);
        let ink: Vec<bool> = gray.pixels().map(|p| p[0] <= cutoff).collect();

        let (width, height) = image.dimensions();
        let glyphs = self.find_glyphs(&ink, width as usize, height as usize);

        let mut findings = Vec::new();
        for line in group_lines(glyphs) {
            findings.extend(self.check_line(document, page, &line.glyphs, width, height)?);
        }
        Ok(findings)
    }
}

fn is_ink(ink: &[bool], width: usize, height: usize, x: isize, y: isize) -> bool {
    x >= 0
        && y >= 0
        && (x as usize) < width
        && (y as usize) < height
        && ink[y as usize * width + x as usize]
}

/// Collects glyphs whose vertical extents overlap into text lines.
fn group_lines(mut glyphs: Vec<Glyph>) -> Vec<Line> {
    glyphs.sort_by_key(|g| g.top);

    let mut lines: Vec<Line> = Vec::new();
    for glyph in glyphs {
        match lines.iter().position(|line| line.overlaps(&glyph)) {
            Some(i) => {
                let line = &mut lines[i];
                line.top = line.top.min(glyph.top);
                line.bottom = line.bottom.max(glyph.bottom);
                line.glyphs.push(glyph);
            }
            None => lines.push(Line {
                top: glyph.top,
                bottom: glyph.bottom,
                glyphs: vec![glyph],
            }),
        }
    }
    lines
}

/// Picks the brightness level that best separates ink from paper.
fn otsu_threshold(gray: &GrayImage) -> u8 {
    let mut histogram = [0u64; 256];
    for p in gray.pixels() {
        histogram[p[0] as usize] += 1;
    }

    let total = u64::from(gray.width()) * u64::from(gray.height());
    let sum_all: f64 = histogram
        .iter()
        .enumerate()
        .map(|(level, &count)| level as f64 * count as f64)
        .sum();

    let (mut weight_bg, mut sum_bg) = (0u64, 0f64);
    let (mut best, mut threshold) = (0f64, 0u8);

    for (level, &count) in histogram.iter().enumerate() {
        weight_bg += count;
        if weight_bg == 0 {
            continue;
        }
        let weight_fg = total - weight_bg;
        if weight_fg == 0 {
            break;
        }

        sum_bg += level as f64 * count as f64;
        let mean_bg = sum_bg / weight_bg as f64;
        let mean_fg = (sum_all - sum_bg) / weight_fg as f64;
        let between = weight_bg as f64 * weight_fg as f64 * (mean_bg - mean_fg).powi(2);

        if between > best {
            best = between;
            threshold = level as u8;
        }
    }
    threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn id() -> DocumentId {
        DocumentId("test-doc".into())
    }

    /// Four vertical bars standing in for the letters of a word.
    fn draw_word(page: &mut RgbImage, x: u32, y: u32, bar_width: u32) {
        for letter in 0..4 {
            for yy in y..y + 14 {
                for xx in x + letter * 8..x + letter * 8 + bar_width {
                    page.put_pixel(xx, yy, Rgb([0, 0, 0]));
                }
            }
        }
    }

    /// One text line; each word is given as (top, bar width).
    fn line_page(words: &[(u32, u32)]) -> RgbImage {
        let mut page = RgbImage::from_pixel(360, 90, Rgb([255, 255, 255]));
        for (i, &(y, bar_width)) in words.iter().enumerate() {
            draw_word(&mut page, 10 + i as u32 * 70, y, bar_width);
        }
        page
    }

    #[test]
    fn blank_page_has_no_findings() {
        let page = RgbImage::from_pixel(200, 100, Rgb([230, 230, 230]));
        let findings = TypographyDetector::default()
            .analyze(&id(), 0, &page)
            .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn evenly_printed_line_has_no_findings() {
        let page = line_page(&[(30, 2); 5]);
        let findings = TypographyDetector::default()
            .analyze(&id(), 0, &page)
            .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn word_on_a_different_baseline_is_flagged() {
        let page = line_page(&[(30, 2), (30, 2), (34, 2), (30, 2), (30, 2)]);
        let findings = TypographyDetector::default()
            .analyze(&id(), 0, &page)
            .unwrap();
        assert_eq!(findings.len(), 1);

        let word = Rect::new(150.0, 34.0, 26.0, 14.0);
        assert!(findings[0].region.iou(&word) > 0.5);
        assert_eq!(findings[0].detector, DetectorKind::Typography);
        assert_eq!(findings[0].status, Status::PotentialManipulation);
    }

    #[test]
    fn word_with_heavier_strokes_is_flagged() {
        let page = line_page(&[(30, 2), (30, 2), (30, 5), (30, 2), (30, 2)]);
        let findings = TypographyDetector::default()
            .analyze(&id(), 0, &page)
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].summary.contains("stroke thickness"));
    }

    #[test]
    fn short_lines_are_not_compared() {
        let page = line_page(&[(30, 2), (36, 2)]);
        let findings = TypographyDetector::default()
            .analyze(&id(), 0, &page)
            .unwrap();
        assert!(findings.is_empty());
    }
}
