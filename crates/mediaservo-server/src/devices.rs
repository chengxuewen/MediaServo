//! G2 device registry -- server-side device credential verification (D-H11 connection-level identity).
//!
//! The registry is file-based config (YAML, same shape as server.yaml), format:
//! ```yaml
//! devices:
//!   ms-0a1b2c3d4e5f:
//!     secret_hash: "sha256:<hex>"   # sha256(device_id + ":" + device_secret) -- legacy secret form
//!   ms-c3d4e5f6a7b8:
//!     public_key: "ed25519:<b64>"   # device-enroll public-key form (base64 32B Ed25519 vk)
//!     name: "jetson-7"              # optional display name
//! ```
//! device-enroll (design S5.1): the two entry forms coexist (D-E3 one cycle); whichever is read drives which verification, save preserves the form.
//! Storage decision (G2): the client carries the secret in the clear on the wire over TLS; the registry stores only a one-way hash;
//! `sha256(device_id + ":" + device_secret)` -- device_id acts as the per-device salt (no extra storage needed).
//! Upgrade path (phase H): argon2id replaces sha256, format prefix `argon2:<encoded>`.
//! Provisioning flow (G2 doc): `host init` generates identity.json -> ops copies the device_id/secret
//! into the server's devices.yaml (`ms-field hash` and similar tools come in phase H; for now compute
//! by hand with `sha256sum` or use this module's test vectors).
//!
//! Hot reload (unified-device-admin): the registry internals become `RwLock<Inner>` -- the external signatures
//! (`Arc<DeviceRegistry>` / `&DeviceRegistry`) are unchanged, so signaling auth call sites need no changes;
//! management operations (register/revoke/reset/list/save) take effect at runtime with no server restart.
//! Write-back strategy: disk is the single source of truth -- `save` serializes first (short critical section), then writes atomically
//! (temp + fsync + rename); on failure returns Err with memory unchanged.

use base64::Engine as _;
use mediaservo_common::error::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, RwLock, RwLockReadGuard};
use subtle::ConstantTimeEq;

/// Device auth failure reason (error code unified as 4010, see the auth-point comment in signaling.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAuthError {
    /// device_id/device_secret with exactly one present (shape check, G4 review Minor 1).
    Incomplete,
    /// device_id is not in the registry.
    Unknown,
    /// secret hash mismatch.
    BadSecret,
}

impl DeviceAuthError {
    /// Client-facing readable message (C15: error responses must be informative).
    /// Anti-enumeration (review #1): unknown device and wrong secret must return a **byte-identical** message --
    /// distinguishing them leaks registry membership. The internal distinction (Unknown/BadSecret) is kept in the audit log only.
    /// A single error code 4010 (the signaling.rs auth-point constant) + this single message.
    pub fn message(&self) -> &'static str {
        match self {
            DeviceAuthError::Incomplete => {
                "device authentication failed: both device_id and device_secret are required"
            }
            DeviceAuthError::Unknown | DeviceAuthError::BadSecret => {
                "device authentication failed: invalid device credentials"
            }
        }
    }
}

