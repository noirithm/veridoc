use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hash::sha256_hex;
use crate::{Confidence, DetectorKind, DocumentId, Finding, Rect, Result, Status};

/// Share of the smaller region that must lie inside the other for two
/// findings to be treated as pointing at the same place.
const MIN_OVERLAP: f32 = 0.5;

/// Combined confidence never reaches certainty.
const MAX_CONFIDENCE: f32 = 0.95;

const HIGH_FROM: f32 = 0.7;
const MEDIUM_FROM: f32 = 0.45;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Priority {
    fn from_confidence(confidence: f32) -> Self {
        if confidence >= HIGH_FROM {
            Priority::High
        } else if confidence >= MEDIUM_FROM {
            Priority::Medium
        } else {
            Priority::Low
        }
    }
}

/// Everything found at one place on a page, merged into a single result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusedFinding {
    pub id: String,
    pub document: DocumentId,
    pub page: u32,
    /// Smallest rectangle covering every member's region.
    pub region: Rect,
    pub status: Status,
    pub confidence: Confidence,
    pub priority: Priority,
    /// The distinct checks that contributed, in a fixed order.
    pub detectors: Vec<DetectorKind>,
    pub reason: String,
    /// The original findings, kept so every result can be traced back.
    pub members: Vec<Finding>,
}

