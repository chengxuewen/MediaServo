use crate::error::MediaResult;
use crate::pixel_format::PixelFormat;
use std::fmt::Debug;

// ── I420BufferRef ────────────────────────────────────────

/// Borrowed zero-copy view of I420 plane data.
///
/// Uses raw-pointer fields so backends can both read (src) and write (dst)
/// through a single type. Callers guarantee that dst pointers are writable.
/// `Copy` + passed by value to avoid double indirection in hot paths.
#[derive(Copy, Clone)]
pub struct I420BufferRef<'a> {
    pub y_ptr: *mut u8,
    pub y_len: usize,
    pub u_ptr: *mut u8,
    pub u_len: usize,
    pub v_ptr: *mut u8,
    pub v_len: usize,
    pub stride_y: usize,
    pub stride_u: usize,
    pub stride_v: usize,
    _phantom: std::marker::PhantomData<&'a [u8]>,
}

// ── I420Buffer ───────────────────────────────────────────

/// Canonical 3-plane YUV 4:2:0 8-bit buffer.
#[derive(Clone)]
pub struct I420Buffer {
    pub data_y: Vec<u8>,
    pub data_u: Vec<u8>,
    pub data_v: Vec<u8>,
    pub stride_y: u32,
    pub stride_u: u32,
    pub stride_v: u32,
    width: u32,
    height: u32,
}

impl Debug for I420Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("I420Buffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl I420Buffer {
    /// Create a zero-filled I420 buffer with given dimensions.
    /// Strides equal `width` (Y) and `width / 2` (U, V).
    pub fn new(width: u32, height: u32) -> Self {
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;
        Self {
            data_y: vec![0u8; y_size],
            data_u: vec![128u8; uv_size],
            data_v: vec![128u8; uv_size],
            stride_y: width,
            stride_u: width / 2,
            stride_v: width / 2,
            width,
            height,
        }
    }

    /// Nearest-neighbor scale to new dimensions.
    pub fn scale(&self, new_width: u32, new_height: u32) -> I420Buffer {
        let mut out = I420Buffer::new(new_width, new_height);
        let src_w = self.width;
        let src_h = self.height;

        // Scale Y plane
        for oy in 0..new_height {
            let sy = (oy as u64 * src_h as u64 / new_height as u64) as usize;
            let src_off = sy * self.stride_y as usize;
            let dst_off = oy as usize * out.stride_y as usize;
            for ox in 0..new_width {
                let sx = (ox as u64 * src_w as u64 / new_width as u64) as usize;
                out.data_y[dst_off + ox as usize] = self.data_y[src_off + sx];
            }
        }

        // Scale U plane
        let half_src_w = src_w / 2;
        let half_src_h = src_h / 2;
        let half_dst_w = new_width / 2;
        let half_dst_h = new_height / 2;
        for oy in 0..half_dst_h {
            let sy = (oy as u64 * half_src_h as u64 / half_dst_h as u64) as usize;
            let src_off = sy * self.stride_u as usize;
            let dst_off = oy as usize * out.stride_u as usize;
            for ox in 0..half_dst_w {
                let sx = (ox as u64 * half_src_w as u64 / half_dst_w as u64) as usize;
                out.data_u[dst_off + ox as usize] = self.data_u[src_off + sx];
            }
        }

        // Scale V plane
        for oy in 0..half_dst_h {
            let sy = (oy as u64 * half_src_h as u64 / half_dst_h as u64) as usize;
            let src_off = sy * self.stride_v as usize;
            let dst_off = oy as usize * out.stride_v as usize;
            for ox in 0..half_dst_w {
                let sx = (ox as u64 * half_src_w as u64 / half_dst_w as u64) as usize;
                out.data_v[dst_off + ox as usize] = self.data_v[src_off + sx];
            }
        }

        out
    }
}

impl VideoBuffer for I420Buffer {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn format(&self) -> PixelFormat {
        PixelFormat::I420
    }

