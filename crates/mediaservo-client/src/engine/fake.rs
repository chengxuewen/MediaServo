//! FakeEngine -- a stateful libwebrtc semantics simulator (feature-gated `engine-fake`, off by default).
//!
//! Reason to exist (K11 / same type as mediasoup-client FakeHandler): mask the "engine behavior" layer so
//! consume/control/ack logic and failure branches run deterministically in CI at second scale -- the real webrtc-sys
//! stays only in the passthrough-posture regression. Semantic differences from the real impl are documented on each injection method; **not simulated**:
//! DTLS/ICE handshake timing, SRTP framing, SCTP congestion/retransmit, real SDP semantic validation.
//!
//! Lock model: one global `std::sync::Mutex` (collect -> release -> act again, never nested across pc locks,
//! never hold the lock across await). Dedicated to the test foundation, no throughput demands.
//! ponytail: a global lock; split into per-pc locks if drill parallelism ever contends.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

use mediaservo_webrtc::data_channel::{RTCDataChannelInit, RTCDataChannelState};
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::stats::{RTCInboundRtpStreamStats, RTCStats};
use mediaservo_webrtc::track::{FrameSink, TrackKind};

use super::{DcHandle, Engine, EngineDcEvent, EngineTrack, PcHandle, TrackHandle};
use crate::error::ClientError;

/// A fake answer SDP (shape-valid is enough -- nobody parses it in the fake domain).
const FAKE_ANSWER_SDP: &str = "v=0\r\no=mediaservo-fake 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";
/// A fake local DTLS fingerprint: 32 bytes of colon-separated hex (matching libwebrtc's output shape, for Connect uplink asserts).
const FAKE_LOCAL_FINGERPRINT: &str = "FA:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";
/// DC event broadcast capacity (drill-level traffic never bottlenecks on it).
const DC_EVENT_CAP: usize = 64;

/// Global mutable state (failure flags + the pc registry + egress bookkeeping).
struct St {
    /// Remaining injected create_pc failures.
    fail_connect: usize,
    /// Remaining injected "set_remote with a remote description already set" failures (the batch-1 resume preset).
    fail_resume: usize,
    /// Whether the next DC send is dropped.
    drop_next_send: bool,
    /// Whether a newly created DC opens immediately (the libwebrtc norm = open once connected; drill c turns it off).
    dc_auto_open: bool,
    /// DC id allocator (starting at 1, mimicking libwebrtc's increasing real ids).
    next_dc_id: i32,
    /// The live pc registry (the injection surface's addressing).
    pcs: Vec<Weak<FakePc>>,
    /// Egress bookkeeping: label -> the sequence of texts actually delivered.
    sent: HashMap<String, Vec<String>>,
}

