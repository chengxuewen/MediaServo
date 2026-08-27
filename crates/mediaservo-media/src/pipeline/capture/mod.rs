//! Camera capture backends — pull-based device semantics.
//!
//! [`CaptureBackend`] is the media-side abstraction over raw capture devices
//! (V4L2 cameras today, desktop/subscriber sources later). It complements the
//! existing [`super::source::VideoSource`] (push/broadcast, used by the
//! generator): `CaptureBackend` is a pull interface for device frames.
//!
//! Frames are delivered as **compact I420** (`stride == width`), the same
//! layout the FrameBus wire format assumes — the generator and camera paths
//! produce identical payloads, so streamers/sinks never see a difference.

#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
pub mod v4l2;

use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A single captured frame in compact I420 (`stride == width`).
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    /// Epoch-anchored capture time in nanoseconds (C17 anchor; see [`TimestampMapper`]).
    pub ts_mono_ns: u64,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Configured capture rate in frames per second.
    pub fps: u32,
    /// Compact I420 planes: Y (w×h), U (w×h/4), V (w×h/4), concatenated.
    pub data: Vec<u8>,
}

impl CapturedFrame {
    /// Size of a compact I420 buffer for the given dimensions.
    pub const fn i420_size(width: u32, height: u32) -> usize {
        (width * height * 3 / 2) as usize
    }
}

/// Capture backend errors. Every variant carries the device path for
/// context (C15 — callers log the error; no silent swallowing).
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("open capture device {path} failed: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("query capability failed on {path}: {source}")]
    QueryCap {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("format negotiation failed on {path}: {message}")]
    Format { path: String, message: String },
    #[error("streaming failed on {path}: {source}")]
    Stream {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("frame conversion failed: {0}")]
    Convert(String),
    #[error("unsupported capture format: {0}")]
    Unsupported(String),
}

/// Pull-based capture backend for a single device/source.
///
/// Implementors must be `Send` (capturer runs a single frame loop thread;
/// no intra-crate protocol, no cross-crate usage).
pub trait CaptureBackend: Send {
    /// Open/negotiate the device. Idempotent: calling again re-opens from
    /// scratch (used for EIO reconnect — the timestamp anchor is NOT
    /// resampled, see [`TimestampMapper`]).
    fn open(&mut self) -> Result<(), CaptureError>;
    /// Block until the next frame is available.
    fn read_frame(&mut self) -> Result<CapturedFrame, CaptureError>;
}

/// Maps v4l2 kernel timestamps (CLOCK_MONOTONIC, seconds since boot) into the
/// epoch domain used by FrameBus (`ts_mono_ns`).
///
/// Momus H1: `anchor_epoch = SystemTime::now() - Instant::now()` sampled
/// **once per backend instance** (process-level), never inside `open()` — so
/// EIO reconnects keep timestamps continuous. `Instant` and the v4l2 kernel
/// clock are the same clock (CLOCK_MONOTONIC since boot), so `anchor_epoch`
/// is that clock's epoch (boot time) in the SystemTime domain — read from
/// `/proc/uptime`. Then `anchor_epoch + kernel_ts` is the true wall-clock
/// capture time. The forbidden alternative — `SystemTime::now() + kernel_ts`
/// — double-counts the uptime and inherits NTP jumps.
#[derive(Debug, Clone, Copy)]
pub struct TimestampMapper {
    /// CLOCK_MONOTONIC epoch (boot time) in the SystemTime domain.
    anchor_epoch: SystemTime,
    /// Instant sampled together with `anchor_epoch`.
    anchor_instant: Instant,
    /// System uptime at sampling (`SystemTime::now() - anchor_epoch`).
    anchor_uptime: Duration,
}

impl TimestampMapper {
    /// Sample the anchor once per backend instance (NOT per open).
    pub fn sample_now() -> Self {
        let anchor_instant = Instant::now();
        let anchor_epoch = clock_monotonic_epoch().unwrap_or_else(|| {
            // ponytail: /proc/uptime unreadable (non-Linux) — anchor ≈ now;
            // mapping degrades to a relative estimate. v4l2 is Linux-only, so
            // this path is effectively unreachable.
            SystemTime::now() - anchor_instant.elapsed()
        });
        let anchor_uptime = SystemTime::now()
            .duration_since(anchor_epoch)
            .unwrap_or_default();
        Self {
            anchor_epoch,
            anchor_instant,
            anchor_uptime,
        }
    }

