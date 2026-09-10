//! T3: serve SIGTERM/SIGINT 优雅退出五步序 —— **真信号 e2e**（design §D2〔sF2 lck-F1〕）。
//!
//! PIT-189 铁律：tokio::signal handler 是**进程级**资源，libtest 并发下进程内 self-kill 会广播
//! 误醒其他案 → 真信号案必须在**独立子进程**跑（此处 spawn 编译好的 `mediaservo-weaknet` bin，
//! 只对子进程 pid 发 SIGTERM，绝不碰测试进程）。
//!
//! 零真触面（两案均不产生真实施压）：A = 无 state（clear 分支不进）+ `--token-file` 生成路径；
//! B = 植一条惰性 state（teardown channel=sidecar ∧ sidecar=null ∧ steps=[]）——启动交叉判定
//! 「通道不可重建」跳过、`engine::clear(quiet=true)` 对空 steps 零执行、仅 remove_file，全程不
//! exec tc / 不起 docker（五步 ③④ 顺序另由 `shutdown_plan` 单测钉住）。夹具全落
//! `/tmp/wnet-t3-*`（WEAKNET_STATEDIR 注入），用后自清。

use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_mediaservo-weaknet");

fn free_port() -> u16 {
    let a = ("127.0.0.1", 0u16)
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();
    let l = std::net::TcpListener::bind(a).unwrap();
    l.local_addr().unwrap().port()
}

/// 惰性 state：sidecar=null → channel_of=None → 启动跳过 tc 交叉判定；steps=[] → clear 零执行。
const INERT_STATE: &str = r#"{
  "schema": "weaknet-state/1",
  "spec": {"rtt_ms": 100, "jitter_ms": 0, "loss": null, "reorder_pct": null,
           "rate_mbps": null, "seed": null, "limit": 100000, "dir": "both"},
  "dir": "both",
  "scope": "media",
  "iface": "lo",
  "expires_at_ms": 99999999999999,
  "created_root": false,
  "teardown": {"channel": "sidecar", "sidecar": null, "steps": []}
}"#;

fn spawn_serve(statedir: &Path, extra: &[&str]) -> (Child, u16) {
    let port = free_port();
    let child = Command::new(BIN)
        .arg("serve")
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .args(extra)
        .env("WEAKNET_STATEDIR", statedir)
        // 不注入 WEAKNET_TOKEN/--token：走 --token-file 分支或 CSPRNG；auto 通道启动不 probe。
        .env_remove("WEAKNET_TOKEN")
        .env_remove("WEAKNET_CHANNEL")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mediaservo-weaknet serve");
    (child, port)
}

fn wait_listening(child: &mut Child, port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            panic!("serve 进程在就绪前退出（exit={st:?}）");
        }
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return; // listener 已 bind → 信号监听已建立（serve future 已入 select）
        }
        if Instant::now() > deadline {
            panic!("serve 10s 内未监听 :{port}");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// 对**子进程**发真 SIGTERM，并断言其在优雅预算内退 0（撞 oxmgr grace SIGKILL 的死路反例）。
fn sigterm_and_reap(child: &mut Child, stdout: &mut Option<std::process::ChildStdout>) -> String {
    let pid = child.id().to_string();
    let code = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .expect("kill -TERM 可用（coreutils）")
        .code();
    assert_eq!(code, Some(0), "kill -TERM 下发失败");
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if Instant::now() > deadline {
            let _ = child.kill(); // 超预算 = lck-F1 违例，测试进程收尸防孤儿
            panic!("SIGTERM 后 3s 内未退出（优雅路无界阻塞?）");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "SIGTERM 退出码非 0：{status:?}");
    let mut buf = String::new();
    if let Some(so) = stdout.as_mut() {
        so.read_to_string(&mut buf).ok();
    }
    buf
}

fn assert_seq(log: &str, markers: &[&str]) {
    let mut prev = 0usize;
    for m in markers {
        let pos = log.find(m).unwrap_or_else(|| panic!("缺优雅退出步骤标记 {m:?}\n---log---\n{log}"));
        assert!(pos >= prev, "步骤乱序：{m:?} 位置 {pos} < 前序 {prev}\n---log---\n{log}");
        prev = pos;
    }
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wnet-t3-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A：无施压 —— SIGTERM 走 ①②⑤（不进 clear），token-file 现场生成 0600。
#[test]
fn serve_sigterm_graceful_no_pressure() {
    let dir = tmp("a");
    let token = dir.join("weaknet.token");
    let (mut child, port) = spawn_serve(&dir, &["--token-file", token.to_str().unwrap()]);
    wait_listening(&mut child, port);
    let mut stdout = child.stdout.take();
    let log = sigterm_and_reap(&mut child, &mut stdout);

    assert_seq(
        &log,
        &[
            "优雅退出① stop 旗标已置",
            "优雅退出② drive 静默",
            "优雅退出⑤ exit 0",
        ],
    );
    assert!(
        !log.contains("优雅退出③④"),
        "无 state 不应进 clear 分支：\n{log}"
    );
    // token-file 生成路径：存在 + 0600（lck-F10/F11）。
    assert!(token.exists(), "token 文件应已生成");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&token).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token 文件权限必 0600，得 {mode:#o}");
    }
    // 无 state 落盘（未 apply）。
    assert!(!dir.join("state.json").exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// B：惰性活跃 state —— SIGTERM 走 ①②③④⑤，clear 分支到场并撤净 state（零真 tc/docker）。
#[test]
fn serve_sigterm_graceful_clears_inert_state() {
    let dir = tmp("b");
    let state_path = dir.join("state.json");
    std::fs::write(&state_path, INERT_STATE).unwrap();
    let (mut child, port) = spawn_serve(&dir, &["--token", "e2e-token-abcdef0123456789"]);
    wait_listening(&mut child, port);
    let mut stdout = child.stdout.take();
    let log = sigterm_and_reap(&mut child, &mut stdout);

    assert_seq(
        &log,
        &[
            "优雅退出① stop 旗标已置",
            "优雅退出② drive 静默",
            "优雅退出③④ clear",
            "优雅退出⑤ exit 0",
        ],
    );
    assert!(
        !state_path.exists(),
        "五步③④ 应经 engine::clear 落 inactive（remove state）：\n{log}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// C：既存 token-file 权限放宽（0644）——启动拒用、非零退出（lck-F11 umask 场景）。
#[test]
fn serve_rejects_world_readable_token_file() {
    #[cfg(not(unix))]
    {
        return; // POSIX mode 判定仅 unix 有意义
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("c");
        let token = dir.join("weaknet.token");
        std::fs::write(&token, "leaky-secret-token\n").unwrap();
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (mut child, _) = spawn_serve(&dir, &["--token-file", token.to_str().unwrap()]);
        // 绑定前即拒（token 解析在 bind 之前）→ 进程应短命非零退出，绝不 SIGTERM 它。
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(st) = child.try_wait().unwrap() {
                break st;
            }
            assert!(Instant::now() < deadline, "拒用案未在 10s 内退出");
            std::thread::sleep(Duration::from_millis(30));
        };
        assert!(!status.success(), "0644 token-file 应被拒 → 非零退出");
        std::fs::remove_dir_all(&dir).ok();
    }
}
