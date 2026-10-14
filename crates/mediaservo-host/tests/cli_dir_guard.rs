//! CLI positional directory argument + `--` prefix guard tests (D-round incident regression).
//!
//! Incident: `host init --dir` treated "--dir" as a path and created a `--dir/` directory - root cause:
//! init took a positional argument while start/stop/status/doctor/token used a `--dir` flag (inconsistent).
//! Fix: the directory is uniformly a **positional argument** (default `.host/`); any directory argument
//! starting with `--` is rejected (exit 2 + explicit error).
//! starting with `--` is rejected (exit 2 + explicit error).

use std::process::Command;

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mediaservo-host"))
}

/// Directory arguments starting with `--` -> exit 2 + guard error (consistent across all subcommands).
fn assert_dir_guard_rejected(args: &[&str]) {
    let out = host().args(args).output().expect("spawn host");
    assert_eq!(
        out.status.code(),
        Some(2),
        "args {args:?} should exit 2, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("must not start with --"),
        "stderr should contain the guard error, got: {stderr}"
    );
}

#[test]
fn init_rejects_dashdash_dir() {
    // Incident repro: `host init --dir` once treated "--dir" as a path and created a `--dir/` directory
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
    // The token subcommand's directory is also positional -> --dir lands as the positional arg -> guard rejects
    assert_dir_guard_rejected(&[
        "token", "issue", "--role", "capture", "--node", "n", "--out", "x.token", "--dir",
    ]);
}

#[test]
fn start_accepts_positional_dir() {
    // The positional arg is accepted -> command logic runs (no host.yaml -> exit 1), not arg-parse failure exit 2
    let dir = tempfile::tempdir().expect("tempdir");
    let out = host().arg("start").arg(dir.path()).output().expect("spawn host start");
    assert_eq!(out.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("host.yaml"), "should report missing host.yaml, got: {stderr}");
}

#[test]
fn init_defaults_to_hidden_host_dir() {
    // Default `.host/` (NEW): in an empty cwd `host init` generates the instance under .host/,
    // not scattering etc/ at the cwd root (same incident class: instance dir landing in an unexpected cwd)
    let cwd = tempfile::tempdir().expect("tempdir");
    let out = host().current_dir(cwd.path()).arg("init").output().expect("spawn host init");
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(
        cwd.path().join(".host/etc/host.yaml").is_file(),
        ".host/etc/host.yaml should have been generated (default .host/), cwd contents: {:?}",
        std::fs::read_dir(cwd.path())
            .map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
            .unwrap_or_default()
    );
    assert!(
        !cwd.path().join("etc").exists(),
        "no etc/ should appear at the cwd root (default directory is .host/, not .)"
    );
}
