//! API-key registry -- credential path for clients without accounts (accountless-client-auth T1).
//!
//! Shape mirrors the LiveKit apikey/apiSecret model: `POST /api/auth/exchange {key_id, secret}`
//! -> verify key -> sign a short-lived JWT `{sub:"apikey:<id>", role, vehicles, exp}` with `admin_jwt_secret`
//! -> go through the existing `/ws` handshake gate (verification/roles/matrix unchanged, see signaling.rs).
//!
//! Discipline mirrors [devices] (deliberately reusing its pattern rather than inventing a new one):
//! - YAML single source of truth + `RwLock` hot effect + atomic save (temp+fsync+rename; memory unchanged on failure);
//! - stores only `sha256(key_id + ":" + secret)` (reuses [`crate::devices::hash_secret`], key_id as salt);
//! - Unknown and BadSecret use **byte-identical messages** (anti-enumeration, same family as review #1); dummy hash pads to equal time;
//! - secret plaintext is returned only once in the register response (C33 semantics); lost = revoke and re-register, no recovery path.
//!
//! Revocation granularity (F10): deleting a key only blocks future exchanges; issued JWTs die naturally at exp (window = TTL).
//! The `sub` namespace prefix `apikey:` keeps API keys isolated from account usernames (F5, auditable attribution).

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::sync::{PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::devices::hash_secret;
use crate::roles::CockpitRole;
use mediaservo_common::auth::JwtClaims;
use mediaservo_common::error::CoreError;

/// key_id lexing (anti YAML-injection / path ambiguity, same discipline as validate_device_id).
fn valid_key_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// key_id is the **key** (not a username); only the secret hash is persisted. vehicles defaults to empty
/// (effective for viewer/operator; ignored for admin/dispatcher = any vehicle -- same as the G3 matrix).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileEntry {
    secret_hash: String,
    role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vehicles: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiKeyEntry {
    pub secret_hash: [u8; 32],
    pub role: CockpitRole,
    pub vehicles: Vec<String>,
    pub label: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    api_keys: BTreeMap<String, FileEntry>,
}

#[derive(Debug)]
struct Inner {
    keys: HashMap<String, ApiKeyEntry>,
    /// Timing-compensation target for unknown keys (random id salt, length always equals a real hash -- same shape as devices.rs review #1).
    dummy_hash: [u8; 32],
}

pub struct ApiKeyRegistry {
    inner: RwLock<Inner>,
}

/// register/revoke errors (admin-plane 400/409/404 mapping).
#[derive(Debug, PartialEq, Eq)]
pub enum ApiKeyRegError {
    Duplicate,
    Unknown,
    Invalid(String),
}

/// exchange authentication failure (externally byte-identical message, anti-enumeration).
#[derive(Debug, PartialEq, Eq)]
pub enum ApiKeyAuthError {
    Unknown,
    BadSecret,
}

impl ApiKeyAuthError {
    #[must_use]
    pub fn message(&self) -> &'static str {
        // Unknown and BadSecret must be byte-identical (integration test pins this) -- the exchange response carries no distinguishing info.
        "api key authentication failed: invalid credentials"
    }
}

