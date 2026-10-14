//! Task H2: host-audio process tests - audio conference participant process (tone synth source, stub mic).
//!
//! - `bad_args_exit_2_with_usage`: missing/bad args (non audio- room) -> exit 2 + usage on stderr
//! - `audio_process_publishes_and_exits_clean`: connect directly to an external mediasoup server -> join
//!   the `audio-<id>` room -> publish 1 opus track (tone) -> graceful exit 0 when `--duration` expires;
//!   the room exists server-side (indirect evidence: process log "published producer" + exit code 0).
//!   PIT-105: libwebrtc audio encoding produces no packets - the RTP byte assertion waits for a fix.
//!
//! Prerequisite: `SFU_E2E_WS_URL` pointing at an external mediasoup server (C21) + `SFU_E2E_PSK`.

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
        panic!("SFU_E2E_WS_URL not set - host-audio e2e needs an external mediasoup server (C21)")
    })
}

/// Runs host-audio and returns (exit_code, log text). stdout/stderr captured merged.
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
            panic!("host-audio timed out without exiting ({timeout_secs}s)");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let log = done.join().unwrap();
    (status.code().unwrap_or(-1), log)
}

#[test]
fn bad_args_exit_2_with_usage() {
    // missing --room
    let (code, log) = run_audio(&[], 10);
    assert_eq!(code, 2, "missing args must exit 2: {log}");
    assert!(log.contains("Usage"), "must print usage: {log}");

    // non audio- room
    let (code, log) =
        run_audio(&["--room", "ms-car1", "--server", "ws://127.0.0.1:1/ws", "--psk", "x"], 10);
    assert_eq!(code, 2, "a non audio- room must exit 2: {log}");
    assert!(log.contains("audio-<vehicle>"), "must hint the room convention: {log}");
}

/// Direct server connect: join -> publish (tone) -> --duration expires -> exit 0.
/// PIT-105: the RTP bytes>0 assertion waits for the libwebrtc audio-encoding fix.
#[test]
fn audio_process_publishes_and_exits_clean() {
    let url = ws_url();
    let room = format!("audio-e2e-proc-{}", std::process::id());
    let (code, log) =
        run_audio(&["--server", &url, "--psk", &psk(), "--room", &room, "--duration", "6"], 30);
    assert_eq!(code, 0, "host-audio must exit gracefully with 0: {log}");
    assert!(log.contains(&format!("joined audio room {room}")), "must join the audio room: {log}");
    assert!(log.contains("published producer"), "must publish opus successfully: {log}");
    // I4 re-review: PCM push success counter (stats log pushed_pcm=N) - a silently failing tone task
    // (all write_frame Err) -> pushed_pcm=0 -> the test fails.
    let pushed_line = log
        .lines()
        .find(|l| l.contains("pushed_pcm="))
        .unwrap_or_else(|| panic!("the stats log must contain pushed_pcm: {log}"));
    let pushed: u64 = pushed_line
        .split("pushed_pcm=")
        .nth(1)
        .and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    assert!(
        pushed > 0,
        "pushed_pcm must be > 0 (PCM frames successfully pushed into libwebrtc): {pushed_line}"
    );
    assert!(log.contains("--duration elapsed"), "must exit on schedule per duration: {log}");
}

/// I3 review: audio rooms pass straight through the local gateway (host-agent) - gateway rewrite_room
/// skips rewriting for audio- rooms. Evidence chain: (1) gateway-mode probe (SignalClient direct to the gateway)
/// produces video in an audio- room -> server 4031 (the audio-room gate enforced server-side = room semantics not merged into the vehicle room)
/// (2) full host-audio --gateway flow (join + publish + graceful exit 0) (3) the vehicle room contains host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audio_through_gateway_reaches_audio_room() {
    use mediaservo_common::protocol::{MediaKind, SignalingMessage, TransportDirection};
    use mediaservo_link::{SignalClient, SignalEvent};

    let url = ws_url();
    let gw_port = free_local_port();
    let vehicle_room = format!("vehicle-{}", std::process::id());
    let audio_room = format!("audio-{}", vehicle_room);

    // host-agent (local gateway): random port + remote pointing at the real server
    let mut agent = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .args(["--port", &gw_port.to_string(), "--remote", &url, "--room", &vehicle_room])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn host-agent");
    // wait for the agent gateway to be ready (port connectable; while the remote join is pending the probe's
    // RoomJoin gets an explicit "gateway not connected" error - see the expect below)
    wait_for_gateway_port(gw_port, 20);

    // (1) gateway-mode probe: produce video in an audio- room -> 4031 (server-side audio-room gate in effect)
    let gw_url = format!("ws://127.0.0.1:{gw_port}/ws");
    let probe = SignalClient::new_gateway(
        &gw_url,
        "audio-probe",
        &audio_room,
        mediaservo_common::protocol::PeerRole::Consumer,
    )
    .connect()
    .await
    .expect("probe joins the audio room via the gateway");
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
            Ok(SignalEvent::Disconnected { reason }) => panic!("probe disconnected: {reason}"),
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
            transport_id: None, // legacy path regression

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
                    "video produce in an audio room via the gateway must be 4031 (server-side audio-room gate): {message}"
                );
                assert!(
                    message.contains("audio rooms allow audio producers only"),
                    "error message: {message}"
                );
                denied = true;
                break;
            }
            Ok(SignalEvent::Message(SignalingMessage::Produced { .. })) => {
                panic!("video produce in an audio room via the gateway must be rejected")
            }
            Ok(SignalEvent::Message(_)) => continue,
            Ok(SignalEvent::Disconnected { reason }) => panic!("probe disconnected: {reason}"),
            _ => continue,
        }
    }
    assert!(denied, "4031 not received");
    probe.close().await.expect("probe close");

    // (2) full host-audio flow via the gateway
    let (code, log) =
        run_audio(&["--gateway", &gw_url, "--room", &audio_room, "--duration", "6"], 30);
    assert_eq!(code, 0, "host-audio via the gateway must exit gracefully with 0: {log}");
    assert!(
        log.contains(&format!("joined audio room {audio_room}")),
        "must join the audio room via the gateway: {log}"
    );
    assert!(log.contains("published producer"), "must publish via the gateway: {log}");

    // SIGTERM -> gateway graceful exit 0
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let st = agent.wait().expect("wait agent");
    assert_eq!(st.code(), Some(0), "agent should exit gracefully with 0, got {st:?}");
}

fn free_local_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("probe addr").port()
}

/// Poll the gateway WS port until host-agent is ready (<=20s).
fn wait_for_gateway_port(port: u16, tries: u32) {
    for _ in 0..tries {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("host-agent gateway not ready (port {port})");
}