/// Merges findings that point at the same region into prioritised results.
/// The most confident results come first.
pub fn fuse(findings: &[Finding]) -> Result<Vec<FusedFinding>> {
    let mut groups = Groups::new(findings.len());
    for i in 0..findings.len() {
        for j in i + 1..findings.len() {
            if related(&findings[i], &findings[j]) {
                groups.union(i, j);
            }
        }
    }

    let mut by_root: BTreeMap<usize, Vec<&Finding>> = BTreeMap::new();
    for (i, finding) in findings.iter().enumerate() {
        by_root.entry(groups.find(i)).or_default().push(finding);
    }

    let mut fused = Vec::with_capacity(by_root.len());
    for members in by_root.values() {
        let Some(&first) = members.first() else {
            continue;
        };
        fused.push(fuse_group(first, members)?);
    }

    fused.sort_by(|a, b| {
        b.confidence
            .value()
            .total_cmp(&a.confidence.value())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(fused)
}

fn fuse_group(first: &Finding, members: &[&Finding]) -> Result<FusedFinding> {
    let region = members
        .iter()
        .skip(1)
        .fold(first.region, |acc, member| acc.union(&member.region));

    // Keep only the strongest score from each detector, so a check that
    // fires several times in one place is not counted as several checks.
    let mut best: Vec<(DetectorKind, f32)> = Vec::new();
    for member in members {
        let score = member.confidence.value();
        match best.iter().position(|(kind, _)| *kind == member.detector) {
            Some(i) => best[i].1 = best[i].1.max(score),
            None => best.push((member.detector, score)),
        }
    }

    // Noisy-OR: the chance that every check is wrong is the product of
    // each check being wrong, so agreement raises the result.
    let all_wrong: f32 = best.iter().map(|(_, score)| 1.0 - score).product();
    let combined = (1.0 - all_wrong).min(MAX_CONFIDENCE);

    let status = members
        .iter()
        .map(|member| member.status)
        .max_by_key(|status| status_rank(*status))
        .unwrap_or(first.status);

    let mut detectors: Vec<DetectorKind> = best.iter().map(|(kind, _)| *kind).collect();
    detectors.sort_by_key(|kind| *kind as u8);

    let names: Vec<&str> = detectors.iter().map(|kind| detector_label(*kind)).collect();
    let reason = match detectors.len() {
        1 => format!("Flagged by one check ({})", names.join(", ")),
        n => format!("Flagged by {n} separate checks ({})", names.join(", ")),
    };

    let mut ids: Vec<&str> = members.iter().map(|member| member.id.as_str()).collect();
    ids.sort_unstable();
    let id = sha256_hex(ids.join("|").as_bytes())[..12].to_string();

    Ok(FusedFinding {
        id,
        document: first.document.clone(),
        page: first.page,
        region,
        status,
        confidence: Confidence::new(combined)?,
        priority: Priority::from_confidence(combined),
        detectors,
        reason,
        members: members.iter().map(|&member| member.clone()).collect(),
    })
}

fn related(a: &Finding, b: &Finding) -> bool {
    if a.document != b.document || a.page != b.page {
        return false;
    }
    let Some(shared) = a.region.intersection(&b.region) else {
        return false;
    };
    let smaller = a.region.area().min(b.region.area());
    smaller > 0.0 && shared.area() / smaller >= MIN_OVERLAP
}

fn status_rank(status: Status) -> u8 {
    match status {
        Status::Observed => 0,
        Status::Inconsistent => 1,
        Status::PotentialManipulation => 2,
    }
}

pub(crate) fn detector_label(kind: DetectorKind) -> &'static str {
    match kind {
        DetectorKind::ErrorLevel => "error level analysis",
        DetectorKind::Noise => "noise analysis",
        DetectorKind::CopyMove => "copy-move check",
        DetectorKind::Typography => "typography check",
        DetectorKind::Signature => "signature check",
        DetectorKind::PdfStructure => "PDF structure check",
        DetectorKind::OcrConfidence => "OCR confidence check",
        DetectorKind::CrossDocument => "cross-document comparison",
        DetectorKind::InkMarks => "pen mark check",
    }
}

/// Union-find: tracks which findings belong to the same group.
struct Groups {
    parent: Vec<usize>,
}

impl Groups {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }

    fn find(&mut self, item: usize) -> usize {
        let mut root = item;
        while self.parent[root] != root {
            root = self.parent[root];
        }

        // Point everything on the path straight at the root, so later
        // lookups are faster.
        let mut current = item;
        while self.parent[current] != root {
            let next = self.parent[current];
            self.parent[current] = root;
            current = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (root_a, root_b) = (self.find(a), self.find(b));
        if root_a != root_b {
            self.parent[root_b] = root_a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(
        doc: &str,
        page: u32,
        detector: DetectorKind,
        status: Status,
        region: Rect,
        confidence: f32,
    ) -> Finding {
        Finding::new(
            DocumentId(doc.into()),
            page,
            region,
            detector,
            status,
            Confidence::new(confidence).unwrap(),
            "test finding",
        )
    }

    fn pixel(detector: DetectorKind, region: Rect, confidence: f32) -> Finding {
        finding(
            "doc",
            0,
            detector,
            Status::PotentialManipulation,
            region,
            confidence,
        )
    }

    #[test]
    fn a_lone_finding_keeps_its_confidence_and_status() {
        let out = fuse(&[pixel(
            DetectorKind::ErrorLevel,
            Rect::new(10.0, 10.0, 50.0, 20.0),
            0.4,
        )])
        .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, Status::PotentialManipulation);
        assert!((out[0].confidence.value() - 0.4).abs() < 1e-5);
        assert_eq!(out[0].detectors, vec![DetectorKind::ErrorLevel]);
        assert_eq!(out[0].priority, Priority::Low);
        assert_eq!(out[0].members.len(), 1);
    }

    #[test]
    fn agreeing_checks_raise_confidence() {
        let out = fuse(&[
            pixel(DetectorKind::Noise, Rect::new(20.0, 15.0, 100.0, 50.0), 0.4),
            pixel(
                DetectorKind::ErrorLevel,
                Rect::new(10.0, 10.0, 100.0, 50.0),
                0.4,
            ),
        ])
        .unwrap();

        assert_eq!(out.len(), 1);
        assert!((out[0].confidence.value() - 0.64).abs() < 1e-5);
        assert_eq!(out[0].priority, Priority::Medium);
        assert_eq!(
            out[0].detectors,
            vec![DetectorKind::ErrorLevel, DetectorKind::Noise]
        );
        assert_eq!(out[0].region, Rect::new(10.0, 10.0, 110.0, 55.0));
        assert_eq!(out[0].members.len(), 2);
        assert!(out[0].reason.contains("2 separate checks"));
    }

    #[test]
    fn one_detector_firing_twice_is_counted_once() {
        let out = fuse(&[
            pixel(
                DetectorKind::ErrorLevel,
                Rect::new(10.0, 10.0, 100.0, 50.0),
                0.4,
            ),
            pixel(
                DetectorKind::ErrorLevel,
                Rect::new(20.0, 15.0, 100.0, 50.0),
                0.3,
            ),
        ])
        .unwrap();

        assert_eq!(out.len(), 1);
        assert!((out[0].confidence.value() - 0.4).abs() < 1e-5);
        assert_eq!(out[0].detectors.len(), 1);
        assert_eq!(out[0].members.len(), 2);
    }

    #[test]
    fn distant_regions_stay_separate_and_the_strongest_comes_first() {
        let out = fuse(&[
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

        assert_eq!(out.len(), 2);
        assert!(out[0].confidence.value() > out[1].confidence.value());
        assert_eq!(out[0].detectors, vec![DetectorKind::Noise]);
    }

    #[test]
    fn findings_in_different_documents_never_merge() {
        let region = Rect::new(10.0, 10.0, 50.0, 20.0);
        let out = fuse(&[
            finding(
                "a.pdf",
                0,
                DetectorKind::ErrorLevel,
                Status::PotentialManipulation,
                region,
                0.4,
            ),
            finding(
                "b.pdf",
                0,
                DetectorKind::Noise,
                Status::PotentialManipulation,
                region,
                0.4,
            ),
        ])
        .unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn findings_on_different_pages_never_merge() {
        let region = Rect::new(10.0, 10.0, 50.0, 20.0);
        let out = fuse(&[
            finding(
                "a.pdf",
                0,
                DetectorKind::ErrorLevel,
                Status::PotentialManipulation,
                region,
                0.4,
            ),
            finding(
                "a.pdf",
                1,
                DetectorKind::Noise,
                Status::PotentialManipulation,
                region,
                0.4,
            ),
        ])
        .unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn grouping_follows_chains_of_overlap() {
        // The first and last regions do not touch each other, but both
        // overlap the middle one.
        let out = fuse(&[
            pixel(
                DetectorKind::ErrorLevel,
                Rect::new(0.0, 0.0, 40.0, 40.0),
                0.3,
            ),
            pixel(DetectorKind::Noise, Rect::new(15.0, 0.0, 40.0, 40.0), 0.3),
            pixel(
                DetectorKind::Typography,
                Rect::new(30.0, 0.0, 40.0, 40.0),
                0.3,
            ),
        ])
        .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].detectors.len(), 3);
    }

    #[test]
    fn confidence_is_capped_below_certainty() {
        let region = Rect::new(0.0, 0.0, 40.0, 40.0);
        let out = fuse(&[
            pixel(DetectorKind::ErrorLevel, region, 0.9),
            pixel(DetectorKind::Noise, region, 0.9),
            pixel(DetectorKind::CopyMove, region, 0.9),
        ])
        .unwrap();

        assert_eq!(out[0].confidence.value(), MAX_CONFIDENCE);
        assert_eq!(out[0].priority, Priority::High);
    }

    #[test]
    fn the_strongest_status_wins() {
        let region = Rect::new(0.0, 0.0, 40.0, 40.0);
        let out = fuse(&[
            finding(
                "doc",
                0,
                DetectorKind::CrossDocument,
                Status::Inconsistent,
                region,
                0.5,
            ),
            finding(
                "doc",
                0,
                DetectorKind::ErrorLevel,
                Status::PotentialManipulation,
                region,
                0.3,
            ),
        ])
        .unwrap();
        assert_eq!(out[0].status, Status::PotentialManipulation);
    }

    #[test]
    fn ids_do_not_depend_on_input_order() {
        let a = pixel(
            DetectorKind::ErrorLevel,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            0.4,
        );
        let b = pixel(DetectorKind::Noise, Rect::new(10.0, 0.0, 40.0, 40.0), 0.4);

        let forward = fuse(&[a.clone(), b.clone()]).unwrap();
        let backward = fuse(&[b, a]).unwrap();
        assert_eq!(forward[0].id, backward[0].id);
    }

    #[test]
    fn nothing_in_gives_nothing_out() {
        assert!(fuse(&[]).unwrap().is_empty());
    }
}
