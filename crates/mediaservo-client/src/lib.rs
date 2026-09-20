//! # mediaservo-client — 舱端/消费侧 SDK v2
//!
//! 最小闭环：账号登录（REST JWT）→ 信令入房（link SignalClient，JWT 经
//! `Sec-WebSocket-Protocol` 头）→ SFU 视频消费（on_track I420 帧流）→
//! 控制 DataChannel（ControlEnvelope/ControlAck，SFU-DC 形，与 host S1 镜像）。
//!
//! 边界纪律：C12（WebRTC 仅经 mediaservo-webrtc）、C21（禁依 mediasoup/server）、
//! 禁依 field/deck/codec（consume 序列本地镜像 field::PullSession 形状）、
//! 无硬编码端点/凭证（全部调用方传入）。
//!
//! ## 5 分钟上手
//!
//! ```ignore
//! use mediaservo_client::{auth::login, ClientConfig, RoomSession};
//! use mediaservo_common::protocol::PeerRole;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let base = "http://10.0.0.2:9800";            // 调用方提供，无默认值
//! let token = login(base, "operator", "<pw>").await?;
//! let cfg = ClientConfig {
//!     signaling_url: "ws://10.0.0.2:9800/ws".into(),
//!     room_id: "vehicle_1".into(),
//!     psk: None,
//!     jwt: Some(token.jwt),
//!     role: PeerRole::Consumer,
//!     hmac_key: None,
//! };
//! let mut session = RoomSession::connect(&cfg).await?;
//! // 视频：发现 producer → 消费，帧从 mpsc receiver 取（latest 语义，容量 3）
//! let producer = session.wait_video_producer(std::time::Duration::from_secs(10)).await?;
//! let mut frames = session.consume_video(&producer).await?;
//! let frame = frames.recv().await.unwrap();
//! // 控制：开 chassis 通道（要求谈成方言 ≥2），发命令等回执
//! let mut ctl = session.open_control(&["chassis"]).await?;
//! ctl.send("chassis", 1, "steer", serde_json::json!({"deg": 10})).await?;
//! let ack = ctl.recv_ack(std::time::Duration::from_secs(5)).await?;
//! assert_eq!(ack.ack, 1);
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod config;
pub mod control;
pub mod engine;
pub mod error;
pub mod session;
pub mod sfu;
pub mod signal;

pub use auth::{LoginOutcome, RoomInfo, list_rooms, login};
pub use config::ClientConfig;
pub use control::ControlChannel;
pub use error::ClientError;
pub use session::{RoomSession, VideoFrame, VideoStreamStats};
