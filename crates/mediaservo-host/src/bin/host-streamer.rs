//! host-streamer: 推流进程（Task C2）— FrameBus 订阅 → WebRTC 推流。
//!
//! 用法: `host-streamer --stream <id> --config <host.yaml 路径> --token <令牌文件路径>`
//!
//! 流程: 读 host.yaml `streams`（source/codec 缺省 id/vp8，
//! [`mediaservo_host::translate::stream_config`]）→ 源配置（fps）→ FrameBus
//! 订阅 `camera/<camera-id>`（FrameMeta + 紧凑 I420，C1 capturer 线格式）→
//! field `PushSession`（connect → publish_video：SFU transport + answer 协商 +
//! Connect + Produce，复用 field 推流链路）→ `TrackSender::write_raw_i420_with_ts`
//! 写帧（时间戳来自 FrameMeta.ts_mono_ns，C17 透传）。P2P 模式：信令直连 Server
//! （Phase D 网关前，MUST NOT 引入总线信令）。
//!
//! 信令目标 = 本地网关 host-agent（D2）：`--gateway <url>`（缺省
//! `ws://127.0.0.1:17980/ws`）。网关本地侧无 PSK 挑战（信任边界
//! 127.0.0.1）；整车 PSK 在 host-agent 的远端连接。房间 = `stream-<id>`
//! （网关拦袪 RoomJoin 并重写为整车房间，多流集合一车会话）。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_field::{PublishOptions, PushConfig, PushSession, SessionEvent};
use mediaservo_host::monitor::flow::StreamerStats;
use mediaservo_link::{FrameBus, FrameMeta, FrameRef, FrameStream, FrameTopic, SignalClient, SignalEvent, SignalSession, TokenFile};
use mediaservo_webrtc::data_channel::{RTCDataChannel, RTCDataChannelInit, RTCDataChannelState};
use mediaservo_webrtc::peer_connection::{RTCConfiguration, RTCIceCandidate};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::stats::RTCStats;
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{RTCPeerConnection, RTCPeerConnectionFactory};
use tokio::sync::{broadcast, mpsc};

/// FrameMeta 像素格式: 1 = I420（D243 枚举，与 C1 capturer 一致）。
const FORMAT_I420: u8 = 1;
/// 无帧看门狗：capturer 未启动/已退出时 10s 无帧即退出（C15，失败可见非挂起；
/// 部署侧 restart_policy=always 拉起，对齐 PIT-87 ICE-failed 自愈模式）。
const NO_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// 出站统计日志间隔（e2e 证据 + 可观测性）。
const STATS_INTERVAL: Duration = Duration::from_secs(2);
/// 视觉 DC label（D-H8 契约：舱端 HMI overlay 按此 label 接收）。
const VISION_DC_LABEL: &str = "vision";
/// 视觉 DC 协商截止：无舱端 answer 则降级（视觉是可选 overlay，视频不受影响）。
const VISION_NEGOTIATE_TIMEOUT: Duration = Duration::from_secs(10);

const USAGE: &str = "用法: host-streamer --stream <id> --config <host.yaml> --token <令牌文件>";

/// 出站统计消息序号（stats topic FrameMeta.seq，单调）。
static STATS_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct Args {
    stream: String,
    config: PathBuf,
    token: PathBuf,
    /// 本地网关 WS 地址（D2）；缺省 `ws://127.0.0.1:17980/ws`。
    gateway: Option<String>,
    /// 编码器后端（auto/software/hardware/nvenc/vaapi；缺省 auto）。
    encoder_backend: String,
    /// 编码码率 kbps（None=field 默认 2000）。
    bitrate_kbps: Option<u32>,
    /// 关键帧间隔秒 GOP（None=field 默认 2）。
    keyframe_interval: Option<u32>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut stream: Option<String> = None;
    let mut config: Option<PathBuf> = None;
    let mut token: Option<PathBuf> = None;
    let mut gateway: Option<String> = None;
    let mut encoder_backend: Option<String> = None;
    let mut bitrate_kbps: Option<u32> = None;
    let mut keyframe_interval: Option<u32> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => stream = Some(args.next().ok_or("--stream 缺值")?),
            "--config" => config = Some(PathBuf::from(args.next().ok_or("--config 缺值")?)),
            "--token" => token = Some(PathBuf::from(args.next().ok_or("--token 缺值")?)),
            "--gateway" => gateway = Some(args.next().ok_or("--gateway 缺值")?),
            "--encoder-backend" => encoder_backend = Some(args.next().ok_or("--encoder-backend 缺值")?),
            "--bitrate-kbps" => bitrate_kbps = Some(args.next().ok_or("--bitrate-kbps 缺值")?.parse().map_err(|_| "--bitrate-kbps 须为数字")?),
            "--keyframe-interval" => keyframe_interval = Some(args.next().ok_or("--keyframe-interval 缺值")?.parse().map_err(|_| "--keyframe-interval 须为数字")?),
            _ => return Err(format!("未知参数: {arg}")),
        }
    }
    Ok(Args {
        stream: stream.ok_or("缺少 --stream")?,
        config: config.ok_or("缺少 --config")?,
        token: token.ok_or("缺少 --token")?,
        gateway,
        encoder_backend: encoder_backend.unwrap_or_else(|| "auto".into()),
        bitrate_kbps,
        keyframe_interval,
    })
}
/// 网关 WS 地址（D2）：`--gateway` 参数 > 缺省本地网关。
fn gateway_url(gateway_arg: Option<&str>) -> String {
    gateway_arg
        .map(str::to_string)
        .unwrap_or_else(|| "ws://127.0.0.1:17980/ws".to_string())
}
/// 视觉 topic（bridge.rs B3 约定: `vision/<camera-id>` 镜像相机 id）。
fn vision_topic(camera_id: &str) -> String {
    format!("vision/{camera_id}")
}