/// Registry management operation errors (admin API; 400/404/409 mapping in admin.rs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceRegError {
    /// device_id already exists on register.
    Duplicate,
    /// device_id does not exist on revoke/reset.
    Unknown,
    /// Admin-provided secret is non-compliant (non-empty, 8-128 chars, no whitespace).
    InvalidSecret(String),
    /// enroll-admitted public_key is non-compliant (not a canonical base64(32B) Ed25519 vk).
    InvalidPublicKey(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct RegistryFile {
    #[serde(default)]
    devices: HashMap<String, DeviceEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DeviceEntry {
    /// legacy secret form (D-E3 coexistence for one cycle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_hash: Option<String>,
    /// Public-key form: "ed25519:<base64 32B vk>".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_key: Option<String>,
    /// Optional display name for the public-key form (auto-admission -> None).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// Registry entry dual form (device-enroll design S5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// secret form: "sha256:<hex>".
    Secret(String),
    /// Public-key form: vk = base64(32B Ed25519 verifying key).
    PublicKey { vk: String, name: Option<String> },
}

/// Registry internal state (wrapped in RwLock; methods all dispatch internally).
#[derive(Debug)]
struct RegistryInner {
    devices: HashMap<String, Entry>, // device_id → Secret("sha256:<hex>") | PublicKey{vk,name}
    /// Fixed comparison target for unknown devices (randomly generated at startup, same length as a real hash, never matches).
    /// review #1: unknown devices must also walk the full sha256 + ct_eq path; response time is indistinguishable.
    dummy_hash: String,
}

impl RegistryInner {
    fn new(devices: HashMap<String, Entry>) -> Self {
        Self { devices, dummy_hash: new_dummy_hash() }
    }

    fn verify(&self, device_id: &str, secret: &str) -> Result<(), DeviceAuthError> {
        let known = self.devices.contains_key(device_id);
        // review #1 anti-timing: unknown devices also walk the full sha256 + ct_eq with dummy_hash (no early return).
        // Known/unknown response times are indistinguishable; matching is decided by same-length ct_eq.
        // Public-key entries use the dummy for secret comparison (same path as unknown devices -- does not leak entry form, review #1 discipline).
        let stored: &String = match self.devices.get(device_id) {
            Some(Entry::Secret(h)) => h,
            _ => &self.dummy_hash,
        };
        let want = hash_secret(device_id, secret);
        let matched: bool = stored.as_bytes().ct_eq(want.as_bytes()).into();
        match (matched, known) {
            (true, _) => Ok(()),
            // Internal distinction kept (for audit); the external wire response is fully identical for both (see message()).
            (false, true) => Err(DeviceAuthError::BadSecret),
            (false, false) => Err(DeviceAuthError::Unknown),
        }
    }
}

/// Device registry (loaded at startup; read hot-reload at runtime, management ops via RwLock).
/// Note: no Default impl -- dummy_hash must be randomly generated at startup (Default would give an empty string,
/// a different length from a real hash -> the unknown-device comparison path would be timing-distinguishable from known devices, reopening the side channel).
/// Note: no longer derives Clone (std RwLock is not Clone) -- sharing always goes through Arc; usage sites confirmed clone-free.
#[derive(Debug)]
pub struct DeviceRegistry {
    inner: RwLock<RegistryInner>,
}

impl DeviceRegistry {
    pub fn empty() -> Self {
        Self { inner: RwLock::new(RegistryInner::new(HashMap::new())) }
    }

    /// Load from a YAML file; a missing file is treated as an empty registry (PSK path unaffected).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            CoreError::ConfigParse(format!("devices file {}: {e}", path.as_ref().display()))
        })?;
        Self::from_yaml(&content).map_err(|e| {
            CoreError::ConfigParse(format!("devices file {}: {e}", path.as_ref().display()))
        })
    }

    /// Parse from YAML text (shared by tests and load).
    pub fn from_yaml(content: &str) -> Result<Self, String> {
        let file: RegistryFile =
            serde_yaml::from_str(content).map_err(|e| format!("YAML parse error: {e}"))?;
        let mut devices = HashMap::new();
        for (id, entry) in file.devices {
            let parsed = match (entry.secret_hash, entry.public_key) {
                (Some(hash), None) => {
                    if !hash.starts_with("sha256:") {
                        return Err(format!(
                            "device {id}: unsupported secret_hash scheme (want sha256:)"
                        ));
                    }
                    if hash.len() != "sha256:".len() + 64 {
                        return Err(format!("device {id}: malformed sha256 hex length"));
                    }
                    Entry::Secret(hash)
                }
                (None, Some(pk)) => {
                    let vk = pk.strip_prefix("ed25519:").ok_or_else(|| {
                        format!("device {id}: unsupported public_key scheme (want ed25519:)")
                    })?;
                    decode_vk(vk)
                        .map_err(|e| format!("device {id}: malformed public_key ({e})"))?;
                    Entry::PublicKey { vk: vk.to_string(), name: entry.name }
                }
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "device {id}: entry carries both secret_hash and public_key (exactly one form allowed)"
                    ));
                }
                (None, None) => {
                    return Err(format!("device {id}: entry must carry secret_hash or public_key"));
                }
            };
            devices.insert(id, parsed);
        }
        Ok(Self { inner: RwLock::new(RegistryInner::new(devices)) })
    }

    /// Standard lock-poison recovery: unpoisoned read lock; on poison take the consistent value left by the writer.
    fn lock_read(&self) -> RwLockReadGuard<'_, RegistryInner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_write(&self) -> std::sync::RwLockWriteGuard<'_, RegistryInner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn len(&self) -> usize {
        self.lock_read().devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock_read().devices.is_empty()
    }

    /// All device_ids in the registry (for the admin list).
    pub fn device_ids(&self) -> Vec<String> {
        self.lock_read().devices.keys().cloned().collect()
    }

    fn verify(&self, device_id: &str, secret: &str) -> Result<(), DeviceAuthError> {
        self.lock_read().verify(device_id, secret)
    }

    /// Serialize to YAML file content (inverse of from_yaml format, stable round-trip).
    fn to_yaml(&self) -> Result<String, CoreError> {
        let inner = self.lock_read();
        let file = RegistryFile {
            devices: inner
                .devices
                .iter()
                .map(|(id, entry)| {
                    let fe = match entry {
                        Entry::Secret(hash) => DeviceEntry {
                            secret_hash: Some(hash.clone()),
                            public_key: None,
                            name: None,
                        },
                        Entry::PublicKey { vk, name } => DeviceEntry {
                            secret_hash: None,
                            public_key: Some(format!("ed25519:{vk}")),
                            name: name.clone(),
                        },
                    };
                    (id.clone(), fe)
                })
                .collect(),
        };
        serde_yaml::to_string(&file)
            .map_err(|e| CoreError::ConfigParse(format!("devices serialize: {e}")))
    }

    /// Atomic write-back of devices.yaml (temp + fsync + rename).
    /// **Memory unchanged**: Ok only when serialization and disk write both succeed; on failure clean up temp and return Err.
    /// The caller (admin API layer) owns the right to call this -- the write path is low-frequency with a short critical section.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CoreError> {
        let path = path.as_ref();
        let yaml = self.to_yaml()?;
        let tmp = path.with_extension("yaml.tmp");
        let res = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(yaml.as_bytes())?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, path)?;
            Ok(())
        })();
        match res {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp); // clean up residue (the success path leaves no tmp)
                Err(CoreError::ConfigParse(format!(
                    "devices file {}: write failed: {e}",
                    path.display()
                )))
            }
        }
    }
    /// sha256(device_id + ":" + device_secret), hex encoded, `sha256:` prefix.
    /// Register a new device: generates a random secret by default.
    /// Returns `(secret_hash, secret)` -- the secret is the **one and only plaintext**, the caller hands it over;
    /// it can only be changed afterward via reset_secret. Persisting is done by the admin API layer calling `save`.
    pub fn register(&self, device_id: &str) -> Result<(String, String), DeviceRegError> {
        self.register_with_secret(device_id, None)
    }

    /// Register a new device: with `secret` = `Some` use the admin-provided secret (the provisioning flow can configure
    /// host first then register; the secret stays under admin control throughout, never lost -- option A); `None` = server-generated.
    /// Validation: non-empty, 8-128 chars, no whitespace (quality is the admin's responsibility; only obvious errors are caught).
    pub fn register_with_secret(
        &self,
        device_id: &str,
        secret: Option<&str>,
    ) -> Result<(String, String), DeviceRegError> {
        let mut inner = self.lock_write();
        if inner.devices.contains_key(device_id) {
            return Err(DeviceRegError::Duplicate);
        }
        let secret = match secret {
            Some(s) => {
                if s.is_empty()
                    || s.len() < 8
                    || s.len() > 128
                    || s.chars().any(|c| c.is_whitespace())
                {
                    return Err(DeviceRegError::InvalidSecret(
                        "secret: 8-128 chars, no whitespace".into(),
                    ));
                }
                s.to_string()
            }
            None => new_secret(),
        };
        let hash = hash_secret(device_id, &secret);
        inner.devices.insert(device_id.to_string(), Entry::Secret(hash.clone()));
        Ok((hash, secret))
    }

    /// Revoke a device: remove from the registry (memory). The next access auth is Unknown -> 4010.
    /// Existing online connections are unaffected (auth happens only at access time) -- operational semantics in the proposal.
    pub fn revoke(&self, device_id: &str) -> Result<(), DeviceRegError> {
        let mut inner = self.lock_write();
        inner.devices.remove(device_id).map(|_| ()).ok_or(DeviceRegError::Unknown)
    }

    /// Reset a device secret: the old secret is invalidated immediately; returns the new secret (one-time plaintext).
    pub fn reset_secret(&self, device_id: &str) -> Result<(String, String), DeviceRegError> {
        let mut inner = self.lock_write();
        // Public-key entries have no secret to reset -> treat as unregistered (404); migration handling = web delete + re-enroll (S8).
        // Without this guard, reset would silently rewrite the public-key form back to the secret form (admission semantics bypassed).
        if !matches!(inner.devices.get(device_id), Some(Entry::Secret(_))) {
            return Err(DeviceRegError::Unknown);
        }
        let secret = new_secret();
        let hash = hash_secret(device_id, &secret);
        inner.devices.insert(device_id.to_string(), Entry::Secret(hash.clone()));
        Ok((hash, secret))
    }

    // -- device-enroll: public-key admission surface (design S5.1) ---------------------------------

    /// Entry lookup (signaling state-machine form dispatch / admin assertions).
    pub fn entry_of(&self, device_id: &str) -> Option<Entry> {
        self.lock_read().devices.get(device_id).cloned()
    }

    /// Public-key device verification (S5.3, registered vk wins). Unknown -> Unknown; secret form -> BadSecret
    /// (signaling already rejects secret-form access before issuing the challenge; this is a defensive fallback).
    pub fn verify_pubkey(
        &self,
        device_id: &str,
        nonce_b64: &str,
        room_id: &str,
        sig_b64: &str,
    ) -> Result<(), DeviceAuthError> {
        let vk = match self.entry_of(device_id) {
            Some(Entry::PublicKey { vk, .. }) => vk,
            Some(Entry::Secret(_)) => return Err(DeviceAuthError::BadSecret),
            None => return Err(DeviceAuthError::Unknown),
        };
        verify_signature(&vk, nonce_b64, device_id, room_id, sig_b64)
    }

    /// Admit after verify passes (enroll_auto = shared entry for manual admin approve and auto mode): enters an
    /// entry in the registry public-key form (name optional). Persisting is done by the caller's save (same Arc = C33 hot-effect chain).
    /// vk comes from the wire -> boundary validation (canonical base64 32B key).
    pub fn enroll_auto(
        &self,
        device_id: &str,
        vk_b64: &str,
        name: Option<&str>,
    ) -> Result<(), DeviceRegError> {
        decode_vk(vk_b64).map_err(|e| {
            DeviceRegError::InvalidPublicKey(format!("public_key not canonical: {e}"))
        })?;
        let mut inner = self.lock_write();
        if inner.devices.contains_key(device_id) {
            return Err(DeviceRegError::Duplicate);
        }
        inner.devices.insert(
            device_id.to_string(),
            Entry::PublicKey { vk: vk_b64.to_string(), name: name.map(str::to_string) },
        );
        Ok(())
    }
}

