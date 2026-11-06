use image::RgbImage;

use super::blocks::find_clusters;
use super::Detector;
use crate::{Confidence, DetectorKind, DocumentId, Finding, Rect, Result, Status};

/// Longest break, in pixels, a pen line may have and still count as one line.
const MAX_RUN_GAP: usize = 3;
/// A line longer than this share of the page width is a rule, not a pen stroke.
const MAX_LINE_SHARE: f32 = 0.6;
/// Handwriting is small. Larger coloured shapes are stamps, logos or artwork.
const MAX_MARK_HEIGHT_SHARE: f32 = 0.06;
const MAX_MARK_WIDTH_SHARE: f32 = 0.4;
/// Pen pixels a block needs before it counts as marked.
const MIN_BLOCK_PIXELS: u32 = 4;

const CORRECTION_CONFIDENCE: f32 = 0.75;
const STRIKE_CONFIDENCE: f32 = 0.6;
const MARKS_CONFIDENCE: f32 = 0.4;

#[derive(Debug, Clone)]
pub struct InkMarksConfig {
    /// Smallest gap between the strongest and weakest colour channel for pen ink.
    pub min_chroma: i32,
    /// Pen ink is dark. Brighter coloured pixels (highlighter, tints) are ignored.
    pub max_pen_luma: i32,
    /// Pixels darker than this count as printed or written ink.
    pub dark_luma: i32,
    /// Shortest pen line worth considering, in pixels on a 1000 px wide page.
    pub min_line: f32,
    /// Thickest pen line, on the same scale.
    pub max_thickness: f32,
    /// How far above and below a line to look for the text it crosses.
    pub reach: f32,
    /// Share of a line's length that must have text on both sides.
    pub min_crossing: f32,
    /// Fewest pen pixels a page needs before it is analysed.
    pub min_pen_pixels: usize,
    /// Least ink in a group of marks worth reporting.
    pub min_mark_pixels: u32,
}

impl Default for InkMarksConfig {
    fn default() -> Self {
        Self {
            min_chroma: 50,
            max_pen_luma: 120,
            dark_luma: 140,
            min_line: 26.0,
            max_thickness: 14.0,
            reach: 6.0,
            min_crossing: 0.2,
            min_pen_pixels: 60,
            min_mark_pixels: 60,
        }
    }
}

pub struct InkMarksDetector {
    config: InkMarksConfig,
}

/// Sizes that depend on the resolution of the page.
struct Limits {
    min_line: usize,
    max_thickness: usize,
    reach: usize,
    block: usize,
}

/// A rectangular part of the page. Right and bottom are exclusive.
#[derive(Debug, Clone, Copy)]
struct Part {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl Part {
    fn width(&self) -> usize {
        self.right - self.left
    }

    fn height(&self) -> usize {
        self.bottom - self.top
    }

    fn rect(&self) -> Rect {
        Rect::new(
            self.left as f32,
            self.top as f32,
            self.width() as f32,
            self.height() as f32,
        )
    }
}

impl InkMarksDetector {
    pub fn new(config: InkMarksConfig) -> Self {
        Self { config }
    }

    /// Two masks with one entry per pixel: pen-coloured ink, and any dark ink.
    fn classify(&self, image: &RgbImage) -> (Vec<bool>, Vec<bool>) {
        let count = (image.width() * image.height()) as usize;
        let mut pen = Vec::with_capacity(count);
        let mut dark = Vec::with_capacity(count);

        for pixel in image.pixels() {
            let [r, g, b] = pixel.0.map(i32::from);
            let spread = r.max(g).max(b) - r.min(g).min(b);
            let luma = (299 * r + 587 * g + 114 * b) / 1000;

            pen.push(spread >= self.config.min_chroma && luma <= self.config.max_pen_luma);
            dark.push(luma < self.config.dark_luma);
        }
        (pen, dark)
    }

    /// Pen lines that cross printed text.
    fn find_strikes(
        &self,
        pen: &[bool],
        dark: &[bool],
        width: usize,
        height: usize,
        limits: &Limits,
    ) -> Vec<Part> {
        let lines = long_runs(pen, width, height, limits.min_line, MAX_RUN_GAP);
        let longest = (width as f32 * MAX_LINE_SHARE) as usize;

        components(&lines, width, height)
            .into_iter()
            .filter(|part| {
                (limits.min_line..=longest).contains(&part.width())
                    && part.height() <= limits.max_thickness
            })
            .filter(|part| {
                crossing_share(dark, width, height, part, limits.reach) >= self.config.min_crossing
            })
            .collect()
    }

