//! V4L2 camera backend (Linux) — open/negotiate → mmap stream → compact I420.
//!
//! Data flow (design camera-capture-v4l2):
//! ```text
//! V4l2Backend::open(path) → querycap → enum_formats → set_format → set_params(fps)
//!   → mmap::Stream::new → read_frame(): next() → (buf, meta)
//!     → ts = TimestampMapper::map(kernel_ts)   (Momus H1 anchor, never resampled)
//!     → libyuv YUY2ToI420 / NV12ToI420 (src stride = Format.stride)
//!     → compact I420 (dst stride == width) → CapturedFrame
//! ```
//! The mmap buffer is only valid until the next `next()` call, so the raw
//! frame is synchronously copied into the compact output here (C5 boundary).

use std::io;
use std::time::Duration;

use v4l::buffer::{Flags as BufferFlags, Type};
use v4l::capability::Flags as CapFlags;
use v4l::device::Device;
use v4l::format::{FourCC, Format};
use v4l::fraction::Fraction;
use v4l::io::mmap;
use v4l::io::traits::CaptureStream;
use v4l::video::capture::Parameters as CaptureParameters;
use v4l::video::Capture;

use super::{CaptureBackend, CaptureError, CapturedFrame, TimestampMapper};

/// Preferred capture pixel formats, in negotiation order (first supported wins).
/// YUYV (UVC) before NV12 (mipi/imx185) — see design decision 5.
pub const PREFERRED_FOURCC: [FourCC; 2] = [
    FourCC { repr: *b"YUYV" },
    FourCC { repr: *b"NV12" },
];

const YUYV: FourCC = FourCC { repr: *b"YUYV" };
const NV12: FourCC = FourCC { repr: *b"NV12" };

// ── Format negotiation (pure, unit-tested) ───────────────

/// Pick the first preferred FourCC the device supports and build the S_FMT
/// request. Returns an error listing the available formats when none match.
pub fn select_format(
    supported: &[FourCC],
    width: u32,
    height: u32,
    preferred: &[FourCC],
) -> Result<Format, String> {
    let fmt_str = |f: &FourCC| f.str().unwrap_or("????").to_string();
    let fourcc = preferred
        .iter()
        .find(|f| supported.contains(f))
        .copied()
        .ok_or_else(|| {
            let wanted = preferred.iter().map(fmt_str).collect::<Vec<_>>().join(", ");
            let available = supported.iter().map(fmt_str).collect::<Vec<_>>().join(", ");
            format!(
                "preferred formats [{wanted}] not supported; device offers: [{available}]"
            )
        })?;
    Ok(Format::new(width, height, fourcc))
}

/// Actual frames-per-second from negotiated capture parameters
/// (interval = num/denom seconds → fps = denom/num).
fn actual_fps(params: &CaptureParameters) -> u32 {
    let f = params.interval;
    if f.numerator == 0 {
        0
    } else {
        f.denominator / f.numerator
    }
}

// ── V4l2Backend ──────────────────────────────────────────

/// Pull-based V4L2 capture backend. Holds the open device and mmap stream;
/// `open()` is idempotent (EIO reconnect re-opens without resampling the
/// timestamp anchor — [`TimestampMapper`] lives on the instance).
pub struct V4l2Backend {
    path: String,
    width: u32,
    height: u32,
    fps: u32,
    ts_mapper: TimestampMapper,
    device: Option<Device>,
    stream: Option<mmap::Stream<'static>>,
    negotiated: Option<Format>,
    negotiated_fps: Option<u32>,
}

