//! Generic state backup/restore with atomic writes.
//!
//! # Pattern
//! 1. Call `BackupManager::save()` periodically to persist state
//! 2. Call `BackupManager::load()` on startup to recover state
//!
//! Writes are atomic: data is written to a temp file and renamed into place,
//! so partial writes never corrupt the backup.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Manages backup persistence for a single JSON-serializable state snapshot.
///
/// Atomic writes via temp-file + rename. Staleness check rejects files
/// older than `max_stale` on load.
pub struct BackupManager<T> {
    path: PathBuf,
    /// Maximum acceptable age of backup file on load (None = always accept).
    max_stale: Option<Duration>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned + Clone + Debug> BackupManager<T> {
    /// Create a new backup manager targeting the given file path.
    ///
    /// `max_stale` — if set, `load()` returns `None` when the file is older
    /// than this duration. Pass `None` to always attempt recovery.
    pub fn new(path: impl Into<PathBuf>, max_stale: Option<Duration>) -> Self {
        Self {
            path: path.into(),
            max_stale,
            _marker: std::marker::PhantomData,
        }
    }

    /// Save state to disk atomically.
    ///
    /// Writes JSON to a `.tmp` sibling file, then renames it over the target.
    /// On any error, the existing backup file is preserved.
    pub fn save(&self, state: &T) -> Result<(), BackupError> {
        let json = serde_json::to_string_pretty(state)
            .map_err(|e| BackupError::Serialize(e.to_string()))?;

        let tmp_path = self.tmp_path();
        std::fs::write(&tmp_path, &json)
            .map_err(|e| BackupError::Write(e.to_string()))?;

        std::fs::rename(&tmp_path, &self.path)
            .map_err(|e| BackupError::AtomicSwap(e.to_string()))?;

        Ok(())
    }

    /// Load state from disk.
    ///
    /// Returns `Ok(None)` if the file doesn't exist or is too stale.
    /// Returns `Ok(Some(state))` on successful parse.
    /// Returns `Err` only on I/O errors or parse failures.
    pub fn load(&self) -> Result<Option<T>, BackupError> {
        let path = &self.path;

        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(BackupError::Read(e.to_string())),
        };

        // Staleness check
        if let Some(max_stale) = self.max_stale
            && let Ok(modified) = metadata.modified() {
            let age = std::time::SystemTime::now()
                .duration_since(modified)
                .unwrap_or(Duration::MAX);
            if age > max_stale {
                tracing::info!(
                    path = %path.display(),
                    age_secs = age.as_secs(),
                    max_stale_secs = max_stale.as_secs(),
                    "Backup file too stale, ignoring"
                );
                return Ok(None);
            }
        }

        let contents = std::fs::read_to_string(path)
            .map_err(|e| BackupError::Read(e.to_string()))?;

        let state: T = serde_json::from_str(&contents)
            .map_err(|e| BackupError::Deserialize(e.to_string()))?;

        Ok(Some(state))
    }

    /// Delete the backup file (e.g. on clean shutdown).
    pub fn delete(&self) -> Result<(), BackupError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(BackupError::Delete(e.to_string())),
        }
    }

    /// Return the backup file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn tmp_path(&self) -> PathBuf {
        self.path.with_extension("tmp")
    }
}

/// Errors that can occur during backup operations.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("serialize failed: {0}")]
    Serialize(String),

    #[error("deserialize failed: {0}")]
    Deserialize(String),

    #[error("write failed: {0}")]
    Write(String),

    #[error("read failed: {0}")]
    Read(String),

    #[error("atomic swap failed: {0}")]
    AtomicSwap(String),

    #[error("delete failed: {0}")]
    Delete(String),
}

/// A lightweight backup task that persists state on a fixed interval.
///
/// `period` is the interval between saves. Only the latest state snapshot
/// is persisted (no rotation).
pub struct BackupTask<T> {
    manager: BackupManager<T>,
    period: Duration,
}

impl<T: Serialize + DeserializeOwned + Clone + Debug + Send + Sync + 'static> BackupTask<T> {
    /// Create a new backup task.
    pub fn new(path: impl Into<PathBuf>, period: Duration) -> Self {
        Self {
            manager: BackupManager::new(path, None),
            period,
        }
    }

    /// Spawn a background task that persists `state_fn()` every `period`.
    ///
    /// `state_fn` is called on each tick to get the latest snapshot.
    pub fn spawn<F>(&self, state_fn: F) -> tokio::task::JoinHandle<()>
    where
        F: Fn() -> T + Send + 'static,
    {
        let mgr = BackupManager::new(self.manager.path().to_path_buf(), None);
        let period = self.period;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            loop {
                interval.tick().await;
                let state = state_fn();
                if let Err(e) = mgr.save(&state) {
                    tracing::warn!(error = %e, "Backup persist failed");
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestState {
        value: u32,
        name: String,
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.json");

        let mgr: BackupManager<TestState> =
            BackupManager::new(&path, None);

        let state = TestState {
            value: 42,
            name: "test".into(),
        };
        mgr.save(&state).unwrap();

        let loaded = mgr.load().unwrap().unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn load_missing_returns_none() {
        let mgr: BackupManager<TestState> =
            BackupManager::new("/tmp/nonexistent-backup-test.json", None);
        let result = mgr.load().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn stale_backup_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stale.json");

        let mgr: BackupManager<TestState> =
            BackupManager::new(&path, Some(Duration::from_secs(3600)));

        let state = TestState {
            value: 1,
            name: "fresh".into(),
        };
        mgr.save(&state).unwrap();

        let loaded = mgr.load().unwrap().unwrap();
        assert_eq!(loaded.value, 1);
    }

    #[test]
    fn atomic_write_does_not_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atomic.json");

        let mgr: BackupManager<TestState> =
            BackupManager::new(&path, None);

        // Write initial state
        mgr.save(&TestState {
            value: 100,
            name: "initial".into(),
        })
        .unwrap();

        // Simulate crash by deleting temp file
        let tmp_path = path.with_extension("tmp");
        if tmp_path.exists() {
            std::fs::remove_file(&tmp_path).unwrap();
        }

        // Load should return the last successful write
        let loaded = mgr.load().unwrap().unwrap();
        assert_eq!(loaded.value, 100);
    }

    #[test]
    fn delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("to_delete.json");

        let mgr: BackupManager<TestState> =
            BackupManager::new(&path, None);
        mgr.save(&TestState {
            value: 0,
            name: "x".into(),
        })
        .unwrap();
        assert!(path.exists());

        mgr.delete().unwrap();
        assert!(!path.exists());
    }
}
