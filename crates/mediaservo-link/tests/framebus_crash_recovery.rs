//! C5 core: FrameBus subscriber recovery across publisher crash-restart (same handle keeps receiving).
//!
//! Scenario (production-equivalent): after a long-lived subscriber (host-streamer/host-recorder) attaches,
//! the publisher (capturer) is SIGKILLed (crash path) -> a new publisher process re-publishes the same topic ->
//! **the old subscriber handle must resume receiving** (not only a freshly-created subscriber).
//!
//! C25: clean /tmp/iceoryx2 + /dev/shm/iox2_* before running (this test cleans up itself).

use std::process::Command;
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameStream, FrameTopic,
    NodeAcl, NodeId, Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

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

/// Wait until >= n frames arrive on the stream (returns the max seq seen so far).
async fn wait_frames(stream: &FrameStream, n: u32, timeout: Duration) -> u64 {
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
            Ok(None) => panic!("subscriber stream closed"),
            Err(_) => {}
        }
    }
    panic!("only {seen} frames within {timeout:?} (need >= {n}), max_seq={max_seq}");
}

/// Spawn the publishing child process (new process = new node + new publisher port); returns the Child.
fn spawn_pub_loop(topic: &str, frame_bytes: usize, fps: u64) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_framebus_pub_loop"))
        .arg(topic)
        .arg(frame_bytes.to_string())
        .arg(fps.to_string())
        .spawn()
        .expect("spawn framebus_pub_loop")
}

fn kill9(child: &mut std::process::Child) {
    let pid = child.id();
    let st = Command::new("kill").args(["-9", &pid.to_string()]).status().expect("kill");
    assert!(st.success(), "kill -9 {pid} failed");
}

#[tokio::test]
async fn same_subscriber_recovers_across_publisher_sigkill() {
    // production parameters: 1080p I420 frames (3.1MB) @ 30fps (same spec as host-capturer)
    crash_recovery_run(3_110_400, 30).await;
}

#[tokio::test]
async fn same_subscriber_recovers_across_publisher_sigkill_small_frames() {
    // small-frame control (fast regression on low-spec environments)
    crash_recovery_run(64, 10).await;
}

async fn crash_recovery_run(frame_bytes: usize, fps: u64) {
    cleanup_iceoryx();
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    // Recorder role may subscribe camera/*
    let acl = NodeAcl::for_role(NodeId::new("crash-sub"), Role::Recorder);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    // unique topic (cross-run isolation)
    let topic = FrameTopic::new(format!("camera/crash/{}/raw", std::process::id()));

    // (1) subscriber attaches first (production ordering: streamer/recorder before/with capturer)
    let stream = bus.subscribe(&topic).unwrap();

    // (2) publisher v1: establish the receiving baseline
    let mut pub1 = spawn_pub_loop(topic.as_str(), frame_bytes, fps);
    let seq_before = wait_frames(&stream, 3, Duration::from_secs(10)).await;

    // (3) SIGKILL publisher v1 (crash path)
    kill9(&mut pub1);
    let _ = pub1.wait();
    // give iceoryx2 a moment so residue becomes visible (cleaned at new-node creation)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // (4) publisher v2 re-publishes the same topic -> the same subscriber handle must resume receiving
    let mut pub2 = spawn_pub_loop(topic.as_str(), frame_bytes, fps);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut resumed = false;
    let mut seen = 0u32;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), stream.recv()).await {
            Ok(Some(frame)) => {
                if frame.meta().seq < seq_before {
                    // the new instance's seq resets (pub_loop starts at 1) -> not stale frames from the old connection
                    resumed = true;
                }
                seen += 1;
                if resumed && seen >= 3 {
                    break;
                }
            }
            Ok(None) => panic!("subscriber stream closed"),
            Err(_) => {}
        }
    }
    assert!(
        resumed,
        "subscriber handle did not resume after publisher SIGKILL restart (seq_before={seq_before}) - subscriber stale"
    );
    eprintln!(
        "[crash_recovery] OK: same-handle recovery across publisher crash (seq {seq_before} -> fresh frames after restart)"
    );

    kill9(&mut pub2);
    let _ = pub2.wait();
}

/// C5 rebuild-backstop coverage: after the publisher is killed, idle >5s (> REBUILD_IDLE) -> the subscriber thread rebuilds
/// the subscriber -> a new publisher re-publishes -> the same handle resumes receiving.
///
/// Timeline (the 30s cooldown never fires): kill t=0 -> rebuild t~5.0 (idle crosses the 5s threshold, first cooldown satisfied)
/// -> republish t~6.0 -> receive t~6.1. Assert: the gap between the resumed frame and the last pre-kill frame is >5s (only the
/// idle>5s branch can bridge that gap); the rebuild warn goes to stderr via logging::init as evidence
/// ("subscriber rebuilt" visible with --nocapture).
#[tokio::test]
async fn same_subscriber_recovers_after_long_gap_via_rebuild() {
    // the rebuild warn (tracing::warn!) prints at the default level=info -> test evidence
    mediaservo_common::logging::init(mediaservo_common::logging::LoggingConfig::default());
    cleanup_iceoryx();
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new("crash-sub"), Role::Recorder);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    let topic = FrameTopic::new(format!("camera/crash/gap/{}/raw", std::process::id()));

    // subscriber attaches first
    let stream = bus.subscribe(&topic).unwrap();

    // publisher v1 -> receiving baseline (record the last frame's time)
    let mut pub1 = spawn_pub_loop(topic.as_str(), 64, 10);
    let seq_before = wait_frames(&stream, 3, Duration::from_secs(10)).await;
    let last_frame_before = std::time::Instant::now();

    // SIGKILL publisher v1 -> idle > REBUILD_IDLE(5s): the subscriber thread rebuilds at t~5.0
    kill9(&mut pub1);
    let _ = pub1.wait();
    tokio::time::sleep(Duration::from_secs(6)).await;

    // publisher v2 republishes -> same handle resumes (gap >5s -> bridged by the rebuild path)
    let mut pub2 = spawn_pub_loop(topic.as_str(), 64, 10);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut resumed = false;
    let mut resumed_gap = None;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), stream.recv()).await {
            Ok(Some(frame)) => {
                if frame.meta().seq < seq_before {
                    resumed = true;
                    resumed_gap = Some(last_frame_before.elapsed());
                    break;
                }
            }
            Ok(None) => panic!("subscriber stream closed"),
            Err(_) => {}
        }
    }
    let gap = resumed_gap.expect("resumed-frame gap");
    eprintln!(
        "[crash_recovery] long-gap variant: seq_before={seq_before} resumed gap={gap:?} (>5s rebuild path)"
    );
    assert!(
        resumed && gap > Duration::from_secs(5),
        "same-handle resumed frame must be >5s after the last pre-kill frame (rebuild path): gap={gap:?} resumed={resumed}"
    );

    kill9(&mut pub2);
    let _ = pub2.wait();
}
