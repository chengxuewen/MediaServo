//! The control-plane SFU DataChannel handle (the S2 rewrite -- the old HMAC-P2P form retired with main.rs).
//!
//! The wire shape = [`ControlEnvelope`]/[`ControlAck`] (the single truth source in common::protocol,
//! paired with host S1's `mediaservo-host::controller`). Receipts come back via the **same-label DC echo**
//! (host `DcAckSink` dual-writes: the ack main-path DC + an inbound DC echo -- client only needs the echo,
//! sparing a Recv consumption chain). Zero HMAC, zero PSK signatures.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mediaservo_common::protocol::{ControlAck, ControlEnvelope};
use tokio::sync::mpsc;

use mediaservo_webrtc::data_channel::RTCDataChannelState;

use crate::engine::{DcHandle, PcHandle};
use crate::error::ClientError;

impl std::fmt::Debug for ControlChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlChannel")
            .field("labels", &self.labels)
            .field("producer_ids", &self.producer_ids)
            .finish_non_exhaustive()
    }
}

/// The egress control channel set established by one `open_control`.
///
/// Construction happens only via [`crate::RoomSession::open_control`] (the transport/DC/announce sequence
/// lives on the session side); this type only sends/receives and keeps things alive (holding the pc reference prevents early release).
pub struct ControlChannel {
    dcs: HashMap<String, Arc<dyn DcHandle>>,
    labels: Vec<String>,
    ack_rx: mpsc::Receiver<ControlAck>,
    producer_ids: Vec<String>,
    /// The egress transport's PC -- held while the handle lives; dropping it disconnects the DCs.
    _pc: Arc<dyn PcHandle>,
}

impl ControlChannel {
    pub(crate) fn new(
        dcs: HashMap<String, Arc<dyn DcHandle>>,
        labels: Vec<String>,
        ack_rx: mpsc::Receiver<ControlAck>,
        producer_ids: Vec<String>,
        pc: Arc<dyn PcHandle>,
    ) -> Self {
        Self { dcs, labels, ack_rx, producer_ids, _pc: pc }
    }

    /// The established channel labels (in open_control input order).
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// The server-assigned data producer id (for auditing/debugging).
    #[must_use]
    pub fn producer_ids(&self) -> &[String] {
        &self.producer_ids
    }

    /// Send one command envelope on the given label's channel (seq maintained by the caller).
    pub async fn send(
        &self,
        label: &str,
        seq: u64,
        cmd: &str,
        payload: serde_json::Value,
    ) -> Result<(), ClientError> {
        self.send_envelope(label, &ControlEnvelope { seq, cmd: cmd.into(), payload, sig: None })
            .await
    }

    /// Envelope send (the low-level entry when already holding a [`ControlEnvelope`]).
    pub async fn send_envelope(
        &self,
        label: &str,
        env: &ControlEnvelope,
    ) -> Result<(), ClientError> {
        let dc = self.dcs.get(label).ok_or_else(|| {
            ClientError::InvalidState(format!("control DC \"{label}\" is not open"))
        })?;
        let text = serde_json::to_string(env)
            .map_err(|e| ClientError::MalformedResponse(format!("envelope serialize: {e}")))?;
        // error mapping lives on the engine side (SysDc="DC {label} send: {e}" / Fake same shape).
        dc.send_text(&text).await
    }

    /// Take the next receipt (host echo/ack broadcast share the queue); no receipt within `wait` ->
    /// [`ClientError::Timeout`]。
    pub async fn recv_ack(&mut self, wait: Duration) -> Result<ControlAck, ClientError> {
        tokio::time::timeout(wait, self.ack_rx.recv())
            .await
            .map_err(|_| ClientError::Timeout { what: "ControlAck" })?
            .ok_or_else(|| {
                ClientError::InvalidState("the ack stream closed (DC disconnected)".into())
            })
    }

    /// S6/K5: ready-state aggregation -- the **worst** state across all egress DCs (Closed > Closing > Connecting > Open).
    /// Predicate: any channel degraded = the whole set must not optimistically send (the single readout for e-stop/command preflight).
    #[must_use]
    pub fn ready_state(&self) -> RTCDataChannelState {
        fn rank(s: RTCDataChannelState) -> u8 {
            match s {
                RTCDataChannelState::Open => 0,
                RTCDataChannelState::Connecting => 1,
                RTCDataChannelState::Closing => 2,
                RTCDataChannelState::Closed => 3,
            }
        }
        self.dcs
            .values()
            .map(|dc| rank(dc.state()))
            .max()
            .map(|r| match r {
                0 => RTCDataChannelState::Open,
                1 => RTCDataChannelState::Connecting,
                2 => RTCDataChannelState::Closing,
                _ => RTCDataChannelState::Closed,
            })
            .unwrap_or(RTCDataChannelState::Open)
    }

    /// S6/K5: the summed outbound-queue waterline (buffered_amount summed over all channels).
    /// Use: backpressure preflight before dispatching bulk commands/an e-stop (a high waterline = shed load or wait first, never blind-pour).
    pub async fn buffered_amount(&self) -> u64 {
        let mut total = 0u64;
        for dc in self.dcs.values() {
            total += dc.buffered_amount().await;
        }
        total
    }

    /// Take the receipt paired with `want_seq` -- earlier stale acks (products of the retransmit window) are dropped and skipped.
    /// `wait` = the whole-window timeout (not per-message).
    pub async fn recv_ack_for(
        &mut self,
        want_seq: u64,
        wait: Duration,
    ) -> Result<ControlAck, ClientError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remain.is_zero() {
                return Err(ClientError::Timeout { what: "ControlAck" });
            }
            let ack = tokio::time::timeout(remain, self.ack_rx.recv())
                .await
                .map_err(|_| ClientError::Timeout { what: "ControlAck" })?
                .ok_or_else(|| {
                    ClientError::InvalidState("the ack stream closed (DC disconnected)".into())
                })?;
            if ack.ack == want_seq {
                return Ok(ack);
            }
            tracing::debug!(
                got = ack.ack,
                want = want_seq,
                "stale ack skipped (a retransmit-window product)"
            );
        }
    }
}