/// Generate a new device secret: uuid v4 (122-bit CSPRNG entropy), 36 chars -- compatible with the G2 hash format,
/// no new dependency needed (design: uuid fallback; in phase H switch to getrandom + 32B hex if stronger entropy is needed).
fn new_secret() -> String {
    uuid::Uuid::new_v4().to_string()
}
/// sha256(device_id + ":" + device_secret), hex encoded, `sha256:` prefix.
/// device_id acts as the per-device salt -- no extra salt storage needed (G2 storage decision, see the module header).
pub fn hash_secret(device_id: &str, secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(device_id.as_bytes());
    hasher.update(b":");
    hasher.update(secret.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Dummy comparison target generated once at startup (review #1): a random device_id guarantees
/// no collision with any registered device (uuid v4), and a length equal to a real hash (71 chars) keeps the ct_eq path identical.
fn new_dummy_hash() -> String {
    hash_secret(&format!("ms-dummy-{}", uuid::Uuid::new_v4()), "dummy")
}

/// base64 -> 32B Ed25519 vk decode validation (wire input boundary; design S3: base64 standard, no newlines).
fn decode_vk(vk_b64: &str) -> Result<[u8; 32], &'static str> {
    let raw =
        base64::engine::general_purpose::STANDARD.decode(vk_b64).map_err(|_| "not valid base64")?;
    let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| "decoded length != 32 bytes")?;
    ed25519_dalek::VerifyingKey::from_bytes(&arr).map_err(|_| "invalid ed25519 public key")?;
    Ok(arr)
}

/// device-enroll S3 crypto contract: sig = Ed25519( nonce_raw(32B) || device_id || room_id ),
/// verify_strict defends against signature malleability. Registered/unknown share the same function (S5.3).
/// Every failure maps to BadSecret (uniform wire message -- anti-enumeration discipline same as review #1; distinction only in caller logs).
pub fn verify_signature(
    vk_b64: &str,
    nonce_b64: &str,
    device_id: &str,
    room_id: &str,
    sig_b64: &str,
) -> Result<(), DeviceAuthError> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let vk_arr = decode_vk(vk_b64).map_err(|_| DeviceAuthError::BadSecret)?;
    let vk =
        ed25519_dalek::VerifyingKey::from_bytes(&vk_arr).map_err(|_| DeviceAuthError::BadSecret)?;
    let mut msg = b64.decode(nonce_b64).map_err(|_| DeviceAuthError::BadSecret)?;
    if msg.len() != 32 {
        return Err(DeviceAuthError::BadSecret);
    }
    let sig_bytes = b64.decode(sig_b64).map_err(|_| DeviceAuthError::BadSecret)?;
    let sig_arr: [u8; 64] =
        sig_bytes.as_slice().try_into().map_err(|_| DeviceAuthError::BadSecret)?;
    msg.extend_from_slice(device_id.as_bytes());
    msg.extend_from_slice(room_id.as_bytes());
    vk.verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&sig_arr))
        .map_err(|_| DeviceAuthError::BadSecret)
}