impl ApiKeyRegistry {
    pub fn empty() -> Self {
        Self { inner: RwLock::new(Inner { keys: HashMap::new(), dummy_hash: new_dummy_hash() }) }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| CoreError::ConfigParse(format!("api_keys file: {e}")))?;
        Self::from_yaml(&text)
    }

    pub fn from_yaml(text: &str) -> Result<Self, CoreError> {
        let file: RegistryFile = serde_yaml::from_str(text)
            .map_err(|e| CoreError::ConfigParse(format!("api_keys yaml: {e}")))?;
        let mut keys = HashMap::new();
        for (id, fe) in file.api_keys {
            if !valid_key_id(&id) {
                return Err(CoreError::ConfigParse(format!("api_keys: invalid key_id {id:?}")));
            }
            let role = CockpitRole::parse(&fe.role).ok_or_else(|| {
                CoreError::ConfigParse(format!(
                    "api_keys[{id}]: role {:?} not in viewer|operator|admin|dispatcher",
                    fe.role
                ))
            })?;
            let hash = decode_hash(&fe.secret_hash).ok_or_else(|| {
                CoreError::ConfigParse(format!("api_keys[{id}]: bad secret_hash"))
            })?;
            keys.insert(
                id,
                ApiKeyEntry {
                    secret_hash: hash,
                    role,
                    vehicles: fe.vehicles.unwrap_or_default(),
                    label: fe.label,
                },
            );
        }
        Ok(Self { inner: RwLock::new(Inner { keys, dummy_hash: new_dummy_hash() }) })
    }

    fn to_yaml(&self) -> Result<String, CoreError> {
        let inner = self.lock_read();
        let file = RegistryFile {
            api_keys: inner
                .keys
                .iter()
                .map(|(id, e)| {
                    (
                        id.clone(),
                        FileEntry {
                            secret_hash: format!(
                                "sha256:{}",
                                e.secret_hash
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect::<String>()
                            ),
                            role: e.role.as_str().to_string(),
                            vehicles: if e.vehicles.is_empty() {
                                None
                            } else {
                                Some(e.vehicles.clone())
                            },
                            label: e.label.clone(),
                        },
                    )
                })
                .collect(),
        };
        serde_yaml::to_string(&file)
            .map_err(|e| CoreError::ConfigParse(format!("api_keys serialize: {e}")))
    }

    /// Atomic write-back of api_keys.yaml (temp + fsync + rename; memory unchanged on failure -- same discipline as devices.rs).
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
                let _ = std::fs::remove_file(&tmp);
                Err(CoreError::ConfigParse(format!(
                    "api_keys file {}: write failed: {e}",
                    path.display()
                )))
            }
        }
    }

    /// Register: server generates the secret (uuid v4, 36 chars, same entropy source as devices); plaintext returned **only this once**.
    /// Role closed-set validation (invalid -> Invalid, caught at deploy time; never silently store an out-of-matrix role).
    pub fn register(
        &self,
        key_id: &str,
        role: &str,
        vehicles: &[String],
        label: Option<&str>,
    ) -> Result<String, ApiKeyRegError> {
        if !valid_key_id(key_id) {
            return Err(ApiKeyRegError::Invalid("key_id: 1-64 chars [A-Za-z0-9-_]".into()));
        }
        let role = CockpitRole::parse(role).ok_or_else(|| {
            ApiKeyRegError::Invalid("role: viewer|operator|admin|dispatcher".into())
        })?;
        let secret = uuid::Uuid::new_v4().to_string();
        let hash = decode_hash(&hash_secret(key_id, &secret))
            .expect("hash_secret output is always decodable");
        let mut inner = self.lock_write();
        if inner.keys.contains_key(key_id) {
            return Err(ApiKeyRegError::Duplicate);
        }
        inner.keys.insert(
            key_id.to_string(),
            ApiKeyEntry {
                secret_hash: hash,
                role,
                vehicles: vehicles.to_vec(),
                label: label.map(str::to_string),
            },
        );
        Ok(secret)
    }

    pub fn revoke(&self, key_id: &str) -> Result<(), ApiKeyRegError> {
        self.lock_write().keys.remove(key_id).map(|_| ()).ok_or(ApiKeyRegError::Unknown)
    }

    /// List (admin plane; secret_hash never leaves memory -- mapped to a plaintext-safe shape).
    #[must_use]
    pub fn list(&self) -> Vec<(String, String, Vec<String>, Option<String>)> {
        let inner = self.lock_read();
        let mut out: Vec<_> = inner
            .keys
            .iter()
            .map(|(id, e)| {
                (id.clone(), e.role.as_str().to_string(), e.vehicles.clone(), e.label.clone())
            })
            .collect();
        out.sort();
        out
    }

    /// exchange authentication decision point: constant-time secret comparison; unknown keys go through a dummy equal-length comparison (no existence timing leak).
    pub fn verify(&self, key_id: &str, secret: &str) -> Result<ApiKeyEntry, ApiKeyAuthError> {
        let inner = self.lock_read();
        // hash_secret output is "sha256:<hex>"; decode failure (theoretically unreachable) = all-zero pad that never matches.
        let provided = decode_hash(&hash_secret(key_id, secret)).unwrap_or([0u8; 32]);
        match inner.keys.get(key_id) {
            Some(e) => {
                if bool::from(e.secret_hash.ct_eq(&provided)) {
                    Ok(e.clone())
                } else {
                    Err(ApiKeyAuthError::BadSecret)
                }
            }
            None => {
                let _ = inner.dummy_hash.ct_eq(&provided); // timing compensation
                Err(ApiKeyAuthError::Unknown)
            }
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lock_read().keys.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock_read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// exchange issuance: same signing infrastructure as accounts::issue_account_token (admin_jwt_secret),
/// difference = the `sub` gets the `apikey:` namespace prefix (F5 audit attribution + username isolation).
pub fn issue_api_token(
    secret: &str,
    key_id: &str,
    entry: &ApiKeyEntry,
    ttl_secs: u64,
) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("clock error: {e}"))?
        .as_secs() as usize;
    let claims = JwtClaims {
        sub: format!("apikey:{key_id}"),
        iat: now,
        exp: now + ttl_secs as usize,
        role: Some(entry.role.as_str().to_string()),
        vehicles: Some(entry.vehicles.clone()),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| format!("JWT encode error: {e}"))
}

