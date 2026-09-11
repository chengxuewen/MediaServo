//! WebSocket signaling client — connects to mediaservo-server /ws endpoint.
//!
//! # Protocol flow
//! 1. Connect to `{server_url}/ws`
//! 2. Send raw PSK as first text message
//! 3. Wait for auth acknowledgment (`Error { code: 0 }`)
//! 4. Send `RoomJoin { room_id, peer_role: Host }`
//! 5. Wait for `RoomJoined { room_id, peer_id }`
//! 6. Return split sender/receiver for SDP/ICE relay

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::error::CoreError;
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

/// Sender half of the signaling WebSocket (for sending SDP/ICE messages).
pub type WsSender = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Receiver half of the signaling WebSocket (for receiving relayed messages).
pub type WsReceiver = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// WebSocket signaling client that connects to the mediaservo-server /ws endpoint.
pub struct SignalingClient {
    server_url: String,
    psk: String,
    room_id: String,
}

impl SignalingClient {
    /// Create a new signaling client.
    ///
    /// # Arguments
    /// * `server_url` — base server URL (e.g., `ws://192.168.1.1:9800`)
    /// * `psk` — pre-shared key for authentication
    /// * `room_id` — room identifier to join
    pub fn new(server_url: &str, psk: &str, room_id: &str) -> Self {
        Self {
            server_url: server_url.trim_end_matches('/').to_string(),
            psk: psk.to_string(),
            room_id: room_id.to_string(),
        }
    }

    /// Connect to the server, authenticate with PSK, and join the room.
    ///
    /// Returns the split sender/receiver on success. The caller is responsible
    /// for sending SDP/ICE messages through the sender and reading relayed
    /// messages from the receiver.
    pub async fn connect(&self) -> Result<(WsSender, WsReceiver), CoreError> {
        let url = self.server_url.clone();
        tracing::info!("Signaling: connecting to {url}");
        let (ws_stream, _resp) = connect_async(&url).await.map_err(|e| {
            CoreError::WebSocketDisconnect(format!("connect to {}: {}", url, e))
        })?;

        let (mut sender, mut receiver) = ws_stream.split();

        // Phase 1: PSK authentication — send raw token as first message
        sender
            .send(Message::Text(self.psk.clone().into()))
            .await
            .map_err(|e| CoreError::WebSocketDisconnect(format!("send auth: {}", e)))?;

        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: SignalingMessage =
                    serde_json::from_str(&text).map_err(|e| {
                        CoreError::ConfigParse(format!("parse auth response: {}", e))
                    })?;
                match msg {
                    SignalingMessage::Error { code, .. } if code == 0 => {
                        tracing::info!("Signaling PSK auth accepted");
                    }
                    SignalingMessage::Error { code, message } => {
                        return Err(CoreError::Unknown(format!(
                            "auth denied [{code}]: {message}"
                        )));
                    }
                    _ => return Err(CoreError::PskAuthFailed),
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(CoreError::WebSocketDisconnect(
                    "connection closed during auth".into(),
                ));
            }
            Some(Err(e)) => {
                return Err(CoreError::WebSocketDisconnect(format!(
                    "auth read error: {}", e
                )));
            }
            _ => return Err(CoreError::PskAuthFailed),
        }

        // Phase 2: Join room
        let join_msg = SignalingMessage::RoomJoin {
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            room_id: self.room_id.clone(),
            peer_role: PeerRole::Host,
            stream_id: None,
        };
        let join_json = serde_json::to_string(&join_msg).map_err(|e| {
            CoreError::ConfigParse(format!("serialize RoomJoin: {}", e))
        })?;

        sender
            .send(Message::Text(join_json.into()))
            .await
            .map_err(|e| CoreError::WebSocketDisconnect(format!("send RoomJoin: {}", e)))?;

        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: SignalingMessage =
                    serde_json::from_str(&text).map_err(|e| {
                        CoreError::ConfigParse(format!(
                            "parse RoomJoined response: {}",
                            e
                        ))
                    })?;
                match msg {
                    SignalingMessage::RoomJoined { .. } => {
                        tracing::info!("Joined room '{}'", self.room_id);
                    }
                    SignalingMessage::Error { code, message } => {
                        return Err(CoreError::Unknown(format!(
                            "room join failed [{code}]: {message}"
                        )));
                    }
                    _ => {
                        return Err(CoreError::Unknown(
                            "unexpected response to RoomJoin".into(),
                        ));
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(CoreError::WebSocketDisconnect(
                    "connection closed during room join".into(),
                ));
            }
            Some(Err(e)) => {
                return Err(CoreError::WebSocketDisconnect(format!(
                    "RoomJoin read error: {}", e
                )));
            }
            _ => {
                return Err(CoreError::WebSocketDisconnect(
                    "no response to RoomJoin".into(),
                ));
            }
        }

        tracing::info!(
            "Signaling client ready — connected to {}, room={}",
            self.server_url,
            self.room_id
        );
        Ok((sender, receiver))
    }
}