/// Pending entry (design S5.1; D-E5 memory-only, not persisted -- cleared on restart, devices re-report naturally on reconnect).
#[derive(Debug, Clone)]
pub struct PendingEntry {
    pub vk: String,
    /// First-seen time (unix ms).
    pub first_seen_ms: u64,
    /// Whether verification has passed (currently the only write point is after verify succeeds -> always true; the field keeps a pending semantic surface).
    pub verified: bool,
}

/// Manual-mode pending table: the wait queue while an unknown pubkey has passed verification but no admin approve yet.
/// Same device_id reporting a new vk -> overwrite + WARN (design S5.1).
#[derive(Debug, Default, Clone)]
pub struct PendingTable {
    inner: Arc<dashmap::DashMap<String, PendingEntry>>,
}

impl PendingTable {
    /// Register/overwrite (verified is decided by the caller -- currently the only call site passes true after verify succeeds).
    pub fn insert(&self, device_id: &str, vk: &str, verified: bool) {
        let entry = PendingEntry { vk: vk.to_string(), first_seen_ms: now_ms(), verified };
        let prev = self.inner.insert(device_id.to_string(), entry);
        if let Some(prev) = prev
            && prev.vk != vk
        {
            tracing::warn!(
                "pending device {device_id} re-reported with a new key (overwrite): {} -> {}",
                prev.vk,
                vk
            );
        }
    }

    pub fn get(&self, device_id: &str) -> Option<PendingEntry> {
        self.inner.get(device_id).map(|e| e.clone())
    }

    pub fn remove(&self, device_id: &str) -> Option<PendingEntry> {
        self.inner.remove(device_id).map(|(_, v)| v)
    }