/// The stateful fake engine. `Clone` shares the same world handle (the test-side injection surface and the session-side engine share one source).
#[derive(Clone)]
pub struct FakeEngine {
    st: Arc<Mutex<St>>,
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            st: Arc::new(Mutex::new(St {
                fail_connect: 0,
                fail_resume: 0,
                drop_next_send: false,
                dc_auto_open: true,
                next_dc_id: 1,
                pcs: Vec::new(),
                sent: HashMap::new(),
            })),
        }
    }

    /// Unified lock acquisition inside the test foundation: on poison, take the inner value and continue (a drill environment has no
    /// "dirty state untrustworthy" semantics; panic propagation would only mask the first real failure).
    fn lock(&self) -> std::sync::MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Collect live pc strong references (acting only after releasing the global lock -- the no-nested-lock discipline).
    fn live_pcs(&self) -> Vec<Arc<FakePc>> {
        let mut st = self.lock();
        st.pcs.retain(|w| w.strong_count() > 0);
        st.pcs.iter().filter_map(Weak::upgrade).collect()
    }

    // -- failure-injection surface (the minimal three, ticket K11) --

    /// The next n `create_pc` calls fail (= libwebrtc factory/ICE establishment-phase failures).
    /// Use: verify engine errors map correctly to `ClientError::WebRtc` without panicking and the session stays usable.
    pub fn fail_next_connect(&self, n: usize) {
        self.lock().fail_connect = n;
    }

    /// The next n "set_remote_offer again on an already-negotiated pc" calls fail. The batch-1 resume path
    /// = renegotiation of an existing transport shape; this flag is the preset injection point for that drill (no resume caller in current code;
    /// until the first consumer appears it is purely fake-side semantics).
    pub fn fail_next_resume(&self, n: usize) {
        self.lock().fail_resume = n;
    }

    /// The next DC send "succeeds but the packet vanishes" (= an SCTP mid-stream break swallowing: send_text returns Ok,
    /// the peer never emits a frame). Use: verify the cockpit's resend/timeout window is not misled by a fake ack.
    pub fn drop_dc_mid_send(&self) {
        self.lock().drop_next_send = true;
    }

    // -- world control surface --

    /// Whether a new DC automatically enters the open state (default true; drill c sets false to verify send-state rejection).
    pub fn set_dc_auto_open(&self, on: bool) {
        self.lock().dc_auto_open = on;
    }

    /// Manually push all registered DCs to open (paired with `set_dc_auto_open(false)`).
    pub fn open_data_channels(&self) {
        for pc in self.live_pcs() {
            for dc in pc.dcs_snapshot() {
                dc.set_open();
            }
        }
    }

    /// Deliver one video ingress track to every pc registered with on_track (= libwebrtc's
    /// "demux produced a track" moment; track_id always "video", the same source as the stats query key).
    pub fn inject_video_track(&self) {
        for pc in self.live_pcs() {
            pc.fire_on_track();
        }
    }

    /// Push one zero-value I420 frame to every attached sink and accumulate receiver stats
    /// (= the merged abstraction of RTP decrypt -> decode -> frame-out; width x height x 3/2 bytes).
    pub fn inject_frame(&self, width: u32, height: u32) {
        let data = vec![0u8; (width as usize) * (height as usize) * 3 / 2];
        for pc in self.live_pcs() {
            pc.deliver_frame(&data, width, height);
        }
    }

    /// Deliver a DC ingress message matching by label (visible to the ack pump and other consumers).
    /// Returns false = that label's consumer DC is not yet established (the caller retries; races are the test side's problem).
    pub fn deliver_dc_message(&self, label: &str, bytes: Vec<u8>) -> bool {
        let mut hit = false;
        for pc in self.live_pcs() {
            for dc in pc.dcs_snapshot() {
                if dc.label_inner() == label {
                    dc.push_message(bytes.clone());
                    hit = true;
                }
            }
        }
        hit
    }

    /// K5 drill slot: close the DC targeted by label (state -> Closed; for the worst-state ready_state aggregation assert).
    pub fn close_dc(&self, label: &str) {
        for pc in self.live_pcs() {
            for dc in pc.dcs_snapshot() {
                if dc.label_inner() == label {
                    dc.close_inner();
                }
            }
        }
    }

    /// K5 drill slot: overwrite the buffered outbound queue of DCs matching label to a given byte count (simulating a congestion waterline).
    pub fn set_dc_buffered(&self, label: &str, bytes: u64) {
        for pc in self.live_pcs() {
            for dc in pc.dcs_snapshot() {
                if dc.label_inner() == label {
                    dc.buffered.store(bytes as usize, Ordering::Release);
                }
            }
        }
    }

    /// Egress bookkeeping read: the texts actually delivered for that label (drop_dc_mid_send's don't count).
    #[must_use]
    pub fn sent_texts(&self, label: &str) -> Vec<String> {
        self.lock().sent.get(label).cloned().unwrap_or_default()
    }
}

#[async_trait]
impl Engine for FakeEngine {
    async fn create_pc(&self) -> Result<Arc<dyn PcHandle>, ClientError> {
        let mut st = self.lock();
        if st.fail_connect > 0 {
            st.fail_connect -= 1;
            return Err(ClientError::WebRtc(
                "fake: create_peer_connection failure injected".into(),
            ));
        }
        let pc = Arc::new(FakePc { st: self.st.clone(), inner: FakePcInner::default() });
        st.pcs.push(Arc::downgrade(&pc));
        Ok(pc)
    }
}

/// The on_track callback slot type (a clippy type_complexity exemption surface collapsed into an alias).
type OnTrackCb = Arc<dyn Fn(EngineTrack) + Send + Sync + 'static>;

/// The pc mutable core (callback slot / track / DCs / stats).
#[derive(Default)]
struct FakePcInner {
    on_track: Mutex<Option<OnTrackCb>>,
    had_remote: AtomicBool,
    tracks: Mutex<Vec<Arc<FakeTrack>>>,
    dcs: Mutex<Vec<Arc<FakeDc>>>,
    stats: Mutex<PcStats>,
}

#[derive(Default)]
struct PcStats {
    frames: u32,
    bytes: u64,
    width: u32,
    height: u32,
}

struct FakePc {
    st: Arc<Mutex<St>>,
    inner: FakePcInner,
}