impl V4l2Backend {
    /// Create a backend for `path` (e.g. `/dev/video0`) requesting the given
    /// geometry. The timestamp anchor is sampled here — once per instance,
    /// never inside `open()` (Momus H1).
    pub fn new(path: impl Into<String>, width: u32, height: u32, fps: u32) -> Self {
        Self {
            path: path.into(),
            width,
            height,
            fps,
            ts_mapper: TimestampMapper::sample_now(),
            device: None,
            stream: None,
            negotiated: None,
            negotiated_fps: None,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Open/negotiate the device. Idempotent — drops previous device/stream
    /// state, so an EIO reconnect is just `open()` again.
    fn open(&mut self) -> Result<(), CaptureError> {
        self.device = None;
        self.stream = None;
        self.negotiated = None;
        self.negotiated_fps = None;

        let dev = Device::with_path(&self.path)
            .map_err(|e| CaptureError::Open { path: self.path.clone(), source: e })?;

        let caps = dev
            .query_caps()
            .map_err(|e| CaptureError::QueryCap { path: self.path.clone(), source: e })?;
        if !caps.capabilities.contains(CapFlags::VIDEO_CAPTURE) {
            return Err(CaptureError::Format {
                path: self.path.clone(),
                message: format!("device is not a video capture device (card: {})", caps.card),
            });
        }

        let supported: Vec<FourCC> = dev
            .enum_formats()
            .map_err(|e| CaptureError::Format { path: self.path.clone(), message: e.to_string() })?
            .into_iter()
            .map(|d| d.fourcc)
            .collect();
        let fmt = select_format(&supported, self.width, self.height, &PREFERRED_FOURCC)
            .map_err(|message| CaptureError::Format { path: self.path.clone(), message })?;

        // S_FMT — driver may adjust; the return value is authoritative.
        let actual = dev
            .set_format(&fmt)
            .map_err(|e| CaptureError::Format { path: self.path.clone(), message: e.to_string() })?;

        // S_PARM — frame interval; tegra-video 驱动不支持（实证: VIDIOC_S_PARM
        // Inappropriate ioctl）→ 降级 warn 不阻断（帧率由传感器/ISP 固定）；
        // UVC 支持则正常设置。fps 失配 warn（C17；实机对照 v4l2-ctl --get-parm）。
        let params = match dev.set_params(&CaptureParameters::with_fps(self.fps)) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    path = %self.path,
                    error = %e,
                    "S_PARM 不支持（tegra 驱动固定帧率）——降级，fps 按驱动默认"
                );
                // 降级: interval 0/0 = 驱动默认帧率
                CaptureParameters::new(Fraction::new(0, 0))
            }
        };
        let fps = actual_fps(&params);
        if fps != self.fps && fps != 0 {
            tracing::warn!(
                path = %self.path,
                configured = self.fps,
                actual = fps,
                "fps mismatch after set_params — 实机对照见 T8 (v4l2-ctl --get-parm)"
            );
        }

        let stream = mmap::Stream::new(&dev, Type::VideoCapture)
            .map_err(|e| CaptureError::Stream { path: self.path.clone(), source: e })?;

        tracing::info!(
            path = %self.path,
            fourcc = %actual.fourcc,
            width = actual.width,
            height = actual.height,
            stride = actual.stride,
            fps,
            "capture device negotiated"
        );
        self.device = Some(dev);
        self.stream = Some(stream);
        self.negotiated = Some(actual);
        self.negotiated_fps = Some(fps);
        Ok(())
    }

    /// Block until the next frame; converts to compact I420 synchronously.
    fn read_frame(&mut self) -> Result<CapturedFrame, CaptureError> {
        let not_open = || CaptureError::Stream {
            path: self.path.clone(),
            source: io::Error::new(io::ErrorKind::NotConnected, "device not open"),
        };
        let stream = self.stream.as_mut().ok_or_else(not_open)?;
        let fmt = self.negotiated.as_ref().ok_or_else(not_open)?;

        let (bytes, meta) = stream.next().map_err(|e| CaptureError::Stream {
            path: self.path.clone(),
            source: e,
        })?;
        // Defensive: never read past the mmap buffer even if bytesused lies.
        let used = meta.bytesused.min(bytes.len() as u32) as usize;
        let data = convert_to_compact_i420(&bytes[..used], fmt.fourcc, fmt.width, fmt.height, fmt.stride)?;

        let ts = if meta.flags.contains(BufferFlags::TIMESTAMP_MONOTONIC) {
            self.ts_mapper.map(Duration::from(meta.timestamp))
        } else {
            tracing::debug!(
                path = %self.path,
                "buffer lacks TIMESTAMP_MONOTONIC flag — falling back to Instant::now()"
            );
            self.ts_mapper.map_now()
        };
        Ok(CapturedFrame {
            ts_mono_ns: ts,
            width: fmt.width,
            height: fmt.height,
            fps: self.negotiated_fps.unwrap_or(self.fps),
            data,
        })
    }
}

