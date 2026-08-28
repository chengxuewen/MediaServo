//! host-audio: 音频会议进程（Task H2 真实实现）。
//!
//! 用法: `host-audio --room audio-<vehicle> [--gateway ws://127.0.0.1:17980/ws | --server ws://host:9800/ws --psk <psk>] [--duration <secs>]`
//!
//! 流程（D-H12: 每车一个音频房间，参与者 publish 1 路 opus + subscribe 其他所有）:
//! 1. 信令连（默认走本地网关 host-agent；`--server` 直连 server）
//! 2. Send transport → 标准 answerer 协商（audio SDP → add_track(audio) → answer）→
//!    Connect → Produce（opus，rtp_parameters 从协商结果推导，C18）
//! 3. tone 源（合成 440Hz 正弦 10ms PCM → AudioTrackSource → libwebrtc opus 编码）——
//!    stub 音频源；真实 ALSA/MMAPI 麦克风 = Phase I+（文档化后续）
//! 4. 事件循环: NewProducer（他人入会）→ Recv transport → Consume → sendonly SDP(ssrc 注入)
//!    → answer → Connect（全互连订阅）
//! 5. 周期 SfuStats 日志（媒体面证据; PIT-105: libwebrtc 音频编码不产包 — 字节>0 待修复）
//! 6. SIGTERM / `--duration` → 优雅退出 0（C15: 失败可见，部署 restart_policy 拉起）

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, MediaKind, PeerRole, SignalingMessage, TransportDirection,
};
use mediaservo_host::audio;
use mediaservo_link::{SignalClient, SignalEvent, SignalSession};
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCPeerConnectionFactory, RTCPeerConnectionState,
    RTCSdpType, RTCSessionDescription, TrackKind, TrackRef,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const STATS_INTERVAL: Duration = Duration::from_secs(5);

const USAGE: &str = "用法: host-audio --room audio-<vehicle> [--gateway <ws> | --server <ws> --psk <psk>] [--duration <secs>] [--tone <hz>]";

struct Args {
    room: String,
    gateway: Option<String>,
    server: Option<String>,
    psk: Option<String>,
    duration_secs: Option<u64>,
    tone_hz: u16,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut room = None;
    let mut gateway = None;
    let mut server = None;
    let mut psk = None;
    let mut duration_secs = None;
    let mut tone_hz = 440u16;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--room" => room = args.next(),
            "--gateway" => gateway = args.next(),
            "--server" => server = args.next(),
            "--psk" => psk = args.next(),
            "--duration" => {
                duration_secs = args.next().and_then(|v| v.parse().ok());
            }
            "--tone" => {
                tone_hz = args.next().and_then(|v| v.parse().ok()).unwrap_or(440);
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("未知参数: {other}\n{USAGE}")),
        }
    }
    let room = room.ok_or_else(|| format!("缺 --room\n{USAGE}"))?;
    if !room.starts_with("audio-") {
        return Err(format!("--room 必须是 audio-<vehicle> 形式（H2 音频房间约定），got {room}"));
    }
    Ok(Args { room, gateway, server, psk, duration_secs, tone_hz })
}

/// 等待一条指定类型的信令事件（跳过无关广播/ack）。
async fn await_event<T, F>(rx: &mut tokio::sync::broadcast::Receiver<SignalEvent>, mut f: F) -> Result<T, String>
where
    F: FnMut(SignalingMessage) -> Result<Option<T>, String>,
{
    loop {
        match rx.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::Error { message, .. }))
                if message == "transport_connected" => continue,
            Ok(SignalEvent::Message(m)) => match f(m) {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => continue,
                Err(e) => return Err(e),
            },
            Ok(SignalEvent::Disconnected { reason }) => {
                return Err(format!("信令断开: {reason}"));
            }
            Ok(SignalEvent::Error(e)) => return Err(format!("信令错误: {e}")),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err("信令通道关闭".into());
            }
            _ => continue,
        }
    }
}

