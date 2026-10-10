//! Task H2: host-audio 进程测试 — 音频会议参与者进程（tone 合成源，stub 麦克风）。
//!
//! - `bad_args_exit_2_with_usage`: 缺参/坏参（非 audio- 房间）→ exit 2 + stderr 用法
//! - `audio_process_publishes_and_exits_clean`: 直连外部 mediasoup server → 加入
//!   `audio-<id>` 房间 → publish 1 路 opus（tone）→ `--duration` 到期优雅退出 0；
//!   server 侧房间存在（间接证据: 进程日志 published producer + 退出码 0）。
//!   PIT-105: libwebrtc 音频编码不产包 — RTP 字节断言待修复后启用。
//!
//! 前置: `SFU_E2E_WS_URL` 指向外部 mediasoup server（C21）+ `SFU_E2E_PSK`。

#![cfg(target_os = "linux")]

use std::process::{Command, Stdio};
use std::time::Duration;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_host-audio")
}

fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "e2e-host-sfu-psk".to_string())
}

fn ws_url() -> String {
    std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| {
        panic!("SFU_E2E_WS_URL 未设置 — host-audio e2e 需连外部 mediasoup server (C21)")
    })
}

/// 跑 host-audio 并返回 (exit_code, 日志文本)。stdout/stderr 合并捕获。
fn run_audio(args: &[&str], timeout_secs: u64) -> (i32, String) {
    let mut child = Command::new(bin())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn host-audio");
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let done = std::thread::spawn(move || {
        use std::io::Read;
        let mut out_s = String::new();
        let mut err_s = String::new();
        let _ = out.read_to_string(&mut out_s);
        let _ = err.read_to_string(&mut err_s);
        format!("{out_s}{err_s}")
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("host-audio 超时未退出（{timeout_secs}s）");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let log = done.join().unwrap();
    (status.code().unwrap_or(-1), log)
}

#[test]
fn bad_args_exit_2_with_usage() {
    // 缺 --room
    let (code, log) = run_audio(&[], 10);
    assert_eq!(code, 2, "缺参必须 exit 2: {log}");
    assert!(log.contains("用法"), "必须输出用法: {log}");

    // 非 audio- 房间
    let (code, log) =
        run_audio(&["--room", "ms-car1", "--server", "ws://127.0.0.1:1/ws", "--psk", "x"], 10);
    assert_eq!(code, 2, "非 audio- 房间必须 exit 2: {log}");
    assert!(log.contains("audio-<vehicle>"), "必须提示房间约定: {log}");
}

/// 直连 server: join → publish（tone）→ --duration 到期 → exit 0。
/// PIT-105: RTP 字节>0 断言待 libwebrtc 音频编码修复后启用。
#[test]
fn audio_process_publishes_and_exits_clean() {
    let url = ws_url();
    let room = format!("audio-e2e-proc-{}", std::process::id());
    let (code, log) =
        run_audio(&["--server", &url, "--psk", &psk(), "--room", &room, "--duration", "6"], 30);
    assert_eq!(code, 0, "host-audio 必须优雅退出 0: {log}");
    assert!(log.contains(&format!("已加入音频房间 {room}")), "必须加入音频房间: {log}");
    assert!(log.contains("published producer"), "必须成功 publish opus: {log}");
    // I4 re-review: PCM 推流成功计数（stats 日志 pushed_pcm=N）— tone 任务静默
    // 失败（write_frame 全 Err）→ pushed_pcm=0 → 测试失败。
    let pushed_line = log
        .lines()
        .find(|l| l.contains("pushed_pcm="))
        .unwrap_or_else(|| panic!("stats 日志必须含 pushed_pcm: {log}"));
    let pushed: u64 = pushed_line
        .split("pushed_pcm=")
        .nth(1)
        .and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    assert!(pushed > 0, "pushed_pcm 必须 > 0（PCM 帧成功推入 libwebrtc）: {pushed_line}");
    assert!(log.contains("--duration 到期"), "必须按 duration 退出: {log}");
}

/// I3 review: 音频房间经本地网关（host-agent）直通 — 网关 rewrite_room 对 audio-
/// 房间跳过重写。证据链: ① 网关模式 probe（SignalClient 直连网关）在 audio- 房间
/// produce 视频 → server 4031（音频房间门在服务端生效 = 房间语义未被并入整车房间）
/// ② host-audio --gateway 全流程（join + publish + 优雅退出 0）③ 整车房间含 host。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audio_through_gateway_reaches_audio_room() {
    use mediaservo_common::protocol::{MediaKind, SignalingMessage, TransportDirection};
    use mediaservo_link::{SignalClient, SignalEvent};

    let url = ws_url();
    let gw_port = free_local_port();
    let vehicle_room = format!("vehicle-{}", std::process::id());
    let audio_room = format!("audio-{}", vehicle_room);

    // host-agent（本地网关）: 随机端口 + 远端指真 server
    let mut agent = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .args(["--port", &gw_port.to_string(), "--remote", &url, "--room", &vehicle_room])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn host-agent");
    // 等 agent 网关就绪（端口可连; 远端 join 未完成时 probe 的
    // RoomJoin 会收到 "gateway not connected" 明确报错 — 见下方 expect）
    wait_for_gateway_port(gw_port, 20);

    // ① 网关模式 probe: audio- 房间 produce 视频 → 4031（服务端音频房间门生效）
    let gw_url = format!("ws://127.0.0.1:{gw_port}/ws");
    let probe = SignalClient::new_gateway(
        &gw_url,
        "audio-probe",
        &audio_room,
        mediaservo_common::protocol::PeerRole::Consumer,
    )
    .connect()
    .await
    .expect("probe 经网关 join 音频房间");
    probe
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: audio_room.clone(),
            peer_id: probe.peer_id().into(),
            direction: TransportDirection::Send,
        })
        .await
        .expect("probe create transport");
    let mut events = probe.events();
    let transport_ok = loop {
        match events.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::WebRtcTransportCreated { .. })) => {
                break true;
            }
            Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                panic!("probe transport error {code}: {message}")
            }
            Ok(SignalEvent::Disconnected { reason }) => panic!("probe 断开: {reason}"),
            _ => continue,
        }
    };
    assert!(transport_ok);
    probe
        .send(SignalingMessage::Produce {
            room_id: audio_room.clone(),
            peer_id: probe.peer_id().into(),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Video,
            rtp_parameters: serde_json::json!({
                "mid": "0",
                "codecs": [{"mimeType": "video/VP8", "payloadType": 96, "clockRate": 90000}],
                "headerExtensions": [],
                "encodings": [{"ssrc": 12345}],
                "rtcp": {"reducedSize": true}
            }),
            transport_id: None, // legacy 路径回归

            label: None,
        })
        .await
        .expect("probe produce");
    let mut denied = false;
    for _ in 0..10 {
        match events.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                assert_eq!(
                    code, 4031,
                    "经网关的音频房间视频 produce 必须 4031（服务端音频房间门）: {message}"
                );
                assert!(
                    message.contains("audio rooms allow audio producers only"),
                    "错误信息: {message}"
                );
                denied = true;
                break;
            }
            Ok(SignalEvent::Message(SignalingMessage::Produced { .. })) => {
                panic!("经网关的音频房间视频 produce 必须被拒")
            }
            Ok(SignalEvent::Message(_)) => continue,
            Ok(SignalEvent::Disconnected { reason }) => panic!("probe 断开: {reason}"),
            _ => continue,
        }
    }
    assert!(denied, "未收到 4031");
    probe.close().await.expect("probe close");

    // ② host-audio 经网关全流程
    let (code, log) =
        run_audio(&["--gateway", &gw_url, "--room", &audio_room, "--duration", "6"], 30);
    assert_eq!(code, 0, "host-audio 经网关必须优雅退出 0: {log}");
    assert!(log.contains(&format!("已加入音频房间 {audio_room}")), "经网关必须加入音频房间: {log}");
    assert!(log.contains("published producer"), "经网关必须 publish: {log}");

    // SIGTERM → 网关优雅退出 0
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let st = agent.wait().expect("wait agent");
    assert_eq!(st.code(), Some(0), "agent 应优雅退出 0, got {st:?}");
}

fn free_local_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("probe addr").port()
}

/// 轮询网关 WS 端口直到 host-agent 就绪（≤20s）。
fn wait_for_gateway_port(port: u16, tries: u32) {
    for _ in 0..tries {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("host-agent 网关未就绪 (port {port})");
}
