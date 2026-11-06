use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::fusion::{detector_label, FusedFinding, Priority};
use crate::{DetectorKind, Finding, Status};

/// A plain-language account of one fused finding.
///
/// The three parts are kept apart on purpose: what was measured, what it
/// might mean, and what a person should check next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    pub headline: String,
    pub observed: Vec<String>,
    pub inferred: Vec<String>,
    pub to_verify: Vec<String>,
}

pub fn explain(finding: &FusedFinding) -> Explanation {
    Explanation {
        headline: headline(finding),
        observed: observations(finding),
        inferred: inferences(finding),
        to_verify: checks(finding),
    }
}

/// One sentence describing all findings of a case, for the top of a report.
pub fn overview(findings: &[FusedFinding]) -> String {
    if findings.is_empty() {
        return "No findings were recorded. This does not show the documents are genuine, \
                because the checks have known limits."
            .to_string();
    }

    let count = |priority: Priority| findings.iter().filter(|f| f.priority == priority).count();
    let documents: BTreeSet<&str> = findings.iter().map(|f| f.document.0.as_str()).collect();

    format!(
        "{} across {}: {} high, {} medium and {} low priority. \
         Each one is an indication that needs human review, not a confirmed conclusion.",
        counted(findings.len(), "finding"),
        counted(documents.len(), "document"),
        count(Priority::High),
        count(Priority::Medium),
        count(Priority::Low),
    )
}

fn headline(finding: &FusedFinding) -> String {
    let what = match finding.status {
        Status::Observed => "an observation worth noting",
        Status::Inconsistent => "records disagree",
        Status::PotentialManipulation => "possible alteration of this region",
    };
    format!(
        "{} priority: {} (confidence score {:.2})",
        priority_label(finding.priority),
        what,
        finding.confidence.value()
    )
}

/// The measurements, one per distinct detector message.
fn observations(finding: &FusedFinding) -> Vec<String> {
    let mut members: Vec<&Finding> = finding.members.iter().collect();
    members.sort_by(|a, b| {
        (a.detector as u8)
            .cmp(&(b.detector as u8))
            .then_with(|| a.summary.cmp(&b.summary))
    });
    members.dedup_by(|a, b| a.detector == b.detector && a.summary == b.summary);

    members
        .iter()
        .map(|member| {
            format!(
                "{}: {}",
                capitalize(detector_label(member.detector)),
                member.summary
            )
        })
        .collect()
}

fn inferences(finding: &FusedFinding) -> Vec<String> {
    let has = |kind: DetectorKind| finding.detectors.contains(&kind);
    let mut notes: Vec<String> = Vec::new();

    match finding.detectors.len() {
        1 => notes.push("Only one check flagged this, so the evidence is limited.".into()),
        n => notes.push(format!(
            "{n} independent checks point at the same place, which is stronger \
             than any one of them alone."
        )),
    }

    if has(DetectorKind::ErrorLevel) && has(DetectorKind::Noise) {
        notes.push(
            "Compression and pixel noise both differ from the surrounding page here. \
             That is consistent with a region that was added or retouched separately, \
             though scanning artefacts can produce a similar pattern."
                .into(),
        );
    } else if has(DetectorKind::ErrorLevel) {
        notes.push(
            "This region compresses differently from the rest of the page, which can \
             happen when it was saved separately from its surroundings."
                .into(),
        );
    } else if has(DetectorKind::Noise) {
        notes.push(
            "The pixel noise here differs from the rest of the page, which can happen \
             when content comes from a different source."
                .into(),
        );
    }

    if has(DetectorKind::CopyMove) {
        notes.push(
            "Two regions of the page are nearly identical, which can happen when content \
             is copied from one place to another. Stamps, logos and repeated form \
             elements can look the same way."
                .into(),
        );
    }
    if has(DetectorKind::Typography) {
        notes.push(
            "A word differs in stroke or position from the rest of its line, which can \
             happen when characters are added or replaced after printing. Deliberate \
             emphasis such as bold labels can look the same way."
                .into(),
        );
    }
    if has(DetectorKind::Signature) {
        notes.push(
            "The signature area has characteristics that differ from what is expected \
             for its placement or appearance."
                .into(),
        );
    }
    if has(DetectorKind::PdfStructure) {
        notes.push(
            "The file's internal structure suggests it was changed after it was first \
             created."
                .into(),
        );
    }
    if has(DetectorKind::OcrConfidence) {
        notes.push(
            "Text here was hard to read, which can happen when it has been overwritten or \
             the scan quality is poor."
                .into(),
        );
    }
    if has(DetectorKind::CrossDocument) {
        notes.push(
            "The same information is recorded differently in another document. That could \
             be a data-entry mistake, a document belonging to someone else, or an edited \
             value. This check cannot tell which."
                .into(),
        );
    }
    if has(DetectorKind::InkMarks) {
        notes.push(
            "Pen marks are visible on the printed page. Hand corrections can be legitimate \
             when initialled or stamped by the issuer, but they also change what the \
             document says, and nothing in the file shows who made them or when."
                .into(),
        );
    }
    notes.push("This is a statistical indication, not proof of tampering.".into());
    notes
}

