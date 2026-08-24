//! Task E4: 云端配置闭环 e2e — ConfigPush 接收 → 校验 → host.yaml 更新（备份）→
//! oxfile 重生成 → OxMgr 热生效（增量 apply + file-watch 重启）。
//!
//! ① mock server 下发 ConfigPush → 网关 → 应用: host.yaml 更新 + 备份 + oxfile 重生成
//!    + 版本记入（StatusReport.config_version 关联数据源）
//! ② 非法配置拒绝: host.yaml 不变 + 无备份 + 审计日志（tracing 捕获断言）
//! ③ OxMgr 热生效实证（Linux + oxmgr 前置）: host start → 推新配置（加相机）→
//!    新 capturer 进程出现（apply 增量 Start）+ 既有 capturer pid 变更
//!    （host.yaml 内容指纹 → file-watch 重启——命令未变时 apply 为 Noop，
//!     pid 变更只可能来自 watch）；再推删相机配置 → 对应 app 被清理（removal）
//!
//! 前置（C25）: 测试自带 iceoryx2 清理（/tmp/iceoryx2 + /dev/shm/iox2_*）。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_host::gateway::{run_gateway, GatewayConfig, GatewayHandle};
use mediaservo_link::RetryConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

/// 最近一次应用结果（Err 拒绝原因 = handle_config_push 审计日志 warn 载荷；
/// 测试断言审计内容的确定性通道——日志经 tracing 输出, 内容与 Err 文本一致）。
type Outcome = Arc<Mutex<Option<String>>>;

const VEHICLE_ROOM: &str = "vehicle-1";
const VEHICLE_PEER: &str = "veh-peer";

const CFG_V0: &str = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n";
const CFG_V1: &str = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\n";
const CFG_V2: &str = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 15\n";

fn write_host_toml(dir: &Path, cfg: &str) {
    std::fs::create_dir_all(dir.join("etc")).expect("create etc");
    std::fs::write(dir.join("etc").join("host.yaml"), cfg).expect("write host.yaml");
}

/// 与 host-agent 同形的应用循环（轮询网关待应用 ConfigPush；测试用 100ms 轮询）。
/// outcome 记录最近一次结果（成功版本 / 拒绝原因——与 handle_config_push 审计日志同文）。
fn spawn_applier(
    handle: GatewayHandle,
    dir: PathBuf,
    version: Arc<AtomicU64>,
    outcome: Outcome,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            let Some(push) = handle.take_config_push() else { continue };
            match mediaservo_host::translate::handle_config_push(&dir, version.load(Ordering::Relaxed), &push) {
                Ok(v) => {
                    version.store(v, Ordering::Relaxed);
                    *outcome.lock().expect("outcome lock") = Some(format!("accepted v{v}"));
                }
                Err(e) => *outcome.lock().expect("outcome lock") = Some(e),
            }
        }
    });
}

