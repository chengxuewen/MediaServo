// ── Generator — multi-mode video frame generator ───────

pub mod fonts;
pub mod smpte_bars;
pub mod squares;
pub mod timestamp;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// Re-exports
pub use fonts::{Anchor, BitmapFont, TextBurner};
pub use smpte_bars::SmpteBarsPattern;
pub use squares::{ColorStrategy, SquaresConfig, SquaresPattern};
pub use timestamp::{TimestampFormat, TimestampOverlay};

use super::sink::{SinkId, VideoSink, VideoSinkWants};
use super::source::VideoSource;
use crate::base::buffer::I420Buffer;
use crate::base::buffer::VideoBuffer;
use crate::base::frame::{BoxVideoFrame, VideoFrame};

// ── FramePattern trait ──────────────────────────────────

/// Draws a pattern into pre-allocated I420 plane buffers.
///
/// Implementors are called once per generated frame from the generator thread.
/// The caller guarantees that plane buffers match `width`×`height` and that
/// stride values correctly describe the memory layout.
pub trait FramePattern: Send + Sync {
    /// Fill the three I420 planes with pixel data.
    ///
    /// `y`, `u`, `v` are mutable slices into the respective planes.
    /// `stride_*` gives the row stride (bytes per row) for each plane.
    /// `width` and `height` are the logical frame dimensions.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        y: &mut [u8],
        u: &mut [u8],
        v: &mut [u8],
        stride_y: usize,
        stride_u: usize,
        stride_v: usize,
        width: u32,
        height: u32,
    );
}

// ── PatternMode ─────────────────────────────────────────

/// Which pattern to generate. Used by [`VideoFrameGenerator::start`].
pub enum PatternMode {
    /// Colored squares with configurable count, size, color strategy, and motion.
    Squares(SquaresConfig),
    /// SMPTE 75% color bars plus grayscale gradient.
    SmpteBars,
}

// ── VideoFrameGenerator ─────────────────────────────────

/// A video source that generates frames on a dedicated thread.
///
/// Each frame uses a [`FramePattern`] to paint into an [`I420Buffer`],
/// which is then wrapped in a [`BoxVideoFrame`] and broadcast to all
/// registered sinks.
pub struct VideoFrameGenerator {
    #[allow(clippy::type_complexity)]
    sinks: Arc<Mutex<Vec<(SinkId, Box<dyn VideoSink<BoxVideoFrame>>, VideoSinkWants)>>>,
    running: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    next_id: AtomicU64,
}

impl Default for VideoFrameGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoFrameGenerator {
    pub fn new() -> Self {
        Self {
            sinks: Arc::new(Mutex::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
            next_id: AtomicU64::new(0),
        }
    }

    /// Start the generation thread with a pattern mode and optional overlay.
    ///
    /// Spawns a new OS thread that:
    /// 1. Creates an [`I420Buffer`] each iteration.
    /// 2. Calls `pattern.draw()` to fill it.
    /// 3. Applies the [`TimestampOverlay`] if provided.
    /// 4. Wraps the buffer in a [`BoxVideoFrame`] with a monotonic timestamp.
    /// 5. Broadcasts the frame to all active sinks.
    /// 6. Sleeps for the frame interval, with drift compensation.
    ///
    /// # Panics
    ///
    /// Panics if the generator is already running (check via `is_running()` first).
    pub fn start(
        &self,
        fps: u32,
        mode: PatternMode,
        overlay: Option<TimestampOverlay>,
        width: u32,
        height: u32,
    ) {
        let mut guard = self.thread.lock().unwrap();
        if guard.is_some() || self.running.load(Ordering::Relaxed) {
            drop(guard);
            panic!("VideoFrameGenerator already running");
        }

        let sinks = Arc::clone(&self.sinks);
        let running = Arc::clone(&self.running);
        self.running.store(true, Ordering::SeqCst);

        // Create the pattern from the mode
        let mut pattern: Box<dyn FramePattern> = match mode {
            PatternMode::Squares(config) => {
                Box::new(SquaresPattern::with_config(width, height, config))
            }
            PatternMode::SmpteBars => Box::new(SmpteBarsPattern::new()),
        };

        let handle = thread::Builder::new()
            .name("omsp-video-gen".into())
            .spawn(move || {
                let frame_duration = Duration::from_secs_f64(1.0 / fps as f64);
                let mut next_frame_time = Instant::now();
                let mut frame_id: u32 = 0;

                while running.load(Ordering::Relaxed) {
                    // ── Create frame ─────────────────
                    let mut buf = I420Buffer::new(width, height);
                    let bw = buf.width();
                    let bh = buf.height();
                    pattern.draw(
                        &mut buf.data_y,
                        &mut buf.data_u,
                        &mut buf.data_v,
                        buf.stride_y as usize,
                        buf.stride_u as usize,
                        buf.stride_v as usize,
                        bw,
                        bh,
                    );

                    // Apply timestamp overlay if provided
                    if let Some(ref ovl) = overlay {
                        ovl.burn_i420(
                            &mut buf.data_y,
                            &mut buf.data_u,
                            &mut buf.data_v,
                            buf.stride_y as usize,
                            buf.stride_u as usize,
                            buf.stride_v as usize,
                            timestamp_us(),
                            frame_id,
                            bw,
                            bh,
                        );
                    }

                    let vb: Box<dyn VideoBuffer> = Box::new(buf);
                    let frame = VideoFrame::new(vb).with_timestamp(timestamp_us());
                    frame_id = frame_id.wrapping_add(1);

                    // ── Broadcast ────────────────────
                    if let Ok(guard) = sinks.lock() {
                        for (_id, sink, wants) in guard.iter() {
                            if wants.is_active {
                                // ponytail: ignore per-sink errors; a bad sink
                                // shouldn't kill the generator thread
                                let _ = sink.on_frame(&frame);
                            }
                        }
                    }

                    // ── Frame pacing with drift compensation
                    let now = Instant::now();
                    if now < next_frame_time {
                        thread::sleep(next_frame_time - now);
                    }
                    next_frame_time += frame_duration;

                    // ponytail: reset clock if we fell behind by more than one frame
                    if next_frame_time <= Instant::now() {
                        next_frame_time = Instant::now() + frame_duration;
                    }
                }
            })
            .expect("failed to spawn generator thread");

        *guard = Some(handle);
    }

    /// Stop the generation thread and wait for it to exit.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.thread.lock()
            && let Some(handle) = guard.take()
        {
            drop(guard);
            // ponytail: join outside the lock to avoid potential deadlock
            let _ = handle.join();
        }
    }

