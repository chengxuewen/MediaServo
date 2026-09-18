//! 控制平面 SFU-DC 化（client-dual-form S1）— host-controller 的传输与命令回路。
//!
//! 背景（勘误定案 09-15）：旧 P2P 形（controller=offerer，Sdp 中继等舱端 answer）
//! 在 all-SFU 决策（2026-08-25，server room.rs DeviceStream 统一）后为死路——
//! 注册房间的 Sdp/ICE 一律被帧过滤静默丢弃。真参照 = field::session：
//! - 出程（[`setup_send_side`]）：`PushSession::publish_video` 的
//!   CreateWebRtcTransport(Send)→TransportCreated→合成 offer→DC→create_answer→
//!   ConnectWebRtcTransport(dtls)→CreateDataProducer 序列，DC-only（无 add_track）。
//! - 入程（[`setup_recv_side`]）：`PullSession::subscribe` 的 Recv transport 形；
//!   S2d：consumer DC = DataConsumed 回执带外建 negotiated 通道（官方
//!   Chrome74.receiveDataChannel 契约；mediasoup 代理 worker 从不代发 DCEP，
//!   in-band on_data_channel 在此域永不触发）。
//!   舱端 control DataProducer 经 `NewDataProducer` 广播 → `ConsumeData` →
//!   SCTP inbound DC 到 `pc.on_data_channel`（mediaservo-webrtc 跨后端在位）。
//!
//! 命令处理（[`handle_command`]）：`parse_envelope` → [`Actuator::on_command`] →
//! `ControlAck{ack: seq, result}` 写回 "ack" DC（主路）+ 入站 DC 同通道回声
//! （两者 best-effort，C15 warn）；旁路镜像 FrameBus `control/cmd` / `control/ack`
//! （总线是镜像面，attach/发布失败永不阻塞执行）。
//!
//! 信令面纪律：合成 remote SDP 与 ICE/DTLS 消息形状逐字段镜像 PushSession（server
//! ICE-Lite：候选随 WebRtcTransportCreated 内联下发，无 RTCIceCandidate 交换回合；
//! `transport_connected`（Error{code:0} 惯例）非真错误）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mediaservo_common::protocol::{
    ControlAck, ControlEnvelope, DtlsParameters, Fingerprint, IceCandidate, IceParameters,
    MediaKind, SctpStreamParameters, SignalingMessage, TransportDirection, control_hmac_verify,
    is_estop_cmd, parse_envelope,
};
use mediaservo_link::{FrameBus, FrameMeta, FrameTopic, SignalEvent, SignalSession};
use mediaservo_webrtc::data_channel::{
    RTCDataChannel, RTCDataChannelEvent, RTCDataChannelInit, RTCDataChannelState,
};
use mediaservo_webrtc::peer_connection::{
    RTCIceConnectionState, RTCIceServer, RTCIceTransportPolicy,
};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCPeerConnection, RTCPeerConnectionFactory,
};
use tokio::sync::{broadcast, mpsc};

use crate::control::Actuator;

/// 命令通道 label（D-H3：chassis/light 可靠有序，gimbal partial-reliable）。
pub const DEFAULT_LABELS: [&str; 3] = ["chassis", "gimbal", "light"];
/// 回执回程 DC label（controller 自建 DataProducer，舱端 consume 后收 ControlAck）。
pub const ACK_LABEL: &str = "ack";
/// DataProducer 子协议名（mediasoup protocol 位，v1 固定 "sctp"）。
const DATA_PROTOCOL: &str = "sctp";
/// SFU peer 键（与 streamer 推流共用 "host"——transport_id 显式绑定防串线，C1 惯例）。
const SFU_PEER_ID: &str = "host";
/// FrameBus 镜像 topic（命令入程 / 回执出程；载荷 = envelope / ack JSON 原文）。
pub const TOPIC_CMD: &str = "control/cmd";
pub const TOPIC_ACK: &str = "control/ack";
/// ICE Failed 自愈退出前等待（PIT-87：状态收敛后退出，部署侧 restart_policy 拉起）。
const ICE_FAILED_WAIT: Duration = Duration::from_secs(1);

/// 通道可靠性（D-H3）: chassis/light 可靠有序（急停/开关类命令）；
/// gimbal partial-reliable（云台连续调节可丢帧，低延迟优先）。
pub fn channel_init(label: &str) -> RTCDataChannelInit {
    match label {
        "gimbal" => {
            RTCDataChannelInit { ordered: false, max_retransmits: Some(5), ..Default::default() }
        }
        _ => RTCDataChannelInit::default(), // chassis / light / ack: reliable ordered
    }
}

