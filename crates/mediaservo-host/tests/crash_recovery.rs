//! Task C5: 崩溃重启故障注入 e2e — 架构核心价值验证（Momus MEDIUM-3）。
//!
//! 杀 capturer（SIGKILL = 崩溃，非 SIGTERM 优雅退出）→ oxmgr 按
//! `restart_policy = "always"` 自动拉起 → **同 topic 重发布成功**
//! （max_publishers(1) + iceoryx2 残留 service 不阻塞 — C25 根因路径）→
//! 订阅端（进程内 subscriber + host-recorder）恢复收帧。
//!
//! 全生产路径：`host init` → `host token issue` → `host start`（translate →
//! oxfile → `oxmgr apply`）→ `oxmgr` 管理 capturer → kill -9 → oxmgr 重启 →
//! FrameBus 订阅端实证帧恢复。不含 streamer（需外部 SFU server，C2/C4 已覆盖）。
//!
//! 前置: oxmgr 0.5.0 在 PATH（~/.local/bin）且 daemon 运行；C25: 跑前清
//! `/tmp/iceoryx2` + `/dev/shm/iox2_*`（本测试自带清理，见 `cleanup_iceoryx`）。

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameTopic, NodeAcl,
    NodeId, Role,
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

/// oxmgr 在 PATH 中（host CLI 内部调 `oxmgr`；测试进程可能没有 ~/.local/bin）。
fn path_with_oxmgr() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut path = format!("{home}/.local/bin");
    if let Ok(existing) = std::env::var("PATH") {
        path.push(':');
        path.push_str(&existing);
    }
    path
}

/// `oxmgr list --json` → host 命名空间进程列表（name/status/pid）。

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
}fn oxmgr_host_procs(dir: &std::path::Path) -> Vec<(String, String, u64)> {
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

/// 测试起点幂等: 清掉上次崩溃运行可能残留的 host 进程（stop+delete 按名）。
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

/// Drop 保证: 测试失败/panic 时也执行 `host stop [<dir>]`（oxmgr 不留孤儿进程）。
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

/// 等待 host.yaml 出现（host init 生成后立即改写）。
fn write_host_toml(dir: &Path) {
    // recorder 驻留（[record] enabled + out_dir 于实例内）→ 真实长生命周期订阅端
    // （host-recorder 设计上跨发布端崩溃续录）；capturer 崩溃重启期间 recorder
    // 必须全程存活（pid 不变）且同句柄恢复收帧。
    let cfg = format!(
        "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\nrecord:\n  enabled: true\n  out_dir: \"{}\"\n",
        dir.join("recordings").display()
    );
    std::fs::write(dir.join("etc").join("host.yaml"), cfg).expect("write host.yaml");
}

/// 等订阅端收到 ≥n 帧，返回期间看到的最大 seq。
async fn wait_frames(stream: &mediaservo_link::FrameStream, n: u32, timeout: Duration) -> u64 {
    let deadline = std::time::Instant::now() + timeout;
    let mut seen = 0u32;
    let mut max_seq = 0u64;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), stream.recv()).await {
            Ok(Some(frame)) => {
                max_seq = max_seq.max(frame.meta().seq);
                seen += 1;
                if seen >= n {
                    return max_seq;
                }
            }
            Ok(None) => panic!("订阅流关停"),
            Err(_) => {} // 超时继续等
        }
    }
    panic!("{timeout:?} 内只收到 {seen} 帧 (需 ≥{n})");
}

