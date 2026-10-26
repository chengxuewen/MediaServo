//! measure goldens (N3 discipline): each frozen pair under tests/goldens/ must be reproduced
//! byte-for-byte (stdout incl. `nan` spelling + rc) by `mediaservo-weaknet measure`.

use std::path::{Path, PathBuf};

fn golden_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens").join(name)
}

fn run_golden(name: &str, extra: &[&str]) -> (String, String) {
    let dir = golden_dir(name);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mediaservo-weaknet"))
        .args([
            "measure".to_string(),
            dir.join("probe.jsonl").display().to_string(),
            dir.join("timeline.jsonl").display().to_string(),
        ])
        .args(extra.iter().map(|s| s.to_string()))
        .output()
        .expect("spawn measure");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code().unwrap_or(-1).to_string())
}

fn expected(name: &str) -> (String, String) {
    let d = golden_dir(name);
    (
        std::fs::read_to_string(d.join("expected-stdout.txt")).unwrap(),
        std::fs::read_to_string(d.join("expected-rc.txt")).unwrap().trim().to_string(),
    )
}

#[test]
fn golden_v4_live() {
    let (out, rc) = run_golden("v4-live", &[]);
    let (e_out, e_rc) = expected("v4-live");
    assert_eq!(rc, e_rc, "v4-live: expected FAIL rc=1 (retention ~0.12)");
    assert_eq!(out, e_out, "v4-live table must match the python oracle byte-for-byte");
}

#[test]
fn golden_recovery_pass() {
    let (out, rc) = run_golden("recovery-pass", &[]);
    assert_eq!((out, rc), expected("recovery-pass"));
}

#[test]
fn golden_nan_recovery() {
    let (out, rc) = run_golden("nan-recovery", &[]);
    assert_eq!((out, rc), expected("nan-recovery"), "nan spelling byte-identical");
}

#[test]
fn golden_tolerance_pass() {
    let (out, rc) = run_golden("tolerance-pass", &[]);
    assert_eq!((out, rc), expected("tolerance-pass"), "WARN-only must not flip rc");
}

#[test]
fn golden_tol_flag() {
    let (out, rc) = run_golden("tol-flag", &["--tol", "0.90"]);
    assert_eq!((out, rc), expected("tol-flag"), "--tol must reach verdict predicate + footer");
}

#[test]
fn missing_scenario_start_errors() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mediaservo-weaknet"))
        .args(["measure", "/dev/null", "/dev/null"])
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(0), "empty inputs = usage-class error, not rc 0");
}