impl FakePc {
    fn lock_st(&self) -> std::sync::MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn dcs_snapshot(&self) -> Vec<Arc<FakeDc>> {
        self.inner.dcs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn fire_on_track(&self) {
        let cb = self.inner.on_track.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(cb) = cb else { return };
        let track = Arc::new(FakeTrack::default());
        self.inner.tracks.lock().unwrap_or_else(|e| e.into_inner()).push(track.clone());
        cb(EngineTrack::new("video".into(), TrackKind::Video, track));
    }

    fn deliver_frame(&self, data: &[u8], width: u32, height: u32) {
        let tracks: Vec<Arc<FakeTrack>> =
            self.inner.tracks.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for t in &tracks {
            t.deliver(data, width, height);
        }
        let mut stats = self.inner.stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.frames = stats.frames.saturating_add(1);
        stats.bytes = stats.bytes.saturating_add(data.len() as u64);
        stats.width = width;
        stats.height = height;
    }
}

#[async_trait]
impl PcHandle for FakePc {
    fn on_track(&self, cb: Box<dyn Fn(EngineTrack) + Send + Sync + 'static>) {
        *self.inner.on_track.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::from(cb));
    }

    /// Recording acceptance (the fake doesn't validate direction/type -- the only call shape in current code = Video+Recvonly).
    fn add_transceiver(
        &self,
        _kind: TrackKind,
        _init: RTCRtpTransceiverInit,
    ) -> Result<(), ClientError> {
        Ok(())
    }

    async fn set_remote_offer(&self, _sdp: String) -> Result<(), ClientError> {
        if self.inner.had_remote.load(Ordering::Acquire) {
            let mut st = self.lock_st();
            if st.fail_resume > 0 {
                st.fail_resume -= 1;
                return Err(ClientError::WebRtc(
                    "fake: renegotiation (the resume shape) failure injected".into(),
                ));
            }
        }
        self.inner.had_remote.store(true, Ordering::Release);
        Ok(())
    }

    async fn create_answer(&self) -> Result<String, ClientError> {
        Ok(FAKE_ANSWER_SDP.to_string())
    }

    async fn set_local_answer(&self, _sdp: &str) -> Result<(), ClientError> {
        Ok(())
    }

    fn local_dtls_fingerprint(&self) -> Option<String> {
        Some(FAKE_LOCAL_FINGERPRINT.to_string())
    }

    /// The injected-frame ledger (zero frames = an empty table, the same shape as a real receiver that never received a stream).
    fn receiver_stats(&self, track_id: &str) -> Vec<RTCStats> {
        let stats = self.inner.stats.lock().unwrap_or_else(|e| e.into_inner());
        if stats.frames == 0 {
            return Vec::new();
        }
        vec![RTCStats::InboundRtp(RTCInboundRtpStreamStats {
            id: format!("fake-inbound-{track_id}"),
            timestamp: 0.0,
            ssrc: 1,
            kind: "video".into(),
            packets_received: u64::from(stats.frames),
            packets_lost: 0,
            bytes_received: stats.bytes,
            frames_decoded: stats.frames,
            frame_width: stats.width,
            frame_height: stats.height,
            frames_per_second: 30.0,
            jitter: 0.0,
            total_decode_time: 0.0,
            frame_dropped: 0,
            nack_count: 0,
            pli_count: 0,
            fir_count: 0,
        })]
    }

    async fn create_data_channel(
        &self,
        label: &str,
        init: RTCDataChannelInit,
    ) -> Result<Arc<dyn DcHandle>, ClientError> {
        let (id, auto_open) = {
            let mut st = self.lock_st();
            // the negotiated form keeps the input id (the server-assigned stream_id semantics);
            // non-negotiated = a libwebrtc real-config incrementing id.
            let id = if init.negotiated { init.id } else { st.next_dc_id };
            if !init.negotiated {
                st.next_dc_id += 1;
            }
            (id, st.dc_auto_open)
        };
        let (tx, _rx) = broadcast::channel(DC_EVENT_CAP);
        let dc = Arc::new(FakeDc {
            label: label.to_string(),
            id,
            open: AtomicBool::new(auto_open),
            closed: AtomicBool::new(false),
            buffered: AtomicUsize::new(0),
            tx,
            st: self.st.clone(),
        });
        if auto_open {
            dc.push_open();
        }
        self.inner.dcs.lock().unwrap_or_else(|e| e.into_inner()).push(dc.clone());
        Ok(dc)
    }
}

/// The sink slot of an injected track (deliver_target of inject_frame).
#[derive(Default)]
struct FakeTrack {
    sink: Mutex<Option<Box<dyn FrameSink>>>,
}

impl FakeTrack {
    fn deliver(&self, data: &[u8], width: u32, height: u32) {
        let guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sink) = guard.as_ref() {
            sink.on_frame(data, width, height);
        }
    }
}

impl TrackHandle for FakeTrack {
    fn attach(&self, sink: Box<dyn FrameSink>) {
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }
}

struct FakeDc {
    label: String,
    id: i32,
    open: AtomicBool,
    /// K5 drill slot: after close, state reports Closed (the single open=false flag can't distinguish not-yet-open from closed).
    closed: AtomicBool,
    /// K5 drill slot: the outbound queue ledger (send accumulates what was actually delivered; set_dc_buffered overwrites).
    buffered: AtomicUsize,
    tx: broadcast::Sender<EngineDcEvent>,
    st: Arc<Mutex<St>>,
}