/// 出程 DC 的 SCTP 流参数：stream_id 取 libwebrtc 实配 DC id（mediasoup Direct
/// transport 无 remap，DataProducer.sctpStreamParameters.streamId == 端点 DC id）；
/// ordered/重传与 [`channel_init`] 同源（announce 与实际通道行为不漂移）。
pub fn sctp_stream_params(label: &str, id: i32) -> Result<SctpStreamParameters, String> {
    let stream_id = u16::try_from(id).map_err(|_| {
        format!("DC {label} id={id} 不可用作 SCTP stream_id（DataChannel 未分配 id）")
    })?;
    let init = channel_init(label);
    Ok(SctpStreamParameters {
        stream_id,
        ordered: init.ordered,
        max_packet_life_time: init.max_retransmit_time.and_then(|v| u16::try_from(v).ok()),
        max_retransmits: init.max_retransmits.and_then(|v| u16::try_from(v).ok()),
    })
}

/// 用 mediasoup transport 参数合成 DC-only remote offer（application m-line）。
/// 形状逐字段对齐 field::sfu::build_remote_sdp（PIT-48: a=candidate 必在 m= 之后；
/// mDNS .local 候选跳过；a=ice-lite + actpass → 本地 answer=DTLS client）。
pub fn build_dc_remote_sdp(
    ice: &IceParameters,
    dtls: &DtlsParameters,
    candidates: Option<&Vec<IceCandidate>>,
) -> String {
    let Some(fp) = dtls.fingerprints.first() else {
        tracing::warn!("dtls_parameters 无 fingerprint — 使用占位（DTLS 必败，日志已留痕）");
        return String::new();
    };
    let conn_ip = candidates
        .and_then(|cs| cs.iter().find(|c| !c.ip.contains(".local")))
        .map(|c| c.ip.clone())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let mut lines = vec![
        "v=0".to_string(),
        "o=- 0 0 IN IP4 0.0.0.0".to_string(),
        "s=-".to_string(),
        "t=0 0".to_string(),
        "a=group:BUNDLE data".to_string(),
        "a=ice-lite".to_string(),
        format!("a=ice-ufrag:{}", ice.username_fragment),
        format!("a=ice-pwd:{}", ice.password),
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".to_string(),
        // libmediasoupclient data 段形状（application/UDP/DTLS/SCTP）
        "m=application 9 UDP/DTLS/SCTP webrtc-datachannel".to_string(),
        format!("c=IN IP4 {conn_ip}"),
        "a=mid:data".to_string(),
        "a=sctp-port:5000".to_string(),
        "a=max-message-size:262144".to_string(),
    ];

    if let Some(cands) = candidates {
        for c in cands {
            if c.ip.contains(".local") {
                continue; // skip mDNS
            }
            let ctype = match c.candidate_type.as_str() {
                "host" | "srflx" | "prflx" | "relay" => c.candidate_type.as_str(),
                _ => "host",
            };
            lines.push(format!(
                "a=candidate:{} 1 {} {} {} {} typ {}",
                c.foundation,
                c.protocol.to_uppercase(),
                c.priority,
                c.ip,
                c.port,
                ctype
            ));
        }
    }
    lines.push("a=end-of-candidates".to_string());
    lines.push(String::new());
    lines.join("\r\n")
}

/// ControlAck 写出口（生产 = DC 双写；测试 = 内存收集）。
#[async_trait]
pub trait AckSink: Send + Sync {
    async fn write(&self, text: &str) -> Result<(), String>;
}

/// 生产实现：ack DC 主路 + 入站命令 DC 同通道回声（两者 best-effort，C15 warn）。
pub struct DcAckSink {
    /// "ack" 出程 DC 槽位（Arc：路由任务可与 ack 建立解耦，后绑定亦可）。
    pub ack: Arc<Mutex<Option<RTCDataChannel>>>,
    /// 触发本次回执的入站 DC（回声通道）。
    pub inbound: RTCDataChannel,
}

#[async_trait]
impl AckSink for DcAckSink {
    async fn write(&self, text: &str) -> Result<(), String> {
        let ack_dc = self.ack.lock().unwrap_or_else(|p| p.into_inner()).clone();
        match ack_dc {
            Some(dc) => {
                if let Err(e) = dc.send_text(text).await {
                    tracing::warn!(label = %dc.label(), "ack DC 写回失败: {e}");
                }
            }
            None => tracing::warn!("ack DC 未建立 — 回执主路丢失（回声仍试发）"),
        }
        if self.inbound.state() == RTCDataChannelState::Open
            && let Err(e) = self.inbound.send_text(text).await
        {
            tracing::warn!(label = %self.inbound.label(), "入站 DC 回声失败: {e}");
        }
        Ok(())
    }
}

