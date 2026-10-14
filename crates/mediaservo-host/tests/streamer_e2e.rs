//! Task C2+D2: host-streamer process tests - FrameBus subscribe -> push through the local gateway (external mediasoup server).
//!
//! - `bad_args_exit_2_with_usage`: missing/bad args -> exit 2 + usage hint on stderr
//! - `streamer_pushes_through_gateway_to_server`: host-agent (real process, local gateway)
//!   + capturer + streamer -> the external Docker server receives the stream (streamer log
//!   `bytes_sent>0 and frames_encoded>0`, D4 evidence mode), and the server
//!   sees exactly one vehicle session (admin API room list = the only vehicle
//!   room containing host, no stream- rooms) -> SIGTERM: all three processes exit gracefully with 0
//!
//! Prerequisite: `SFU_E2E_WS_URL` pointing at an external mediasoup server (C21 pure external mode, no
//! server type import); C25: clean `/tmp/iceoryx2` + `/dev/shm/iox2_*` before running.

#![allow(clippy::doc_lazy_continuation)]
// header-doc list continuation ` + capturer` shape = prose, not a syntactic list; no per-space fussing
#![cfg(target_os = "linux")]

use std::io::Read;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, NodeAcl, NodeId, Role, TokenFile,
};

// Test Ed25519 keypair (generated with openssl, test-only; same source as link/deck/capturer tests).
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: clean iceoryx2 0.9.3 runtime residue (/tmp/iceoryx2 + /dev/shm/iox2_*).
fn cleanup_iceoryx() {
    let _ = std::fs::remove_dir_all("/tmp/iceoryx2");
    if let Ok(entries) = std::fs::read_dir("/dev/shm") {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("iox2_") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn ws_url() -> String {
    std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| {
        panic!(
            "SFU_E2E_WS_URL not set - streamer e2e needs an external mediasoup server (C21);
            e.g.: SFU_E2E_WS_URL=ws://127.0.0.1:9800/ws"
        )
    })
}

/// Serialize the two real-server-room e2es inside the same binary (D2 single stream / D3 dual streams): concurrent runs would
/// collide on the "vehicle" room (the server's single host slot, join_full_host_slot_errors) and on the
/// camera/cam0 topic (FrameBus single publisher, TopicConflict).
static ROOM_E2E_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn bad_args_exit_2_with_usage() {
    for args in [
        vec![],                 // all missing
        vec!["--stream"],       // missing value
        vec!["--stream", "s0"], // missing config/token
        vec!["--bogus", "x"],   // unknown arg
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_host-streamer"))
            .args(&args)
            .output()
            .expect("spawn host-streamer");
        assert_eq!(out.status.code(), Some(2), "args {args:?} should exit 2");
        assert!(
            !out.stderr.is_empty(),
            "args {args:?} stderr should show usage, got: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// I1 review: fps != 30 must be rejected at startup (the push encoder is built for 30fps, a PIT-64-class
/// rate-control mismatch). Pure config-validation path - the fps check precedes token/signaling, no server needed.
#[test]
fn rejects_non_30_fps_with_clear_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 25\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n",
    )
    .expect("write host.yaml");
    let tok_path = dir.path().join("t.token");
    std::fs::write(&tok_path, b"garbage").expect("write token");
    let out = Command::new(env!("CARGO_BIN_EXE_host-streamer"))
        .args([
            "--stream",
            "s0",
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            tok_path.to_str().expect("token utf8"),
        ])
        .output()
        .expect("spawn host-streamer");
    assert_eq!(out.status.code(), Some(1), "fps=25 should exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("fps"), "should name the fps field, got: {stderr}");
    assert!(stderr.contains("25"), "should contain the actual value 25, got: {stderr}");
}

/// Read the child process log (stdout+stderr merged into one file).
fn read_log(file: &tempfile::NamedTempFile) -> String {
    let mut out = String::new();
    file.reopen().expect("reopen log").read_to_string(&mut out).expect("read log");
    out
}

/// Poll the log until the needle appears (<=10s).
fn wait_for(log: &tempfile::NamedTempFile, needle: &str) {
    for _ in 0..20 {
        if read_log(log).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("needle {needle:?} not seen, log:\n{}", read_log(log));
}

/// Poll the log until the stats line has bytes_sent>0 and frames_encoded>0 (<=30s, D4 evidence mode).
fn wait_for_flow(log: &tempfile::NamedTempFile) -> String {
    for _ in 0..60 {
        for line in read_log(log).lines() {
            let Some(rest) = line.split("streamer stats:").nth(1) else {
                continue;
            };
            let bytes: u64 = rest
                .split_whitespace()
                .find_map(|t| t.strip_prefix("bytes_sent="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let frames: u64 = rest
                .split_whitespace()
                .find_map(|t| t.strip_prefix("frames_encoded="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if bytes > 0 && frames > 0 {
                return format!("bytes_sent={bytes} frames_encoded={frames}");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("no stream evidence within 30s, log:\n{}", read_log(log));
}

/// Gateway host port (host.toml [signaling] local_port separated for the test).
fn free_local_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("probe addr").port()
}

/// Admin API room list - since H3 the admin REST enforces JWT (auth_middleware),
/// logging in with the dev admin account from accounts.yaml to get the token (under the I2 review guard
/// the dev compose path is explicitly exempted).
fn admin_rooms() -> serde_json::Value {
    let port = ws_url()
        .trim_start_matches("ws://")
        .trim_end_matches("/ws")
        .split(':')
        .nth(1)
        .unwrap_or("9800")
        .parse::<u16>()
        .unwrap_or(9800);
    use std::io::{Read, Write};
    // 1. login (dev account admin/admin123 - accounts.yaml)
    let mut login = std::net::TcpStream::connect(("127.0.0.1", port)).expect("login connect");
    let login_body = r#"{"username":"admin","password":"admin123"}"#;
    let login_req = format!(
        "POST /api/auth/login HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        login_body.len(),
        login_body
    );
    login.write_all(login_req.as_bytes()).expect("login req");
    let mut login_buf = String::new();
    login.read_to_string(&mut login_buf).expect("login read");
    let login_json = login_buf.split("\r\n\r\n").nth(1).expect("login body");
    let token: String = serde_json::from_str::<serde_json::Value>(login_json)
        .expect("login json")
        .get("token")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .expect("login token");
    // 2. room list (Bearer)
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("admin connect");
    let req = format!(
        "GET /api/admin/rooms HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).expect("admin req");
    let mut buf = String::new();
    stream.read_to_string(&mut buf).expect("admin read");
    let body = buf.split("\r\n\r\n").nth(1).expect("admin body");
    serde_json::from_str(body).expect("admin json")
}

/// E2E (D2): host-agent (local gateway) + capturer + streamer(--gateway)
/// -> the external mediasoup server receives the stream, and exactly one vehicle session.
#[tokio::test]
async fn streamer_pushes_through_gateway_to_server() {
    let _guard = ROOM_E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    cleanup_iceoryx();
    let _url = ws_url();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let stream_id = format!("s{pid}-stream");

    // host.yaml: cam0 generator 30fps + a unique stream (explicit source reference, exercising the non-default path)
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n\
             streams:\n  - id: \"{stream_id}\"\n    source: \"cam0\"\n    codec: \"vp8\"\n"
        ),
    )
    .expect("write host.yaml");

    // tokens: capturer=Capture (may publish camera/*), streamer=Recorder (may subscribe camera/*)
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam0.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
    let (str_tok, str_vk) = token(Role::Recorder, &format!("streamer-{pid}"));
    let str_path = dir.path().join("streamer.token");
    std::fs::write(&str_path, TokenFile::encode(&str_tok, &str_vk)).expect("write str token");

    // host-agent (local gateway): random port + remote pointing at the real server
    let gw_port = free_local_port();
    let agent_log = tempfile::NamedTempFile::new().expect("agent log");
    let mut agent = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .args(["--port", &gw_port.to_string(), "--remote", &ws_url(), "--room", "vehicle"])
        .stdout(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .stderr(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .spawn()
        .expect("spawn host-agent");
    // wait for the agent to join the vehicle room (otherwise the streamer's RoomJoin is intercepted with 5001)
    wait_for(&agent_log, "agent joined the vehicle room");

    // capturer process (started first: the streamer's first-frame gate depends on the publisher)
    let cap_log = tempfile::NamedTempFile::new().expect("cap log");
    let mut capturer = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
        .args([
            "--camera",
            "cam0",
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            cap_path.to_str().expect("cap token utf8"),
        ])
        .stdout(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .stderr(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .spawn()
        .expect("spawn host-capturer");
    wait_for(&cap_log, "capturer ready");

    // streamer process -> the local gateway
    let str_log = tempfile::NamedTempFile::new().expect("str log");
    let mut streamer = Command::new(env!("CARGO_BIN_EXE_host-streamer"))
        .args([
            "--stream",
            &stream_id,
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            str_path.to_str().expect("str token utf8"),
            "--gateway",
            &format!("ws://127.0.0.1:{gw_port}/ws"),
        ])
        .stdout(Stdio::from(str_log.reopen().expect("reopen str log")))
        .stderr(Stdio::from(str_log.reopen().expect("reopen str log")))
        .spawn()
        .expect("spawn host-streamer");
    wait_for(&str_log, "streamer ready");

    // stream evidence: outbound stats bytes_sent>0 and frames_encoded>0 (the server has received frames)
    let evidence = wait_for_flow(&str_log);
    eprintln!("[streamer_e2e] stream evidence: {evidence}");

    // one vehicle one session: the server room list should hold only vehicle (with host), no stream- rooms
    let rooms = admin_rooms();
    let rooms_arr = rooms["rooms"].as_array().expect("rooms array");
    eprintln!("[streamer_e2e] admin rooms: {rooms}");
    assert!(
        rooms_arr.iter().any(|r| r["id"] == "vehicle" && r["host"].is_string()),
        "the server should show the vehicle room + host, got {rooms}"
    );
    assert!(
        !rooms_arr.iter().any(|r| r["id"].as_str().is_some_and(|id| id.starts_with("stream-"))),
        "the streamer's RoomJoin should be intercepted by the gateway (no stream-* rooms), got {rooms}"
    );

    // SIGTERM -> all three processes exit gracefully with 0
    unsafe { libc::kill(streamer.id() as i32, libc::SIGTERM) };
    let st = streamer.wait().expect("wait streamer");
    assert_eq!(st.code(), Some(0), "the streamer should exit gracefully with 0, got {st:?}");
    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "the capturer should exit gracefully with 0, got {ct:?}");
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let at = agent.wait().expect("wait agent");
    assert_eq!(at.code(), Some(0), "agent should exit gracefully with 0, got {at:?}");
}

/// E2E (D3): two push streams through one gateway session - host-agent + capturer x2 (cam0/cam1)
/// + streamer x2 (s0/s1, same --gateway) -> the external mediasoup server:
/// (1) both streams outbound with bytes_sent>0 and frames_encoded>0 (D4 evidence mode)
/// (2) the admin room list = the unique vehicle room (with host), no stream-* rooms
/// (3) both streamers + both capturers stay alive throughout
/// (4) SIGTERM -> all five processes exit gracefully with 0
///
/// D-H6 "multi-path produce = same peer, multiple transports": each streamer reaches the server over the same gateway's
/// single WS; their Create/Connect/Produce pair in order in the gateway FIFO.
#[tokio::test]
async fn two_streamers_share_one_vehicle_session() {
    let _guard = ROOM_E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    cleanup_iceoryx();
    let _url = ws_url();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let s0 = format!("s{pid}-0");
    let s1 = format!("s{pid}-1");

    // host.yaml: cam0/cam1 generator 30fps + two streams (each with an explicit source reference)
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n\
              - id: \"cam1\"\n    mode: \"generator\"\n    fps: 30\n\
             streams:\n  - id: \"{s0}\"\n    source: \"cam0\"\n    codec: \"vp8\"\n\
              - id: \"{s1}\"\n    source: \"cam1\"\n    codec: \"vp8\"\n"
        ),
    )
    .expect("write host.yaml");

    // tokens: capturer=Capture (camera/*), streamer=Recorder (camera/*) - independent node ids per process
    let mut tok_paths = Vec::new();
    for (role, node) in [
        (Role::Capture, format!("capture-{pid}-cam0")),
        (Role::Capture, format!("capture-{pid}-cam1")),
        (Role::Recorder, format!("streamer-{pid}-s0")),
        (Role::Recorder, format!("streamer-{pid}-s1")),
    ] {
        let (tok, vk) = token(role, &node);
        let p = dir.path().join(format!("{node}.token"));
        std::fs::write(&p, TokenFile::encode(&tok, &vk)).expect("write token");
        tok_paths.push(p);
    }

    // host-agent (local gateway): random port + remote pointing at the real server
    let gw_port = free_local_port();
    let agent_log = tempfile::NamedTempFile::new().expect("agent log");
    let mut agent = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .args(["--port", &gw_port.to_string(), "--remote", &ws_url(), "--room", "vehicle"])
        .stdout(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .stderr(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .spawn()
        .expect("spawn host-agent");
    wait_for(&agent_log, "agent joined the vehicle room");

    // capturer x2 (started first: the streamers' first-frame gate depends on the publishers)
    let mut capturers = Vec::new();
    let mut cap_logs = Vec::new();
    for (i, cam) in ["cam0", "cam1"].iter().enumerate() {
        let cap_log = tempfile::NamedTempFile::new().expect("cap log");
        let capturer = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
            .args([
                "--camera",
                cam,
                "--config",
                cfg_path.to_str().expect("cfg utf8"),
                "--token",
                tok_paths[i].to_str().expect("tok utf8"),
            ])
            .stdout(Stdio::from(cap_log.reopen().expect("reopen cap log")))
            .stderr(Stdio::from(cap_log.reopen().expect("reopen cap log")))
            .spawn()
            .expect("spawn host-capturer");
        wait_for(&cap_log, "capturer ready");
        capturers.push(capturer);
        cap_logs.push(cap_log);
    }

    // streamer x2 -> the same local gateway (started sequentially, each negotiation completes before the next starts)
    let mut streamers = Vec::new();
    let mut str_logs = Vec::new();
    for (i, s) in [s0.as_str(), s1.as_str()].iter().enumerate() {
        let str_log = tempfile::NamedTempFile::new().expect("str log");
        let streamer = Command::new(env!("CARGO_BIN_EXE_host-streamer"))
            .args([
                "--stream",
                s,
                "--config",
                cfg_path.to_str().expect("cfg utf8"),
                "--token",
                tok_paths[2 + i].to_str().expect("tok utf8"),
                "--gateway",
                &format!("ws://127.0.0.1:{gw_port}/ws"),
            ])
            .stdout(Stdio::from(str_log.reopen().expect("reopen str log")))
            .stderr(Stdio::from(str_log.reopen().expect("reopen str log")))
            .spawn()
            .expect("spawn host-streamer");
        wait_for(&str_log, "streamer ready");
        streamers.push(streamer);
        str_logs.push(str_log);
    }
    // (1) stream evidence for both: each outbound stats bytes_sent>0 and frames_encoded>0 (the server has received frames)
    let ev0 = wait_for_flow(&str_logs[0]);
    let ev1 = wait_for_flow(&str_logs[1]);
    eprintln!("[streamer_e2e] stream {s0} evidence: {ev0}");
    eprintln!("[streamer_e2e] stream {s1} evidence: {ev1}");

    // (3) both streamers + both capturers stay alive throughout (no early exit)
    for (i, st) in streamers.iter_mut().enumerate() {
        assert!(st.try_wait().expect("try_wait").is_none(), "streamer {i} should be alive");
    }
    for (i, cp) in capturers.iter_mut().enumerate() {
        assert!(cp.try_wait().expect("try_wait").is_none(), "capturer {i} should be alive");
    }

    // (2) one vehicle one session: the unique vehicle room (with host), no stream-* rooms
    let rooms = admin_rooms();
    let rooms_arr = rooms["rooms"].as_array().expect("rooms array");
    eprintln!("[streamer_e2e] admin rooms: {rooms}");
    let vehicle: Vec<_> = rooms_arr.iter().filter(|r| r["id"] == "vehicle").collect();
    assert_eq!(
        vehicle.len(),
        1,
        "there should be exactly one vehicle room (both streams on one vehicle): got {rooms}"
    );
    assert!(
        vehicle[0]["host"].is_string(),
        "the vehicle room should contain the host peer: got {rooms}"
    );
    assert!(
        !rooms_arr.iter().any(|r| r["id"].as_str().is_some_and(|id| id.starts_with("stream-"))),
        "the streamer's RoomJoin should be intercepted by the gateway (no stream-* rooms): got {rooms}"
    );

    // (4) SIGTERM -> all five processes exit gracefully with 0 (streamers first, then capturers, then the agent)
    for st in &streamers {
        unsafe { libc::kill(st.id() as i32, libc::SIGTERM) };
    }
    for st in &mut streamers {
        assert_eq!(
            st.wait().expect("wait streamer").code(),
            Some(0),
            "the streamer should exit gracefully with 0"
        );
    }
    for cp in &capturers {
        unsafe { libc::kill(cp.id() as i32, libc::SIGTERM) };
    }
    for cp in &mut capturers {
        assert_eq!(
            cp.wait().expect("wait capturer").code(),
            Some(0),
            "the capturer should exit gracefully with 0"
        );
    }
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let at = agent.wait().expect("wait agent");
    assert_eq!(at.code(), Some(0), "agent should exit gracefully with 0, got {at:?}");
}
