//! 信令状态监控 + 状态上报核心（Task E3，D-H4 信令面维度 + 上报 Server）。
//!
//! 信令平面数据源 = 网关（`gateway::GatewayHandle::snapshot`，连接状态唯一
//! 持有者）：本地子进程 WS 连接（src/最近消息时刻）+ 远端 server WS 状态
//! （joined/since/peer_id）。本模块负责采集（[`SignalMonitor`]）与聚合上报
//! （[`build_status_report`] → `SignalingMessage::StatusReport`）。
//!
//! 上报路径: reporter 任务持有网关句柄，`send_remote` 直发远端 WS（joined
//! 检查在源头，C15：失败打日志不静默）。上报为周期性幂等消息——断线窗口
//! 丢弃/发送失败均可由下一周期自愈。

use mediaservo_common::protocol::{
    ChildSignalJson, ProcessStateJson, SignalStatusJson, SignalingMessage, StreamFlowJson,
    TopicFlowJson,
};
use mediaservo_link::{CapabilityToken, Ed25519VerifyingKey};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::gateway::{ChildStatus, GatewayHandle, GatewayStatus};
use crate::monitor::flow::{FlowMonitor, FlowSnapshot, StreamFlow, TopicFlow};
use crate::monitor::topology::{DEFAULT_GRACE, Mismatch, TopologyMonitor, TopologySnapshot};

/// 上报间隔（5s，与监控采集同 tick；报告 <2KB JSON，WS 流量可忽略。
/// 10s 减半流量的选项被否——与采集解耦无收益，仅增加状态窗口漂移）。
pub const STATUS_INTERVAL: Duration = Duration::from_secs(5);
/// 信令平面快照（E3；数据源 = 网关连接状态 + agent 运行时长）。
#[derive(Debug, Clone)]
pub struct SignalSnapshot {
    pub gateway: GatewayStatus,
    /// host-agent 启动至今秒数。
    pub agent_uptime_secs: u64,
}

/// 信令状态监控器：从网关快照采集（E3）。
pub struct SignalMonitor {
    gateway: GatewayHandle,
    started: Instant,
}

impl SignalMonitor {
    /// `started` = agent 进程启动时刻（E1 审查: 与拓扑 grace 起点一致）。
    pub fn new(gateway: GatewayHandle, started: Instant) -> Self {
        Self { gateway, started }
    }

    /// 采集一次信令平面快照。
    pub fn collect(&self) -> SignalSnapshot {
        SignalSnapshot {
            gateway: self.gateway.snapshot(),
            agent_uptime_secs: self.started.elapsed().as_secs(),
        }
    }
}