/// 视觉消息格式门禁：仅 JSON 载荷（FrameMeta::FORMAT_JSON，E2 stats 同款线格式）。
/// vision topic 上出现像素载荷 = 发布端协议违反，拒绝转发。
fn vision_meta_ok(meta: &FrameMeta) -> bool {
    meta.format == FrameMeta::FORMAT_JSON
}

/// 透明转发（D-H8 消息格式决策）：payload 原样作为 DC 文本（streamer = pipe，
/// HMI 解析，不重编码——帧关联 ts_mono/seq 由 ROS 视觉节点写入 payload JSON，
/// 它与它消费的 camera 帧 meta 对齐）。非 UTF-8 payload = 协议违反，拒绝转发。
fn vision_payload_text(payload: &[u8]) -> Option<&str> {
    std::str::from_utf8(payload).ok()
}

/// 视觉 transport B 协商状态（D-H8：独立 PC 纯 DC 无 track，与视频 transport A 分离；
/// F1/F2 offerer 模式——经本地网关 relay 到舱端）。
struct VisionNegotiation {
    signal: SignalSession,
    events: broadcast::Receiver<SignalEvent>,
    ice_rx: mpsc::UnboundedReceiver<RTCIceCandidate>,
    pc: RTCPeerConnection,
    dc: RTCDataChannel,
    /// answer 落地前缓存的远端候选（libwebrtc 协商前 add_ice_candidate 不可靠）。
    pending_ice: Vec<RTCIceCandidate>,
    remote_set: bool,
    /// 协商截止：answer 未到即降级禁用（视觉可选，视频不受影响）。
    deadline: tokio::time::Instant,
}

/// 视觉异步事件（transport B 全部异步源聚合）。
enum VisionEvent {
    Signal(Result<SignalEvent, broadcast::error::RecvError>),
    Ice(Option<RTCIceCandidate>),
    Frame(Option<FrameRef>),
    /// 协商截止到期（无舱端 answer）。
    Deadline,
}

/// 建立 transport B（信令 + PC + DC "vision" + offer）。任何一步失败 → None
/// （降级为纯视频；C15 每分支打日志）。
async fn setup_vision_dc(gateway: &str, src: &str, room: &str) -> Option<VisionNegotiation> {
    let signal = match SignalClient::new_gateway(gateway, src, room, PeerRole::Host)
        .connect()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("vision 信令连接失败（视觉转发禁用）: {e}");
            return None;
        }
    };
    let events = signal.events();
    let factory = RTCPeerConnectionFactory::new();
    let pc = match factory.create_peer_connection(RTCConfiguration::default()).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("vision create_peer_connection 失败: {e}");
            return None;
        }
    };
    let (ice_tx, ice_rx) = mpsc::unbounded_channel::<RTCIceCandidate>();
    pc.on_ice_candidate(move |candidate| {
        let _ = ice_tx.send(candidate);
    });
    let dc = match pc
        .create_data_channel(VISION_DC_LABEL, RTCDataChannelInit::default())
        .await
    {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("vision create_data_channel 失败: {e}");
            return None;
        }
    };
    let offer = match pc.create_offer(&Default::default()).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("vision create_offer 失败: {e}");
            return None;
        }
    };
    if let Err(e) = pc.set_local_description(&offer).await {
        tracing::warn!("vision set_local_description 失败: {e}");
        return None;
    }
    let offer_json = match serde_json::to_string(&offer) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("vision 序列化 offer 失败: {e}");
            return None;
        }
    };
    if let Err(e) = signal
        .send(SignalingMessage::Sdp {
            room_id: room.into(),
            target: None,
            sdp: offer_json,
        })
        .await
    {
        tracing::warn!("vision 发送 offer 失败: {e}");
        return None;
    }
    tracing::info!("vision offer 已发送（transport B，等待舱端 answer）");
    Some(VisionNegotiation {
        signal,
        events,
        ice_rx,
        pc,
        dc,
        pending_ice: Vec::new(),
        remote_set: false,
        deadline: tokio::time::Instant::now() + VISION_NEGOTIATE_TIMEOUT,
    })
}

