//! mediaservo-webrtc — multi-backend W3C WebRTC API wrapper.
//!
//! Provides RTCPeerConnection and RTCDataChannel types through
//! [`RTCEngine::create_factory`] as the unified entry point.
//! Two backends:
//! - `backend-webrtc-rs` feature: real webrtc-rs implementation
//! - default (no feature): stub for compilation without WebRTC

#![allow(async_fn_in_trait)]
// W3C 镜像 trait 的 async 签名 = 设计形（auto-trait-bound 限制随上游 async trait 稳定退役）
#![allow(private_interfaces)] // backend 访问器返回 cfg 姿态 opaque 类型（Active*）；'进包≠公开面' F12 裁定，不公开后端类型名
pub mod backend;
pub mod data_channel;
pub mod engine;
pub mod factory;
pub mod peer_connection;
pub mod rtp;
pub mod sdp;
pub mod stats;
pub mod track;
pub mod track_sink;
pub mod traits;

// Re-export backend-specific types for examples/tests
pub use backend::TrackWriteBackend;
pub use data_channel::*;
pub use engine::*;
pub use factory::*;
pub use peer_connection::*;
pub use rtp::*;
pub use sdp::*;
pub use stats::*;
pub use track::*;
pub use track_sink::*;

/// Error type for all WebRTC operations.
#[derive(Debug, thiserror::Error)]
pub enum RTCError {
    #[error("RTCPeerConnection error: {0}")]
    RTCPeerConnection(String),
    #[error("RTCDataChannel error: {0}")]
    RTCDataChannel(String),
    #[error("SDP error: {0}")]
    Sdp(String),
    #[error("Track error: {0}")]
    Track(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Not supported by backend: {0}")]
    NotSupported(String),
}

#[cfg(feature = "backend-webrtc-rs")]
impl From<webrtc::error::Error> for RTCError {
    fn from(e: webrtc::error::Error) -> Self {
        RTCError::Internal(e.to_string())
    }
}

impl RTCError {
    /// Return a stable, context-free identifier for this error.
    /// Used by future i18n layers to look up locale-specific text.
    pub fn locale_key(&self) -> &'static str {
        match self {
            RTCError::RTCPeerConnection(_) => "RTCPC",
            RTCError::RTCDataChannel(_) => "RTCDC",
            RTCError::Sdp(_) => "RTCSD",
            RTCError::Track(_) => "RTCTK",
            RTCError::Internal(_) => "RTCIN",
            RTCError::NotSupported(_) => "RTCNS",
        }
    }
}

/// Re-export webrtc-rs for callback types used by consumers.
#[cfg(feature = "backend-webrtc-rs")]
pub use webrtc;
