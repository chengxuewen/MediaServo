//! Task E1: topology monitor e2e - closed loop with real data sources.
//!
//! (1) start/kill an oxmgr process -> the snapshot's actual state changes + the diff shows a mismatch (process level);
//! (2) publish on FrameBus -> collect()'s actual_topics see it (discovered actual, publisher level).
//!
//! Requires: the oxmgr CLI (PATH or ~/.local/bin, daemon auto-spawned) + the iceoryx2 runtime.
//! Prerequisite (C25): `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*`.
//! Skipped when oxmgr is unavailable (the remaining tests run in CI without a daemon).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use mediaservo_host::monitor::topology::{Mismatch, OxmgrClient, TopologyMonitor, diff};
use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

/// Look up oxmgr on PATH, falling back to ~/.local/bin (same logic as find_oxmgr in topology.rs).
fn oxmgr_bin() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let cand = dir.join("oxmgr");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let fallback =
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".local/bin/oxmgr");
    fallback.is_file().then_some(fallback)
}

/// Cleanup guard for oxmgr-managed processes (removed even when an assertion fails).
struct OxCleanup(PathBuf, String);
impl Drop for OxCleanup {
    fn drop(&mut self) {
        let _ = Command::new(&self.0).args(["delete", &self.1]).output();
    }
}

#[test]
fn oxmgr_process_lifecycle_reflected_in_snapshot() {
    let Some(bin) = oxmgr_bin() else {
        eprintln!("oxmgr unavailable, skipping");
        return;
    };
    let name = format!("e1-topo-{}", std::process::id());

    let out = Command::new(&bin)
        .args(["start", "--name", &name, "sleep 300"])
        .output()
        .expect("oxmgr start executes");
    assert!(out.status.success(), "oxmgr start failed: {}", String::from_utf8_lossy(&out.stderr));
    let _guard = OxCleanup(bin.clone(), name.clone());

    // Wait until registered as running (small window between oxmgr registration and process spawn)
    let ox = OxmgrClient::with_bin(bin.clone());
    let mut procs = vec![];
    for _ in 0..50 {
        procs = ox.list().unwrap_or_default();
        if procs.iter().any(|p| p.name == name && p.status == "running") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        procs.iter().any(|p| p.name == name && p.status == "running"),
        "should be running after start: {procs:#?}"
    );

    // Snapshot: actual state contains the process -> no mismatch
    let monitor = TopologyMonitor::new_with_grace(String::new(), Duration::ZERO);
    let snap = monitor.collect();
    assert!(snap.actual_processes.iter().any(|p| p.name == name && p.status == "running"));
    assert!(diff(std::slice::from_ref(&name), &[], &snap.actual_processes, &[]).is_empty());

    // Kill -> actual state misses it -> mismatch appears (grace=0 takes effect immediately)
    let out = Command::new(&bin).args(["stop", &name]).output().expect("oxmgr stop executes");
    assert!(out.status.success(), "oxmgr stop failed: {}", String::from_utf8_lossy(&out.stderr));
    for _ in 0..50 {
        procs = ox.list().unwrap_or_default();
        if !procs.iter().any(|p| p.name == name && p.status == "running") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let snap = monitor.collect();
    assert!(
        diff(std::slice::from_ref(&name), &[], &snap.actual_processes, &[])
            .contains(&Mismatch::ProcessMissing { name: name.clone() }),
        "after stop the process should be reported missing: {snap:#?}"
    );
}

#[tokio::test]
async fn publisher_discovery_visible_in_snapshot() {
    // Real publisher -> collect()'s actual_topics should show it (discovered-actual closed loop)
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new("capture-e1e2"), Role::Capture);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    let topic = FrameTopic::new(format!("camera/e1e2/{}/raw", std::process::id()));
    bus.publish(&topic, &[1u8, 2, 3], &FrameMeta::default()).unwrap();

    let monitor = TopologyMonitor::new_with_grace(String::new(), Duration::ZERO);
    let snap = monitor.collect();
    assert!(
        snap.actual_topics.iter().any(|t| t == &topic),
        "the publisher topic should appear in the snapshot's actual_topics: {snap:#?}"
    );
}