/// 状态上报任务（E3）：单循环持有三监控器，每 tick 采集 + 日志（E1/E2
/// 行为迁移）+ 聚合上报。上报节奏由 `interval` 决定（生产 5s）。
///
/// 上报内容: 拓扑 + 数据流 + 信令三快照 + ts + config_version（E4 关联）。
/// C15: 发送失败打 warn 日志，不静默丢弃（周期性上报，下一周期自愈）。
pub fn spawn_status_reporter(
    host_toml: String,
    started: Instant,
    token: Option<(CapabilityToken, Ed25519VerifyingKey)>,
    gateway: GatewayHandle,
    interval: Duration,
    // E4: 配置版本（ConfigPush 应用后由 agent 更新；StatusReport.config_version 数据源）。
    config_version: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let topology = TopologyMonitor::new_at(host_toml.clone(), DEFAULT_GRACE, started);
        let flow = match token {
            Some((tok, vk)) => match FlowMonitor::attach(host_toml, &tok, &vk) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::error!("数据流监控 attach 失败: {e} — 跳过（拓扑/信令监控不受影响）");
                    None
                }
            },
            None => None,
        };
        let signal = SignalMonitor::new(gateway.clone(), started);
        let room = gateway.vehicle_room();
        // web stats 面板数据源: streamer 编码信息（StreamerStats 扩展字段）→ EncoderStatus 信令
        // 上报（server relay 广播 → 浏览器 sfu-client emitMetrics——旧 host EncoderStatus 同链路）
        let _last_enc: std::collections::HashMap<String, (f64, u64)> =
            std::collections::HashMap::new();
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // 消费首个立即 tick（对齐 E1/E2 行为）
        loop {
            tick.tick().await;
            let topo = topology.collect();
            // E1 日志（行为与 E1 一致：计数 + grace 内抑制 mismatch 告警）
            tracing::info!(
                expected = topo.expected_processes.len(),
                actual_procs = topo.actual_processes.len(),
                actual_topics = topo.actual_topics.len(),
                mismatches = topo.mismatches.len(),
                grace = topo.grace_active,
                "拓扑快照"
            );
            if !topo.grace_active {
                for m in &topo.mismatches {
                    match m {
                        Mismatch::ProcessMissing { name } => {
                            tracing::warn!(process = %name, "拓扑差异: 期望进程缺失/未运行");
                        }
                        Mismatch::PublisherMissing { topic } => {
                            tracing::warn!(topic = %topic, "拓扑差异: 期望相机无活跃发布者");
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
                            "数据流 topic 统计"
                        );
                    }
                    for sf in &snap.streams {
                        tracing::info!(
                            stream = %sf.id,
                            bytes_sent = sf.bytes_sent,
                            frames_encoded = sf.frames_encoded,
                            connected = sf.connected,
                            "推流状态"
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
                "信令快照"
            );
            let report = build_status_report(
                &room,
                &topo,
                &flow_snap,
                &sig,
                config_version.load(Ordering::Relaxed),
            );
            if let Err(e) = gateway.send_remote(report) {
                tracing::warn!("StatusReport 发送失败: {e}"); // C15
            }
            // 编码信息上报: 每路 stream 的编码字段 → EncoderStatus（浏览器 web stats 面板）
            for sf in flow_snap.streams.iter() {
                if sf.avg_encode_ms.is_none() && sf.encoder_implementation.is_none() {
                    continue; // 无编码信息（旧字段/未采集）跳过
                }
                // avg_encode_ms 已由 streamer 增量计算（ΔtotalEncodeTime/ΔframesEncoded），直接透传
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
                    tracing::warn!("EncoderStatus 发送失败: {e}"); // C15
                }
            }
        }
    });
}

/// 聚合三快照 → StatusReport 信令消息（纯函数，E3 单测覆盖）。
fn build_status_report(
    room_id: &str,
    topology: &TopologySnapshot,
    flow: &FlowSnapshot,
    signal: &SignalSnapshot,
    config_version: u64,
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
    }
}

/// 拓扑快照 → wire 进程列表（期望 + 实际并集；running = oxmgr running）。
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
        assert!(agent.running && agent.expected, "期望进程在跑: {agent:?}");
        let cap = procs.iter().find(|p| p.name == "host-capturer-cam0").unwrap();
        assert!(!cap.running && cap.expected, "期望进程缺失: {cap:?}");
        let stray = procs.iter().find(|p| p.name == "stray-proc").unwrap();
        assert!(stray.running && !stray.expected, "非期望进程: {stray:?}");
    }

    #[test]
    fn build_status_report_aggregates_three_snapshots() {
        let msg = build_status_report("vehicle-1", &topo_snap(), &flow_snap(), &signal_snap(), 0);
        match msg {
            SignalingMessage::StatusReport {
                room_id,
                topics,
                streams,
                processes,
                signal,
                ts,
                config_version,
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
                assert!(ts > 0, "ts 应为 unix 秒");
                assert_eq!(config_version, 0);
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }

    #[test]
    fn empty_flow_snapshot_serializes_empty_sections() {
        let flow = FlowSnapshot::default();
        let msg = build_status_report("r", &topo_snap(), &flow, &signal_snap(), 0);
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
        let msg = build_status_report("r", &topo_snap(), &snap, &signal_snap(), 3);
        match msg {
            SignalingMessage::StatusReport { topics, streams, config_version, .. } => {
                assert_eq!(topics[0].fps, 29.0);
                assert!(!topics[0].stalled);
                assert_eq!(streams[0].bytes_sent, 999);
                assert_eq!(streams[0].frame_width, 640);
                assert_eq!(config_version, 3, "config_version 透传");
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }
}
