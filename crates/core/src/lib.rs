//! Core forensic analysis engine.

pub mod audit;
mod clock;
pub mod correlation;
pub mod detectors;
pub mod error;
pub mod evidence;
pub mod explain;
pub mod fields;
pub mod fusion;
pub mod geometry;
pub mod hash;
pub mod matching;
pub mod ocr;

pub use audit::{AuditEntry, AuditLog};
pub use correlation::{correlate, CaseGraph, Correlation, Link, Relation};
pub use detectors::Detector;
pub use error::{Error, Result};
pub use evidence::{Confidence, DetectorKind, DocumentId, Finding, Status};
pub use explain::{explain, overview, Explanation};
pub use fields::{Field, FieldExtractor, FieldKind};
pub use fusion::{fuse, FusedFinding, Priority};
pub use geometry::Rect;
pub use ocr::{OcrEngine, OcrLine, OcrWord};