    /// All pending entries (admin list; stably sorted by device_id).
    pub fn list(&self) -> Vec<(String, PendingEntry)> {
        let mut out: Vec<(String, PendingEntry)> =
            self.inner.iter().map(|e| (e.key().clone(), e.value().clone())).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Device auth decision point (called by RoomJoin handling; pure function for easy unit testing).
///
/// Returns `None` = no device credential carried -> PSK path (keeps the original flow).
/// `Some(Err)` = incomplete shape or credential verification failed -> Error 4010 (see `DeviceAuthError::message`).
/// `Some(Ok)` = device auth passed -> connection-level identity binding (peer_id -> device_id, D-H11).
pub fn authenticate(
    registry: &DeviceRegistry,
    device_id: Option<&str>,
    device_secret: Option<&str>,
) -> Option<Result<(), DeviceAuthError>> {
    match (device_id, device_secret) {
        (None, None) => None,
        (Some(id), Some(secret)) => Some(registry.verify(id, secret)),
        _ => Some(Err(DeviceAuthError::Incomplete)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> DeviceRegistry {
        // secret = "s3cret"; hash = sha256("ms-0a1b2c3d4e5f:s3cret")
        let secret = "s3cret";
        let hash = hash_secret("ms-0a1b2c3d4e5f", secret);
        let yaml = format!("devices:\n  ms-0a1b2c3d4e5f:\n    secret_hash: \"{hash}\"\n");
        DeviceRegistry::from_yaml(&yaml).unwrap()
    }

    fn dummy_hash_of(reg: &DeviceRegistry) -> String {
        reg.lock_read().dummy_hash.clone()
    }

    #[test]
    fn hash_secret_uses_device_id_as_salt() {
        let a = hash_secret("ms-a", "same-secret");
        let b = hash_secret("ms-b", "same-secret");
        assert_ne!(a, b, "device_id must take part in the hash (salt)");
        assert!(a.starts_with("sha256:") && a.len() == "sha256:".len() + 64);
        // Stable vector: sha256("ms-a:same-secret")
        let mut h = Sha256::new();
        h.update(b"ms-a:same-secret");
        let expected: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(a, format!("sha256:{expected}"));
    }

    #[test]
    fn verify_ok_with_matching_secret() {
        let reg = test_registry();
        assert_eq!(reg.verify("ms-0a1b2c3d4e5f", "s3cret"), Ok(()));
    }

    #[test]
    fn verify_unknown_device() {
        let reg = test_registry();
        assert_eq!(reg.verify("ms-nope", "s3cret"), Err(DeviceAuthError::Unknown));
    }

    #[test]
    fn verify_wrong_secret() {
        let reg = test_registry();
        assert_eq!(reg.verify("ms-0a1b2c3d4e5f", "wrong"), Err(DeviceAuthError::BadSecret));
    }

    #[test]
    fn authenticate_shape_checks() {
        let reg = test_registry();
        // Both missing = PSK path (None)
        assert_eq!(authenticate(&reg, None, None), None);
        // Half present = Incomplete
        assert_eq!(
            authenticate(&reg, Some("ms-0a1b2c3d4e5f"), None),
            Some(Err(DeviceAuthError::Incomplete))
        );
        assert_eq!(
            authenticate(&reg, None, Some("s3cret")),
            Some(Err(DeviceAuthError::Incomplete))
        );
        // Both present = verify
        assert_eq!(authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some("s3cret")), Some(Ok(())));
        assert_eq!(
            authenticate(&reg, Some("ms-nope"), Some("s3cret")),
            Some(Err(DeviceAuthError::Unknown))
        );
    }

    #[test]
    fn from_yaml_rejects_unsupported_scheme() {
        let yaml = "devices:\n  ms-x:\n    secret_hash: \"md5:abc\"\n";
        let err = DeviceRegistry::from_yaml(yaml).unwrap_err();
        assert!(err.contains("unsupported secret_hash"), "{err}");
    }

    #[test]
    fn from_yaml_rejects_bad_hex_length() {
        let yaml = "devices:\n  ms-x:\n    secret_hash: \"sha256:abc\"\n";
        let err = DeviceRegistry::from_yaml(yaml).unwrap_err();
        assert!(err.contains("malformed"), "{err}");
    }

    #[test]
    fn empty_registry_never_authenticates() {
        let reg = DeviceRegistry::empty();
        assert!(reg.is_empty());
        assert_eq!(
            authenticate(&reg, Some("ms-x"), Some("anything")),
            Some(Err(DeviceAuthError::Unknown))
        );
    }

    #[test]
    fn error_messages_informative_and_unknown_badsecret_identical() {
        // C15: message readable; single error code 4010 (signaling.rs constant) + single message.
        assert!(DeviceAuthError::Incomplete.message().contains("both device_id"));
        assert!(DeviceAuthError::Incomplete.message().contains("device authentication failed"));
        // review #1: the wire message for unknown device and wrong secret must be byte-identical (anti-enumeration).
        assert_eq!(DeviceAuthError::Unknown.message(), DeviceAuthError::BadSecret.message());
        assert!(DeviceAuthError::Unknown.message().contains("invalid device credentials"));
    }

    #[test]
    fn unknown_vs_bad_secret_wire_response_identical() {
        // review #1 TDD: the full wire response (code=4010 + message) for both failure paths must match.
        // code is unified as 4010 at the signaling.rs auth point; here the message-layer equivalence is pinned.
        let reg = test_registry();
        let e_unknown =
            authenticate(&reg, Some("ms-nope"), Some("x")).expect("creds present").unwrap_err();
        let e_bad = authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some("wrong"))
            .expect("creds present")
            .unwrap_err();
        assert_eq!(e_unknown, DeviceAuthError::Unknown);
        assert_eq!(e_bad, DeviceAuthError::BadSecret);
        // Internal error types differ (audit can distinguish) but the wire message is the same -- anti-enumeration.
        assert_ne!(e_unknown, e_bad);
        assert_eq!(e_unknown.message(), e_bad.message());
        // Both paths must use the "device auth failed" family message (client identifies by 4010 + prefix).
        assert!(e_unknown.message().starts_with("device authentication failed"));
    }

    #[test]
    fn dummy_hash_is_per_instance_random_and_same_length() {
        // review #1: dummy is generated each startup, same length as a real hash (identical ct_eq path).
        let a = DeviceRegistry::empty();
        let b = DeviceRegistry::empty();
        assert_ne!(dummy_hash_of(&a), dummy_hash_of(&b), "dummy must be random per instance");
        assert_eq!(
            dummy_hash_of(&a).len(),
            "sha256:".len() + 64,
            "dummy is the same length as a real hash"
        );
        assert_eq!(dummy_hash_of(&a).len(), hash_secret("ms-x", "s").len());
        // Unknown devices still walk the full verify path (returns Unknown but sha256+ct_eq already ran internally).
        assert_eq!(a.verify("ms-anyone", "x"), Err(DeviceAuthError::Unknown));
    }

    #[test]
    fn save_roundtrip_preserves_entries_and_no_temp_leftover() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ms-devices-test-{}.yaml", uuid::Uuid::new_v4()));
        let reg = test_registry();
        reg.save(&path).unwrap();
        // No temp residue
        assert!(!path.with_extension("yaml.tmp").exists(), "temp file must be cleaned up");
        // reload round-trip preserves auth semantics
        let reloaded = DeviceRegistry::load(&path).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(authenticate(&reloaded, Some("ms-0a1b2c3d4e5f"), Some("s3cret")), Some(Ok(())));
        assert_eq!(
            authenticate(&reloaded, Some("ms-nope"), Some("s3cret")),
            Some(Err(DeviceAuthError::Unknown))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn register_generates_secret_and_authenticates() {
        let reg = DeviceRegistry::empty();
        let (hash, secret) = reg.register("ms-new-1").unwrap();
        assert!(hash.starts_with("sha256:") && hash.len() == "sha256:".len() + 64);
        assert_eq!(secret.len(), 36, "uuid v4 36 chars");
        // authenticate immediately after register (no restart = hot-reload semantics)
        assert_eq!(authenticate(&reg, Some("ms-new-1"), Some(&secret)), Some(Ok(())));
        assert_eq!(reg.device_ids(), vec!["ms-new-1".to_string()]);
    }

    #[test]
    fn register_duplicate_errors() {
        let reg = test_registry();
        let err = reg.register("ms-0a1b2c3d4e5f").unwrap_err();
        assert_eq!(err, DeviceRegError::Duplicate);
    }

    #[test]
    fn revoke_makes_device_unknown() {
        let reg = test_registry();
        reg.revoke("ms-0a1b2c3d4e5f").unwrap();
        assert_eq!(
            authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some("s3cret")),
            Some(Err(DeviceAuthError::Unknown))
        );
    }

    #[test]
    fn revoke_unknown_device_errors() {
        let reg = test_registry();
        assert_eq!(reg.revoke("ms-nope"), Err(DeviceRegError::Unknown));
    }

    #[test]
    fn reset_secret_invalidates_old_secret() {
        let reg = test_registry();
        let (_, new_secret) = reg.reset_secret("ms-0a1b2c3d4e5f").unwrap();
        assert_eq!(
            authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some("s3cret")),
            Some(Err(DeviceAuthError::BadSecret))
        );
        assert_eq!(authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some(&new_secret)), Some(Ok(())));
    }

