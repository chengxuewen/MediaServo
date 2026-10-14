//! RTCDataChannel thin wrapper.
//!
//! Delegates operations to the backend (DcBackend trait)
//! via compile-time type alias dispatch.

use crate::RTCError;
use crate::backend::{ActiveDc, DcBackend};

#[derive(Debug, Clone)]
pub struct RTCDataChannelInit {
    pub ordered: bool,
    pub max_retransmit_time: Option<i32>,
    pub max_retransmits: Option<i32>,
    pub protocol: String,
    pub negotiated: bool,
    pub id: i32,
}

impl Default for RTCDataChannelInit {
    fn default() -> Self {
        Self {
            ordered: true,
            max_retransmit_time: None,
            max_retransmits: None,
            protocol: String::new(),
            negotiated: false,
            id: -1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCDataChannelState {
    Connecting,
    Open,
    Closing,
    Closed,
}

pub struct RTCDataChannel {
    pub(crate) label: String,
    pub(crate) id: i32,
    pub(crate) backend: ActiveDc,
}

impl RTCDataChannel {
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn state(&self) -> RTCDataChannelState {
        self.backend.state()
    }

    /// S6/K5: the outbound queue byte count (the congestion-waterline predicate; for preflight before dispatching an e-stop).
    pub async fn buffered_amount(&self) -> u64 {
        self.backend.buffered_amount().await
    }

    pub async fn send(&self, data: &[u8]) -> Result<(), RTCError> {
        self.backend.send(data).await
    }

    pub async fn send_text(&self, text: &str) -> Result<(), RTCError> {
        self.backend.send_text(text).await
    }

    pub async fn spool(&self) -> RTCDataChannelRx {
        self.backend.spool().await
    }

    pub async fn close(&mut self) {
        self.backend.close().await;
    }
}
#[cfg(feature = "backend-webrtc-rs")]
impl RTCDataChannel {
    pub async fn from_webrtc(dc: std::sync::Arc<webrtc::data_channel::RTCDataChannel>) -> Self {
        let label = dc.label().to_string();
        let id = dc.id() as i32;
        Self { label, id, backend: crate::backend::webrtc_rs::WebrtcRsDc::new(dc) }
    }
}
impl Clone for RTCDataChannel {
    fn clone(&self) -> Self {
        Self { label: self.label.clone(), id: self.id, backend: self.backend.clone() }
    }
}

impl std::fmt::Debug for RTCDataChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RTCDataChannel").field("id", &self.id).field("label", &self.label).finish()
    }
}

// ── Events ──

#[derive(Clone)]
pub enum RTCDataChannelEvent {
    Open,
    Closed,
    Message(RTCDataMessage),
    Error(String),
}

impl std::fmt::Debug for RTCDataChannelEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "Open"),
            Self::Closed => write!(f, "Closed"),
            Self::Message(m) => write!(f, "Message({}B)", m.data.len()),
            Self::Error(e) => write!(f, "Error({e})"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RTCDataMessage {
    pub data: Vec<u8>,
}

/// Receiver for RTCDataChannel events.
/// Created by spool() — polls the backend's event stream.
///
/// The event source = a broadcast channel (the Sender clones freely): `RTCDataChannelRx` itself is not Clone,
/// but multiple spool() calls on the same DC each yield an independent receiver (F1 review #1: the register-once semantics
/// requires spool() be callable repeatedly, each call returning a subscriber end of the shared event stream).
pub struct RTCDataChannelRx {
    rx: Option<tokio::sync::broadcast::Receiver<RTCDataChannelEvent>>,
}

impl RTCDataChannelRx {
    pub(crate) fn new(rx: Option<tokio::sync::broadcast::Receiver<RTCDataChannelEvent>>) -> Self {
        Self { rx }
    }

    #[allow(dead_code)] // the stub/webrtc-rs posture constructor (unused in the sfu posture = the PIT-203 family)
    pub(crate) fn stub() -> Self {
        Self { rx: None }
    }

    pub async fn recv(&mut self) -> Option<RTCDataChannelEvent> {
        match &mut self.rx {
            Some(rx) => match rx.recv().await {
                Ok(ev) => Some(ev),
                // Lagged: the subscriber fell behind and events were dropped -- returns an Error event (visible to the consumer, not silent)
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    Some(RTCDataChannelEvent::Error(format!("lagged {n} events")))
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
            },
            None => std::future::pending().await,
        }
    }
}

// ── Tests ──

#[cfg(all(test, not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys"))))]
mod tests {
    use super::*;

    #[test]
    fn stub_data_channel_state_is_closed() {
        let dc = RTCDataChannel { label: "test".into(), id: 0, backend: Default::default() };
        assert_eq!(dc.state(), RTCDataChannelState::Closed);
    }

    #[test]
    fn stub_label_and_id() {
        let dc = RTCDataChannel { label: "mylabel".into(), id: 42, backend: Default::default() };
        assert_eq!(dc.label(), "mylabel");
        assert_eq!(dc.id(), 42);
    }

    #[test]
    fn stub_send_is_noop() {
        let dc = RTCDataChannel { label: "x".into(), id: 0, backend: Default::default() };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            assert!(dc.send(b"hello").await.is_ok());
            assert!(dc.send_text("world").await.is_ok());
        });
    }

    #[test]
    fn stub_data_channel_init_defaults() {
        let init = RTCDataChannelInit::default();
        assert!(init.ordered);
        assert_eq!(init.max_retransmit_time, None);
        assert_eq!(init.max_retransmits, None);
        assert_eq!(init.protocol, "");
        assert!(!init.negotiated);
        assert_eq!(init.id, -1);
    }
}