impl CaptureBackend for V4l2Backend {
    fn open(&mut self) -> Result<(), CaptureError> {
        self.open()
    }

    fn read_frame(&mut self) -> Result<CapturedFrame, CaptureError> {
        self.read_frame()
    }
}

// ── Frame conversion → compact I420 ──────────────────────

/// Convert a raw V4L buffer (YUYV or NV12, possibly padded stride) to compact
/// I420 (`stride == width`) — the FrameBus wire layout. Padding is dropped
/// via libyuv dst strides == width (one pass, no second copy).
/// MJPG and other FourCCs fail explicitly (C36 — no silent fallback).
fn convert_to_compact_i420(
    raw: &[u8],
    fourcc: FourCC,
    width: u32,
    height: u32,
    stride: u32,
) -> Result<Vec<u8>, CaptureError> {
    let (w, h) = (width as usize, height as usize);
    let stride = stride as usize;
    if fourcc == YUYV {
        yuyv_to_compact_i420(raw, w, h, stride)
    } else if fourcc == NV12 {
        nv12_to_compact_i420(raw, w, h, stride)
    } else {
        Err(CaptureError::Unsupported(format!(
            "FourCC {} 解码未实现（Phase 3 支持范围；当前仅 YUYV/NV12）",
            fourcc.str().unwrap_or("????")
        )))
    }
}

fn yuyv_to_compact_i420(raw: &[u8], w: usize, h: usize, stride: usize) -> Result<Vec<u8>, CaptureError> {
    let needed = stride * h;
    if raw.len() < needed {
        return Err(CaptureError::Convert(format!(
            "YUYV buffer too small: {} bytes < needed {needed} ({}x{} stride {stride})",
            raw.len(),
            w,
            h
        )));
    }
    let y_len = w * h;
    let c_len = w * h / 4;
    let mut out = vec![0u8; y_len + 2 * c_len];
    // SAFETY: `raw` covers stride×h (checked above); `out` covers compact
    // I420 planes (y_len + 2×c_len); libyuv reads only within src rows and
    // writes only within dst rows.
    let r = unsafe {
        yuv_sys::rs_YUY2ToI420(
            raw.as_ptr(),
            stride as i32,
            out.as_mut_ptr(),
            w as i32,
            out.as_mut_ptr().add(y_len),
            (w / 2) as i32,
            out.as_mut_ptr().add(y_len + c_len),
            (w / 2) as i32,
            w as i32,
            h as i32,
        )
    };
    if r != 0 {
        return Err(CaptureError::Convert(format!("YUY2ToI420 failed with code {r}")));
    }
    Ok(out)
}

