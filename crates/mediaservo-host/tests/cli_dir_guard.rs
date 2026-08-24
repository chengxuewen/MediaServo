//! CLI 目录参数位置化 + `--` 前缀守卫测试（D 轮事故回归）。
//!
//! 事故: `host init --dir` 把 "--dir" 当作路径创建了 `--dir/` 目录 —— 根因是
//! init 用位置参数而 start/stop/status/doctor/token 用 `--dir` flag（不一致）。
//! 修复: 目录统一为**位置参数**（缺省 `.host/`）；任何以 `--` 开头的目录参数一律
//! 拒绝（exit 2 + 明确报错）。
//! 拒绝（exit 2 + 明确报错）。

use std::process::Command;

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mediaservo-host"))
}

/// 以 `--` 开头的目录参数 → exit 2 + 守卫报错（任何子命令一致）。
fn assert_dir_guard_rejected(args: &[&str]) {
    let out = host().args(args).output().expect("spawn host");
    assert_eq!(
        out.status.code(),
        Some(2),
        "args {args:?} 应 exit 2，stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("不能以 -- 开头"),
        "stderr 应含守卫报错, got: {stderr}"
    );
}

#[test]
fn init_rejects_dashdash_dir() {
    // 事故复现: `host init --dir` 曾把 "--dir" 当路径创建 `--dir/` 目录
    assert_dir_guard_rejected(&["init", "--dir"]);
}

#[test]
fn start_rejects_dashdash_dir() {
    assert_dir_guard_rejected(&["start", "--dir"]);
}

#[test]
fn stop_rejects_dashdash_dir() {
    assert_dir_guard_rejected(&["stop", "--dir"]);
}

#[test]
fn doctor_rejects_dashdash_dir() {
    assert_dir_guard_rejected(&["doctor", "--dir"]);
}

#[test]
fn token_issue_rejects_dashdash_dir() {
    // token 子命令目录也统一位置参数 → --dir 落到位置参数 → 守卫拒绝
    assert_dir_guard_rejected(&[
        "token", "issue", "--role", "capture", "--node", "n", "--out", "x.token", "--dir",
    ]);
}

#[test]
fn start_accepts_positional_dir() {
    // 位置参数被接受 → 进入命令逻辑（无 host.yaml → exit 1），而非参数解析失败 exit 2
    let dir = tempfile::tempdir().expect("tempdir");
    let out = host().arg("start").arg(dir.path()).output().expect("spawn host start");
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("host.yaml"), "应报缺 host.yaml, got: {stderr}");
}

#[test]
fn init_defaults_to_hidden_host_dir() {
    // 缺省 `.host/`（NEW）: 空 cwd 中 `host init` 在 .host/ 下生成实例，
    // 不在 cwd 根散落 etc/（事故同类: 实例目录掉在意外 cwd）
    let cwd = tempfile::tempdir().expect("tempdir");
    let out = host().current_dir(cwd.path()).arg("init").output().expect("spawn host init");
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(
        cwd.path().join(".host/etc/host.yaml").is_file(),
        ".host/etc/host.yaml 应已生成（缺省 .host/），cwd 内容: {:?}",
        std::fs::read_dir(cwd.path()).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>()).unwrap_or_default()
    );
    assert!(
        !cwd.path().join("etc").exists(),
        "cwd 根不应出现 etc/（默认目录是 .host/ 而非 .）"
    );
}
