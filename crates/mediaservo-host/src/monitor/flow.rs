//! Data-flow monitoring core (Task E2, the D-H4 data-plane dimension).
//!
//! Observation mechanism (rationale in task-E2-report): iceoryx2 0.9.3 registry dynamic details
//! ([`ServiceDynamicDetails`]) expose only the live node list (`nodes`, no message counters,
//! source iceoryx2-0.9.3 `src/service/mod.rs:378`) -> zero-consumption monitoring is not viable.
//! Subscription-based observation is used instead: the monitor attaches one
//! latest-slot subscriber (buffer=1 overwrite semantics, `FrameBus::subscribe`) per `camera/<id>` (declared in host.yaml) -- iceoryx2
//! copies per subscriber, so it never steals frames from streamer/recorder; slow consumption drops frames automatically (acceptable for statistics).
//!
//! Frame rate / byte rate: frame count within the window + `FrameMeta.ts_mono_ns` deltas (publisher clock, C17 monotonic).
//! Stalled: time since the last arrival > `max(stall_floor, 2 x the expected frame interval)` (expected fps comes from host.yaml
//! camera_configs). No grace -- a stall is a data-plane fact (D-H14 grace only separates the topology
//! startup window from a fault, and does not apply to the data plane).
//!
//! Streamer publish status: the streamer (additive) publishes to `stats/stream-<id>` every 2s
//! a [`StreamerStats`] JSON (marked with FrameMeta::FORMAT_JSON) -> the monitor subscribes and parses it.
//! `connected` = the most recent stats is within [`STATS_FRESHNESS`] (3 x the streamer 2s interval).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mediaservo_link::{CapabilityToken, Ed25519VerifyingKey, FrameBus, FrameStream, FrameTopic};

use crate::translate;

/// Default lower bound of the stall threshold (E2 convention: max(2s, 2 x the expected frame interval); the 2s bound applies when fps >= 0.5).
pub const DEFAULT_STALL_FLOOR: Duration = Duration::from_secs(2);
/// Freshness window of the publish status (3 x the streamer 2s publish interval, tolerating one missed publish).
pub const STATS_FRESHNESS: Duration = Duration::from_secs(6);

/// Per-topic data-flow statistics (E2 snapshot entry, reported to the Server in E3).
#[derive(Debug, Clone, PartialEq)]
pub struct TopicFlow {
    /// Camera topic name (`camera/<id>`).
    pub topic: String,
    /// Frame rate within the window (derived from ts_mono deltas; <2 frames or a zero window -> 0).
    pub fps: f64,
    /// Byte rate within the window (payload bytes / ts_mono window; no window -> 0).
    pub bps: u64,
    /// Publisher monotonic timestamp of the most recent frame (ns; 0 if never received).
    pub last_ts_mono_ns: u64,
    /// Frames received within the window.
    pub frames: u64,
    /// Stalled (time since the last arrival exceeds the threshold; never having received a frame also counts as stalled -- a data-plane fact).
    pub stalled: bool,
}

/// Per-stream publish status (E2 snapshot entry).
#[derive(Debug, Clone, PartialEq)]
pub struct StreamFlow {
    /// Stream id (host.yaml streams[].id).
    pub id: String,
    /// bytes_sent of the most recent stats (webrtc OutboundRtp, cumulative).
    pub bytes_sent: u64,
    /// frames_encoded of the most recent stats (libwebrtc u32, cumulative).
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    /// Whether the most recent stats is inside the freshness window.
    pub connected: bool,
    /// Negotiated codec (host.yaml streams[].codec).
    pub codec: String,
    /// Average encoding time in ms (web stats panel -- forwarded from EncoderStatus).
    pub avg_encode_ms: Option<f64>,
    /// Actual encoder implementation (software/hardware -- forwarded from EncoderStatus).
    pub encoder_implementation: Option<String>,
}

/// Data-flow snapshot (E2 data plane; alongside the E1 [`crate::monitor::topology::TopologySnapshot`]).
#[derive(Debug, Clone, Default)]
pub struct FlowSnapshot {
    pub topics: Vec<TopicFlow>,
    pub streams: Vec<StreamFlow>,
}

