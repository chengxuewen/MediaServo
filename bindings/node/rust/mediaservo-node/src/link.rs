//! link signaling-plane napi bindings — SignalSession (events via ThreadsafeFunction → JS main thread,
//! livekit async_queue shape: Rust broadcast thread → tsfn → JS callback).

use std::sync::Arc;

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{SignalClient, SignalEvent, SignalSession};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;

/// Signaling configuration (role: "Host"/"Pusher"→Host, "Client"/"Puller"→Remote; default Host).
#[napi(object)]
pub struct JsSignalConfig {
    pub url: String,
    pub psk: String,
    pub room: String,
    pub role: Option<String>,
}

fn closed_err() -> napi::Error {
    napi::Error::from_reason("session closed")
}

fn event_to_json(ev: &SignalEvent) -> String {
    use mediaservo_link::SignalEvent::*;
    match ev {
        Connected { room_id } => serde_json::json!({"type": "connected", "room_id": room_id}),
        Message(m) => serde_json::json!({"type": "message", "message": m}),
        Disconnected { reason } => serde_json::json!({"type": "disconnected", "reason": reason}),
        Error(e) => serde_json::json!({"type": "error", "error": e}),
        _ => serde_json::json!({"type": "unknown"}),
    }
    .to_string()
}

/// Signaling session (async; event subscription via the onEvent callback — forwarded safely from the C pump thread to the JS main thread).
#[napi]
pub struct JsSignalSession {
    inner: Arc<tokio::sync::Mutex<Option<SignalSession>>>,
}

// SAFETY: all methods are serialized through a tokio Mutex (same precedent as field-c).
unsafe impl Send for JsSignalSession {}
unsafe impl Sync for JsSignalSession {}

#[napi]
impl JsSignalSession {
    /// Connect signaling and join the room (async, PSK authentication built in).
    #[napi(factory)]
    pub async fn connect(cfg: JsSignalConfig) -> Result<Self> {
        let role = match cfg.role.as_deref().unwrap_or("Host") {
            "Host" | "Pusher" => PeerRole::Host,
            "Client" | "Puller" => PeerRole::Remote,
            other => return Err(napi::Error::from_reason(format!("unknown role: {other}"))),
        };
        let client = SignalClient::new(&cfg.url, &cfg.psk, &cfg.room, role);
        let session = client
            .connect()
            .await
            .map_err(|e| napi::Error::from_reason(format!("connect: {e}")))?;
        Ok(Self { inner: Arc::new(tokio::sync::Mutex::new(Some(session))) })
    }

    /// Send a signaling message (JSON string in SignalingMessage serde format).
    #[napi]
    pub async fn send(&self, json: String) -> Result<()> {
        let msg: SignalingMessage = serde_json::from_str(&json)
            .map_err(|e| napi::Error::from_reason(format!("invalid message: {e}")))?;
        let mut guard = self.inner.lock().await;
        guard
            .as_mut()
            .ok_or_else(closed_err)?
            .send(msg)
            .await
            .map_err(|e| napi::Error::from_reason(format!("send: {e}")))
    }

    /// Subscribe to events (JSON string callback; can be registered any time after connect, replacing the old callback).
    #[napi]
    #[allow(clippy::while_let_loop)] // keep the match-shaped pump loop with the None branch log hook; do not rewrite as while-let
    pub fn on_event(&self, cb: Function<String, ()>) -> Result<()> {
        // The TSFN's T is auto-converted to JS args (JsValuesTupleIntoVec); build() without a closure is enough.
        let tsfn = cb.build_threadsafe_function::<String>().build()?;

        let session = self.inner.clone();
        // Pump: broadcast receiver → tsfn.call (JS main thread); exits once the broadcast closes (session closed).
        // Sync methods have no tokio context → use the global shared runtime (same pattern as field-c).
        super::event_runtime().spawn(async move {
            let (room_id, rx) = {
                let guard = session.lock().await;
                match guard.as_ref() {
                    Some(s) => (s.room_id().to_string(), Some(s.events())),
                    None => (String::new(), None),
                }
            };
            let Some(mut rx) = rx else { return };
            // The Connected event is emitted inside connect() (lost before subscribing) — re-synthesize it (same approach as link-c).
            let _ = tsfn.call(
                serde_json::json!({"type": "connected", "room_id": room_id}).to_string(),
                ThreadsafeFunctionCallMode::Blocking,
            );
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let json = event_to_json(&ev);
                        let _ = tsfn.call(json, ThreadsafeFunctionCallMode::Blocking);
                    }
                    Err(_) => break, // broadcast closed (session closed)
                }
            }
        });
        Ok(())
    }

    /// Close the session (idempotent).
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        match guard.take() {
            Some(session) => {
                session.close().await.map_err(|e| napi::Error::from_reason(format!("close: {e}")))
            }
            None => Ok(()),
        }
    }
}
