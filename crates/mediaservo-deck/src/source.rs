//! Capture domain — CameraSource / MediaDevices / frame streams.
//!
//! MVP form: stub frame sources (color bars / squares pattern, frames produced via mediaservo-media's
//! `VideoFrameGenerator`), providing a unified capture entry for the field composition SDK's push
//! pipeline and local recording loop. Real device capture (GStreamer v4l2src) lands in a
//! later deck version (see `docs/modules/04-sdk-layers.md` §10).

use std::sync::Arc;

use mediaservo_codec::codec::{PixelFormat, VideoFormat};
use mediaservo_codec::frame::{Plane, VideoFrame};
use mediaservo_media::base::buffer::VideoBuffer;
use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::pipeline::generator::{
    ColorStrategy, PatternMode, SquaresConfig, VideoFrameGenerator,
};
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};
use mediaservo_media::pipeline::source::VideoSource;
use tokio::sync::mpsc;

use crate::DeckError;

/// Device kinds (contract §6: Camera/Audio/Screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDeviceKind {
    Camera,
    Audio,
    Screen,
}

/// Device identity (opaque handle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceId(pub String);

/// Capture options (contract §6).
#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub resolution: Option<(u32, u32)>,
    pub framerate: Option<u32>,
    pub format: Option<PixelFormat>,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            resolution: Some((1280, 720)),
            framerate: Some(30),
            format: Some(PixelFormat::Yuv420p),
        }
    }
}

/// Device enumeration (stub: fixed device table).
pub struct MediaDevices;

impl MediaDevices {
    pub fn enumerate(kind: MediaDeviceKind) -> Vec<DeviceId> {
        match kind {
            MediaDeviceKind::Camera => vec![DeviceId("stub:test-camera".into())],
            MediaDeviceKind::Audio | MediaDeviceKind::Screen => vec![],
        }
    }
}

/// Frame stream (bounded channel; latest-frame drop to prevent backlog).
#[derive(Debug)]
pub struct FrameStream {
    rx: mpsc::Receiver<VideoFrame>,
}

impl FrameStream {
    /// Asynchronously receive the next frame.
    pub async fn recv(&mut self) -> Option<VideoFrame> {
        self.rx.recv().await
    }
}

/// Frame watch sink: generator thread → channel (drop when full = latest-frame semantics).
struct ChannelSink {
    tx: mpsc::Sender<VideoFrame>,
}

impl VideoSink<BoxVideoFrame> for ChannelSink {
    fn on_frame(
        &self,
        frame: &BoxVideoFrame,
    ) -> Result<VideoSinkWants, mediaservo_media::error::MediaError> {
        let buf = frame.buffer.as_i420().ok_or_else(|| {
            mediaservo_media::error::MediaError::Internal("frame buffer not I420".into())
        })?;
        let frame = VideoFrame {
            format: VideoFormat {
                width: buf.width(),
                height: buf.height(),
                pixel_format: PixelFormat::Yuv420p,
            },
            planes: vec![
                Plane { data: buf.data_y.clone(), stride: buf.stride_y },
                Plane { data: buf.data_u.clone(), stride: buf.stride_u },
                Plane { data: buf.data_v.clone(), stride: buf.stride_v },
            ],
            pts: frame.timestamp_us.max(0) as u64,
            keyframe: false,
        };
        let _ = self.tx.try_send(frame); // Drop when full (latest-frame, prevents backlog)
        Ok(VideoSinkWants::default())
    }
}

/// Camera capture source (stub frame source).
pub struct CameraSource {
    _dev: DeviceId,
    generator: Arc<VideoFrameGenerator>,
    _tx: Option<mpsc::Sender<VideoFrame>>,
    started: bool,
}

impl CameraSource {
    pub fn open(dev: DeviceId, _opts: CaptureOptions) -> Result<Self, DeckError> {
        let known = MediaDevices::enumerate(MediaDeviceKind::Camera);
        if !known.contains(&dev) {
            return Err(DeckError::NotFound(format!("camera device {dev:?}")));
        }
        Ok(Self {
            _dev: dev,
            generator: Arc::new(VideoFrameGenerator::new()),
            _tx: None,
            started: false,
        })
    }

    /// Start producing frames; returns the frame stream (each generated frame is pushed into it). start is allowed only once.
    pub fn start(&mut self, opts: &CaptureOptions) -> Result<FrameStream, DeckError> {
        if self.started {
            return Err(DeckError::InvalidState("already started".into()));
        }
        let (width, height) =
            opts.resolution.ok_or_else(|| DeckError::InvalidState("resolution required".into()))?;
        let fps = opts.framerate.unwrap_or(30);
        let (tx, rx) = mpsc::channel(8);
        self._tx = Some(tx.clone());
        self.generator.add_or_update_sink(Box::new(ChannelSink { tx }), VideoSinkWants::default());
        self.generator.start(
            fps,
            PatternMode::Squares(SquaresConfig {
                count: 16,
                min_size: 32,
                max_size: 96,
                motion_speed: 0,
                color_strategy: ColorStrategy::RandomPerSquare,
            }),
            None,
            width,
            height,
        );
        self.started = true;
        Ok(FrameStream { rx })
    }

    /// Stop producing frames.
    pub fn stop(&self) {
        self.generator.stop();
    }
}

impl Drop for CameraSource {
    fn drop(&mut self) {
        self.generator.stop();
    }
}
