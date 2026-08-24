//! Task C2+D2: host-streamer 进程测试 — FrameBus 订阅 → 经本地网关推流（外部 mediasoup server）。
//!
//! - `bad_args_exit_2_with_usage`: 缺参/坏参 → exit 2 + stderr 用法提示
//! - `streamer_pushes_through_gateway_to_server`: host-agent（真进程，本地网关）
//!   + capturer + streamer → 外部 Docker server 收流（streamer 日志
//!   `bytes_sent>0 且 frames_encoded>0`，D4 证据模式），并且 server
//!   见到仅一个车位会话（admin API 房间列表 = 唯一 vehicle
//!   房间且含 host，无 stream-房间）→ SIGTERM 三进程优雅退出 0
//!
//! 前置: `SFU_E2E_WS_URL` 指向外部 mediasoup server（C21 纯外部模式，不 import
//! server 类型）; C25: 跑前清 `/tmp/iceoryx2` + `/dev/shm/iox2_*`。

#![cfg(target_os = "linux")]

use std::io::Read;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, NodeAcl, NodeId, Role, TokenFile,
};

// 测试用 Ed25519 密钥对（openssl 生成，仅测试用；与 link/deck/capturer 测试同源）。
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: 清 iceoryx2 0.9.3 运行时残留（/tmp/iceoryx2 + /dev/shm/iox2_*）。
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
            "SFU_E2E_WS_URL 未设置 — streamer e2e 需连外部 mediasoup server (C21);
            例: SFU_E2E_WS_URL=ws://127.0.0.1:9800/ws"
        )
    })
}

