//! Emergency-stop plane (Task F2) -- EmergencyActuator latch + strong audit.
//!
//! Semantics (D-H3/D-H11 landing):
//! - The e-stop channel = a separate process (host-emergency) + a separate PC (a controller PC crash does not affect
//!   the e-stop) + a single DC label "emergency" (reliable-ordered; D-H3 requires the e-stop to be reliable and ordered)
//! - Only one command (no parameterized control): `{"seq": N, "cmd": "stop"}` ([`crate::control`]
//!   envelope reused, F1 wire); `payload` is ignored
//! - `EmergencyActuator` is conceptually a one-shot latch: the first trigger arms it,
//!   later triggers do not re-arm but are still recorded (audit) and ack with `latched=false`
//! - Strong audit (D-H11 local side): a JSONL append-only file, one line per trigger
//!   `{"ts","source","seq","cmd","latched","trigger_count"}`
//!   `source` = `"dc"` (remote e-stop over DC) | `"local"` (local fallback, F2 = SIGUSR1);
//!   "who" (cockpit-side identity) is carried by the server-side session/authorization layer (Phase G, D-H11 authorization matrix)
//! - Local fallback interface: the same trigger() trait surface; the CAN/GPIO implementations replace it after Phase I
//!   (Stub = latch + audit file; its behavior is what this module demonstrates)

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The only command of the emergency channel (EMERGENCY STOP).
pub const EMERGENCY_STOP: &str = "stop";

/// Source of an e-stop trigger (the audit source field + the ack source field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencySource {
    /// Remote e-stop over the emergency DC (issued by the cockpit side).
    Dc,
    /// Local fallback trigger (F2 = SIGUSR1; a network-independent path, D-H3).
    Local,
}

impl EmergencySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmergencySource::Dc => "dc",
            EmergencySource::Local => "local",
        }
    }
}

/// Result of one trigger (data source of the ack result).
#[derive(Debug, Clone)]
pub struct EmergencyTrigger {
    /// Whether this trigger armed the latch (first = true, later = false).
    pub latched: bool,
    /// Cumulative trigger count (audit + ack; repeated e-stops stay observable).
    pub trigger_count: u64,
    /// Audit write result (an Err is already logged -- C15: never silent, though the e-stop itself already happened).
    pub audit: Result<(), String>,
}

/// E-stop actuator interface -- one-shot latch semantics.
/// Implementations must log (C15); error info goes back to the peer (ACK semantics, never silent).
pub trait EmergencyActuator: Send + Sync {
    /// Trigger the e-stop latch. `seq` is the DC envelope sequence number (None for local triggers, which the audit distinguishes).
    fn trigger(
        &self,
        source: EmergencySource,
        seq: Option<u64>,
    ) -> Result<EmergencyTrigger, String>;
}

/// Stub e-stop actuator (F2 stage): latch + JSONL strong audit file.
/// The real CAN/GPIO implementations replace it after Phase I (interface unchanged; the audit stays).
pub struct StubEmergencyActuator {
    latch: AtomicBool,
    count: AtomicU64,
    audit: Mutex<File>,
}

impl StubEmergencyActuator {
    /// Open (create + append) the audit file; failure = exit at startup (no strong audit, no run).
    pub fn new(audit_path: &Path) -> Result<Self, String> {
        let file = File::options()
            .create(true)
            .append(true)
            .open(audit_path)
            .map_err(|e| format!("failed to open audit file {}: {e}", audit_path.display()))?;
        Ok(Self {
            latch: AtomicBool::new(false),
            count: AtomicU64::new(0),
            audit: Mutex::new(file),
        })
    }