    #[test]
    fn register_then_save_persists() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ms-devices-crud-{}.yaml", uuid::Uuid::new_v4()));
        let reg = DeviceRegistry::empty();
        let (_, secret) = reg.register("ms-persist-1").unwrap();
        reg.save(&path).unwrap();
        let reloaded = DeviceRegistry::load(&path).unwrap();
        assert_eq!(authenticate(&reloaded, Some("ms-persist-1"), Some(&secret)), Some(Ok(())));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn register_with_provided_secret_authenticates() {
        let reg = DeviceRegistry::empty();
        let (hash, secret) =
            reg.register_with_secret("ms-prov-1", Some("admin-chosen-secret-1")).unwrap();
        assert_eq!(secret, "admin-chosen-secret-1", "self-supplied secret returned as-is");
        assert_eq!(hash, hash_secret("ms-prov-1", "admin-chosen-secret-1"));
        assert_eq!(
            authenticate(&reg, Some("ms-prov-1"), Some("admin-chosen-secret-1")),
            Some(Ok(()))
        );
    }

    #[test]
    fn register_provided_secret_validation() {
        let reg = DeviceRegistry::empty();
        let cases = ["", "short", "has space", &"x".repeat(129)];
        for bad in cases {
            let err = reg.register_with_secret("ms-bad", Some(bad)).unwrap_err();
            assert_eq!(
                err,
                DeviceRegError::InvalidSecret("secret: 8-128 chars, no whitespace".into()),
                "{bad:?}"
            );
        }
        // Valid boundary: 8 chars
        assert!(reg.register_with_secret("ms-ok8", Some("12345678")).is_ok());
        // 128 chars
        assert!(reg.register_with_secret("ms-ok128", Some(&"x".repeat(128))).is_ok());
    }

    #[test]
    fn register_with_provided_secret_duplicate_priority() {
        let reg = test_registry();
        // Already exists -> Duplicate takes priority over validation
        assert_eq!(
            reg.register_with_secret("ms-0a1b2c3d4e5f", Some("bad-short")).unwrap_err(),
            DeviceRegError::Duplicate
        );
    }

    // --- device-enroll T2: signature byte contract (design S3) -------------------------------
    // sig = base64( Ed25519::sign( nonce_raw(32B) || device_id || room_id ) ); verify via verify_strict.
    // This vector = the host/server cross-verification anchor: batch 3 on the host side (T6) with seed=bytes(0..=31),
    // nonce=bytes(0x40..=0x5f), same device_id/room_id reproduces the same signature.