/// `sha256:<64 hex>` -> 32B; wrong shape = None (rejected at load time, never guessed at runtime).
fn decode_hash(s: &str) -> Option<[u8; 32]> {
    let hex = s.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut out = [0u8; 32];
    for (i, chunk) in bytes.chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// Dummy comparison target generated once at startup (random key_id salt guarantees no collision with any registered key).
fn new_dummy_hash() -> [u8; 32] {
    let h = hash_secret(&format!("apikey-dummy-{}", uuid::Uuid::new_v4()), "dummy");
    decode_hash(&h).expect("hash_secret output is always decodable")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_yaml(key: &str, secret: &str, role: &str) -> String {
        let h = hash_secret(key, secret);
        format!("api_keys:\n  {key}:\n    secret_hash: \"{h}\"\n    role: {role}\n")
    }

    #[test]
    fn load_roundtrip_preserves_shape_and_salts() {
        let reg =
            ApiKeyRegistry::from_yaml(&entry_yaml("cockpit-1", "s3cret", "viewer")).expect("load");
        let yaml = reg.to_yaml().unwrap();
        let back = ApiKeyRegistry::from_yaml(&yaml).unwrap();
        assert_eq!(back.verify("cockpit-1", "s3cret").unwrap().role.as_str(), "viewer");
        // Salt = key_id: same secret, different key -> different hash (same pin as devices).
        assert_ne!(hash_secret("a", "x"), hash_secret("b", "x"));
    }

    #[test]
    fn unknown_and_badsecret_messages_identical() {
        let reg = ApiKeyRegistry::from_yaml(&entry_yaml("k", "right", "operator")).unwrap();
        let a = reg.verify("k", "wrong").unwrap_err();
        let b = reg.verify("nope", "right").unwrap_err();
        assert_eq!(a.message(), b.message());
        assert_eq!(a, ApiKeyAuthError::BadSecret);
        assert_eq!(b, ApiKeyAuthError::Unknown);
    }

    #[test]
    fn role_closed_set_rejects_illegal_at_load_and_register() {
        assert!(ApiKeyRegistry::from_yaml(&entry_yaml("k", "s", "superuser")).is_err());
        let reg = ApiKeyRegistry::empty();
        assert!(matches!(
            reg.register("k", "superuser", &[], None),
            Err(ApiKeyRegError::Invalid(_))
        ));
        assert!(matches!(
            reg.register("bad id!!", "viewer", &[], None),
            Err(ApiKeyRegError::Invalid(_))
        ));
    }

    #[test]
    fn register_returns_secret_once_and_verify_passes() {
        let reg = ApiKeyRegistry::empty();
        let secret =
            reg.register("ci-1", "viewer", &["vehicle_a".into()], Some("ci integration")).unwrap();
        assert!(matches!(
            reg.register("ci-1", "viewer", &[], None),
            Err(ApiKeyRegError::Duplicate)
        ));
        let e = reg.verify("ci-1", &secret).unwrap();
        assert_eq!(e.vehicles, vec!["vehicle_a".to_string()]);
        assert!(reg.verify("ci-1", "typo").is_err());
    }

    #[test]
    fn issued_token_claims_shape() {
        let reg = ApiKeyRegistry::from_yaml(&entry_yaml("k", "s", "dispatcher")).unwrap();
        let e = reg.verify("k", "s").unwrap();
        let tok = issue_api_token("test-secret-min-32-bytes!!!", "k", &e, 3600).unwrap();
        let claims: JwtClaims = jsonwebtoken::decode::<JwtClaims>(
            &tok,
            &jsonwebtoken::DecodingKey::from_secret(b"test-secret-min-32-bytes!!!"),
            &jsonwebtoken::Validation::default(),
        )
        .unwrap()
        .claims;
        assert_eq!(claims.sub, "apikey:k", "F5: sub must use the apikey: namespace");
        assert_eq!(claims.role.as_deref(), Some("dispatcher"));
        assert!(claims.exp > claims.iat + 3590 && claims.exp <= claims.iat + 3600);
    }

    #[test]
    fn decode_hash_shape_guard() {
        assert!(decode_hash(&hash_secret("k", "s")).is_some());
        assert!(decode_hash("sha256:zz").is_none());
        assert!(decode_hash("nope").is_none());
    }
}