/// 发布 1 路 opus: Send transport → answerer 协商 → Connect → Produce。
/// 返回 (producer_id, 本房间成员可复用的 events 接收器)。
async fn publish_audio(
    signal: &SignalSession,
    room: &str,
    tone_hz: u16,
) -> Result<
    (
        String,
        Arc<mediaservo_webrtc::track::TrackSender>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    ),
    String,
> {
    // 1. Send transport
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: room.into(),
            peer_id: signal.peer_id().into(),
            direction: TransportDirection::Send,
        })
        .await
        .map_err(|e| format!("create transport send: {e}"))?;
    let mut events = signal.events();
    let (send_tid, ice_params, dtls_params, ice_candidates) = await_event(
        &mut events,
        |m| match m {
            SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            } => Ok(Some((transport_id, ice_parameters, dtls_parameters, ice_candidates))),
            SignalingMessage::Error { code, message } => {
                Err(format!("CreateWebRtcTransport error {code}: {message}"))
            }
            _ => Ok(None),
        },
    )
    .await?;

    // 2. PC + 标准 answerer 协商（C18: remote SDP → add_track → create_answer）
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory
        .create_peer_connection(RTCConfiguration::default())
        .await
        .map_err(|e| format!("create pc: {e}"))?;
    let connected = Arc::new(tokio::sync::Notify::new());
    let connected_clone = connected.clone();
    pc.on_peer_connection_state_change(move |state| {
        if state == RTCPeerConnectionState::Connected {
            connected_clone.notify_one();
        }
    });
    let remote_sdp = audio::build_remote_audio_sdp(
        &ice_params, &dtls_params, ice_candidates.as_ref(), false, None, "audio",
    );
    let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
    pc.set_remote_description(&remote_desc)
        .await
        .map_err(|e| format!("set remote description: {e}"))?;
    // track id 必须与 libwebrtc 内部 track label 一致（create_audio_track 建 "audio"）
    let track_id = pc
        .add_track("audio", TrackKind::Audio)
        .map_err(|e| format!("add audio track: {e}"))?;
    let answer = pc
        .create_answer(&RTCAnswerOptions {})
        .await
        .map_err(|e| format!("create answer: {e}"))?;
    pc.set_local_description(&answer)
        .await
        .map_err(|e| format!("set local description: {e}"))?;

    // 3. ConnectWebRtcTransport（DTLS）
    let fp_hex = pc.local_dtls_fingerprint().ok_or("no DTLS fingerprint")?;
    signal
        .send(SignalingMessage::ConnectWebRtcTransport {
            room_id: room.into(),
            peer_id: signal.peer_id().into(),
            transport_id: send_tid.clone(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".to_string(),
                    value: fp_hex,
                }],
                role: "client".to_string(),
            },
        })
        .await
        .map_err(|e| format!("connect transport: {e}"))?;
    tokio::time::timeout(CONNECT_TIMEOUT, connected.notified())
        .await
        .map_err(|_| "producer PC connect timeout".to_string())?;
    if pc.connection_state() != RTCPeerConnectionState::Connected {
        return Err(format!("PC 未连接: {:?}", pc.connection_state()));
    }

    // 4. Produce（协商结果推导 rtp_parameters — C18 官方路径）
    let rtp_params = pc
        .get_sending_rtp_parameters(&track_id)
        .map_err(|e| format!("get_sending_rtp_parameters: {e}"))?;
    signal
        .send(SignalingMessage::Produce {
            room_id: room.into(),
            peer_id: signal.peer_id().into(),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Audio,
            rtp_parameters: audio::build_audio_produce_rtp_parameters(&rtp_params),
            // C1: 指名绑定本会话的 send transport（host 子进程共享 peer_id 防串线）
            transport_id: Some(send_tid),
        })
        .await
        .map_err(|e| format!("produce: {e}"))?;
    let producer_id = await_event(&mut events, |m| match m {
        SignalingMessage::Produced { producer_id, .. } => Ok(Some(producer_id)),
        SignalingMessage::Error { code, message } => {
            Err(format!("Produce error {code}: {message}"))
        }
        _ => Ok(None),
    })
    .await?;

    // 5. tone 推送 task（stub 音频源 — 真实麦克风 Phase I+）
    let sender = match pc.get_track(&track_id) {
        Some(TrackRef::Sender(s)) => s,
        _ => return Err("expected TrackSender".into()),
    };
    let tone_track = sender.clone();
    // PCM 推流成功计数（I4 re-review）: 周期 stats 日志 surfacing — write_frame
    // 静默失败可观测（host_audio_e2e 断言 pushed>0）。
    let pushed: std::sync::Arc<std::sync::atomic::AtomicU64> = Default::default();
    let pushed_for_task = pushed.clone();
    let tone_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let mut phase: f64 = 0.0;
        let freq = f64::from(tone_hz);
        loop {
            interval.tick().await;
            let frame = audio::tone_frame(&mut phase, freq);
            match tone_track.write_frame(&frame).await {
                Ok(()) => {
                    pushed_for_task.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::warn!("host-audio: tone write_frame failed: {e}");
                    break;
                }
            }
        }
    });
    tracing::info!(
        "host-audio: published producer {producer_id} (tone={tone_hz}Hz, PIT-105: 音频 RTP 待 libwebrtc 修复)"
    );
    // PIT-105: tone task 全生命周期存活（避免假性内存回收）— 随进程退出
    std::mem::forget(tone_task);
    Ok((producer_id, Arc::new(sender), pushed))
}

