//! Signaling-state monitoring + status reporting core (Task E3, the D-H4 signaling-plane dimension + reporting to the Server).
//!
//! The signaling-plane data source = the gateway (`gateway::GatewayHandle::snapshot`, the sole
//! holder of the connection state): local child-process WS connections (most recent message time) + the remote server WS state
//! (joined/since/peer_id). This module collects ([`SignalMonitor`]) and aggregates the report
//! ([`build_status_report`] -> `SignalingMessage::StatusReport`)).
//!
//! Reporting path: the reporter task holds the gateway handle and `send_remote` sends straight over the remote WS (the joined
//! check is at the source, C15: failures are logged, never silent). The report is a periodic idempotent message -- a dropped report inside a disconnect window
//! or a send failure both self-heal in the next cycle.

use mediaservo_common::protocol::{
    ChildSignalJson, ConfigFlagsEcho, ProcessStateJson, SignalStatusJson, SignalingMessage,
    StreamFlowJson, TopicFlowJson,
};
use mediaservo_link::{CapabilityToken, Ed25519VerifyingKey};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config_core::{Ledger, file_hash, short12};
use crate::gateway::{ChildStatus, GatewayHandle, GatewayStatus};
use crate::monitor::flow::{FlowMonitor, FlowSnapshot, StreamFlow, TopicFlow};
use crate::monitor::topology::{DEFAULT_GRACE, Mismatch, TopologyMonitor, TopologySnapshot};
use std::path::{Path, PathBuf};

/// Reporting interval (5s, the same tick as the monitoring collection; the report is <2KB JSON, so the WS traffic is negligible.
/// The 10s option that halves the traffic was rejected -- decoupling from the collection buys nothing and only adds state-window drift).
pub const STATUS_INTERVAL: Duration = Duration::from_secs(5);
/// Signaling-plane snapshot (E3; data source = the gateway connection state + the agent uptime).
#[derive(Debug, Clone)]
pub struct SignalSnapshot {
    pub gateway: GatewayStatus,
    /// Seconds since the host-agent started.
    pub agent_uptime_secs: u64,
}

/// Signaling-state monitor: collects from the gateway snapshot (E3).
pub struct SignalMonitor {
    gateway: GatewayHandle,
    started: Instant,
}

impl SignalMonitor {
    /// `started` = the agent process start time (E1 review: consistent with the topology grace start point).
    pub fn new(gateway: GatewayHandle, started: Instant) -> Self {
        Self { gateway, started }
    }

    /// Capture one signaling-plane snapshot.
    pub fn collect(&self) -> SignalSnapshot {
        SignalSnapshot {
            gateway: self.gateway.snapshot(),
            agent_uptime_secs: self.started.elapsed().as_secs(),
        }
    }
}

