//! Core forensic analysis engine.

pub mod audit;
mod clock;
pub mod error;
pub mod evidence;
pub mod geometry;
pub mod hash;

pub use audit::{AuditEntry, AuditLog};
pub use error::{Error, Result};
pub use evidence::{Confidence, DetectorKind, DocumentId, Finding, Status};
pub use geometry::Rect;
