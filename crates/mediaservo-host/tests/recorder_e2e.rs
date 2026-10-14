//! Task C3: host-recorder process tests - FrameBus subscribe -> deck Recorder writes MP4.
//!
//! - `bad_args_exit_2_with_usage`: missing/bad args -> exit 2 + usage hint on stderr
//! - `record_disabled_exits_0`: [record] enabled=false -> exit 0 + stdout message
//!   (process-level gating; the oxfile still contains the recorder app, runtime gate)
//! - `recorder_writes_mp4_from_capturer_frames`: capturer (real process) + recorder
//!   (real process) -> MP4 written to disk -> ffprobe verifies duration>0 + codec=h264 ->
//!   SIGTERM: both exit gracefully with 0
//!
//! Prerequisite: tests must run in the pixi run environment (ffprobe/FFmpeg shared libs come from the pixi env);
//! C25: clean `/tmp/iceoryx2` + `/dev/shm/iox2_*` before running.

#![cfg(target_os = "linux")]

use std::io::Read;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, NodeAcl, NodeId, Role, TokenFile,
};

// Test Ed25519 keypair (generated with openssl, test-only; same source as link/deck/capturer/streamer tests).
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: clean iceoryx2 0.9.3 runtime residue (/tmp/iceoryx2 + /dev/shm/iox2_*).
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