/// 测试实现：收集写出的 ack 文本（无网络 env→ack 往返断言用）。
#[derive(Clone, Default)]
pub struct CollectAckSink {
    pub items: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl AckSink for CollectAckSink {
    async fn write(&self, text: &str) -> Result<(), String> {
        self.items.lock().unwrap_or_else(|p| p.into_inner()).push(text.to_string());
        Ok(())
    }
}

/// FrameBus 镜像发布（旁路面：任何失败仅 warn，永不阻塞执行，C15）。
pub fn publish_bus(bus: Option<&FrameBus>, topic: &str, payload: &[u8]) {
    let Some(bus) = bus else { return };
    if let Err(e) = bus.publish(&FrameTopic::new(topic), payload, &FrameMeta::default()) {
        tracing::warn!(topic, "FrameBus 镜像发布失败（不影响执行）: {e}");
    }
}

/// 单条命令处理：parse_envelope → Actuator::on_command → ControlAck 写回 + 总线镜像。
/// S4/a4·T3.5：命令执行策略（e-stop 验签门 + actuation 审计落点）。
/// 规则（无自锁迁移形态）：hmac_key 未配置 = 验签不启用（estop 放行 + WARN 留痕）；
/// 配置后 estop 必验，sig 缺失仅在 `allow_unsigned_estop=true` 时放行。
#[derive(Debug, Clone, Default)]
pub struct CommandPolicy {
    pub hmac_key: Option<String>,
    pub allow_unsigned_estop: bool,
    /// act-then-audit jsonl 落点（None = 仅 tracing 行，永不阻塞执行）。
    pub actuation_log: Option<std::path::PathBuf>,
}

impl CommandPolicy {
    /// env 驱动构造（deploy 面单点）：
    /// `MEDIASERVO_CONTROL_HMAC_KEY_FILE`（**推荐**，G13 文件形 0600，与舱端 client-c
    /// `hmac_key_file` 同纪律同真源）优先于 `MEDIASERVO_CONTROL_HMAC_KEY`（明文形，
    /// 迁移兼容）；皆缺省 = 验签不启用。文件**配置了但读失败 = panic 早死**（fail-fast：
    /// 安全功能静默降级为未配置形不可接受，与舱端"坏路径不拦会话"方向相反——车端
    /// 密钥是验签闸门本身，静默放行 = 装门不锁）。`CONTROL_ALLOW_UNSIGNED_ESTOP=1` 逃生位；
    /// `CONTROL_ACTUATION_LOG` 未设 = 仅 tracing 行。
    #[must_use]
    pub fn from_env() -> Self {
        let key_file =
            std::env::var("MEDIASERVO_CONTROL_HMAC_KEY_FILE").ok().filter(|v| !v.is_empty());
        Self {
            hmac_key: match key_file {
                Some(p) => Some(
                    mediaservo_common::protocol::control_hmac_key_from_file(&p)
                        .unwrap_or_else(|e| panic!("controller 急停密钥不可用: {e}")),
                ),
                None => std::env::var("MEDIASERVO_CONTROL_HMAC_KEY").ok().filter(|v| !v.is_empty()),
            },
            allow_unsigned_estop: std::env::var("CONTROL_ALLOW_UNSIGNED_ESTOP").as_deref()
                == Ok("1"),
            actuation_log: std::env::var("CONTROL_ACTUATION_LOG")
                .ok()
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from),
        }
    }
}

/// estop 验签裁决矩阵（纯函数可测）。
#[must_use]
pub fn estop_verdict(policy: &CommandPolicy, env: &ControlEnvelope) -> (&'static str, bool) {
    // (sig_state, 是否放行执行)
    if !is_estop_cmd(&env.cmd) {
        return ("n/a", true);
    }
    match (policy.hmac_key.as_deref(), env.sig.as_deref()) {
        (None, Some(_)) => ("signed-unconfigured", true),
        (None, None) => ("unsigned-unconfigured", true),
        (Some(_), Some(sig)) => {
            if control_hmac_verify(policy.hmac_key.as_deref().unwrap_or_default(), env, sig) {
                ("ok", true)
            } else {
                ("invalid", false)
            }
        }
        (Some(_), None) => {
            if policy.allow_unsigned_estop {
                ("absent-allowed", true)
            } else {
                ("absent", false)
            }
        }
    }
}

