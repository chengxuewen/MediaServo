//! S6/K4: the Consumer object model -- one handle per subscription path, no cross-path coupling.
//!
//! Shape reconciliation (frozen design):
//! - [`ConsumerSlot`] = the session-side ledger entry (frame outlet + current recv pc); the K1 re-mount replay
//!   swaps the pc not the slot -- the [`Consumer::frames`] receiver the app holds keeps streaming across reconnects;
//! - [`Consumer`] = the app-side handle (**no Drop impl**): `close(self)` consumes itself, which
//!   removes the receiver; the ledger slot is declared dead via `frame_tx.is_closed()` (the replay skips that path).
//!
//! The legacy `consume_video` form stays as a bridge (returns the bare receiver), byte-identical behavior (R3).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mediaservo_webrtc::stats::RTCStats;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::engine::PcHandle;
use crate::session::{VideoFrame, VideoStreamStats, fold_inbound_stats};

/// The converted QOE snapshot (consumer-qoe-callback: units aligned with TRRO OnMediaState).
///
/// `epoch` = the differential-baseline generation -- the first round after a renegotiation/pc swap returns 0s and bumps the generation; consumers use it only
/// for break identification, **never for business decisions** (plan §5 gate).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct QoeSnapshot {
    pub fps: u32,
    /// bitrate kbps (differential bytesReceived inside the SDK, rounded; first round placeholder 0)
    pub kbps: u32,
    /// loss % (packets_lost / (lost + received) x 100; 0 when received=0)
    pub lost_percent: f32,
    pub decoded_total: u64,
    /// average decode ms/frame (delta totalDecodeTime x 1000 / delta framesDecoded; first round/no increment = 0)
    pub avg_decode_ms: u32,
    /// jitter ms (inbound jitter seconds x 1000, rounded; max across paths)
    pub jitter_ms: u32,
    /// RTT ms (candidate-pair currentRoundTripTime; 0 when the backend has no source)
    pub rtt_ms: u32,
    pub width: u32,
    pub height: u32,
    pub epoch: u32,
}

/// The differential state machine (exclusive within the slot -- plan §2.2).
///
/// `last_ts == 0` = baseline not built (first round returns 0 placeholders); a pc swap (the K1 replay) goes through
/// `ConsumerSlot::set_pc`, which calls `bump_epoch`, bumping the generation and resetting the baseline.
#[derive(Debug, Default)]
pub(crate) struct QoeDiffer {
    last_bytes: u64,
    last_ts_ms: u64,
    last_decode_time: f64,
    last_frames_decoded: u64,
    epoch: u32,
}

impl QoeDiffer {
    /// Fold the latest stats -> snapshot. `now_ms` injected by the caller's clock (testability).
    pub(crate) fn diff(&mut self, stats: &VideoStreamStats, now_ms: u64) -> QoeSnapshot {
        let mut kbps = 0u32;
        let mut avg_decode_ms = 0u32;
        if self.last_ts_ms > 0 && now_ms > self.last_ts_ms {
            let elapsed_s = (now_ms - self.last_ts_ms) as f64 / 1000.0;
            let delta = stats.bytes_received.saturating_sub(self.last_bytes);
            kbps = ((delta as f64 * 8.0 / elapsed_s) / 1000.0).round() as u32;
            // the decode-time differential ratio (same convention as host-streamer avg_encode_ms):
            // delta totalDecodeTime x 1000 / delta framesDecoded; renegotiation reset/no increment -> 0.
            let delta_decoded = stats.frames_decoded.saturating_sub(self.last_frames_decoded);
            if delta_decoded > 0 {
                let delta_decode_s = (stats.total_decode_time - self.last_decode_time).max(0.0);
                avg_decode_ms = (delta_decode_s * 1000.0 / delta_decoded as f64).round() as u32;
            }
        }
        self.last_bytes = stats.bytes_received;
        self.last_ts_ms = now_ms;
        self.last_decode_time = stats.total_decode_time;
        self.last_frames_decoded = stats.frames_decoded;
        let lost = stats.packets_lost;
        let total = lost + stats.packets_received;
        QoeSnapshot {
            fps: stats.frames_per_second.round() as u32,
            kbps,
            lost_percent: if total > 0 { (lost as f64 / total as f64 * 100.0) as f32 } else { 0.0 },
            decoded_total: stats.frames_decoded,
            avg_decode_ms,
            jitter_ms: (stats.jitter * 1000.0).round() as u32,
            rtt_ms: 0, // connection_stats sampled at the tick fills this (keeps the diff signature testable)
            width: stats.frame_width,
            height: stats.frame_height,
            epoch: self.epoch,
        }
    }

