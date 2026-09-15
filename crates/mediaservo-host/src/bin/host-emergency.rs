//! host-emergency: 紧急停车进程（Task F2）— 独立 PC + DC "emergency" + 本地兜底。
//!
//! 用法: `host-emergency [--gateway <本地网关 ws url>] [--audit <审计文件>]`
//! 缺省: 网关 `ws://127.0.0.1:17980/ws`（D2 本地网关）；审计
//! `/tmp/mediaservo-emergency-audit.jsonl`（D-H11 强审计本地侧）。
//!
//! 结构 = host-controller（F1）的兄弟进程，三处差异（task-F2-brief）:
//! 1. 单 DC label "emergency"（reliable-ordered — D-H3 急停必须可靠有序）
//! 2. 命令语义唯一: `{"seq":N,"cmd":"stop"}` → `EmergencyActuator::trigger`
//!    （概念一次性闩锁: 首次 armed，重复触发仍审计 + 回执 latched=false）
//! 3. 本地兜底（D-H3 网络无关）: SIGUSR1 → 同一 trigger()（source=local）
//!
//! 协商: **emergency 为 offerer**（legacy webrtc_transport 惯例，与 controller
//! 相同）；与 controller 各自独立 PC（D-H3: PC 崩不影响急停）。网关
//! `p2p_owner` 单槽归属在两次**顺序**协商下安全移交（已完成协商不再上行
//! Sdp/ICE；并发协商不在 F2 范围，见 task-F2-report）。
//!
//! 失败语义（C15 + PIT-87 自愈惯例）：信令断开 / ICE Failed / 会话错误 →
//! 打日志退出 1，部署侧 restart_policy=always 拉起。审计文件不可写 →
//! 启动即退出 2（强审计不可用不可运行）。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_host::control::{ControlAck, ControlEnvelope, parse_envelope};
use mediaservo_host::emergency::{
    EMERGENCY_STOP, EmergencyActuator, EmergencySource, StubEmergencyActuator,
};
use mediaservo_link::{SignalClient, SignalEvent};
use mediaservo_webrtc::data_channel::{RTCDataChannel, RTCDataChannelEvent, RTCDataChannelInit};
use mediaservo_webrtc::peer_connection::{RTCConfiguration, RTCIceCandidate};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{RTCPeerConnection, RTCPeerConnectionFactory};
use tokio::sync::mpsc;

/// 本地房间名（网关拦截并重写为整车房间；与 controller 同房间，各持独立 PC）。
const ROOM: &str = "control";
/// 本地信封 src（网关子进程标识）。
const SRC: &str = "host-emergency";
const DEFAULT_GATEWAY: &str = "ws://127.0.0.1:17980/ws";
const DEFAULT_AUDIT: &str = "/tmp/mediaservo-emergency-audit.jsonl";
/// ICE Failed 自愈退出前等待（PIT-87 同款：状态收敛后退出待拉起）。
const ICE_FAILED_WAIT: Duration = Duration::from_secs(1);

const USAGE: &str = "用法: host-emergency [--gateway <本地网关 ws url>] [--audit <审计文件>]";

/// 纯参数解析（可单测）：`--gateway <url>` / `--audit <path>`，均带缺省。
fn args_from(args: impl Iterator<Item = String>) -> Result<(String, PathBuf), String> {
    let mut args = args.peekable();
    let mut gateway: Option<String> = None;
    let mut audit: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gateway" => gateway = Some(args.next().ok_or("--gateway 缺值")?),
            "--audit" => audit = Some(args.next().ok_or("--audit 缺值")?.into()),
            _ => return Err(format!("未知参数: {arg}\n{USAGE}")),
        }
    }
    Ok((
        gateway.unwrap_or_else(|| DEFAULT_GATEWAY.to_string()),
        audit.unwrap_or_else(|| PathBuf::from(DEFAULT_AUDIT)),
    ))
}

fn parse_args() -> Result<(String, PathBuf), String> {
    args_from(std::env::args().skip(1))
}