fn checks(finding: &FusedFinding) -> Vec<String> {
    let has = |kind: DetectorKind| finding.detectors.contains(&kind);
    let mut steps: Vec<String> = Vec::new();

    let pixel_based = has(DetectorKind::ErrorLevel) || has(DetectorKind::Noise);
    if pixel_based {
        add_unique(
            &mut steps,
            "Inspect the highlighted region at full resolution on the original file.",
        );
        add_unique(
            &mut steps,
            "Request the original file or a fresh scan to compare against.",
        );
    }
    if has(DetectorKind::CopyMove) {
        add_unique(&mut steps, "Check whether the matching regions are legitimately repeated elements such as a stamp or logo.");
    }
    if has(DetectorKind::Typography) {
        add_unique(
            &mut steps,
            "Compare the highlighted word with the rest of its line at full resolution.",
        );
    }
    if has(DetectorKind::Signature) {
        add_unique(&mut steps, "Compare the signature with a known specimen.");
    }
    if has(DetectorKind::PdfStructure) {
        add_unique(
            &mut steps,
            "Request the original source file from the issuer.",
        );
    }
    if has(DetectorKind::OcrConfidence) {
        add_unique(&mut steps, "Read the text directly from the original scan instead of relying on the extracted value.");
    }
    if has(DetectorKind::CrossDocument) {
        add_unique(
            &mut steps,
            "Read both values directly from the original scans to rule out a misread.",
        );
        add_unique(
            &mut steps,
            "Ask the applicant to confirm the correct value and provide a supporting document.",
        );
    }
    if has(DetectorKind::InkMarks) {
        add_unique(
            &mut steps,
            "Check whether the correction is initialled, dated or stamped by the issuer.",
        );
        add_unique(&mut steps, "Compare with the issuer's own copy or a fresh statement to confirm the corrected values.");
    }

    if finding.priority == Priority::Low {
        add_unique(
            &mut steps,
            "Review only if other findings on the same document add support.",
        );
    }
    if steps.is_empty() {
        add_unique(
            &mut steps,
            "Compare with the original paper document if it is available.",
        );
    }
    steps
}

fn add_unique(list: &mut Vec<String>, text: &str) {
    if !list.iter().any(|existing| existing == text) {
        list.push(text.to_string());
    }
}

