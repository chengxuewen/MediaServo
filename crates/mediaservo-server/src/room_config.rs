//! Managed-room config store (P4-3, O20/RM12-C): `rooms.yaml` is the explicit
//! kind authority ("config kind wins", the room.rs P4 seam). Follows the
//! devices.yaml store shape (temp + fsync + rename, memory unchanged on
//! failure) with ONE deliberate difference: a corrupt EXISTING file refuses
//! startup (main.rs panic, same as accounts.yaml) — silently empty would
//! resurrect the id-inference behavior RM12 exists to kill. A MISSING file is
//! a legal first-run state (empty store). The in-memory `Vec` is the single
//! writer; every mutation saves to disk first-class and re-syncs the
//! process-global managed table in `mediaservo_common::room` (RM12: the
//! single instance is the only process touching this file).

use mediaservo_common::error::CoreError;
use mediaservo_common::room::{self, RoomKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// One managed room entry (rooms.yaml line). `kind` is the wire label
/// string (`talk|meet|desk|view|ctrl|stream`) — parsed via `RoomKind::parse`
/// at every boundary (load/create/update); RoomKind itself has no serde
/// derive (the wire enum is FloorMode), so the label string is the stored form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedRoom {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<bool>,
}

/// Label validation shared by load and CRUD (a bad label in the FILE is
/// refuse-on-corrupt; in a REQUEST it maps to 400).
fn parse_kind_label(label: &str) -> Result<RoomKind, CoreError> {
    RoomKind::parse(label).map_err(|e| CoreError::ConfigParse(format!("managed room kind: {e}")))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ManagedRoomFile {
    rooms: Vec<ManagedRoom>,
}

pub struct RoomConfigStore {
    rooms: RwLock<Vec<ManagedRoom>>,
    path: PathBuf,
}

impl RoomConfigStore {
    /// Load from disk. Corrupt (exists but unparsable) -> Err -> caller
    /// refuses startup (refuse-on-corrupt, the accounts.yaml discipline).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let path = path.as_ref().to_path_buf();
        let content = std::fs::read_to_string(&path)
            .map_err(|e| CoreError::ConfigParse(format!("rooms file {}: {e}", path.display())))?;
        let file: ManagedRoomFile = serde_yaml::from_str(&content)
            .map_err(|e| CoreError::ConfigParse(format!("rooms file {}: {e}", path.display())))?;
        for r in &file.rooms {
            parse_kind_label(&r.kind)?; // bad label = corrupt (refuse-on-corrupt)
        }
        let store = Self { rooms: RwLock::new(file.rooms), path };
        store.sync_global();
        Ok(store)
    }

    /// Empty store bound to a path (missing-file first run; tests).
    pub fn empty_with_path(path: impl Into<PathBuf>) -> Self {
        Self { rooms: RwLock::new(Vec::new()), path: path.into() }
    }

    pub fn list(&self) -> Vec<ManagedRoom> {
        self.rooms.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn get(&self, id: &str) -> Option<ManagedRoom> {
        self.rooms.read().unwrap_or_else(|e| e.into_inner()).iter().find(|r| r.id == id).cloned()
    }

    /// Create. Duplicate id or bad kind label -> Err (caller maps to 409/400).
    pub fn create(&self, mut entry: ManagedRoom) -> Result<(), CoreError> {
        let kind = parse_kind_label(&entry.kind)?;
        entry.kind = kind.as_str().to_string(); // canonical lowercase form
        {
            let mut rooms = self.rooms.write().unwrap_or_else(|e| e.into_inner());
            if rooms.iter().any(|r| r.id == entry.id) {
                return Err(CoreError::ConfigParse(format!(
                    "managed room {} already exists",
                    entry.id
                )));
            }
            rooms.push(entry.clone());
        }
        if let Err(e) = self.save() {
            // memory rollback: disk stays the single source of truth
            self.rooms.write().unwrap_or_else(|e| e.into_inner()).retain(|r| r.id != entry.id);
            return Err(e);
        }
        self.sync_global();
        Ok(())
    }

    /// Update kind/video. Unknown id or bad label -> Err (404/400).
    pub fn update(
        &self,
        id: &str,
        kind: Option<String>,
        video: Option<Option<bool>>,
    ) -> Result<ManagedRoom, CoreError> {
        let canonical_kind =
            kind.map(|k| parse_kind_label(&k).map(|k| k.as_str().to_string())).transpose()?;
        let updated = {
            let mut rooms = self.rooms.write().unwrap_or_else(|e| e.into_inner());
            let Some(entry) = rooms.iter_mut().find(|r| r.id == id) else {
                return Err(CoreError::ConfigParse(format!("managed room {id} not found")));
            };
            if let Some(k) = canonical_kind {
                entry.kind = k;
            }
            if let Some(v) = video {
                entry.video = v;
            }
            entry.clone()
        };
        if let Err(e) = self.save() {
            self.reload_from_disk_locked();
            return Err(e);
        }
        self.sync_global();
        Ok(updated)
    }

    /// Remove. Unknown id -> Err (caller maps to 404).
    pub fn remove(&self, id: &str) -> Result<ManagedRoom, CoreError> {
        let removed = {
            let mut rooms = self.rooms.write().unwrap_or_else(|e| e.into_inner());
            let pos = rooms
                .iter()
                .position(|r| r.id == id)
                .ok_or_else(|| CoreError::ConfigParse(format!("managed room {id} not found")))?;
            rooms.remove(pos)
        };
        if let Err(e) = self.save() {
            self.rooms.write().unwrap_or_else(|e| e.into_inner()).push(removed.clone());
            return Err(e);
        }
        self.sync_global();
        Ok(removed)
    }

    /// Atomic write-back (temp + fsync + rename), the devices.yaml shape.
    fn save(&self) -> Result<(), CoreError> {
        let file =
            ManagedRoomFile { rooms: self.rooms.read().unwrap_or_else(|e| e.into_inner()).clone() };
        let tmp = self.path.with_extension("yaml.tmp");
        let write = || -> std::io::Result<()> {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            {
                let mut f = std::fs::File::create(&tmp)?;
                serde_yaml::to_writer(std::io::BufWriter::new(&mut f), &file)
                    .map_err(std::io::Error::other)?;
                f.sync_all()?;
            }
            std::fs::rename(&tmp, &self.path)
        };
        match write() {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp); // success path leaves no tmp
                Err(CoreError::ConfigParse(format!(
                    "rooms file {} write-back: {e}",
                    self.path.display()
                )))
            }
        }
    }

    /// Disk-truth reload after a failed save (update path keeps the on-disk
    /// state instead of hand-rolling a reverse patch).
    fn reload_from_disk_locked(&self) {
        if let Ok(content) = std::fs::read_to_string(&self.path)
            && let Ok(file) = serde_yaml::from_str::<ManagedRoomFile>(&content)
        {
            *self.rooms.write().unwrap_or_else(|e| e.into_inner()) = file.rooms;
        }
    }

    /// Push the current table into the process-global resolve authority.
    fn sync_global(&self) {
        let rooms = self.rooms.read().unwrap_or_else(|e| e.into_inner()).clone();
        room::set_managed_rooms(rooms.into_iter().filter_map(|r| {
            // labels were validated at the boundary; skip defensively anyway
            RoomKind::parse(&r.kind).ok().map(|k| (r.id, (k, r.video)))
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ms-roomcfg-test-{tag}-{}.yaml", std::process::id()))
    }

    #[test]
    fn refuse_on_corrupt_and_persist_across_restart() {
        let path = tmp_path("corrupt");
        std::fs::write(&path, "rooms: [ { id: broken, kind: ").unwrap();
        // refuse-on-corrupt: a corrupt EXISTING file is an Err, not an empty store
        assert!(RoomConfigStore::load(&path).is_err());

        // legal first-run shape: missing file handled by the caller; via a
        // store built by create, persistence works across a fresh load.
        std::fs::remove_file(&path).ok();
        let store = RoomConfigStore::empty_with_path(&path);
        store
            .create(ManagedRoom { id: "talk-ops".into(), kind: "talk".into(), video: None })
            .unwrap();
        store
            .create(ManagedRoom {
                id: "meet-voice".into(),
                kind: "meet".into(),
                video: Some(false),
            })
            .unwrap();

        // "restart": a fresh load sees the same truth (acceptance: persists)
        let reloaded = RoomConfigStore::load(&path).unwrap();
        assert_eq!(reloaded.list().len(), 2);
        assert_eq!(reloaded.get("meet-voice").unwrap().video, Some(false));
        assert!(!path.with_extension("yaml.tmp").exists(), "no tmp residue");
        std::fs::remove_file(&path).ok();
        room::clear_managed_rooms();
    }

    #[test]
    fn create_duplicate_fails_without_state_change() {
        let path = tmp_path("dup");
        let store = RoomConfigStore::empty_with_path(&path);
        store
            .create(ManagedRoom { id: "fleet1".into(), kind: "view".into(), video: None })
            .unwrap();
        assert!(
            store
                .create(ManagedRoom { id: "fleet1".into(), kind: "talk".into(), video: None })
                .is_err()
        );
        assert_eq!(store.get("fleet1").unwrap().kind, "view", "memory unchanged");
        // bad kind label maps to 400-shaped Err at the store boundary too
        assert!(
            store
                .create(ManagedRoom { id: "x".into(), kind: "banana".into(), video: None })
                .is_err()
        );
        std::fs::remove_file(&path).ok();
        room::clear_managed_rooms();
    }
}