    /// Groups of pen marks, such as handwriting, once strike lines are removed.
    fn find_marks(
        &self,
        pen: &[bool],
        strikes: &[Part],
        width: usize,
        height: usize,
        limits: &Limits,
    ) -> Vec<Part> {
        // A strike line is already reported, so clear it away, with a small
        // margin for its soft edges, before looking for writing.
        let mut remaining = pen.to_vec();
        for strike in strikes {
            let top = strike.top.saturating_sub(2);
            let bottom = (strike.bottom + 2).min(height);
            for y in top..bottom {
                remaining[y * width + strike.left..y * width + strike.right].fill(false);
            }
        }

        let block = limits.block;
        let cols = width.div_ceil(block);
        let rows = height.div_ceil(block);
        let mut counts = vec![0u32; cols * rows];
        for (index, &on) in remaining.iter().enumerate() {
            if on {
                counts[(index / width / block) * cols + (index % width) / block] += 1;
            }
        }
        let flagged: Vec<bool> = counts
            .iter()
            .map(|&count| count >= MIN_BLOCK_PIXELS)
            .collect();

        // Letters and words of one piece of writing sit a few blocks apart, so
        // marked blocks are joined across small gaps before grouping.
        let mut near = vec![false; flagged.len()];
        for (index, _) in flagged.iter().enumerate().filter(|pair| *pair.1) {
            let (col, row) = (index % cols, index / cols);
            for r in row.saturating_sub(1)..=(row + 1).min(rows - 1) {
                for c in col.saturating_sub(2)..=(col + 2).min(cols - 1) {
                    near[r * cols + c] = true;
                }
            }
        }

        let mut marks = Vec::new();
        for cluster in find_clusters(&near, cols, rows) {
            let mut bounds: Option<(usize, usize, usize, usize)> = None;
            let mut total = 0u32;

            // Only blocks that really hold ink set the size, not the joining gaps.
            for &index in cluster.members.iter().filter(|&&i| flagged[i]) {
                let (col, row) = (index % cols, index / cols);
                total += counts[index];
                bounds = Some(match bounds {
                    None => (col, row, col, row),
                    Some((a, b, c, d)) => (a.min(col), b.min(row), c.max(col), d.max(row)),
                });
            }
            let Some((min_col, min_row, max_col, max_row)) = bounds else {
                continue;
            };

            let part = Part {
                left: min_col * block,
                top: min_row * block,
                right: ((max_col + 1) * block).min(width),
                bottom: ((max_row + 1) * block).min(height),
            };

            let too_big = part.height() as f32 > height as f32 * MAX_MARK_HEIGHT_SHARE
                || part.width() as f32 > width as f32 * MAX_MARK_WIDTH_SHARE;
            if total >= self.config.min_mark_pixels && !too_big {
                marks.push(part);
            }
        }
        marks
    }
}

impl Default for InkMarksDetector {
    fn default() -> Self {
        Self::new(InkMarksConfig::default())
    }
}

impl Detector for InkMarksDetector {
    fn kind(&self) -> DetectorKind {
        DetectorKind::InkMarks
    }