/// 聚合 transport B 异步源（vision=None/vision_frames=None → 永久挂起，即视觉禁用）。
async fn next_vision_event(
    vision: &mut Option<VisionNegotiation>,
    vision_frames: &Option<FrameStream>,
) -> VisionEvent {
    match (vision.as_mut(), vision_frames.as_ref()) {
        (Some(v), Some(frames)) => tokio::select! {
            ev = v.events.recv() => VisionEvent::Signal(ev),
            c = v.ice_rx.recv() => VisionEvent::Ice(c),
            f = frames.recv() => VisionEvent::Frame(f),
            _ = tokio::time::sleep_until(v.deadline) => VisionEvent::Deadline,
        },
        _ => std::future::pending::<VisionEvent>().await,
    }
}

/// 处理 vision 信令事件（answer/ICE/断开）。返回 true = 继续，false = 降级禁用。
async fn handle_vision_signal(
    v: &mut VisionNegotiation,
    ev: Result<SignalEvent, broadcast::error::RecvError>,
) -> bool {
    match ev {
        Ok(SignalEvent::Message(SignalingMessage::Sdp { sdp, .. })) => {
            let desc = match serde_json::from_str::<RTCSessionDescription>(&sdp) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("vision answer 解析失败: {e}");
                    return false;
                }
            };
            if desc.sdp_type != RTCSdpType::Answer {
                tracing::warn!(sdp_type = %desc.sdp_type, "vision 非 answer Sdp，降级");
                return false;
            }
            match v.pc.set_remote_description(&desc).await {
                Ok(()) => {
                    tracing::info!("vision answer 已设置 — 协商完成");
                    v.remote_set = true;
                    // 协商完成：取消截止（慢 ICE/慢首帧不再触发降级）
                    v.deadline = tokio::time::Instant::now() + Duration::from_secs(3600);
                    for c in v.pending_ice.drain(..) {
                        if let Err(e) = v.pc.add_ice_candidate(&c).await {
                            tracing::warn!("vision add_ice_candidate: {e}");
                        }
                    }
                    true
                }
                Err(e) => {
                    tracing::warn!("vision set_remote_description 失败: {e}");
                    false
                }
            }
        }
        Ok(SignalEvent::Message(SignalingMessage::RTCIceCandidate {
            candidate, sdp_mid, sdp_mline_index, ..
        })) => {
            let c = RTCIceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
            };
            if v.remote_set {
                if let Err(e) = v.pc.add_ice_candidate(&c).await {
                    tracing::warn!("vision add_ice_candidate: {e}");
                }
            } else {
                v.pending_ice.push(c);
            }
            true
        }
        Ok(SignalEvent::Message(_)) => true, // RoomJoined/其他透传忽略
        Ok(SignalEvent::Error(e)) => {
            tracing::warn!("vision 信令错误: {e} — 降级禁用");
            false
        }
        Ok(SignalEvent::Disconnected { reason }) => {
            tracing::warn!("vision 信令断开: {reason} — 降级禁用");
            false
        }
        Ok(SignalEvent::Connected { .. }) => true,
        Ok(_) => true, // SignalEvent non_exhaustive
        Err(broadcast::error::RecvError::Lagged(n)) => {
            tracing::warn!("vision 信令事件滞后 {n} 条");
            true
        }
        Err(broadcast::error::RecvError::Closed) => {
            tracing::warn!("vision 信令事件流关闭 — 降级禁用");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_encoder_status, gateway_url, upstream_unavailable, vision_meta_ok, vision_payload_text, vision_topic};
    use mediaservo_link::FrameMeta;

    #[test]
    fn upstream_unavailable_matches_gateway_5001_only() {
        // 实测错误串（test6 日志原样）
        assert!(upstream_unavailable(
            "link: signal error: room join failed [5001]: gateway not connected to server",
        ));
        assert!(!upstream_unavailable("link: signal error: connect refused"));
        assert!(!upstream_unavailable("auth failed [4001]"));
    }

    #[test]
    fn gateway_url_defaults_to_local_gateway() {
        assert_eq!(gateway_url(None), "ws://127.0.0.1:17980/ws");
    }

    #[test]
    fn gateway_url_override_wins() {
        assert_eq!(
            gateway_url(Some("ws://127.0.0.1:18888/ws")),
            "ws://127.0.0.1:18888/ws"
        );
    }

    #[test]
    fn vision_topic_mirrors_camera_id() {
        // bridge.rs B3 约定: vision/<camera-id> 镜像相机 id（ROS 桥接配置单一来源）
        assert_eq!(vision_topic("cam0"), "vision/cam0");
        assert_eq!(vision_topic("front/raw"), "vision/front/raw");
    }

    #[test]
    fn vision_meta_ok_accepts_only_json_format() {
        let json = FrameMeta { format: FrameMeta::FORMAT_JSON, ..Default::default() };
        assert!(vision_meta_ok(&json), "JSON 载荷必须放行");
        let i420 = FrameMeta { format: super::FORMAT_I420, ..Default::default() };
        assert!(!vision_meta_ok(&i420), "像素载荷不得当视觉 JSON 转发");
    }

    #[test]
    fn vision_payload_text_is_transparent() {
        // D-H8 决策: 透明转发（streamer = pipe，HMI 解析，不重编码）
        let payload = br#"{"frame":{"seq":1,"ts_mono_ns":2},"objects":[]}"#;
        assert_eq!(
            vision_payload_text(payload),
            Some(r#"{"frame":{"seq":1,"ts_mono_ns":2},"objects":[]}"#)
        );
        assert_eq!(vision_payload_text(&[0xff, 0xfe]), None, "非 UTF-8 载荷拒绝（协议违反）");
    }

    #[test]
    fn encoder_status_wire_matches_web_contract() {
        // 字段名与 sfu-client.ts msg.* 消费面逐字对齐（web-stream-stats 断链正在于无人发送）。
        let v = serde_json::to_value(build_encoder_status(
            "vehicle_test", "host-1", "h264", "software",
            Some("OpenH264".into()), 30.0, 1280, 720, Some(3.1),
        ))
        .unwrap();
        assert_eq!(v["type"], "encoder_status");
        assert_eq!(v["room_id"], "vehicle_test");
        assert_eq!(v["codec"], "video/H264");
        assert_eq!(v["encoder_backend"], "software");
        assert_eq!(v["encoder_implementation"], "OpenH264");
        assert_eq!(v["frames_per_second"], 30.0);
        assert_eq!(v["frame_width"], 1280);
        assert_eq!(v["avg_encode_ms"], 3.1);
        let v2 = serde_json::to_value(build_encoder_status(
            "r", "p", "vp8", "auto", None, 0.0, 0, 0, None,
        ))
        .unwrap();
        assert!(v2.get("encoder_implementation").is_none() && v2.get("avg_encode_ms").is_none());
        assert_eq!(v2["codec"], "video/VP8");
    }
}

