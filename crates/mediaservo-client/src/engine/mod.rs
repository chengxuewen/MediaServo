//! The Engine abstraction layer -- the minimal WebRTC-primitive surface client actually consumes (S6 batch 0 / K11).
//!
//! Boundary discipline: **circle only the real call sites in current code; never invent methods client doesn't use**. Same type as mediasoup-client's
//! FakeHandler -- mask the "engine behavior" layer so recovery/error/DC logic runs deterministically in CI at second scale.
//!
//! The real impl = [`sys::SysEngine`] (a mediaservo-webrtc backend-webrtc-sys passthrough, zero logic changes);
//! the fake impl = [`fake::FakeEngine`] (feature-gated `engine-fake`, off by default, absent from release builds).
//!
//! Type choices: SDP crosses the boundary as a bare `String` (all three set_remote call sites are Offers, and answer only takes
//! `.sdp`); `RTCDataChannelInit`/`RTCStats`/`TrackKind`/`FrameSink` are
//! lightweight pure-data/trait types of the mediaservo-webrtc abstraction layer (no backend instance pulled in), reused directly --
//! sfu.rs's public signature (returns RTCDataChannelInit) therefore needs zero changes.
//!
//! # Method <-> current-call-site reconciliation (line numbers = session.rs/control.rs@HEAD before the refactor)
//!
//! | method | purpose | current call site |
//! |------|------|---------|
//! | `Engine::create_pc` | one ICE-Lite SFU peer connection (no STUN) | session.rs:435-444 `create_pc()` |
//! | `PcHandle::on_track` | register the ingress track callback (must precede set_remote -- late registration loses the first track) | session.rs:247-256 |
//! | `PcHandle::add_transceiver` | add a transceiver (current code only Video+Recvonly; the signature mirrors libwebrtc without trimming) | session.rs:301-308 |
//! | `PcHandle::set_remote_offer` | set the remote offer (fed the SDP synthesized from mediasoup transport params) | session.rs:310, 367, 642 |
//! | `PcHandle::create_answer` | produce the local answer SDP body | session.rs:313, 386, 645 |
//! | `PcHandle::set_local_answer` | set the local answer, closing the negotiation | session.rs:317, 390, 649 |
//! | `PcHandle::local_dtls_fingerprint` | local DTLS fingerprint -> ConnectWebRtcTransport dtls_parameters | session.rs:453-455 |
//! | `PcHandle::receiver_stats` | the given track's inbound-rtp stats (the truth source for diagnostics/mini-stats) | session.rs:101-106 |
//! | `PcHandle::create_data_channel` | create a DC (egress control = non-negotiated; ack consumption = negotiated out-of-band) | session.rs:377-380, 741-744 |
//! | `DcHandle::label` / `DcHandle::id` | SCTP stream-param derivation (stream_id = the libwebrtc real-config DC id) | session.rs:407 |
//! | `DcHandle::send_text` | egress the control envelope | control.rs:93-95 |
//! | `DcHandle::events` | the ingress event stream (Message/Closed) | session.rs:752, 758-775 |
//! | `DcHandle::close` | targeted local self-teardown when the vehicle DataProducer dies (frees the sid) | session.rs:779 |

#[cfg(feature = "engine-fake")]
// release builds exclude the fake engine (the K11 gating discipline)
pub mod fake;
pub mod sys;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use mediaservo_webrtc::data_channel::{RTCDataChannelInit, RTCDataChannelState};
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::stats::RTCStats;
use mediaservo_webrtc::track::{FrameSink, TrackKind};

use crate::error::ClientError;

pub use sys::SysEngine;

/// DC ingress events (mirrors `mediaservo_webrtc::RTCDataChannelEvent`'s four variants,
/// payload de-backend-ized -- current code only consumes Message/Closed; the rest pass through preserving event semantics).
#[derive(Debug, Clone)]
pub enum EngineDcEvent {
    Open,
    Closed,
    Message(Vec<u8>),
    Error(String),
}

