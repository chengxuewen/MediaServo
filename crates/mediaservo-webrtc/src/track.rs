//! Media track types — sending (TrackSender) and receiving (TrackReceiver).
//!
//! # Multi-track architecture
//! - TrackKind distinguishes Video/Audio tracks (D147)
//! - TrackSender writes encoded frames to the RTP pipeline (delegates to backend)
//! - TrackReceiver reads frames from remote RTCPeerConnection
//! - TrackRef is the unified handle enum for registry management (D148)

use crate::RTCError;
use crate::backend::{ActiveTrack, TrackWriteBackend};

/// Audio track configuration (D147).
///
/// Opus defaults: 48kHz sample rate, 2 channels (stereo).
/// Frame duration: 20ms (960 samples per channel at 48kHz).
#[derive(Debug, Clone, Copy)]
pub struct RTCAudioTrackConfig {
    pub sample_rate: u32,
    pub channels: u32,
}

impl Default for RTCAudioTrackConfig {
    fn default() -> Self {
        Self { sample_rate: 48000, channels: 2 }
    }
}

impl RTCAudioTrackConfig {
    /// Samples per frame at 20ms (standard Opus frame duration).
    pub fn samples_per_frame(&self) -> u32 {
        self.sample_rate / 50 // 48000 / 50 = 960
    }

    /// Frame duration in milliseconds.
    pub fn frame_duration_ms(&self) -> u64 {
        20 // ponytail: Opus standard 20ms frame
    }
}

/// Track media type (D147).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

impl TrackKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackKind::Video => "video",
            TrackKind::Audio => "audio",
        }
    }
}

/// Receiver-side video/audio track.
/// Receives frames from a remote RTCPeerConnection via on_track callback.
pub struct TrackReceiver {
    pub id: String,
    pub kind: TrackKind,
    pub(crate) sink: std::sync::Arc<std::sync::Mutex<Option<Box<dyn FrameSink>>>>,
}

impl Clone for TrackReceiver {
    fn clone(&self) -> Self {
        Self { id: self.id.clone(), kind: self.kind, sink: self.sink.clone() }
    }
}

impl std::fmt::Debug for TrackReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackReceiver").field("id", &self.id).field("kind", &self.kind).finish()
    }
}

impl TrackReceiver {
    pub fn new(id: String, kind: TrackKind) -> Self {
        Self { id, kind, sink: std::sync::Arc::new(std::sync::Mutex::new(None)) }
    }

    /// Register a frame sink to receive decoded I420 frames.
    pub fn set_frame_sink(&self, sink: Box<dyn FrameSink>) {
        *self.sink.lock().unwrap() = Some(sink);
    }
}

/// Callback trait for receiving decoded video frames.
/// The backend delivers raw I420 (YUV 4:2:0 planar) frames via on_frame.
pub trait FrameSink: Send + Sync + 'static {
    /// Called when a decoded I420 frame arrives.
    /// data layout: Y plane (w*h) + U plane (w*h/4) + V plane (w*h/4).
    fn on_frame(&self, data: &[u8], width: u32, height: u32);
}

/// Sender-side video/audio track.
/// Pushes encoded frames to the WebRTC RTP pipeline through the backend.
///
/// # Backend
/// - `backend-webrtc-rs`: wraps webrtc `TrackLocalStaticSample` for RTP packetization
/// - stub: logs and returns Ok (no-op for testing without WebRTC)
#[derive(Debug, Clone)]
pub struct TrackSender {
    pub id: String,
    pub kind: TrackKind,
    /// Audio configuration (D147). Only meaningful when kind == Audio.
    /// None for video tracks.
    pub audio_config: Option<RTCAudioTrackConfig>,
    /// Direct access to the backend for write_raw_i420 etc.
    pub backend: ActiveTrack,
}
impl TrackSender {
    pub fn new(id: String, kind: TrackKind) -> Self {
        Self { id, kind, audio_config: None, backend: ActiveTrack::default() }
    }

    /// Create an audio track with Opus configuration (D147).
    /// Defaults: 48kHz, 2 channels, 20ms frames.
    pub fn new_audio(id: String, config: RTCAudioTrackConfig) -> Self {
        Self {
            id,
            kind: TrackKind::Audio,
            audio_config: Some(config),
            backend: ActiveTrack::default(),
        }
    }