    fn as_i420_ref(&self) -> Option<I420BufferRef<'_>> {
        Some(I420BufferRef {
            y_ptr: self.data_y.as_ptr() as *mut u8,
            y_len: self.data_y.len(),
            u_ptr: self.data_u.as_ptr() as *mut u8,
            u_len: self.data_u.len(),
            v_ptr: self.data_v.as_ptr() as *mut u8,
            v_len: self.data_v.len(),
            stride_y: self.stride_y as usize,
            stride_u: self.stride_u as usize,
            stride_v: self.stride_v as usize,
            _phantom: std::marker::PhantomData,
        })
    }

    fn to_i420(&self) -> MediaResult<I420Buffer> {
        Ok(self.clone())
    }

    fn as_i420(&self) -> Option<&I420Buffer> {
        Some(self)
    }
}

// ── I422Buffer ───────────────────────────────────────────

/// 3-plane YUV 4:2:2 buffer — full-width U/V, half-height chroma.
#[derive(Clone)]
pub struct I422Buffer {
    pub data_y: Vec<u8>,
    pub data_u: Vec<u8>,
    pub data_v: Vec<u8>,
    pub stride_y: u32,
    pub stride_u: u32,
    pub stride_v: u32,
    width: u32,
    height: u32,
}

impl Debug for I422Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("I422Buffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl VideoBuffer for I422Buffer {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn format(&self) -> PixelFormat {
        PixelFormat::I422
    }

    fn as_i420_ref(&self) -> Option<I420BufferRef<'_>> {
        None
    }

    fn to_i420(&self) -> MediaResult<I420Buffer> {
        let mut out = I420Buffer::new(self.width, self.height);
        let y_size = (self.width * self.height) as usize;
        out.data_y[..y_size].copy_from_slice(&self.data_y[..y_size]);

        let half_w = (self.width / 2) as usize;
        let half_h = (self.height / 2) as usize;
        let u_stride = self.stride_u as usize;
        let v_stride = self.stride_v as usize;
        let out_u_stride = out.stride_u as usize;
        let out_v_stride = out.stride_v as usize;

        // ponytail: horizontal 2:1 averaging — U/V planes go from width×h/2 → width/2×h/2
        for y in 0..half_h {
            let src_u = y * u_stride;
            let src_v = y * v_stride;
            let dst_u = y * out_u_stride;
            let dst_v = y * out_v_stride;
            for x in 0..half_w {
                out.data_u[dst_u + x] = ((self.data_u[src_u + 2 * x] as u16
                    + self.data_u[src_u + 2 * x + 1] as u16)
                    / 2) as u8;
                out.data_v[dst_v + x] = ((self.data_v[src_v + 2 * x] as u16
                    + self.data_v[src_v + 2 * x + 1] as u16)
                    / 2) as u8;
            }
        }

        Ok(out)
    }

    fn as_i422(&self) -> Option<&I422Buffer> {
        Some(self)
    }
}

// ── I444Buffer ───────────────────────────────────────────

/// 3-plane YUV 4:4:4 buffer — full-resolution chroma.
#[derive(Clone)]
pub struct I444Buffer {
    pub data_y: Vec<u8>,
    pub data_u: Vec<u8>,
    pub data_v: Vec<u8>,
    pub stride_y: u32,
    pub stride_u: u32,
    pub stride_v: u32,
    width: u32,
    height: u32,
}