/// 同二进制内两个真 server 房间 e2e（D2 单流 / D3 双流）串行化：并发运行会
/// 撞 "vehicle" 房间（server 单 host 槽，join_full_host_slot_errors）与
/// camera/cam0 topic（FrameBus 单发布者，TopicConflict）。
static ROOM_E2E_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn bad_args_exit_2_with_usage() {
    for args in [
        vec![],                   // 全缺
        vec!["--stream"],         // 缺值
        vec!["--stream", "s0"],   // 缺 config/token
        vec!["--bogus", "x"],     // 未知参数
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_host-streamer"))
            .args(&args)
            .output()
            .expect("spawn host-streamer");
        assert_eq!(out.status.code(), Some(2), "args {args:?} 应 exit 2");
        assert!(
            !out.stderr.is_empty(),
            "args {args:?} stderr 应有用法提示, got: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// I1 审查: fps != 30 必须在启动即拒绝（推流编码器内置 30fps，PIT-64 类
/// rate-control 失配）。纯配置校验路径 — fps 检查先于令牌/信令，无需 server。
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
    assert_eq!(out.status.code(), Some(1), "fps=25 应 exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("fps"), "应指明 fps 字段, got: {stderr}");
    assert!(stderr.contains("25"), "应含实际值 25, got: {stderr}");
}

/// 读取子进程日志（stdout+stderr 合并到同一文件）。
fn read_log(file: &tempfile::NamedTempFile) -> String {
    let mut out = String::new();
    file.reopen()
        .expect("reopen log")
        .read_to_string(&mut out)
        .expect("read log");
    out
}

/// 轮询日志直到出现 needle（≤10s）。
fn wait_for(log: &tempfile::NamedTempFile, needle: &str) {
    for _ in 0..20 {
        if read_log(log).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("未见 {needle:?}, log:\n{}", read_log(log));
}

/// 轮询日志直到 stats 行 bytes_sent>0 且 frames_encoded>0（≤30s, D4 证据模式）。
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
    panic!("30s 内未见流证据, log:\n{}", read_log(log));
}

/// 网关主机口（host.toml [signaling] local_port 与测试分离）。
fn free_local_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("probe addr").port()
}

/// admin API 房间列表 — H3 起 admin REST 强制 JWT（auth_middleware），
/// 用 accounts.docker.yaml 的 dev admin 账号登录取 token（I2 review 守卫下
/// dev compose 已显式豁免）。
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
    // 1. 登录（dev 账号 admin/admin123 — accounts.docker.yaml）
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
    // 2. 房间列表（Bearer）
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

/// E2E (D2): host-agent（本地网关）+ capturer + streamer(--gateway)
/// → 外部 mediasoup server 收流，且仅一个车位会话。
#[tokio::test]
async fn streamer_pushes_through_gateway_to_server() {
    let _guard = ROOM_E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    cleanup_iceoryx();
    let _url = ws_url();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let stream_id = format!("s{pid}-stream");

    // host.yaml: cam0 generator 30fps + 唯一流（显式 source 引用，验证缺省外路径）
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n\
             streams:\n  - id: \"{stream_id}\"\n    source: \"cam0\"\n    codec: \"vp8\"\n"
        ),
    )
    .expect("write host.yaml");

    // 令牌: capturer=Capture（可发布 camera/*），streamer=Recorder（可订阅 camera/*）
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam0.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
    let (str_tok, str_vk) = token(Role::Recorder, &format!("streamer-{pid}"));
    let str_path = dir.path().join("streamer.token");
    std::fs::write(&str_path, TokenFile::encode(&str_tok, &str_vk)).expect("write str token");

    // host-agent（本地网关）: 随机端口 + 远端指真 server
    let gw_port = free_local_port();
    let agent_log = tempfile::NamedTempFile::new().expect("agent log");
    let mut agent = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .args([
            "--port",
            &gw_port.to_string(),
            "--remote",
            &ws_url(),
            "--room",
            "vehicle",
        ])
        .stdout(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .stderr(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .spawn()
        .expect("spawn host-agent");
    // 等 agent 加入整车房间（否则 streamer RoomJoin 被拦截回 5001）
    wait_for(&agent_log, "agent 已加入整车房间");

    // capturer 进程（先起：streamer 首帧 gate 依赖发布端）
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

    // streamer 进程 → 本地网关
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

    // 流证据: 出站统计 bytes_sent>0 且 frames_encoded>0（server 已收帧）
    let evidence = wait_for_flow(&str_log);
    eprintln!("[streamer_e2e] 流证据: {evidence}");

    // 一车一会话: server 房间列表应只有 vehicle（含 host），无 stream-房间
    let rooms = admin_rooms();
    let rooms_arr = rooms["rooms"].as_array().expect("rooms array");
    eprintln!("[streamer_e2e] admin rooms: {rooms}");
    assert!(
        rooms_arr
            .iter()
            .any(|r| r["id"] == "vehicle" && r["host"].is_string()),
        "server 应见到 vehicle 房间 + host, got {rooms}"
    );
    assert!(
        !rooms_arr
            .iter()
            .any(|r| r["id"].as_str().is_some_and(|id| id.starts_with("stream-"))),
        "streamer RoomJoin 应被网关拦截（无 stream-* 房间）, got {rooms}"
    );

    // SIGTERM → 三进程优雅退出 0
    unsafe { libc::kill(streamer.id() as i32, libc::SIGTERM) };
    let st = streamer.wait().expect("wait streamer");
    assert_eq!(st.code(), Some(0), "streamer 应优雅退出 0, got {st:?}");
    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "capturer 应优雅退出 0, got {ct:?}");
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let at = agent.wait().expect("wait agent");
    assert_eq!(at.code(), Some(0), "agent 应优雅退出 0, got {at:?}");
}

/// E2E (D3): 双路推流经同一网关会话 — host-agent + capturer×2 (cam0/cam1)
/// + streamer×2 (s0/s1, 同一 --gateway) → 外部 mediasoup server:
/// ① 两路流均出站 bytes_sent>0 且 frames_encoded>0（D4 证据模式）
/// ② admin 房间列表 = 唯一 vehicle 房间（含 host），无 stream-* 房间
/// ③ 两 streamer + 两 capturer 全程存活
/// ④ SIGTERM → 五进程优雅退出 0
///
/// D-H6「多路 produce = 同 peer 多 transport」: 每 streamer 经同一网关
/// 单 WS 上 Server，各自 Create/Connect/Produce 在网关 FIFO 按序配对。
#[tokio::test]
async fn two_streamers_share_one_vehicle_session() {
    let _guard = ROOM_E2E_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    cleanup_iceoryx();
    let _url = ws_url();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let s0 = format!("s{pid}-0");
    let s1 = format!("s{pid}-1");

    // host.yaml: cam0/cam1 generator 30fps + 两路流（各自显式 source 引用）
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

    // 令牌: capturer=Capture（camera/*）, streamer=Recorder（camera/*）— 每进程独立 node id
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

    // host-agent（本地网关）: 随机端口 + 远端指真 server
    let gw_port = free_local_port();
    let agent_log = tempfile::NamedTempFile::new().expect("agent log");
    let mut agent = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .args([
            "--port",
            &gw_port.to_string(),
            "--remote",
            &ws_url(),
            "--room",
            "vehicle",
        ])
        .stdout(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .stderr(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .spawn()
        .expect("spawn host-agent");
    wait_for(&agent_log, "agent 已加入整车房间");

    // capturer×2（先起：streamer 首帧 gate 依赖发布端）
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

    // streamer×2 → 同一本地网关（顺序启动，各自协商完整后再起下一个）
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
    // ① 两路流证据: 各自出站统计 bytes_sent>0 且 frames_encoded>0（server 已收帧）
    let ev0 = wait_for_flow(&str_logs[0]);
    let ev1 = wait_for_flow(&str_logs[1]);
    eprintln!("[streamer_e2e] stream {s0} 证据: {ev0}");
    eprintln!("[streamer_e2e] stream {s1} 证据: {ev1}");

    // ③ 两 streamer + 两 capturer 全程存活（未提前退出）
    for (i, st) in streamers.iter_mut().enumerate() {
        assert!(st.try_wait().expect("try_wait").is_none(), "streamer {i} 应存活");
    }
    for (i, cp) in capturers.iter_mut().enumerate() {
        assert!(cp.try_wait().expect("try_wait").is_none(), "capturer {i} 应存活");
    }

    // ② 一车一会话: 唯一 vehicle 房间（含 host），无 stream-* 房间
    let rooms = admin_rooms();
    let rooms_arr = rooms["rooms"].as_array().expect("rooms array");
    eprintln!("[streamer_e2e] admin rooms: {rooms}");
    let vehicle: Vec<_> = rooms_arr.iter().filter(|r| r["id"] == "vehicle").collect();
    assert_eq!(vehicle.len(), 1, "应恰好一个 vehicle 房间（两流同车）: got {rooms}");
    assert!(
        vehicle[0]["host"].is_string(),
        "vehicle 房间应含 host peer: got {rooms}"
    );
    assert!(
        !rooms_arr
            .iter()
            .any(|r| r["id"].as_str().is_some_and(|id| id.starts_with("stream-"))),
        "streamer RoomJoin 应被网关拦截（无 stream-* 房间）: got {rooms}"
    );

    // ④ SIGTERM → 五进程优雅退出 0（先 streamer 后 capturer 后 agent）
    for st in &streamers {
        unsafe { libc::kill(st.id() as i32, libc::SIGTERM) };
    }
    for st in &mut streamers {
        assert_eq!(st.wait().expect("wait streamer").code(), Some(0), "streamer 应优雅退出 0");
    }
    for cp in &capturers {
        unsafe { libc::kill(cp.id() as i32, libc::SIGTERM) };
    }
    for cp in &mut capturers {
        assert_eq!(cp.wait().expect("wait capturer").code(), Some(0), "capturer 应优雅退出 0");
    }
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let at = agent.wait().expect("wait agent");
    assert_eq!(at.code(), Some(0), "agent 应优雅退出 0, got {at:?}");
}
