//! W3C PeerConnectionApi trait — the public W3C WebRTC API contract.
//!
//! This trait defines the standard W3C PeerConnection interface.
//! Currently implemented by `crate::peer_connection::RTCPeerConnection` struct.
//!
//! # Backend dispatch
//!
//! The trait is backend-agnostic. Backend selection happens at compile time
//! via `ActivePc` type alias in `crate::backend`.
//!
//! # Usage
//!
//! ```ignore
//! use mediaservo_webrtc::traits::PeerConnectionApi;
//!
//! fn use_pc(pc: &impl PeerConnectionApi) {
//!     pc.create_offer(&Default::default()).await.unwrap();
//! }
//! ```

use crate::peer_connection::{
    RTCAnswerOptions, RTCIceCandidate, RTCOfferOptions, RTCConfiguration, RTCIceCandidateError,
    RTCIceConnectionState, RTCIceGatheringState, RTCPeerConnectionState, RTCSignalingState,
};
use crate::sdp::RTCSessionDescription;
use crate::data_channel::{RTCDataChannel, RTCDataChannelInit};
use crate::track::{TrackKind, TrackRef, TrackSender};
use crate::rtp::{RTCRtpSender, RTCRtpReceiver};
use crate::RTCError;

/// W3C WebRTC RTCPeerConnection interface (D146).
///
/// Provides the standard W3C methods: SDP negotiation, ICE management,
/// DataChannel creation, track management, and event callbacks.
///
/// Each backend (webrtc-sys, webrtc-rs, stub) provides an implementation
/// via compile-time `ActivePc` type alias dispatch.
pub trait PeerConnectionApi: Send + Sync + 'static {
    /// Create an SDP offer for initiating a new connection.
    async fn create_offer(&self, options: &RTCOfferOptions) -> Result<RTCSessionDescription, RTCError>;

    /// Create an SDP answer in response to an offer.
    async fn create_answer(&self, options: &RTCAnswerOptions) -> Result<RTCSessionDescription, RTCError>;

    /// Set the local session description (offer or answer).
    async fn set_local_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError>;

    /// Set the remote session description (offer or answer from peer).
    async fn set_remote_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError>;

    /// Add a remote ICE candidate received from the signaling channel.
    async fn add_ice_candidate(&self, candidate: &RTCIceCandidate) -> Result<(), RTCError>;

    /// Create a data channel for sending/receiving arbitrary data.
    async fn create_data_channel(&self, label: &str, init: RTCDataChannelInit) -> Result<RTCDataChannel, RTCError>;

    /// Current state of the peer connection.
    fn connection_state(&self) -> RTCPeerConnectionState;

    /// Current state of the ICE connection.
    fn ice_connection_state(&self) -> RTCIceConnectionState;

    /// Current state of ICE gathering.
    fn ice_gathering_state(&self) -> RTCIceGatheringState;

    /// Current signaling state.
    fn signaling_state(&self) -> RTCSignalingState;

    /// Close the peer connection.
    async fn close(&self);

    /// Register a local media track for RTP transmission.
    /// Returns the track ID on success (max 8 tracks per connection).
    fn add_track(&self, track_id: &str, kind: TrackKind) -> Result<String, RTCError>;

    /// Remove a previously registered track.
    fn remove_track(&self, track_id: &str) -> Result<(), RTCError>;

    /// Get a track reference by ID.
    fn get_track(&self, track_id: &str) -> Option<TrackRef>;

    /// Number of registered tracks.
    fn track_count(&self) -> usize;

    /// IDs of all registered tracks.
    fn track_ids(&self) -> Vec<String>;

    /// Get all sender (outgoing) tracks as RTCRtpSender objects.
    fn get_senders(&self) -> Vec<RTCRtpSender>;

    /// Get all receiver (incoming) tracks as RTCRtpReceiver objects.
    fn get_receivers(&self) -> Vec<RTCRtpReceiver>;

    /// Register a callback for incoming remote tracks.
    /// The callback receives an RTCRtpReceiver when a remote track is added.
    fn on_track<F>(&self, callback: F) where F: Fn(RTCRtpReceiver) + Send + Sync + 'static;

    // ── v2: W3C API 补全 ──

    /// W3C getTransceivers
    fn get_transceivers(&self) -> Result<Vec<crate::rtp::RTCRtpTransceiver>, RTCError>;
    /// W3C addTransceiver(kind, init) — 同步
    fn add_transceiver(&self, kind: TrackKind, init: crate::rtp::RTCRtpTransceiverInit) -> Result<crate::rtp::RTCRtpTransceiver, RTCError>;
    /// W3C addTransceiver(track, init) — track 版（P3 核心）
    fn add_transceiver_with_track(&self, track: &TrackSender, init: crate::rtp::RTCRtpTransceiverInit) -> Result<crate::rtp::RTCRtpTransceiver, RTCError>;
    /// W3C RTCRtpSender.getParameters 对应（经 track_id）
    /// W3C RTCRtpSender.getParameters 对应（经 track_id）
    fn get_sending_rtp_parameters(&self, track_id: &str) -> Result<crate::rtp::RTCRtpParameters, RTCError>;
    /// PIT-76: 请求关键帧（经 track_id，同步）— libwebrtc 每次调用触发一次
    fn request_key_frame(&self, track_id: &str) -> Result<(), RTCError>;
    /// W3C RTCRtpReceiver.getParameters 对应（经 track_id）
    fn get_receiving_rtp_parameters(&self, track_id: &str) -> Result<crate::rtp::RTCRtpParameters, RTCError>;
    /// W3C 静态 getCapabilities
    fn get_sender_capabilities(&self, kind: TrackKind) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError>;
    fn get_receiver_capabilities(&self, kind: TrackKind) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError>;
    /// W3C restartIce — 同步
    fn restart_ice(&self) -> Result<(), RTCError>;
    /// W3C RTCRtpSender.getStats — 出站统计（outbound-rtp, v2 web-stream-stats T2）。
    fn sender_get_stats(&self, track_id: &str) -> Vec<crate::stats::RTCStats>;

    /// S2c：收侧 inbound-rtp stats（packetsReceived/framesDecoded 二分判据）。
    /// 默认空面 = 未接线的后端零成本。
    fn receiver_get_stats(&self, _track_id: &str) -> Vec<crate::stats::RTCStats> {
        Vec::new()
    }

    /// W3C RTCRtpTransceiver.setCodecPreferences — 协商 codec 偏好（降序）。
    /// v2 实证修正: 按 track_id 定位 transceiver（mid 在协商前不存在 — offerer 场景核心）,
    /// 同 sender_get_parameters/request_key_frame 的 sender.track().id() 匹配模式。
    fn transceiver_set_codec_preferences(&self, track_id: &str, codecs: Vec<crate::rtp::RTCRtpCodecCapability>) -> Result<(), RTCError>;
    /// W3C getConfiguration / setConfiguration
    fn get_configuration(&self) -> RTCConfiguration;
    fn set_configuration(&self, config: &RTCConfiguration) -> Result<(), RTCError>;
    /// W3C currentLocalDescription / currentRemoteDescription
    fn current_local_description(&self) -> Result<Option<RTCSessionDescription>, RTCError>;
    fn current_remote_description(&self) -> Result<Option<RTCSessionDescription>, RTCError>;
    /// W3C onnegotiationneeded
    fn on_negotiation_needed<F>(&self, callback: F) where F: Fn() + Send + Sync + 'static;
    /// W3C onicegatheringstatechange
    fn on_ice_gathering_state_change<F>(&self, callback: F) where F: Fn(RTCIceGatheringState) + Send + Sync + 'static;
    /// W3C onicecandidateerror
    fn on_ice_candidate_error<F>(&self, callback: F) where F: Fn(crate::peer_connection::RTCIceCandidateError) + Send + Sync + 'static;
}