fn nv12_to_compact_i420(raw: &[u8], w: usize, h: usize, stride: usize) -> Result<Vec<u8>, CaptureError> {
    let y_len = stride * h;
    let uv_offset = y_len; // UV plane offset = stride×h (NOT width×h — padded)
    let needed = uv_offset + stride * (h / 2);
    if raw.len() < needed {
        return Err(CaptureError::Convert(format!(
            "NV12 buffer too small: {} bytes < needed {needed} ({}x{} stride {stride})",
            raw.len(),
            w,
            h
        )));
    }
    let out_y = w * h;
    let out_c = w * h / 4;
    let mut out = vec![0u8; out_y + 2 * out_c];
    // SAFETY: `raw` covers stride×h + stride×(h/2) interleaved UV (checked
    // above); `out` covers compact I420 planes; libyuv reads/writes rows only.
    let r = unsafe {
        yuv_sys::rs_NV12ToI420(
            raw.as_ptr(),
            stride as i32,
            raw.as_ptr().add(uv_offset),
            stride as i32,
            out.as_mut_ptr(),
            w as i32,
            out.as_mut_ptr().add(out_y),
            (w / 2) as i32,
            out.as_mut_ptr().add(out_y + out_c),
            (w / 2) as i32,
            w as i32,
            h as i32,
        )
    };
    if r != 0 {
        return Err(CaptureError::Convert(format!("NV12ToI420 failed with code {r}")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fourcc_list(codes: &[FourCC]) -> Vec<FourCC> {
        codes.to_vec()
    }
    const YUYV: FourCC = FourCC { repr: *b"YUYV" };
    const NV12: FourCC = FourCC { repr: *b"NV12" };
    const MJPG: FourCC = FourCC { repr: *b"MJPG" };


    // ── select_format (T3) ─────────────────────────────────

    #[test]
    fn negotiation_prefers_yuyv_then_nv12() {
        let supported = fourcc_list(&[NV12, YUYV]);
        let fmt = select_format(&supported, 1920, 1080, &PREFERRED_FOURCC).expect("YUYV supported");
        assert_eq!(fmt.fourcc, YUYV);
        assert_eq!((fmt.width, fmt.height), (1920, 1080));

        let supported = fourcc_list(&[NV12, MJPG]);
        let fmt = select_format(&supported, 640, 480, &PREFERRED_FOURCC).expect("NV12 supported");
        assert_eq!(fmt.fourcc, NV12);
    }

    #[test]
    fn negotiation_reports_available_formats_on_failure() {
        let supported = fourcc_list(&[MJPG]);
        let err = select_format(&supported, 640, 480, &PREFERRED_FOURCC).unwrap_err();
        assert!(err.contains("YUYV"), "err lists preferred: {err}");
        assert!(err.contains("MJPG"), "err lists available: {err}");
    }

    #[test]
    fn negotiation_fails_on_empty_supported_list() {
        let err = select_format(&[], 640, 480, &PREFERRED_FOURCC).unwrap_err();
        assert!(err.contains("offers: []"), "err: {err}");
    }

    #[test]
    fn actual_fps_from_interval() {
        let params = CaptureParameters::with_fps(30);
        assert_eq!(actual_fps(&params), 30);
    }

    // ── YUYV conversion (T5) ───────────────────────────────

    #[test]
    fn yuyv_solid_color_block_converts_to_known_i420() {
        // 4x2, stride == width*2 (compact YUYV), constant chroma — output
        // unaffected by libyuv's 2-row chroma averaging.
        let mut raw = vec![0u8; 16];
        for (i, px) in raw.chunks_exact_mut(4).enumerate() {
            let _ = i;
            px.copy_from_slice(&[76, 84, 76, 255]); // Y U Y V per pixel pair
        }
        let out = convert_to_compact_i420(&raw, YUYV, 4, 2, 8).expect("converts");
        assert_eq!(out.len(), CapturedFrame::i420_size(4, 2)); // 12 bytes
        assert!(out[..8].iter().all(|&b| b == 76), "Y plane");
        assert!(out[8..10].iter().all(|&b| b == 84), "U plane");
        assert!(out[10..12].iter().all(|&b| b == 255), "V plane");
    }

    #[test]
    fn yuyv_stride_with_padding_produces_compact_i420() {
        // 6x2, stride 16 (12 used + 4 pad bytes/row). Byte layout per row:
        // [Y0 U0 Y1 V0 Y2 U1 Y3 V1 Y4 U2 Y5 V2 | P P P P] — chroma is shared
        // per pixel pair (U at pair*4+1, V at pair*4+3). Padding = 0xFF must
        // NOT leak into the output.
        let mut raw = vec![0xFFu8; 32];
        for row in 0..2 {
            for px in 0..6 {
                raw[row * 16 + px * 2] = (row * 6 + px) as u8; // Y
            }
            for pair in 0..3 {
                raw[row * 16 + pair * 4 + 1] = 84; // U
                raw[row * 16 + pair * 4 + 3] = 255; // V
            }
        }
        let out = convert_to_compact_i420(&raw, YUYV, 6, 2, 16).expect("converts");
        assert_eq!(out.len(), CapturedFrame::i420_size(6, 2)); // 18 bytes
        let expected_y: Vec<u8> = (0..12).collect();
        assert_eq!(&out[..12], expected_y.as_slice(), "Y plane rows, padding dropped");
        assert_eq!(&out[12..15], &[84, 84, 84], "U plane");
        assert_eq!(&out[15..18], &[255, 255, 255], "V plane");
    }

    #[test]
    fn yuyv_short_buffer_is_rejected() {
        let err = convert_to_compact_i420(&[0u8; 10], YUYV, 6, 2, 16).unwrap_err();
        assert!(matches!(err, CaptureError::Convert(_)), "got {err:?}");
    }

    // ── NV12 conversion (T5) ───────────────────────────────

    #[test]
    fn nv12_solid_color_block_converts_to_known_i420() {
        // 4x2, stride 4 (compact): Y 8 bytes, then interleaved UV 4 bytes.
        let mut raw = vec![76u8; 12];
        for (i, px) in raw[8..].chunks_exact_mut(2).enumerate() {
            let _ = i;
            px.copy_from_slice(&[84, 255]);
        }
        let out = convert_to_compact_i420(&raw, NV12, 4, 2, 4).expect("converts");
        assert_eq!(out.len(), CapturedFrame::i420_size(4, 2));
        assert!(out[..8].iter().all(|&b| b == 76), "Y plane");
        assert!(out[8..10].iter().all(|&b| b == 84), "U plane");
        assert!(out[10..12].iter().all(|&b| b == 255), "V plane");
    }

    #[test]
    fn nv12_stride_with_padding_produces_compact_i420() {
        // 6x2, stride 16: Y rows 6+10 pad, UV plane at stride×h = 32.
        // UV plane offset must be stride×h (32), NOT width×h (12).
        let mut raw = vec![0xFFu8; 32 + 16];
        for row in 0..2 {
            for px in 0..6 {
                raw[row * 16 + px] = (row * 6 + px) as u8; // Y
            }
        }
        for px in 0..3 {
            raw[32 + px * 2] = 84; // U interleaved
            raw[32 + px * 2 + 1] = 255; // V
        }
        let out = convert_to_compact_i420(&raw, NV12, 6, 2, 16).expect("converts");
        assert_eq!(out.len(), CapturedFrame::i420_size(6, 2)); // 18 bytes
        let expected_y: Vec<u8> = (0..12).collect();
        assert_eq!(&out[..12], expected_y.as_slice(), "Y plane, padding dropped");
        assert_eq!(&out[12..15], &[84, 84, 84], "U plane");
        assert_eq!(&out[15..18], &[255, 255, 255], "V plane");
    }

    #[test]
    fn nv12_short_buffer_is_rejected() {
        // width 6, stride 16 → needs 32 + 16 = 48 bytes
        let err = convert_to_compact_i420(&[0u8; 32], NV12, 6, 2, 16).unwrap_err();
        assert!(matches!(err, CaptureError::Convert(_)), "got {err:?}");
    }

    // ── Unsupported (C36) ─────────────────────────────────

    #[test]
    fn mjpg_fails_explicitly_as_phase_3() {
        let err = convert_to_compact_i420(&[0u8; 64], MJPG, 640, 480, 1280).unwrap_err();
        assert!(
            matches!(&err, CaptureError::Unsupported(msg) if msg.contains("Phase 3")),
            "got {err:?}"
        );
    }
}
