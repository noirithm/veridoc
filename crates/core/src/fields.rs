use std::ops::Range;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::hash::sha256_hex;
use crate::ocr::{OcrLine, OcrWord};
use crate::{Confidence, DocumentId, Rect};

const DATE: &str = r"[0-9]{1,2}[/.\-][0-9]{1,2}[/.\-][0-9]{4}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Name,
    DateOfBirth,
    Date,
    Amount,
    PanNumber,
    AadhaarNumber,
    Phone,
    Email,
}

/// A value read from a document, tied to the place it was read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub id: String,
    pub document: DocumentId,
    pub page: u32,
    pub kind: FieldKind,
    /// The text exactly as it was recognised.
    pub raw: String,
    /// A cleaned-up form that can be compared across documents.
    pub value: String,
    pub region: Rect,
    /// Confidence of the least certain word the field was built from.
    pub confidence: Confidence,
}

impl Field {
    pub(crate) fn new(
        document: DocumentId,
        page: u32,
        kind: FieldKind,
        raw: String,
        region: Rect,
        confidence: Confidence,
    ) -> Self {
        let key = format!(
            "{}|{}|{:?}|{}|{}|{}|{}",
            document.0,
            page,
            kind,
            region.x.to_bits(),
            region.y.to_bits(),
            region.width.to_bits(),
            region.height.to_bits(),
        );
        let id = sha256_hex(key.as_bytes())[..12].to_string();
        let value = normalize(kind, &raw);

        Self {
            id,
            document,
            page,
            kind,
            raw,
            value,
            region,
            confidence,
        }
    }
}

struct Rule {
    kind: FieldKind,
    pattern: Regex,
}

/// Finds known kinds of fields in recognised text. Every pattern names
/// the part to keep as `v`, so labels like "Name:" stay out of the value.
pub struct FieldExtractor {
    rules: Vec<Rule>,
}

impl FieldExtractor {
    pub fn new() -> Self {
        // Order matters: when two rules match the same text, the earlier
        // one wins, so a date of birth is not also reported as a plain date.
        let patterns: [(FieldKind, String); 8] = [
            (
                FieldKind::Name,
                r"(?i)^(?:applicant(?:'s)?\s+|full\s+|account\s+holder\s+)?name\s*[:\-]\s*(?P<v>.+?)\s*$"
                    .to_string(),
            ),
            (
                FieldKind::DateOfBirth,
                format!(r"(?i)\b(?:date\s+of\s+birth|dob|d\.o\.b\.?)\s*[:\-]?\s*(?P<v>{DATE})"),
            ),
            (
                FieldKind::PanNumber,
                r"\b(?P<v>[A-Z]{5}[0-9]{4}[A-Z])\b".to_string(),
            ),
            (
                FieldKind::AadhaarNumber,
                r"\b(?P<v>[0-9]{4}\s[0-9]{4}\s[0-9]{4})\b".to_string(),
            ),
            (
                FieldKind::Email,
                r"(?P<v>[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,})".to_string(),
            ),
            (
                FieldKind::Phone,
                r"\b(?P<v>[6-9][0-9]{9})\b".to_string(),
            ),
            (
                FieldKind::Amount,
                r"(?i)(?:₹|\brs\.?|\binr)\s*(?P<v>[0-9][0-9,]*(?:\.[0-9]{1,2})?)".to_string(),
            ),
            (FieldKind::Date, format!(r"\b(?P<v>{DATE})\b")),
        ];

        let rules = patterns
            .into_iter()
            .map(|(kind, pattern)| Rule {
                kind,
                pattern: Regex::new(&pattern).expect("built-in pattern is valid"),
            })
            .collect();
        Self { rules }
    }

    pub fn extract(&self, document: &DocumentId, page: u32, lines: &[OcrLine]) -> Vec<Field> {
        let mut fields = Vec::new();
        for line in lines {
            self.extract_line(document, page, line, &mut fields);
        }
        fields
    }

    fn extract_line(&self, document: &DocumentId, page: u32, line: &OcrLine, out: &mut Vec<Field>) {
        let (text, spans) = join_words(line);
        let mut taken: Vec<Range<usize>> = Vec::new();

        for rule in &self.rules {
            for caps in rule.pattern.captures_iter(&text) {
                let Some(found) = caps.name("v") else {
                    continue;
                };
                let range = found.start()..found.end();
                if taken.iter().any(|t| overlaps(t, &range)) {
                    continue;
                }
                let Some((region, confidence)) = locate(line, &spans, &range) else {
                    continue;
                };

                taken.push(range);
                out.push(Field::new(
                    document.clone(),
                    page,
                    rule.kind,
                    found.as_str().trim().to_string(),
                    region,
                    confidence,
                ));
            }
        }
    }
}