    fn analyze(&self, document: &DocumentId, page: u32, image: &RgbImage) -> Result<Vec<Finding>> {
        let (width, height) = (image.width() as usize, image.height() as usize);
        if width < 32 || height < 32 {
            return Ok(Vec::new());
        }

        let (pen, dark) = self.classify(image);
        let pen_pixels = pen.iter().filter(|&&on| on).count();
        let dark_pixels = dark.iter().filter(|&&on| on).count();

        // A page that is mostly colour, such as a designed layout, says nothing
        // about pen corrections made on top of printed text.
        if pen_pixels < self.config.min_pen_pixels || pen_pixels * 2 > dark_pixels {
            return Ok(Vec::new());
        }

        let scale = width as f32 / 1000.0;
        let scaled = |value: f32, floor: f32| (value * scale).max(floor);
        let reach = scaled(self.config.reach, 3.0).round() as usize;
        let limits = Limits {
            min_line: scaled(self.config.min_line, 16.0) as usize,
            max_thickness: scaled(self.config.max_thickness, 6.0) as usize,
            reach,
            block: reach.max(4),
        };

        let strikes = self.find_strikes(&pen, &dark, width, height, &limits);
        let marks = self.find_marks(&pen, &strikes, width, height, &limits);
        describe(document, page, &strikes, &marks, &limits, height)
    }
}

/// Turns strike lines and marks into findings, pairing each strike-through
/// with the writing beside it.
fn describe(
    document: &DocumentId,
    page: u32,
    strikes: &[Part],
    marks: &[Part],
    limits: &Limits,
    height: usize,
) -> Result<Vec<Finding>> {
    let reach = limits.reach;
    let mut used = vec![false; marks.len()];
    let mut findings = Vec::new();

    for strike in strikes {
        // The text that was crossed out is a little taller than the line.
        let top = strike.top.saturating_sub(2 * reach);
        let bottom = (strike.bottom + 2 * reach).min(height);
        let struck = Rect::new(
            strike.left as f32,
            top as f32,
            strike.width() as f32,
            (bottom - top) as f32,
        );

        let r = reach as f32;
        let line = strike.rect();
        let search = Rect::new(
            line.x - 2.0 * r,
            line.y - 5.0 * r,
            line.width + 4.0 * r,
            line.height + 10.0 * r,
        );
        let line_middle = line.y + line.height / 2.0;
        let distance = |mark: &Part| {
            let rect = mark.rect();
            (rect.y + rect.height / 2.0 - line_middle).abs()
        };

        let partner = marks
            .iter()
            .enumerate()
            .filter(|pair| pair.1.rect().intersection(&search).is_some())
            .min_by(|x, y| distance(x.1).total_cmp(&distance(y.1)))
            .map(|pair| pair.0);

        let (region, confidence, summary) = match partner {
            Some(index) => {
                used[index] = true;
                (
                    struck.union(&marks[index].rect()),
                    CORRECTION_CONFIDENCE,
                    "Printed text is struck through with a pen line and replacement text is \
                     written beside it, the pattern left when a value is overwritten by hand",
                )
            }
            None => (
                struck,
                STRIKE_CONFIDENCE,
                "Printed text is struck through with a pen line",
            ),
        };

        findings.push(Finding::new(
            document.clone(),
            page,
            region,
            DetectorKind::InkMarks,
            Status::Observed,
            Confidence::new(confidence)?,
            summary,
        ));
    }

    for (mark, _) in marks.iter().zip(&used).filter(|pair| !*pair.1) {
        findings.push(Finding::new(
            document.clone(),
            page,
            mark.rect(),
            DetectorKind::InkMarks,
            Status::Observed,
            Confidence::new(MARKS_CONFIDENCE)?,
            "Pen-coloured marks appear on a printed page, such as handwriting, a signature or a line",
        ));
    }
    Ok(findings)
}

/// Marks every pixel that belongs to a horizontal run of at least `min_len`
/// pixels, bridging breaks of up to `max_gap` pixels.
fn long_runs(
    mask: &[bool],
    width: usize,
    height: usize,
    min_len: usize,
    max_gap: usize,
) -> Vec<bool> {
    let mut out = vec![false; mask.len()];

    for y in 0..height {
        let row = &mask[y * width..(y + 1) * width];
        let marked = &mut out[y * width..(y + 1) * width];
        let mut run: Option<(usize, usize)> = None;

        for (x, &set) in row.iter().enumerate() {
            if !set {
                continue;
            }
            run = match run {
                Some((start, last)) if x - last - 1 <= max_gap => Some((start, x)),
                Some((start, last)) => {
                    keep_if_long(marked, start, last, min_len);
                    Some((x, x))
                }
                None => Some((x, x)),
            };
        }
        if let Some((start, last)) = run {
            keep_if_long(marked, start, last, min_len);
        }
    }
    out
}

fn keep_if_long(row: &mut [bool], start: usize, last: usize, min_len: usize) {
    if last + 1 - start >= min_len {
        row[start..=last].fill(true);
    }
}

/// Groups set pixels that touch, diagonals included, and returns each group's box.
fn components(mask: &[bool], width: usize, height: usize) -> Vec<Part> {
    let mut seen = vec![false; mask.len()];
    let mut found = Vec::new();

    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }

        seen[start] = true;
        let mut stack = vec![start];
        let mut part = Part {
            left: width,
            top: height,
            right: 0,
            bottom: 0,
        };

        while let Some(index) = stack.pop() {
            let (x, y) = (index % width, index / width);
            part.left = part.left.min(x);
            part.top = part.top.min(y);
            part.right = part.right.max(x + 1);
            part.bottom = part.bottom.max(y + 1);

            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let next = ny * width + nx;
                    if mask[next] && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        found.push(part);
    }
    found
}