fn priority_label(priority: Priority) -> &'static str {
    match priority {
        Priority::High => "High",
        Priority::Medium => "Medium",
        Priority::Low => "Low",
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn counted(number: usize, noun: &str) -> String {
    if number == 1 {
        format!("{number} {noun}")
    } else {
        format!("{number} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fuse, Confidence, DocumentId, Rect};

    fn finding(detector: DetectorKind, status: Status, region: Rect, confidence: f32) -> Finding {
        Finding::new(
            DocumentId("doc".into()),
            0,
            region,
            detector,
            status,
            Confidence::new(confidence).unwrap(),
            "measured something",
        )
    }

    fn pixel(detector: DetectorKind, region: Rect, confidence: f32) -> Finding {
        finding(detector, Status::PotentialManipulation, region, confidence)
    }

    fn mentions(lines: &[String], text: &str) -> bool {
        lines.iter().any(|line| line.contains(text))
    }

    #[test]
    fn a_single_weak_check_is_described_as_limited_evidence() {
        let fused = fuse(&[pixel(
            DetectorKind::ErrorLevel,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            0.3,
        )])
        .unwrap();
        let text = explain(&fused[0]);

        assert_eq!(
            text.headline,
            "Low priority: possible alteration of this region (confidence score 0.30)"
        );
        assert_eq!(
            text.observed,
            vec!["Error level analysis: measured something"]
        );
        assert!(mentions(&text.inferred, "Only one check"));
        assert!(mentions(&text.to_verify, "Review only if other findings"));
    }

    #[test]
    fn agreeing_pixel_checks_are_explained_together() {
        let fused = fuse(&[
            pixel(
                DetectorKind::ErrorLevel,
                Rect::new(0.0, 0.0, 40.0, 40.0),
                0.4,
            ),
            pixel(DetectorKind::Noise, Rect::new(10.0, 0.0, 40.0, 40.0), 0.4),
        ])
        .unwrap();
        let text = explain(&fused[0]);

        assert_eq!(text.observed.len(), 2);
        assert!(mentions(&text.inferred, "2 independent checks"));
        assert!(mentions(
            &text.inferred,
            "Compression and pixel noise both differ"
        ));
        assert!(!mentions(&text.to_verify, "Review only if"));
    }

    #[test]
    fn repeated_identical_observations_are_listed_once() {
        let region = Rect::new(0.0, 0.0, 40.0, 40.0);
        let one = pixel(DetectorKind::ErrorLevel, region, 0.4);
        let fused = fuse(&[one.clone(), one]).unwrap();
        assert_eq!(explain(&fused[0]).observed.len(), 1);
    }

    #[test]
    fn conflicting_records_suggest_reading_the_originals() {
        let fused = fuse(&[finding(
            DetectorKind::CrossDocument,
            Status::Inconsistent,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            0.8,
        )])
        .unwrap();
        let text = explain(&fused[0]);

        assert!(text.headline.contains("records disagree"));
        assert!(mentions(&text.inferred, "data-entry mistake"));
        assert!(mentions(&text.to_verify, "rule out a misread"));
    }

    #[test]
    fn every_explanation_says_it_is_not_proof() {
        let fused = fuse(&[
            pixel(
                DetectorKind::CopyMove,
                Rect::new(0.0, 0.0, 40.0, 40.0),
                0.85,
            ),
            pixel(
                DetectorKind::Typography,
                Rect::new(200.0, 0.0, 40.0, 40.0),
                0.55,
            ),
        ])
        .unwrap();

        for result in &fused {
            let text = explain(result);
            assert!(text.inferred.last().unwrap().contains("not proof"));
        }
    }

    #[test]
    fn wording_never_accuses_anyone() {
        let fused = fuse(&[
            pixel(
                DetectorKind::ErrorLevel,
                Rect::new(0.0, 0.0, 40.0, 40.0),
                0.4,
            ),
            pixel(
                DetectorKind::CopyMove,
                Rect::new(10.0, 0.0, 40.0, 40.0),
                0.8,
            ),
            finding(
                DetectorKind::CrossDocument,
                Status::Inconsistent,
                Rect::new(10.0, 10.0, 40.0, 40.0),
                0.7,
            ),
        ])
        .unwrap();

        let text = explain(&fused[0]);
        let everything = format!(
            "{} {} {} {}",
            text.headline,
            text.observed.join(" "),
            text.inferred.join(" "),
            text.to_verify.join(" ")
        )
        .to_lowercase();

        assert!(!everything.contains("fraud"));
        assert!(!everything.contains("forged"));
    }

    #[test]
    fn overview_counts_findings_by_priority() {
        let fused = fuse(&[
            pixel(
                DetectorKind::ErrorLevel,
                Rect::new(0.0, 0.0, 30.0, 30.0),
                0.3,
            ),
            pixel(
                DetectorKind::Noise,
                Rect::new(200.0, 200.0, 30.0, 30.0),
                0.6,
            ),
        ])
        .unwrap();

        assert!(overview(&fused)
            .starts_with("2 findings across 1 document: 0 high, 1 medium and 1 low priority."));
    }

    #[test]
    fn overview_of_an_empty_case_does_not_claim_the_documents_are_genuine() {
        let text = overview(&[]);
        assert!(text.contains("No findings"));
        assert!(text.contains("does not show"));
    }
}