/// 等待 SIGINT/SIGTERM（unix 主路径；其他平台仅 ctrl_c）。
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// DC 信封 → 急停执行器 → ACK（纯函数可单测）。
/// 命令语义唯一: `cmd == EMERGENCY_STOP` → trigger；其余拒绝（回执 Err）。
fn handle_envelope(actuator: &dyn EmergencyActuator, env: &ControlEnvelope) -> ControlAck {
    if env.cmd != EMERGENCY_STOP {
        return ControlAck::err(
            env.seq,
            format!("unknown cmd {:?}（仅接受 {EMERGENCY_STOP:?}）", env.cmd),
        );
    }
    match actuator.trigger(EmergencySource::Dc, Some(env.seq)) {
        Ok(t) => {
            let audit = match &t.audit {
                Ok(()) => serde_json::json!("ok"),
                Err(e) => serde_json::json!({ "error": e }),
            };
            ControlAck::ok(
                env.seq,
                serde_json::json!({
                    "ok": true,
                    "source": "dc",
                    "latched": t.latched,
                    "trigger_count": t.trigger_count,
                    "audit": audit,
                }),
            )
        }
        Err(e) => ControlAck::err(env.seq, e),
    }
}

/// 单通道路由：spool 接收 → 解析信封 → handle_envelope → 同通道回 ACK。
async fn route_channel(dc: RTCDataChannel, actuator: Arc<dyn EmergencyActuator>) {
    let label = dc.label().to_string();
    let mut rx = dc.spool().await;
    tracing::info!(label, "DC 路由启动");
    while let Some(ev) = rx.recv().await {
        match ev {
            RTCDataChannelEvent::Open => tracing::info!(label, "DC open"),
            RTCDataChannelEvent::Closed => {
                tracing::info!(label, "DC closed");
                break;
            }
            RTCDataChannelEvent::Error(e) => tracing::warn!(label, "DC error: {e}"),
            RTCDataChannelEvent::Message(m) => {
                let env = match parse_envelope(&m.data) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(label, len = m.data.len(), "信封解析失败: {e}");
                        continue;
                    }
                };
                let ack = handle_envelope(actuator.as_ref(), &env);
                if let Ok(json) = serde_json::to_string(&ack)
                    && let Err(e) = dc.send_text(&json).await
                {
                    tracing::warn!(label, seq = env.seq, "ACK 发送失败: {e}");
                }
            }
        }
    }
}

/// 建立纯 DC PC：注册回调、创建 "emergency" 通道（reliable-ordered）、发起 offer。
async fn setup_pc(
    pc: &RTCPeerConnection,
    actuator: Arc<dyn EmergencyActuator>,
    ice_tx: mpsc::UnboundedSender<RTCIceCandidate>,
) -> Result<(), String> {
    // 对端创建的通道（offerer 路径不预期，防御性路由）
    let a = actuator.clone();
    pc.on_data_channel(move |dc| {
        tokio::spawn(route_channel(dc, a.clone()));
    });
    pc.on_ice_candidate(move |candidate| {
        let _ = ice_tx.send(candidate);
    });
    let dc = pc
        .create_data_channel("emergency", RTCDataChannelInit::default())
        .await
        .map_err(|e| format!("create_data_channel emergency: {e}"))?;
    tokio::spawn(route_channel(dc, actuator.clone()));
    Ok(())
}

