use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("confidence must be between 0 and 1, got {0}")]
    InvalidConfidence(f32),

    #[error("audit log is broken at entry {index}: {reason}")]
    BrokenChain { index: u64, reason: &'static str },

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Image(#[from] image::ImageError),

    #[error("no document with id '{0}' has been added")]
    UnknownDocument(String),

    #[error("a document with id '{0}' was already added")]
    DuplicateDocument(String),
}

pub type Result<T> = std::result::Result<T, Error>;
