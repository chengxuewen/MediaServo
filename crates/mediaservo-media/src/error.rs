use thiserror::Error;

/// Errors for media operations.
#[derive(Error, Debug)]
pub enum MediaError {
    #[error("buffer size mismatch: expected {expected}, got {actual}")]
    BufferSizeMismatch { expected: u32, actual: u32 },

    #[error("unsupported pixel format: {0:?}")]
    UnsupportedFormat(crate::pixel_format::PixelFormat),

    #[error("invalid dimensions: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },

    #[error("invalid rotation: {0:?}")]
    InvalidRotation(crate::base::rotation::VideoRotation),

    #[error("backend error: {0}")]
    BackendError(String),

    #[error("internal error: {0}")]
    Internal(String),
}

/// Result type alias for media operations.
pub type MediaResult<T> = Result<T, MediaError>;