/// 本地兜底（D-H3 网络无关）: SIGUSR1 → 同一 EmergencyActuator（source=local）。
async fn local_trigger(actuator: &dyn EmergencyActuator) {
    match actuator.trigger(EmergencySource::Local, None) {
        Ok(t) => tracing::info!(
            latched = t.latched,
            trigger_count = t.trigger_count,
            "本地兜底急停触发（SIGUSR1）"
        ),
        // C15: 错误分支必须打日志
        Err(e) => tracing::error!("本地急停执行器失败: {e}"),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("emergency");
    let (gateway, audit_path) = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    // 强审计（D-H11 本地侧）: 文件不可用 = 不可运行（启动即失败）
    let actuator: Arc<dyn EmergencyActuator> = match StubEmergencyActuator::new(&audit_path) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("emergency: {e}");
            return ExitCode::from(2);
        }
    };
    tracing::info!(audit = %audit_path.display(), "强审计文件就绪");

    // 本地兜底信号尽早注册 — handler 注册前到达的 SIGUSR1 = 默认终止进程
    #[cfg(unix)]
    let mut usr1 =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("emergency: SIGUSR1 注册失败: {e}");
                return ExitCode::from(1);
            }
        };

    // 信令：经本地网关（D2 信封 wire；网关拦截 RoomJoin 合成 RoomJoined）
    let signal = match SignalClient::new_gateway(&gateway, SRC, ROOM, PeerRole::Host)
        .connect()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("emergency: 信令连接失败: {e}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(room = %signal.room_id(), "emergency 已加入本地房间");

    // 纯 DC PC（无 track）— 独立于 controller 的 PC（D-H3）
    let factory = RTCPeerConnectionFactory::new();
    let pc = match factory
        .create_peer_connection(RTCConfiguration::default())
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("emergency: create_peer_connection 失败: {e}");
            return ExitCode::from(1);
        }
    };

    // ICE 候选上行通道（回调同步线程 → 主循环异步发送）
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidate>();
    if let Err(e) = setup_pc(&pc, actuator.clone(), ice_tx).await {
        eprintln!("emergency: {e}");
        return ExitCode::from(1);
    }

    // ICE Failed 自愈（PIT-87 模式：状态收敛后退出待拉起）
    let (fail_tx, mut fail_rx) = mpsc::unbounded_channel::<()>();
    pc.on_ice_connection_state_change(move |state| {
        if state == mediaservo_webrtc::peer_connection::RTCIceConnectionState::Failed {
            tracing::error!("ICE Failed — 退出待重启（PIT-87 自愈）");
            let _ = fail_tx.send(());
        }
    });

    // 标准 offerer 协商（legacy 惯例：host 发起 offer）
    let offer = match pc.create_offer(&Default::default()).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("emergency: create_offer 失败: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = pc.set_local_description(&offer).await {
        eprintln!("emergency: set_local_description 失败: {e}");
        return ExitCode::from(1);
    }
    let offer_json = match serde_json::to_string(&offer) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("emergency: 序列化 offer 失败: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = signal
        .send(SignalingMessage::Sdp {
            room_id: ROOM.into(),
            target: None,
            sdp: offer_json,
        })
        .await
    {
        eprintln!("emergency: 发送 offer 失败: {e}");
        return ExitCode::from(1);
    }
    tracing::info!("offer 已发送（等待舱端 answer）");

    // 远端候选在 answer 落地前缓存（libwebrtc 协商前 add_ice_candidate 不可靠）
    let mut pending_ice: Vec<RTCIceCandidate> = Vec::new();
    let mut remote_set = false;
    let mut signal_events = signal.events();
    let mut exit_code: u8 = 0;

    println!(
        "emergency ready: gateway={gateway} room={ROOM} label=emergency audit={}",
        audit_path.display()
    );

    'run: loop {
        tokio::select! {
            sig = shutdown_signal() => match sig {
                Ok(()) => break 'run,
                Err(e) => {
                    eprintln!("emergency: 信号处理失败: {e}");
                    exit_code = 1;
                    break 'run;
                }
            },
            _ = fail_rx.recv() => {
                // ICE Failed：短暂等待状态收敛后退出（PIT-87）
                tokio::time::sleep(ICE_FAILED_WAIT).await;
                exit_code = 1;
                break 'run;
            }
            ev = signal_events.recv() => match ev {
                Ok(SignalEvent::Message(SignalingMessage::Sdp { sdp, .. })) => {
                    let desc = match serde_json::from_str::<RTCSessionDescription>(&sdp) {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!("answer 解析失败: {e}");
                            continue;
                        }
                    };
                    if desc.sdp_type != RTCSdpType::Answer {
                        tracing::warn!(sdp_type = %desc.sdp_type, "非 answer Sdp，忽略");
                        continue;
                    }
                    match pc.set_remote_description(&desc).await {
                        Ok(()) => {
                            tracing::info!("answer 已设置 — P2P 协商完成");
                            remote_set = true;
                            for c in pending_ice.drain(..) {
                                if let Err(e) = pc.add_ice_candidate(&c).await {
                                    tracing::warn!("add_ice_candidate: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("set_remote_description 失败: {e}");
                            exit_code = 1;
                            break 'run;
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
                    if remote_set {
                        if let Err(e) = pc.add_ice_candidate(&c).await {
                            tracing::warn!("add_ice_candidate: {e}");
                        }
                    } else {
                        pending_ice.push(c);
                    }
                }
                Ok(SignalEvent::Message(_)) => {} // RoomJoined/其他透传忽略
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
                Ok(SignalEvent::Connected { .. }) => {}
                // SignalEvent non_exhaustive 兜底
                Ok(_) => {}
                Err(_) => {
                    tracing::error!("信令事件流关闭");
                    exit_code = 1;
                    break 'run;
                }
            },
            Some(candidate) = ice_rx.recv() => {
                if let Err(e) = signal
                    .send(SignalingMessage::RTCIceCandidate {
                        room_id: ROOM.into(),
                        target: None,
                        candidate: candidate.candidate,
                        sdp_mid: candidate.sdp_mid,
                        sdp_mline_index: candidate.sdp_mline_index,
                    })
                    .await
                {
                    tracing::warn!("ICE 候选上行失败: {e}");
                }
            }
            _ = async {
                // 本地兜底（SIGUSR1）；非 unix 平台无本地兜底路径（仅 DC 急停）
                #[cfg(unix)]
                usr1.recv().await;
                #[cfg(not(unix))]
                std::future::pending::<()>().await;
            } => {
                local_trigger(actuator.as_ref()).await;
            }
        }
    }

    if let Err(e) = signal.close().await {
        tracing::warn!("close: {e}");
    }
    pc.close().await;
    tracing::info!("emergency stopped (exit={exit_code})");
    ExitCode::from(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_host::emergency::EmergencyTrigger;
    use std::path::Path;

    fn actuator(temp: &tempfile::NamedTempFile) -> StubEmergencyActuator {
        StubEmergencyActuator::new(temp.path()).unwrap()
    }

    #[test]
    fn args_from_defaults() {
        let (gw, audit) = args_from(vec![].into_iter()).unwrap();
        assert_eq!(gw, DEFAULT_GATEWAY);
        assert_eq!(audit, PathBuf::from(DEFAULT_AUDIT));
    }

    #[test]
    fn args_from_override_wins() {
        let (gw, audit) = args_from(
            vec![
                "--gateway".into(),
                "ws://127.0.0.1:18888/ws".into(),
                "--audit".into(),
                "/tmp/x.jsonl".into(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(gw, "ws://127.0.0.1:18888/ws");
        assert_eq!(audit, PathBuf::from("/tmp/x.jsonl"));
    }

    #[test]
    fn args_from_rejects_unknown_and_missing() {
        assert!(args_from(vec!["--bogus".into()].into_iter()).is_err());
        assert!(args_from(vec!["--gateway".into()].into_iter()).is_err());
        assert!(args_from(vec!["--audit".into()].into_iter()).is_err());
    }

    #[test]
    fn handle_envelope_rejects_non_stop_cmd() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = actuator(&f);
        let env = ControlEnvelope {
            seq: 9,
            cmd: "panic".into(),
            payload: serde_json::json!({}),
            sig: None,
        };
        let ack = handle_envelope(&a, &env);
        assert_eq!(ack.ack, 9);
        let err = ack.result["error"].as_str().expect("拒绝必须带 error");
        assert!(err.contains("unknown cmd"), "err: {err}");
        assert_eq!(std::fs::read_to_string(f.path()).unwrap(), "", "拒绝的命令不得触发/审计");
    }

    #[test]
    fn handle_envelope_stop_first_arms_then_holds() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = actuator(&f);
        let env = |seq| ControlEnvelope {
            seq,
            cmd: EMERGENCY_STOP.into(),
            payload: serde_json::json!({}),
            sig: None,
        };
        let ack1 = handle_envelope(&a, &env(1));
        assert_eq!(ack1.result["ok"], true);
        assert_eq!(ack1.result["source"], "dc");
        assert_eq!(ack1.result["latched"], true, "首次必须 armed");
        assert_eq!(ack1.result["trigger_count"], 1);
        assert_eq!(ack1.result["audit"], "ok");
        let ack2 = handle_envelope(&a, &env(2));
        assert_eq!(ack2.result["ok"], true);
        assert_eq!(ack2.result["latched"], false, "重复不重复 armed");
        assert_eq!(ack2.result["trigger_count"], 2);
    }

    #[test]
    fn audit_failure_surfaces_in_ack() {
        // 不可写审计（/dev/full，Linux）→ 回执 audit 带 error，但急停仍 ok
        #[cfg(target_os = "linux")]
        {
            let a = StubEmergencyActuator::new(Path::new("/dev/full")).unwrap();
            let env = ControlEnvelope {
                seq: 1,
                cmd: EMERGENCY_STOP.into(),
                payload: serde_json::json!({}),
            sig: None,
            };
            let ack = handle_envelope(&a, &env);
            assert_eq!(ack.result["ok"], true, "急停本身不受审计失败影响");
            assert!(ack.result["audit"]["error"].is_string(), "审计错误必须回执");
        }
    }

    #[test]
    fn local_trigger_uses_same_latch() {
        // 本地兜底与 DC 共享同一闩锁（EmergencyTrigger 形状一致）
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = actuator(&f);
        let t: EmergencyTrigger = a.trigger(EmergencySource::Local, None).unwrap();
        assert!(t.latched);
        assert_eq!(t.trigger_count, 1);
    }
}