/// Share of a line's length that has dark ink just above and just below it.
/// A strike-through cuts through letters, so both sides are inked. An
/// underline, a rule or a stamp border does not.
fn crossing_share(dark: &[bool], width: usize, height: usize, part: &Part, reach: usize) -> f32 {
    let above_from = part.top.saturating_sub(reach);
    let below_to = (part.bottom + reach).min(height);

    let mut crossing = 0usize;
    for x in part.left..part.right {
        let above = (above_from..part.top).any(|y| dark[y * width + x]);
        let below = (part.bottom..below_to).any(|y| dark[y * width + x]);
        if above && below {
            crossing += 1;
        }
    }
    crossing as f32 / part.width() as f32
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use image::Rgb;

    use super::*;

    const BLACK: [u8; 3] = [0, 0, 0];
    const RED: [u8; 3] = [190, 30, 40];
    const BLUE: [u8; 3] = [30, 40, 150];

    fn id() -> DocumentId {
        DocumentId("test-doc".into())
    }

    fn fill(page: &mut RgbImage, xs: Range<u32>, ys: Range<u32>, colour: [u8; 3]) {
        for y in ys {
            for x in xs.clone() {
                page.put_pixel(x, y, Rgb(colour));
            }
        }
    }

    /// Three rows of printed strokes, a stand-in for lines of text.
    fn printed_page() -> RgbImage {
        let mut page = RgbImage::from_pixel(1000, 800, Rgb([255, 255, 255]));
        for row in 0..3 {
            let top = 40 + row * 60;
            for stroke in 0..24 {
                let left = 10 + stroke * 8;
                fill(&mut page, left..left + 3, top..top + 20, BLACK);
            }
        }
        page
    }

    /// A red pen line through the middle of the second row of text.
    fn cross_out(page: &mut RgbImage) {
        fill(page, 5..210, 108..111, RED);
    }

    /// A red pen line underneath the second row of text.
    fn underline(page: &mut RgbImage) {
        fill(page, 5..210, 124..127, RED);
    }

    /// Blue strokes below the second row, standing in for handwriting.
    fn handwriting(page: &mut RgbImage) {
        for i in 0..10 {
            let left = 20 + i * 10;
            fill(page, left..left + 3, 124..146, BLUE);
        }
    }

    fn analyse(page: &RgbImage) -> Vec<Finding> {
        InkMarksDetector::default().analyze(&id(), 0, page).unwrap()
    }

    #[test]
    fn printed_text_alone_has_no_findings() {
        assert!(analyse(&printed_page()).is_empty());
    }

    #[test]
    fn a_pen_line_through_text_is_reported_as_struck_through() {
        let mut page = printed_page();
        cross_out(&mut page);

        let findings = analyse(&page);
        assert_eq!(findings.len(), 1);

        let found = &findings[0];
        assert_eq!(found.detector, DetectorKind::InkMarks);
        assert_eq!(found.status, Status::Observed);
        assert!((found.confidence.value() - STRIKE_CONFIDENCE).abs() < 1e-5);
        assert!(found.summary.contains("struck through"));
        assert!(found.region.iou(&Rect::new(5.0, 96.0, 205.0, 27.0)) > 0.9);
    }

    #[test]
    fn writing_beside_a_strike_is_reported_as_one_correction() {
        let mut page = printed_page();
        cross_out(&mut page);
        handwriting(&mut page);

        let findings = analyse(&page);
        assert_eq!(findings.len(), 1);

        let found = &findings[0];
        assert!((found.confidence.value() - CORRECTION_CONFIDENCE).abs() < 1e-5);
        assert!(found.summary.contains("replacement text"));
        // The region covers the struck text and the writing below it.
        assert!(found.region.y <= 96.0);
        assert!(found.region.y + found.region.height >= 144.0);
    }

    #[test]
    fn handwriting_alone_is_a_weak_observation() {
        let mut page = printed_page();
        handwriting(&mut page);

        let findings = analyse(&page);
        assert_eq!(findings.len(), 1);
        assert!((findings[0].confidence.value() - MARKS_CONFIDENCE).abs() < 1e-5);
        assert!(findings[0].summary.contains("Pen-coloured marks"));
    }

    #[test]
    fn an_underline_is_not_reported_as_a_strike() {
        let mut page = printed_page();
        underline(&mut page);

        let findings = analyse(&page);
        assert!(findings.iter().all(|f| !f.summary.contains("struck")));
    }

    #[test]
    fn a_large_stamp_is_neither_a_strike_nor_handwriting() {
        let mut page = printed_page();
        fill(&mut page, 400..600, 300..304, RED);
        fill(&mut page, 400..600, 376..380, RED);
        fill(&mut page, 400..404, 300..380, RED);
        fill(&mut page, 596..600, 300..380, RED);

        assert!(analyse(&page).is_empty());
    }

    #[test]
    fn a_mostly_coloured_page_is_not_analysed() {
        let mut page = printed_page();
        fill(&mut page, 300..600, 300..600, RED);
        assert!(analyse(&page).is_empty());
    }

    #[test]
    fn a_tiny_image_is_skipped() {
        let page = RgbImage::from_pixel(16, 16, Rgb(RED));
        assert!(analyse(&page).is_empty());
    }
}
