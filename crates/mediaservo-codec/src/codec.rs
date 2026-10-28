/// RFC 6381-aligned codec identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecId {
    H264,
}

/// Backend implementation identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendId {
    FFmpeg,
    GStreamer,
    Stub,
}

/// Pixel format for raw video frames (codec-crate internal, C5 boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    Yuv420p,
    Nv12,
}

impl PixelFormat {
    pub fn plane_count(&self) -> u8 {
        match self {
            PixelFormat::Yuv420p | PixelFormat::Nv12 => 3,
        }
    }
}

/// Frame dimensions and pixel format descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
}

/// Rational frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }
    pub fn fps(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
}
