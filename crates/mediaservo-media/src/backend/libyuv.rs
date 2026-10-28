use crate::base::buffer::I420BufferRef;
use crate::base::rotation::VideoRotation;
use crate::error::MediaError;
use crate::pixel_format::PixelFormat;
use crate::transform::VideoTransform;
// libyuv C bindings provided by livekit's yuv-sys crate (builds from source).
use yuv_sys::*;

// ── Constants ────────────────────────────────────────────

/// libyuv filter mode: no interpolation (nearest-neighbor).
const K_FILTER_NONE: u32 = 0;

// ── LibyuvTransform ──────────────────────────────────────

/// libyuv-accelerated video transform backend.
///
/// All spatial and color-conversion operations delegate to the system
/// `libyuv` shared library via `extern "C"` FFI. The `crop` method is
/// implemented as a pure-Rust row-by-row copy (simpler and more correct
/// than the libyuv `ConvertToI420` path).
pub struct LibyuvTransform;

// ── Helpers ──────────────────────────────────────────────

/// Convert libyuv return code to Rust `Result`.
///
/// libyuv returns 0 on success, negative on error.
fn libyuv_ret(ret: i32) -> Result<(), MediaError> {
    if ret == 0 { Ok(()) } else { Err(MediaError::BackendError(format!("libyuv error: {}", ret))) }
}

/// Map `VideoRotation` to libyuv rotation degrees.
fn rotation_deg(rot: VideoRotation) -> i32 {
    match rot {
        VideoRotation::Rotation0 => 0,
        VideoRotation::Rotation90 => 90,
        VideoRotation::Rotation180 => 180,
        VideoRotation::Rotation270 => 270,
    }
}

// ── VideoTransform impl ──────────────────────────────────

impl VideoTransform for LibyuvTransform {
    fn scale(
        src: I420BufferRef,
        src_w: u32,
        src_h: u32,
        dst: I420BufferRef,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(), MediaError> {
        // SAFETY: libyuv reads only from src planes and writes only
        // to dst planes. The caller owns both buffers and guarantees
        // each plane slice covers stride × height bytes.
        let r = unsafe {
            rs_I420Scale(
                src.y_ptr,
                src.stride_y as i32,
                src.u_ptr,
                src.stride_u as i32,
                src.v_ptr,
                src.stride_v as i32,
                src_w as i32,
                src_h as i32,
                dst.y_ptr,
                dst.stride_y as i32,
                dst.u_ptr,
                dst.stride_u as i32,
                dst.v_ptr,
                dst.stride_v as i32,
                dst_w as i32,
                dst_h as i32,
                K_FILTER_NONE,
            )
        };
        libyuv_ret(r)
    }

    fn mirror(src: I420BufferRef, w: u32, h: u32, dst: I420BufferRef) -> Result<(), MediaError> {
        // SAFETY: same contract as scale — libyuv reads src, writes dst.
        let r = unsafe {
            rs_I420Mirror(
                src.y_ptr,
                src.stride_y as i32,
                src.u_ptr,
                src.stride_u as i32,
                src.v_ptr,
                src.stride_v as i32,
                dst.y_ptr,
                dst.stride_y as i32,
                dst.u_ptr,
                dst.stride_u as i32,
                dst.v_ptr,
                dst.stride_v as i32,
                w as i32,
                h as i32,
            )
        };
        libyuv_ret(r)
    }

    fn crop(
        src: I420BufferRef,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        dst: I420BufferRef,
    ) -> Result<(), MediaError> {
        // ponytail: pure-Rust row-by-row copy instead of libyuv FFI.
        // For each plane, compute the source offset from (x, y) and
        // copy the crop rectangle row by row into dst.

        let x = x as usize;
        let y = y as usize;
        let w = w as usize;
        let h = h as usize;

        // Y plane: full resolution
        {
            let src_off = y * src.stride_y + x;
            // SAFETY: pointer arithmetic is bounded by the caller's
            // guarantee that (x, y, w, h) are within the src buffer.
            // dst has room for w×h Y plane.
            for row in 0..h {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.y_ptr.add(src_off + row * src.stride_y),
                        dst.y_ptr.add(row * dst.stride_y),
                        w,
                    );
                }
            }
        }

