//! Session state persistence — write to disk every 10s, load on startup if fresh.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time;

/// Session state serialized to `/tmp/mediaservo-host-session.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier (host ID from config).
    pub id: String,
    /// Room the host is currently publishing to.
    pub room_id: Option<String>,
    /// Session start timestamp.
    pub started_at: DateTime<Utc>,
}

impl Session {
    /// Load session from `state_path` if the file is < 60s stale.
    /// Returns a fresh session otherwise.
    pub fn load(state_path: Option<&str>) -> Self {
        let path = PathBuf::from(state_path.unwrap_or("/tmp/mediaservo-host-session.json"));

        let _fallback = Self { id: "unknown".to_string(), room_id: None, started_at: Utc::now() };
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                tracing::info!("No existing session state, starting fresh");
                return new_session();
            }
        };

        let state: Self = match serde_json::from_str(&contents) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to parse session state: {e}, starting fresh");
                return new_session();
            }
        };

        // Check staleness
        if let Ok(meta) = std::fs::metadata(&path)
            && let Ok(modified) = meta.modified()
        {
            let age = std::time::SystemTime::now()
                .duration_since(modified)
                .unwrap_or(Duration::from_secs(999));
            if age < Duration::from_secs(60) {
                tracing::info!(
                    "Loaded session: id={}, room={:?}, age={}s",
                    state.id,
                    state.room_id,
                    age.as_secs()
                );
                return state;
            }
            tracing::info!("Session state stale ({}s > 60s), starting fresh", age.as_secs());
        }
        new_session()
    }

    /// Start background persistence — writes session JSON every 10 seconds.
    pub fn start_persist(&self) -> tokio::task::JoinHandle<()> {
        let path = PathBuf::from("/tmp/mediaservo-host-session.json");
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let content = serde_json::to_string_pretty(&state).unwrap_or_else(|e| {
                    tracing::error!("Failed to serialize session: {e}");
                    "{}".to_string()
                });
                // ponytail: atomic write via temp file + rename
                let tmp_path = path.with_extension("tmp");
                if let Err(e) = std::fs::write(&tmp_path, &content) {
                    tracing::error!("Failed to write session temp file: {e}");
                    continue;
                }
                if let Err(e) = std::fs::rename(&tmp_path, &path) {
                    tracing::error!("Failed to rename session file: {e}");
                }
            }
        })
    }
}

fn new_session() -> Session {
    Session { id: "unknown".to_string(), room_id: None, started_at: Utc::now() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_serialization() {
        let state = Session {
            id: "host-001".to_string(),
            room_id: Some("room-1".to_string()),
            started_at: Utc::now(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "host-001");
        assert_eq!(parsed.room_id, Some("room-1".to_string()));
    }

    #[test]
    fn session_load_missing_file_returns_fresh() {
        let session = Session::load(Some("/tmp/nonexistent-mediaservo-session.json"));
        assert_eq!(session.id, "unknown");
        assert!(session.room_id.is_none());
    }
}

#[test]
fn session_load_writes_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let path_str = path.to_str().unwrap();

    let state = Session {
        id: "host-001".to_string(),
        room_id: Some("room-1".to_string()),
        started_at: Utc::now(),
    };
    std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();

    let loaded = Session::load(Some(path_str));
    assert_eq!(loaded.id, "host-001");
    assert_eq!(loaded.room_id, Some("room-1".to_string()));
}

#[tokio::test]
async fn session_start_persist_spawns() {
    let session =
        Session { id: "host-persist-test".to_string(), room_id: None, started_at: Utc::now() };
    let handle = session.start_persist();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    handle.abort();

    let path = std::path::PathBuf::from("/tmp/mediaservo-host-session.json");
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("host-persist-test"));
}