    pub(crate) fn bump_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.last_ts_ms = 0; // the next round re-placeholders (break identification)
        self.last_decode_time = 0.0;
        self.last_frames_decoded = 0;
    }
}

/// One consumer path's shared skeleton (the session ledger holds an Arc; the app handle holds one too).
///
/// Liveness predicate = every receiver of `frame_tx` dropped (Consumer drop / bare receiver drop,
/// same shape); the slot itself is reclaimed with the session ledger (no per-path removal, same semantics as the old `_pcs`).
pub struct ConsumerSlot {
    producer_id: String,
    frame_tx: mpsc::Sender<VideoFrame>,
    /// The current recv pc -- replaced wholesale once a replay (K1) succeeds; None = establishing/dead.
    pc: Mutex<Option<Arc<dyn PcHandle>>>,
    /// The QOE differential state (consumer-qoe-callback) -- bumps the epoch on pc swap.
    pub(crate) qoe_differ: Mutex<QoeDiffer>,
}

impl ConsumerSlot {
    pub(crate) fn new(producer_id: &str, frame_tx: mpsc::Sender<VideoFrame>) -> Self {
        Self {
            producer_id: producer_id.to_string(),
            frame_tx,
            pc: Mutex::new(None),
            qoe_differ: Mutex::new(QoeDiffer::default()),
        }
    }

    pub(crate) fn set_pc(&self, pc: Arc<dyn PcHandle>) {
        *self.pc.lock().unwrap_or_else(|e| e.into_inner()) = Some(pc);
        // renegotiation = a media-path break: the differential baseline resets + the generation bumps (the next round returns 0 placeholders).
        self.qoe_differ.lock().unwrap_or_else(|e| e.into_inner()).bump_epoch();
    }

    #[must_use]
    pub(crate) fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// All receivers dropped = this path's app side has withdrawn (the K1 replay skip predicate).
    #[must_use]
    pub(crate) fn is_dead(&self) -> bool {
        self.frame_tx.is_closed()
    }

    /// The frame outlet clone (feeds the replayed new pc's on_track sinks; the slot holds the original = the persistence anchor).
    pub(crate) fn frame_tx(&self) -> mpsc::Sender<VideoFrame> {
        self.frame_tx.clone()
    }

    /// The current pc's "video" inbound-rtp readout (no pc = empty table).
    pub(crate) fn receiver_stats(&self) -> Vec<RTCStats> {
        let guard = self.pc.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(pc) => pc.receiver_stats("video"),
            None => Vec::new(),
        }
    }
}

/// The app-side single-path video consumption handle (the output of [`crate::RoomSession::consume`]).
///
/// The frame stream and stats share the ledger slot as their source: after a reconnect replay frames continue from the **same** receiver
/// (imperceptible to the app), stats automatically switch to the new pc. No Drop impl -- withdrawing means `close(self)`
/// or directly dropping the handle/bare receiver (same semantics: receivers closed -> the replay declares it dead).
pub struct Consumer {
    producer_id: String,
    slot: Arc<ConsumerSlot>,
    frames: mpsc::Receiver<VideoFrame>,
    /// Captured at construction (inside async consume() = runtime context) -- on_qoe can then safely register from any
    /// non-runtime thread (a C++ worker). PIT-216: Handle::current() taken at the call site would panic (the TPHMI worker has no runtime).
    /// taken at the call site would panic (the TPHMI worker has no runtime).
    rt: tokio::runtime::Handle,
}

impl Consumer {
    pub(crate) fn new(
        producer_id: String,
        slot: Arc<ConsumerSlot>,
        frames: mpsc::Receiver<VideoFrame>,
    ) -> Self {
        Self { producer_id, slot, frames, rt: tokio::runtime::Handle::current() }
    }