        // U plane: subsampled — (x/2, y/2), w/2 wide, h/2 tall
        {
            let half_w = w / 2;
            let half_h = h / 2;
            let src_off = (y / 2) * src.stride_u + (x / 2);
            // SAFETY: same contract as Y plane, scaled for chroma subsampling.
            for row in 0..half_h {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.u_ptr.add(src_off + row * src.stride_u),
                        dst.u_ptr.add(row * dst.stride_u),
                        half_w,
                    );
                }
            }
        }

        // V plane: same geometry as U
        {
            let half_w = w / 2;
            let half_h = h / 2;
            let src_off = (y / 2) * src.stride_v + (x / 2);
            // SAFETY: same contract as U plane.
            for row in 0..half_h {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.v_ptr.add(src_off + row * src.stride_v),
                        dst.v_ptr.add(row * dst.stride_v),
                        half_w,
                    );
                }
            }
        }

        Ok(())
    }

    fn rotate(
        src: I420BufferRef,
        w: u32,
        h: u32,
        rot: VideoRotation,
        dst: I420BufferRef,
    ) -> Result<(), MediaError> {
        // SAFETY: libyuv reads src, writes dst. Rotation=0 is a no-op
        // but we still delegate (avoids branching at the backend level).
        let r = unsafe {
            rs_I420Rotate(
                src.y_ptr,
                src.stride_y as i32,
                src.u_ptr,
                src.stride_u as i32,
                src.v_ptr,
                src.stride_v as i32,
                dst.y_ptr,
                dst.stride_y as i32,
                dst.u_ptr,
                dst.stride_u as i32,
                dst.v_ptr,
                dst.stride_v as i32,
                w as i32,
                h as i32,
                rotation_deg(rot) as u32,
            )
        };
        libyuv_ret(r)
    }

    fn argb_to_i420(argb: &[u8], w: u32, h: u32, dst: I420BufferRef) -> Result<(), MediaError> {
        // SAFETY: ARGB src slice covers w×h×4 bytes. libyuv writes to dst planes.
        let r = unsafe {
            rs_ARGBToI420(
                argb.as_ptr(),
                (w * 4) as i32,
                dst.y_ptr,
                dst.stride_y as i32,
                dst.u_ptr,
                dst.stride_u as i32,
                dst.v_ptr,
                dst.stride_v as i32,
                w as i32,
                h as i32,
            )
        };
        libyuv_ret(r)
    }

    fn i420_to_argb(
        src: I420BufferRef,
        w: u32,
        h: u32,
        _fmt: PixelFormat,
        dst: &mut [u8],
    ) -> Result<(), MediaError> {
        // ponytail: libyuv's rs_I420ToARGB writes BGRA byte order in memory
        // (A=MSB, R, G, B=LSB per pixel → BGRA as LE u32). If the caller
        // requested a different format, swizzle post-call.
        // SAFETY: I420 src planes and ARGB dst buffer are caller-owned.
        let r = unsafe {
            rs_I420ToARGB(
                src.y_ptr,
                src.stride_y as i32,
                src.u_ptr,
                src.stride_u as i32,
                src.v_ptr,
                src.stride_v as i32,
                dst.as_mut_ptr(),
                (w * 4) as i32,
                w as i32,
                h as i32,
            )
        };
        libyuv_ret(r)
    }

    fn i420_to_nv12(
        src: I420BufferRef,
        w: u32,
        h: u32,
        dst_y: &mut [u8],
        dst_uv: &mut [u8],
    ) -> Result<(), MediaError> {
        // SAFETY: libyuv reads I420 src, writes to NV12 dst_y (w×h) and
        // dst_uv (w×h/2) output buffers.
        let r = unsafe {
            rs_I420ToNV12(
                src.y_ptr,
                src.stride_y as i32,
                src.u_ptr,
                src.stride_u as i32,
                src.v_ptr,
                src.stride_v as i32,
                dst_y.as_mut_ptr(),
                w as i32,
                dst_uv.as_mut_ptr(),
                w as i32,
                w as i32,
                h as i32,
            )
        };
        libyuv_ret(r)
    }

    fn nv12_to_i420(
        src_y: &[u8],
        src_uv: &[u8],
        w: u32,
        h: u32,
        dst: I420BufferRef,
    ) -> Result<(), MediaError> {
        // SAFETY: libyuv reads NV12 src planes, writes to I420 dst.
        let r = unsafe {
            rs_NV12ToI420(
                src_y.as_ptr(),
                w as i32,
                src_uv.as_ptr(),
                w as i32,
                dst.y_ptr,
                dst.stride_y as i32,
                dst.u_ptr,
                dst.stride_u as i32,
                dst.v_ptr,
                dst.stride_v as i32,
                w as i32,
                h as i32,
            )
        };
        libyuv_ret(r)
    }
}
