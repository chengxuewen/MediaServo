use crate::base::buffer::VideoBuffer;

/// Per-frame metadata carried alongside pixel data.
#[derive(Debug, Clone)]
pub struct FrameMetadata {
    /// Application-assigned timestamp (e.g., capture time).
    pub user_timestamp: Option<u64>,
    /// Monotonically incrementing frame sequence number.
    pub frame_id: Option<u32>,
}

impl FrameMetadata {
    pub fn new() -> Self {
        Self { user_timestamp: None, frame_id: None }
    }
}

impl Default for FrameMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// A complete video frame with rotation, timestamp, metadata, and pixel data.
pub struct VideoFrame<T> {
    pub rotation: super::rotation::VideoRotation,
    pub timestamp_us: i64,
    pub metadata: Option<FrameMetadata>,
    pub buffer: T,
}

impl<T> VideoFrame<T> {
    pub fn new(buffer: T) -> Self {
        Self { rotation: Default::default(), timestamp_us: 0, metadata: None, buffer }
    }

    pub fn with_rotation(mut self, rotation: super::rotation::VideoRotation) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_timestamp(mut self, timestamp_us: i64) -> Self {
        self.timestamp_us = timestamp_us;
        self
    }

    pub fn with_metadata(mut self, metadata: FrameMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for VideoFrame<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("rotation", &self.rotation)
            .field("timestamp_us", &self.timestamp_us)
            .field("metadata", &self.metadata)
            .field("buffer", &self.buffer)
            .finish()
    }
}

/// Type-erased video frame for pipeline use.
pub type BoxVideoFrame = VideoFrame<Box<dyn VideoBuffer>>;
