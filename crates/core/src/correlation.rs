use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::matching::{compare_names, NameMatch};
use crate::{Confidence, DetectorKind, Field, FieldKind, Finding, Result, Status};

/// Kinds of fields that identify a person or a record. Dates and amounts
/// are left out because they legitimately differ between documents.
const COMPARED: [FieldKind; 6] = [
    FieldKind::Name,
    FieldKind::DateOfBirth,
    FieldKind::PanNumber,
    FieldKind::AadhaarNumber,
    FieldKind::Phone,
    FieldKind::Email,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Agree,
    Variation,
    Conflict,
}

impl Relation {
    /// Higher means closer agreement.
    fn rank(self) -> u8 {
        match self {
            Relation::Agree => 2,
            Relation::Variation => 1,
            Relation::Conflict => 0,
        }
    }
}

/// An edge between two fields of different documents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub a: String,
    pub b: String,
    pub kind: FieldKind,
    pub relation: Relation,
}

/// Fields as nodes, comparisons as links.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CaseGraph {
    pub nodes: Vec<Field>,
    pub links: Vec<Link>,
}

#[derive(Debug, Clone)]
pub struct Correlation {
    pub graph: CaseGraph,
    pub findings: Vec<Finding>,
}

/// A field that disagrees with another document, with what it disagrees with.
struct Conflicted<'a> {
    field: &'a Field,
    others: Vec<String>,
    strength: f32,
}

/// Compares fields across documents and reports values that conflict.
pub fn correlate(fields: &[Field]) -> Result<Correlation> {
    let mut links = Vec::new();
    let mut conflicts = BTreeMap::new();

    for kind in COMPARED {
        let mut by_document: BTreeMap<&str, Vec<&Field>> = BTreeMap::new();
        for field in fields.iter().filter(|f| f.kind == kind) {
            by_document
                .entry(field.document.0.as_str())
                .or_default()
                .push(field);
        }

        let documents: Vec<&[&Field]> = by_document.values().map(Vec::as_slice).collect();
        for (i, left) in documents.iter().enumerate() {
            for right in documents.iter().skip(i + 1) {
                compare_documents(kind, left, right, &mut links, &mut conflicts);
            }
        }
    }

    let mut findings = Vec::new();
    for conflicted in conflicts.into_values() {
        let field = conflicted.field;
        let score = (0.2 + 0.7 * conflicted.strength).clamp(0.2, 0.9);
        let caution = if conflicted.strength < 0.6 {
            " Low OCR confidence means this may be a misread."
        } else {
            ""
        };

        findings.push(Finding::new(
            field.document.clone(),
            field.page,
            field.region,
            DetectorKind::CrossDocument,
            Status::Inconsistent,
            Confidence::new(score)?,
            format!(
                "{} reads '{}' here but {}.{}",
                label(field.kind),
                field.raw,
                conflicted.others.join(" and "),
                caution
            ),
        ));
    }

    Ok(Correlation {
        graph: CaseGraph {
            nodes: fields.to_vec(),
            links,
        },
        findings,
    })
}

/// Compares every field of one document with every field of another.
/// A document may legitimately hold several values (two phone numbers),
/// so a conflict is only reported when none of them agree.
fn compare_documents<'a>(
    kind: FieldKind,
    left: &[&'a Field],
    right: &[&'a Field],
    links: &mut Vec<Link>,
    conflicts: &mut BTreeMap<String, Conflicted<'a>>,
) {
    let mut pairs: Vec<(&'a Field, &'a Field, Relation)> = Vec::new();
    for &a in left {
        for &b in right {
            pairs.push((a, b, relate(kind, a, b)));
        }
    }

    let Some(best) = pairs.iter().map(|p| p.2).max_by_key(|r| r.rank()) else {
        return;
    };

    for &(a, b, relation) in pairs.iter().filter(|p| p.2 == best) {
        links.push(Link {
            a: a.id.clone(),
            b: b.id.clone(),
            kind,
            relation,
        });
    }
    if best != Relation::Conflict {
        return;
    }

    // Report the pair that was read most reliably.
    let Some(&(a, b, _)) = pairs
        .iter()
        .max_by(|x, y| strength(x.0, x.1).total_cmp(&strength(y.0, y.1)))
    else {
        return;
    };
    let strength = strength(a, b);
    note_conflict(conflicts, a, b, strength);
    note_conflict(conflicts, b, a, strength);
}

fn note_conflict<'a>(
    conflicts: &mut BTreeMap<String, Conflicted<'a>>,
    field: &'a Field,
    other: &Field,
    strength: f32,
) {
    let entry = conflicts
        .entry(field.id.clone())
        .or_insert_with(|| Conflicted {
            field,
            others: Vec::new(),
            strength,
        });
    entry.strength = entry.strength.max(strength);
    entry.others.push(format!(
        "'{}' in {} (page {})",
        other.raw,
        other.document.0,
        other.page + 1
    ));
}

/// A pair is only as reliable as its less certain reading.
fn strength(a: &Field, b: &Field) -> f32 {
    a.confidence.value().min(b.confidence.value())
}

fn relate(kind: FieldKind, a: &Field, b: &Field) -> Relation {
    if a.value == b.value {
        return Relation::Agree;
    }
    if kind == FieldKind::Name {
        return match compare_names(&a.value, &b.value) {
            NameMatch::Same => Relation::Agree,
            NameMatch::Variation => Relation::Variation,
            NameMatch::Different => Relation::Conflict,
        };
    }
    Relation::Conflict
}