    /// The consumed producer id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.producer_id
    }

    /// The frame receiver borrow (latest-wins semantics, capacity 3 -- overflow drops carried by the pump's try_send).
    pub fn frames(&mut self) -> &mut mpsc::Receiver<VideoFrame> {
        &mut self.frames
    }

    /// Hand over the bare receiver (the equivalent of the old `consume_video` return shape).
    ///
    /// Replay-deregistration semantics: this method consumes the handle and hands over the receiver -- the ledger slot's
    /// liveness predicate is `frame_tx` **receivers closed**, i.e. this path is declared dead the moment the handed-over
    /// receiver drops (handing over does not detach; holding the receiver = staying replay-eligible).
    #[must_use]
    pub fn into_receiver(self) -> mpsc::Receiver<VideoFrame> {
        self.frames
    }

    /// This path's video stats (the K4 gain: single-path readout, without the session summary).
    /// Never touches the QOE differential state (qoe_snapshot/on_qoe own it exclusively -- pulling and pushing never pollute each other's baselines).
    #[must_use]
    pub fn stats(&self) -> VideoStreamStats {
        fold_inbound_stats(self.slot.receiver_stats())
    }

    /// The current QOE snapshot (shares the differential state with [`Self::on_qoe`]; a pull is one tick).
    #[must_use]
    pub fn qoe_snapshot(&self) -> QoeSnapshot {
        let stats = fold_inbound_stats(self.slot.receiver_stats());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.slot.qoe_differ.lock().unwrap_or_else(|e| e.into_inner()).diff(&stats, now)
    }

    /// The QOE periodic callback (consumer-qoe-callback: mirrors the TRRO OnMediaState shape).
    ///
    /// The SDK's built-in cadence/differentials/conversion -- the callback receives a ready [`QoeSnapshot`]. Idempotent: re-registering
    /// replaces the previous callback; dropping the Consumer or `close(self)` stops it. The callback fires inside the SDK's
    /// cadence task -- **blocking inside the callback is forbidden** (plan §5 gate).
    pub fn on_qoe(
        &self,
        period: Duration,
        cb: Box<dyn Fn(QoeSnapshot) + Send + Sync>,
    ) -> tokio::task::JoinHandle<()> {
        self.on_qoe_inner(period, cb)
    }

    pub(crate) fn on_qoe_inner(
        &self,
        period: Duration,
        cb: Box<dyn Fn(QoeSnapshot) + Send + Sync>,
    ) -> tokio::task::JoinHandle<()> {
        // a sync fn -- the C++/FFI side calls it from any thread. The Handle was captured in Consumer::new
        // (the async consume() context) -- never taking current() here (PIT-216).
        let handle = self.rt.clone();
        let slot = Arc::clone(&self.slot);
        handle.spawn(async move {
            let mut tick = tokio::time::interval(period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                // the slot dead (all app-side receivers dropped) -> stop the cadence
                if slot.is_dead() {
                    break;
                }
                let stats = fold_inbound_stats(slot.receiver_stats());
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let mut snap =
                    slot.qoe_differ.lock().unwrap_or_else(|e| e.into_inner()).diff(&stats, now);
                // rtt connection-level sample (no backend source/no succeeded pair -> None -> 0)
                if let Some(conn) = slot
                    .pc
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .and_then(|pc| pc.connection_stats())
                {
                    snap.rtt_ms = conn.rtt_ms.round() as u32;
                }
                cb(snap);
            }
        })
    }

    /// Actively withdraw this path (consumes self -> the receiver drops -> the slot declared dead, the replay no longer continues it).
    /// The pc is reclaimed with the session (same lifecycle semantics as the old `_pcs`).
    pub fn close(self) {
        // an empty body: self leaving scope drops the frames receiver + the slot Arc reference.
    }
}

#[cfg(test)]
mod qoe_tests {
    use super::*;
    use crate::session::VideoStreamStats;
    use mediaservo_webrtc::stats::{RTCInboundRtpStreamStats, RTCStats};

    fn stats_of(bytes: u64, fps: f64, lost: u64, received: u64, decoded: u64) -> VideoStreamStats {
        VideoStreamStats {
            bytes_received: bytes,
            frames_per_second: fps,
            packets_lost: lost,
            packets_received: received,
            frames_decoded: decoded,
            frame_width: 1920,
            frame_height: 1080,
            ..VideoStreamStats::default()
        }
    }

    fn stats_with_decode(decoded: u64, total_decode_s: f64, jitter_s: f64) -> VideoStreamStats {
        VideoStreamStats {
            frames_decoded: decoded,
            total_decode_time: total_decode_s,
            jitter: jitter_s,
            ..VideoStreamStats::default()
        }
    }