    #[test]
    fn sig_vector_binds_nonce_device_room_and_verifies_strict() {
        use base64::Engine as _;
        use ed25519_dalek::Signer; // verify_strict is a VerifyingKey built-in; no Verifier trait needed

        let b64 = base64::engine::general_purpose::STANDARD;
        let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        let vk = signing.verifying_key();

        let nonce: Vec<u8> = (0x40u8..0x60).collect();
        let device_id = "ms-0a1b2c3d4e5f";
        let room_id = "vehicle_cam0";
        let mut msg = nonce.clone();
        msg.extend_from_slice(device_id.as_bytes());
        msg.extend_from_slice(room_id.as_bytes());

        // Pin 1: vk base64 = devices.yaml public_key form fingerprint (same value as the protocol.rs pubkey case)
        assert_eq!(b64.encode(vk.to_bytes()), "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=");

        // Pin 2: byte-contract signature pinned (host batch 3 cross-verification with the same vector = same seed/nonce/ids on both sides must produce this value)
        let sig = signing.sign(&msg);
        assert_eq!(
            b64.encode(sig.to_bytes()),
            "Gnz2kGCFH6igsOfv5QW0+8aRyu/lP5ytAa8fJA0CPYP3fIX5UsYr6uTjFjqOEEFBUB2scnDffIZ1WfP9O2ECCg=="
        );

        // Pin 3: registered vk verifies (verify_strict -- S3 anti signature-malleability note)
        vk.verify_strict(&msg, &sig).unwrap();

        // Pin 4: replay with a changed room_id = D-E7 cross-room binding -> reject
        let mut other_room = nonce.clone();
        other_room.extend_from_slice(device_id.as_bytes());
        other_room.extend_from_slice(b"vehicle_cam1");
        assert!(vk.verify_strict(&other_room, &sig).is_err());
    }

    // --- device-enroll T3: dual-form registry / verify_pubkey / PendingTable / enroll_auto ---
    // Reuses the sig_vector constants (seed=bytes(0..=31) vk/nonce/sig; batch 3 host cross-verification uses the same anchor).

    const TEST_VK: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";
    const TEST_SIG_CAM0: &str =
        "Gnz2kGCFH6igsOfv5QW0+8aRyu/lP5ytAa8fJA0CPYP3fIX5UsYr6uTjFjqOEEFBUB2scnDffIZ1WfP9O2ECCg==";

    fn test_nonce_b64() -> String {
        base64::engine::general_purpose::STANDARD.encode((0x40u8..0x60).collect::<Vec<u8>>())
    }

    fn pubkey_yaml() -> String {
        format!(
            "devices:\n  ms-0a1b2c3d4e5f:\n    public_key: \"ed25519:{TEST_VK}\"\n    name: jetson-7\n"
        )
    }