/// Status reporting task (E3): a single loop holding the three monitors, collecting each tick + logging (E1/E2
/// behavior migration) + aggregated reporting. The reporting cadence is set by `interval` (5s in production).
///
/// Report content: the topology + data-flow + signaling snapshots + ts + config_version (E4 association).
/// C15: a send failure is logged as warn, never dropped silently (periodic report, self-heals next cycle).
pub fn spawn_status_reporter(
    host_toml: String,
    started: Instant,
    token: Option<(CapabilityToken, Ed25519VerifyingKey)>,
    gateway: GatewayHandle,
    interval: Duration,
    // E4: the config version (updated by the agent after a ConfigPush apply; data source of StatusReport.config_version).
    config_version: Arc<AtomicU64>,
    // T1b3 (k11): instance dir for ledger-epoch polling + telemetry re-attach (None = pinned monitors).
    dir: Option<PathBuf>,
) {
    tokio::spawn(async move {
        let mut topology = TopologyMonitor::new_at(host_toml.clone(), DEFAULT_GRACE, started);
        let mut flow = match &token {
            Some((tok, vk)) => match FlowMonitor::attach(host_toml, tok, vk) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::error!(
                        "failed to attach the data-flow monitor: {e} -- skipping (topology/signaling monitoring is unaffected)"
                    );
                    None
                }
            },
            None => None,
        };
        let signal = SignalMonitor::new(gateway.clone(), started);
        let room = gateway.vehicle_room();
        // Data source of the web stats panel: streamer encoding info (the extra StreamerStats fields) -> the EncoderStatus signaling
        // report (server relay broadcast -> the browser sfu-client emitMetrics -- the old host EncoderStatus used the same path)
        let _last_enc: std::collections::HashMap<String, (f64, u64)> =
            std::collections::HashMap::new();
        let mut tick = tokio::time::interval(interval);
        // T1b3 (k11): last observed committed config epoch (running.hash) from the ledger.
        let mut last_epoch: Option<String> = None;
        // consume the first immediate tick (aligned with the E1/E2 behavior)
        loop {
            tick.tick().await;
            // T1b3 (k11): ledger-driven telemetry re-attach. The first observation only
            // records the epoch (startup monitors are fresh by definition); a later flip
            // means the core rebuilt the cluster -> drop the stale monitors, re-read
            // host.yaml, re-attach against the new expected state.
            if let Some(d) = &dir {
                let epoch = Ledger::load(d).running.map(|r| r.hash);
                if let Some(h) = epoch_bump(&last_epoch, &epoch) {
                    reattach_monitors(d, &token, &mut topology, &mut flow, h);
                }
                last_epoch = epoch; // absent/corrupt ledger reads as None, never panics
            }
            let topo = topology.collect();
            // E1 logging (behavior identical to E1: counting + suppressing mismatch warnings inside grace)
            tracing::info!(
                expected = topo.expected_processes.len(),
                actual_procs = topo.actual_processes.len(),
                actual_topics = topo.actual_topics.len(),
                mismatches = topo.mismatches.len(),
                grace = topo.grace_active,
                "topology snapshot"
            );
            if !topo.grace_active {
                for m in &topo.mismatches {
                    match m {
                        Mismatch::ProcessMissing { name } => {
                            tracing::warn!(process = %name, "topology mismatch: the expected process is missing/not running");
                        }
                        Mismatch::PublisherMissing { topic } => {
                            tracing::warn!(topic = %topic, "topology mismatch: the expected camera has no active publisher");
                        }
                    }
                }
            }
            let flow_snap = match &flow {
                Some(m) => {
                    let snap = m.collect();
                    for tf in &snap.topics {
                        tracing::info!(
                            topic = %tf.topic,
                            fps = tf.fps,
                            bps = tf.bps,
                            frames = tf.frames,
                            stalled = tf.stalled,
                            "data-flow topic statistics"
                        );
                    }
                    for sf in &snap.streams {
                        tracing::info!(
                            stream = %sf.id,
                            bytes_sent = sf.bytes_sent,
                            frames_encoded = sf.frames_encoded,
                            connected = sf.connected,
                            "publish status"
                        );
                    }
                    snap
                }
                None => FlowSnapshot::default(),
            };
            let sig = signal.collect();
            tracing::info!(
                children = sig.gateway.children.len(),
                remote_connected = sig.gateway.remote.connected,
                "signaling snapshot"
            );
            // T1b6 (k9): fresh disk read each tick (<1ms on a 5s cadence) — the echo must
            // reflect the on-disk truth of THIS window, never a cached memo.
            let echo = match &dir {
                Some(d) => ConfigEcho::collect(d),
                None => ConfigEcho::default(),
            };
            let report = build_status_report(
                &room,
                &topo,
                &flow_snap,
                &sig,
                config_version.load(Ordering::Relaxed),
                &echo,
            );
            if let Err(e) = gateway.send_remote(report) {
                tracing::warn!("failed to send the StatusReport: {e}"); // C15
            }
            // Encoding-info report: the encoding fields of each stream -> EncoderStatus (the browser web stats panel)
            for sf in flow_snap.streams.iter() {
                if sf.avg_encode_ms.is_none() && sf.encoder_implementation.is_none() {
                    continue; // no encoding info (old fields / not collected), skip
                }
                // avg_encode_ms is already computed incrementally by the streamer (delta totalEncodeTime / delta framesEncoded), passed through as is
                let avg = sf.avg_encode_ms;
                let msg = mediaservo_common::protocol::SignalingMessage::EncoderStatus {
                    room_id: room.clone(),
                    peer_id: sf.id.clone(),
                    codec: sf.codec.clone(),
                    encoder_backend: "auto".into(),
                    encoder_implementation: sf.encoder_implementation.clone(),
                    frames_per_second: 0.0,
                    frame_width: sf.frame_width,
                    frame_height: sf.frame_height,
                    avg_encode_ms: avg,
                };
                if let Err(e) = gateway.send_remote(msg) {
                    tracing::warn!("failed to send the EncoderStatus: {e}"); // C15
                }
            }
        }
    });
}

