use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::error::MediaError;
use mediaservo_media::pipeline::generator::{PatternMode, SquaresConfig, VideoFrameGenerator};
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};
use mediaservo_media::pipeline::source::VideoSource;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FPS: u32 = 30;

/// Sink that atomically counts frames and detects colored pixels.
struct StatsSink {
    frame_count: Arc<AtomicU64>,
    has_color: Arc<AtomicBool>,
}

impl VideoSink<BoxVideoFrame> for StatsSink {
    fn on_frame(&self, frame: &BoxVideoFrame) -> Result<VideoSinkWants, MediaError> {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        if let Ok(i420) = frame.buffer.to_i420() {
            // ponytail: color = any non-gray chroma (U/V != 128)
            let has_non_gray =
                i420.data_u.iter().any(|&p| p != 128) || i420.data_v.iter().any(|&p| p != 128);
            if has_non_gray {
                self.has_color.store(true, Ordering::Relaxed);
            }
        }
        Ok(VideoSinkWants::default())
    }
}

#[test]
fn generator_produces_colored_frames() {
    let generator = VideoFrameGenerator::new();
    let frame_count = Arc::new(AtomicU64::new(0));
    let has_color = Arc::new(AtomicBool::new(false));

    let sink = StatsSink { frame_count: frame_count.clone(), has_color: has_color.clone() };

    generator.add_or_update_sink(Box::new(sink), VideoSinkWants::default());
    let config = SquaresConfig { count: 5, ..Default::default() };
    generator.start(FPS, PatternMode::Squares(config), None, WIDTH, HEIGHT);

    std::thread::sleep(Duration::from_secs(1));
    generator.stop();

    let count = frame_count.load(Ordering::Relaxed);
    let colored = has_color.load(Ordering::Relaxed);

    assert!(count > 0, "expected at least 1 frame, got {}", count);
    assert!(colored, "expected colored pixels in generator output");
}

#[test]
fn generator_double_stop_is_safe() {
    let generator = VideoFrameGenerator::new();
    let config = SquaresConfig { count: 2, ..Default::default() };
    generator.start(30, PatternMode::Squares(config), None, 32, 32);
    std::thread::sleep(Duration::from_millis(100));
    generator.stop();
    generator.stop(); // should not panic
}

#[test]
#[should_panic(expected = "already running")]
fn generator_double_start_panics() {
    let generator = VideoFrameGenerator::new();
    let config = SquaresConfig { count: 2, ..Default::default() };
    generator.start(10, PatternMode::Squares(config), None, 32, 32);
    generator.start(10, PatternMode::SmpteBars, None, 32, 32);
}