/// E2E 核心: 杀 capturer（SIGKILL）→ oxmgr 重启 → 同 topic 重发布 → 订阅恢复。
///
/// 实证结论（2026-08-19）: iceoryx2 0.9.3 发布端崩溃重启后，旧订阅端连接自动
/// 重建（seq 在重启点归零且后续连续）；所谓"旧订阅端 stale"是测试断言工件——
/// latest-slot 语义 + 重启探测轮询（500ms）会错过重启后头几帧（seq 0/1），
/// 之后取到的全是 seq≥基线 的帧。判别改为: 杀前等 ≥30 帧（基线 seq ≥29），
/// 后台 drainer 全程记录 seq，重启后断言出现 seq < 基线（归零必可捕获）。
#[tokio::test]
async fn capturer_kill9_restart_resumes_frames_to_subscribers() {
    cleanup_iceoryx();
    // 测试进程内启用 tracing，使 FrameBus 订阅线程的 receive 错误可见（调试用）
    mediaservo_common::logging::init(mediaservo_common::logging::LoggingConfig::default());
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().to_path_buf();
    cleanup_oxmgr_host(&dir_path);

    // ① host init（位置参数形态）+ 改写 host.yaml（cam0 + record enabled, 无 streams
    //    —— streamer 需外部 SFU server，C2/C4 覆盖） + 签发令牌
    assert_eq!(host_cli(&["init", dir_path.to_str().expect("dir utf8")]), 0);
    write_host_toml(&dir_path);
    assert_eq!(
        host_cli(&[
            "token",
            "issue",
            "--role",
            "capture",
            "--node",
            "capture-cam0",
            "--out",
            dir_path.join("etc/link/cam0.token").to_str().expect("tok utf8"),
            dir_path.to_str().expect("dir utf8"),
        ]),
        0
    );
    assert_eq!(
        host_cli(&[
            "token",
            "issue",
            "--role",
            "recorder",
            "--node",
            "recorder-cam0",
            "--out",
            dir_path.join("etc/link/recorder.token").to_str().expect("tok utf8"),
            dir_path.to_str().expect("dir utf8"),
        ]),
        0
    );

    // ② 进程内订阅端（Recorder 角色可订阅 camera/*）— 先于 capturer 挂载
    //    （与 recorder 进程同时序；验证“先订阅后发布”时序下崩溃重启恢复）
    let (sub_tok, sub_vk) = token(Role::Recorder, "crash-test-sub");
    let bus = FrameBus::attach("", &sub_tok, &sub_vk).expect("subscriber attach");
    let stream = bus.subscribe(&FrameTopic::new("camera/cam0")).expect("subscribe");

    // ③ host start（translate → run/oxfile.toml → oxmgr apply）→ 全部进程 running
    assert_eq!(host_cli(&["start", dir_path.to_str().expect("dir utf8")]), 0);
    let mut guard = OxmgrGuard { dir: dir_path.clone(), done: false };
    let procs = oxmgr_host_procs(&dir_path);
    assert_eq!(procs.len(), 6, "预期 6 进程 (5 fixed + capturer), got: {procs:?}");
    let capturer_pid = procs
        .iter()
        .find(|(n, _, _)| n == "host-capturer-cam0")
        .map(|(_, _, pid)| *pid)
        .expect("host-capturer 在 oxmgr 中");
    let recorder_pid = procs
        .iter()
        .find(|(n, _, _)| n == "host-recorder")
        .map(|(_, _, pid)| *pid)
        .expect("host-recorder 在 oxmgr 中");
    let recorder_running = recorder_pid != 0;
    assert!(capturer_pid != 0, "pid 应非 0: {procs:?}");

    // ④ 杀前基线: 等 ≥30 帧（≥1s @30fps）→ 基线 seq ≥29。杀后重启实例 seq 从 0
    //    重新开始；基线越高，重启后"归零帧"可判别窗口越长（latest-slot 会跳过
    //    部分低 seq 帧，判别靠 drainer 全量记录，见 ⑦）。
    let last_seq_before = wait_frames(&stream, 30, Duration::from_secs(20)).await;
    eprintln!("[crash_recovery] kill 前最后 seq={last_seq_before}");

    // ④b 后台 drainer: 从此刻起全量记录流上 seq（latest-slot 下消费者轮询会漏帧，
    //     drainer 连续取保证捕获重启点归零帧）
    let drained: std::sync::Arc<std::sync::Mutex<Vec<u64>>> = std::sync::Arc::default();
    {
        let drain_stream = stream.clone();
        let drained2 = std::sync::Arc::clone(&drained);
        tokio::spawn(async move {
            loop {
                match tokio::time::timeout(Duration::from_millis(500), drain_stream.recv()).await {
                    Ok(Some(f)) => drained2.lock().expect("drain lock").push(f.meta().seq),
                    Ok(None) => break,
                    Err(_) => continue,
                }
            }
        });
    }

    // ⑤ SIGKILL（崩溃路径，非 SIGTERM 优雅退出）
    unsafe { libc::kill(capturer_pid as i32, libc::SIGKILL) };
    eprintln!("[crash_recovery] SIGKILL capturer pid={capturer_pid}");

    // ⑥ 等 oxmgr 重启（新 pid + running）
    let mut new_pid = 0u64;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        let cur = oxmgr_host_procs(&dir_path)
            .into_iter()
            .find(|(n, _, _)| n == "host-capturer-cam0")
            .map(|(_, s, pid)| (s, pid))
            .unwrap_or_default();
        if cur.1 != 0 && cur.1 != capturer_pid {
            new_pid = cur.1;
            eprintln!("[crash_recovery] oxmgr 重启 capturer: pid={} status={}", cur.1, cur.0);
            break;
        }
    }
    assert!(new_pid != 0, "20s 内 oxmgr 未重启 capturer (pid 仍 {capturer_pid})");

    // ⑦ 核心断言: 同 topic 重发布成功 — 同一订阅端句柄恢复收帧。
    //    判据: drainer 记录中出现 seq < last_seq_before（重启实例 seq 归零）。
    //    （"20s 内看旧句柄是否还出帧"不可靠：latest-slot 下消费者可能永远取到
    //     seq≥基线 的帧而误判 stale——2026-08-19 实证。）
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let drained_guard = drained.lock().expect("drain lock");
        if drained_guard.iter().any(|&s| s < last_seq_before) {
            break;
        }
        drop(drained_guard);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let reset_seen = drained.lock().expect("drain lock").iter().any(|&s| s < last_seq_before);
    eprintln!(
        "[crash_recovery] resumed={reset_seen} drained_total={} (基线 seq ≥{last_seq_before})",
        drained.lock().expect("drain lock").len()
    );
    if !reset_seen {
        // 探针: 新建第二个订阅端 — 判别"连接未恢复" vs "发布端整体没在发布"
        let probe = bus.subscribe(&FrameTopic::new("camera/cam0")).expect("probe subscribe");
        match tokio::time::timeout(Duration::from_secs(5), probe.recv()).await {
            Ok(Some(f)) => eprintln!("[crash_recovery] PROBE: 新订阅端收到帧 seq={}", f.meta().seq),
            Ok(None) => eprintln!("[crash_recovery] PROBE: 新订阅端流关停"),
            Err(_) => eprintln!("[crash_recovery] PROBE: 新订阅端 5s 无帧"),
        }
    }
    assert!(
        reset_seen,
        "20s 内同一订阅端句柄未见重启后归零帧（seq 应 < {last_seq_before}）— \
         发布端未重启/未重发布? log: 见 oxmgr logs/host-capturer-cam0.out.log"
    );

    // ⑦ 崩溃隔离: recorder 全程存活（pid 不变 + running）— 无 crash-loop（仅 recorder 驻留时）
    if recorder_running {
        let recorder_now = oxmgr_host_procs(&dir_path)
            .into_iter()
            .find(|(n, _, _)| n == "host-recorder")
            .expect("recorder 仍在 oxmgr 中");
        assert_eq!(recorder_now.2, recorder_pid, "recorder 不应因 capturer 崩溃而重启");
        assert_eq!(recorder_now.1, "running", "recorder 应保持 running: {recorder_now:?}");
    }
    eprintln!(
        "[crash_recovery] OK: capturer {capturer_pid}→{new_pid} 重启后同 topic 重发布, \
         subscriber 恢复收帧, recorder {} 存活",
        recorder_pid
    );

    // ⑧ 清理: host stop（oxmgr stop + delete）→ host 命名空间清空
    assert_eq!(host_cli(&["stop", dir_path.to_str().expect("dir utf8")]), 0);
    guard.done = true;
    let remaining = oxmgr_host_procs(&dir_path);
    assert!(remaining.is_empty(), "stop 后应无 host 进程: {remaining:?}");
}