/// T1b3 (k11): pure epoch verdict — re-attach only when both observations are present and
/// differ; returns the new epoch label for the re-attach log. `None` (first observation,
/// absent/corrupt ledger, or quiet) = record-only.
fn epoch_bump<'a>(prev: &Option<String>, next: &'a Option<String>) -> Option<&'a str> {
    if prev.is_some() && next.as_deref().is_some_and(|h| prev.as_deref() != Some(h)) {
        return next.as_deref();
    }
    None
}

/// T1b3 (k11): rebuild the two config-sensitive monitors against the current on-disk
/// host.yaml (grace window restarts with the fresh topology; the old FlowMonitor's Drop
/// detaches its bus subscriptions + drain tasks). Unreadable host.yaml keeps the current
/// monitors (warn, C15) — repair rides the next epoch flip or a restart (deliberately simple).
fn reattach_monitors(
    dir: &Path,
    token: &Option<(CapabilityToken, Ed25519VerifyingKey)>,
    topology: &mut TopologyMonitor,
    flow: &mut Option<FlowMonitor>,
    epoch: &str,
) {
    let cfg_path = dir.join("etc").join("host.yaml");
    let Ok(host_toml) = std::fs::read_to_string(&cfg_path) else {
        tracing::warn!(path = %cfg_path.display(), "monitor re-attach: host.yaml unreadable -- keeping current monitors");
        return;
    };
    *topology = TopologyMonitor::new(host_toml.clone());
    *flow = match token {
        Some((tok, vk)) => match FlowMonitor::attach(host_toml, tok, vk) {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::error!(
                    "monitor re-attach: data-flow attach failed: {e} -- flow paused until the next epoch"
                );
                None
            }
        },
        None => None,
    };
    tracing::info!("telemetry monitors re-attached (config epoch {})", short12(epoch));
}

/// T1b6 (k9): the four config facts read from disk for the StatusReport echo.
/// Ledger absent => all None (pre-T1″ vehicle / never applied): the cloud shows GRAY,
/// never green (PC-2 presence gate; nothing here is version-gated).
#[derive(Debug, Default)]
struct ConfigEcho {
    running: Option<String>,
    file: Option<String>,
    last_push: Option<String>,
    flags: Option<ConfigFlagsEcho>,
}

impl ConfigEcho {
    /// Collect from the instance dir. `file` is hashed from the CURRENT host.yaml bytes
    /// on demand (§9.1: a memoized file hash is erasable); flags ride only a present ledger
    /// (a default ledger with no file would fabricate a "lamps all green" lie).
    fn collect(dir: &Path) -> Self {
        if !Ledger::path(dir).exists() {
            return Self::default();
        }
        let ledger = Ledger::load(dir);
        let file = std::fs::read(dir.join("etc").join("host.yaml"))
            .ok()
            .map(|bytes| short12(&file_hash(&bytes)).to_owned());
        Self {
            running: ledger.running.as_ref().map(|r| short12(&r.hash).to_owned()),
            file,
            last_push: ledger.last_push.as_ref().map(|p| short12(&p.hash).to_owned()),
            flags: Some(ConfigFlagsEcho {
                pending: ledger.flags.pending,
                error: ledger.flags.error,
                conflict: ledger.flags.conflict,
            }),
        }
    }
}

/// Aggregate the three snapshots -> the StatusReport signaling message (pure function, covered by the E3 unit tests).
fn build_status_report(
    room_id: &str,
    topology: &TopologySnapshot,
    flow: &FlowSnapshot,
    signal: &SignalSnapshot,
    config_version: u64,
    echo: &ConfigEcho,
) -> SignalingMessage {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    SignalingMessage::StatusReport {
        room_id: room_id.to_string(),
        topics: flow.topics.iter().map(topic_to_json).collect(),
        streams: flow.streams.iter().map(stream_to_json).collect(),
        processes: processes_to_json(topology),
        signal: signal_to_json(signal),
        ts,
        config_version,
        config_running: echo.running.clone(),
        config_file: echo.file.clone(),
        config_last_push: echo.last_push.clone(),
        config_flags: echo.flags.clone(),
    }
}

/// Topology snapshot -> the wire process list (union of expected and actual; running = oxmgr running).
fn processes_to_json(snap: &TopologySnapshot) -> Vec<ProcessStateJson> {
    let mut out: Vec<ProcessStateJson> = snap
        .expected_processes
        .iter()
        .map(|name| ProcessStateJson {
            name: name.clone(),
            running: snap.actual_processes.iter().any(|p| &p.name == name && p.status == "running"),
            expected: true,
        })
        .collect();
    for p in &snap.actual_processes {
        if !snap.expected_processes.iter().any(|n| n == &p.name) {
            out.push(ProcessStateJson {
                name: p.name.clone(),
                running: p.status == "running",
                expected: false,
            });
        }
    }
    out
}