    /// Map a v4l2 kernel timestamp (CLOCK_MONOTONIC since boot) to epoch ns.
    pub fn map(&self, kernel_ts: Duration) -> u64 {
        self.to_epoch_ns(self.anchor_epoch + kernel_ts)
    }

    /// Fallback for drivers without `V4L2_BUF_FLAG_TIMESTAMP_MONOTONIC`:
    /// map the current monotonic time to epoch ns.
    pub fn map_now(&self) -> u64 {
        let uptime = self.anchor_uptime + self.anchor_instant.elapsed();
        self.to_epoch_ns(self.anchor_epoch + uptime)
    }

    fn to_epoch_ns(&self, t: SystemTime) -> u64 {
        t.duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}

/// CLOCK_MONOTONIC epoch (boot time) in the SystemTime domain, from
/// `/proc/uptime` (first field — monotonic seconds since boot, 10ms
/// resolution). This is the exact value of `SystemTime::now() - uptime`
/// that `SystemTime::now() - Instant::now()` denotes.
pub(crate) fn clock_monotonic_epoch() -> Option<SystemTime> {
    let uptime = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()?;
    Some(SystemTime::now() - Duration::from_secs_f64(uptime))
}

/// Fake backend for unit tests: replays a configurable frame sequence and
/// optionally injects open/read failures.
pub struct FakeBackend {
    frames: VecDeque<CapturedFrame>,
    open_error: Option<CaptureError>,
    read_error: Option<CaptureError>,
    opened: bool,
    reads: usize,
}

impl FakeBackend {
    /// Backend replaying the given frames in order; exhausted reads return
    /// a `Stream` error (simulating device end).
    pub fn new(frames: Vec<CapturedFrame>) -> Self {
        Self {
            frames: frames.into(),
            open_error: None,
            read_error: None,
            opened: false,
            reads: 0,
        }
    }

    /// Inject a failure at `open()`.
    pub fn fail_open(err: CaptureError) -> Self {
        Self {
            frames: VecDeque::new(),
            open_error: Some(err),
            read_error: None,
            opened: false,
            reads: 0,
        }
    }

    /// Inject a failure at `read_frame()` (e.g. simulated EIO).
    pub fn fail_read(err: CaptureError) -> Self {
        Self {
            frames: VecDeque::new(),
            open_error: None,
            read_error: Some(err),
            opened: false,
            reads: 0,
        }
    }

    /// Number of successful `read_frame` calls (test observation).
    pub fn reads(&self) -> usize {
        self.reads
    }
}

impl CaptureBackend for FakeBackend {
    fn open(&mut self) -> Result<(), CaptureError> {
        if let Some(err) = self.open_error.take() {
            return Err(err);
        }
        self.opened = true;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<CapturedFrame, CaptureError> {
        if !self.opened {
            return Err(CaptureError::Stream {
                path: "<fake>".into(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "read_frame before open",
                ),
            });
        }
        if let Some(err) = self.read_error.take() {
            return Err(err);
        }
        let frame = self.frames.pop_front().ok_or_else(|| CaptureError::Stream {
            path: "<fake>".into(),
            source: std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "frame sequence exhausted"),
        })?;
        self.reads += 1;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ts: u64) -> CapturedFrame {
        CapturedFrame {
            ts_mono_ns: ts,
            width: 4,
            height: 2,
            fps: 30,
            data: vec![0u8; CapturedFrame::i420_size(4, 2)],
        }
    }

    // ── Trait contract (FakeBackend) ─────────────────────────

    #[test]
    fn fake_backend_replays_frame_sequence_in_order() {
        let frames = vec![frame(100), frame(200), frame(300)];
        let mut backend = FakeBackend::new(frames);
        backend.open().expect("open ok");

        let f0 = backend.read_frame().expect("frame 0");
        assert_eq!(f0.ts_mono_ns, 100);
        let f1 = backend.read_frame().expect("frame 1");
        assert_eq!(f1.ts_mono_ns, 200);
        let f2 = backend.read_frame().expect("frame 2");
        assert_eq!(f2.ts_mono_ns, 300);

        // shape assertions
        assert_eq!((f2.width, f2.height, f2.fps), (4, 2, 30));
        assert_eq!(f2.data.len(), CapturedFrame::i420_size(4, 2));
        assert_eq!(backend.reads(), 3);
    }

