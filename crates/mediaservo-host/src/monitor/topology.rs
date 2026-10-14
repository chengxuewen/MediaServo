//! Topology monitoring core (Task E1, D-H4: declarative expected + discovered actual).
//!
//! Expected state: host.yaml declarations (N capturers + N streamers + fixed
//! recorder/controller/emergency/audio/agent processes + one `camera/<id>` publish topic per camera).
//! Actual state: oxmgr list process liveness + FrameBus cross-process publisher enumeration
//! (Momus MEDIUM-2 option ①, `mediaservo_link::FrameBus::list_topics`, iceoryx2 service registry).
//! Mismatch list + grace period suppresses the startup window (D-H14: startup window vs fault).
//!
//! Data consumers: E2 (flow stats) / E3 (signal state + Server reporting) reuse the
//! [`TopologySnapshot`] structure.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use mediaservo_link::FrameBus;
use mediaservo_link::FrameTopic;

use crate::translate;

/// Expected process missing from oxmgr or not running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    ProcessMissing {
        name: String,
    },
    /// Expected camera topic has no active connection (publisher process alive but no publisher on the bus).
    PublisherMissing {
        topic: String,
    },
}

/// One process entry from oxmgr list --json (unknown fields ignored).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct OxProcess {
    pub name: String,
    pub status: String,
}

/// Parse `oxmgr list --json` output (superset-tolerant; serde ignores unknown fields).
pub fn parse_oxmgr_json(json: &str) -> Result<Vec<OxProcess>, String> {
    serde_json::from_str(json).map_err(|e| format!("oxmgr list --json parse failed: {e}"))
}

/// Expected-vs-actual diff (pure function, covered by E1 unit tests).
///
/// Process expectation: present in oxmgr with status == "running"; topic expectation: an active
/// connection in FrameBus discovery (the caller already filtered topics with alive_nodes >= 1).
pub fn diff(
    expected_procs: &[String],
    expected_topics: &[FrameTopic],
    actual_procs: &[OxProcess],
    actual_topics: &[FrameTopic],
) -> Vec<Mismatch> {
    let mut out = Vec::new();
    for name in expected_procs {
        let running = actual_procs.iter().any(|p| &p.name == name && p.status == "running");
        if !running {
            out.push(Mismatch::ProcessMissing { name: name.clone() });
        }
    }
    for topic in expected_topics {
        if !actual_topics.iter().any(|t| t == topic) {
            out.push(Mismatch::PublisherMissing { topic: topic.as_str().into() });
        }
    }
    out
}

/// oxmgr CLI wrapper (process-level actual-state data source).
pub struct OxmgrClient {
    bin: PathBuf,
}

impl OxmgrClient {
    /// Default construction: find oxmgr on PATH, fall back to `~/.local/bin/oxmgr` (dev-machine convention).
    pub fn new() -> Self {
        Self::with_bin(find_oxmgr())
    }

    /// Explicit oxmgr path (test injection).
    pub fn with_bin(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// `oxmgr list --json` → process list. Returns Err on failure (caller logs, C15).
    pub fn list(&self) -> Result<Vec<OxProcess>, String> {
        let out = Command::new(&self.bin)
            .args(["list", "--json"])
            .output()
            .map_err(|e| format!("oxmgr list exec failed ({}): {e}", self.bin.display()))?;
        if !out.status.success() {
            return Err(format!(
                "oxmgr list non-zero exit: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        parse_oxmgr_json(&String::from_utf8_lossy(&out.stdout))
    }
}

impl Default for OxmgrClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Topology monitor: periodic collection (host-agent 5s loop) + snapshot.
pub struct TopologyMonitor {
    host_toml: String,
    grace: Duration,
    started: Instant,
    oxmgr: OxmgrClient,
}

/// One topology snapshot (data basis for E2/E3).
#[derive(Debug, Clone)]
pub struct TopologySnapshot {
    /// Expected process names (host.yaml declarations, incl. fixed processes and instances).
    pub expected_processes: Vec<String>,
    /// Actual processes (oxmgr list, incl. status).
    pub actual_processes: Vec<OxProcess>,
    /// Actual active topics (FrameBus discovery, alive_nodes >= 1).
    pub actual_topics: Vec<FrameTopic>,
    /// Mismatch list (raw; the caller suppresses reporting during the grace window).
    pub mismatches: Vec<Mismatch>,
    /// Whether the startup window is still in effect (D-H14).
    pub grace_active: bool,
}

/// Default startup window (D-H14: suppress missing reports for N seconds after process start).
pub const DEFAULT_GRACE: Duration = Duration::from_secs(15);

impl TopologyMonitor {
    /// Default grace 15s, start = now.
    pub fn new(host_toml: String) -> Self {
        Self::new_at(host_toml, DEFAULT_GRACE, Instant::now())
    }

    /// Explicit grace (short window injected by tests), start = now.
    pub fn new_with_grace(host_toml: String, grace: Duration) -> Self {
        Self::new_at(host_toml, grace, Instant::now())
    }

    /// Explicit grace + start point (E1 review: host-agent starts at main entry, covering the slow gateway connect window).
    pub fn new_at(host_toml: String, grace: Duration, started: Instant) -> Self {
        Self { host_toml, grace, started, oxmgr: OxmgrClient::new() }
    }

    /// For tests: inject the oxmgr client (no daemon dependency).
    pub fn new_for_test(host_toml: String, grace: Duration) -> Self {
        Self {
            host_toml,
            grace,
            started: Instant::now(),
            oxmgr: OxmgrClient::with_bin(PathBuf::from("/nonexistent/oxmgr")),
        }
    }

    /// Whether the startup window is still in effect.
    pub fn grace_active(&self) -> bool {
        self.started.elapsed() < self.grace
    }

    /// Collect one topology snapshot: expected (host.yaml) + actual (oxmgr + FrameBus discovery) → diff.
    pub fn collect(&self) -> TopologySnapshot {
        let expected_processes =
            translate::expected_process_names(&self.host_toml).unwrap_or_else(|e| {
                tracing::warn!("host.yaml expected-process parse failed: {e}");
                Vec::new()
            });
        let expected_topics: Vec<FrameTopic> = translate::camera_configs(&self.host_toml)
            .unwrap_or_else(|e| {
                tracing::warn!("host.yaml camera parse failed: {e}");
                Vec::new()
            })
            .into_iter()
            .map(|src| FrameTopic::new(format!("camera/{}", src.id)))
            .collect();

        let actual_processes = self.oxmgr.list().unwrap_or_else(|e| {
            tracing::warn!("oxmgr list failed: {e}");
            Vec::new()
        });
        // Discovered actual: only topics with active connections (alive_nodes >= 1) count
        let actual_topics: Vec<FrameTopic> = FrameBus::list_topics()
            .unwrap_or_else(|e| {
                tracing::warn!("FrameBus discovery failed: {e}");
                Vec::new()
            })
            .into_iter()
            .filter(|t| t.alive_nodes >= 1)
            .map(|t| t.topic)
            .collect();

        let mismatches =
            diff(&expected_processes, &expected_topics, &actual_processes, &actual_topics);
        TopologySnapshot {
            expected_processes,
            actual_processes,
            actual_topics,
            mismatches,
            grace_active: self.grace_active(),
        }
    }
}

/// Find oxmgr on PATH; on miss fall back to `$HOME/.local/bin/oxmgr`.
fn find_oxmgr() -> PathBuf {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let cand = dir.join("oxmgr");
            if cand.is_file() {
                return cand;
            }
        }
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".local/bin/oxmgr")
}