    /// Check whether the generator thread is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Number of currently registered sinks.
    pub fn sink_count(&self) -> usize {
        self.sinks.lock().map(|g| g.len()).unwrap_or(0)
    }
}

/// v2 (planned video-source-unification BLOCKER-4): stop the generator thread on Drop,
/// preventing 30fps idle CPU burn after the frame source's lifetime ends. stop() is idempotent; join happens outside the lock (deadlock-free).
impl Drop for VideoFrameGenerator {
    fn drop(&mut self) {
        self.stop();
    }
}

impl VideoSource<BoxVideoFrame> for VideoFrameGenerator {
    fn add_or_update_sink(
        &self,
        sink: Box<dyn VideoSink<BoxVideoFrame>>,
        wants: VideoSinkWants,
    ) -> SinkId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut guard) = self.sinks.lock() {
            guard.push((id, sink, wants));
        }
        id
    }

    fn remove_sink(&self, id: SinkId) {
        if let Ok(mut guard) = self.sinks.lock() {
            guard.retain(|(sid, _, _)| *sid != id);
        }
    }
}

// ── Helpers ─────────────────────────────────────────────

/// Monotonic timestamp in microseconds.
fn timestamp_us() -> i64 {
    // ponytail: use Instant-based monotonic counter instead of wall clock
    use std::sync::atomic::AtomicI64;
    use std::time::UNIX_EPOCH;
    static BASE: AtomicI64 = AtomicI64::new(-1);
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

    let start = START.get_or_init(Instant::now);
    if BASE.load(Ordering::Relaxed) < 0 {
        let sys_us = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        BASE.store(sys_us, Ordering::Relaxed);
    }
    BASE.load(Ordering::Relaxed) + start.elapsed().as_micros() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::base::frame::BoxVideoFrame;
    use crate::error::MediaError;
    use crate::pipeline::sink::{VideoSink, VideoSinkWants};
    /// Sink that records how many frames it received.
    struct CountingSink {
        count: Arc<Mutex<u32>>,
        wants: VideoSinkWants,
    }

    impl VideoSink<BoxVideoFrame> for CountingSink {
        fn on_frame(&self, _frame: &BoxVideoFrame) -> Result<VideoSinkWants, MediaError> {
            *self.count.lock().unwrap() += 1;
            Ok(self.wants)
        }
    }

    #[test]
    fn generator_produces_frames_within_timeout() {
        let generator = VideoFrameGenerator::new();
        assert!(!generator.is_running());
        assert_eq!(generator.sink_count(), 0);

        let count = Arc::new(Mutex::new(0u32));
        let sink =
            Box::new(CountingSink { count: count.clone(), wants: VideoSinkWants::default() });
        generator.add_or_update_sink(sink, VideoSinkWants::default());
        assert_eq!(generator.sink_count(), 1);

        // Start at 60 fps with a 16×16 frame in SmpteBars mode
        generator.start(60, PatternMode::SmpteBars, None, 16, 16);
        assert!(generator.is_running());

        // Wait 200ms — at 60fps we expect ~12 frames
        std::thread::sleep(Duration::from_millis(200));

        generator.stop();
        assert!(!generator.is_running());

        let frames = *count.lock().unwrap();
        // ponytail: scheduler jitter means we might get fewer frames,
        // but anything >= 5 in 200ms proves the generator works
        assert!(frames >= 5, "expected at least 5 frames in 200ms at 60fps, got {}", frames);
    }

    #[test]
    #[should_panic(expected = "already running")]
    fn generator_double_start_panics() {
        let generator = VideoFrameGenerator::new();
        generator.start(10, PatternMode::SmpteBars, None, 4, 4);
        // Second start should panic
        generator.start(10, PatternMode::SmpteBars, None, 4, 4);
    }

    #[test]
    fn squares_mode_creates_configured_pattern() {
        let generator = VideoFrameGenerator::new();
        let config = SquaresConfig { count: 3, ..Default::default() };
        generator.start(10, PatternMode::Squares(config), None, 32, 24);
        assert!(generator.is_running());
        generator.stop();
    }
}
