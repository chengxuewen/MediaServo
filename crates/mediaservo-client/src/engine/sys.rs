//! The real impl: a mediaservo-webrtc (backend-webrtc-sys) passthrough shell.
//!
//! **A refactor, not a rewrite** -- each method body = the current code moved in verbatim (incl. error-context strings), zero logic changes.
//! Error mapping keeps the `ClientError::WebRtc("<original call-site prefix>: <RTCError>")` verbatim shape;
//! the observation surface (logs/error strings) matches the pre-refactor state.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use mediaservo_webrtc::data_channel::{RTCDataChannel, RTCDataChannelEvent, RTCDataChannelInit};
use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
use mediaservo_webrtc::peer_connection::{
    RTCAnswerOptions, RTCConfiguration, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnection,
};
use mediaservo_webrtc::rtp::{RTCRtpReceiver, RTCRtpTransceiverInit};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::stats::RTCStats;
use mediaservo_webrtc::track::{FrameSink, TrackKind, TrackReceiver, TrackRef};
use mediaservo_webrtc::traits::PeerConnectionApi;

use super::{DcHandle, Engine, EngineDcEvent, EngineTrack, PcHandle, TrackHandle};
use crate::error::ClientError;

/// The passthrough engine: each `create_pc` builds the factory fresh (verbatim-identical to pre-refactor `create_pc()`).
#[derive(Debug, Default, Clone, Copy)]
pub struct SysEngine;

impl SysEngine {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Engine for SysEngine {
    async fn create_pc(&self) -> Result<Arc<dyn PcHandle>, ClientError> {
        let factory = RTCPeerConnectionFactory::new();
        let pc = factory
            .create_peer_connection(RTCConfiguration {
                ice_servers: Vec::<RTCIceServer>::new(),
                ice_transport_type: RTCIceTransportPolicy::All,
            })
            .await
            .map_err(|e| ClientError::WebRtc(format!("create_peer_connection: {e}")))?;
        Ok(Arc::new(SysPc { pc }))
    }
}

/// The real PC handle (RTCPeerConnection itself is Arc-semantics Clone; handle alive = connection alive).
struct SysPc {
    pc: RTCPeerConnection,
}

#[async_trait]
impl PcHandle for SysPc {
    fn on_track(&self, cb: Box<dyn Fn(EngineTrack) + Send + Sync + 'static>) {
        self.pc.on_track(move |receiver: RTCRtpReceiver| {
            // current-code semantics: only Receiver shapes attach a sink; Sender shapes (never seen in this domain) pass an empty handle,
            // the callback still firing to keep the observation log points unchanged.
            let handle: Arc<dyn TrackHandle> = match receiver.track {
                TrackRef::Receiver(r) => Arc::new(SysTrack { receiver: r }),
                TrackRef::Sender(_) => Arc::new(SysTrack {
                    receiver: TrackReceiver::new(receiver.track_id.clone(), receiver.kind),
                }),
            };
            cb(EngineTrack::new(receiver.track_id, receiver.kind, handle));
        });
    }

    fn add_transceiver(
        &self,
        kind: TrackKind,
        init: RTCRtpTransceiverInit,
    ) -> Result<(), ClientError> {
        self.pc
            .add_transceiver(kind, init)
            .map(|_| ())
            .map_err(|e| ClientError::WebRtc(format!("add_transceiver: {e}")))
    }

    async fn set_remote_offer(&self, sdp: String) -> Result<(), ClientError> {
        self.pc
            .set_remote_description(&RTCSessionDescription::new(RTCSdpType::Offer, sdp))
            .await
            .map_err(|e| ClientError::WebRtc(format!("set_remote_description: {e}")))
    }

    async fn create_answer(&self) -> Result<String, ClientError> {
        self.pc
            .create_answer(&RTCAnswerOptions)
            .await
            .map(|d| d.sdp)
            .map_err(|e| ClientError::WebRtc(format!("create_answer: {e}")))
    }

    async fn set_local_answer(&self, sdp: &str) -> Result<(), ClientError> {
        self.pc
            .set_local_description(&RTCSessionDescription::new(RTCSdpType::Answer, sdp.to_string()))
            .await
            .map_err(|e| ClientError::WebRtc(format!("set_local_description: {e}")))
    }

    fn local_dtls_fingerprint(&self) -> Option<String> {
        self.pc.local_dtls_fingerprint()
    }

    fn connection_stats(&self) -> Option<mediaservo_webrtc::stats::ConnectionStats> {
        // the RTCPeerConnection common wrapper already forwards (PcBackend is pub(crate), not directly referenceable)
        mediaservo_webrtc::traits::PeerConnectionApi::connection_stats(&self.pc)
    }

    fn receiver_stats(&self, track_id: &str) -> Vec<RTCStats> {
        self.pc.receiver_get_stats(track_id)
    }

    async fn create_data_channel(
        &self,
        label: &str,
        init: RTCDataChannelInit,
    ) -> Result<Arc<dyn DcHandle>, ClientError> {
        let dc = self
            .pc
            .create_data_channel(label, init)
            .await
            .map_err(|e| ClientError::WebRtc(format!("create_data_channel {label}: {e}")))?;
        Ok(Arc::new(SysDc { dc }))
    }
}

/// The real track: delegates sink registration to the libwebrtc receiver (set_frame_sink semantics unchanged).
struct SysTrack {
    receiver: TrackReceiver,
}

impl TrackHandle for SysTrack {
    fn attach(&self, sink: Box<dyn FrameSink>) {
        self.receiver.set_frame_sink(sink);
    }
}

/// The real DC: the spool -> events forwarder task preserves the "one subscriber per call" broadcast semantics.
struct SysDc {
    dc: RTCDataChannel,
}

#[async_trait]
impl DcHandle for SysDc {
    fn label(&self) -> &str {
        self.dc.label()
    }

    fn id(&self) -> i32 {
        self.dc.id()
    }

    async fn send_text(&self, text: &str) -> Result<(), ClientError> {
        let label = self.label().to_string();
        self.dc
            .send_text(text)
            .await
            .map_err(|e| ClientError::WebRtc(format!("DC {label} send: {e}")))
    }

    fn state(&self) -> mediaservo_webrtc::data_channel::RTCDataChannelState {
        self.dc.state()
    }

    async fn buffered_amount(&self) -> u64 {
        self.dc.buffered_amount().await
    }

    async fn events(&self) -> mpsc::UnboundedReceiver<EngineDcEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut spool = self.dc.spool().await;
        tokio::spawn(async move {
            while let Some(ev) = spool.recv().await {
                let mapped = match ev {
                    RTCDataChannelEvent::Open => EngineDcEvent::Open,
                    RTCDataChannelEvent::Closed => EngineDcEvent::Closed,
                    RTCDataChannelEvent::Message(m) => EngineDcEvent::Message(m.data),
                    RTCDataChannelEvent::Error(e) => EngineDcEvent::Error(e),
                };
                if tx.send(mapped).is_err() {
                    break; // the subscriber dropped = the pump exits (same as the old rx-drop semantics)
                }
            }
        });
        rx
    }

    async fn close(&self) {
        let mut dc = self.dc.clone();
        dc.close().await;
    }
}