pub async fn handle_command(
    label: &str,
    data: &[u8],
    actuator: &dyn Actuator,
    bus: Option<&FrameBus>,
    sink: &dyn AckSink,
    policy: &CommandPolicy,
) {
    let env = match parse_envelope(data) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(label, len = data.len(), "信封解析失败: {e}");
            return;
        }
    };
    // S4/a4: e-stop 验签门（执行前拒 = 语义；拒执亦回执 err ack + 审计行）。
    let (sig_state, allow) = estop_verdict(policy, &env);
    if !allow {
        tracing::warn!(
            channel = %label,
            cmd = %env.cmd,
            seq = env.seq,
            "S4: e-stop 验签失败拒执（sig={sig_state}）"
        );
        actuation_audit(policy, label, &env, sig_state, "rejected");
        let json = serde_json::to_string(&ControlAck::err(env.seq, "estop_signature_rejected"))
            .unwrap_or_default();
        let _ = sink.write(&json).await;
        return;
    }
    if sig_state == "unsigned-unconfigured" && is_estop_cmd(&env.cmd) {
        tracing::warn!("S4: estop 到达但 hmac_key 未配置——放行（部署缺口，见 actuation 审计）");
    }
    publish_bus(bus, TOPIC_CMD, data);
    let mut exec_state = "ok";
    let ack = match actuator.on_command(label, &env) {
        Ok(result) => ControlAck::ok(env.seq, result),
        Err(e) => {
            exec_state = "error";
            // C15: 错误分支必须打日志（错误回执仍发对端，本侧可观测性不能丢）
            tracing::warn!(
                channel = %label,
                cmd = %env.cmd,
                seq = env.seq,
                error = %e,
                "actuator 命令失败"
            );
            ControlAck::err(env.seq, e)
        }
    };
    actuation_audit(policy, label, &env, sig_state, exec_state);
    let json = match serde_json::to_string(&ack) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(channel = %label, seq = env.seq, "ControlAck 序列化失败: {e}");
            return;
        }
    };
    publish_bus(bus, TOPIC_ACK, json.as_bytes());
    if let Err(e) = sink.write(&json).await {
        tracing::warn!(channel = %label, seq = env.seq, "回执写回失败: {e}");
    }
}

/// act-then-audit：jsonl（env 显式路径才写文件）+ 恒发 tracing 行；IO 错误容忍
/// （审计永不阻塞执行——席3 裁决）。
fn actuation_audit(
    policy: &CommandPolicy,
    label: &str,
    env: &ControlEnvelope,
    sig: &str,
    exec: &str,
) {
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    tracing::info!(
        channel = %label,
        cmd = %env.cmd,
        seq = env.seq,
        sig,
        exec,
        ts_ms,
        "actuation"
    );
    let Some(path) = &policy.actuation_log else { return };
    let line = serde_json::json!({
        "ts_ms": ts_ms, "channel": label, "cmd": env.cmd, "seq": env.seq,
        "sig": sig, "exec": exec,
    });
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}"); // 审计写失败仅容忍（执行已过）
    }
}

/// 单通道路由：spool 接收 → [`handle_command`] → 回执（ack DC 主路 + 同通道回声）。
pub async fn route_dc(
    mut dc: RTCDataChannel,
    actuator: Arc<dyn Actuator>,
    bus: Option<Arc<FrameBus>>,
    ack: Arc<Mutex<Option<RTCDataChannel>>>,
    mut purge: mpsc::UnboundedReceiver<()>,
    policy: Arc<CommandPolicy>,
) {
    let label = dc.label().to_string();
    let mut rx = dc.spool().await;
    tracing::info!(label, "DC 路由启动");
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(RTCDataChannelEvent::Open) => tracing::info!(label, "DC open"),
                Some(RTCDataChannelEvent::Closed) => {
                    tracing::info!(label, "DC closed");
                    break;
                }
                Some(RTCDataChannelEvent::Error(e)) => tracing::warn!(label, "DC error: {e}"),
                Some(RTCDataChannelEvent::Message(m)) => {
                    let sink = DcAckSink {
                        ack: ack.clone(),
                        inbound: dc.clone(),
                    };
                    handle_command(&label, &m.data, actuator.as_ref(), bus.as_deref(), &sink, &policy)
                        .await;
                }
                None => break,
            },
            // S4′: 上游 DataProducer 死亡（主循环定向）→ 本地关闭释放 libwebrtc
            // stream id 占用（否则 zombie DC = `StreamId reserved` 消费天花板）。
            _ = purge.recv() => {
                tracing::info!(label, "S4′: 上游 DataProducer 死亡—consumer DC 自拆");
                dc.close().await;
                break;
            }
        }
    }
}