    fn append_audit(
        &self,
        source: EmergencySource,
        seq: Option<u64>,
        latched: bool,
        trigger_count: u64,
    ) -> Result<(), String> {
        let line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "source": source.as_str(),
            "seq": seq,
            "cmd": EMERGENCY_STOP,
            "latched": latched,
            "trigger_count": trigger_count,
        })
        .to_string()
            + "\n";
        let mut f = self.audit.lock().map_err(|e| format!("audit lock: {e}"))?;
        f.write_all(line.as_bytes())
            .and_then(|_| f.flush())
            .map_err(|e| format!("audit append: {e}"))
    }
}

impl EmergencyActuator for StubEmergencyActuator {
    fn trigger(
        &self,
        source: EmergencySource,
        seq: Option<u64>,
    ) -> Result<EmergencyTrigger, String> {
        let trigger_count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        let latched = !self.latch.swap(true, Ordering::SeqCst);
        tracing::info!(
            source = source.as_str(),
            seq,
            latched,
            trigger_count,
            "EMERGENCY STOP triggered"
        );
        // C15: an audit write failure must be logged loudly (the e-stop already happened, the error is not swallowed)
        let audit = self.append_audit(source, seq, latched, trigger_count);
        if let Err(e) = &audit {
            tracing::error!(source = source.as_str(), seq, error = %e, "audit write failed");
        }
        Ok(EmergencyTrigger { latched, trigger_count, audit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latch_first_trigger_arms_subsequent_reported() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = StubEmergencyActuator::new(f.path()).unwrap();
        let t1 = a.trigger(EmergencySource::Dc, Some(1)).unwrap();
        assert!(t1.latched, "the first trigger must arm");
        assert_eq!(t1.trigger_count, 1);
        assert!(t1.audit.is_ok());
        let t2 = a.trigger(EmergencySource::Dc, Some(2)).unwrap();
        assert!(!t2.latched, "a repeated trigger must not re-arm");
        assert_eq!(t2.trigger_count, 2, "a repeated trigger still counts (audit-observable)");
        let t3 = a.trigger(EmergencySource::Local, None).unwrap();
        assert!(!t3.latched);
        assert_eq!(t3.trigger_count, 3);
    }

    #[test]
    fn audit_line_records_source_seq_latched() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = StubEmergencyActuator::new(f.path()).unwrap();
        a.trigger(EmergencySource::Dc, Some(7)).unwrap();
        a.trigger(EmergencySource::Local, None).unwrap();

        let lines: Vec<serde_json::Value> = std::fs::read_to_string(f.path())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        // dc line: all fields + an RFC3339 timestamp
        let l0 = &lines[0];
        assert_eq!(l0["source"], "dc");
        assert_eq!(l0["seq"], 7);
        assert_eq!(l0["cmd"], EMERGENCY_STOP);
        assert_eq!(l0["latched"], true);
        assert_eq!(l0["trigger_count"], 1);
        let ts = l0["ts"].as_str().expect("ts must be a string");
        chrono::DateTime::parse_from_rfc3339(ts).expect("ts must parse (RFC3339)");
        // local line: seq = null (no envelope)
        let l1 = &lines[1];
        assert_eq!(l1["source"], "local");
        assert!(l1["seq"].is_null());
        assert_eq!(l1["latched"], false);
        assert_eq!(l1["trigger_count"], 2);
    }

    #[test]
    fn new_rejects_unwritable_audit_path() {
        let dir = tempfile::TempDir::new().unwrap();
        // a directory path cannot be appended to -> fail at startup (no strong audit, no run)
        assert!(StubEmergencyActuator::new(dir.path()).is_err());
        // a non-existent parent directory -> failure
        assert!(StubEmergencyActuator::new(&dir.path().join("no/such/dir/a.jsonl")).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn audit_write_failure_is_surfaced_not_silent() {
        // /dev/full: a write always gets ENOSPC (a classic error-injection surface)
        let a = StubEmergencyActuator::new(Path::new("/dev/full")).unwrap();
        let t = a.trigger(EmergencySource::Dc, Some(1)).unwrap();
        assert!(t.latched);
        assert!(t.audit.is_err(), "an audit failure must surface in the result (C15)");
    }
}
