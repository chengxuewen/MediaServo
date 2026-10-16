//! run/config-state.json — the vehicle-side mini-git ledger (annex §9.3).
//!
//! Three-fact model (§9.1): `file.hash` is computed on demand from host.yaml bytes (never
//! trusted from a memo), `running` says what IS running (written by the core at step7),
//! `last_push` says what the cloud believes it set (written by the push bell — batch3).
//! Pairwise comparison is the whole truth table: pending / site-changed / conflict.
//!
//! Absent or corrupt files load as the empty default ledger: losing the ledger costs
//! provenance, never correctness (the core must not panic on its own bookkeeping).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Config identity = full sha256 hex over the complete host.yaml bytes (§9.2: the version
/// lives OUTSIDE the editable file — self-reference and erasability ruled out).
pub fn file_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Display form for the status surface: the first 12 hex chars, never an identity.
pub fn short12(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// RFC3339 UTC stamp helper (shared by ledger rows and the core's pending_apply).
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// What IS running — written at core step7 on success.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Running {
    pub hash: String,
    pub at: String,
    /// "push" | "local" | "cli" — writer self-report (§9.2).
    pub origin: String,
    #[serde(default)]
    pub actor: Option<String>,
}

/// What the cloud believes it set — written by the push bell (batch3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastPush {
    pub hash: String,
    #[serde(default)]
    pub actor: Option<String>,
    pub at: String,
}

/// Prepare marker written BEFORE activation (annex §2 step6): survives a crash between
/// staging-rename and reconcile so the boot bell can re-verify instead of re-tearing-down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingApply {
    pub hash: String,
    pub at: String,
}

/// The three lamps (§9.1). `conflict` is bell-owned; the core only touches pending/error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Flags {
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub error: bool,
    #[serde(default)]
    pub conflict: bool,
}

/// One append-only provenance row; `result` carries the refusal detail when not "applied".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub hash: String,
    pub origin: String,
    #[serde(default)]
    pub actor: Option<String>,
    pub at: String,
    pub result: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub running: Option<Running>,
    #[serde(default)]
    pub last_push: Option<LastPush>,
    #[serde(default)]
    pub pending_apply: Option<PendingApply>,
    #[serde(default)]
    pub flags: Flags,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

/// ponytail: rolling window cap 64, oldest dropped (annex §9.3) — not an archive.
const HISTORY_CAP: usize = 64;

impl Ledger {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("run").join("config-state.json")
    }

    /// Read; absent or corrupt => empty default (never panics).
    pub fn load(dir: &Path) -> Ledger {
        std::fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Atomic write (tmp + rename in the same dir — readers never see half-written state).
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let path = Self::path(dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {} failed: {e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize {} failed: {e}", path.display()))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &text).map_err(|e| format!("write {} failed: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("rename {} failed: {e}", path.display()))
    }

    /// Append a provenance row (cap HISTORY_CAP, oldest dropped).
    pub fn push_history(&mut self, hash: &str, origin: &str, actor: Option<&str>, result: &str) {
        self.history.push(HistoryEntry {
            hash: hash.to_string(),
            origin: origin.to_string(),
            actor: actor.map(str::to_string),
            at: now_rfc3339(),
            result: result.to_string(),
        });
        if self.history.len() > HISTORY_CAP {
            let excess = self.history.len() - HISTORY_CAP;
            self.history.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_hash_is_stable_sha256_hex() {
        let h = file_hash(b"");
        assert_eq!(h.len(), 64);
        assert!(h.starts_with("e3b0c442")); // published sha256 of empty input
        assert_eq!(h, file_hash(b""));
        assert_ne!(h, file_hash(b"x"));
    }

    #[test]
    fn short12_is_display_prefix_only() {
        assert_eq!(short12("0123456789abcdef"), "0123456789ab");
        assert_eq!(short12("abc"), "abc");
    }

    #[test]
    fn ledger_roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let led = Ledger {
            running: Some(Running {
                hash: file_hash(b"cfg"),
                at: now_rfc3339(),
                origin: "push".into(),
                actor: Some("cockpit-admin".into()),
            }),
            flags: Flags { pending: true, ..Flags::default() },
            ..Ledger::default()
        };
        led.save(dir.path()).unwrap();
        assert_eq!(Ledger::load(dir.path()), led);
    }

    #[test]
    fn ledger_history_caps_at_64_dropping_oldest() {
        let mut led = Ledger::default();
        for i in 0..70 {
            led.push_history("h", "cli", None, &format!("r{i}"));
        }
        assert_eq!(led.history.len(), 64);
        assert_eq!(led.history[0].result, "r6");
        assert_eq!(led.history[63].result, "r69");
    }

    #[test]
    fn load_tolerates_absent_and_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Ledger::load(dir.path()), Ledger::default()); // never written
        std::fs::create_dir_all(dir.path().join("run")).unwrap();
        std::fs::write(Ledger::path(dir.path()), "{ not json").unwrap();
        assert_eq!(Ledger::load(dir.path()), Ledger::default()); // corrupt
    }
}
