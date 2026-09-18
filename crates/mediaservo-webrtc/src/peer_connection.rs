//! RTCPeerConnection — W3C WebRTC API.
use crate::RTCError;
use crate::backend::{ActivePc, PcBackend};
use crate::data_channel::{RTCDataChannel, RTCDataChannelInit};
use crate::rtp::{RTCRtpReceiver, RTCRtpSender};
use crate::sdp::RTCSessionDescription;
use crate::track::{TrackKind, TrackReceiver, TrackRef, TrackSender};
use crate::traits::PeerConnectionApi as _;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "backend-webrtc-rs")]
use std::{future::Future, pin::Pin};

// ── Configuration types ──
#[derive(Debug, Clone)]
pub struct RTCIceServer {
    pub urls: Vec<String>,
    pub username: String,
    pub password: String,
}
#[derive(Debug, Clone)]
pub struct RTCConfiguration {
    pub ice_servers: Vec<RTCIceServer>,
    pub ice_transport_type: RTCIceTransportPolicy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCIceTransportPolicy {
    Relay,
    NoHost,
    All,
}
impl Default for RTCConfiguration {
    fn default() -> Self {
        Self { ice_servers: vec![], ice_transport_type: RTCIceTransportPolicy::All }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCPeerConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCIceConnectionState {
    New,
    Checking,
    Connected,
    Completed,
    Failed,
    Disconnected,
    Closed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCIceGatheringState {
    New,
    Gathering,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCSignalingState {
    Stable,
    HaveLocalOffer,
    HaveLocalPrAnswer,
    HaveRemoteOffer,
    HaveRemotePrAnswer,
    Closed,
}
#[derive(Debug, Clone, Default)]
pub struct RTCOfferOptions {
    pub ice_restart: bool,
    pub offer_to_receive_audio: bool,
    pub offer_to_receive_video: bool,
}
#[derive(Debug, Clone, Default)]
pub struct RTCAnswerOptions;
#[derive(Debug, Clone)]
pub struct RTCIceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}

/// W3C RTCIceCandidateError — onicecandidateerror 事件参数 (v2)
#[derive(Debug, Clone, Default)]
pub struct RTCIceCandidateError {
    pub error_code: i32,
    pub error_text: String,
}
pub const MAX_TRACKS: usize = 8;

// ── RTCPeerConnection struct ──

type OnTrackCallback = Arc<std::sync::Mutex<Option<Box<dyn Fn(RTCRtpReceiver) + Send + Sync>>>>;

pub struct RTCPeerConnection {
    pub backend: ActivePc,
    pub(crate) factory: Arc<crate::backend::ActiveFactory>,
    pub(crate) tracks: Arc<std::sync::Mutex<HashMap<String, TrackRef>>>,
    pub(crate) on_track_callback: OnTrackCallback,
}

// ── PeerConnectionApi trait implementation ──

impl crate::traits::PeerConnectionApi for RTCPeerConnection {
    async fn create_offer(
        &self,
        options: &RTCOfferOptions,
    ) -> Result<RTCSessionDescription, RTCError> {
        self.backend.create_offer(options).await
    }
    async fn create_answer(
        &self,
        options: &RTCAnswerOptions,
    ) -> Result<RTCSessionDescription, RTCError> {
        self.backend.create_answer(options).await
    }
    async fn set_local_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError> {
        self.backend.set_local_description(desc).await
    }
    async fn set_remote_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError> {
        self.backend.set_remote_description(desc).await
    }
    async fn add_ice_candidate(&self, candidate: &RTCIceCandidate) -> Result<(), RTCError> {
        self.backend.add_ice_candidate(candidate).await
    }
    async fn create_data_channel(
        &self,
        label: &str,
        init: RTCDataChannelInit,
    ) -> Result<RTCDataChannel, RTCError> {
        self.backend.create_data_channel(label, init).await
    }
    fn connection_state(&self) -> RTCPeerConnectionState {
        self.backend.connection_state()
    }
    fn ice_connection_state(&self) -> RTCIceConnectionState {
        self.backend.ice_connection_state()
    }
    fn ice_gathering_state(&self) -> RTCIceGatheringState {
        self.backend.ice_gathering_state()
    }
    fn signaling_state(&self) -> RTCSignalingState {
        self.backend.signaling_state()
    }
    async fn close(&self) {
        self.backend.close().await;
    }
    fn add_track(&self, track_id: &str, kind: TrackKind) -> Result<String, RTCError> {
        let mut tracks = self.tracks.lock().unwrap();
        if tracks.len() >= MAX_TRACKS {
            return Err(RTCError::Track("max tracks reached".into()));
        }

        let sender = self.create_track_sender(track_id, kind);

        let id = track_id.to_string();
        tracks.insert(id.clone(), TrackRef::Sender(sender));
        self.backend.register_track(track_id, kind)?;
        Ok(id)
    }
    fn remove_track(&self, track_id: &str) -> Result<(), RTCError> {
        let mut tracks = self.tracks.lock().unwrap();
        tracks
            .remove(track_id)
            .map(|_| ())
            .ok_or_else(|| RTCError::Track(format!("track not found: {}", track_id)))
    }
    fn get_track(&self, track_id: &str) -> Option<TrackRef> {
        self.tracks.lock().unwrap().get(track_id).cloned()
    }
    fn track_count(&self) -> usize {
        self.tracks.lock().unwrap().len()
    }
    fn track_ids(&self) -> Vec<String> {
        self.tracks.lock().unwrap().keys().cloned().collect()
    }
    fn get_senders(&self) -> Vec<RTCRtpSender> {
        let backend = self.backend.clone();
        self.tracks
            .lock()
            .unwrap()
            .values()
            .filter(|tr| matches!(tr, TrackRef::Sender(_)))
            .map(|tr| RTCRtpSender::new(tr.clone()).with_backend(backend.clone()))
            .collect()
    }
    fn get_receivers(&self) -> Vec<RTCRtpReceiver> {
        self.tracks
            .lock()
            .unwrap()
            .values()
            .filter(|tr| matches!(tr, TrackRef::Receiver(_)))
            .map(|tr| RTCRtpReceiver::new(tr.clone()))
            .collect()
    }
    fn on_track<F>(&self, callback: F)
    where
        F: Fn(RTCRtpReceiver) + Send + Sync + 'static,
    {
        *self.on_track_callback.lock().unwrap() = Some(Box::new(callback));
        let bk_cb_from = self.on_track_callback.clone();
        self.backend.set_on_track(Box::new(move |tr: TrackReceiver| {
            if let Some(ref f) = *bk_cb_from.lock().unwrap() {
                f(RTCRtpReceiver::new(TrackRef::Receiver(tr)));
            }
        }));
    }

    // ── v2: W3C API 实现 ──

    fn get_transceivers(&self) -> Result<Vec<crate::rtp::RTCRtpTransceiver>, RTCError> {
        self.backend.get_transceivers()
    }

    fn add_transceiver(
        &self,
        kind: TrackKind,
        init: crate::rtp::RTCRtpTransceiverInit,
    ) -> Result<crate::rtp::RTCRtpTransceiver, RTCError> {
        self.backend.add_transceiver(kind, &init)
    }

    fn add_transceiver_with_track(
        &self,
        track: &TrackSender,
        init: crate::rtp::RTCRtpTransceiverInit,
    ) -> Result<crate::rtp::RTCRtpTransceiver, RTCError> {
        self.backend.add_transceiver_with_track(track, &init)
    }

    fn get_sending_rtp_parameters(
        &self,
        track_id: &str,
    ) -> Result<crate::rtp::RTCRtpParameters, RTCError> {
        self.backend.sender_get_parameters(track_id)
    }

    fn request_key_frame(&self, track_id: &str) -> Result<(), RTCError> {
        self.backend.request_key_frame(track_id)
    }

    fn get_receiving_rtp_parameters(
        &self,
        track_id: &str,
    ) -> Result<crate::rtp::RTCRtpParameters, RTCError> {
        self.backend.receiver_get_parameters(track_id)
    }

    fn get_sender_capabilities(
        &self,
        kind: TrackKind,
    ) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError> {
        self.backend.get_sender_capabilities(kind)
    }

    fn get_receiver_capabilities(
        &self,
        kind: TrackKind,
    ) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError> {
        self.backend.get_receiver_capabilities(kind)
    }

    fn restart_ice(&self) -> Result<(), RTCError> {
        self.backend.restart_ice()
    }

    fn transceiver_set_codec_preferences(
        &self,
        track_id: &str,
        codecs: Vec<crate::rtp::RTCRtpCodecCapability>,
    ) -> Result<(), RTCError> {
        self.backend.transceiver_set_codec_preferences(track_id, codecs)
    }

    fn sender_get_stats(&self, track_id: &str) -> Vec<crate::stats::RTCStats> {
        self.backend.sender_get_stats(track_id)
    }

    fn receiver_get_stats(&self, track_id: &str) -> Vec<crate::stats::RTCStats> {
        self.backend.receiver_get_stats(track_id)
    }

    fn get_configuration(&self) -> RTCConfiguration {
        self.backend.pc_configuration()
    }

    fn set_configuration(&self, config: &RTCConfiguration) -> Result<(), RTCError> {
        self.backend.set_configuration(config)
    }

    fn current_local_description(&self) -> Result<Option<RTCSessionDescription>, RTCError> {
        self.backend.current_local_description()
    }

    fn current_remote_description(&self) -> Result<Option<RTCSessionDescription>, RTCError> {
        self.backend.current_remote_description()
    }

    fn on_negotiation_needed<F>(&self, _callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        // v2: backend 无 negotiation callback 接口，暂空实现（可后续接 on_renegotiation_needed）
    }

    fn on_ice_gathering_state_change<F>(&self, _callback: F)
    where
        F: Fn(RTCIceGatheringState) + Send + Sync + 'static,
    {
        // v2: webrtc-sys observer 有 on_ice_gathering_change，可后续暴露
    }

    fn on_ice_candidate_error<F>(&self, _callback: F)
    where
        F: Fn(RTCIceCandidateError) + Send + Sync + 'static,
    {
        // v2: webrtc-sys observer 有 on_ice_candidate_error，可后续暴露
    }
}

// ── Internal helpers + unified callback API ──

impl RTCPeerConnection {
    /// Create a TrackSender, binding to real backend via factory when available.
    /// Stages the underlying media track for `add_transceiver_with_track` (webrtc-sys).
    pub fn create_track_sender(&self, track_id: &str, kind: TrackKind) -> TrackSender {
        if kind == TrackKind::Video {
            #[cfg(feature = "backend-webrtc-sys")]
            {
                let (track_backend, media_track) = self.factory.create_video_track();
                self.backend.stage_media_track(media_track);
                return TrackSender {
                    id: track_id.to_string(),
                    kind,
                    audio_config: None,
                    backend: track_backend,
                };
            }
            #[cfg(not(feature = "backend-webrtc-sys"))]
            {
                let (backend, _media_track) = self.factory.create_video_track();
                return TrackSender { id: track_id.to_string(), kind, audio_config: None, backend };
            }
        }
        // H2: 音频 track（webrtc-sys）— AudioTrackSource + AudioTrack，stage 供
        // add_transceiver_with_track 使用；write_frame 推 PCM i16（libwebrtc 内部 opus 编码）。
        #[cfg(feature = "backend-webrtc-sys")]
        if kind == TrackKind::Audio {
            let (track_backend, media_track) = self.factory.create_audio_track();
            self.backend.stage_media_track(media_track);
            return TrackSender {
                id: track_id.to_string(),
                kind,
                audio_config: Some(crate::track::RTCAudioTrackConfig {
                    sample_rate: 48000,
                    channels: 1,
                }),
                backend: track_backend,
            };
        }
        TrackSender::new(track_id.to_string(), kind)
    }

    /// Extract the local DTLS fingerprint from the local description SDP.
    /// Returns colon:hex string from `a=fingerprint:sha-256 XX:XX:...` line.
    pub fn local_dtls_fingerprint(&self) -> Option<String> {
        let sdp = self.backend.local_description_sdp()?;
        let fp_line = sdp.lines().find(|l| l.starts_with("a=fingerprint:"))?;
        let colon = fp_line.find(':')?;
        let after_prefix = &fp_line[colon + 1..];
        let space = after_prefix.find(' ')?;
        Some(after_prefix[space + 1..].to_string())
    }

    /// Register callback for ICE connection state changes.
    /// Available on all backends — non cfg-gated.
    pub fn on_ice_connection_state_change<F>(&self, callback: F)
    where
        F: Fn(RTCIceConnectionState) + Send + Sync + 'static,
    {
        self.backend.set_on_ice_connection_state_change(Box::new(callback));
    }

    /// Register callback for peer connection state changes.
    /// Available on all backends — non cfg-gated.
    pub fn on_peer_connection_state_change<F>(&self, callback: F)
    where
        F: Fn(RTCPeerConnectionState) + Send + Sync + 'static,
    {
        self.backend.set_on_peer_connection_state_change(Box::new(callback));
    }

    /// Register callback for trickled local ICE candidates (P2P 必需).
    /// Available on all backends — non cfg-gated.
    pub fn on_ice_candidate<F>(&self, callback: F)
    where
        F: Fn(RTCIceCandidate) + Send + Sync + 'static,
    {
        self.backend.set_on_ice_candidate(Box::new(callback));
    }

    /// v2: 通用 incoming data channel 回调（三后端）。
    /// 替代 webrtc-rs cfg 版 on_data_channel（现在 webrtc-sys 也支持）。
    pub fn on_data_channel<F>(&self, callback: F)
    where
        F: Fn(RTCDataChannel) + Send + Sync + 'static,
    {
        self.backend.set_on_data_channel(Box::new(callback));
    }
}

// ── webrtc-rs specific callback methods ──

#[cfg(feature = "backend-webrtc-rs")]
impl RTCPeerConnection {
    /// webrtc-rs 原生候选回调 (P2P 路径) — 与通用 trickle 版 (on_ice_candidate) 区分命名。
    pub fn on_ice_candidate_native(
        &self,
        f: Box<
            dyn FnMut(
                    Option<webrtc::ice_transport::ice_candidate::RTCIceCandidate>,
                ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>
                + Send
                + Sync
                + 'static,
        >,
    ) {
        self.backend.on_ice_candidate(f);
    }
}

impl Clone for RTCPeerConnection {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            factory: Arc::clone(&self.factory),
            tracks: self.tracks.clone(),
            on_track_callback: Arc::clone(&self.on_track_callback),
        }
    }
}

impl std::fmt::Debug for RTCPeerConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RTCPeerConnection")
            .field("connection_state", &self.connection_state())
            .field("track_count", &self.track_count())
            .finish()
    }
}

// ── Tests ──

#[cfg(all(test, not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys"))))]
mod tests {
    use super::*;
    use crate::data_channel::RTCDataChannelInit;
    use crate::factory::RTCPeerConnectionFactory;
    use crate::sdp::RTCSdpType;

