// ── RTCPeerConnectionFactory ──

use std::collections::HashMap;
use std::sync::Arc;

use crate::RTCError;
use crate::backend::ActiveFactory;
use crate::peer_connection::{RTCConfiguration, RTCPeerConnection};
use crate::track::{TrackKind, TrackSender};

pub struct RTCPeerConnectionFactory {
    pub backend: Arc<ActiveFactory>,
}

impl RTCPeerConnectionFactory {
    pub fn new() -> Self {
        Self { backend: Arc::new(ActiveFactory::default()) }
    }

    pub async fn create_peer_connection(
        &self,
        config: RTCConfiguration,
    ) -> Result<RTCPeerConnection, RTCError> {
        let pc_backend = self.backend.create_peer_connection(config).await?;
        Ok(RTCPeerConnection {
            backend: pc_backend,
            factory: Arc::clone(&self.backend),
            tracks: Arc::new(std::sync::Mutex::new(HashMap::new())),
            on_track_callback: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// Create a video track with a real video track backend.
    pub fn create_video_track(&self, track_id: &str) -> TrackSender {
        let (backend, _media_track) = self.backend.create_video_track();
        TrackSender {
            id: track_id.to_string(),
            kind: TrackKind::Video,
            audio_config: None,
            backend,
        }
    }

    /// Create a video track returning raw backend + media track (for webrtc-sys binding).
    #[cfg(feature = "backend-webrtc-sys")]
    pub fn create_video_track_raw(
        &self,
    ) -> (
        crate::backend::webrtc_sys::WebrtcSysTrack,
        cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>,
    ) {
        self.backend.create_video_track()
    }
    /// Create an audio track with a real audio track backend (H2).
    /// 48kHz mono AudioTrackSource — write_frame 推 PCM i16，libwebrtc 内部 opus 编码。
    #[cfg(feature = "backend-webrtc-sys")]
    pub fn create_audio_track(&self, track_id: &str) -> TrackSender {
        let (backend, _media_track) = self.backend.create_audio_track();
        TrackSender {
            id: track_id.to_string(),
            kind: TrackKind::Audio,
            audio_config: Some(crate::track::RTCAudioTrackConfig {
                sample_rate: 48000,
                channels: 1,
            }),
            backend,
        }
    }

    /// Create an audio track returning raw backend + media track (for webrtc-sys binding).
    #[cfg(feature = "backend-webrtc-sys")]
    pub fn create_audio_track_raw(
        &self,
    ) -> (
        crate::backend::webrtc_sys::WebrtcSysTrack,
        cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>,
    ) {
        self.backend.create_audio_track()
    }
}

impl Default for RTCPeerConnectionFactory {
    fn default() -> Self {
        Self::new()
    }
}
