//! Builds a case one piece at a time and runs the analysis on it.

use crate::pipeline::{Analyzer, CaseReport, DocumentInput, PageInput};
use crate::{DocumentId, Error, OcrLine, Result};

/// Collects documents, their page images and the text read from them, then
/// analyses everything together. Hosts such as the browser and the phone
/// receive files piece by piece, which is the shape this type is built for.
#[derive(Default)]
pub struct CaseBuilder {
    documents: Vec<DocumentInput>,
}

impl CaseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Registers a document. `source` is the original file, kept so its
    /// hash can be recorded in the audit log.
    pub fn add_document(&mut self, id: &str, name: &str, source: &[u8]) -> Result<()> {
        if self.documents.iter().any(|d| d.id.0 == id) {
            return Err(Error::DuplicateDocument(id.to_string()));
        }

        self.documents.push(DocumentInput {
            id: DocumentId(id.to_string()),
            name: name.to_string(),
            source: source.to_vec(),
            pages: Vec::new(),
        });
        Ok(())
    }

    /// Adds a page image (PNG or JPEG) and the text lines read from it,
    /// given as JSON. Returns the zero-based page number.
    pub fn add_page(&mut self, id: &str, page_image: &[u8], lines_json: &str) -> Result<u32> {
        // Everything that can fail happens before the case is changed, so a
        // rejected page leaves the case exactly as it was.
        let image = image::load_from_memory(page_image)?.to_rgb8();
        let lines: Vec<OcrLine> = if lines_json.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(lines_json)?
        };

        let document = self
            .documents
            .iter_mut()
            .find(|d| d.id.0 == id)
            .ok_or_else(|| Error::UnknownDocument(id.to_string()))?;

        document.pages.push(PageInput { image, lines });
        Ok((document.pages.len() - 1) as u32)
    }

    pub fn analyze(&self) -> Result<CaseReport> {
        Analyzer::standard().analyze(&self.documents)
    }

    pub fn analyze_json(&self) -> Result<String> {
        self.analyze()?.to_json()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

    use super::*;

    fn png(size: u32) -> Vec<u8> {
        let page = DynamicImage::ImageRgb8(RgbImage::from_pixel(size, size, Rgb([240, 240, 240])));
        let mut bytes = Vec::new();
        page.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// One line of text, laid out left to right, as JSON.
    fn lines_json(text: &str) -> String {
        let mut x = 10.0f32;
        let words: Vec<_> = text
            .split_whitespace()
            .map(|t| {
                let width = t.chars().count() as f32 * 8.0;
                let word = serde_json::json!({
                    "text": t,
                    "region": { "x": x, "y": 20.0, "width": width, "height": 12.0 },
                    "confidence": 0.9,
                });
                x += width + 6.0;
                word
            })
            .collect();
        serde_json::json!([{ "words": words }]).to_string()
    }

    fn case_with(documents: &[(&str, &str)]) -> CaseBuilder {
        let mut case = CaseBuilder::new();
        for (id, text) in documents {
            case.add_document(id, id, id.as_bytes()).unwrap();
            case.add_page(id, &png(64), &lines_json(text)).unwrap();
        }
        case
    }

    #[test]
    fn an_empty_case_analyses_to_an_empty_report() {
        let report = CaseBuilder::new().analyze().unwrap();
        assert!(report.documents.is_empty());
        assert!(report.findings.is_empty());
    }

    #[test]
    fn pages_are_numbered_in_the_order_they_are_added() {
        let mut case = CaseBuilder::new();
        case.add_document("a", "a.png", b"bytes").unwrap();
        assert_eq!(case.add_page("a", &png(64), "[]").unwrap(), 0);
        assert_eq!(case.add_page("a", &png(64), "").unwrap(), 1);

        let report = case.analyze().unwrap();
        assert_eq!(report.documents[0].pages, 2);
        assert_eq!(case.document_count(), 1);
    }

    #[test]
    fn text_lines_become_fields() {
        let report = case_with(&[("id.png", "Date of Birth: 04/12/1990")])
            .analyze()
            .unwrap();
        assert_eq!(report.fields.len(), 1);
        assert_eq!(report.fields[0].value, "1990-12-04");
    }

    #[test]
    fn conflicting_documents_produce_findings() {
        let report = case_with(&[
            ("id.png", "Date of Birth: 04/12/1990"),
            ("form.png", "Date of Birth: 14/12/1990"),
        ])
        .analyze()
        .unwrap();
        assert_eq!(report.findings.len(), 2);
    }

    #[test]
    fn a_page_for_an_unknown_document_is_rejected() {
        let mut case = CaseBuilder::new();
        let result = case.add_page("missing", &png(8), "[]");
        assert!(matches!(result, Err(Error::UnknownDocument(id)) if id == "missing"));
    }

    #[test]
    fn a_document_id_cannot_be_used_twice() {
        let mut case = CaseBuilder::new();
        case.add_document("a", "a.png", b"x").unwrap();
        let result = case.add_document("a", "other.png", b"y");
        assert!(matches!(result, Err(Error::DuplicateDocument(_))));
    }

    #[test]
    fn bytes_that_are_not_an_image_are_rejected() {
        let mut case = CaseBuilder::new();
        case.add_document("a", "a.png", b"x").unwrap();
        let result = case.add_page("a", b"not an image", "[]");
        assert!(matches!(result, Err(Error::Image(_))));
    }

    #[test]
    fn a_rejected_page_leaves_the_case_unchanged() {
        let mut case = CaseBuilder::new();
        case.add_document("a", "a.png", b"x").unwrap();

        let result = case.add_page("a", &png(64), "{ not json");
        assert!(matches!(result, Err(Error::Json(_))));
        assert_eq!(case.analyze().unwrap().documents[0].pages, 0);
    }

    #[test]
    fn the_report_can_be_returned_as_json() {
        let json = case_with(&[("a.png", "Name: Rahul Sharma")])
            .analyze_json()
            .unwrap();
        assert!(json.contains("\"overview\""));
        assert!(json.contains("\"audit\""));
    }
}
