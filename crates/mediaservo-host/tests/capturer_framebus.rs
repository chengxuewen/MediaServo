//! Task C1: host-capturer process tests - capture -> FrameBus publish (I420).
//!
//! - `bad_args_exit_2_with_usage`: missing/bad args -> exit 2 + usage hint on stderr
//! - `capturer_publishes_i420_frames_to_framebus`: real binary + generated token file ->
//!   subscribe `camera/cam0` -> assert 3 frames (correct meta, payload = 1280x720 I420) ->
//!   SIGTERM -> exit code 0
//!
//! Prerequisite (C25 revised): `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*` (the iceoryx2 0.9.3 runtime
//! root is /tmp/iceoryx2; leftover services make a second open of the fixed topic camera/cam0 stick in SystemInFlux,
//! so a full cleanup is mandatory before re-running).

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameTopic, NodeAcl, NodeId,
    Role, TokenFile,
};

// Test Ed25519 keypair (generated with openssl, test-only; same source as link/deck tests).
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

#[test]
fn bad_args_exit_2_with_usage() {
    for args in [
        vec![],                   // all missing
        vec!["--camera"],         // missing value
        vec!["--camera", "cam0"], // missing config/token
        vec!["--bogus", "x"],     // unknown arg
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
            .args(&args)
            .output()
            .expect("spawn host-capturer");
        assert_eq!(out.status.code(), Some(2), "args {args:?} should exit 2");
        assert!(
            !out.stderr.is_empty(),
            "args {args:?} stderr should show usage, got: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// mode dispatch (contract): camera/desktop/subscriber capture unimplemented -> print "unsupported" and exit;
/// generator is really implemented (frame evidence: capturer_publishes_i420_frames_to_framebus).
#[test]
fn unsupported_modes_exit_1_with_clear_error() {
    for (mode, needle) in
        [("camera", "camera"), ("desktop", "desktop"), ("subscriber", "subscriber")]
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg_path = dir.path().join("host.yaml");
        std::fs::write(
            &cfg_path,
            format!("sources:\n  - id: \"cam0\"\n    mode: \"{mode}\"\n    fps: 30\n"),
        )
        .expect("write host.yaml");
        let tok_path = dir.path().join("cam0.token");
        std::fs::write(&tok_path, b"garbage").expect("write token");
        let out = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
            .args([
                "--camera",
                "cam0",
                "--config",
                cfg_path.to_str().expect("cfg utf8"),
                "--token",
                tok_path.to_str().expect("token utf8"),
            ])
            .output()
            .expect("spawn host-capturer");
        assert_eq!(out.status.code(), Some(1), "mode={mode} should exit 1");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("unsupported"),
            "mode={mode} should report unsupported, got: {stderr}"
        );
        assert!(
            stderr.contains(needle),
            "mode={mode} error should contain category {needle}, got: {stderr}"
        );
    }
}

#[tokio::test]
async fn capturer_publishes_i420_frames_to_framebus() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    width: 640\n    height: 480\n    fps: 30\n",
    )
    .expect("write host.yaml");

    // Generate the capturer token file (Role::Capture -> may publish camera/*)
    let (tok, vk) = token(Role::Capture, "capture-cam0");
    let tok_path = dir.path().join("cam0.token");
    std::fs::write(&tok_path, TokenFile::encode(&tok, &vk)).expect("write token file");

    // Subscriber side (Recorder role -> may subscribe camera/*): attach + subscribe first, then start the capturer
    let (sub_tok, sub_vk) = token(Role::Recorder, "test-recorder");
    let bus = FrameBus::attach("", &sub_tok, &sub_vk).expect("subscriber attach");
    let topic = FrameTopic::new("camera/cam0");
    let stream = bus.subscribe(&topic).expect("subscribe camera/cam0");

    let log = tempfile::NamedTempFile::new().expect("log file");
    let mut child = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
        .args([
            "--camera",
            "cam0",
            "--config",
            cfg_path.to_str().expect("cfg path utf8"),
            "--token",
            tok_path.to_str().expect("token path utf8"),
        ])
        .stdout(Stdio::from(log.reopen().expect("reopen log")))
        .stderr(Stdio::from(log.reopen().expect("reopen log")))
        .spawn()
        .expect("spawn host-capturer");

    // Receive 3 frames, assert meta + payload
    let mut last_seq: Option<u64> = None;
    let mut ts_positive = false;
    for _ in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(10), stream.recv())
            .await
            .expect("recv timeout - capturer published no frames")
            .expect("frame");
        let m = frame.meta();
        assert_eq!(m.width, 640, "width should be consumed from config (width: 640)");
        assert_eq!(m.height, 480, "height should be consumed from config (height: 480)");
        assert_eq!(m.format, 1, "format should be I420(1)");
        assert_eq!(m.version, 1);
        assert_eq!(
            frame.payload().len(),
            640 * 480 * 3 / 2,
            "payload should be the compact I420 size"
        );
        if let Some(prev) = last_seq {
            assert!(m.seq > prev, "seq should strictly increase: {prev} -> {}", m.seq);
        }
        last_seq = Some(m.seq);
        ts_positive |= m.ts_mono_ns > 0;
    }
    assert!(ts_positive, "ts_mono_ns should be non-zero (C17 monotonic clock)");

    // ready line (evidence startup completed)
    let mut out = String::new();
    log.reopen().expect("reopen log").read_to_string(&mut out).expect("read log");
    assert!(out.contains("capturer ready"), "stdout missing the ready line, got: {out:?}");
    assert!(
        out.contains("source=generator"),
        "the ready line should identify the generator source, got: {out:?}"
    );

    // SIGTERM -> graceful exit (exit code 0)
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().expect("wait host-capturer");
    assert_eq!(status.code(), Some(0), "expected graceful exit 0, got {status:?}");
}
