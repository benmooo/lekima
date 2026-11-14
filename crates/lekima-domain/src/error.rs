//! Domain-level failures.
//!
//! Errors are intentionally coarse: frontends decide how to present them, and
//! adapters map their own error types into these variants.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("authentication required: {0}")]
    AuthenticationRequired(String),

    #[error("operation not supported by this backend: {0}")]
    Unsupported(String),

    #[error("backend unavailable: {0}")]
    Unavailable(String),

    #[error("playback failed: {0}")]
    Playback(String),
}

pub type Result<T> = std::result::Result<T, DomainError>;