/// transport 建立结果（WebRtcTransportCreated 载荷四元组）。
struct TransportCreated {
    transport_id: String,
    ice: IceParameters,
    dtls: DtlsParameters,
    candidates: Option<Vec<IceCandidate>>,
}

/// 待消费的舱端 data producer（建立期夹带广播的缓存，防 broadcast 无重放竞态）。
#[derive(Clone)]
struct PendingProducer {
    data_producer_id: String,
    label: String,
}

/// 消费信令直到 WebRtcTransportCreated；途中 NewDataProducer 入 pending（不丢）。
/// `events` 必须在发请求前订阅（broadcast 无历史重放 — field session.rs:137 同款）。
async fn await_transport_created(
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
) -> Result<TransportCreated, String> {
    loop {
        match events.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            })) => {
                return Ok(TransportCreated {
                    transport_id,
                    ice: ice_parameters,
                    dtls: dtls_parameters,
                    candidates: ice_candidates,
                });
            }
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                ..
            })) => {
                pending.push(PendingProducer { data_producer_id, label });
            }
            // transport_connected 是 Connect 确认（server 惯例 Error{code:0}，非真错误）
            Ok(SignalEvent::Message(SignalingMessage::Error { message, .. }))
                if message == "transport_connected" => {}
            Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                return Err(format!("SFU error [{code}]: {message}"));
            }
            Ok(SignalEvent::Disconnected { reason }) => {
                return Err(format!("transport 建立期信令断开: {reason}"));
            }
            Ok(SignalEvent::Connected { .. }) => {}
            Ok(SignalEvent::Error(e)) => return Err(format!("信令错误: {e}")),
            Ok(_) => {} // 其余消息忽略（NewProducer/RoomJoined 等，同 field 惯例）
            Err(_) => return Err("transport 建立期信令事件流关闭".into()),
        }
    }
}

/// 消费信令直到 DataProducerCreated（返回 producer id）；缓存/豁免规则同
/// [`await_transport_created`]。
async fn await_data_producer_created(
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
) -> Result<String, String> {
    loop {
        match events.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::DataProducerCreated {
                data_producer_id,
                ..
            })) => return Ok(data_producer_id),
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                ..
            })) => {
                pending.push(PendingProducer { data_producer_id, label });
            }
            Ok(SignalEvent::Message(SignalingMessage::Error { message, .. }))
                if message == "transport_connected" => {}
            Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                return Err(format!("DataProducer 建立被拒 [{code}]: {message}"));
            }
            Ok(SignalEvent::Disconnected { reason }) => {
                return Err(format!("DataProducer 建立期信令断开: {reason}"));
            }
            Ok(SignalEvent::Connected { .. }) => {}
            Ok(SignalEvent::Error(e)) => return Err(format!("信令错误: {e}")),
            Ok(_) => {}
            Err(_) => return Err("DataProducer 建立期信令事件流关闭".into()),
        }
    }
}

/// CreateWebRtcTransport(direction) + PC 建立 + set_remote（合成 offer）。
/// answer 不在这里发——出程要先 create_data_channel、入程要先注册
/// on_data_channel（晚注册丢首事件，PullSession 教训），故拆为 [`connect_answer`]。
async fn open_transport(
    signal: &SignalSession,
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
    direction: TransportDirection,
) -> Result<(RTCPeerConnection, TransportCreated), String> {
    let dir_tag = format!("{direction:?}");
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: signal.room_id().to_string(),
            peer_id: SFU_PEER_ID.into(),
            direction,
        })
        .await
        .map_err(|e| format!("CreateWebRtcTransport({dir_tag}): {e}"))?;
    let t = await_transport_created(events, pending).await?;

    let factory = RTCPeerConnectionFactory::new();
    let pc = factory
        .create_peer_connection(RTCConfiguration {
            ice_servers: Vec::<RTCIceServer>::new(), // SFU: mediasoup ICE-Lite，无需 STUN
            ice_transport_type: RTCIceTransportPolicy::All,
        })
        .await
        .map_err(|e| format!("create_peer_connection: {e}"))?;

    let remote_sdp = build_dc_remote_sdp(&t.ice, &t.dtls, t.candidates.as_ref());
    pc.set_remote_description(&RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp))
        .await
        .map_err(|e| format!("set_remote_description: {e}"))?;
    Ok((pc, t))
}

