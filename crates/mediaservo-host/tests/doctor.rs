//! Tests for the `host doctor` subcommand (Task A3).
//!
//! Asserts the three checks (oxmgr available / host.yaml parseable / oxfile generatable) and
//! the semantics that exit code = number of failed checks. Environment independence: whether oxmgr is
//! on PATH is probed by the test and the expected exit code follows the probe (holds in CI without oxmgr).

use std::path::PathBuf;
use std::process::Command;

fn host_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mediaservo-host")
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("host-doctor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("etc")).unwrap();
    d
}

fn run_doctor(dir: &PathBuf) -> std::process::Output {
    Command::new(host_bin()).arg("doctor").arg(dir).output().unwrap()
}

/// oxmgr presence probe (a successful spawn means executable, regardless of how oxmgr answers the args).
fn oxmgr_present() -> bool {
    Command::new("oxmgr").arg("--version").output().is_ok()
}

#[test]
fn doctor_healthy_dir_checks_pass() {
    // With oxmgr missing (e.g. CI) the all-healthy assertion is meaningless -> skip; config checks stay independent.
    if !oxmgr_present() {
        return;
    }
    let dir = tmp_dir("healthy");
    std::fs::write(dir.join("etc").join("host.yaml"), "host:\n  device_id: \"car-01\"\n").unwrap();
    let out = run_doctor(&dir);
    assert!(
        out.status.success(),
        "doctor should exit 0, got {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for needle in ["[ok] oxmgr", "[ok] host.yaml", "[ok] oxfile"] {
        assert!(stdout.contains(needle), "stdout missing {needle}:\n{stdout}");
    }
}

#[test]
fn doctor_broken_config_counts_failures() {
    // Syntactically broken host.yaml: checks 2/3 always fail -> exit code = 2 + (oxmgr missing ? 1 : 0).
    let dir = tmp_dir("broken");
    std::fs::write(dir.join("etc").join("host.yaml"), "[host\nbroken yaml").unwrap();
    let out = run_doctor(&dir);
    let expected = if oxmgr_present() { 2 } else { 3 };
    assert_eq!(
        out.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Output must mark each failed check with a reason (this assertion fails on old binaries without the doctor subcommand)
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[fail] host.yaml"),
        "stdout missing host.yaml failure marker:\n{stdout}"
    );
    assert!(stdout.contains("[fail] oxfile"), "stdout missing oxfile failure marker:\n{stdout}");
}

#[test]
fn doctor_missing_config_counts_both_config_failures() {
    // Empty etc/ (no host.yaml): checks 2/3 both fail, each printing one [fail] marker ->
    // exit code = 2 + (oxmgr missing ? 1 : 0), matching the printed failure count.
    let dir = tmp_dir("missing"); // tmp_dir only creates etc/, writes no host.yaml
    let out = run_doctor(&dir);
    let expected = if oxmgr_present() { 2 } else { 3 };
    assert_eq!(
        out.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[fail] failed to read"),
        "stdout missing read-failure marker:\n{stdout}"
    );
    assert!(stdout.contains("[fail] oxfile"), "stdout missing oxfile failure marker:\n{stdout}");
}
