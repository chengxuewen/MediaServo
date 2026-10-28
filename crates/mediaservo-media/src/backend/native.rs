use crate::base::buffer::I420BufferRef;
use crate::base::rotation::VideoRotation;
use crate::error::MediaError;
use crate::pixel_format::PixelFormat;
use crate::transform::VideoTransform;

/// Pure-Rust native video transform backend.
///
/// Implements nearest-neighbor scale, vertical-flip mirror, sub-rectangle crop,
/// pixel-reordering rotate, and BT.601 full-range color-space conversion — all
/// with integer arithmetic and zero external dependencies.
///
/// # Safety of pointer-based buffer access
///
/// `I420BufferRef` stores raw pointers. The caller guarantees that `src` pointers
/// are readable for the given dimensions and `dst` pointers are writable.
/// Each method reconstructs `&[u8]` / `&mut [u8]` from the raw parts once at the
/// top and operates in safe Rust thereafter.
pub struct NativeTransform;

impl NativeTransform {
    /// Reconstruct a read-only slice from a raw pointer + length.
    ///
    /// # Safety
    /// Caller must ensure the pointer is valid for reads of `len` bytes.
    #[inline]
    unsafe fn read_slice(ptr: *const u8, len: usize) -> &'static [u8] {
        // SAFETY: caller guarantees validity.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    /// Reconstruct a mutable slice from a raw pointer + length.
    ///
    /// # Safety
    /// Caller must ensure the pointer is valid for writes of `len` bytes
    /// and no other reference aliases it.
    #[inline]
    unsafe fn write_slice(ptr: *mut u8, len: usize) -> &'static mut [u8] {
        // SAFETY: caller guarantees validity and exclusive access.
        unsafe { std::slice::from_raw_parts_mut(ptr, len) }
    }
}

