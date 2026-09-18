//! Square video frame generator — smoke test example.
//!
//! Creates a `VideoFrameGenerator` with `SquaresPattern`, runs for 2 seconds,
//! collects frame statistics via a `StatsSink`, and prints results.
//!
//! Usage:
//!   cargo run -p mediaservo-media --example square-gen --no-default-features --features backend-native

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::error::MediaError;
use mediaservo_media::pipeline::generator::{
    Anchor, BitmapFont, PatternMode, SquaresConfig, TextBurner, TimestampFormat, TimestampOverlay,
    VideoFrameGenerator,
};
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};
use mediaservo_media::pipeline::source::VideoSource;

/// A sink that records frame statistics for smoke testing.
struct StatsSink {
    count: Arc<Mutex<u32>>,
    first_width: Arc<Mutex<Option<u32>>>,
    first_height: Arc<Mutex<Option<u32>>>,
    wants: VideoSinkWants,
}

impl StatsSink {
    #[allow(clippy::type_complexity)] // 样例三通道回执元组，不值得起别名
    fn new() -> (Self, Arc<Mutex<u32>>, Arc<Mutex<Option<u32>>>, Arc<Mutex<Option<u32>>>) {
        let count = Arc::new(Mutex::new(0u32));
        let first_width = Arc::new(Mutex::new(None));
        let first_height = Arc::new(Mutex::new(None));
        let sink = Self {
            count: count.clone(),
            first_width: first_width.clone(),
            first_height: first_height.clone(),
            wants: VideoSinkWants::default(),
        };
        (sink, count, first_width, first_height)
    }
}

impl VideoSink<BoxVideoFrame> for StatsSink {
    fn on_frame(&self, frame: &BoxVideoFrame) -> Result<VideoSinkWants, MediaError> {
        let mut cnt = self.count.lock().unwrap();
        if *cnt == 0 {
            let mut w = self.first_width.lock().unwrap();
            let mut h = self.first_height.lock().unwrap();
            *w = Some(frame.buffer.width());
            *h = Some(frame.buffer.height());
        }
        *cnt += 1;
        Ok(self.wants)
    }
}

fn main() {
    let width: u32 = 320;
    let height: u32 = 240;
    let num_squares: u32 = 8;
    let fps: u32 = 30;
    let duration = Duration::from_secs(2);

    let generator = VideoFrameGenerator::new();

    let (sink, count, first_width, first_height) = StatsSink::new();
    generator.add_or_update_sink(Box::new(sink), VideoSinkWants::default());

    let font = BitmapFont::new();
    let burner = TextBurner::new(font, false, Anchor::TopLeft);
    let overlay = TimestampOverlay::new(burner, TimestampFormat::Combined);
    let config = SquaresConfig { count: num_squares, motion_speed: 3, ..Default::default() };
    generator.start(fps, PatternMode::Squares(config), Some(overlay), width, height);

    thread::sleep(duration);

    generator.stop();

    let frame_count = *count.lock().unwrap();
    let fw = *first_width.lock().unwrap();
    let fh = *first_height.lock().unwrap();

    assert!(frame_count > 0, "No frames produced!");
    assert_eq!(fw, Some(width), "Frame width mismatch");
    assert_eq!(fh, Some(height), "Frame height mismatch");

    println!("SUCCESS: Generator produced {} colored frames", frame_count);
    println!("  Resolution: {}x{}", fw.unwrap_or(0), fh.unwrap_or(0));
    println!("  Duration: {:?}", duration);
}