impl Debug for I444Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("I444Buffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl VideoBuffer for I444Buffer {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn format(&self) -> PixelFormat {
        PixelFormat::I444
    }

    fn as_i420_ref(&self) -> Option<I420BufferRef<'_>> {
        None
    }

    fn to_i420(&self) -> MediaResult<I420Buffer> {
        let mut out = I420Buffer::new(self.width, self.height);
        let y_size = (self.width * self.height) as usize;
        out.data_y[..y_size].copy_from_slice(&self.data_y[..y_size]);

        let half_w = (self.width / 2) as usize;
        let half_h = (self.height / 2) as usize;
        let u_stride = self.stride_u as usize;
        let v_stride = self.stride_v as usize;
        let out_u_stride = out.stride_u as usize;
        let out_v_stride = out.stride_v as usize;

        // ponytail: average 2×2 blocks for UV downsampling
        for y in 0..half_h {
            let src_u_r0 = (2 * y) * u_stride;
            let src_u_r1 = (2 * y + 1) * u_stride;
            let src_v_r0 = (2 * y) * v_stride;
            let src_v_r1 = (2 * y + 1) * v_stride;
            let dst_u = y * out_u_stride;
            let dst_v = y * out_v_stride;
            for x in 0..half_w {
                let u00 = self.data_u[src_u_r0 + 2 * x] as u16;
                let u01 = self.data_u[src_u_r0 + 2 * x + 1] as u16;
                let u10 = self.data_u[src_u_r1 + 2 * x] as u16;
                let u11 = self.data_u[src_u_r1 + 2 * x + 1] as u16;
                out.data_u[dst_u + x] = ((u00 + u01 + u10 + u11) / 4) as u8;

                let v00 = self.data_v[src_v_r0 + 2 * x] as u16;
                let v01 = self.data_v[src_v_r0 + 2 * x + 1] as u16;
                let v10 = self.data_v[src_v_r1 + 2 * x] as u16;
                let v11 = self.data_v[src_v_r1 + 2 * x + 1] as u16;
                out.data_v[dst_v + x] = ((v00 + v01 + v10 + v11) / 4) as u8;
            }
        }

        Ok(out)
    }

    fn as_i444(&self) -> Option<&I444Buffer> {
        Some(self)
    }
}

// ── NV12Buffer ───────────────────────────────────────────

/// Biplanar YUV 4:2:0 buffer — full-resolution Y, interleaved UV.
#[derive(Clone)]
pub struct NV12Buffer {
    pub data_y: Vec<u8>,
    pub data_uv: Vec<u8>,
    pub stride_y: u32,
    pub stride_uv: u32,
    width: u32,
    height: u32,
}

impl Debug for NV12Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NV12Buffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl VideoBuffer for NV12Buffer {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn format(&self) -> PixelFormat {
        PixelFormat::NV12
    }

    fn as_i420_ref(&self) -> Option<I420BufferRef<'_>> {
        None
    }

    fn to_i420(&self) -> MediaResult<I420Buffer> {
        let mut out = I420Buffer::new(self.width, self.height);
        let y_size = (self.width * self.height) as usize;
        out.data_y[..y_size].copy_from_slice(&self.data_y[..y_size]);

        let half_w = (self.width / 2) as usize;
        let half_h = (self.height / 2) as usize;
        let uv_stride = self.stride_uv as usize;
        let out_uv_stride = out.stride_u as usize; // u_stride == v_stride in I420

        // ponytail: de-interleave UVUV into separate U and V planes
        for y in 0..half_h {
            let src = y * uv_stride;
            let dst = y * out_uv_stride;
            for x in 0..half_w {
                out.data_u[dst + x] = self.data_uv[src + 2 * x];
                out.data_v[dst + x] = self.data_uv[src + 2 * x + 1];
            }
        }

        Ok(out)
    }

    fn as_nv12(&self) -> Option<&NV12Buffer> {
        Some(self)
    }
}

// ── I010Buffer ───────────────────────────────────────────

/// 10-bit planar YUV 4:2:0 buffer — same geometry as I420, `u16` per sample.
#[derive(Clone)]
pub struct I010Buffer {
    pub data_y: Vec<u16>,
    pub data_u: Vec<u16>,
    pub data_v: Vec<u16>,
    pub stride_y: u32,
    pub stride_u: u32,
    pub stride_v: u32,
    width: u32,
    height: u32,
}

impl Debug for I010Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("I010Buffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl VideoBuffer for I010Buffer {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn format(&self) -> PixelFormat {
        PixelFormat::I010
    }

    fn as_i420_ref(&self) -> Option<I420BufferRef<'_>> {
        None
    }

    fn to_i420(&self) -> MediaResult<I420Buffer> {
        let mut out = I420Buffer::new(self.width, self.height);
        let y_size = (self.width * self.height) as usize;
        let uv_size = ((self.width / 2) * (self.height / 2)) as usize;

        // ponytail: right-shift 2 truncates 10→8 bits
        for i in 0..y_size {
            out.data_y[i] = (self.data_y[i] >> 2) as u8;
        }
        for i in 0..uv_size {
            out.data_u[i] = (self.data_u[i] >> 2) as u8;
            out.data_v[i] = (self.data_v[i] >> 2) as u8;
        }

        Ok(out)
    }

    fn as_i010(&self) -> Option<&I010Buffer> {
        Some(self)
    }
}