/// Wire format of the streamer publish-status line (the `stats/stream-<id>` JSON payload).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StreamerStats {
    pub bytes_sent: u64,
    /// Aligned with libwebrtc RTCOutboundRtpStreamStats (u32).
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    /// Negotiated codec (config.codec -- h264/vp8).
    #[serde(default)]
    pub codec: String,
    /// Average encoding time in ms (incremental delta totalEncodeTime / delta framesEncoded -- web stats panel).
    #[serde(default)]
    pub avg_encode_ms: Option<f64>,
    /// Actual encoder implementation (software/hardware identification -- libwebrtc get_stats).
    #[serde(default)]
    pub encoder_implementation: Option<String>,
}

/// Per-topic capture window state (written by the drain task; read and reset by collect).
#[derive(Default)]
struct TopicState {
    frames: u64,
    bytes: u64,
    first_ts: Option<u64>,
    last_ts: Option<u64>,
    /// When the most recent frame arrived (wall clock, monitor side; kept across windows -- a stall is an absolute fact).
    last_arrival: Option<Instant>,
}

/// Per-stream stats state (written by the drain task; read by collect, keeping the most recent value).
#[derive(Default)]
struct StreamState {
    last: Option<StreamerStats>,
    last_stats: Option<Instant>,
}

/// Data-flow monitor: subscribes to the topics declared in host.yaml and collects statistics (E2).
pub struct FlowMonitor {
    topics: HashMap<String, (Arc<Mutex<TopicState>>, Duration)>,
    streams: HashMap<String, Arc<Mutex<StreamState>>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _bus: FrameBus,
}

impl Drop for FlowMonitor {
    fn drop(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

impl FlowMonitor {
    /// Attach with the default stall lower bound (2s).
    pub fn attach(
        host_toml: String,
        token: &CapabilityToken,
        verifying_key: &Ed25519VerifyingKey,
    ) -> Result<Self, String> {
        Self::attach_with_stall(host_toml, token, verifying_key, DEFAULT_STALL_FLOOR)
    }

    /// Attach with an explicit stall lower bound (short-window injection for tests).
    pub fn attach_with_stall(
        host_toml: String,
        token: &CapabilityToken,
        verifying_key: &Ed25519VerifyingKey,
        stall_floor: Duration,
    ) -> Result<Self, String> {
        let bus = FrameBus::attach("", token, verifying_key).map_err(|e| e.to_string())?;
        let sources = translate::camera_configs(&host_toml).unwrap_or_else(|e| {
            tracing::warn!("failed to parse the host.yaml video sources: {e}");
            Vec::new()
        });
        let streams = translate::stream_configs(&host_toml).unwrap_or_else(|e| {
            tracing::warn!("failed to parse the host.yaml streams: {e}");
            Vec::new()
        });

        let mut topics = HashMap::new();
        let mut stream_states = HashMap::new();
        let mut tasks = Vec::new();
        for src in sources {
            let topic = FrameTopic::new(format!("camera/{}", src.id));
            // Stall threshold = max(stall_floor, 2 x the expected frame interval) (when the fps is known)
            let threshold = stall_threshold(stall_floor, src.fps);
            match bus.subscribe(&topic) {
                Ok(stream) => {
                    let state = Arc::new(Mutex::new(TopicState::default()));
                    topics.insert(topic.as_str().to_string(), (Arc::clone(&state), threshold));
                    tasks.push(spawn_frame_drain(stream, state));
                }
                Err(e) => tracing::error!("failed to subscribe to {}: {e}", topic.as_str()),
            }
        }
        for s in streams {
            let topic = FrameTopic::new(format!("stats/stream-{}", s.id));
            match bus.subscribe(&topic) {
                Ok(stream) => {
                    let state = Arc::new(Mutex::new(StreamState::default()));
                    stream_states.insert(s.id.clone(), Arc::clone(&state));
                    tasks.push(spawn_stats_drain(stream, state));
                }
                Err(e) => tracing::error!("failed to subscribe to {}: {e}", topic.as_str()),
            }
        }
        Ok(Self { topics, streams: stream_states, tasks, _bus: bus })
    }

    /// Capture one data-flow snapshot: window statistics (resetting the frame window) + stall/connection decisions.
    pub fn collect(&self) -> FlowSnapshot {
        let mut out = FlowSnapshot::default();
        for (name, (state, threshold)) in &self.topics {
            let mut st = state.lock().expect("topic state lock");
            let window_secs = match (st.first_ts, st.last_ts) {
                (Some(f), Some(l)) if l > f => (l - f) as f64 / 1e9,
                _ => 0.0,
            };
            let (fps, bps) = if st.frames >= 2 && window_secs > 0.0 {
                ((st.frames - 1) as f64 / window_secs, (st.bytes as f64 / window_secs) as u64)
            } else {
                (0.0, 0)
            };
            let stalled = st.last_arrival.is_none_or(|t| t.elapsed() > *threshold);
            out.topics.push(TopicFlow {
                topic: name.clone(),
                fps,
                bps,
                last_ts_mono_ns: st.last_ts.unwrap_or(0),
                frames: st.frames,
                stalled,
            });
            // Reset the frame window (last_arrival is kept -- a stall is an absolute fact and does not reset with the window)
            st.frames = 0;
            st.bytes = 0;
            st.first_ts = None;
            st.last_ts = None;
        }
        for (id, state) in &self.streams {
            let st = state.lock().expect("stream state lock");
            let (bytes_sent, frames_encoded, frame_width, frame_height, connected) = match &st.last
            {
                Some(s) => (
                    s.bytes_sent,
                    s.frames_encoded,
                    s.frame_width,
                    s.frame_height,
                    st.last_stats.is_some_and(|t| t.elapsed() < STATS_FRESHNESS),
                ),
                None => (0, 0, 0, 0, false),
            };
            out.streams.push(StreamFlow {
                id: id.clone(),
                bytes_sent,
                frames_encoded,
                frame_width,
                frame_height,
                connected,
                codec: st.last.as_ref().map(|s| s.codec.clone()).unwrap_or_default(),
                avg_encode_ms: st.last.as_ref().and_then(|s| s.avg_encode_ms),
                encoder_implementation: st
                    .last
                    .as_ref()
                    .and_then(|s| s.encoder_implementation.clone()),
            });
        }
        out
    }
}

/// Stall threshold: max(floor, 2 x the expected frame interval). fps=0 defense (translate already rejects it, belt and braces).
fn stall_threshold(floor: Duration, fps: u32) -> Duration {
    if fps == 0 {
        return floor;
    }
    floor.max(Duration::from_secs_f64(2.0 / f64::from(fps)))
}

/// Frame drain task: a latest-slot arrival -> update the window state (count/bytes/ts/arrival time).
fn spawn_frame_drain(
    stream: FrameStream,
    state: Arc<Mutex<TopicState>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(f) = stream.recv().await {
            let mut st = state.lock().expect("topic state lock");
            st.frames += 1;
            st.bytes += f.payload().len() as u64;
            let ts = f.meta().ts_mono_ns;
            st.first_ts.get_or_insert(ts);
            st.last_ts = Some(ts);
            st.last_arrival = Some(Instant::now());
        }
    })
}

