//! # mediaservo-client -- the cockpit/consumer-side SDK v2
//!
//! The minimal loop: account login (REST JWT) -> signaling join (link SignalClient, JWT via the
//! `Sec-WebSocket-Protocol` header) -> SFU video consume (on_track I420 frame stream) ->
//! control DataChannel (ControlEnvelope/ControlAck, the SFU-DC form, mirroring host S1).
//!
//! Boundary discipline: C12 (WebRTC only via mediaservo-webrtc), C21 (never depend on mediasoup/server),
//! never depend on field/deck/codec (the consume sequence locally mirrors field::PullSession's shape),
//! no hardcoded endpoints/credentials (everything passed in by the caller).
//!
//! ## 5-minute quickstart
//!
//! ```ignore
//! use mediaservo_client::{auth::login, ClientConfig, RoomSession};
//! use mediaservo_common::protocol::PeerRole;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let base = "http://10.0.0.2:9800";            // caller-provided, no default
//! let token = login(base, "operator", "<pw>").await?;
//! let cfg = ClientConfig {
//!     signaling_url: "ws://10.0.0.2:9800/ws".into(),
//!     room_id: "vehicle_1".into(),
//!     psk: None,
//!     jwt: Some(token.jwt),
//!     role: PeerRole::Consumer,
//!     hmac_key: None,
//!     identity_dir: None,
//! };
//! let mut session = RoomSession::connect(&cfg).await?;
//! // video: discover a producer -> consume, frames come from an mpsc receiver (latest-wins, capacity 3)
//! let producer = session.wait_video_producer(std::time::Duration::from_secs(10)).await?;
//! let mut frames = session.consume_video(&producer).await?;
//! let frame = frames.recv().await.unwrap();
//! // control: open the chassis channel (requires negotiated dialect >= 2), send a command and await the receipt
//! let mut ctl = session.open_control(&["chassis"]).await?;
//! ctl.send("chassis", 1, "steer", serde_json::json!({"deg": 10})).await?;
//! let ack = ctl.recv_ack(std::time::Duration::from_secs(5)).await?;
//! assert_eq!(ack.ack, 1);
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod config;
pub mod consumer;
pub mod control;
pub mod engine;
pub mod error;
pub mod session;
pub mod sfu;
pub mod signal;
pub mod supervisor;

pub use auth::{LoginOutcome, RoomInfo, exchange, list_rooms, list_rooms_psk, login};
pub use config::ClientConfig;
pub use consumer::Consumer;
pub use control::ControlChannel;
pub use error::ClientError;
pub use mediaservo_webrtc::data_channel::RTCDataChannelState;
pub use session::{RoomSession, VideoFrame, VideoStreamStats};
pub use supervisor::ConnectionState;
