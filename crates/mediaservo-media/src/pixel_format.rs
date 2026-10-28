/// Unified pixel format enum — follows OpenCTK's single VideoType approach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    // RGB formats
    ARGB,
    BGRA,
    ABGR,
    RGBA,
    // YUV planar formats
    I420,
    I422,
    I444,
    // YUV biplanar
    NV12,
    // YUV 10-bit
    I010,
    P010, // merged from RawPixelFormat
}