    /// Write an encoded frame to the track.
    /// For audio tracks: uses frame_duration_ms from RTCAudioTrackConfig.
    /// For video tracks: defaults to 33ms (30fps).
    /// Delegates to the active backend.
    pub async fn write_frame(&self, data: &[u8]) -> Result<(), RTCError> {
        self.backend.write_frame(data, self.kind, self.audio_config.as_ref()).await
    }

    /// Write a raw I420 (YUV 4:2:0 planar) frame to the video track.
    /// The backend handles encoding. Delegates to the active backend.
    ///
    /// `data` layout: Y plane (w*h) + U plane (w*h/4) + V plane (w*h/4).
    pub async fn write_raw_i420(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), RTCError> {
        self.backend.write_raw_i420(data, width, height).await
    }

    /// PIT-63: 时间戳参数化 — ts_us 为帧捕获时刻 (µs); None → 后端默认 (锚定单调 wall-clock)。
    pub async fn write_raw_i420_with_ts(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        ts_us: Option<i64>,
    ) -> Result<(), RTCError> {
        self.backend.write_raw_i420_with_ts(data, width, height, ts_us).await
    }
}

/// Unified handle for multi-track registry (D148).
/// HashMap<String, TrackRef> in RTCPeerConnection manages tracks.
#[derive(Debug, Clone)]
pub enum TrackRef {
    Sender(TrackSender),
    Receiver(TrackReceiver),
}

impl TrackRef {
    pub fn id(&self) -> &str {
        match self {
            TrackRef::Sender(t) => &t.id,
            TrackRef::Receiver(t) => &t.id,
        }
    }

    pub fn kind(&self) -> TrackKind {
        match self {
            TrackRef::Sender(t) => t.kind,
            TrackRef::Receiver(t) => t.kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_kind_as_str() {
        assert_eq!(TrackKind::Video.as_str(), "video");
        assert_eq!(TrackKind::Audio.as_str(), "audio");
    }

    #[test]
    fn receiver_new_sets_fields() {
        let tr = TrackReceiver::new("track-1".into(), TrackKind::Video);
        assert_eq!(tr.id, "track-1");
        assert_eq!(tr.kind, TrackKind::Video);
    }

    #[test]
    fn receiver_clone_preserves_fields() {
        let tr = TrackReceiver::new("t2".into(), TrackKind::Audio);
        let tr2 = tr.clone();
        assert_eq!(tr2.id, "t2");
        assert_eq!(tr2.kind, TrackKind::Audio);
    }

    #[test]
    fn sender_new_sets_fields() {
        let ts = TrackSender::new("ts-1".into(), TrackKind::Video);
        assert_eq!(ts.id, "ts-1");
        assert_eq!(ts.kind, TrackKind::Video);
    }

    #[test]
    fn track_ref_id_and_kind() {
        let ts = TrackSender::new("ref-ts".into(), TrackKind::Audio);
        let tr = TrackReceiver::new("ref-tr".into(), TrackKind::Video);
        let r1 = TrackRef::Sender(ts);
        let r2 = TrackRef::Receiver(tr);
        assert_eq!(r1.id(), "ref-ts");
        assert_eq!(r1.kind(), TrackKind::Audio);
        assert_eq!(r2.id(), "ref-tr");
        assert_eq!(r2.kind(), TrackKind::Video);
    }
    #[test]
    fn audio_track_config_defaults() {
        let cfg = RTCAudioTrackConfig::default();
        assert_eq!(cfg.sample_rate, 48000);
        assert_eq!(cfg.channels, 2);
        assert_eq!(cfg.samples_per_frame(), 960);
        assert_eq!(cfg.frame_duration_ms(), 20);
    }

    #[test]
    fn new_audio_sets_kind_and_config() {
        let ts = TrackSender::new_audio("mic-1".into(), RTCAudioTrackConfig::default());
        assert_eq!(ts.kind, TrackKind::Audio);
        assert!(ts.audio_config.is_some());
        assert_eq!(ts.audio_config.unwrap().sample_rate, 48000);
    }

    #[test]
    fn video_track_has_no_audio_config() {
        let ts = TrackSender::new("vid-1".into(), TrackKind::Video);
        assert_eq!(ts.kind, TrackKind::Video);
        assert!(ts.audio_config.is_none());
    }
}