// ── VideoBuffer trait ────────────────────────────────────

/// Unified buffer interface for all pixel formats.
///
/// Every concrete buffer (I420, NV12, etc.) implements this trait.
/// Backend processing works through `as_i420_ref()` for zero-copy
/// access; format conversion goes through `to_i420()`.
pub trait VideoBuffer: Debug + Send + Sync {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn format(&self) -> PixelFormat;

    /// Zero-copy borrow of I420 planes for backend processing.
    ///
    /// Returns `None` if the buffer cannot be viewed as I420
    /// without a copy (e.g. packed RGB formats).
    fn as_i420_ref(&self) -> Option<I420BufferRef<'_>>;

    /// Convert to I420 (center format). Always succeeds for
    /// any format — may allocate and copy.
    fn to_i420(&self) -> MediaResult<I420Buffer>;

    // ── Downcast helpers ─────────────────────────────
    // Default implementations return None. Concrete buffer
    // types override their own variant.

    fn as_i420(&self) -> Option<&I420Buffer> {
        None
    }
    fn as_i422(&self) -> Option<&I422Buffer> {
        None
    }
    fn as_i444(&self) -> Option<&I444Buffer> {
        None
    }
    fn as_nv12(&self) -> Option<&NV12Buffer> {
        None
    }
    fn as_i010(&self) -> Option<&I010Buffer> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ────────────────────────────────────────

    fn make_i422(w: u32, h: u32) -> I422Buffer {
        I422Buffer {
            data_y: vec![0u8; (w * h) as usize],
            data_u: vec![128u8; (w * (h / 2)) as usize],
            data_v: vec![128u8; (w * (h / 2)) as usize],
            stride_y: w,
            stride_u: w,
            stride_v: w,
            width: w,
            height: h,
        }
    }

    fn make_i444(w: u32, h: u32) -> I444Buffer {
        I444Buffer {
            data_y: vec![0u8; (w * h) as usize],
            data_u: vec![128u8; (w * h) as usize],
            data_v: vec![128u8; (w * h) as usize],
            stride_y: w,
            stride_u: w,
            stride_v: w,
            width: w,
            height: h,
        }
    }

    fn make_nv12(w: u32, h: u32) -> NV12Buffer {
        NV12Buffer {
            data_y: vec![0u8; (w * h) as usize],
            data_uv: vec![128u8; (w * (h / 2)) as usize],
            stride_y: w,
            stride_uv: w,
            width: w,
            height: h,
        }
    }

    fn make_i010(w: u32, h: u32) -> I010Buffer {
        I010Buffer {
            data_y: vec![0u16; (w * h) as usize],
            data_u: vec![512u16; ((w / 2) * (h / 2)) as usize],
            data_v: vec![512u16; ((w / 2) * (h / 2)) as usize],
            stride_y: w,
            stride_u: w / 2,
            stride_v: w / 2,
            width: w,
            height: h,
        }
    }

    // ── T5.1 buffer round-trip ─────────────────────────

    #[test]
    fn i420_new_creates_correct_plane_sizes() {
        let buf = I420Buffer::new(16, 8);
        assert_eq!(buf.data_y.len(), 128); // 16*8
        assert_eq!(buf.data_u.len(), 32); // (16/2)*(8/2) = 8*4
        assert_eq!(buf.data_v.len(), 32);
        assert_eq!(buf.stride_y, 16);
        assert_eq!(buf.stride_u, 8);
        assert_eq!(buf.stride_v, 8);
        assert_eq!(buf.width(), 16);
        assert_eq!(buf.height(), 8);
        assert_eq!(buf.format(), PixelFormat::I420);
    }