    #[test]
    fn qoe_first_tick_emits_zero_placeholder() {
        // the PIT-2026-10-09 pin: baseline not built (last_ts=0), the first round returns 0 placeholders -- the consumer UI
        // pipeline recognizes the key on its first round (the old pull-style first-round absence + integer truncation = the root cause of the stuck-at-0 bitrate illusion).
        let mut d = QoeDiffer::default();
        let snap = d.diff(&stats_of(100_000, 30.0, 0, 1000, 60), 1_000);
        assert_eq!(snap.kbps, 0, "first round placeholder 0");
        assert_eq!(snap.fps, 30, "instantaneous values are real from the first round");
        assert_eq!(snap.decoded_total, 60);
        assert_eq!(snap.epoch, 0);
    }

    #[test]
    fn qoe_diff_rounds_and_scales() {
        // 300KB/1s = 2400 kbps; loss 2%
        let mut d = QoeDiffer::default();
        let _ = d.diff(&stats_of(0, 30.0, 0, 0, 0), 1_000);
        let snap = d.diff(&stats_of(300_000, 30.0, 20, 980, 120), 2_000);
        assert_eq!(snap.kbps, 2400);
        assert!((snap.lost_percent - 2.0).abs() < 1e-4, "lost% = 20/1000");
        assert_eq!(snap.decoded_total, 120);
    }

    #[test]
    fn qoe_epoch_bump_resets_baseline() {
        // the K1 replay swapping the pc -> bump_epoch: the next round returns 0 placeholders (break identification)
        let mut d = QoeDiffer::default();
        let _ = d.diff(&stats_of(0, 30.0, 0, 0, 0), 1_000);
        let mid = d.diff(&stats_of(300_000, 30.0, 0, 0, 0), 2_000);
        assert_eq!(mid.kbps, 2400);
        d.bump_epoch();
        let after = d.diff(&stats_of(900_000, 30.0, 0, 0, 0), 3_000);
        assert_eq!(after.epoch, 1);
        assert_eq!(after.kbps, 0, "first round returns 0 placeholders after renegotiation");
        let next = d.diff(&stats_of(1_200_000, 30.0, 0, 0, 0), 4_000);
        assert_eq!(next.kbps, 2400);
    }

    #[test]
    fn qoe_jitter_passes_through_ms() {
        // qoe-full-metrics: jitter seconds -> ms passthrough rounded (0.0037s -> 4ms)
        let mut d = QoeDiffer::default();
        let _ = d.diff(&stats_with_decode(60, 1.0, 0.0037), 1_000);
        let snap = d.diff(&stats_with_decode(120, 2.0, 0.0037), 2_000);
        assert_eq!(snap.jitter_ms, 4, "jitter*1000 rounded");
    }

    #[test]
    fn qoe_avg_decode_ratio() {
        // delta decode=1.0s / delta frames=100 -> 10ms/frame; first-round placeholder 0
        let mut d = QoeDiffer::default();
        let first = d.diff(&stats_with_decode(0, 0.0, 0.0), 1_000);
        assert_eq!(first.avg_decode_ms, 0, "first-round placeholder");
        let snap = d.diff(&stats_with_decode(100, 1.0, 0.0), 2_000);
        assert_eq!(snap.avg_decode_ms, 10);
        // no frame increments -> 0 (divide-by-zero guard)
        let flat = d.diff(&stats_with_decode(100, 1.5, 0.0), 3_000);
        assert_eq!(flat.avg_decode_ms, 0);
        // renegotiation reset (totalDecodeTime regressed) -> negative differentials clamped to 0
        d.bump_epoch();
        let _ = d.diff(&stats_with_decode(200, 0.5, 0.0), 4_000);
        let after = d.diff(&stats_with_decode(300, 0.7, 0.0), 5_000);
        assert_eq!(after.avg_decode_ms, 2, "(0.7-0.5)s/100f");
    }

    #[test]
    fn qoe_rtt_none_yields_zero() {
        // the diff layer's rtt stays 0 (connection_stats fills it at the tick); None -> stays 0
        let mut d = QoeDiffer::default();
        let snap = d.diff(&stats_of(300_000, 30.0, 0, 1000, 60), 2_000);
        assert_eq!(snap.rtt_ms, 0);
    }
}