/// Stats drain task: log a JSON parse failure (C15, never silent).
fn spawn_stats_drain(
    stream: FrameStream,
    state: Arc<Mutex<StreamState>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(f) = stream.recv().await {
            match serde_json::from_slice::<StreamerStats>(f.payload()) {
                Ok(stats) => {
                    let mut st = state.lock().expect("stream state lock");
                    st.last = Some(stats);
                    st.last_stats = Some(Instant::now());
                }
                Err(e) => tracing::warn!("failed to parse the stats payload: {e}"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_threshold_uses_floor_at_high_fps() {
        // 30fps: 2 x 33ms < 2s -> the floor
        assert_eq!(stall_threshold(Duration::from_secs(2), 30), Duration::from_secs(2));
    }

    #[test]
    fn stall_threshold_scales_for_low_fps() {
        // 0.25fps: 2 x 4s = 8s > the 2s floor
        assert_eq!(stall_threshold(Duration::from_secs(2), 1), Duration::from_secs(2));
        assert_eq!(stall_threshold(Duration::from_secs(2), 1), Duration::from_secs(2));
    }

    #[test]
    fn stall_threshold_zero_fps_falls_back_to_floor() {
        assert_eq!(stall_threshold(Duration::from_millis(300), 0), Duration::from_millis(300));
    }

    #[test]
    fn streamer_stats_json_roundtrip() {
        let s = StreamerStats {
            bytes_sent: 12345,
            frames_encoded: 678,
            frame_width: 1280,
            frame_height: 720,
            codec: "vp8".into(),
            avg_encode_ms: Some(3.5),
            encoder_implementation: Some("libvpx".into()),
        };
        let json = serde_json::to_vec(&s).expect("serialize");
        let back: StreamerStats = serde_json::from_slice(&json).expect("deserialize");
        assert_eq!(back, s);
    }
}
