//! 数据流监控核心（Task E2，D-H4 数据面维度）。
//!
//! 观测机制（选择理由见 task-E2-report）：iceoryx2 0.9.3 注册表动态详情
//! （[`ServiceDynamicDetails`]）仅暴露节点存活列表（`nodes`，无消息计数器，
//! 源码 iceoryx2-0.9.3 `src/service/mod.rs:378`）→ 零消费监控不可行。
//! 采用订阅式观测：monitor 对每个 `camera/<id>`（host.yaml 声明）attach 一个
//! latest-slot 订阅者（buffer=1 覆盖语义，`FrameBus::subscribe`）——iceoryx2
//! 每订阅者独立拷贝，不从 streamer/recorder 抢帧；慢消费自动跳帧（统计可接受）。
//!
//! 帧率/字节率: 窗口内帧数 + `FrameMeta.ts_mono_ns` 增量（发布端时钟，C17 单调）。
//! 停滞: 距最近到达 > `max(stall_floor, 2×期望帧间隔)`（期望 fps 来自 host.yaml
//! camera_configs）。无 grace —— 停滞是数据面事实（D-H14 的 grace 仅用于拓扑
//! 启动窗口 vs 故障区分，不适用于数据面）。
//!
//! streamer 推流状态: streamer（additive）每 2s 向 `stats/stream-<id>` 发布
//! [`StreamerStats`] JSON（FrameMeta::FORMAT_JSON 标记）→ monitor 订阅解析。
//! `connected` = 最近一条 stats 在 [`STATS_FRESHNESS`] 内（3× streamer 2s 间隔）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mediaservo_link::{CapabilityToken, Ed25519VerifyingKey, FrameBus, FrameStream, FrameTopic};

use crate::translate;

/// 默认停滞阈值下限（E2 约定: max(2s, 2×期望帧间隔)；fps≥0.5 时 2s 生效）。
pub const DEFAULT_STALL_FLOOR: Duration = Duration::from_secs(2);
/// 推流状态新鲜窗口（3× streamer 2s 发布间隔，容忍一次漏发）。
pub const STATS_FRESHNESS: Duration = Duration::from_secs(6);

/// 单 topic 数据流统计（E2 快照条目，E3 上报 Server 用）。
#[derive(Debug, Clone, PartialEq)]
pub struct TopicFlow {
    /// 相机 topic 名（`camera/<id>`）。
    pub topic: String,
    /// 窗口内帧率（ts_mono 增量推导；<2 帧或窗口为零 → 0）。
    pub fps: f64,
    /// 窗口内字节率（payload 字节 / ts_mono 窗口；无窗口 → 0）。
    pub bps: u64,
    /// 最近一帧的发布端单调时间戳（ns；从未收到 → 0）。
    pub last_ts_mono_ns: u64,
    /// 窗口内收到的帧数。
    pub frames: u64,
    /// 停滞（距最近到达超过阈值；从未收到帧也视为停滞——数据面事实）。
    pub stalled: bool,
}

/// 单流推流状态（E2 快照条目）。
#[derive(Debug, Clone, PartialEq)]
pub struct StreamFlow {
    /// 流 id（host.yaml streams[].id）。
    pub id: String,
    /// 最近一次 stats 的 bytes_sent（webrtc OutboundRtp，累计）。
    pub bytes_sent: u64,
    /// 最近一次 stats 的 frames_encoded（libwebrtc u32，累计）。
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    /// 最近 stats 是否在新鲜窗口内。
    pub connected: bool,
    /// 协商编解码器（host.yaml streams[].codec）。
    pub codec: String,
    /// 平均编码耗时 ms（web stats 面板——EncoderStatus 转发）。
    pub avg_encode_ms: Option<f64>,
    /// 实际编码器实现（软编/硬编——EncoderStatus 转发）。
    pub encoder_implementation: Option<String>,
}

/// 数据流快照（E2 数据面；与 E1 [`crate::monitor::topology::TopologySnapshot`] 并行）。
#[derive(Debug, Clone, Default)]
pub struct FlowSnapshot {
    pub topics: Vec<TopicFlow>,
    pub streams: Vec<StreamFlow>,
}

/// streamer 推流状态线格式（`stats/stream-<id>` JSON payload）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StreamerStats {
    pub bytes_sent: u64,
    /// 与 libwebrtc RTCOutboundRtpStreamStats 对齐（u32）。
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    /// 协商编解码器（config.codec——h264/vp8）。
    #[serde(default)]
    pub codec: String,
    /// 平均编码耗时 ms（增量 ΔtotalEncodeTime/ΔframesEncoded——web stats 面板）。
    #[serde(default)]
    pub avg_encode_ms: Option<f64>,
    /// 实际编码器实现（软编/硬编识别——libwebrtc get_stats）。
    #[serde(default)]
    pub encoder_implementation: Option<String>,
}

/// 单 topic 采集窗口状态（drain 任务写入；collect 读取并重置窗口）。
#[derive(Default)]
struct TopicState {
    frames: u64,
    bytes: u64,
    first_ts: Option<u64>,
    last_ts: Option<u64>,
    /// 最近一帧到达时刻（墙钟，monitor 侧；跨窗口保留——停滞是绝对事实）。
    last_arrival: Option<Instant>,
}

