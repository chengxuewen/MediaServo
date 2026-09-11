//! WebSocket signaling client — connects to mediaservo-server /ws endpoint.
//!
//! Handles: PSK auth → RoomJoin → RoomJoined → SDP/ICE relay loop.

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::error::CoreError;
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
/// Manages the signaling WebSocket lifecycle: auth, room join, message relay.
pub struct SignalingClient {
    server_url: String,
    psk: String,
    room_id: String,
    frame_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>, 
}
impl SignalingClient {
    /// Create a new signaling client.
    ///
    /// `server_url` is the base URL (e.g., `ws://server.local:9800`).
    /// The `/ws` path is appended automatically.
    pub fn new(server_url: &str, psk: &str, room_id: &str) -> Self {
        Self {
            server_url: server_url.to_string(),
            psk: psk.to_string(),
            room_id: room_id.to_string(),
            frame_tx: None,
        }
    }

    pub fn new_with_frame_tx(
        server_url: &str,
        psk: &str,
        room_id: &str,
        frame_tx: tokio::sync::mpsc::Sender<Vec<u8>>, 
    ) -> Self {
        Self {
            server_url: server_url.to_string(),
            psk: psk.to_string(),
            room_id: room_id.to_string(),
            frame_tx: Some(frame_tx),
        }
    }
    /// Connect to the signaling server, authenticate, join a room,
    /// and enter the SDP/ICE relay loop. Blocks until disconnect.
    pub async fn connect(&self) -> Result<(), CoreError> {
        let url = self.server_url.clone();

        let (mut ws, _) = connect_async(&url)
            .await
            .map_err(|e| CoreError::WebSocketDisconnect(format!("connect failed: {e}")))?;

        // Step 1: Send PSK for authentication
        ws.send(Message::Text(self.psk.clone().into()))
            .await
            .map_err(|e| CoreError::WebSocketDisconnect(format!("send PSK: {e}")))?;

        // Step 2: Wait for auth acknowledgment
        match ws.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: SignalingMessage = serde_json::from_str(&text)
                    .map_err(|e| CoreError::WebSocketDisconnect(format!("parse auth response: {e}")))?;
                match msg {
                    SignalingMessage::Error { code: 0, .. } => {
                        tracing::info!("Signaling: authenticated");
                    }
                    SignalingMessage::Error { code, message } => {
                        tracing::error!("Auth failed [{code}]: {message}");
                        return Err(CoreError::PskAuthFailed);
                    }
                    _ => {
                        return Err(CoreError::WebSocketDisconnect(
                            "unexpected auth response".into(),
                        ));
                    }
                }
            }
            Some(Ok(_)) => {
                return Err(CoreError::WebSocketDisconnect("non-text auth response".into()));
            }
            Some(Err(e)) => {
                return Err(CoreError::WebSocketDisconnect(format!("auth read error: {e}")));
            }
            None => {
                return Err(CoreError::WebSocketDisconnect(
                    "connection closed during auth".into(),
                ));
            }
        }

        // Step 3: Send RoomJoin
        let join = SignalingMessage::RoomJoin {
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            room_id: self.room_id.clone(),
            peer_role: PeerRole::Remote,
            stream_id: None,
        };
        let join_json = serde_json::to_string(&join)
            .map_err(|e| CoreError::ConfigParse(format!("serialize RoomJoin: {e}")))?;
        ws.send(Message::Text(join_json.into()))
            .await
            .map_err(|e| CoreError::WebSocketDisconnect(format!("send RoomJoin: {e}")))?;

        // Step 4: Wait for RoomJoined
        let _peer_id = match ws.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: SignalingMessage = serde_json::from_str(&text)
                    .map_err(|e| CoreError::WebSocketDisconnect(format!("parse room response: {e}")))?;
                match msg {
                    SignalingMessage::RoomJoined { room_id, peer_id } => {
                        tracing::info!("Signaling: joined room {room_id} as {peer_id}");
                        peer_id
                    }
                    SignalingMessage::Error { code, message } => {
                        tracing::error!("Room join error [{code}]: {message}");
                        return Err(CoreError::WebSocketDisconnect(format!(
                            "room join rejected [{code}]: {message}"
                        )));
                    }
                    _ => {
                        return Err(CoreError::WebSocketDisconnect(
                            "unexpected RoomJoined response".into(),
                        ));
                    }
                }
            }
            Some(Ok(_)) => {
                return Err(CoreError::WebSocketDisconnect("non-text room response".into()));
            }
            Some(Err(e)) => {
                return Err(CoreError::WebSocketDisconnect(format!("room read error: {e}")));
            }
            None => {
                return Err(CoreError::WebSocketDisconnect(
                    "connection closed during room join".into(),
                ));
            }
        };

        // Step 5: SDP/ICE relay loop with WebRTC transport

        // Set up WebRTC transport for data channel frame reception
        let (ice_tx, mut ice_rx) = crate::webrtc_transport::ice_channel();
        let frame_tx = self
            .frame_tx
            .clone()
            .expect("frame_tx required for WebRTC transport");
        let webrtc = crate::webrtc_transport::WebrtcTransport::new(frame_tx, ice_tx);

        loop {
            tokio::select! {
                // Incoming signaling messages from server
                msg = ws.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<SignalingMessage>(&text) {
                                Ok(SignalingMessage::Sdp { sdp, room_id, .. }) => {
                                    // ponytail: only process SDP offers; ignore answer echoes
                                    let sdp_parsed: mediaservo_webrtc::RTCSessionDescription =
                                        match serde_json::from_str(&sdp) {
                                            Ok(d) => d,
                                            Err(e) => {
                                                tracing::warn!("Signaling: parse SDP: {e}");
                                                continue;
                                            }
                                        };
                                    if sdp_parsed.sdp_type != mediaservo_webrtc::RTCSdpType::Offer {
                                        tracing::debug!("Signaling: ignoring non-offer SDP ({})", sdp_parsed.sdp_type);
                                        continue;
                                    }
                                    tracing::info!("Signaling: received SDP offer");
                                    match webrtc.handle_offer(&sdp).await {
                                        Ok(answer_json) => {
                                            let answer = SignalingMessage::Sdp {
                                                room_id,
                                                target: None,
                                                sdp: answer_json,
                                            };
                                            if let Ok(json) = serde_json::to_string(&answer) {
                                                if ws.send(Message::Text(json.into())).await.is_err() {
                                                    tracing::error!("Signaling: failed to send answer SDP");
                                                    break;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Signaling: handle_offer error: {e}");
                                        }
                                    }
                                }
                                Ok(SignalingMessage::RTCIceCandidate { candidate, sdp_mid, sdp_mline_index, .. }) => {
                                    tracing::debug!("Signaling: received ICE candidate");
                                    let init_json = serde_json::json!({
                                        "candidate": candidate,
                                        "sdpMid": sdp_mid,
                                        "sdpMLineIndex": sdp_mline_index,
                                    });
                                    if let Err(e) = webrtc.handle_ice(&init_json.to_string()).await {
                                        tracing::error!("Signaling: handle_ice error: {e}");
                                    }
                                }
                                Ok(SignalingMessage::RoomLeave { peer_id, .. }) => {
                                    tracing::info!("Signaling: peer {peer_id} left room");
                                }
                                Ok(SignalingMessage::Frame { data_base64, .. }) => {
                                    match base64::engine::general_purpose::STANDARD.decode(&data_base64) {
                                        Ok(data) => {
                                            let size = data.len();
                                            // ponytail: WS frame relay still works for bootstrapping;
                                            // once WebRTC DC is established, frames arrive via on_data_channel
                                            if let Some(ref tx) = self.frame_tx {
                                                if tx.send(data).await.is_err() {
                                                    tracing::warn!("Signaling: frame receiver dropped");
                                                }
                                            }
                                            tracing::info!("Signaling: frame received ({size} bytes)");
                                        }
                                        Err(e) => {
                                            tracing::warn!("Signaling: base64 decode error: {e}");
                                        }
                                    }
                                }
                                Ok(_) => {} // ponytail: ignore uncharted variants
                                Err(_e) => {
                                    // Not a SignalingMessage — try raw JSON for Frame
                                    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&text) {
                                        if raw.get("type").and_then(|v| v.as_str()) == Some("frame") {
                                            if let Some(b64) = raw
                                                .get("data_base64")
                                                .and_then(|v| v.as_str())
                                            {
                                                match base64::engine::general_purpose::STANDARD.decode(b64) {
                                                    Ok(data) => {
                                                        let size = data.len();
                                                        if let Some(ref tx) = self.frame_tx {
                                                            if tx.send(data).await.is_err() {
                                                                tracing::warn!("Signaling: frame receiver dropped");
                                                            }
                                                        }
                                                        tracing::info!("Signaling: frame received ({size} bytes)");
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!("Signaling: base64 decode error: {e}");
                                                    }
                                                }
                                            } else {
                                                tracing::warn!("Signaling: frame missing data_base64");
                                            }
                                        } else {
                                            tracing::warn!("Signaling: unknown message: {}", &text[..text.len().min(120)]);
                                        }
                                    } else {
                                        tracing::warn!("Signaling: parse error: {_e}");
                                    }
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            tracing::info!("Signaling: connection closed");
                            break;
                        }
                        Some(Ok(Message::Ping(data))) => {
                            let _ = ws.send(Message::Pong(data)).await;
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            tracing::error!("Signaling: WebSocket error: {e}");
                            break;
                        }
                        None => break,
                    }
                }
                // Outgoing ICE candidates from WebRTC transport → relay via WS
                ice = ice_rx.recv() => {
                    match ice {
                        Some(ice_json) => {
                            if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&ice_json) {
                                let candidate = raw
                                    .get("candidate")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let sdp_mid = raw
                                    .get("sdpMid")
                                    .and_then(|v| v.as_str())
                                    .map(String::from);
                                let sdp_mline_index = raw
                                    .get("sdpMLineIndex")
                                    .and_then(|v| v.as_u64())
                                    .map(|n| n as u16);
                                let ice_msg = SignalingMessage::RTCIceCandidate {
                                    room_id: self.room_id.clone(),
                                    target: None,
                                    candidate: candidate.to_string(),
                                    sdp_mid,
                                    sdp_mline_index,
                                };
                                if let Ok(json) = serde_json::to_string(&ice_msg) {
                                    if ws.send(Message::Text(json.into())).await.is_err() {
                                        tracing::error!("Signaling: failed to send ICE candidate");
                                        break;
                                    }
                                }
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        Ok(())
    }
}