fn label(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Name => "Name",
        FieldKind::DateOfBirth => "Date of birth",
        FieldKind::Date => "Date",
        FieldKind::Amount => "Amount",
        FieldKind::PanNumber => "PAN number",
        FieldKind::AadhaarNumber => "Aadhaar number",
        FieldKind::Phone => "Phone number",
        FieldKind::Email => "Email address",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{DocumentId, Rect};

    fn read(doc: &str, kind: FieldKind, raw: &str, y: f32, confidence: f32) -> Field {
        Field::new(
            DocumentId(doc.into()),
            0,
            kind,
            raw.into(),
            Rect::new(10.0, y, 80.0, 12.0),
            Confidence::new(confidence).unwrap(),
        )
    }

    fn dob(doc: &str, raw: &str) -> Field {
        read(doc, FieldKind::DateOfBirth, raw, 20.0, 0.9)
    }

    #[test]
    fn matching_values_produce_an_agreeing_link_and_no_findings() {
        let out = correlate(&[dob("id.pdf", "04/12/1990"), dob("form.pdf", "04-12-1990")]).unwrap();
        assert!(out.findings.is_empty());
        assert_eq!(out.graph.links.len(), 1);
        assert_eq!(out.graph.links[0].relation, Relation::Agree);
    }

    #[test]
    fn conflicting_dates_are_reported_on_both_documents() {
        let out = correlate(&[dob("id.pdf", "04/12/1990"), dob("form.pdf", "14/12/1990")]).unwrap();
        assert_eq!(out.findings.len(), 2);
        assert!(out
            .findings
            .iter()
            .all(|f| f.status == Status::Inconsistent));
        assert!(out
            .findings
            .iter()
            .all(|f| f.detector == DetectorKind::CrossDocument));
        assert_eq!(out.graph.links[0].relation, Relation::Conflict);

        let on_id = out
            .findings
            .iter()
            .find(|f| f.document.0 == "id.pdf")
            .unwrap();
        assert_eq!(
            on_id.summary,
            "Date of birth reads '04/12/1990' here but '14/12/1990' in form.pdf (page 1)."
        );
    }

    #[test]
    fn reordered_names_are_a_variation_not_a_finding() {
        let out = correlate(&[
            read("id.pdf", FieldKind::Name, "Rahul Kumar Sharma", 20.0, 0.9),
            read("form.pdf", FieldKind::Name, "Sharma Rahul Kumar", 20.0, 0.9),
        ])
        .unwrap();
        assert!(out.findings.is_empty());
        assert_eq!(out.graph.links[0].relation, Relation::Variation);
    }

    #[test]
    fn an_extra_phone_number_is_not_a_conflict() {
        let out = correlate(&[
            read("a.pdf", FieldKind::Phone, "9876543210", 20.0, 0.9),
            read("a.pdf", FieldKind::Phone, "9123456780", 40.0, 0.9),
            read("b.pdf", FieldKind::Phone, "9876543210", 20.0, 0.9),
        ])
        .unwrap();
        assert!(out.findings.is_empty());
        assert_eq!(out.graph.links.len(), 1);
    }

    #[test]
    fn fields_within_one_document_are_not_compared() {
        let out = correlate(&[
            read("a.pdf", FieldKind::Name, "Rahul Sharma", 20.0, 0.9),
            read("a.pdf", FieldKind::Name, "Rohan Verma", 40.0, 0.9),
        ])
        .unwrap();
        assert!(out.findings.is_empty());
        assert!(out.graph.links.is_empty());
    }

    #[test]
    fn dates_and_amounts_are_not_compared() {
        let out = correlate(&[
            read("a.pdf", FieldKind::Amount, "Rs 5,000", 20.0, 0.9),
            read("b.pdf", FieldKind::Amount, "Rs 9,000", 20.0, 0.9),
        ])
        .unwrap();
        assert!(out.findings.is_empty());
        assert!(out.graph.links.is_empty());
    }

    #[test]
    fn three_conflicting_documents_give_one_finding_each() {
        let out = correlate(&[
            dob("a.pdf", "01/01/1990"),
            dob("b.pdf", "02/01/1990"),
            dob("c.pdf", "03/01/1990"),
        ])
        .unwrap();
        assert_eq!(out.findings.len(), 3);

        let ids: HashSet<&str> = out.findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(out.graph.links.len(), 3);
    }

    #[test]
    fn weak_ocr_lowers_confidence_and_adds_a_caution() {
        let strong = correlate(&[
            read("a.pdf", FieldKind::DateOfBirth, "04/12/1990", 20.0, 0.95),
            read("b.pdf", FieldKind::DateOfBirth, "14/12/1990", 20.0, 0.95),
        ])
        .unwrap();
        let weak = correlate(&[
            read("a.pdf", FieldKind::DateOfBirth, "04/12/1990", 20.0, 0.3),
            read("b.pdf", FieldKind::DateOfBirth, "14/12/1990", 20.0, 0.9),
        ])
        .unwrap();

        assert!(weak.findings[0].confidence.value() < strong.findings[0].confidence.value());
        assert!(weak.findings[0].summary.contains("misread"));
        assert!(!strong.findings[0].summary.contains("misread"));
    }
}
