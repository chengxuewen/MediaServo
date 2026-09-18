// WebRTC transport module for MediaServo Host.
#[cfg(feature = "webrtc-p2p")]
// Creates a RTCPeerConnection, establishes an unordered unreliable RTCDataChannel
// named "frames", and exchanges SDP/ICE candidates via the existing signaling WS.
//
// # Flow
// 1. `WebrtcTransport::new(sender, room_id)` — builds PC, creates DC,
//    registers ICE handler, creates offer, sends SDP via WS.
//    Returns `(Self, mpsc::UnboundedReceiver<DcEvent>)` — the receiver
//    yields RTCDataChannel lifecycle events (Open/Closed/Message).
// 2. The ICE handler sends candidates automatically via the WS sender.
// 3. `send_frame(data)` — sends raw bytes through the RTCDataChannel.
// 4. The caller spawns a task to poll the event receiver.
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::SinkExt;
use mediaservo_common::error::CoreError;
use mediaservo_common::protocol::SignalingMessage;
use tokio::sync::{Mutex as TokioMutex, mpsc};
use tokio_tungstenite::tungstenite::Message;
use webrtc::api::APIBuilder;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::data_channel_state::RTCDataChannelState;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;

use crate::control::{self, ControlHandler};
use crate::signaling::WsSender;
/// RTCDataChannel lifecycle events forwarded from callbacks.
pub enum DcEvent {
    Open,
    Closed,
    Message(Vec<u8>),
}

/// WebRTC transport over RTCDataChannel.
///
/// Owns the RTCPeerConnection, RTCDataChannel, and WS sender (for ICE callbacks).
/// The WS receiver remains with the caller for processing incoming SDP answers.
///
/// After construction, extract the event receiver via the returned tuple
/// and spawn a task to process DC lifecycle events.
pub struct WebrtcTransport {
    #[allow(dead_code)]
    pc: Arc<webrtc::peer_connection::RTCPeerConnection>,
    dc: Arc<RTCDataChannel>,
    _ws_sender: Arc<TokioMutex<WsSender>>,
    // Keep the mpsc sender alive so the callbacks don't fail silently
    _dc_tx: mpsc::UnboundedSender<DcEvent>,
}