    #[test]
    fn stub_factory_default_creates() {
        let factory = RTCPeerConnectionFactory::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt.block_on(factory.create_peer_connection(RTCConfiguration::default()));
        assert!(pc.is_ok());
    }

    #[test]
    fn stub_create_offer_returns_sdp_type_offer() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let offer = rt.block_on(pc.create_offer(&RTCOfferOptions::default())).unwrap();
        assert_eq!(offer.sdp_type, RTCSdpType::Offer);
    }

    #[test]
    fn stub_create_answer_returns_sdp_type_answer() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let answer = rt.block_on(pc.create_answer(&RTCAnswerOptions::default())).unwrap();
        assert_eq!(answer.sdp_type, RTCSdpType::Answer);
    }

    #[test]
    fn stub_sdp_operations_are_noop() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let sd = RTCSessionDescription::new(RTCSdpType::Offer, String::new());
        assert!(rt.block_on(pc.set_local_description(&sd)).is_ok());
        assert!(rt.block_on(pc.set_remote_description(&sd)).is_ok());
    }

    #[test]
    fn stub_add_ice_candidate_is_noop() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let ic = RTCIceCandidate {
            candidate: "candidate:1".into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        };
        assert!(rt.block_on(pc.add_ice_candidate(&ic)).is_ok());
    }

    #[test]
    fn stub_create_data_channel_preserves_label() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let dc =
            rt.block_on(pc.create_data_channel("mychan", RTCDataChannelInit::default())).unwrap();
        assert_eq!(dc.label(), "mychan");
        assert_eq!(dc.id(), 0);
    }

    #[test]
    fn stub_connection_states_are_default() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        assert!(matches!(pc.connection_state(), RTCPeerConnectionState::New));
        assert!(matches!(pc.ice_connection_state(), RTCIceConnectionState::New));
        assert!(matches!(pc.ice_gathering_state(), RTCIceGatheringState::New));
        assert!(matches!(pc.signaling_state(), RTCSignalingState::Stable));
    }

    #[test]
    fn stub_close_is_noop() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        rt.block_on(pc.close());
    }

    #[test]
    fn pc_config_default_is_all_transport() {
        let cfg = RTCConfiguration::default();
        assert!(cfg.ice_servers.is_empty());
        assert_eq!(cfg.ice_transport_type, RTCIceTransportPolicy::All);
    }

    // ── D148: multi-track registry tests ──

    #[test]
    fn add_track_registers_in_map() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let id = pc.add_track("video-1", TrackKind::Video).unwrap();
        assert_eq!(id, "video-1");
        assert_eq!(pc.track_count(), 1);
        let ids = pc.track_ids();
        assert!(ids.contains(&"video-1".to_string()));
    }

    #[test]
    fn add_track_respects_max() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        for i in 0..8 {
            pc.add_track(&format!("track-{}", i), TrackKind::Video).unwrap();
        }
        assert_eq!(pc.track_count(), 8);
        let err = pc.add_track("overflow", TrackKind::Video).unwrap_err();
        assert!(matches!(err, RTCError::Track(_)));
    }

    #[test]
    fn remove_track_not_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let err = pc.remove_track("nonexistent").unwrap_err();
        assert!(matches!(err, RTCError::Track(_)));
    }

    #[test]
    fn remove_track_succeeds() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        pc.add_track("audio-1", TrackKind::Audio).unwrap();
        assert_eq!(pc.track_count(), 1);
        pc.remove_track("audio-1").unwrap();
        assert_eq!(pc.track_count(), 0);
    }

    #[test]
    fn get_track_returns_clone() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        pc.add_track("vid-1", TrackKind::Video).unwrap();
        let tr = pc.get_track("vid-1").unwrap();
        assert_eq!(tr.id(), "vid-1");
        assert_eq!(tr.kind(), TrackKind::Video);
    }

    #[test]
    fn get_track_missing_returns_none() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        assert!(pc.get_track("missing").is_none());
    }

    // ── D146: W3C API tests ──

    #[test]
    fn add_track_w3c_returns_sender() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        let track_id = pc.add_track("video-w3c", TrackKind::Video).unwrap();
        assert_eq!(track_id, "video-w3c");
        let tr = pc.get_track("video-w3c").unwrap();
        assert_eq!(tr.kind(), TrackKind::Video);
    }

    #[test]
    fn get_senders_filters_correctly() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        pc.add_track("vid-1", TrackKind::Video).unwrap();
        pc.add_track("vid-2", TrackKind::Video).unwrap();
        let senders = pc.get_senders();
        assert_eq!(senders.len(), 2);
    }

    #[test]
    fn get_receivers_empty_initially() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        assert!(pc.get_receivers().is_empty());
    }

    #[test]
    fn on_track_registers_callback() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(
                RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
            )
            .unwrap();
        pc.on_track(|_receiver| {});
    }
}