/// 单流 stats 状态（drain 任务写入；collect 读取，保留最近值）。
#[derive(Default)]
struct StreamState {
    last: Option<StreamerStats>,
    last_stats: Option<Instant>,
}

/// 数据流监控器：host.yaml 声明 topic 的订阅 + 统计（E2）。
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
    /// 默认停滞下限（2s）attach。
    pub fn attach(
        host_toml: String,
        token: &CapabilityToken,
        verifying_key: &Ed25519VerifyingKey,
    ) -> Result<Self, String> {
        Self::attach_with_stall(host_toml, token, verifying_key, DEFAULT_STALL_FLOOR)
    }

    /// 显式停滞下限 attach（测试注入短窗口）。
    pub fn attach_with_stall(
        host_toml: String,
        token: &CapabilityToken,
        verifying_key: &Ed25519VerifyingKey,
        stall_floor: Duration,
    ) -> Result<Self, String> {
        let bus = FrameBus::attach("", token, verifying_key).map_err(|e| e.to_string())?;
        let sources = translate::camera_configs(&host_toml).unwrap_or_else(|e| {
            tracing::warn!("host.yaml 视频源解析失败: {e}");
            Vec::new()
        });
        let streams = translate::stream_configs(&host_toml).unwrap_or_else(|e| {
            tracing::warn!("host.yaml 流解析失败: {e}");
            Vec::new()
        });

        let mut topics = HashMap::new();
        let mut stream_states = HashMap::new();
        let mut tasks = Vec::new();
        for src in sources {
            let topic = FrameTopic::new(format!("camera/{}", src.id));
            // 停滞阈值 = max(stall_floor, 2×期望帧间隔)（fps 已知时）
            let threshold = stall_threshold(stall_floor, src.fps);
            match bus.subscribe(&topic) {
                Ok(stream) => {
                    let state = Arc::new(Mutex::new(TopicState::default()));
                    topics.insert(topic.as_str().to_string(), (Arc::clone(&state), threshold));
                    tasks.push(spawn_frame_drain(stream, state));
                }
                Err(e) => tracing::error!("订阅 {} 失败: {e}", topic.as_str()),
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
                Err(e) => tracing::error!("订阅 {} 失败: {e}", topic.as_str()),
            }
        }
        Ok(Self { topics, streams: stream_states, tasks, _bus: bus })
    }

    /// 采集一次数据流快照：窗口统计（重置帧窗口）+ 停滞/连接判定。
    pub fn collect(&self) -> FlowSnapshot {
        let mut out = FlowSnapshot::default();
        for (name, (state, threshold)) in &self.topics {
            let mut st = state.lock().expect("topic state lock");
            let window_secs = match (st.first_ts, st.last_ts) {
                (Some(f), Some(l)) if l > f => (l - f) as f64 / 1e9,
                _ => 0.0,
            };
            let (fps, bps) = if st.frames >= 2 && window_secs > 0.0 {
                (
                    (st.frames - 1) as f64 / window_secs,
                    (st.bytes as f64 / window_secs) as u64,
                )
            } else {
                (0.0, 0)
            };
            let stalled = st
                .last_arrival
                .is_none_or(|t| t.elapsed() > *threshold);
            out.topics.push(TopicFlow {
                topic: name.clone(),
                fps,
                bps,
                last_ts_mono_ns: st.last_ts.unwrap_or(0),
                frames: st.frames,
                stalled,
            });
            // 重置帧窗口（last_arrival 保留——停滞是绝对事实，不随窗口重置）
            st.frames = 0;
            st.bytes = 0;
            st.first_ts = None;
            st.last_ts = None;
        }
        for (id, state) in &self.streams {
            let st = state.lock().expect("stream state lock");
            let (bytes_sent, frames_encoded, frame_width, frame_height, connected) = match &st.last {
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
                encoder_implementation: st.last
                    .as_ref()
                    .and_then(|s| s.encoder_implementation.clone()),
            });
        }
        out
    }
}

/// 停滞阈值: max(floor, 2×期望帧间隔)。fps=0 防御（translate 已拒绝，双保险）。
fn stall_threshold(floor: Duration, fps: u32) -> Duration {
    if fps == 0 {
        return floor;
    }
    floor.max(Duration::from_secs_f64(2.0 / f64::from(fps)))
}

/// 帧 drain 任务：latest-slot 到达 → 窗口状态更新（计数/字节/ts/到达时刻）。
fn spawn_frame_drain(stream: FrameStream, state: Arc<Mutex<TopicState>>) -> tokio::task::JoinHandle<()> {
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

/// stats drain 任务：JSON 解析失败打日志（C15，不静默）。
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
                Err(e) => tracing::warn!("stats payload 解析失败: {e}"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_threshold_uses_floor_at_high_fps() {
        // 30fps: 2×33ms < 2s → floor
        assert_eq!(stall_threshold(Duration::from_secs(2), 30), Duration::from_secs(2));
    }

    #[test]
    fn stall_threshold_scales_for_low_fps() {
        // 0.25fps: 2×4s = 8s > 2s floor
        assert_eq!(stall_threshold(Duration::from_secs(2), 1), Duration::from_secs(2));
        assert_eq!(
            stall_threshold(Duration::from_secs(2), 1),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn stall_threshold_zero_fps_falls_back_to_floor() {
        assert_eq!(
            stall_threshold(Duration::from_millis(300), 0),
            Duration::from_millis(300)
        );
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