/// 协商收口：create_answer → set_local → ConnectWebRtcTransport(本地 DTLS 指纹,
/// role=client)。PushSession 第 6 步同款；mediasoup direct transport 无 SDP 上行，
/// answer 仅供本地状态机（server 只消费 connect 指纹 + CreateDataProducer 参数）。
async fn connect_answer(
    signal: &SignalSession,
    pc: &RTCPeerConnection,
    t: &TransportCreated,
) -> Result<(), String> {
    let answer =
        pc.create_answer(&RTCAnswerOptions).await.map_err(|e| format!("create_answer: {e}"))?;
    tracing::debug!("controller answer SDP:\n{}", answer.sdp);
    pc.set_local_description(&answer).await.map_err(|e| format!("set_local_description: {e}"))?;
    let fp_hex = pc.local_dtls_fingerprint().ok_or_else(|| "无本地 DTLS 指纹".to_string())?;
    signal
        .send(SignalingMessage::ConnectWebRtcTransport {
            room_id: signal.room_id().to_string(),
            peer_id: SFU_PEER_ID.into(),
            transport_id: t.transport_id.clone(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
                role: "client".to_string(),
            },
        })
        .await
        .map_err(|e| format!("ConnectWebRtcTransport: {e}"))?;
    Ok(())
}

/// 出程建立：Send transport → 每 label create_data_channel（含 "ack"，先于 answer）→
/// answer/connect → 逐 DC CreateDataProducer announce。返回 (pc, 自建 producer id 集)。
#[allow(clippy::too_many_arguments)] // SFU-DC 建连参数组（S1 形），打包 = 独立重构
async fn setup_send_side(
    signal: &SignalSession,
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
    labels: &[String],
    ack: Arc<Mutex<Option<RTCDataChannel>>>,
    actuator: &Arc<dyn Actuator>,
    bus: &Option<Arc<FrameBus>>,
    policy: &Arc<CommandPolicy>,
) -> Result<(RTCPeerConnection, HashSet<String>), String> {
    let (pc, t) = open_transport(signal, events, pending, TransportDirection::Send).await?;
    let mut dcs = Vec::with_capacity(labels.len());
    for label in labels {
        let dc = pc
            .create_data_channel(label, channel_init(label))
            .await
            .map_err(|e| format!("create_data_channel {label}: {e}"))?;
        if label == ACK_LABEL {
            *ack.lock().unwrap_or_else(|p| p.into_inner()) = Some(dc.clone());
        }
        // 出程通道也挂路由（防御：对端若经本通道回写仍走统一命令链）
        // 自建出程 DC 无定向拆除需求：tx 刻意 forget = 通道永活不发消息
        //（select purge 臂恒悬停；4 字节泄漏为显式契约）。
        let (purge_tx, purge_rx) = mpsc::unbounded_channel();
        std::mem::forget(purge_tx);
        tokio::spawn(route_dc(
            dc.clone(),
            actuator.clone(),
            bus.clone(),
            ack.clone(),
            purge_rx,
            policy.clone(),
        ));
        dcs.push((label.clone(), dc));
    }
    connect_answer(signal, &pc, &t).await?;
    let mut own = HashSet::new();
    for (label, dc) in &dcs {
        signal
            .send(SignalingMessage::CreateDataProducer {
                room_id: signal.room_id().to_string(),
                peer_id: SFU_PEER_ID.into(),
                transport_direction: TransportDirection::Send,
                label: label.clone(),
                protocol: DATA_PROTOCOL.into(),
                sctp_stream_parameters: Some(sctp_stream_params(dc.label(), dc.id())?),
                transport_id: Some(t.transport_id.clone()),
            })
            .await
            .map_err(|e| format!("CreateDataProducer {label}: {e}"))?;
        let dp_id = await_data_producer_created(events, pending).await?;
        tracing::info!(label = %label, data_producer_id = %dp_id, "controller DataProducer 已建立");
        own.insert(dp_id);
    }
    Ok((pc, own))
}

/// 入程建立：Recv transport → answer/connect。S2d 起不再注册 on_data_channel
/// （mediasoup 代理域 in-band DCEP 永不来）；消费通道由主循环收到 DataConsumed
/// 后按服务端回执参数以 negotiated 形建（[`consume_data`] 的请求侧闭环）。
async fn setup_recv_side(
    signal: &SignalSession,
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
) -> Result<(RTCPeerConnection, String), String> {
    let (pc, t) = open_transport(signal, events, pending, TransportDirection::Recv).await?;
    connect_answer(signal, &pc, &t).await?;
    Ok((pc, t.transport_id))
}