    #[test]
    fn yaml_dual_form_roundtrip_preserves_shape() {
        let secret_hash = hash_secret("ms-sec000000000", "s3cret");
        let yaml = format!(
            "devices:\n  ms-sec000000000:\n    secret_hash: \"{secret_hash}\"\n  ms-pub000000000:\n    public_key: \"ed25519:{TEST_VK}\"\n    name: jetson-7\n"
        );
        let reg = DeviceRegistry::from_yaml(&yaml).unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(
            reg.entry_of("ms-pub000000000").unwrap(),
            Entry::PublicKey { vk: TEST_VK.into(), name: Some("jetson-7".into()) }
        );
        let path = format!("/tmp/ms-devices-dual-{}.yaml", uuid::Uuid::new_v4());
        reg.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("secret_hash")
                && text.contains("ed25519:")
                && text.contains("name: jetson-7"),
            "{text}"
        );
        // Write-back preserves form: the two forms do not bleed (a public_key entry must not grow a secret_hash line)
        let re = DeviceRegistry::load(&path).unwrap();
        assert_eq!(authenticate(&re, Some("ms-sec000000000"), Some("s3cret")), Some(Ok(())));
        assert_eq!(re.entry_of("ms-pub000000000"), reg.entry_of("ms-pub000000000"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn from_yaml_rejects_both_or_neither_form() {
        let both = "devices:\n  ms-x:\n    secret_hash: \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n    public_key: \"ed25519:AAAA\"\n";
        let err = DeviceRegistry::from_yaml(both).unwrap_err();
        assert!(err.contains("both secret_hash and public_key"), "{err}");
        let neither = "devices:\n  ms-x:\n    name: orphan\n";
        let err = DeviceRegistry::from_yaml(neither).unwrap_err();
        assert!(err.contains("must carry"), "{err}");
    }

    #[test]
    fn from_yaml_rejects_bad_public_key() {
        let no_prefix = "devices:\n  ms-x:\n    public_key: \"AAAAB3NzaC1yc2E=\"\n";
        let err = DeviceRegistry::from_yaml(no_prefix).unwrap_err();
        assert!(err.contains("unsupported public_key scheme"), "{err}");
        let short = "devices:\n  ms-x:\n    public_key: \"ed25519:bm90MzJieXRlcw==\"\n";
        let err = DeviceRegistry::from_yaml(short).unwrap_err();
        assert!(err.contains("malformed public_key"), "{err}");
    }

    #[test]
    fn verify_pubkey_uses_registered_vk_and_rejects_tampering() {
        let reg = DeviceRegistry::from_yaml(&pubkey_yaml()).unwrap();
        let n = test_nonce_b64();
        assert_eq!(reg.verify_pubkey("ms-0a1b2c3d4e5f", &n, "vehicle_cam0", TEST_SIG_CAM0), Ok(()));
        // Replay with a changed room (D-E7 binding) -> reject
        assert_eq!(
            reg.verify_pubkey("ms-0a1b2c3d4e5f", &n, "vehicle_cam1", TEST_SIG_CAM0),
            Err(DeviceAuthError::BadSecret)
        );
        // Changed nonce (single-use cross-connection replay surface) -> reject
        let other_nonce = base64::engine::general_purpose::STANDARD.encode([0xABu8; 32]);
        assert_eq!(
            reg.verify_pubkey("ms-0a1b2c3d4e5f", &other_nonce, "vehicle_cam0", TEST_SIG_CAM0),
            Err(DeviceAuthError::BadSecret)
        );
        // Unregistered -> Unknown; secret-form entry -> BadSecret (same-family wire message, anti-enumeration)
        assert_eq!(
            reg.verify_pubkey("ms-nope", &n, "vehicle_cam0", TEST_SIG_CAM0),
            Err(DeviceAuthError::Unknown)
        );
        let sec = test_registry();
        assert_eq!(
            sec.verify_pubkey("ms-0a1b2c3d4e5f", &n, "vehicle_cam0", TEST_SIG_CAM0),
            Err(DeviceAuthError::BadSecret)
        );
    }

    #[test]
    fn verify_signature_accepts_offered_vk_for_unknown_device() {
        // Unknown device: verification uses the reported vk (pre-enroll, S5.3 same function).
        assert_eq!(
            verify_signature(
                TEST_VK,
                &test_nonce_b64(),
                "ms-0a1b2c3d4e5f",
                "vehicle_cam0",
                TEST_SIG_CAM0
            ),
            Ok(())
        );
        // Garbage vk / garbage sig -> uniform BadSecret (no failure kind leaked).
        assert_eq!(
            verify_signature(
                "!!!",
                &test_nonce_b64(),
                "ms-0a1b2c3d4e5f",
                "vehicle_cam0",
                TEST_SIG_CAM0
            ),
            Err(DeviceAuthError::BadSecret)
        );
        assert_eq!(
            verify_signature(
                TEST_VK,
                &test_nonce_b64(),
                "ms-0a1b2c3d4e5f",
                "vehicle_cam0",
                "notb64"
            ),
            Err(DeviceAuthError::BadSecret)
        );
    }

    #[test]
    fn enroll_auto_persists_and_rejects_bad_inputs() {
        // ms-0a1b2c3d4e5f = sig_vector-bound device (the sig message includes device_id, D-E7).
        let reg = DeviceRegistry::empty();
        reg.enroll_auto("ms-0a1b2c3d4e5f", TEST_VK, Some("cam-9")).unwrap();
        assert_eq!(
            reg.entry_of("ms-0a1b2c3d4e5f").unwrap(),
            Entry::PublicKey { vk: TEST_VK.into(), name: Some("cam-9".into()) }
        );
        assert_eq!(
            reg.enroll_auto("ms-0a1b2c3d4e5f", TEST_VK, None).unwrap_err(),
            DeviceRegError::Duplicate
        );
        assert_eq!(
            reg.enroll_auto("ms-badvk", "zzz", None).unwrap_err(),
            DeviceRegError::InvalidPublicKey("public_key not canonical: not valid base64".into())
        );
        // After save+reload the public-key form still verifies directly (hot-effect chain = same Arc instance live at runtime, persisted across restart)
        let path = format!("/tmp/ms-devices-enroll-{}.yaml", uuid::Uuid::new_v4());
        reg.save(&path).unwrap();
        let re = DeviceRegistry::load(&path).unwrap();
        assert_eq!(
            re.verify_pubkey("ms-0a1b2c3d4e5f", &test_nonce_b64(), "vehicle_cam0", TEST_SIG_CAM0),
            Ok(())
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pending_table_lifecycle_with_vk_overwrite() {
        let t = PendingTable::default();
        assert!(t.is_empty());
        t.insert("ms-p1", "vk-A", true);
        t.insert("ms-p2", "vk-B", true);
        let e1 = t.get("ms-p1").unwrap();
        assert!(e1.verified && e1.vk == "vk-A" && e1.first_seen_ms > 0);
        assert_eq!(
            t.list().iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["ms-p1", "ms-p2"]
        );
        // Same id re-reported with a new vk -> overwrite + WARN (first_seen refreshed to the re-report time, D-E5 semantics)
        std::thread::sleep(std::time::Duration::from_millis(3));
        t.insert("ms-p1", "vk-C", true);
        assert_eq!(t.len(), 2);
        let e1b = t.get("ms-p1").unwrap();
        assert_eq!(e1b.vk, "vk-C");
        assert!(e1b.first_seen_ms > e1.first_seen_ms);
        assert_eq!(t.remove("ms-p1").unwrap().vk, "vk-C");
        assert!(t.get("ms-p1").is_none());
        assert!(t.remove("ms-nope").is_none());
    }

    #[test]
    fn pubkey_entry_isolated_from_secret_surface() {
        // secret path against a public-key entry: authenticate=BadSecret family / reset_secret=Unknown / register=Duplicate
        let reg = DeviceRegistry::from_yaml(&pubkey_yaml()).unwrap();
        assert_eq!(
            authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some("whatever")),
            Some(Err(DeviceAuthError::BadSecret))
        );
        assert_eq!(reg.reset_secret("ms-0a1b2c3d4e5f").unwrap_err(), DeviceRegError::Unknown);
        assert_eq!(reg.register("ms-0a1b2c3d4e5f").unwrap_err(), DeviceRegError::Duplicate);
        // Timing surface: unknown devices and public-key entries both walk the full dummy path (response times indistinguishable, review #1)
        assert_eq!(
            authenticate(&reg, Some("ms-ghost"), Some("x")),
            Some(Err(DeviceAuthError::Unknown))
        );
    }
}