impl FakeDc {
    fn label_inner(&self) -> &str {
        &self.label
    }

    fn set_open(&self) {
        if !self.open.swap(true, Ordering::AcqRel) {
            self.push_open();
        }
    }

    fn push_open(&self) {
        // no subscribers = events() never called -- Open semantics carried by the later send-state check, safe to drop.
        let _ = self.tx.send(EngineDcEvent::Open);
    }

    fn push_message(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(EngineDcEvent::Message(bytes));
    }

    /// Synchronously close the core (shared by async close and FakeEngine::close_dc -- flag + event, no await).
    fn close_inner(&self) {
        self.open.store(false, Ordering::Release);
        self.closed.store(true, Ordering::Release);
        let _ = self.tx.send(EngineDcEvent::Closed);
    }
}

#[async_trait]
impl DcHandle for FakeDc {
    fn label(&self) -> &str {
        &self.label
    }

    fn id(&self) -> i32 {
        self.id
    }

    /// The three-state mapping: open -> Open; closed before -> Closed; otherwise (auto_open=false, never pushed) -> Connecting.
    fn state(&self) -> RTCDataChannelState {
        if self.open.load(Ordering::Acquire) {
            RTCDataChannelState::Open
        } else if self.closed.load(Ordering::Acquire) {
            RTCDataChannelState::Closed
        } else {
            RTCDataChannelState::Connecting
        }
    }

    async fn buffered_amount(&self) -> u64 {
        self.buffered.load(Ordering::Acquire) as u64
    }

    async fn send_text(&self, text: &str) -> Result<(), ClientError> {
        if !self.open.load(Ordering::Acquire) {
            return Err(ClientError::WebRtc(format!("DC {} send: not open (fake)", self.label)));
        }
        let mut st = self.st.lock().unwrap_or_else(|e| e.into_inner());
        if st.drop_next_send {
            st.drop_next_send = false;
            tracing::warn!(label = %self.label, "fake: drop_dc_mid_send hit -- send returned Ok but never reached the peer");
            return Ok(());
        }
        st.sent.entry(self.label.clone()).or_default().push(text.to_string());
        self.buffered.fetch_add(text.len(), Ordering::AcqRel);
        Ok(())
    }

    async fn events(&self) -> mpsc::UnboundedReceiver<EngineDcEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut brx = self.tx.subscribe();
        tokio::spawn(async move {
            loop {
                match brx.recv().await {
                    Ok(ev) => {
                        if tx.send(ev).is_err() {
                            break; // the subscriber dropped
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        rx
    }

    async fn close(&self) {
        self.close_inner();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's own semantics pin (bypassing the session layer): one-shot flag consumption + the DC state gate + bookkeeping.
    #[tokio::test]
    async fn fault_knobs_are_one_shot() {
        let eng = FakeEngine::new();
        eng.fail_next_connect(1);
        assert!(matches!(eng.create_pc().await, Err(ClientError::WebRtc(_))));
        assert!(eng.create_pc().await.is_ok(), "an injection should be consumed in one shot");
    }

    #[tokio::test]
    async fn dc_send_requires_open_and_drop_mid_send_skips_ledger() {
        let eng = FakeEngine::new();
        eng.set_dc_auto_open(false);
        let pc = eng.create_pc().await.unwrap();
        let dc = pc.create_data_channel("chassis", RTCDataChannelInit::default()).await.unwrap();
        assert!(
            matches!(dc.send_text("x").await, Err(ClientError::WebRtc(m)) if m.contains("not open")),
            "a non-open DC must reject sends"
        );
        eng.open_data_channels();
        eng.drop_dc_mid_send();
        assert!(
            dc.send_text("dropped").await.is_ok(),
            "the drop injection still returns Ok (a fake delivery)"
        );
        assert!(eng.sent_texts("chassis").is_empty());
        assert!(dc.send_text("real").await.is_ok());
        assert_eq!(eng.sent_texts("chassis"), ["real"]);
    }

    #[tokio::test]
    async fn resume_injection_hits_only_re_negotiation() {
        let eng = FakeEngine::new();
        eng.fail_next_resume(1);
        let pc = eng.create_pc().await.unwrap();
        // the first negotiation is unaffected by the resume flag (resume = renegotiation semantics on an existing pc).
        assert!(pc.set_remote_offer("offer1".into()).await.is_ok());
        assert!(
            matches!(pc.set_remote_offer("offer2".into()).await, Err(ClientError::WebRtc(_))),
            "the second set_remote should be hit by the resume injection"
        );
        assert!(
            pc.set_remote_offer("offer3".into()).await.is_ok(),
            "the injection is consumed in one shot"
        );
    }
}