impl VideoTransform for NativeTransform {
    fn scale(
        src: I420BufferRef,
        src_w: u32,
        src_h: u32,
        dst: I420BufferRef,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(), MediaError> {
        // SAFETY: dst is backed by I420Buffer allocation, writable exclusively here.
        let dst_y = unsafe { Self::write_slice(dst.y_ptr, dst.y_len) };
        let dst_u = unsafe { Self::write_slice(dst.u_ptr, dst.u_len) };
        let dst_v = unsafe { Self::write_slice(dst.v_ptr, dst.v_len) };
        // SAFETY: src is readable for its dimensions.
        let src_y = unsafe { Self::read_slice(src.y_ptr, src.y_len) };
        let src_u = unsafe { Self::read_slice(src.u_ptr, src.u_len) };
        let src_v = unsafe { Self::read_slice(src.v_ptr, src.v_len) };

        // Y plane — full resolution, nearest-neighbor
        for dy in 0..dst_h {
            let sy = (dy as u64 * src_h as u64 / dst_h as u64) as usize;
            let src_row = sy * src.stride_y;
            let dst_row = dy as usize * dst.stride_y;
            for dx in 0..dst_w {
                let sx = (dx as u64 * src_w as u64 / dst_w as u64) as usize;
                dst_y[dst_row + dx as usize] = src_y[src_row + sx];
            }
        }

        let half_src_w = src_w / 2;
        let half_src_h = src_h / 2;
        let half_dst_w = dst_w / 2;
        let half_dst_h = dst_h / 2;

        for dy in 0..half_dst_h {
            let sy = (dy as u64 * half_src_h as u64 / half_dst_h as u64) as usize;
            let src_row = sy * src.stride_u;
            let dst_row = dy as usize * dst.stride_u;
            for dx in 0..half_dst_w {
                let sx = (dx as u64 * half_src_w as u64 / half_dst_w as u64) as usize;
                dst_u[dst_row + dx as usize] = src_u[src_row + sx];
            }
        }

        for dy in 0..half_dst_h {
            let sy = (dy as u64 * half_src_h as u64 / half_dst_h as u64) as usize;
            let src_row = sy * src.stride_v;
            let dst_row = dy as usize * dst.stride_v;
            for dx in 0..half_dst_w {
                let sx = (dx as u64 * half_src_w as u64 / half_dst_w as u64) as usize;
                dst_v[dst_row + dx as usize] = src_v[src_row + sx];
            }
        }

        Ok(())
    }

    /// Horizontal mirror (left-right flip) matching libyuv I420Mirror.
    fn mirror(src: I420BufferRef, w: u32, h: u32, dst: I420BufferRef) -> Result<(), MediaError> {
        let w = w as usize;
        let h = h as usize;

        let dst_y = unsafe { Self::write_slice(dst.y_ptr, dst.y_len) };
        let dst_u = unsafe { Self::write_slice(dst.u_ptr, dst.u_len) };
        let dst_v = unsafe { Self::write_slice(dst.v_ptr, dst.v_len) };
        let src_y = unsafe { Self::read_slice(src.y_ptr, src.y_len) };
        let src_u = unsafe { Self::read_slice(src.u_ptr, src.u_len) };
        let src_v = unsafe { Self::read_slice(src.v_ptr, src.v_len) };

        for y in 0..h {
            let src_row = y * src.stride_y;
            let dst_row = y * dst.stride_y;
            for x in 0..w {
                dst_y[dst_row + (w - 1 - x)] = src_y[src_row + x];
            }
        }

        let half_w = w / 2;
        let half_h = h / 2;
        for y in 0..half_h {
            let src_row = y * src.stride_u;
            let dst_row = y * dst.stride_u;
            for x in 0..half_w {
                dst_u[dst_row + (half_w - 1 - x)] = src_u[src_row + x];
                dst_v[dst_row + (half_w - 1 - x)] = src_v[src_row + x];
            }
        }

        Ok(())
    }

    fn crop(
        src: I420BufferRef,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        dst: I420BufferRef,
    ) -> Result<(), MediaError> {
        let w = w as usize;
        let h = h as usize;
        let x = x as usize;
        let y = y as usize;

        let dst_y = unsafe { Self::write_slice(dst.y_ptr, dst.y_len) };
        let dst_u = unsafe { Self::write_slice(dst.u_ptr, dst.u_len) };
        let dst_v = unsafe { Self::write_slice(dst.v_ptr, dst.v_len) };
        let src_y = unsafe { Self::read_slice(src.y_ptr, src.y_len) };
        let src_u = unsafe { Self::read_slice(src.u_ptr, src.u_len) };
        let src_v = unsafe { Self::read_slice(src.v_ptr, src.v_len) };

        for row in 0..h {
            let src_offset = (y + row) * src.stride_y + x;
            let dst_offset = row * dst.stride_y;
            dst_y[dst_offset..dst_offset + w].copy_from_slice(&src_y[src_offset..src_offset + w]);
        }

        let half_x = x / 2;
        let half_y = y / 2;
        let half_w = w / 2;
        let half_h = h / 2;
        for row in 0..half_h {
            let src_offset = (half_y + row) * src.stride_u + half_x;
            let dst_offset = row * dst.stride_u;
            dst_u[dst_offset..dst_offset + half_w]
                .copy_from_slice(&src_u[src_offset..src_offset + half_w]);
            dst_v[dst_offset..dst_offset + half_w]
                .copy_from_slice(&src_v[src_offset..src_offset + half_w]);
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
        let w = w as usize;
        let h = h as usize;

        let dst_y = unsafe { Self::write_slice(dst.y_ptr, dst.y_len) };
        let dst_u = unsafe { Self::write_slice(dst.u_ptr, dst.u_len) };
        let dst_v = unsafe { Self::write_slice(dst.v_ptr, dst.v_len) };
        let src_y = unsafe { Self::read_slice(src.y_ptr, src.y_len) };
        let src_u = unsafe { Self::read_slice(src.u_ptr, src.u_len) };
        let src_v = unsafe { Self::read_slice(src.v_ptr, src.v_len) };

        match rot {
            VideoRotation::Rotation0 => {
                for y in 0..h {
                    let off = y * src.stride_y;
                    let dst_off = y * dst.stride_y;
                    dst_y[dst_off..dst_off + w].copy_from_slice(&src_y[off..off + w]);
                }
                let half_w = w / 2;
                let half_h = h / 2;
                for y in 0..half_h {
                    let off = y * src.stride_u;
                    let dst_off = y * dst.stride_u;
                    dst_u[dst_off..dst_off + half_w].copy_from_slice(&src_u[off..off + half_w]);
                    dst_v[dst_off..dst_off + half_w].copy_from_slice(&src_v[off..off + half_w]);
                }
            }
            VideoRotation::Rotation90 => {
                for sy in 0..h {
                    for sx in 0..w {
                        let dy = sx;
                        let dx = h - 1 - sy;
                        dst_y[dy * dst.stride_y + dx] = src_y[sy * src.stride_y + sx];
                    }
                }
                let half_w = w / 2;
                let half_h = h / 2;
                for sy in 0..half_h {
                    for sx in 0..half_w {
                        let dy = sx;
                        let dx = half_h - 1 - sy;
                        dst_u[dy * dst.stride_u + dx] = src_u[sy * src.stride_u + sx];
                        dst_v[dy * dst.stride_v + dx] = src_v[sy * src.stride_v + sx];
                    }
                }
            }
            VideoRotation::Rotation180 => {
                for sy in 0..h {
                    for sx in 0..w {
                        let dy = h - 1 - sy;
                        let dx = w - 1 - sx;
                        dst_y[dy * dst.stride_y + dx] = src_y[sy * src.stride_y + sx];
                    }
                }
                let half_w = w / 2;
                let half_h = h / 2;
                for sy in 0..half_h {
                    for sx in 0..half_w {
                        let dy = half_h - 1 - sy;
                        let dx = half_w - 1 - sx;
                        dst_u[dy * dst.stride_u + dx] = src_u[sy * src.stride_u + sx];
                        dst_v[dy * dst.stride_v + dx] = src_v[sy * src.stride_v + sx];
                    }
                }
            }
            VideoRotation::Rotation270 => {
                for sy in 0..h {
                    for sx in 0..w {
                        let dy = w - 1 - sx;
                        let dx = sy;
                        dst_y[dy * dst.stride_y + dx] = src_y[sy * src.stride_y + sx];
                    }
                }
                let half_w = w / 2;
                let half_h = h / 2;
                for sy in 0..half_h {
                    for sx in 0..half_w {
                        let dy = half_w - 1 - sx;
                        let dx = sy;
                        dst_u[dy * dst.stride_u + dx] = src_u[sy * src.stride_u + sx];
                        dst_v[dy * dst.stride_v + dx] = src_v[sy * src.stride_v + sx];
                    }
                }
            }
        }

        Ok(())
    }

    fn argb_to_i420(argb: &[u8], w: u32, h: u32, dst: I420BufferRef) -> Result<(), MediaError> {
        let w = w as usize;
        let h = h as usize;
        let stride_argb = w * 4;

        let dst_y = unsafe { Self::write_slice(dst.y_ptr, dst.y_len) };
        let dst_u = unsafe { Self::write_slice(dst.u_ptr, dst.u_len) };
        let dst_v = unsafe { Self::write_slice(dst.v_ptr, dst.v_len) };

        for by in 0..(h / 2) {
            for bx in 0..(w / 2) {
                let mut u_sum: u32 = 0;
                let mut v_sum: u32 = 0;

                for dy in 0..2usize {
                    for dx in 0..2usize {
                        let px_idx = (by * 2 + dy) * stride_argb + (bx * 2 + dx) * 4;
                        let b = argb[px_idx] as i32;
                        let g = argb[px_idx + 1] as i32;
                        let r = argb[px_idx + 2] as i32;

                        let y_val = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
                        let y_pos = (by * 2 + dy) * dst.stride_y + (bx * 2 + dx);
                        dst_y[y_pos] = y_val.clamp(16, 235) as u8;

                        let u_val = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                        let v_val = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                        u_sum += u_val.clamp(16, 240) as u32;
                        v_sum += v_val.clamp(16, 240) as u32;
                    }
                }

                let uv_pos = by * dst.stride_u + bx;
                dst_u[uv_pos] = ((u_sum + 2) / 4) as u8;
                dst_v[uv_pos] = ((v_sum + 2) / 4) as u8;
            }
        }

        Ok(())
    }

    fn i420_to_argb(
        src: I420BufferRef,
        w: u32,
        h: u32,
        fmt: PixelFormat,
        dst: &mut [u8],
    ) -> Result<(), MediaError> {
        let w = w as usize;
        let h = h as usize;

        let src_y = unsafe { Self::read_slice(src.y_ptr, src.y_len) };
        let src_u = unsafe { Self::read_slice(src.u_ptr, src.u_len) };
        let src_v = unsafe { Self::read_slice(src.v_ptr, src.v_len) };

        for y in 0..h {
            for x in 0..w {
                let yy = src_y[y * src.stride_y + x] as i32;
                let u = src_u[(y / 2) * src.stride_u + (x / 2)] as i32;
                let v = src_v[(y / 2) * src.stride_v + (x / 2)] as i32;

                let yp = yy - 16;
                let up = u - 128;
                let vp = v - 128;

                let r = ((298 * yp + 409 * vp + 128) >> 8).clamp(0, 255) as u8;
                let g = ((298 * yp - 100 * up - 208 * vp + 128) >> 8).clamp(0, 255) as u8;
                let b = ((298 * yp + 516 * up + 128) >> 8).clamp(0, 255) as u8;

                let dst_offset = (y * w + x) * 4;
                match fmt {
                    PixelFormat::ARGB => {
                        dst[dst_offset] = 255;
                        dst[dst_offset + 1] = r;
                        dst[dst_offset + 2] = g;
                        dst[dst_offset + 3] = b;
                    }
                    PixelFormat::BGRA => {
                        dst[dst_offset] = b;
                        dst[dst_offset + 1] = g;
                        dst[dst_offset + 2] = r;
                        dst[dst_offset + 3] = 255;
                    }
                    PixelFormat::ABGR => {
                        dst[dst_offset] = 255;
                        dst[dst_offset + 1] = b;
                        dst[dst_offset + 2] = g;
                        dst[dst_offset + 3] = r;
                    }
                    PixelFormat::RGBA => {
                        dst[dst_offset] = r;
                        dst[dst_offset + 1] = g;
                        dst[dst_offset + 2] = b;
                        dst[dst_offset + 3] = 255;
                    }
                    PixelFormat::I420
                    | PixelFormat::I422
                    | PixelFormat::I444
                    | PixelFormat::NV12
                    | PixelFormat::I010
                    | PixelFormat::P010 => {
                        return Err(MediaError::UnsupportedFormat(fmt));
                    }
                }
            }
        }

        Ok(())
    }

    fn i420_to_nv12(
        src: I420BufferRef,
        w: u32,
        h: u32,
        dst_y: &mut [u8],
        dst_uv: &mut [u8],
    ) -> Result<(), MediaError> {
        let w = w as usize;
        let h = h as usize;

        let src_y = unsafe { Self::read_slice(src.y_ptr, src.y_len) };
        let src_u = unsafe { Self::read_slice(src.u_ptr, src.u_len) };
        let src_v = unsafe { Self::read_slice(src.v_ptr, src.v_len) };

        for row in 0..h {
            let src_off = row * src.stride_y;
            let dst_off = row * w;
            dst_y[dst_off..dst_off + w].copy_from_slice(&src_y[src_off..src_off + w]);
        }

        let half_w = w / 2;
        let half_h = h / 2;
        for y in 0..half_h {
            for x in 0..half_w {
                let src_off = y * src.stride_u + x;
                let dst_off = (y * half_w + x) * 2;
                dst_uv[dst_off] = src_u[src_off];
                dst_uv[dst_off + 1] = src_v[src_off];
            }
        }

        Ok(())
    }

    fn nv12_to_i420(
        src_y: &[u8],
        src_uv: &[u8],
        w: u32,
        h: u32,
        dst: I420BufferRef,
    ) -> Result<(), MediaError> {
        let w = w as usize;
        let h = h as usize;

        let dst_y = unsafe { Self::write_slice(dst.y_ptr, dst.y_len) };
        let dst_u = unsafe { Self::write_slice(dst.u_ptr, dst.u_len) };
        let dst_v = unsafe { Self::write_slice(dst.v_ptr, dst.v_len) };

        for row in 0..h {
            let src_off = row * w;
            let dst_off = row * dst.stride_y;
            dst_y[dst_off..dst_off + w].copy_from_slice(&src_y[src_off..src_off + w]);
        }

        let half_w = w / 2;
        let half_h = h / 2;
        for y in 0..half_h {
            for x in 0..half_w {
                let src_idx = (y * half_w + x) * 2;
                let dst_off = y * dst.stride_u + x;
                dst_u[dst_off] = src_uv[src_idx];
                dst_v[dst_off] = src_uv[src_idx + 1];
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::buffer::{I420Buffer, VideoBuffer};

    fn make_i420(w: u32, h: u32) -> I420Buffer {
        I420Buffer::new(w, h)
    }

    fn i420_ref(buf: &I420Buffer) -> I420BufferRef<'_> {
        buf.as_i420_ref().unwrap()
    }

    // ── scale ──────────────────────────────────────────

    #[test]
    fn scale_same_size_is_copy() {
        let src_buf = make_i420(8, 8);
        let dst_buf = make_i420(8, 8);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::scale(src_ref, 8, 8, dst_ref, 8, 8).unwrap();
        assert_eq!(dst_buf.data_y, src_buf.data_y);
    }

    #[test]
    fn scale_half_size() {
        let mut src_buf = make_i420(8, 8);
        for i in 0..64 {
            src_buf.data_y[i] = (i % 256) as u8;
        }
        src_buf.data_u.fill(50);
        src_buf.data_v.fill(150);

        let dst_buf = make_i420(4, 4);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::scale(src_ref, 8, 8, dst_ref, 4, 4).unwrap();
        assert_eq!(dst_buf.data_y.len(), 16);
        assert_eq!(dst_buf.data_y[0], src_buf.data_y[0]);
    }

    // ── mirror ──────────────────────────────────────────

    #[test]
    fn mirror_horizontal_flip() {
        // 4x4 frame with distinct column values: row 0=[0,1,2,3], row 1=[4,5,6,7], etc.
        let mut src_buf = make_i420(4, 4);
        for y in 0..4u32 {
            for x in 0..4u32 {
                src_buf.data_y[(y * 4 + x) as usize] = (y * 4 + x) as u8;
            }
        }
        let dst_buf = make_i420(4, 4);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::mirror(src_ref, 4, 4, dst_ref).unwrap();
        // After horizontal mirror: each row reversed
        assert_eq!(dst_buf.data_y[0], 3); // row0 col0 = old row0 col3
        assert_eq!(dst_buf.data_y[3], 0); // row0 col3 = old row0 col0
        assert_eq!(dst_buf.data_y[4], 7); // row1 col0 = old row1 col3
        assert_eq!(dst_buf.data_y[7], 4); // row1 col3 = old row1 col0
    }

    // ── crop ────────────────────────────────────────────

    #[test]
    fn crop_2x2_from_4x4() {
        let mut src_buf = make_i420(4, 4);
        for i in 0..16u8 {
            src_buf.data_y[i as usize] = i;
        }

        let dst_buf = make_i420(2, 2);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::crop(src_ref, 0, 0, 2, 2, dst_ref).unwrap();
        assert_eq!(dst_buf.data_y[0], 0);
        assert_eq!(dst_buf.data_y[1], 1);
        assert_eq!(dst_buf.data_y[2], 4);
        assert_eq!(dst_buf.data_y[3], 5);
    }

    #[test]
    fn crop_bottom_right_2x2() {
        let mut src_buf = make_i420(4, 4);
        for i in 0..16u8 {
            src_buf.data_y[i as usize] = i;
        }

        let dst_buf = make_i420(2, 2);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::crop(src_ref, 2, 2, 2, 2, dst_ref).unwrap();
        assert_eq!(dst_buf.data_y[0], 10);
        assert_eq!(dst_buf.data_y[1], 11);
        assert_eq!(dst_buf.data_y[2], 14);
        assert_eq!(dst_buf.data_y[3], 15);
    }

    // ── rotate ──────────────────────────────────────────

    #[test]
    fn rotate_0_is_identity() {
        let mut src_buf = make_i420(4, 4);
        for i in 0..16u8 {
            src_buf.data_y[i as usize] = i;
        }

        let dst_buf = make_i420(4, 4);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::rotate(src_ref, 4, 4, VideoRotation::Rotation0, dst_ref).unwrap();
        assert_eq!(dst_buf.data_y, src_buf.data_y);
    }

    #[test]
    fn rotate_180_is_invert() {
        let mut src_buf = make_i420(4, 4);
        for i in 0..16u8 {
            src_buf.data_y[i as usize] = i;
        }

        let dst_buf = make_i420(4, 4);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::rotate(src_ref, 4, 4, VideoRotation::Rotation180, dst_ref).unwrap();
        assert_eq!(dst_buf.data_y[0], 15);
        assert_eq!(dst_buf.data_y[15], 0);
    }

    #[test]
    fn rotate_90_swaps_axes() {
        let mut src_buf = make_i420(4, 2);
        // Fill with position-dependent values: row*stride + col
        for row in 0..2u32 {
            for col in 0..4u32 {
                let s_off = (row * 4 + col) as usize;
                src_buf.data_y[s_off] = (row * 4 + col) as u8;
            }
        }
        src_buf.data_u.fill(128);
        src_buf.data_v.fill(128);

        // After 90° CW rotation, 4×2 becomes 2×4
        let dst_buf = make_i420(2, 4);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::rotate(src_ref, 4, 2, VideoRotation::Rotation90, dst_ref).unwrap();
        // src[0][0]=0 -> dy=sx=0, dx=h-1-sy=1 -> dst[0][1]=dst[1]
        assert_eq!(dst_buf.data_y[1], 0);
        // src[1][0]=4 -> dy=0, dx=0 -> dst[0][0]=dst[0]
        assert_eq!(dst_buf.data_y[0], 4);
    }

    #[test]
    fn rotate_270_swaps_axes() {
        let mut src_buf = make_i420(4, 2);
        for row in 0..2u32 {
            for col in 0..4u32 {
                let s_off = (row * 4 + col) as usize;
                src_buf.data_y[s_off] = (row * 4 + col) as u8;
            }
        }
        src_buf.data_u.fill(128);
        src_buf.data_v.fill(128);

        // After 270° CCW rotation, 4×2 becomes 2×4
        let dst_buf = make_i420(2, 4);
        let src_ref = i420_ref(&src_buf);
        let dst_ref = i420_ref(&dst_buf);
        NativeTransform::rotate(src_ref, 4, 2, VideoRotation::Rotation270, dst_ref).unwrap();
        // src[0][0]=0 -> dy=w-1-sx=3, dx=sy=0 -> dst[3][0]=dst[6]
        assert_eq!(dst_buf.data_y[6], 0);
        // src[1][3]=7 -> dy=0, dx=1 -> dst[0][1]=dst[1]
        assert_eq!(dst_buf.data_y[1], 7);
    }

    // ── argb_to_i420 ────────────────────────────────────

    #[test]
    fn argb_to_i420_black() {
        let mut argb = vec![0u8; 16];
        for i in (0..16).step_by(4) {
            argb[i] = 0;
            argb[i + 1] = 0;
            argb[i + 2] = 0;
            argb[i + 3] = 255;
        }

        let dst_buf = make_i420(2, 2);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::argb_to_i420(&argb, 2, 2, dst_ref).unwrap();
        assert!((dst_buf.data_y[0] as i32 - 16).abs() <= 1);
        assert!((dst_buf.data_u[0] as i32 - 128).abs() <= 1);
        assert!((dst_buf.data_v[0] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn argb_to_i420_white() {
        let mut argb = vec![0u8; 64];
        for i in (0..64).step_by(4) {
            argb[i] = 255;
            argb[i + 1] = 255;
            argb[i + 2] = 255;
            argb[i + 3] = 255;
        }

        let dst_buf = make_i420(4, 4);
        let dst_ref = i420_ref(&dst_buf);

        NativeTransform::argb_to_i420(&argb, 4, 4, dst_ref).unwrap();
        assert!((dst_buf.data_y[0] as i32 - 235).abs() <= 1);
        assert!((dst_buf.data_u[0] as i32 - 128).abs() <= 1);
    }

    // ── i420_to_argb ────────────────────────────────────

    #[test]
    fn i420_to_argb_roundtrip_black() {
        let mut src_buf = make_i420(4, 4);
        src_buf.data_y.fill(16);
        src_buf.data_u.fill(128);
        src_buf.data_v.fill(128);

        let mut argb_out = vec![0u8; 64];
        let src_ref = i420_ref(&src_buf);

        NativeTransform::i420_to_argb(src_ref, 4, 4, PixelFormat::ARGB, &mut argb_out).unwrap();

        for i in (0..64).step_by(4) {
            assert_eq!(argb_out[i], 255);
            assert!(argb_out[i + 1] <= 5);
            assert!(argb_out[i + 2] <= 5);
            assert!(argb_out[i + 3] <= 5);
        }
    }

    #[test]
    fn i420_to_argb_bgra_format() {
        let mut src_buf = make_i420(4, 4);
        src_buf.data_y.fill(16);
        src_buf.data_u.fill(128);
        src_buf.data_v.fill(128);

        let mut argb_out = vec![0u8; 64];
        let src_ref = i420_ref(&src_buf);

        NativeTransform::i420_to_argb(src_ref, 4, 4, PixelFormat::BGRA, &mut argb_out).unwrap();
        assert_eq!(argb_out[3], 255);
        assert!(argb_out[0] <= 5);
        assert!(argb_out[1] <= 5);
        assert!(argb_out[2] <= 5);
    }

    #[test]
    fn i420_to_argb_unsupported_format() {
        let src_buf = make_i420(4, 4);
        let src_ref = i420_ref(&src_buf);
        let mut dst = vec![0u8; 64];

        let result = NativeTransform::i420_to_argb(src_ref, 4, 4, PixelFormat::NV12, &mut dst);
        assert!(result.is_err());
    }

    // ── i420 ↔ nv12 roundtrip ───────────────────────────

    #[test]
    fn i420_nv12_roundtrip() {
        let mut src_buf = make_i420(8, 8);
        for i in 0..64u8 {
            src_buf.data_y[i as usize] = i;
        }
        src_buf.data_u.fill(50);
        src_buf.data_v.fill(150);

        let mut nv12_y = vec![0u8; 64];
        let mut nv12_uv = vec![0u8; 32];
        let src_ref = i420_ref(&src_buf);
        NativeTransform::i420_to_nv12(src_ref, 8, 8, &mut nv12_y, &mut nv12_uv).unwrap();

        assert_eq!(&nv12_y[..64], &src_buf.data_y[..64]);

        let dst_buf = make_i420(8, 8);
        let dst_ref = i420_ref(&dst_buf);
        NativeTransform::nv12_to_i420(&nv12_y, &nv12_uv, 8, 8, dst_ref).unwrap();

        assert_eq!(dst_buf.data_y, src_buf.data_y);
        assert_eq!(dst_buf.data_u, src_buf.data_u);
        assert_eq!(dst_buf.data_v, src_buf.data_v);
    }

    // ── T5.4 argb_to_i420 roundtrip ────────────────────

    #[test]
    fn argb_to_i420_roundtrip_loss_within_tolerance() {
        // Create a 4×4 BGRA image with uniform 2×2 quadrants.
        // Each 2×2 block has one color (matches 4:2:0 chroma subsampling).
        let mut input = vec![0u8; 64];
        // pixel(x,y) = quad_colors[y/2 * 2 + x/2]
        let quad_colors: [(u8, u8, u8); 4] = [
            (255, 0, 0),     // top-left: red
            (0, 255, 0),     // top-right: green
            (0, 0, 255),     // bottom-left: blue
            (128, 128, 128), // bottom-right: gray
        ];
        for y in 0..4u32 {
            for x in 0..4u32 {
                let qi = (y / 2 * 2 + x / 2) as usize;
                let (r, g, b) = quad_colors[qi];
                let off = ((y * 4 + x) * 4) as usize;
                input[off] = b;
                input[off + 1] = g;
                input[off + 2] = r;
                input[off + 3] = 255;
            }
        }

        // BGRA → I420
        let i420_buf = make_i420(4, 4);
        let dst_ref = i420_ref(&i420_buf);
        NativeTransform::argb_to_i420(&input, 4, 4, dst_ref).unwrap();

        // I420 → BGRA (same byte order as input)
        let mut output = vec![0u8; 64];
        let src_ref = i420_ref(&i420_buf);
        NativeTransform::i420_to_argb(src_ref, 4, 4, PixelFormat::BGRA, &mut output).unwrap();

        // Round-trip through YUV 4:2:0 is lossy.
        // Input and output are both BGRA: byte0=B, byte1=G, byte2=R, byte3=A.
        // Compare R,G,B per pixel with ±15 tolerance.
        for px in 0..16usize {
            let off = px * 4;
            let orig_b = input[off] as i32;
            let orig_g = input[off + 1] as i32;
            let orig_r = input[off + 2] as i32;

            let out_b = output[off] as i32;
            let out_g = output[off + 1] as i32;
            let out_r = output[off + 2] as i32;

            // ponytail: 4:2:0 chroma subsampling loses precision
            assert!((orig_r - out_r).abs() <= 15, "R diff too large at px {}", px);
            assert!((orig_g - out_g).abs() <= 15, "G diff too large at px {}", px);
            assert!((orig_b - out_b).abs() <= 15, "B diff too large at px {}", px);
        }
    }

    // ── T5.3 backend consistency (native vs libyuv) ────

    /// Compare native and libyuv scale results.
    /// libyuv may not be linkable; ignored by default.
    #[test]
    #[ignore = "libyuv shared library may not be available at link time"]
    #[cfg(all(feature = "backend-native", feature = "backend-yuv-sys"))]
    fn native_and_libyuv_scale_match() {
        let src_buf = make_i420(16, 16);
        let dst_native = make_i420(8, 8);
        let dst_libyuv = make_i420(8, 8);

        let src_ref = i420_ref(&src_buf);
        let dst_ref_n = i420_ref(&dst_native);
        let dst_ref_l = i420_ref(&dst_libyuv);

        NativeTransform::scale(src_ref, 16, 16, dst_ref_n, 8, 8).unwrap();
        crate::backend::LibyuvTransform::scale(src_ref, 16, 16, dst_ref_l, 8, 8).unwrap();

        // Both backends should produce identical outputs for nearest-neighbor scale
        assert_eq!(dst_native.data_y, dst_libyuv.data_y);
        assert_eq!(dst_native.data_u, dst_libyuv.data_u);
        assert_eq!(dst_native.data_v, dst_libyuv.data_v);
    }

    #[test]
    #[ignore = "libyuv shared library may not be available at link time"]
    #[cfg(all(feature = "backend-native", feature = "backend-yuv-sys"))]
    fn native_and_libyuv_i420_nv12_roundtrip_match() {
        let mut src_buf = make_i420(8, 8);
        for i in 0..64u8 {
            src_buf.data_y[i as usize] = i;
        }
        src_buf.data_u.fill(50);
        src_buf.data_v.fill(150);

        // Native path
        let mut nv12_y_n = vec![0u8; 64];
        let mut nv12_uv_n = vec![0u8; 32];
        let src_ref = i420_ref(&src_buf);
        NativeTransform::i420_to_nv12(src_ref, 8, 8, &mut nv12_y_n, &mut nv12_uv_n).unwrap();

        // Libyuv path (fresh buffer references)
        let mut nv12_y_l = vec![0u8; 64];
        let mut nv12_uv_l = vec![0u8; 32];
        let src_ref2 = i420_ref(&src_buf);
        crate::backend::LibyuvTransform::i420_to_nv12(
            src_ref2,
            8,
            8,
            &mut nv12_y_l,
            &mut nv12_uv_l,
        )
        .unwrap();

        assert_eq!(nv12_y_n, nv12_y_l);
        assert_eq!(nv12_uv_n, nv12_uv_l);
    }
}