    #[test]
    fn fake_backend_read_before_open_fails() {
        let mut backend = FakeBackend::new(vec![frame(1)]);
        let err = backend.read_frame().unwrap_err();
        assert!(matches!(err, CaptureError::Stream { .. }), "got {err:?}");
    }

    #[test]
    fn fake_backend_open_error_propagates() {
        let err = CaptureError::Open {
            path: "/dev/video0".into(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such device"),
        };
        let mut backend = FakeBackend::fail_open(err);
        let got = backend.open().unwrap_err();
        assert!(
            matches!(got, CaptureError::Open { ref path, .. } if path == "/dev/video0"),
            "got {got:?}"
        );
    }

    #[test]
    fn fake_backend_read_error_propagates_once() {
        let mut backend = FakeBackend::fail_read(CaptureError::Convert("boom".into()));
        backend.open().expect("open ok");
        let got = backend.read_frame().unwrap_err();
        assert!(matches!(got, CaptureError::Convert(msg) if msg == "boom"));
    }

    #[test]
    fn fake_backend_exhausted_sequence_fails() {
        let mut backend = FakeBackend::new(vec![frame(1)]);
        backend.open().expect("open ok");
        let _ = backend.read_frame().expect("first frame");
        let err = backend.read_frame().unwrap_err();
        assert!(matches!(err, CaptureError::Stream { .. }), "got {err:?}");
    }

    // ── Timestamp mapping (Momus H1) ─────────────────────────

    #[test]
    fn kernel_ts_maps_into_process_lifetime_window() {
        // Realistic kernel ts = CLOCK_MONOTONIC uptime at capture, which must
        // lie within [uptime@process_start, uptime_now]. Mapped (anchor + kts)
        // must therefore land within [proc_start, now] (Momus H1 interval
        // assertion — monotonicity alone cannot catch the offset bug).
        let proc_start = SystemTime::now();
        let mapper = TimestampMapper::sample_now();
        // /proc/uptime has 10ms resolution (mapper and test may read it
        // 20ms apart) — keep every mapped value ≥100ms away from both bounds.
        std::thread::sleep(Duration::from_secs(1));
        let now = SystemTime::now();
        let proc_start_ns = proc_start.duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
        let now_ns = now.duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;

        let boot = clock_monotonic_epoch().expect("boot time from /proc/uptime");
        let uptime_now = now.duration_since(boot).expect("uptime positive");
        let kts_values = [
            uptime_now - Duration::from_millis(200),
            uptime_now - Duration::from_millis(500),
            uptime_now - Duration::from_millis(800),
        ];
        for kts in kts_values {
            let mapped = mapper.map(kts);
            assert!(
                mapped >= proc_start_ns,
                "kernel_ts {kts:?} mapped {mapped} < process start {proc_start_ns}"
            );
            assert!(
                mapped <= now_ns,
                "kernel_ts {kts:?} mapped {mapped} > now {now_ns}"
            );
        }
    }

    #[test]
    fn kernel_ts_mapping_is_monotonic_in_kernel_ts() {
        let mapper = TimestampMapper::sample_now();
        let t0 = mapper.map(Duration::ZERO);
        let t1 = mapper.map(Duration::from_secs(1));
        let t2 = mapper.map(Duration::from_secs(2));
        assert!(t0 < t1 && t1 < t2, "{t0} < {t1} < {t2}");
        // 1s apart in kernel clock ⇒ 1s apart in mapped domain
        assert_eq!(t1 - t0, 1_000_000_000);
        assert_eq!(t2 - t1, 1_000_000_000);
    }

    #[test]
    fn map_now_falls_within_lifetime_window() {
        let mapper = TimestampMapper::sample_now();
        let mapped = mapper.map_now();
        let proc_start_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        // map_now ≈ current wall time (allow small clock skew backward)
        assert!(
            mapped.abs_diff(proc_start_ns) < 5_000_000_000,
            "map_now {mapped} drifted from now {proc_start_ns}"
        );
    }

    #[test]
    fn i420_size_matches_y_plus_two_chroma_planes() {
        assert_eq!(CapturedFrame::i420_size(1920, 1080), 1920 * 1080 * 3 / 2);
        assert_eq!(CapturedFrame::i420_size(4, 2), 12);
    }
}