/// 紧凑 I420 payload 校验（线格式假设: tight strides Y + U + V）。
fn valid_i420(meta: &FrameMeta, payload_len: usize) -> bool {
    meta.format == FORMAT_I420
        && meta.width.is_multiple_of(2)
        && meta.height.is_multiple_of(2)
        && payload_len == (meta.width * meta.height * 3 / 2) as usize
}

/// 等待 SIGINT/SIGTERM（unix 主路径；其他平台仅 ctrl_c）。
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// 出站统计：日志（e2e 证据: bytes_sent/frames_encoded > 0，对齐 field D4 模式）
/// + FrameBus 发布 [`StreamerStats`] JSON 到 `stats/stream-<id>`（E2 数据面监控，
/// additive；监控订阅者才消费，无消费者时发布零开销级）。
/// 编码耗时增量基线（ΔtotalEncodeTime/ΔframesEncoded——web stats 面板 avg_encode_ms）
static LAST_ENCODE: std::sync::Mutex<Option<(f64, u64)>> = std::sync::Mutex::new(None);

/// codec 标识 → W3C mimeType（web 面板 `codec.replace('video/','')` 显示约定）。
fn codec_mime(codec: &str) -> String {
    format!("video/{}", codec.to_uppercase())
}

/// 组 EncoderStatus（v2 协议消息——server relay / web 合并两侧早已就位，host 发送端
/// 系 E3 多进程拆分迁移时丢失的旧 host 行为，此处补齐；wire shape 有单测钉住）。
#[allow(clippy::too_many_arguments)]
fn build_encoder_status(
    room_id: &str,
    peer_id: &str,
    codec: &str,
    backend: &str,
    encoder_implementation: Option<String>,
    frames_per_second: f64,
    frame_width: u32,
    frame_height: u32,
    avg_encode_ms: Option<f64>,
) -> SignalingMessage {
    SignalingMessage::EncoderStatus {
        room_id: room_id.into(),
        peer_id: peer_id.into(),
        codec: codec_mime(codec),
        encoder_backend: backend.into(),
        encoder_implementation,
        frames_per_second,
        frame_width,
        frame_height,
        avg_encode_ms,
    }
}