impl WebrtcTransport {
    /// Create a new WebRTC transport.
    ///
    /// Consumes the WS sender for SDP/ICE exchange.
    /// Returns the transport and an mpsc receiver for RTCDataChannel events.
    /// Spawn a task to poll the receiver for lifecycle management.
    pub async fn new(
        ws_sender: WsSender,
        room_id: String,
    ) -> Result<(Self, mpsc::UnboundedReceiver<DcEvent>), CoreError> {
        let ws = Arc::new(TokioMutex::new(ws_sender));

        // Create API and RTCPeerConnection
        let api = APIBuilder::new().build();
        let mut config = RTCConfiguration::default();
        config.ice_servers = vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_string()],
            ..Default::default()
        }];
        let pc = Arc::new(
            api.new_peer_connection(config)
                .await
                .map_err(|e| CoreError::PeerConnectionFailure(e.to_string()))?,
        );

        tracing::info!("RTCPeerConnection created");

        // Register ICE candidate callback
        {
            let ws_clone = ws.clone();
            let room = room_id.clone();
            pc.on_ice_candidate_native(Box::new(move |candidate: Option<RTCIceCandidate>| {
                let ws = ws_clone.clone();
                let room_id = room.clone();
                Box::pin(async move {
                    if let Some(c) = candidate {
                        if let Ok(init) = c.to_json() {
                            let msg = SignalingMessage::RTCIceCandidate {
                                room_id,
                                target: None,
                                candidate: init.candidate,
                                sdp_mid: init.sdp_mid,
                                sdp_mline_index: init.sdp_mline_index,
                            };
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let mut sender = ws.lock().await;
                                let _ = sender.send(Message::Text(json.into())).await;
                            }
                        }
                    }
                })
            }));
        }

        // Create unordered unreliable RTCDataChannel for low-latency frame delivery
        let dc = pc
            .create_data_channel(
                "frames",
                Some(RTCDataChannelInit {
                    ordered: Some(false),
                    max_retransmits: Some(0),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| CoreError::PeerConnectionFailure(e.to_string()))?;

        tracing::info!(
            "RTCDataChannel '{}' (id={}) created — unordered, unreliable",
            dc.label(),
            dc.id()
        );

        // Set up DC event forwarding via mpsc
        let (dc_tx, dc_rx) = mpsc::unbounded_channel();

        dc.on_open(Box::new({
            let tx = dc_tx.clone();
            move || {
                let _ = tx.send(DcEvent::Open);
                Box::pin(async {}) as Pin<Box<dyn Future<Output = ()> + Send + 'static>>
            }
        }));

        dc.on_message(Box::new({
            let tx = dc_tx.clone();
            move |msg: DataChannelMessage| {
                let _ = tx.send(DcEvent::Message(msg.data.to_vec()));
                Box::pin(async {}) as Pin<Box<dyn Future<Output = ()> + Send + 'static>>
            }
        }));

        dc.on_close(Box::new({
            let tx = dc_tx.clone();
            move || {
                let _ = tx.send(DcEvent::Closed);
                Box::pin(async {}) as Pin<Box<dyn Future<Output = ()> + Send + 'static>>
            }
        }));

        // Create offer and send SDP via signaling
        let offer = pc
            .create_offer(None)
            .await
            .map_err(|e| CoreError::PeerConnectionFailure(e.to_string()))?;

        pc.set_local_description(offer.clone())
            .await
            .map_err(|e| CoreError::PeerConnectionFailure(e.to_string()))?;

        let sdp_json = serde_json::to_string(&offer)
            .map_err(|e| CoreError::ConfigParse(format!("serialize SDP: {e}")))?;

        let sdp_msg = SignalingMessage::Sdp { room_id, target: None, sdp: sdp_json };

        let sdp_text = serde_json::to_string(&sdp_msg)
            .map_err(|e| CoreError::ConfigParse(format!("serialize Sdp message: {e}")))?;

        {
            let mut sender = ws.lock().await;
            sender
                .send(Message::Text(sdp_text.into()))
                .await
                .map_err(|e| CoreError::WebSocketDisconnect(format!("send SDP offer: {e}")))?;
        }

        tracing::info!("SDP offer sent via signaling");

        Ok((Self { pc, dc, _ws_sender: ws, _dc_tx: dc_tx }, dc_rx))
    }

    /// Send a frame (raw bytes) through the RTCDataChannel.
    ///
    /// No-op if RTCDataChannel is not yet open (ponytail: non-fatal for startup race).
    pub async fn send_frame(&self, data: &[u8]) -> Result<(), CoreError> {
        if !self.is_open() {
            return Ok(()); // ponytail: DC not open yet, skip silently
        }
        let chunk = Bytes::copy_from_slice(data);
        self.dc
            .send(&chunk)
            .await
            .map(|_s| ())
            .map_err(|e| CoreError::PeerConnectionFailure(format!("DC send: {e}")))
    }

    /// Handle incoming SDP answer from remote peer.
    ///
    /// Parses the JSON SDP, calls pc.set_remote_description().
    pub async fn handle_answer(&self, sdp_json: &str) -> Result<(), CoreError> {
        let answer: webrtc::peer_connection::sdp::session_description::RTCSessionDescription =
            serde_json::from_str(sdp_json)
                .map_err(|e| CoreError::ConfigParse(format!("parse answer SDP: {e}")))?;
        self.pc
            .set_remote_description(answer)
            .await
            .map_err(|e| CoreError::PeerConnectionFailure(format!("set remote: {e}")))?;
        tracing::info!("Remote description set — WebRTC negotiation complete");
        Ok(())
    }

    /// Handle incoming ICE candidate from remote peer.
    ///
    /// Parses JSON RTCIceCandidateInit to RTCIceCandidate, calls pc.add_ice_candidate().
    pub async fn handle_remote_ice(&self, candidate_json: &str) -> Result<(), CoreError> {
        let init: webrtc::ice_transport::ice_candidate::RTCIceCandidateInit =
            serde_json::from_str(candidate_json)
                .map_err(|e| CoreError::ConfigParse(format!("parse ice: {e}")))?;
        self.pc
            .add_ice_candidate(init)
            .await
            .map_err(|e| CoreError::PeerConnectionFailure(format!("add ice: {e}")))?;
        Ok(())
    }

    /// Get a reference to the underlying RTCDataChannel.
    #[allow(dead_code)]
    pub fn data_channel(&self) -> &Arc<RTCDataChannel> {
        &self.dc
    }

    /// Check if the RTCDataChannel is open.
    #[allow(dead_code)]
    pub fn is_open(&self) -> bool {
        self.dc.ready_state() == RTCDataChannelState::Open
    }
}

impl Drop for WebrtcTransport {
    fn drop(&mut self) {
        // ponytail: best-effort close RTCPeerConnection and RTCDataChannel on drop.
        // webrtc-rs RTCPeerConnection does NOT auto-close — ICE sockets and
        // UDP ports persist without explicit close().
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let pc = self.pc.clone();
            let dc = self.dc.clone();
            handle.spawn(async move {
                dc.close().await;
                pc.close().await;
                tracing::debug!("WebrtcTransport: PC and DC closed");
            });
        }
    }
}

/// Run the RTCDataChannel event loop — logs lifecycle events.
///
/// Call this in a spawned task with the receiver from `WebrtcTransport::new()`.
pub async fn run_dc_event_loop(
    mut rx: mpsc::UnboundedReceiver<DcEvent>,
    control_handler: Arc<Mutex<ControlHandler>>,
) {
    loop {
        match rx.recv().await {
            Some(DcEvent::Open) => {
                tracing::info!("RTCDataChannel opened");
            }
            Some(DcEvent::Message(data)) => {
                if data.len() >= 9 && data[0] == 0x01 {
                    let tag: [u8; 8] = data[1..9].try_into().unwrap();
                    let body_str = String::from_utf8_lossy(&data[9..]).to_string();
                    if let Some(frame) = control::parse_and_validate(&body_str, &tag) {
                        control_handler.lock().unwrap().enqueue(frame);
                    }
                }
                tracing::debug!("RTCDataChannel received {} bytes", data.len());
            }
            Some(DcEvent::Closed) | None => {
                tracing::info!("RTCDataChannel closed");
                break;
            }
        }
    }
}
