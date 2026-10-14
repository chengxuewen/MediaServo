//! The signaling abstraction surface -- one shape shared by session/control, for mock tests (the I2 semantics pinned by fixtures).
//!
//! The production impl = [`LinkSignal`] (wrapping mediaservo_link::SignalSession);
//! the test impl = MockSignal inside tests/sfu_surface.rs (an in-memory broadcast + uplink collection).

use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use mediaservo_common::protocol::SignalingMessage;
use mediaservo_link::{SignalEvent, SignalSession};
use tokio::sync::broadcast;

use crate::error::ClientError;

/// The signaling transport surface -- session/control share one trait; tests inject MockSignal (I2).
#[async_trait]
pub trait Signal: Send + Sync {
    /// Send one uplink signaling message.
    async fn send(&self, msg: SignalingMessage) -> Result<(), ClientError>;
    /// Subscribe the downlink event stream (broadcast: multiple subscribers consume independently).
    fn events(&self) -> broadcast::Receiver<SignalEvent>;
    /// The room ID (fixed after connect).
    fn room_id(&self) -> &str;
    /// This side's SFU peer key ("remote"/"consumer"/"host").
    fn sfu_peer_id(&self) -> &str;
    /// The negotiated dialect version (S0).
    fn negotiated(&self) -> u32;
    /// Close the session (releases the WS connection + background tasks).
    async fn close(&self) -> Result<(), ClientError>;
}

/// The production impl: wraps link::SignalSession (a tokio::sync::Mutex preserving close ownership).
pub struct LinkSignal {
    session: tokio::sync::Mutex<Option<SignalSession>>,
    room: String,
    sfu_peer: String,
    /// S6/K1: atomically updated after a reconnect swap (trait read points go lock-free).
    negotiated: AtomicU32,
}

impl LinkSignal {
    pub fn new(session: SignalSession, sfu_peer: String) -> Self {
        let room = session.room_id().to_string();
        let negotiated = AtomicU32::new(session.negotiated_protocol());
        Self { session: tokio::sync::Mutex::new(Some(session)), room, sfu_peer, negotiated }
    }

    /// S6/K1: after a successful supervisor reconnect, replaces the underlying session (the negotiated dialect updates with the new session;
    /// the old session has already broken = dropped, no compensating close).
    pub async fn swap_session(&self, new: SignalSession) {
        self.negotiated.store(new.negotiated_protocol(), Ordering::Release);
        *self.session.lock().await = Some(new);
    }

    /// S6/K1: the current session's a2 one-shot re-mount ticket (old servers / never issued = None -> a full join).
    pub async fn session_nonce(&self) -> Option<String> {
        self.session.lock().await.as_ref().and_then(|s| s.session_nonce().map(str::to_string))
    }
}

#[async_trait]
impl Signal for LinkSignal {
    async fn send(&self, msg: SignalingMessage) -> Result<(), ClientError> {
        let guard = self.session.lock().await;
        match guard.as_ref() {
            Some(s) => s.send(msg).await.map_err(ClientError::Signal),
            None => Err(ClientError::InvalidState("session closed".into())),
        }
    }

    fn events(&self) -> broadcast::Receiver<SignalEvent> {
        // std::sync::Mutex short lock — broadcast::subscribe is sync, no await
        let guard = self.session.blocking_lock();
        match guard.as_ref() {
            Some(s) => s.events(),
            None => {
                // return a receiver that never yields (the closed-state fallback)
                let (_, rx) = broadcast::channel(1);
                rx
            }
        }
    }

    fn room_id(&self) -> &str {
        &self.room
    }

    fn sfu_peer_id(&self) -> &str {
        &self.sfu_peer
    }

    fn negotiated(&self) -> u32 {
        self.negotiated.load(Ordering::Acquire)
    }

    async fn close(&self) -> Result<(), ClientError> {
        let session = self.session.lock().await.take();
        if let Some(s) = session {
            s.close().await.map_err(ClientError::Signal)?;
        }
        Ok(())
    }
}
