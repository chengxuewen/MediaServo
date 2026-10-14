//! Deck error types.

/// Errors for the deck media data plane.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeckError {
    #[error("device: {0}")]
    Device(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("invalid_state: {0}")]
    InvalidState(String),
}
