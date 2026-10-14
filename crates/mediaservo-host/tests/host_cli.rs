//! Tests for the `host -h` / `--help` CLI.

use std::process::Command;

fn host_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mediaservo-host")
}

#[test]
fn host_no_args_prints_usage_exits_0() {
    let out = Command::new(host_bin()).output().unwrap();
    assert!(out.status.success(), "host (no args) should exit 0, got {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Usage: mediaservo-host"), "stdout should contain usage: {stdout}");
}

#[test]
fn host_help_flag_exits_0() {
    for flag in ["-h", "--help"] {
        let out = Command::new(host_bin()).arg(flag).output().unwrap();
        assert!(out.status.success(), "host {flag} should exit 0, got {:?}", out.status.code());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("Usage: mediaservo-host"),
            "host {flag} stdout should contain usage: {stdout}"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.is_empty(), "host {flag} stderr should be empty: {stderr}");
    }
}

#[test]
fn host_unknown_subcommand_exits_2() {
    for cmd in ["does-not-exist-xyz", "foobar"] {
        let out = Command::new(host_bin()).arg(cmd).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "host {cmd} should exit 2, got {:?}",
            out.status.code()
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("unknown subcommand"),
            "host {cmd} stderr should contain 'unknown subcommand': {stderr}"
        );
    }
}
