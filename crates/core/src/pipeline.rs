use image::RgbImage;
use serde::{Deserialize, Serialize};

use crate::audit::AuditLog;
use crate::correlation::{correlate, CaseGraph};
use crate::detectors::{
    CopyMoveDetector, Detector, ElaDetector, NoiseDetector, TypographyDetector,
};
use crate::explain::{explain, overview, Explanation};
use crate::fusion::{detector_label, fuse, FusedFinding};
use crate::hash::sha256_hex;
use crate::{DocumentId, Field, FieldExtractor, Finding, OcrLine, Result};

/// One page image together with the text lines read from it.
pub struct PageInput {
    pub image: RgbImage,
    pub lines: Vec<OcrLine>,
}

/// A submitted document. `source` holds the original file bytes, which
/// are hashed so the audit log can show what was received.
pub struct DocumentInput {
    pub id: DocumentId,
    pub name: String,
    pub source: Vec<u8>,
    pub pages: Vec<PageInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: DocumentId,
    pub name: String,
    pub sha256: String,
    pub pages: u32,
    pub fields: usize,
}

/// A step that could not complete. The rest of the analysis carried on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub document: DocumentId,
    pub page: u32,
    pub step: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssessedFinding {
    pub finding: FusedFinding,
    pub explanation: Explanation,
}

/// Everything the analysis produced for one case.
#[derive(Debug, Serialize)]
pub struct CaseReport {
    pub documents: Vec<DocumentSummary>,
    pub fields: Vec<Field>,
    pub graph: CaseGraph,
    pub findings: Vec<AssessedFinding>,
    pub overview: String,
    pub failures: Vec<Failure>,
    pub audit: AuditLog,
}

impl CaseReport {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub struct Analyzer {
    detectors: Vec<Box<dyn Detector>>,
    extractor: FieldExtractor,
}

impl Analyzer {
    pub fn new(detectors: Vec<Box<dyn Detector>>) -> Self {
        Self {
            detectors,
            extractor: FieldExtractor::new(),
        }
    }

    /// The detectors that ship with the engine, with their default settings.
    pub fn standard() -> Self {
        Self::new(vec![
            Box::new(ElaDetector::default()),
            Box::new(NoiseDetector::default()),
            Box::new(CopyMoveDetector::default()),
            Box::new(TypographyDetector::default()),
        ])
    }

    pub fn analyze(&self, documents: &[DocumentInput]) -> Result<CaseReport> {
        let mut audit = AuditLog::new();
        let mut summaries = Vec::with_capacity(documents.len());
        let mut failures = Vec::new();
        let mut signals: Vec<Finding> = Vec::new();
        let mut fields: Vec<Field> = Vec::new();

        for document in documents {
            let sha256 = sha256_hex(&document.source);
            audit.append(
                "pipeline",
                "document_received",
                format!(
                    "{} ({}) sha256={} pages={}",
                    document.name,
                    document.id.0,
                    sha256,
                    document.pages.len()
                ),
            );

            let fields_before = fields.len();
            for (index, page) in document.pages.iter().enumerate() {
                let number = index as u32;

                for detector in &self.detectors {
                    let label = detector_label(detector.kind());
                    match detector.analyze(&document.id, number, &page.image) {
                        Ok(found) => {
                            audit.append(
                                "pipeline",
                                "detector_completed",
                                format!(
                                    "{label} on {} page {}: {} finding(s)",
                                    document.id.0,
                                    number + 1,
                                    found.len()
                                ),
                            );
                            signals.extend(found);
                        }
                        Err(error) => {
                            audit.append(
                                "pipeline",
                                "detector_failed",
                                format!(
                                    "{label} on {} page {}: {error}",
                                    document.id.0,
                                    number + 1
                                ),
                            );
                            failures.push(Failure {
                                document: document.id.clone(),
                                page: number,
                                step: label.to_string(),
                                message: error.to_string(),
                            });
                        }
                    }
                }

                let found = self.extractor.extract(&document.id, number, &page.lines);
                audit.append(
                    "pipeline",
                    "fields_extracted",
                    format!(
                        "{} page {}: {} field(s)",
                        document.id.0,
                        number + 1,
                        found.len()
                    ),
                );
                fields.extend(found);
            }

            summaries.push(DocumentSummary {
                id: document.id.clone(),
                name: document.name.clone(),
                sha256,
                pages: document.pages.len() as u32,
                fields: fields.len() - fields_before,
            });
        }

        let correlation = correlate(&fields)?;
        audit.append(
            "pipeline",
            "correlation_completed",
            format!(
                "{} link(s), {} conflicting field(s)",
                correlation.graph.links.len(),
                correlation.findings.len()
            ),
        );
        signals.extend(correlation.findings);
        let graph = correlation.graph;

        let fused = fuse(&signals)?;
        audit.append(
            "pipeline",
            "fusion_completed",
            format!(
                "{} signal(s) merged into {} finding(s)",
                signals.len(),
                fused.len()
            ),
        );

        let summary = overview(&fused);
        let findings: Vec<AssessedFinding> = fused
            .into_iter()
            .map(|finding| {
                let explanation = explain(&finding);
                AssessedFinding {
                    finding,
                    explanation,
                }
            })
            .collect();

        audit.append(
            "pipeline",
            "analysis_completed",
            format!(
                "{} finding(s), {} failure(s)",
                findings.len(),
                failures.len()
            ),
        );

        Ok(CaseReport {
            documents: summaries,
            fields,
            graph,
            findings,
            overview: summary,
            failures,
            audit,
        })
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::standard()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Confidence, DetectorKind, Error, OcrWord, Rect, Status};