/// 消费一个舱端 data producer（ConsumeData；应答 DataConsumed 由主循环建 negotiated DC，S2d）。
async fn consume_data(signal: &SignalSession, recv_transport_id: &str, dp_id: &str) {
    if let Err(e) = signal
        .send(SignalingMessage::ConsumeData {
            room_id: signal.room_id().to_string(),
            peer_id: SFU_PEER_ID.into(),
            transport_direction: TransportDirection::Recv,
            data_producer_id: dp_id.to_string(),
            transport_id: Some(recv_transport_id.to_string()),
        })
        .await
    {
        tracing::warn!(data_producer_id = %dp_id, "ConsumeData 发送失败: {e}");
    }
}

/// 控制器配置（v1：默认三命令通道 + ack；label 面可覆写供夹具/演进）。
#[derive(Debug, Clone)]
pub struct ControllerConfig {
    pub labels: Vec<String>,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        let mut labels: Vec<String> = DEFAULT_LABELS.iter().map(|s| s.to_string()).collect();
        labels.push(ACK_LABEL.to_string());
        Self { labels }
    }
}

/// 主循环：双侧 transport 建立 → 排空建立期缓存 → 事件循环（NewDataProducer 消费 /
/// DataConsumed·DataProducerCreated 日志 / 信令断连·ICE Failed 自愈退出）。
/// 返回进程退出码（0 = 优雅退出，1 = 故障退出待拉起）。
pub async fn control_loop(
    signal: SignalSession,
    cfg: ControllerConfig,
    actuator: Arc<dyn Actuator>,
    bus: Option<Arc<FrameBus>>,
    policy: CommandPolicy,
) -> u8 {
    let policy = Arc::new(policy);
    // 先订阅事件再发任何请求（broadcast 无历史重放）
    let mut events = signal.events();
    let mut pending: Vec<PendingProducer> = Vec::new();
    let ack: Arc<Mutex<Option<RTCDataChannel>>> = Arc::new(Mutex::new(None));

    let (send_pc, own) = match setup_send_side(
        &signal,
        &mut events,
        &mut pending,
        &cfg.labels,
        ack.clone(),
        &actuator,
        &bus,
        &policy,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("controller: 出程（Send transport + DataProducer）建立失败: {e}");
            return 1;
        }
    };
    let (recv_pc, recv_tid) = match setup_recv_side(&signal, &mut events, &mut pending).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("controller: 入程（Recv transport）建立失败: {e}");
            send_pc.close().await;
            return 1;
        }
    };
    // own 回声去重：server 把 NewDataProducer 广播给全房（含自己）——自建 id 集跳过
    let mut consumed: HashSet<String> = own.clone();
    // S4′: dp_id → consumer DC 定向拆除通道（ProducerClosed(Data) 到达时 send）。
    let mut consumer_purge: HashMap<String, mpsc::UnboundedSender<()>> = HashMap::new();

    // 建立期夹带的 NewDataProducer：排空消费
    for np in pending.drain(..) {
        if consumed.insert(np.data_producer_id.clone()) {
            tracing::info!(
                label = %np.label,
                data_producer_id = %np.data_producer_id,
                "controller 消费舱端 DataProducer（建立期缓存）"
            );
            consume_data(&signal, &recv_tid, &np.data_producer_id).await;
        }
    }

    // ICE Failed 自愈（PIT-87：双侧任一 Failed → 状态收敛后退出待拉起）
    let (fail_tx, mut fail_rx) = mpsc::unbounded_channel::<()>();
    for pc in [&send_pc, &recv_pc] {
        let tx = fail_tx.clone();
        pc.on_ice_connection_state_change(move |state| {
            if state == RTCIceConnectionState::Failed {
                tracing::error!("ICE Failed — 退出待重启（PIT-87 自愈）");
                let _ = tx.send(());
            }
        });
    }

    println!("controller ready: room={} labels={}", signal.room_id(), cfg.labels.join(","));

    let mut exit_code: u8 = 0;
    'run: loop {
        tokio::select! {
            _ = wait_shutdown() => break 'run,
            _ = fail_rx.recv() => {
                tokio::time::sleep(ICE_FAILED_WAIT).await;
                exit_code = 1;
                break 'run;
            }
            ev = events.recv() => match ev {
                Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                    data_producer_id,
                    label,
                    ..
                })) => {
                    if consumed.insert(data_producer_id.clone()) {
                        tracing::info!(
                            label = %label,
                            data_producer_id = %data_producer_id,
                            "controller 消费舱端 DataProducer"
                        );
                        consume_data(&signal, &recv_tid, &data_producer_id).await;
                    }
                }
                Ok(SignalEvent::Message(SignalingMessage::DataConsumed {
                    data_consumer_id,
                    data_producer_id,
                    sctp_stream_parameters,
                    label,
                    protocol,
                    ..
                })) => {
                    // S2d 官方契约：consumer 以带外 negotiated DC（id=worker 分配的
                    // stream_id）接收转发消息。缺参数 = 老 server，明确报错（C15）。
                    let Some(sp) = sctp_stream_parameters else {
                        tracing::error!(
                            data_consumer_id,
                            "DataConsumed 缺 sctp 参数（server 过旧？）——无法建 negotiated DC"
                        );
                        continue;
                    };
                    let init = RTCDataChannelInit {
                        negotiated: true,
                        id: sp.stream_id as i32,
                        ordered: sp.ordered,
                        max_retransmit_time: sp.max_packet_life_time.map(i32::from),
                        max_retransmits: sp.max_retransmits.map(i32::from),
                        protocol,
                    };
                    match recv_pc.create_data_channel(&label, init).await {
                        Ok(dc) => {
                            tracing::info!(
                                data_consumer_id,
                                data_producer_id,
                                label,
                                stream_id = sp.stream_id,
                                "DataConsumed：negotiated consumer DC 已建，开始收令"
                            );
                            let (a, b, k) = (actuator.clone(), bus.clone(), ack.clone());
                            let (purge_tx, purge_rx) = mpsc::unbounded_channel();
                            consumer_purge.insert(data_producer_id.clone(), purge_tx);
                            tokio::spawn(route_dc(dc, a, b, k, purge_rx, policy.clone()));
                        }
                        Err(e) => {
                            tracing::error!(label, "negotiated consumer DC 创建失败: {e}");
                        }
                    }
                }
                Ok(SignalEvent::Message(SignalingMessage::ProducerClosed {
                    producer_id,
                    kind: MediaKind::Data,
                    ..
                })) => {
                    // S4′: 上游 data producer 死亡（server 定向广播）→ 本地 consumer
                    // DC 自拆 + 台账清理（允许对端重订阅后再次消费）。
                    if let Some(tx) = consumer_purge.remove(&producer_id) {
                        let _ = tx.send(());
                        consumed.remove(&producer_id);
                        tracing::info!(producer_id, "S4′: 定向拆除已下发");
                    }
                }
                Ok(SignalEvent::Message(SignalingMessage::DataProducerCreated { .. })) => {
                    // announce 确认已在建立期消费；网关广播回声在此忽略（C16 已建链）
                }
                Ok(SignalEvent::Message(SignalingMessage::Error { code, message }))
                    if code != 0 =>
                {
                    // data 域请求不入网关 FIFO 队列，非配对 Error 可能广播至此——
                    // 记录并继续，信令级故障由 Disconnected 收口。
                    tracing::warn!("控制域信令错误 [{code}]: {message}");
                }
                Ok(SignalEvent::Message(_)) => {} // 其余透传（RoomJoined/媒体面等）
                Ok(SignalEvent::Connected { .. }) => {}
                Ok(SignalEvent::Error(e)) => {
                    tracing::error!("信令错误: {e}");
                    exit_code = 1;
                    break 'run;
                }
                Ok(SignalEvent::Disconnected { reason }) => {
                    tracing::error!("信令断开: {reason} — 退出待重启");
                    exit_code = 1;
                    break 'run;
                }
                Ok(_) => {} // SignalEvent non_exhaustive 兜底
                Err(_) => {
                    tracing::error!("信令事件流关闭");
                    exit_code = 1;
                    break 'run;
                }
            },
        }
    }

    if let Err(e) = signal.close().await {
        tracing::warn!("close: {e}");
    }
    send_pc.close().await;
    recv_pc.close().await;
    tracing::info!("controller stopped (exit={exit_code})");
    exit_code
}

/// 等待 SIGINT/SIGTERM（unix 主路径；其他平台仅 ctrl_c）。
pub async fn wait_shutdown() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = sigterm.recv() => {}
                }
            }
            Err(e) => {
                tracing::warn!("SIGTERM 监听失败（仅 ctrl_c）: {e}");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
