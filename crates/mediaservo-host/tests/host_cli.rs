//! `host -h` / `--help` CLI 测试。

use std::process::Command;

fn host_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mediaservo-host")
}

#[test]
fn host_no_args_prints_usage_exits_0() {
    let out = Command::new(host_bin()).output().unwrap();
    assert!(
        out.status.success(),
        "host (no args) 应 exit 0，实际 {:?}",
        out.status.code()
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("用法: mediaservo-host"),
        "stdout 应含用法: {stdout}"
    );
}

#[test]
fn host_help_flag_exits_0() {
    for flag in ["-h", "--help"] {
        let out = Command::new(host_bin()).arg(flag).output().unwrap();
        assert!(
            out.status.success(),
            "host {flag} 应 exit 0，实际 {:?}",
            out.status.code()
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("用法: mediaservo-host"),
            "host {flag} stdout 应含用法: {stdout}"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.is_empty(),
            "host {flag} stderr 应为空: {stderr}"
        );
    }
}

#[test]
fn host_unknown_subcommand_exits_2() {
    for cmd in ["does-not-exist-xyz", "foobar"] {
        let out = Command::new(host_bin()).arg(cmd).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "host {cmd} 应 exit 2，实际 {:?}",
            out.status.code()
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("未知子命令"),
            "host {cmd} stderr 应含'未知子命令': {stderr}"
        );
    }
}