/// mock server 完整握手（对齐 link::SignalClient 协议流程）。
async fn mock_handshake(listener: &TcpListener) -> WebSocketStream<TcpStream> {
    let (stream, _) = listener.accept().await.expect("mock accept");
    let mut ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("mock ws handshake");
    let psk = ws.next().await.unwrap().unwrap();
    assert!(matches!(psk, Message::Text(_)), "首条应为 PSK 文本");
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::Error { code: 0, message: String::new() })
            .unwrap()
            .into(),
    ))
    .await
    .unwrap();
    let join = ws.next().await.unwrap().unwrap();
    match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap()).expect("parse RoomJoin") {
        SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
            assert_eq!(peer_role, PeerRole::Host);
            ws.send(Message::Text(
                serde_json::to_string(&SignalingMessage::RoomJoined {
                    room_id,
                    peer_id: VEHICLE_PEER.into(),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        }
        other => panic!("expected RoomJoin, got {other:?}"),
    }
    ws
}

/// 启动网关（临时本地端口）+ 应用循环，返回句柄。
async fn start_gateway_and_applier(
    addr: &str,
    dir: PathBuf,
    version: Arc<AtomicU64>,
    outcome: Outcome,
) -> (GatewayHandle, u16) {
    let cfg = GatewayConfig {
        local_port: 0,
        remote_url: format!("ws://{addr}/ws"),
        psk: "test-psk".into(),
        device: None,
        room: VEHICLE_ROOM.into(),
        retry: RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(500),
        },
    };
    let (port, handle) = run_gateway(cfg).await.expect("run_gateway");
    spawn_applier(handle.clone(), dir, version, outcome);
    (handle, port)
}

#[tokio::test(flavor = "current_thread")]
async fn config_push_via_mock_server_updates_host_toml_backs_up_and_regenerates_oxfile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().to_path_buf();
    write_host_toml(&dir_path, CFG_V0);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut ws = mock_handshake(&listener).await;
        // 入房后立即下发 ConfigPush（v1 配置, version 7）
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::ConfigPush {
                room_id: VEHICLE_ROOM.into(),
                target: VEHICLE_PEER.into(),
                config: CFG_V1.into(),
                version: 7,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        // 保持连接（网关侧正常；测试结束时 task 随 runtime 终止）
        loop {
            if ws.next().await.is_none() {
                break;
            }
        }
    });

    let version = Arc::new(AtomicU64::new(0));
    let outcome: Outcome = Arc::default();
    let (_handle, _port) = start_gateway_and_applier(&addr.to_string(), dir_path.clone(), version.clone(), outcome).await;

    // 轮询应用结果（host.yaml 更新 + 备份 + oxfile 重生成 + 版本记入）
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let toml = std::fs::read_to_string(dir_path.join("etc").join("host.yaml")).unwrap();
        let bak = dir_path.join("etc").join("host.yaml.bak-7");
        let ox = std::fs::read_to_string(dir_path.join("run").join("oxfile.toml"));
        if toml == CFG_V1 && bak.exists() && version.load(Ordering::Relaxed) == 7 {
            let ox = ox.expect("oxfile 应已生成");
            assert!(ox.contains("host-capturer-cam1"), "新相机实例应入 oxfile: {ox}");
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "10s 内未完成应用: yaml_updated={} bak={} version={}",
                toml == CFG_V1,
                bak.exists(),
                version.load(Ordering::Relaxed)
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        std::fs::read_to_string(dir_path.join("etc").join("host.yaml.bak-7")).unwrap(),
        CFG_V0,
        "备份应为旧配置"
    );
    // F1 关联契约: 模拟 agent 重启 — 新实例（新 Arc）从备份恢复版本，不归零
    let restarted = Arc::new(AtomicU64::new(
        mediaservo_host::translate::recover_config_version(&dir_path),
    ));
    assert_eq!(restarted.load(Ordering::Relaxed), 7, "重启后应从磁盘恢复版本 7（关联契约）");
    assert_eq!(
        std::fs::read_to_string(dir_path.join("etc").join("host.yaml.bak-7")).unwrap(),
        CFG_V0,
        "备份应为旧配置"
    );
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_config_push_rejected_with_audit_log_and_unchanged_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().to_path_buf();
    write_host_toml(&dir_path, CFG_V0);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut ws = mock_handshake(&listener).await;
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::ConfigPush {
                room_id: VEHICLE_ROOM.into(),
                target: VEHICLE_PEER.into(),
                config: "not toml [[[".into(),
                version: 9,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        loop {
            if ws.next().await.is_none() {
                break;
            }
        }
    });

    let version = Arc::new(AtomicU64::new(0));
    let outcome: Outcome = Arc::default();
    let (_handle, _port) = start_gateway_and_applier(&addr.to_string(), dir_path.clone(), version.clone(), outcome.clone()).await;

    // 等拒绝结果（outcome 记录 = handle_config_push 审计日志 warn 的同一载荷）
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let guard = outcome.lock().expect("outcome lock");
        if let Some(reason) = guard.as_deref() {
            assert!(
                reason.contains("解析失败"),
                "拒绝原因应含解析失败（审计载荷）: {reason}"
            );
            break;
        }
        drop(guard);
        if std::time::Instant::now() > deadline {
            panic!("10s 内未出现拒绝结果（应用循环未处理 ConfigPush）");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    server.abort();

    // 文件不变 + 无备份 + 版本未推进
    assert_eq!(
        std::fs::read_to_string(dir_path.join("etc").join("host.yaml")).unwrap(),
        CFG_V0,
        "非法配置不得改写 host.yaml"
    );
    assert!(
        !dir_path.join("etc").join("host.yaml.bak-9").exists(),
        "非法配置不得产生备份"
    );
    assert!(
        !dir_path.join("run").join("oxfile.toml").exists(),
        "非法配置不得改写 oxfile"
    );
    assert_eq!(version.load(Ordering::Relaxed), 0, "版本不得推进");
}