impl Default for FieldExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Joins the words with single spaces and remembers where each one sits
/// in the joined text, so a match can be traced back to its words.
fn join_words(line: &OcrLine) -> (String, Vec<Range<usize>>) {
    let mut text = String::new();
    let mut spans = Vec::with_capacity(line.words.len());

    for word in &line.words {
        if !text.is_empty() {
            text.push(' ');
        }
        let start = text.len();
        text.push_str(&word.text);
        spans.push(start..text.len());
    }
    (text, spans)
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Box and confidence of the words that a matched range touches.
fn locate(
    line: &OcrLine,
    spans: &[Range<usize>],
    range: &Range<usize>,
) -> Option<(Rect, Confidence)> {
    let hits: Vec<&OcrWord> = line
        .words
        .iter()
        .zip(spans)
        .filter(|(_, span)| overlaps(span, range))
        .map(|(word, _)| word)
        .collect();

    let first = hits.first()?;
    let region = hits
        .iter()
        .skip(1)
        .fold(first.region, |acc, word| acc.union(&word.region));
    let confidence = hits
        .iter()
        .map(|word| word.confidence)
        .min_by(|a, b| a.value().total_cmp(&b.value()))?;

    Some((region, confidence))
}

fn normalize(kind: FieldKind, raw: &str) -> String {
    match kind {
        FieldKind::Name => raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_uppercase(),
        FieldKind::DateOfBirth | FieldKind::Date => {
            normalize_date(raw).unwrap_or_else(|| raw.to_string())
        }
        FieldKind::Amount => raw.replace(',', ""),
        FieldKind::PanNumber => raw.to_uppercase(),
        FieldKind::AadhaarNumber => digits(raw),
        FieldKind::Phone => {
            let all = digits(raw);
            all[all.len().saturating_sub(10)..].to_string()
        }
        FieldKind::Email => raw.to_lowercase(),
    }
}

fn digits(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_digit).collect()
}

/// Reads day-first dates (31/12/1990) and returns them as 1990-12-31.
/// An impossible date is returned as `None` so the caller keeps the raw text.
fn normalize_date(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.split(['/', '-', '.']).collect();
    if parts.len() != 3 {
        return None;
    }

    let day: u32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let year: u32 = parts[2].parse().ok()?;

    let valid =
        (1..=31).contains(&day) && (1..=12).contains(&month) && (1900..=2100).contains(&year);
    valid.then(|| format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> DocumentId {
        DocumentId("test-doc".into())
    }

    fn word(text: &str, x: f32, confidence: f32) -> OcrWord {
        OcrWord {
            text: text.into(),
            region: Rect::new(x, 20.0, 40.0, 12.0),
            confidence: Confidence::new(confidence).unwrap(),
        }
    }

    /// Lays the words of `text` out left to right, 8 px per character.
    fn line_of(text: &str) -> OcrLine {
        let mut x = 10.0;
        let words = text
            .split_whitespace()
            .map(|t| {
                let width = t.chars().count() as f32 * 8.0;
                let w = OcrWord {
                    text: t.into(),
                    region: Rect::new(x, 20.0, width, 12.0),
                    confidence: Confidence::new(0.9).unwrap(),
                };
                x += width + 6.0;
                w
            })
            .collect();
        OcrLine { words }
    }

    fn extract(text: &str) -> Vec<Field> {
        FieldExtractor::new().extract(&id(), 0, &[line_of(text)])
    }

    #[test]
    fn labelled_name_keeps_only_the_value() {
        let fields = extract("Name: Rahul Kumar Sharma");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].kind, FieldKind::Name);
        assert_eq!(fields[0].value, "RAHUL KUMAR SHARMA");
        assert_eq!(fields[0].region.x, 56.0);
    }

    #[test]
    fn fathers_name_is_not_the_applicant_name() {
        assert!(extract("Father's Name: Mohan Sharma").is_empty());
    }

    #[test]
    fn date_of_birth_is_normalised_and_not_repeated() {
        let fields = extract("Date of Birth: 04/12/1990");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].kind, FieldKind::DateOfBirth);
        assert_eq!(fields[0].value, "1990-12-04");
    }

    #[test]
    fn impossible_dates_keep_their_raw_text() {
        let fields = extract("Issued 45/13/2020");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].kind, FieldKind::Date);
        assert_eq!(fields[0].value, "45/13/2020");
    }

    #[test]
    fn pan_and_amount_are_recognised() {
        let pan = extract("PAN: ABCDE1234F");
        assert_eq!(pan[0].kind, FieldKind::PanNumber);
        assert_eq!(pan[0].value, "ABCDE1234F");

        let amount = extract("Amount Rs. 1,25,000.50");
        assert_eq!(amount[0].kind, FieldKind::Amount);
        assert_eq!(amount[0].value, "125000.50");
    }

    #[test]
    fn aadhaar_spans_all_three_groups() {
        let fields = extract("Aadhaar: 1234 5678 9012");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].kind, FieldKind::AadhaarNumber);
        assert_eq!(fields[0].value, "123456789012");
        assert_eq!(fields[0].region.x, 80.0);
        assert_eq!(fields[0].region.width, 108.0);
    }

    #[test]
    fn confidence_is_that_of_the_weakest_word() {
        let line = OcrLine {
            words: vec![
                word("Name:", 10.0, 0.95),
                word("Rahul", 56.0, 0.9),
                word("Sharma", 100.0, 0.4),
            ],
        };
        let fields = FieldExtractor::new().extract(&id(), 0, &[line]);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].confidence.value(), 0.4);
    }
}