/// 信令发送连续失败计数（≥SIGNAL_FAIL_MAX 判定僵尸会话，'run 退出重建；
/// 独立于 H6 5001 通知的兑底通道——通知目标表项可能已随上游断开被清理）。
/// 单流单进程形态，文件级 static 与 LAST_ENCODE 同模式。
static SIGNAL_FAIL_STREAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const SIGNAL_FAIL_MAX: u32 = 3;
/// 上游不可达签名（gateway 未接入 server：room join 被 5001 拒 / 未连接提示）。
fn upstream_unavailable(err: &str) -> bool {
    err.contains("[5001]") || err.contains("gateway not connected")
}

async fn log_stats(session: &PushSession, bus: &FrameBus, topic: &FrameTopic, started: Instant, codec: &str, backend: &str) {
    let Some(pc) = session.peer_connection() else {
        return;
    };
    let stats = pc.sender_get_stats("video");
    let Some(o) = stats.iter().find_map(|s| match s {
        RTCStats::OutboundRtp(o) => Some(o),
        _ => None,
    }) else {
        return;
    };
    // 编码耗时增量（旧 host EncoderStatus 同款——ΔtotalEncodeTime/ΔframesEncoded）
    let avg_encode_ms = {
        let mut guard = LAST_ENCODE.lock().unwrap_or_else(|e| e.into_inner());
        let now = (o.total_encode_time.unwrap_or(0.0), o.frames_encoded as u64);
        let avg = match *guard {
            Some((t0, f0)) if now.1 > f0 && now.0 >= t0 => {
                Some((now.0 - t0) * 1000.0 / (now.1 - f0) as f64)
            }
            _ => None,
        };
        *guard = Some(now);
        avg
    };
    tracing::info!(
        "streamer stats: bytes_sent={} frames_encoded={} frame={}x{} codec={} avg_encode_ms={:?} enc={:?}",
        o.bytes_sent,
        o.frames_encoded,
        o.frame_width,
        o.frame_height,
        codec,
        avg_encode_ms,
        o.encoder_implementation
    );
    // E2 additive: 发布推流状态（FrameMeta::FORMAT_JSON 标记；ts_mono = 进程启动
    // 单调时钟（C17 锚定语义；监控侧不消费 stats ts_mono，仅作为单调标记））
    let meta = FrameMeta {
        seq: STATS_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        format: FrameMeta::FORMAT_JSON,
        ts_mono_ns: started.elapsed().as_nanos() as u64,
        ..Default::default()
    };
    let payload = match serde_json::to_vec(&StreamerStats {
        bytes_sent: o.bytes_sent,
        frames_encoded: o.frames_encoded,
        frame_width: o.frame_width,
        frame_height: o.frame_height,
        codec: codec.to_string(),
        avg_encode_ms,
        encoder_implementation: o.encoder_implementation.clone(),
    }) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("stats 序列化失败: {e}");
            return;
        }
    };
    if let Err(e) = bus.publish(topic, &payload, &meta) {
        tracing::warn!(topic = %topic.as_str(), "stats 发布失败: {e}");
    }
    // EncoderStatus 上报（room 广播 relay 给同房间浏览器消费者；C15 失败打日志）。
    let msg = build_encoder_status(
        session.signal().room_id(),
        session.signal().peer_id(),
        codec,
        backend,
        o.encoder_implementation.clone(),
        o.frames_per_second,
        o.frame_width,
        o.frame_height,
        avg_encode_ms,
    );
    if let Err(e) = session.signal().send(msg).await {
        let streak = SIGNAL_FAIL_STREAK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(streak, "encoder_status 发送失败: {e}");
    } else {
        SIGNAL_FAIL_STREAK.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("streamer");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    // 流配置（camera/codec 缺省 id/vp8）+ 相机配置（fps）
    let cfg_text = match std::fs::read_to_string(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("streamer: 读取配置 {} 失败: {e}", args.config.display());
            return ExitCode::from(1);
        }
    };
    let stream = match mediaservo_host::translate::stream_config(&cfg_text, &args.stream) {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("streamer: 配置中无流 {}", args.stream);
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("streamer: {e}");
            return ExitCode::from(1);
        }
    };
    let cam = match mediaservo_host::translate::camera_config(&cfg_text, &stream.source) {
        Ok(Some(c)) => c,
        Ok(None) => {
            eprintln!("streamer: 流 {} 引用的源 {} 不存在", stream.id, stream.source);
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("streamer: {e}");
            return ExitCode::from(1);
        }
    };
    // 房间约定（PIT-140 v2 + multi-stream P3）: <整车房间>_<流 id> ——每流独立房间，
    // 前端按 device_stream 勾选/播放（与 deleteRoom 同规则）；整车房间 = [signaling].room
    // 缺省 "vehicle"。PIT-140 v1 曾全部推整车房间导致多流无法按流区分，v2 统一 per-stream。
    let vehicle_room = mediaservo_host::translate::signaling_room(&cfg_text)
        .ok()
        .flatten()
        .unwrap_or_else(|| "vehicle".to_string());
    let room = format!("{}_{}", vehicle_room, stream.id);

    // 令牌 → FrameBus attach → 订阅 camera/<camera-id>
    let token_bytes = match std::fs::read(&args.token) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("streamer: 读取令牌 {} 失败: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let (verifying_key, token) = match TokenFile::decode(&token_bytes) {
        Ok(kv) => kv,
        Err(e) => {
            eprintln!("streamer: 令牌 {} 无效: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let bus = match FrameBus::attach("", &token, &verifying_key) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("streamer: FrameBus attach 失败: {e}");
            return ExitCode::from(1);
        }
    };
    let topic = FrameTopic::new(format!("camera/{}", cam.id));
    // D-H14 顺序无关 + SystemInFlux 残留容错: 订阅失败（capturer 后起 / SHM 残留瞬态）
    // 每 3s 重试最多 10 次（30s）——避免一次失败即退出被 oxmgr 重启到 errored（PIT）
    let frames = loop {
        match bus.subscribe(&topic) {
            Ok(f) => break f,
            Err(e) => {
                eprintln!(
                    "streamer: 订阅 {} 失败（3s 后重试——capturer 后起/SHM 残留容错）: {e}",
                    topic.as_str()
                );
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    };
    tracing::info!(topic = %topic.as_str(), "FrameBus subscribed");

    // F3 (D-H7/D-H8): 订阅视觉结果 vision/<camera-id>（ROS 视觉节点发布；
    // 外部节点 attach 走 Perception 角色 + 能力令牌，见 acl.rs）。失败降级（视觉可选）。
    let vision_topic = FrameTopic::new(vision_topic(&cam.id));
    let mut vision_frames: Option<FrameStream> = match bus.subscribe(&vision_topic) {
        Ok(f) => {
            tracing::info!(topic = %vision_topic.as_str(), "vision FrameBus subscribed");
            Some(f)
        }
        Err(e) => {
            tracing::warn!("订阅 {} 失败（视觉转发禁用）: {e}", vision_topic.as_str());
            None
        }
    };

    // 推流会话（field PushSession 复用；D2: 经本地网关，无 PSK）
    // 自报 src 品牌派生（D269/T2——display-only，gateway 快照/StatusReport 展示串，无匹配消费方；
    // 默认品牌 app_prefix="host-" → 与旧字面量逐字节一致，零行为变化）
    let app_prefix = mediaservo_common::brand::media_brand().app_prefix;
    let mut cfg = PushConfig::via_gateway(
        gateway_url(args.gateway.as_deref()),
        format!("{app_prefix}streamer-{}", stream.id),
        room.clone(),
    );
    cfg.framerate = cam.fps;
    // 编码参数透传（streams 配置面；None=field 默认 2000kbps/2s GOP）
    if let Some(k) = args.bitrate_kbps {
        cfg.bitrate_kbps = k;
    }
    if let Some(g) = args.keyframe_interval {
        cfg.keyframe_interval = g as u64;
    }
    // C 修：宕机窗口新进程 connect 直吃 5001 → 立即退出会以 1-2s 一轮触发
    // oxmgr 熔断（3次/5min）→ server 恢复后也无人再拉。进程内 10s 退避重试
    // （最长 ~6min），与 gateway remote_loop connect_with_retry 同约定。
    let (mut session, mut events) = 'connect: {
        let mut last_err = String::from("never attempted");
        for attempt in 1..=36u32 {
            match PushSession::connect(cfg.clone()).await {
                Ok(se) => break 'connect se,
                Err(e) if upstream_unavailable(&e.to_string()) => {
                    tracing::warn!(attempt, "上游未就绪（网关 5001），10s 后重试 connect: {e}");
                    last_err = e.to_string();
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
                Err(e) => {
                    eprintln!("streamer: PushSession connect 失败: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        eprintln!("streamer: 上游持续未就绪（~6min），退出待 oxmgr 重拉: {last_err}");
        return ExitCode::from(1);
    };

    // F3 (D-H8): transport B — 独立纯 DC PC（label "vision"），经同一本地网关
    // relay 到舱端（F1/F2 offerer 模式；mediasoup send/recv 分离 + SCTP/RTP 同 PC
    // 调度互拖 → 与视频 transport A 分离）。失败降级为纯视频（视觉可选 overlay）。
    let mut vision: Option<VisionNegotiation> = setup_vision_dc(
        &gateway_url(args.gateway.as_deref()),
        &format!("{app_prefix}streamer-{}-vision", stream.id),
        &room,
    )
    .await;
    if vision.is_none() {
        tracing::warn!("transport B 不可用 — 降级为纯视频推流");
    }


    // 首帧决定分辨率（capturer 固定 1280x720，按 meta 自适应更稳）→ publish
    let first = match tokio::time::timeout(NO_FRAME_TIMEOUT, frames.recv()).await {
        Ok(Some(f)) => f,
        Ok(None) => {
            eprintln!("streamer: 帧流关闭（capturer 未运行?）");
            return ExitCode::from(1);
        }
        Err(_) => {
            eprintln!("streamer: {NO_FRAME_TIMEOUT:?} 无帧 — capturer 未启动或已退出");
            return ExitCode::from(1);
        }
    };
    if !valid_i420(first.meta(), first.payload().len()) {
        eprintln!(
            "streamer: 首帧无效（format={} {}x{} payload={}）",
            first.meta().format,
            first.meta().width,
            first.meta().height,
            first.payload().len()
        );
        return ExitCode::from(1);
    }
    cfg.width = first.meta().width;
    cfg.height = first.meta().height;
    let opts = PublishOptions {
        codec: stream.codec.clone(),
        encoder_backend: args.encoder_backend.clone(),
    };
    if let Err(e) = session.publish_video(&cfg, &opts).await {
        eprintln!("streamer: publish_video 失败: {e}");
        return ExitCode::from(1);
    }
    let sender = match session.video_sender() {
        Some(s) => s,
        None => {
            eprintln!("streamer: publish 后无 video sender");
            return ExitCode::from(1);
        }
    };
    println!(
        "streamer ready: stream={} topic={} {}x{}@{} codec={} room={} vision={}",
        stream.id, topic.as_str(), cfg.width, cfg.height, cam.fps, stream.codec, cfg.room,
        if vision.is_some() && vision_frames.is_some() { "on (transport B)" } else { "off" }
    );

    // E2 additive: 推流状态 topic + 单调时钟起点（stats 发布用）
    let stats_topic = FrameTopic::new(format!("stats/stream-{}", stream.id));
    let started = Instant::now();
    let mut exit_code: u8 = 0;
    let mut last_stats = Instant::now();
    'run: loop {
        tokio::select! {
            sig = shutdown_signal() => match sig {
                Ok(()) => break 'run,
                Err(e) => {
                    eprintln!("streamer: 信号处理失败: {e}");
                    exit_code = 1;
                    break 'run;
                }
            },
            ev = events.recv() => match ev {
                Some(SessionEvent::Error(e)) => {
                    // H6: 上游切换（gateway 5001）— 本会话 transport/producer 已全死，
                    // 退出待 oxmgr 重启 → 全新 produce（与 Disconnected 同自愈通道）。
                    if e.to_string().contains("[5001]") {
                        // H6: server 宕机窗口的被动 5001——立即退出会 1-2s 一轮重启
                        // 触发 oxmgr 熔断；退避 15s 给 server 重启时间，再退出重建。
                        tracing::error!("上游切换（网关 5001）: {e} — 15s 退避后退出重 produce");
                        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                        break 'run;
                    }
                    tracing::warn!("session error: {e}");
                }
                Some(SessionEvent::Disconnected { reason }) => {
                    tracing::error!("signal disconnected: {reason}");
                    exit_code = 1;
                    break 'run;
                }
                None => {
                    tracing::error!("session event stream closed");
                    exit_code = 1;
                    break 'run;
                }
                _ => {} // Message/Connected/TrackPublished 忽略
            },
            frame = tokio::time::timeout(NO_FRAME_TIMEOUT, frames.recv()) => match frame {
                Ok(Some(f)) => {
                    let meta = f.meta();
                    if !valid_i420(meta, f.payload().len()) {
                        tracing::warn!(
                            seq = meta.seq,
                            "invalid frame (format={} {}x{} payload={})",
                            meta.format, meta.width, meta.height, f.payload().len()
                        );
                        continue;
                    }
                    // C17: 时间戳透传 — capturer 已锚定单调时钟（ts_mono_ns）
                    let ts_us = (meta.ts_mono_ns / 1000) as i64;
                    if let Err(e) = sender
                        .write_raw_i420_with_ts(
                            f.payload(),
                            meta.width,
                            meta.height,
                            Some(ts_us),
                        )
                        .await
                    {
                        tracing::warn!(seq = meta.seq, "write frame: {e}");
                    }
                    if last_stats.elapsed() >= STATS_INTERVAL {
                        log_stats(&session, &bus, &stats_topic, started, &stream.codec, &args.encoder_backend).await;
                        last_stats = Instant::now();
                        // 僵尸兑底：信令 WS 半死时 events 不会送达任何事件，
                        // 唯发送失败计数可见——连续 3 次（≈6s）即退出重 produce。
                        if SIGNAL_FAIL_STREAK.load(std::sync::atomic::Ordering::Relaxed) >= SIGNAL_FAIL_MAX {
                            tracing::error!("信令发送连续 {} 次失败 — 会话僵尸，退出重 produce", SIGNAL_FAIL_MAX);
                            exit_code = 1;
                            break 'run;
                        }
                    }
                }
                Ok(None) => {
                    tracing::error!("帧流关闭（capturer 退出?）");
                    exit_code = 1;
                    break 'run;
                }
                Err(_) => {
                    tracing::error!("{NO_FRAME_TIMEOUT:?} 无帧 — capturer 停止，退出待重启");
                    exit_code = 1;
                    break 'run;
                }
            },
            // F3: transport B 事件（视觉 DC；禁用时挂起不干扰视频）
            vis = next_vision_event(&mut vision, &vision_frames) => match vis {
                VisionEvent::Signal(ev) => {
                    let keep = match vision.as_mut() {
                        Some(v) => handle_vision_signal(v, ev).await,
                        None => true,
                    };
                    if !keep {
                        tracing::warn!("视觉转发降级禁用（视频不受影响）");
                        vision = None;
                        vision_frames = None;
                    }
                }
                VisionEvent::Ice(Some(c)) => {
                    let Some(v) = vision.as_mut() else { continue };
                    let msg = SignalingMessage::RTCIceCandidate {
                        room_id: format!("stream-{}", stream.id),
                        target: None,
                        candidate: c.candidate,
                        sdp_mid: c.sdp_mid,
                        sdp_mline_index: c.sdp_mline_index,
                    };
                    if let Err(e) = v.signal.send(msg).await {
                        tracing::warn!("vision ICE 候选上行失败: {e}");
                    }
                }
                VisionEvent::Ice(None) => {
                    tracing::warn!("vision ICE 通道关闭 — 降级禁用");
                    vision = None;
                    vision_frames = None;
                }
                VisionEvent::Frame(Some(f)) => {
                    let meta = f.meta();
                    if !vision_meta_ok(meta) {
                        tracing::warn!(seq = meta.seq, "vision 载荷非 JSON 格式（发布端协议违反），丢弃");
                        continue;
                    }
                    let Some(text) = vision_payload_text(f.payload()) else {
                        tracing::warn!(seq = meta.seq, "vision 载荷非 UTF-8，丢弃");
                        continue;
                    };
                    let Some(v) = vision.as_mut() else { continue };
                    if v.dc.state() != RTCDataChannelState::Open {
                        continue; // DC 未 Open（协商中）— 静默跳过，不刷屏
                    }
                    if let Err(e) = v.dc.send_text(text).await {
                        tracing::warn!(seq = meta.seq, "vision DC 发送失败: {e}");
                    }
                }
                VisionEvent::Frame(None) => {
                    tracing::warn!("vision 帧流关闭 — 降级禁用");
                    vision = None;
                    vision_frames = None;
                }
                VisionEvent::Deadline => {
                    tracing::warn!(
                        "{VISION_NEGOTIATE_TIMEOUT:?} 无舱端 answer — 视觉转发禁用（视频不受影响）"
                    );
                    vision = None;
                    vision_frames = None;
                }
            },
        }
    }

    // F3: 关闭 transport B（信号 + PC；独立于视频 transport A）
    if let Some(v) = vision {
        if let Err(e) = v.signal.close().await {
            tracing::warn!("vision signal close: {e}");
        }
        v.pc.close().await;
    }

    if let Err(e) = session.close().await {
        tracing::warn!("close: {e}");
    }
    tracing::info!("streamer stopped (exit={exit_code})");
    ExitCode::from(exit_code)
}