/// 订阅一路 producer: Recv transport → Consume（先拿 ssrc）→ sendonly SDP → answer → Connect。
async fn subscribe_producer(
    signal: &SignalSession,
    room: &str,
    producer_id: &str,
    consumer_ids: Arc<std::sync::Mutex<Vec<String>>>,
) {
    let peer = signal.peer_id().to_string();
    let result: Result<(), String> = async {
        // 1. Recv transport
        signal
            .send(SignalingMessage::CreateWebRtcTransport {
                room_id: room.into(),
                peer_id: signal.peer_id().into(),
                direction: TransportDirection::Recv,
            })
            .await
            .map_err(|e| format!("create transport recv: {e}"))?;
        let mut events = signal.events();
        let (recv_tid, ice_params, dtls_params, ice_candidates) = await_event(
            &mut events,
            |m| match m {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    ice_parameters,
                    dtls_parameters,
                    ice_candidates,
                    ..
                } => Ok(Some((transport_id, ice_parameters, dtls_parameters, ice_candidates))),
                SignalingMessage::Error { code, message } => {
                    Err(format!("CreateWebRtcTransport(recv) error {code}: {message}"))
                }
                _ => Ok(None),
            },
        )
        .await?;

        // 2. Consume 先于 SDP（拿 consumer ssrc 注入 remote SDP — PullSession 顺序）
        signal
            .send(SignalingMessage::Consume {
                room_id: room.into(),
                peer_id: signal.peer_id().into(),
                producer_id: producer_id.to_string(),
                rtp_capabilities: serde_json::json!({
                    "codecs": [{"mimeType": "audio/opus", "clockRate": 48000, "kind": "audio", "channels": 2}],
                    "headerExtensions": [
                        {"uri": "urn:ietf:params:rtp-hdrext:sdes:mid", "preferredId": 1, "kind": "audio", "preferredEncrypt": false, "direction": "sendrecv"},
                        {"uri": "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01", "preferredId": 3, "kind": "audio", "preferredEncrypt": false, "direction": "sendrecv"},
                    ],
                }),
                // C1: 指名绑定本会话的 recv transport（防多连接共享 peer_id 串线）
                transport_id: Some(recv_tid.clone()),
            })
            .await
            .map_err(|e| format!("consume: {e}"))?;
        let (consumer_id, consumer_rtp) = await_event(&mut events, |m| match m {
            SignalingMessage::Consumed { consumer_id, rtp_parameters, .. } => {
                Ok(Some((consumer_id, rtp_parameters)))
            }
            SignalingMessage::Error { code, message } => {
                Err(format!("Consume error {code}: {message}"))
            }
            _ => Ok(None),
        })
        .await?;
        let ssrc = consumer_rtp
            .get("encodings")
            .and_then(|e| e.as_array())
            .and_then(|a| a.first())
            .and_then(|enc| enc.get("ssrc"))
            .and_then(|s| s.as_u64());
        let mid = consumer_rtp
            .get("mid")
            .and_then(|m| m.as_str())
            .unwrap_or("0")
            .to_string();

        // 3. PC + recvonly audio transceiver → sendonly SDP（含 ssrc）→ answer
        let factory = RTCPeerConnectionFactory::new();
        let pc = factory
            .create_peer_connection(RTCConfiguration::default())
            .await
            .map_err(|e| format!("create pc: {e}"))?;
        let connected = Arc::new(tokio::sync::Notify::new());
        let connected_clone = connected.clone();
        pc.on_peer_connection_state_change(move |state| {
            if state == RTCPeerConnectionState::Connected {
                connected_clone.notify_one();
            }
        });
        pc.add_transceiver(
            TrackKind::Audio,
            RTCRtpTransceiverInit {
                direction: mediaservo_webrtc::rtp::RTCRtpTransceiverDirection::Recvonly,
                ..Default::default()
            },
        )
        .map_err(|e| format!("add_transceiver: {e}"))?;
        let remote_sdp = audio::build_remote_audio_sdp(
            &ice_params,
            &dtls_params,
            ice_candidates.as_ref(),
            true,
            ssrc,
            &mid,
        );
        let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
        pc.set_remote_description(&remote_desc)
            .await
            .map_err(|e| format!("set remote description: {e}"))?;
        let answer = pc
            .create_answer(&RTCAnswerOptions {})
            .await
            .map_err(|e| format!("create answer: {e}"))?;
        pc.set_local_description(&answer)
            .await
            .map_err(|e| format!("set local description: {e}"))?;

        // 4. ConnectWebRtcTransport（DTLS — mediasoup 转发前提）
        let fp_hex = pc.local_dtls_fingerprint().ok_or("no DTLS fingerprint")?;
        signal
            .send(SignalingMessage::ConnectWebRtcTransport {
                room_id: room.into(),
                peer_id: signal.peer_id().into(),
                transport_id: recv_tid,
                dtls_parameters: DtlsParameters {
                    fingerprints: vec![Fingerprint {
                        algorithm: "sha-256".to_string(),
                        value: fp_hex,
                    }],
                    role: "client".to_string(),
                },
            })
            .await
            .map_err(|e| format!("connect recv transport: {e}"))?;
        tokio::time::timeout(CONNECT_TIMEOUT, connected.notified())
            .await
            .map_err(|_| "consumer PC connect timeout".to_string())?;

        tracing::info!("host-audio: subscribed {producer_id} → consumer {consumer_id} (ssrc={ssrc:?})");
        consumer_ids.lock().unwrap().push(consumer_id);
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("host-audio({peer}): subscribe {producer_id} 失败: {e}");
    }
}

