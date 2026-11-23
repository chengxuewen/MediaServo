//! T9 evidence retention (design K5=A dual-buffer / K14=B TTL / 7.3 privacy).
//!
//! Event-triggered talk-spurt clips: explicit consent (client modal), stored
//! server-side with a 7-day TTL, every action audited (retain/download/
//! withdraw/purge). STRICTLY separate from the T7 replay buffer (that one
//! never leaves the browser and is wiped on exit).
//!
//! Global store following the audit-ring precedent (no AdminState field churn):
//! directory = MEDIASERVO_EVIDENCE_DIR or <tmp>/mediaservo-evidence. Items are
//!   <id>.webm  (raw audio bytes)
//!   <id>.json  (metadata sidecar)
//! `list()` scans the sidecars -- cheap at 7-day retention scale, and the
//! truth survives restarts (unlike the 256-entry audit ring, §7-16 debt).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// K14=B: real-time TTL -- 7 days.
pub const RETENTION_SECS: u64 = 7 * 24 * 3600;
/// Per-clip upload cap (talk spurts are short; rejects abuse early).
pub const MAX_CLIP_BYTES: usize = 2 * 1024 * 1024;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceItem {
    pub id: String,
    pub room_id: String,
    /// Account that declared the clip (JWT sub).
    pub actor: String,
    #[serde(default)]
    pub label: String,
    /// Unix seconds of the declaration.
    pub ts: u64,
    /// Unix seconds; sweep removes the item at/after this instant.
    pub retain_until: u64,
    pub size: u64,
}

pub struct EvidenceStore {
    dir: PathBuf,
    write_lock: Mutex<()>,
}

impl EvidenceStore {
    /// Open (and create) a store at `dir`.
    pub fn at(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref().to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir, write_lock: Mutex::new(()) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Store one clip (raw audio bytes) with 7-day retention.
    pub fn retain(
        &self,
        room_id: &str,
        actor: &str,
        label: &str,
        bytes: &[u8],
    ) -> Result<EvidenceItem, String> {
        if bytes.is_empty() {
            return Err("empty clip".into());
        }
        if bytes.len() > MAX_CLIP_BYTES {
            return Err(format!("clip exceeds {MAX_CLIP_BYTES} bytes"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now_secs();
        let item = EvidenceItem {
            id: id.clone(),
            room_id: room_id.to_string(),
            actor: actor.to_string(),
            label: label.to_string(),
            ts,
            retain_until: ts + RETENTION_SECS,
            size: bytes.len() as u64,
        };
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::write(self.clip_path(&id), bytes).map_err(|e| format!("write clip: {e}"))?;
        let meta = serde_json::to_vec_pretty(&item).map_err(|e| format!("meta: {e}"))?;
        std::fs::write(self.meta_path(&id), meta).map_err(|e| format!("write meta: {e}"))?;
        Ok(item)
    }

    /// All retained items (expired ones included until swept).
    pub fn list(&self) -> Vec<EvidenceItem> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read(&path) {
                if let Ok(item) = serde_json::from_slice::<EvidenceItem>(&raw) {
                    out.push(item);
                }
            }
        }
        out.sort_by_key(|i| i.ts);
        out
    }

    /// Load one item's meta + bytes (None when missing).
    pub fn load(&self, id: &str) -> Option<(EvidenceItem, Vec<u8>)> {
        let raw = std::fs::read(self.meta_path(id)).ok()?;
        let item: EvidenceItem = serde_json::from_slice(&raw).ok()?;
        let bytes = std::fs::read(self.clip_path(id)).ok()?;
        Some((item, bytes))
    }

    /// Consent withdrawal / manual delete. Returns true when something was removed.
    pub fn delete(&self, id: &str) -> bool {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let a = std::fs::remove_file(self.clip_path(id)).is_ok();
        let b = std::fs::remove_file(self.meta_path(id)).is_ok();
        a || b
    }

    /// K14=B sweeper: purge items past retention; returns what was purged.
    pub fn sweep(&self) -> Vec<EvidenceItem> {
        let now = now_secs();
        let expired: Vec<EvidenceItem> =
            self.list().into_iter().filter(|i| i.retain_until <= now).collect();
        for item in &expired {
            let _ = self.delete(&item.id);
        }
        expired
    }

    fn clip_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.webm"))
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
}

/// Process-global store (audit-ring precedent; AdminState untouched).
pub fn store() -> &'static EvidenceStore {
    static STORE: OnceLock<EvidenceStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let dir = std::env::var("MEDIASERVO_EVIDENCE_DIR").unwrap_or_else(|_| {
            std::env::temp_dir().join("mediaservo-evidence").to_string_lossy().into_owned()
        });
        EvidenceStore::at(dir)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> EvidenceStore {
        let dir = std::env::temp_dir().join(format!("ms-ev-{}", uuid::Uuid::new_v4()));
        EvidenceStore::at(dir)
    }

    #[test]
    fn retain_list_load_delete_roundtrip() {
        let s = tmp_store();
        let item = s.retain("talk-ops", "carol", "spurt", b"fake-audio-bytes").unwrap();
        assert_eq!(item.retain_until, item.ts + RETENTION_SECS);
        assert_eq!(s.list().len(), 1);
        let (meta, bytes) = s.load(&item.id).unwrap();
        assert_eq!(meta.room_id, "talk-ops");
        assert_eq!(bytes, b"fake-audio-bytes");
        assert!(s.delete(&item.id));
        assert!(s.list().is_empty());
        assert!(!s.delete(&item.id));
    }

    #[test]
    fn rejects_empty_and_oversize() {
        let s = tmp_store();
        assert!(s.retain("r", "a", "", b"").is_err());
        let big = vec![0u8; MAX_CLIP_BYTES + 1];
        assert!(s.retain("r", "a", "", &big).is_err());
    }

    #[test]
    fn sweep_purges_only_expired() {
        let s = tmp_store();
        let fresh = s.retain("r", "a", "fresh", b"x").unwrap();
        let mut old = s.retain("r", "a", "old", b"y").unwrap();
        old.retain_until = now_secs() - 1; // backdate on disk
        std::fs::write(s.meta_path(&old.id), serde_json::to_vec(&old).unwrap()).unwrap();
        let purged = s.sweep();
        assert_eq!(purged.len(), 1);
        assert_eq!(purged[0].id, old.id);
        assert!(s.load(&old.id).is_none());
        assert!(s.load(&fresh.id).is_some());
    }
}