    /// Reports one fixed region on every page it sees.
    struct Stub {
        kind: DetectorKind,
    }

    impl Detector for Stub {
        fn kind(&self) -> DetectorKind {
            self.kind
        }

        fn analyze(
            &self,
            document: &DocumentId,
            page: u32,
            _image: &RgbImage,
        ) -> Result<Vec<Finding>> {
            Ok(vec![Finding::new(
                document.clone(),
                page,
                Rect::new(10.0, 10.0, 40.0, 20.0),
                self.kind,
                Status::PotentialManipulation,
                Confidence::new(0.5)?,
                "stub measurement",
            )])
        }
    }

    struct Failing;

    impl Detector for Failing {
        fn kind(&self) -> DetectorKind {
            DetectorKind::Noise
        }

        fn analyze(
            &self,
            _document: &DocumentId,
            _page: u32,
            _image: &RgbImage,
        ) -> Result<Vec<Finding>> {
            Err(Error::InvalidConfidence(2.0))
        }
    }

    fn line(text: &str) -> OcrLine {
        let mut x = 10.0;
        let words = text
            .split_whitespace()
            .map(|t| {
                let width = t.chars().count() as f32 * 8.0;
                let word = OcrWord {
                    text: t.into(),
                    region: Rect::new(x, 20.0, width, 12.0),
                    confidence: Confidence::new(0.9).unwrap(),
                };
                x += width + 6.0;
                word
            })
            .collect();
        OcrLine { words }
    }

    fn document(name: &str, lines: Vec<OcrLine>) -> DocumentInput {
        DocumentInput {
            id: DocumentId(name.into()),
            name: name.into(),
            source: name.as_bytes().to_vec(),
            pages: vec![PageInput {
                image: RgbImage::new(128, 128),
                lines,
            }],
        }
    }

    fn stub_analyzer() -> Analyzer {
        Analyzer::new(vec![Box::new(Stub {
            kind: DetectorKind::ErrorLevel,
        })])
    }

    #[test]
    fn an_empty_case_produces_an_empty_but_valid_report() {
        let report = Analyzer::standard().analyze(&[]).unwrap();
        assert!(report.findings.is_empty());
        assert!(report.overview.contains("No findings"));
        assert!(report.audit.verify().is_ok());
    }

    #[test]
    fn detector_findings_are_fused_and_explained_for_every_page() {
        let mut doc = document("a.pdf", Vec::new());
        doc.pages.push(PageInput {
            image: RgbImage::new(128, 128),
            lines: Vec::new(),
        });

        let report = stub_analyzer().analyze(&[doc]).unwrap();
        assert_eq!(report.findings.len(), 2);

        let mut pages: Vec<u32> = report.findings.iter().map(|a| a.finding.page).collect();
        pages.sort_unstable();
        assert_eq!(pages, vec![0, 1]);

        for assessed in &report.findings {
            assert!(assessed
                .explanation
                .observed
                .iter()
                .any(|line| line.contains("stub measurement")));
        }

        assert_eq!(report.documents[0].pages, 2);
        assert_eq!(report.documents[0].sha256, sha256_hex(b"a.pdf"));
        assert!(report.audit.verify().is_ok());
    }

    #[test]
    fn a_failing_detector_is_recorded_and_the_others_still_run() {
        let analyzer = Analyzer::new(vec![
            Box::new(Failing),
            Box::new(Stub {
                kind: DetectorKind::ErrorLevel,
            }),
        ]);
        let report = analyzer.analyze(&[document("a.pdf", Vec::new())]).unwrap();

        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].step, "noise analysis");
        assert_eq!(report.failures[0].page, 0);
        assert_eq!(report.findings.len(), 1);
        assert!(report
            .audit
            .entries()
            .iter()
            .any(|entry| entry.action == "detector_failed"));
        assert!(report.audit.verify().is_ok());
    }

    #[test]
    fn conflicting_fields_across_documents_become_findings() {
        let analyzer = Analyzer::new(Vec::new());
        let report = analyzer
            .analyze(&[
                document("id.pdf", vec![line("Date of Birth: 04/12/1990")]),
                document("form.pdf", vec![line("Date of Birth: 14/12/1990")]),
            ])
            .unwrap();

        assert_eq!(report.fields.len(), 2);
        assert_eq!(report.graph.links.len(), 1);
        assert_eq!(report.findings.len(), 2);
        assert!(report
            .findings
            .iter()
            .all(|a| a.finding.status == Status::Inconsistent));
        assert_eq!(report.documents[0].fields, 1);
    }

    #[test]
    fn standard_detectors_find_nothing_on_a_blank_page() {
        let report = Analyzer::standard()
            .analyze(&[document("blank.pdf", Vec::new())])
            .unwrap();
        assert!(report.failures.is_empty());
        assert!(report.findings.is_empty());
    }

    #[test]
    fn finding_ids_are_the_same_on_every_run() {
        let documents = [document("a.pdf", Vec::new())];
        let first = stub_analyzer().analyze(&documents).unwrap();
        let second = stub_analyzer().analyze(&documents).unwrap();
        assert_eq!(first.findings[0].finding.id, second.findings[0].finding.id);
    }

    #[test]
    fn the_report_serialises_to_json() {
        let report = stub_analyzer()
            .analyze(&[document("a.pdf", Vec::new())])
            .unwrap();
        let json = report.to_json().unwrap();
        assert!(json.contains("\"overview\""));
        assert!(json.contains("\"audit\""));
    }
}
