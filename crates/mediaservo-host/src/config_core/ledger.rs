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

/// T1b5 (k5): render shape for `host status` — one fact per line, fixed columns, plain text
/// (no box drawing). An all-empty ledger (file absent / corrupt / never applied —
/// [`Ledger::load`] collapses all three to default) renders the single pre-T1\u{2033} line.
pub fn config_status_lines(led: &Ledger, file_hash_now: &str) -> Vec<String> {
    if led.running.is_none()
        && led.last_push.is_none()
        && led.pending_apply.is_none()
        && led.history.is_empty()
    {
        return vec![
            "config: no config-state.json yet (pre-T1\u{2033} state or never applied)".to_string(),
        ];
    }
    let mut out = Vec::new();
    match &led.running {
        Some(r) => out.push(format!(
            "config:  running  = {}  (origin {}, actor {}, {})",
            short12(&r.hash),
            r.origin,
            r.actor.as_deref().unwrap_or("-"),
            date16(&r.at)
        )),
        None => out.push("config:  running  = -  (nothing applied yet)".to_string()),
    }
    let file_note = if led.pending_apply.as_ref().is_some_and(|p| p.hash == file_hash_now) {
        "BOOT-PENDING: apply in progress"
    } else if led.running.as_ref().is_some_and(|r| r.hash == file_hash_now) {
        "= matches"
    } else {
        "PENDING: edited, not applied"
    };
    out.push(format!("         file     = {}  [{file_note}]", short12(file_hash_now)));
    match &led.last_push {
        Some(p) => {
            let note = match &led.running {
                Some(r) if r.hash == p.hash => "  [consistent]",
                Some(_) => "  [CONFLICT: site changed since push]",
                None => "",
            };
            out.push(format!("         last push= {}{note}", short12(&p.hash)));
        }
        None => out.push("         last push= -  (no cloud push recorded)".to_string()),
    }
    if !led.history.is_empty() {
        let items: Vec<String> = led
            .history
            .iter()
            .rev()
            .take(3)
            .map(|h| {
                format!(
                    "{} {}:{} {}",
                    hm(&h.at),
                    h.origin,
                    h.actor.as_deref().unwrap_or("-"),
                    trunc(&h.result, 48)
                )
            })
            .collect();
        out.push(format!("         history: {}", items.join("  |  ")));
    }
    out
}

/// Display shortening of an RFC3339 stamp: "2026-10-16T12:34:56+00:00" -> "2026-10-16 12:34".
/// Never parses — a malformed stamp falls back to the raw string.
fn date16(ts: &str) -> String {
    ts.get(..16).unwrap_or(ts).replace('T', " ")
}

/// "2026-10-16T12:34:56+00:00" -> "12:34": history rows carry same-day context only, the
/// date lives on the running row.
fn hm(ts: &str) -> &str {
    ts.get(11..16).unwrap_or(ts)
}

/// Char-boundary-safe display truncation (refusal reasons can run long).
fn trunc(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
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

    fn r_row(hash: &str, origin: &str, actor: Option<&str>) -> Running {
        Running {
            hash: hash.to_string(),
            at: "2026-10-16T12:34:56+00:00".to_string(),
            origin: origin.to_string(),
            actor: actor.map(str::to_string),
        }
    }

    #[test]
    fn config_status_lines_absent_ledger_is_one_line() {
        let lines = config_status_lines(&Ledger::default(), "f");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("no config-state.json yet"));
    }

    #[test]
    fn config_status_lines_matches_shape() {
        let h = "a".repeat(64);
        let led = Ledger { running: Some(r_row(&h, "cli", Some("max"))), ..Ledger::default() };
        let text = config_status_lines(&led, &h).join("\n");
        assert!(text.contains("aaaaaaaaaaaa")); // short12 display form
        assert!(text.contains("= matches"));
        assert!(text.contains("actor max"));
        assert!(text.contains("2026-10-16 12:34"));
        assert!(!text.contains("CONFLICT"));
        assert!(!text.contains("PENDING"));
    }

    #[test]
    fn config_status_lines_edited_file_is_pending() {
        let led =
            Ledger { running: Some(r_row(&"a".repeat(64), "cli", None)), ..Ledger::default() };
        let text = config_status_lines(&led, &"b".repeat(64)).join("\n");
        assert!(text.contains("PENDING: edited, not applied"));
        assert!(text.contains("no cloud push recorded"));
    }

    #[test]
    fn config_status_lines_boot_pending_when_pending_apply_matches_file() {
        let h = "c".repeat(64);
        let led = Ledger {
            running: Some(r_row(&"d".repeat(64), "cli", None)),
            pending_apply: Some(PendingApply {
                hash: h.clone(),
                at: "2026-10-16T13:00:00+00:00".to_string(),
            }),
            ..Ledger::default()
        };
        let text = config_status_lines(&led, &h).join("\n");
        assert!(text.contains("BOOT-PENDING: apply in progress"));
    }

    #[test]
    fn config_status_lines_conflict_and_history_shape() {
        let mut led = Ledger {
            running: Some(r_row(&"e".repeat(64), "push", None)),
            last_push: Some(LastPush {
                hash: "f".repeat(64),
                actor: None,
                at: "2026-10-16T12:00:00+00:00".to_string(),
            }),
            ..Ledger::default()
        };
        led.push_history(&"e".repeat(64), "local", Some("max"), "applied");
        led.push_history(
            &"e".repeat(64),
            "push",
            None,
            "refused-residue: very long reason that should be truncated well past the display budget",
        );
        let text = config_status_lines(&led, &"e".repeat(64)).join("\n");
        assert!(text.contains("CONFLICT: site changed since push"));
        assert!(text.contains("last push= ffffffffffff"));
        let hist = text.lines().find(|l| l.contains("history:")).unwrap();
        assert!(hist.contains("push:- refused-residue"));
        assert!(hist.contains("local:max applied"));
        // newest first
        assert!(hist.find("push:- refused").unwrap() < hist.find("local:max applied").unwrap());
    }
}
