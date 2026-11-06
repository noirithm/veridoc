use serde::{Deserialize, Serialize};

use crate::clock::now_ms;
use crate::hash::sha256_hex;
use crate::{Error, Rect, Result};

/// Stable identifier for a submitted document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DocumentId(pub String);

/// A score in the range 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Confidence(f32);

impl Confidence {
    pub fn new(value: f32) -> Result<Self> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(Error::InvalidConfidence(value))
        }
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Confidence {
    type Error = Error;

    fn try_from(value: f32) -> Result<Self> {
        Self::new(value)
    }
}

impl From<Confidence> for f32 {
    fn from(c: Confidence) -> f32 {
        c.0
    }
}

/// How strongly a finding is supported. Findings are never labelled as
/// fraud; they describe what was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// A directly measurable characteristic of the document.
    Observed,
    /// Fields or documents disagree with each other.
    Inconsistent,
    /// Evidence suggests alteration but does not prove it.
    PotentialManipulation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorKind {
    ErrorLevel,
    Noise,
    CopyMove,
    Typography,
    Signature,
    PdfStructure,
    OcrConfidence,
    CrossDocument,
    InkMarks,
}

/// A single piece of evidence tied to a region of a document page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub document: DocumentId,
    pub page: u32,
    pub region: Rect,
    pub detector: DetectorKind,
    pub status: Status,
    pub confidence: Confidence,
    pub summary: String,
    pub detected_at_ms: u64,
}

impl Finding {
    /// Creates a finding. The id is derived from where and how it was
    /// detected, so re-running a detector yields the same id.
    pub fn new(
        document: DocumentId,
        page: u32,
        region: Rect,
        detector: DetectorKind,
        status: Status,
        confidence: Confidence,
        summary: impl Into<String>,
    ) -> Self {
        let key = format!(
            "{}|{}|{:?}|{}|{}|{}|{}",
            document.0,
            page,
            detector,
            region.x.to_bits(),
            region.y.to_bits(),
            region.width.to_bits(),
            region.height.to_bits(),
        );
        let id = sha256_hex(key.as_bytes())[..12].to_string();

        Self {
            id,
            document,
            page,
            region,
            detector,
            status,
            confidence,
            summary: summary.into(),
            detected_at_ms: now_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(summary: &str) -> Finding {
        Finding::new(
            DocumentId("doc-1".into()),
            0,
            Rect::new(10.0, 20.0, 100.0, 30.0),
            DetectorKind::ErrorLevel,
            Status::PotentialManipulation,
            Confidence::new(0.8).unwrap(),
            summary,
        )
    }

    #[test]
    fn confidence_rejects_out_of_range_values() {
        assert!(Confidence::new(-0.1).is_err());
        assert!(Confidence::new(1.1).is_err());
        assert!(Confidence::new(f32::NAN).is_err());
        assert!(Confidence::new(0.0).is_ok());
        assert!(Confidence::new(1.0).is_ok());
    }

    #[test]
    fn confidence_is_validated_on_deserialize() {
        assert!(serde_json::from_str::<Confidence>("0.5").is_ok());
        assert!(serde_json::from_str::<Confidence>("1.5").is_err());
    }

    #[test]
    fn finding_id_is_stable_for_the_same_location() {
        assert_eq!(sample("first").id, sample("second").id);
    }

    #[test]
    fn finding_round_trips_through_json() {
        let finding = sample("compression mismatch around the amount field");
        let json = serde_json::to_string(&finding).unwrap();
        let back: Finding = serde_json::from_str(&json).unwrap();
        assert_eq!(finding, back);
    }
}