#[test]
fn bad_args_exit_2_with_usage() {
    for args in [
        vec![],                // all missing
        vec!["--config"],      // missing value
        vec!["--config", "x"], // missing token
        vec!["--bogus", "y"],  // unknown arg
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
            .args(&args)
            .output()
            .expect("spawn host-recorder");
        assert_eq!(out.status.code(), Some(2), "args {args:?} should exit 2");
        assert!(
            !out.stderr.is_empty(),
            "args {args:?} stderr should show usage, got: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn record_disabled_exits_0() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\nrecord:\n  enabled: false\n",
    )
    .expect("write host.yaml");
    // gating precedes token reading - even with an invalid credential, disabled must exit 0 (garbage-token order check)
    let tok_path = dir.path().join("garbage.token");
    std::fs::write(&tok_path, b"garbage").expect("write token");
    let out = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
        .args([
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            tok_path.to_str().expect("token utf8"),
        ])
        .output()
        .expect("spawn host-recorder");
    assert_eq!(out.status.code(), Some(0), "disabled should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("not enabled"), "should state it is not enabled, got: {stdout}");
}

/// Read the child process log (stdout+stderr merged into one file).
fn read_log(file: &tempfile::NamedTempFile) -> String {
    let mut out = String::new();
    file.reopen().expect("reopen log").read_to_string(&mut out).expect("read log");
    out
}

/// Poll the log until the needle appears (<= timeout_secs).
fn wait_for_secs(log: &tempfile::NamedTempFile, needle: &str, timeout_secs: u64) {
    for _ in 0..(timeout_secs * 2) {
        if read_log(log).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("needle {needle:?} not seen within {timeout_secs}s, log:\n{}", read_log(log));
}

/// Poll the log until the needle appears (<=10s).
fn wait_for(log: &tempfile::NamedTempFile, needle: &str) {
    wait_for_secs(log, needle, 10);
}

/// ffprobe single-value query (ffprobe on PATH from the pixi run environment).
fn ffprobe(args: &[&str]) -> String {
    let out = Command::new("ffprobe")
        .args(args)
        .output()
        .expect("spawn ffprobe (requires the pixi run environment)");
    assert!(out.status.success(), "ffprobe failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// E2E: capturer (real process publishing camera/cam0) -> recorder (real process subscribing + writing)
/// -> ffprobe verifies the MP4 (duration>0 + h264) -> SIGTERM: both exit gracefully with 0.
#[tokio::test]
async fn recorder_writes_mp4_from_capturer_frames() {
    cleanup_iceoryx();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let out_dir = dir.path().join("recordings");

    // host.yaml: cam0 generator 30fps + [record] enabled + a test-only out_dir
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n\
             record:\n  enabled: true\n  out_dir: \"{}\"\n",
            out_dir.display()
        ),
    )
    .expect("write host.yaml");

    // tokens: capturer=Capture (publishes camera/*), recorder=Recorder (subscribes camera/*)
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam0.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
    let (rec_tok, rec_vk) = token(Role::Recorder, &format!("recorder-{pid}"));
    let rec_path = dir.path().join("recorder.token");
    std::fs::write(&rec_path, TokenFile::encode(&rec_tok, &rec_vk)).expect("write rec token");

    // capturer process (started first: the first-frame gate depends on the publisher)
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

    // recorder process
    let rec_log = tempfile::NamedTempFile::new().expect("rec log");
    let mut recorder = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
        .args([
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            rec_path.to_str().expect("rec token utf8"),
        ])
        .stdout(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .stderr(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .spawn()
        .expect("spawn host-recorder");
    wait_for(&rec_log, "recorder ready");

    // record ~2.5s -> SIGTERM -> flush + trailer finalize
    tokio::time::sleep(Duration::from_millis(2500)).await;
    unsafe { libc::kill(recorder.id() as i32, libc::SIGTERM) };
    let st = recorder.wait().expect("wait recorder");
    assert_eq!(st.code(), Some(0), "recorder should exit gracefully with 0, got {st:?}");

    // ffprobe verification: duration > 0.5s + codec h264 (complete moov = trailer written)
    let mp4 = out_dir.join("cam0.mp4");
    let duration = ffprobe(&[
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "csv=p=0",
        mp4.to_str().expect("mp4 utf8"),
    ]);
    let codec = ffprobe(&[
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name",
        "-of",
        "csv=p=0",
        mp4.to_str().expect("mp4 utf8"),
    ]);
    eprintln!("[recorder_e2e] duration={duration}s codec={codec}");
    assert!(
        duration.parse::<f64>().unwrap_or(0.0) > 0.5,
        "duration should be > 0.5s, got: {duration}"
    );
    assert_eq!(codec.trim(), "h264", "codec should be h264, got: {codec}");

    // the capturer exits gracefully as well
    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "capturer should exit gracefully with 0, got {ct:?}");
}

/// SIGKILLs the child on assertion failure (orphan prevention; the happy path already waited -> ESRCH ignored).
struct KillOnDrop(u32);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        unsafe { libc::kill(self.0 as i32, libc::SIGKILL) };
    }
}

/// C5 crash-recovery contract (review #1 regression): the recorder survives without the capturer and warns
/// (the stall warning must appear within <=20s - before the fix the inner 10s timeout was cancelled
/// by the deck pump's 50ms outer polling -> never fired); recording resumes once the capturer starts late (frames flow -> valid MP4).
#[tokio::test]
async fn recorder_survives_missing_capturer_and_resumes() {
    cleanup_iceoryx();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let out_dir = dir.path().join("recordings");

    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n\
             record:\n  enabled: true\n  out_dir: \"{}\"\n",
            out_dir.display()
        ),
    )
    .expect("write host.yaml");

    // recorder starts first (capturer absent)
    let (rec_tok, rec_vk) = token(Role::Recorder, &format!("recorder-{pid}"));
    let rec_path = dir.path().join("recorder.token");
    std::fs::write(&rec_path, TokenFile::encode(&rec_tok, &rec_vk)).expect("write rec token");
    let rec_log = tempfile::NamedTempFile::new().expect("rec log");
    let mut recorder = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
        .args([
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            rec_path.to_str().expect("rec token utf8"),
        ])
        .stdout(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .stderr(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .spawn()
        .expect("spawn host-recorder");
    let _kill_on_drop = KillOnDrop(recorder.id()); // no orphan process on panic
    wait_for(&rec_log, "recorder ready");

    // the no-frame stall warning must appear (10s without frames + throttling -> ~10-11s; allow a 20s window)
    wait_for_secs(&rec_log, "no frames", 20);

    // capturer starts late -> recording resumes (frames flow -> MP4 duration>0)
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam0.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
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

    tokio::time::sleep(Duration::from_millis(2500)).await;
    unsafe { libc::kill(recorder.id() as i32, libc::SIGTERM) };
    let st = recorder.wait().expect("wait recorder");
    assert_eq!(st.code(), Some(0), "recorder should exit gracefully with 0, got {st:?}");

    let mp4 = out_dir.join("cam0.mp4");
    let duration = ffprobe(&[
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "csv=p=0",
        mp4.to_str().expect("mp4 utf8"),
    ]);
    let codec = ffprobe(&[
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name",
        "-of",
        "csv=p=0",
        mp4.to_str().expect("mp4 utf8"),
    ]);
    eprintln!("[recorder_e2e] stall-recovery duration={duration}s codec={codec}");
    assert!(
        duration.parse::<f64>().unwrap_or(0.0) > 0.5,
        "recording should resume after the late capturer start, duration got {duration}"
    );
    assert_eq!(codec.trim(), "h264", "codec should be h264, got: {codec}");

    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "capturer should exit gracefully with 0, got {ct:?}");
}
