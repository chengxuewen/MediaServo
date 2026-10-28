//! End-to-end test: VideoFrameGenerator → sink → frame queue.
//!
//! Verifies that the generator produces frames, delivers them to a sink,
//! and the sink correctly captures I420→RGBA conversion.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::error::MediaError;
use mediaservo_media::pipeline::generator::{
    Anchor, BitmapFont, PatternMode, SquaresConfig, TextBurner, TimestampFormat, TimestampOverlay,
    VideoFrameGenerator,
};
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};
use mediaservo_media::pipeline::source::VideoSource;
use mediaservo_media::pixel_format::PixelFormat;

#[cfg(feature = "backend-native")]
use mediaservo_media::backend::NativeTransform;
use mediaservo_media::transform::VideoTransform;

#[cfg(feature = "backend-yuv-sys")]
use mediaservo_media::backend::LibyuvTransform;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FPS: u32 = 30;

/// Sink that captures RGBA frames into a shared queue.
struct CaptureSink {
    queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    width: u32,
    height: u32,
}

impl VideoSink<BoxVideoFrame> for CaptureSink {
    fn on_frame(&self, frame: &BoxVideoFrame) -> Result<VideoSinkWants, MediaError> {
        if let Some(i420_ref) = frame.buffer.as_i420_ref() {
            let mut rgba = vec![0u8; (self.width * self.height * 4) as usize];

            #[cfg(feature = "backend-yuv-sys")]
            let result = LibyuvTransform::i420_to_argb(
                i420_ref,
                self.width,
                self.height,
                PixelFormat::RGBA,
                &mut rgba,
            );
            #[cfg(all(feature = "backend-native", not(feature = "backend-yuv-sys")))]
            let result = NativeTransform::i420_to_argb(
                i420_ref,
                self.width,
                self.height,
                PixelFormat::RGBA,
                &mut rgba,
            );

            if let Err(e) = result {
                eprintln!("i420_to_argb failed: {e:?}");
                return Ok(VideoSinkWants::default());
            }
            self.queue.lock().unwrap().push_back(rgba);
        }
        Ok(VideoSinkWants::default())
    }
}

fn squares_config() -> SquaresConfig {
    SquaresConfig { count: 30, motion_speed: 2, ..Default::default() }
}

/// Verify generator produces frames and sink receives them.
#[test]
fn generator_produces_frames() {
    let generator = VideoFrameGenerator::new();
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    let sink = CaptureSink { queue: queue.clone(), width: WIDTH, height: HEIGHT };

    generator.add_or_update_sink(
        Box::new(sink),
        VideoSinkWants { is_active: true, ..Default::default() },
    );

    generator.start(FPS, PatternMode::Squares(squares_config()), None, WIDTH, HEIGHT);

    // Wait for at least 3 frames (100ms per frame at 30fps, wait up to 2s)
    let timeout = Duration::from_secs(2);
    let start = std::time::Instant::now();

    loop {
        let count = queue.lock().unwrap().len();
        if count >= 3 {
            break;
        }
        if start.elapsed() > timeout {
            panic!("Timed out waiting for frames: got {} frames in {:?}", count, start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    generator.stop();

    let frames: Vec<Vec<u8>> = {
        let mut q = queue.lock().unwrap();
        (0..q.len()).map(|_| q.pop_front().unwrap()).collect()
    };

    assert!(frames.len() >= 3, "Expected at least 3 frames, got {}", frames.len());

    // Verify each frame has correct RGBA byte size
    let expected_size = (WIDTH * HEIGHT * 4) as usize;
    for (i, frame) in frames.iter().enumerate() {
        assert_eq!(
            frame.len(),
            expected_size,
            "Frame {} has wrong size: {} != {}",
            i,
            frame.len(),
            expected_size
        );
    }

    // Verify frames are not all identical (generator should produce varying output)
    if frames.len() >= 2 {
        assert!(frames[0] != frames[frames.len() - 1], "Frames should differ over time");
    }
}

/// Verify generator stops cleanly.
#[test]
fn generator_stop_releases_thread() {
    let generator = VideoFrameGenerator::new();
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    let sink = CaptureSink { queue, width: WIDTH, height: HEIGHT };

    generator.add_or_update_sink(
        Box::new(sink),
        VideoSinkWants { is_active: true, ..Default::default() },
    );

    generator.start(FPS, PatternMode::Squares(squares_config()), None, WIDTH, HEIGHT);
    std::thread::sleep(Duration::from_millis(200));
    generator.stop();

    // If stop() didn't deadlock, we pass.
    // generator is dropped here — if thread is still running, JoinHandle will panic on drop.
}

/// Verify VideoSinkWants::is_active=false skips the sink.
#[test]
fn inactive_sink_receives_no_frames() {
    let generator = VideoFrameGenerator::new();
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    let sink = CaptureSink { queue: queue.clone(), width: WIDTH, height: HEIGHT };

    generator.add_or_update_sink(
        Box::new(sink),
        VideoSinkWants {
            is_active: false, // explicitly inactive
            ..Default::default()
        },
    );

    generator.start(FPS, PatternMode::Squares(squares_config()), None, WIDTH, HEIGHT);
    std::thread::sleep(Duration::from_millis(500));
    generator.stop();

    let count = queue.lock().unwrap().len();
    assert_eq!(count, 0, "Inactive sink should receive 0 frames, got {}", count);
}

/// Verify frames with timestamp overlay differ between captures.
#[test]
fn frames_differ_with_timestamp_overlay() {
    let generator = VideoFrameGenerator::new();
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    let sink = CaptureSink { queue: queue.clone(), width: WIDTH, height: HEIGHT };

    generator.add_or_update_sink(
        Box::new(sink),
        VideoSinkWants { is_active: true, ..Default::default() },
    );

    let font = BitmapFont::new();
    let burner = TextBurner::new(font, false, Anchor::TopLeft);
    let overlay = TimestampOverlay::new(burner, TimestampFormat::Combined);

    generator.start(FPS, PatternMode::Squares(squares_config()), Some(overlay), WIDTH, HEIGHT);

    let timeout = Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let count = queue.lock().unwrap().len();
        if count >= 3 {
            break;
        }
        if start.elapsed() > timeout {
            panic!("Timed out waiting for frames");
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    generator.stop();

    let frames: Vec<Vec<u8>> = {
        let mut q = queue.lock().unwrap();
        (0..q.len()).map(|_| q.pop_front().unwrap()).collect()
    };

    assert!(frames.len() >= 3, "Expected at least 3 frames, got {}", frames.len());

    // All frames should differ — timestamp changes every frame
    // ponytail: compare adjacent frames instead of all-pairs
    for i in 1..frames.len() {
        assert!(
            frames[i - 1] != frames[i],
            "Frame {} and {} should differ due to timestamp overlay + motion",
            i - 1,
            i
        );
    }
}