#[tokio::main]
async fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    mediaservo_host::init_logging("audio");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return Ok(ExitCode::from(2));
        }
    };

    // 信令连接（网关模式 = 整车会话子进程; --server = 直连）
    let signal = match (&args.gateway, &args.server) {
        (Some(gw), _) => {
            SignalClient::new_gateway(gw, "audio", &args.room, PeerRole::Consumer)
                .connect()
                .await
        }
        (_, Some(srv)) => {
            let psk = args.psk.clone().ok_or("直连模式需要 --psk")?;
            SignalClient::new(srv, &psk, &args.room, PeerRole::Consumer)
                .connect()
                .await
        }
        _ => {
            // 缺省: 本地网关（D2 整车架构）
            SignalClient::new_gateway("ws://127.0.0.1:17980/ws", "audio", &args.room, PeerRole::Consumer)
                .connect()
                .await
        }
    };
    let signal = match signal {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!("host-audio: 信令连接失败: {e}");
            return Ok(ExitCode::from(1));
        }
    };
    tracing::info!("host-audio: 已加入音频房间 {}", args.room);

    // 发布 1 路 opus
    let (producer_id, _sender, pushed) =
        match publish_audio(&signal, &args.room, args.tone_hz).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("host-audio: publish 失败: {e}");
            if let Ok(s) = Arc::try_unwrap(signal) {
                let _ = s.close().await;
            }
            return Ok(ExitCode::from(1));
        }
    };

    // 事件循环 + 周期统计 + 退出信号
    let consumer_ids: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let consumer_ids_for_events = consumer_ids.clone();
    let room = args.room.clone();
    let signal_for_events = signal;
    let pid = producer_id.clone();
    let consumer_ids_for_stats = consumer_ids.clone();

    let stats_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(STATS_INTERVAL);
        loop {
            interval.tick().await;
            let own_consumers = consumer_ids_for_stats.lock().unwrap().len();
            let pushed_n = pushed.load(std::sync::atomic::Ordering::Relaxed);
            tracing::info!(
                "host-audio: producer={pid} consumers={own_consumers} pushed_pcm={pushed_n} (PIT-105: RTP 字节统计待 libwebrtc 音频编码修复后生效)"
            );
        }
    });

    let mut events = signal_for_events.events();
    let mut subscribed = std::collections::HashSet::new();
    let mut shutdown = false;

    // SIGTERM/SIGINT
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut duration_wait = args
        .duration_secs
        .map(|d| Box::pin(tokio::time::sleep(Duration::from_secs(d))));

    loop {
        tokio::select! {
            ev = events.recv() => {
                match ev {
                    Ok(SignalEvent::Message(SignalingMessage::NewProducer { producer_id: np, .. })) => {
                        if np == producer_id || subscribed.contains(&np) {
                            continue;
                        }
                        subscribed.insert(np.clone());
                        let signal = signal_for_events.clone();
                        let room = room.clone();
                        let cids = consumer_ids_for_events.clone();
                        tokio::spawn(async move {
                            subscribe_producer(&signal, &room, &np, cids).await;
                        });
                    }
                    Ok(SignalEvent::Disconnected { reason }) => {
                        tracing::warn!("host-audio: 信令断开: {reason}");
                        break;
                    }
                    // H6: 上游切换（gateway 重连后通知/宕机期被动 5001）——会话不可信，
                    // 退出重启重建（与 Disconnected 同通道；server 就绪后一发即中）。
                    Ok(SignalEvent::Message(SignalingMessage::Error { code: 5001, .. })) => {
                        tracing::warn!("host-audio: 上游切换（网关 5001）— 退出重启重建会话");
                        break;
                    }
                    Ok(SignalEvent::Message(SignalingMessage::SfuStats { .. })) => {}
                    _ => {}
                }
            },
            _ = sigterm.recv() => { tracing::info!("host-audio: SIGTERM 收到，优雅退出"); shutdown = true; },
            _ = sigint.recv() => { tracing::info!("host-audio: SIGINT 收到，优雅退出"); shutdown = true; },
            _ = async {
                match &mut duration_wait {
                    Some(sleep) => sleep.as_mut().await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                tracing::info!("host-audio: --duration 到期，退出");
                shutdown = true;
            },
        }
        if shutdown {
            break;
        }
    }

    stats_task.abort();
    Arc::try_unwrap(signal_for_events)
        .map_err(|_| "session still shared")?
        .close()
        .await?;
    tracing::info!("host-audio: 已退出 (code 0)");
    Ok(ExitCode::SUCCESS)
}