fn topic_to_json(tf: &TopicFlow) -> TopicFlowJson {
    TopicFlowJson {
        topic: tf.topic.clone(),
        fps: tf.fps,
        bps: tf.bps,
        last_ts_mono_ns: tf.last_ts_mono_ns,
        frames: tf.frames,
        stalled: tf.stalled,
    }
}

fn stream_to_json(sf: &StreamFlow) -> StreamFlowJson {
    StreamFlowJson {
        id: sf.id.clone(),
        bytes_sent: sf.bytes_sent,
        frames_encoded: sf.frames_encoded,
        frame_width: sf.frame_width,
        frame_height: sf.frame_height,
        connected: sf.connected,
    }
}

fn signal_to_json(snap: &SignalSnapshot) -> SignalStatusJson {
    SignalStatusJson {
        remote_connected: snap.gateway.remote.connected,
        remote_since_secs: snap.gateway.remote.since_secs,
        remote_peer_id: snap.gateway.remote.peer_id.clone(),
        children: snap
            .gateway
            .children
            .iter()
            .map(|c: &ChildStatus| ChildSignalJson {
                src: c.src.clone(),
                connected: c.connected,
                last_msg_secs: c.last_msg_secs,
            })
            .collect(),
        agent_uptime_secs: snap.agent_uptime_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::RemoteStatus;
    use crate::monitor::topology::OxProcess;

    fn topo_snap() -> TopologySnapshot {
        TopologySnapshot {
            expected_processes: vec!["host-agent".into(), "host-capturer-cam0".into()],
            actual_processes: vec![
                OxProcess { name: "host-agent".into(), status: "running".into() },
                OxProcess { name: "stray-proc".into(), status: "running".into() },
            ],
            actual_topics: vec![],
            mismatches: vec![],
            grace_active: false,
        }
    }

    fn flow_snap() -> FlowSnapshot {
        FlowSnapshot {
            topics: vec![TopicFlow {
                topic: "camera/cam0".into(),
                fps: 29.0,
                bps: 1000,
                last_ts_mono_ns: 5,
                frames: 29,
                stalled: false,
            }],
            streams: vec![StreamFlow {
                id: "cam0".into(),
                bytes_sent: 999,
                frames_encoded: 42,
                frame_width: 640,
                frame_height: 360,
                connected: true,
                codec: "vp8".into(),
                avg_encode_ms: Some(4.2),
                encoder_implementation: Some("libvpx".into()),
            }],
        }
    }

    fn signal_snap() -> SignalSnapshot {
        SignalSnapshot {
            gateway: GatewayStatus {
                children: vec![ChildStatus {
                    src: "host-streamer".into(),
                    connected: true,
                    last_msg_secs: 2,
                }],
                remote: RemoteStatus {
                    connected: true,
                    since_secs: Some(10),
                    peer_id: "veh-peer".into(),
                },
            },
            agent_uptime_secs: 123,
        }
    }

    #[test]
    fn processes_to_json_union_expected_and_actual() {
        let procs = processes_to_json(&topo_snap());
        assert_eq!(procs.len(), 3);
        let agent = procs.iter().find(|p| p.name == "host-agent").unwrap();
        assert!(agent.running && agent.expected, "the expected process is running: {agent:?}");
        let cap = procs.iter().find(|p| p.name == "host-capturer-cam0").unwrap();
        assert!(!cap.running && cap.expected, "the expected process is missing: {cap:?}");
        let stray = procs.iter().find(|p| p.name == "stray-proc").unwrap();
        assert!(stray.running && !stray.expected, "an unexpected process: {stray:?}");
    }

    #[test]
    fn build_status_report_aggregates_three_snapshots() {
        let msg = build_status_report(
            "vehicle-1",
            &topo_snap(),
            &flow_snap(),
            &signal_snap(),
            0,
            &ConfigEcho::default(),
        );
        match msg {
            SignalingMessage::StatusReport {
                room_id,
                topics,
                streams,
                processes,
                signal,
                ts,
                config_version,
                config_running,
                config_file,
                config_last_push,
                config_flags,
            } => {
                assert_eq!(room_id, "vehicle-1");
                assert_eq!(topics[0].topic, "camera/cam0");
                assert_eq!(streams[0].frames_encoded, 42);
                assert!(
                    processes.iter().any(|p| p.name == "host-agent" && p.running && p.expected)
                );
                assert_eq!(signal.remote_peer_id, "veh-peer");
                assert_eq!(signal.children[0].src, "host-streamer");
                assert_eq!(signal.agent_uptime_secs, 123);
                assert!(ts > 0, "ts should be unix seconds");
                assert_eq!(config_version, 0);
                assert_eq!(config_running, None, "default echo = legacy shape");
                assert_eq!(config_file, None);
                assert_eq!(config_last_push, None);
                assert_eq!(config_flags, None);
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }

    #[test]
    fn empty_flow_snapshot_serializes_empty_sections() {
        let flow = FlowSnapshot::default();
        let msg = build_status_report(
            "r",
            &topo_snap(),
            &flow,
            &signal_snap(),
            0,
            &ConfigEcho::default(),
        );
        match msg {
            SignalingMessage::StatusReport { topics, streams, .. } => {
                assert!(topics.is_empty());
                assert!(streams.is_empty());
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }

    #[test]
    fn flow_stats_conversions_preserve_fields() {
        let snap = flow_snap();
        let msg = build_status_report(
            "r",
            &topo_snap(),
            &snap,
            &signal_snap(),
            3,
            &ConfigEcho::default(),
        );
        match msg {
            SignalingMessage::StatusReport { topics, streams, config_version, .. } => {
                assert_eq!(topics[0].fps, 29.0);
                assert!(!topics[0].stalled);
                assert_eq!(streams[0].bytes_sent, 999);
                assert_eq!(streams[0].frame_width, 640);
                assert_eq!(config_version, 3, "config_version is passed through");
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }

    #[test]
    fn epoch_bump_requires_two_present_differing_epochs() {
        let none: Option<String> = None;
        let a = || Some("a".to_string());
        let b = || Some("b".to_string());
        assert_eq!(epoch_bump(&none, &a()), None, "first observation records only");
        assert_eq!(epoch_bump(&a(), &none), None, "absent ledger records only");
        assert_eq!(epoch_bump(&a(), &a()), None, "same epoch is quiet");
        assert_eq!(epoch_bump(&a(), &b()), Some("b"), "epoch flip re-attaches");
    }

    #[test]
    fn config_echo_absent_ledger_is_all_none() {
        let dir = tempfile::tempdir().unwrap();
        let echo = ConfigEcho::collect(dir.path());
        assert_eq!(echo.running, None);
        assert_eq!(echo.file, None);
        assert_eq!(echo.last_push, None);
        assert_eq!(echo.flags, None, "no ledger file = no lamps (honest legacy shape)");
    }

    #[test]
    fn config_echo_collects_ledger_facts_and_file_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("run")).unwrap();
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        let yaml = b"sources: [];\n";
        std::fs::write(dir.path().join("etc").join("host.yaml"), yaml).unwrap();
        let full = file_hash(yaml); // 64 hex; the echo carries the short12 display form
        std::fs::write(
            Ledger::path(dir.path()),
            format!(
                "{{\"running\":{{\"hash\":\"{full}\",\"at\":\"t\",\"origin\":\"push\"}},\
\"last_push\":{{\"hash\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",\"at\":\"t\"}},\
\"flags\":{{\"pending\":true,\"error\":false,\"conflict\":true}}}}",
            ),
        )
        .unwrap();
        let echo = ConfigEcho::collect(dir.path());
        assert_eq!(echo.running.as_deref(), Some(short12(&full)));
        assert_eq!(echo.file.as_deref(), Some(short12(&full)));
        assert_eq!(echo.last_push.as_deref(), Some("0123456789ab"));
        assert_eq!(
            echo.flags,
            Some(ConfigFlagsEcho { pending: true, error: false, conflict: true })
        );

        let msg = build_status_report(
            "r",
            &topo_snap(),
            &FlowSnapshot::default(),
            &signal_snap(),
            0,
            &echo,
        );
        match msg {
            SignalingMessage::StatusReport {
                config_running,
                config_file,
                config_last_push,
                config_flags,
                ..
            } => {
                assert_eq!(config_running.as_deref(), Some(short12(&full)));
                assert_eq!(config_file.as_deref(), Some(short12(&full)));
                assert_eq!(config_last_push.as_deref(), Some("0123456789ab"));
                assert_eq!(
                    config_flags,
                    Some(ConfigFlagsEcho { pending: true, error: false, conflict: true })
                );
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }
}