/// The engine-side track handle (sys = the real receiver / fake = an injection slot), opaque to EngineTrack users.
pub(crate) trait TrackHandle: Send + Sync {
    /// Attach the frame outlet (a pump-thread callback, non-async; overflow-drop = the latest-wins semantics, decided by the sink side).
    fn attach(&self, sink: Box<dyn FrameSink>);
}

/// The `on_track` callback payload: one ingress remote track.
pub struct EngineTrack {
    pub track_id: String,
    pub kind: TrackKind,
    handle: Arc<dyn TrackHandle>,
}

impl EngineTrack {
    pub(crate) fn new(track_id: String, kind: TrackKind, handle: Arc<dyn TrackHandle>) -> Self {
        Self { track_id, kind, handle }
    }

    /// Attach an I420 frame sink (the only consumer in current code = session.rs FrameChanSink).
    pub fn set_frame_sink(&self, sink: Box<dyn FrameSink>) {
        self.handle.attach(sink);
    }
}

/// The WebRTC engine factory surface. The sole entry point through which client touches engines;
/// the `RoomSession::connect_with_engine` injection point.
#[async_trait]
pub trait Engine: Send + Sync + 'static {
    /// Build one SFU peer connection (ICE-Lite, empty ice_servers, policy=All).
    /// [session.rs:435-444]
    async fn create_pc(&self) -> Result<Arc<dyn PcHandle>, ClientError>;
}

/// The single peer connection's operation surface (the handle is the lifetime: the last Arc drop disconnects).
#[async_trait]
pub trait PcHandle: Send + Sync + 'static {
    /// Register the ingress track callback. [session.rs:247]
    fn on_track(&self, cb: Box<dyn Fn(EngineTrack) + Send + Sync + 'static>);
    /// Add a transceiver. [session.rs:301]
    fn add_transceiver(
        &self,
        kind: TrackKind,
        init: RTCRtpTransceiverInit,
    ) -> Result<(), ClientError>;
    /// Set the remote offer. [session.rs:310, 367, 642]
    async fn set_remote_offer(&self, sdp: String) -> Result<(), ClientError>;
    /// Produce the local answer (the SDP body). [session.rs:313, 386, 645]
    async fn create_answer(&self) -> Result<String, ClientError>;
    /// Set the local answer. [session.rs:317, 390, 649]
    async fn set_local_answer(&self, sdp: &str) -> Result<(), ClientError>;
    /// The local DTLS fingerprint (sha-256 colon-separated hex). [session.rs:453]
    fn local_dtls_fingerprint(&self) -> Option<String>;
    /// The given track's receiver stats. [session.rs:104]
    fn receiver_stats(&self, track_id: &str) -> Vec<RTCStats>;
    /// Connection-level stats (the candidate-pair rtt). [consumer.rs on_qoe tick]
    fn connection_stats(&self) -> Option<mediaservo_webrtc::stats::ConnectionStats> {
        None
    }
    /// Create a data channel. [session.rs:377, 741]
    async fn create_data_channel(
        &self,
        label: &str,
        init: RTCDataChannelInit,
    ) -> Result<Arc<dyn DcHandle>, ClientError>;
}

/// A single DataChannel's operation surface.
#[async_trait]
pub trait DcHandle: Send + Sync + 'static {
    /// The channel label (the sctp_stream_params derivation input). [session.rs:407]
    fn label(&self) -> &str;
    /// The libwebrtc real-config DC id = the SCTP stream_id. [session.rs:407]
    fn id(&self) -> i32;
    /// Text send (the control envelope). [control.rs:93]
    async fn send_text(&self, text: &str) -> Result<(), ClientError>;
    /// S6/K5: the channel ready state (open/closing/closed + connecting -- the backpressure/availability predicate).
    fn state(&self) -> RTCDataChannelState;
    /// S6/K5: the buffered outbound byte count (for pre-flight waterline checks before dispatching an e-stop).
    async fn buffered_amount(&self) -> u64;
    /// Subscribe the ingress event stream (each call gets its own subscriber end, same spool semantics). [session.rs:752]
    async fn events(&self) -> mpsc::UnboundedReceiver<EngineDcEvent>;
    /// Actively close (the local self-teardown). [session.rs:779]
    async fn close(&self);
}