    #[test]
    fn i420_new_default_fill() {
        let buf = I420Buffer::new(8, 8);
        // Y plane: zero-filled (black)
        assert!(buf.data_y.iter().all(|&v| v == 0));
        // U/V planes: 128 (gray chroma = no color bias)
        assert!(buf.data_u.iter().all(|&v| v == 128));
        assert!(buf.data_v.iter().all(|&v| v == 128));
    }

    #[test]
    fn i422_to_i420_preserves_dimensions() {
        let buf = make_i422(8, 8);
        let result = buf.to_i420().unwrap();
        assert_eq!(result.width(), 8);
        assert_eq!(result.height(), 8);
        assert_eq!(result.format(), PixelFormat::I420);
    }

    #[test]
    fn i444_to_i420_preserves_dimensions() {
        let buf = make_i444(8, 8);
        let result = buf.to_i420().unwrap();
        assert_eq!(result.width(), 8);
        assert_eq!(result.height(), 8);
        assert_eq!(result.format(), PixelFormat::I420);
    }

    #[test]
    fn nv12_to_i420_preserves_dimensions() {
        let buf = make_nv12(8, 8);
        let result = buf.to_i420().unwrap();
        assert_eq!(result.width(), 8);
        assert_eq!(result.height(), 8);
        assert_eq!(result.format(), PixelFormat::I420);
    }

    #[test]
    fn i010_to_i420_preserves_dimensions() {
        let buf = make_i010(8, 8);
        let result = buf.to_i420().unwrap();
        assert_eq!(result.width(), 8);
        assert_eq!(result.height(), 8);
        assert_eq!(result.format(), PixelFormat::I420);
    }

    #[test]
    fn i420_scale_produces_correct_output_dimensions() {
        let src = I420Buffer::new(16, 16);

        let half = src.scale(8, 8);
        assert_eq!(half.width(), 8);
        assert_eq!(half.height(), 8);

        let quarter = src.scale(4, 4);
        assert_eq!(quarter.width(), 4);
        assert_eq!(quarter.height(), 4);

        let double = src.scale(32, 32);
        assert_eq!(double.width(), 32);
        assert_eq!(double.height(), 32);
    }

    #[test]
    fn i420_scale_identity_returns_same_size() {
        let src = I420Buffer::new(10, 6);
        let result = src.scale(10, 6);
        assert_eq!(result.width(), 10);
        assert_eq!(result.height(), 6);
        assert_eq!(result.data_y.len(), 60);
    }

    // ── format() correctness ───────────────────────────

    #[test]
    fn buffer_formats_match_pixel_format() {
        assert_eq!(I420Buffer::new(4, 4).format(), PixelFormat::I420);
        assert_eq!(make_i422(4, 4).format(), PixelFormat::I422);
        assert_eq!(make_i444(4, 4).format(), PixelFormat::I444);
        assert_eq!(make_nv12(4, 4).format(), PixelFormat::NV12);
        assert_eq!(make_i010(4, 4).format(), PixelFormat::I010);
    }

    // ── downcast helpers ───────────────────────────────

    #[test]
    fn i420_as_downcasts_return_some() {
        let buf = I420Buffer::new(4, 4);
        let vb: &dyn VideoBuffer = &buf;
        assert!(vb.as_i420().is_some());
        assert!(vb.as_i422().is_none());
        assert!(vb.as_i444().is_none());
        assert!(vb.as_nv12().is_none());
        assert!(vb.as_i010().is_none());
    }

    #[test]
    fn i422_as_downcasts_return_some() {
        let buf = make_i422(4, 4);
        let vb: &dyn VideoBuffer = &buf;
        assert!(vb.as_i420().is_none());
        assert!(vb.as_i422().is_some());
    }

    #[test]
    fn i010_to_i420_truncates_10bit() {
        let mut buf = make_i010(4, 4);
        // Fill with 10-bit values that truncate predictably
        buf.data_y.fill(0x2A8); // >>2 = 170 (0xAA)
        let result = buf.to_i420().unwrap();
        assert_eq!(result.data_y[0], 0xAA);
        assert_eq!(result.data_y[15], 0xAA);
    }
}