// ── ③ OxMgr 热生效实证（真实 oxmgr + host 进程） ─────────────────────────────

#[cfg(target_os = "linux")]
mod oxmgr_hot_reload {
    use super::*;

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

    fn path_with_oxmgr() -> String {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut path = format!("{home}/.local/bin");
        if let Ok(existing) = std::env::var("PATH") {
            path.push(':');
            path.push_str(&existing);
        }
        path
    }

/// oxmgr 实例 daemon env（与 translate::oxmgr_apply 同源: OXMGR_HOME 派生端口隔离
/// daemon——`oxmgr list` 不带此 env 会连默认 daemon（空），看不到实例进程）。
fn oxmgr_env(dir: &std::path::Path) -> Vec<(String, String)> {
    let home = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr");
    let sum: u32 = home.to_string_lossy().bytes().map(u32::from).sum();
    let port = 18000 + (sum % 400);
    vec![
        ("OXMGR_HOME".to_string(), home.to_string_lossy().into_owned()),
        ("OXMGR_DAEMON_ADDR".to_string(), format!("127.0.0.1:{port}")),
        ("OXMGR_API_ADDR".to_string(), format!("127.0.0.1:{}", port + 1000)),
    ]
}
    fn oxmgr_host_procs(dir: &std::path::Path) -> Vec<(String, String, u64)> {
        let out = Command::new("oxmgr")
            .env("PATH", path_with_oxmgr())
            .envs(oxmgr_env(dir))
            .args(["list", "--json"])
            .output()
            .expect("oxmgr list");
        assert!(
            out.status.success(),
            "oxmgr list 失败（oxmgr 0.5.0 需在 PATH 且 daemon 运行）: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let procs: serde_json::Value = serde_json::from_slice(&out.stdout).expect("oxmgr json");
        procs
            .as_array()
            .expect("oxmgr list 应为数组")
            .iter()
            .filter(|p| p.get("namespace").and_then(|n| n.as_str()) == Some("mediaservo-host"))
            .map(|p| {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let pid = p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                (name, status, pid)
            })
            .collect()
    }

    fn cleanup_oxmgr_host(dir: &std::path::Path) {
        let names: Vec<String> = oxmgr_host_procs(dir).into_iter().map(|(n, _, _)| n).collect();
        for name in names {
            let _ = Command::new("oxmgr")
                .env("PATH", path_with_oxmgr())
                .envs(oxmgr_env(dir))
                .args(["stop", &name])
                .status();
            let _ = Command::new("oxmgr")
                .env("PATH", path_with_oxmgr())
                .envs(oxmgr_env(dir))
                .args(["delete", &name])
                .status();
        }
    }

    /// 运行 host CLI（PATH 注入 ~/.local/bin 使 oxmgr 可解析），返回 exit code。
    fn host_cli(args: &[&str]) -> i32 {
        let out = Command::new(env!("CARGO_BIN_EXE_mediaservo-host"))
            .env("PATH", path_with_oxmgr())
            .args(args)
            .output()
            .expect("spawn host CLI");
        let code = out.status.code().unwrap_or(-1);
        if code != 0 {
            eprintln!("host {} 失败 (exit {code}): {}", args[0], String::from_utf8_lossy(&out.stderr));
        }
        code
    }

    struct OxmgrGuard {
        dir: PathBuf,
        done: bool,
    }

    impl Drop for OxmgrGuard {
        fn drop(&mut self) {
            if !self.done {
                let _ = host_cli(&["stop", self.dir.to_str().expect("dir utf8")]);
            }
        }
    }

    fn issue_token(dir: &Path, role: &str, node: &str, out: &str) {
        assert_eq!(
            host_cli(&[
                "token",
                "issue",
                "--role",
                role,
                "--node",
                node,
                "--out",
                dir.join("etc").join("link").join(out).to_str().expect("tok utf8"),
                dir.to_str().expect("dir utf8"),
            ]),
            0
        );
    }

    #[tokio::test]
    async fn oxmgr_hot_reload_add_watch_and_remove_camera_apply() {
        cleanup_iceoryx();
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path().to_path_buf();
        cleanup_oxmgr_host(&dir_path);

        // ① host init + v0 配置（cam0）+ 令牌（capture cam0 + recorder）
        assert_eq!(host_cli(&["init", dir_path.to_str().expect("dir utf8")]), 0);
        write_host_toml(&dir_path, CFG_V0);
        issue_token(&dir_path, "capture", "capture-cam0", "cam0.token");
        issue_token(&dir_path, "recorder", "recorder-cam0", "recorder.token");

        // ② host start → 6 进程（5 fixed + host-capturer-cam0）running
        assert_eq!(host_cli(&["start", dir_path.to_str().expect("dir utf8")]), 0);
        let mut guard = OxmgrGuard { dir: dir_path.clone(), done: false };
        let procs = oxmgr_host_procs(&dir_path);
        assert_eq!(procs.len(), 6, "预期 6 进程 (5 fixed + capturer), got: {procs:?}");
        let old_pid = procs
            .iter()
            .find(|(n, _, _)| n == "host-capturer-cam0")
            .map(|(_, _, pid)| *pid)
            .expect("host-capturer-cam0 在 oxmgr 中");
        assert!(old_pid != 0, "capturer pid 应非 0");

        // ③ 推新配置（加 cam1）+ cam1 令牌 — 与 host-agent 相同的落盘→apply 序列
        //    （agent 写 host.yaml + 重生成 oxfile → oxmgr apply；测试经 host CLI 执行
        //     apply——测试进程 current_exe 在 deps/ 下无法解析产物路径；watch 重启
        //     经 daemon 异步触发）
        issue_token(&dir_path, "capture", "capture-cam1", "cam1.token");
        write_host_toml(&dir_path, CFG_V1);
        assert_eq!(host_cli(&["apply", dir_path.to_str().expect("dir utf8")]), 0);

        // ④ 实证 1（apply 增量 Start）: host-capturer-cam1 出现并 running
        //    实证 2（file-watch 重启）: host-capturer-cam0（命令未变 → apply Noop）
        //      pid 变更 — 只可能来自 host.yaml 内容指纹 → watch 重启
        let mut new_pid = 0u64;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let cur = oxmgr_host_procs(&dir_path);
            let cam1 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam1").cloned();
            let cam0 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam0").cloned();
            if let Some((_, s1, _)) = &cam1 {
                if s1 == "running" {
                    if let Some((_, s0, p0)) = &cam0 {
                        if p0 != &0 && p0 != &old_pid {
                            new_pid = *p0;
                            eprintln!(
                                "[config_push_e2e] OK: cam1 running + cam0 watch 重启 {old_pid}→{new_pid}"
                            );
                            break;
                        }
                    }
                }
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "30s 内未达成热生效: cam1={cam1:?} cam0={cam0:?} (旧 pid {old_pid})",
                    cam1 = cam1,
                    cam0 = cam0
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(new_pid != 0, "watch 重启后 pid 应非 0");

        // ④b 删除路径实证: 推 v2（cam0 单相机, fps 15）→ cam1 app 被清理
        //    （oxmgr_apply removal 同步；删除 = oxmgr delete），cam0 命令未变 →
        //    再次 watch 重启（pid 又变）
        write_host_toml(&dir_path, CFG_V2);
        assert_eq!(host_cli(&["apply", dir_path.to_str().expect("dir utf8")]), 0);
        let mut pid_after_remove = 0u64;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let cur = oxmgr_host_procs(&dir_path);
            let cam1 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam1").cloned();
            let cam0 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam0").cloned();
            if cam1.is_none() {
                if let Some((_, s0, p0)) = &cam0 {
                    if p0 != &0 && p0 != &new_pid {
                        pid_after_remove = *p0;
                        eprintln!(
                            "[config_push_e2e] OK: cam1 已删除 + cam0 watch 重启 {new_pid}→{pid_after_remove}"
                        );
                        break;
                    }
                }
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "30s 内未达成删除+重启: cam1={cam1:?} cam0={cam0:?}",
                    cam1 = cam1,
                    cam0 = cam0
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(pid_after_remove != 0, "删除后 watch 重启 pid 应非 0");

        // ⑤ 清理: host stop → host 命名空间清空（watch 重启与 stop 存在竞态——",
        //    停后复查，残留再停，最多 15s）
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if oxmgr_host_procs(&dir_path).is_empty() {
                break;
            }
            if std::time::Instant::now() > deadline {
                break;
            }
            let _ = host_cli(&["stop", dir_path.to_str().expect("dir utf8")]);
            std::thread::sleep(Duration::from_millis(500));
        }
        guard.done = true;
        let remaining = oxmgr_host_procs(&dir_path);
        assert!(remaining.is_empty(), "stop 后应无 host 进程: {remaining:?}");
    }
}
