use crate::base::buffer::I420BufferRef;
use crate::base::rotation::VideoRotation;
use crate::error::MediaError;
use crate::pixel_format::PixelFormat;

/// Merged trait for spatial transforms and color space conversion.
/// Backends implement both. I420BufferRef passed by value (it is Copy).
pub trait VideoTransform: Send + Sync {
    // --- Spatial transforms ---

    fn scale(
        src: I420BufferRef,
        src_w: u32,
        src_h: u32,
        dst: I420BufferRef,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(), MediaError>;

    fn mirror(src: I420BufferRef, w: u32, h: u32, dst: I420BufferRef) -> Result<(), MediaError>;

    fn crop(
        src: I420BufferRef,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        dst: I420BufferRef,
    ) -> Result<(), MediaError>;

    fn rotate(
        src: I420BufferRef,
        w: u32,
        h: u32,
        rot: VideoRotation,
        dst: I420BufferRef,
    ) -> Result<(), MediaError>;

    // --- Color space conversion ---

    fn argb_to_i420(argb: &[u8], w: u32, h: u32, dst: I420BufferRef) -> Result<(), MediaError>;

    fn i420_to_argb(
        src: I420BufferRef,
        w: u32,
        h: u32,
        fmt: PixelFormat,
        dst: &mut [u8],
    ) -> Result<(), MediaError>;

    fn i420_to_nv12(
        src: I420BufferRef,
        w: u32,
        h: u32,
        dst_y: &mut [u8],
        dst_uv: &mut [u8],
    ) -> Result<(), MediaError>;

    fn nv12_to_i420(
        src_y: &[u8],
        src_uv: &[u8],
        w: u32,
        h: u32,
        dst: I420BufferRef,
    ) -> Result<(), MediaError>;
}
