//! G3 cockpit account registry (D-H11 dual-class identity, operator side) -- login auth + token issuance.
//!
//! File-based config (YAML, same shape as devices.yaml), format:
//! ```yaml
//! accounts:
//!   carol:                       # username (JWT sub)
//!     password_hash: "sha256:<hex>"   # sha256(username + ":" + password)
//!     role: operator             # viewer|operator|admin|dispatcher
//!     vehicles: ["ms-car1"]      # vehicle x cockpit allowlist (omittable for admin/dispatcher)
//! ```
//! Storage decision same as G2 devices: one-way hash only (username as salt); subtle constant-time comparison;
//! unknown-user and wrong-password wire responses are byte-identical (anti-enumeration). Upgrade path same as G2: argon2id prefix.
//! Token design (G3 adopts D-H11 option 2): on login a JWT is issued
//! `{sub: username, role, vehicles, iat, exp}`, same secret as the admin JWT (admin_jwt_secret)
//! and same algorithm (HS256) -- reuses the existing JwtAuth/middleware machinery, no second signing system.

use mediaservo_common::auth::JwtClaims;
use mediaservo_common::error::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::{RwLock, RwLockReadGuard};
use subtle::ConstantTimeEq;

use crate::roles::{AccountIdentity, CockpitRole};

/// Account auth failure reason (wire returns a uniform 401; internal distinction goes to the audit log only -- anti-enumeration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAuthError {
    /// User is not in the registry.
    Unknown,
    /// Password hash mismatch.
    BadPassword,
}

impl AccountAuthError {
    /// Client-facing readable message -- unknown-user and wrong-password must be byte-identical (anti-enumeration, same as G2).
    pub fn message(&self) -> &'static str {
        match self {
            AccountAuthError::Unknown | AccountAuthError::BadPassword => {
                "account authentication failed: invalid credentials"
            }
        }
    }
}

/// Registry management operation errors (admin API; 400/404/409 mapping in admin.rs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountRegError {
    /// Username already exists on create.
    Duplicate,
    /// Username does not exist on update/delete.
    Unknown,
    /// Role is not one of viewer|operator|admin|dispatcher.
    InvalidRole(String),
    /// vehicles contains an empty item or invalid characters.
    InvalidVehicles(String),
}

/// Admin list view (no password_hash -- the password hash never leaks).
#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub username: String,
    pub role: String,
    pub vehicles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AccountEntryFile {
    password_hash: String,
    role: String,
    #[serde(default)]
    vehicles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct AccountsFile {
    #[serde(default)]
    accounts: HashMap<String, AccountEntryFile>,
}

#[derive(Debug, Clone)]
struct AccountEntry {
    password_hash: String,
    role: CockpitRole,
    vehicles: Vec<String>,
}

/// Cockpit account registry (loaded at startup, read-only; empty = no accounts, PSK/device path unaffected).
/// Registry internal state (wrapped in RwLock).
#[derive(Debug)]
struct AccountRegistryInner {
    accounts: HashMap<String, AccountEntry>,
}

impl AccountRegistryInner {
    fn new(accounts: HashMap<String, AccountEntry>) -> Self {
        Self { accounts }
    }
}

/// Cockpit account registry (loaded at startup; hot-reloadable read/write at runtime).
/// Empty = no accounts, PSK/device path unaffected.
#[derive(Debug)]
pub struct AccountRegistry {
    inner: RwLock<AccountRegistryInner>,
}

impl Default for AccountRegistry {
    fn default() -> Self {
        Self { inner: RwLock::new(AccountRegistryInner::new(HashMap::new())) }
    }
}

impl AccountRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Standard lock-poison recovery: unpoisoned read lock; on poison take the consistent value left by the writer.
    fn lock_read(&self) -> RwLockReadGuard<'_, AccountRegistryInner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_write(&self) -> std::sync::RwLockWriteGuard<'_, AccountRegistryInner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Load from a YAML file.
    ///
    /// File **missing** -> empty registry (`Ok` -- PSK/device path unaffected, kept per I4 review);
    /// file **exists but is corrupt** -> `Err` (fail-fast: the starter must refuse to continue -- a silently degraded registry
    /// = authorization enforcement silently disabled; same discipline as identity.json corruption reporting loudly).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let content = match std::fs::read_to_string(path.as_ref()) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::empty());
            }
            Err(e) => {
                return Err(CoreError::ConfigParse(format!(
                    "accounts file {}: {e}",
                    path.as_ref().display()
                )));
            }
        };
        Self::from_yaml(&content).map_err(|e| {
            CoreError::ConfigParse(format!("accounts file {}: {e}", path.as_ref().display()))
        })
    }

    /// Parse from YAML text (shared by tests and load). Unknown role -> parse error (account unusable).
    pub fn from_yaml(content: &str) -> Result<Self, String> {
        let file: AccountsFile =
            serde_yaml::from_str(content).map_err(|e| format!("YAML parse error: {e}"))?;
        let mut accounts = HashMap::new();
        for (username, entry) in file.accounts {
            if !entry.password_hash.starts_with("sha256:") {
                return Err(format!(
                    "account {username}: unsupported password_hash scheme (want sha256:)"
                ));
            }
            if entry.password_hash.len() != "sha256:".len() + 64 {
                return Err(format!("account {username}: malformed sha256 hex length"));
            }
            let role = CockpitRole::parse(&entry.role).ok_or_else(|| {
                format!(
                    "account {username}: unknown role {:?} (want viewer|operator|admin|dispatcher)",
                    entry.role
                )
            })?;
            accounts.insert(
                username,
                AccountEntry { password_hash: entry.password_hash, role, vehicles: entry.vehicles },
            );
        }
        Ok(Self { inner: RwLock::new(AccountRegistryInner::new(accounts)) })
    }

    pub fn len(&self) -> usize {
        self.lock_read().accounts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock_read().accounts.is_empty()
    }

    /// Authentication decision (called by the login handler): validation returns the account identity (goes into JWT claims).
    fn verify(&self, username: &str, password: &str) -> Result<AccountIdentity, AccountAuthError> {
        let inner = self.lock_read();
        let known = inner.accounts.contains_key(username);
        // Unknown users also walk the full sha256 + ct_eq path (anti-timing, same as the G2 dummy mechanism --
        // here a fixed string is the target, length equal to a real hash).
        let stored =
            inner.accounts.get(username).map(|e| e.password_hash.as_str()).unwrap_or(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            );
        let want = hash_password(username, password);
        let matched: bool = stored.as_bytes().ct_eq(want.as_bytes()).into();
        match (matched, known) {
            (true, _) => {
                let entry = &inner.accounts[username];
                Ok(AccountIdentity {
                    username: username.to_string(),
                    role: entry.role,
                    vehicles: entry.vehicles.clone(),
                })
            }
            (false, true) => Err(AccountAuthError::BadPassword),
            (false, false) => Err(AccountAuthError::Unknown),
        }
    }

    /// Login entry point (called by the admin.rs login handler).
    pub fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AccountIdentity, AccountAuthError> {
        self.verify(username, password)
    }

    /// I2 review: detect whether the registry contains known dev placeholder hashes (matched usernames returned).
    pub fn dev_credentials_present(&self) -> Vec<&'static str> {
        let inner = self.lock_read();
        DEV_CREDENTIAL_HASHES
            .iter()
            .filter(|(_, hash)| inner.accounts.values().any(|e| e.password_hash == *hash))
            .map(|(user, _)| *user)
            .collect()
    }

    /// I2 review: startup guard -- contains dev placeholder accounts -> Err (fail-fast, same as the corrupt-registry discipline);
    /// `allow_dev` = explicit override (env MEDIASERVO_ALLOW_DEV_CREDENTIALS=1, set only in dev compose).
    pub fn check_dev_credentials(&self, allow_dev: bool) -> Result<(), String> {
        let found = self.dev_credentials_present();
        if found.is_empty() || allow_dev {
            return Ok(());
        }
        Err(format!(
            "DEVELOPMENT CREDENTIALS DETECTED in accounts registry ({}):              default dev accounts (admin123/dispatch123/operator123) are placeholder              credentials for local dev only — refuse to start. Replace them with real              hashes, or set MEDIASERVO_ALLOW_DEV_CREDENTIALS=1 to explicitly allow              (dev environment only).",
            found.join(", ")
        ))
    }

    /// Serialize to YAML file content (inverse of from_yaml format).
    fn to_yaml(&self) -> Result<String, CoreError> {
        let inner = self.lock_read();
        let accounts = inner
            .accounts
            .iter()
            .map(|(username, entry)| {
                (
                    username.clone(),
                    AccountEntryFile {
                        password_hash: entry.password_hash.clone(),
                        role: entry.role.as_str().to_string(),
                        vehicles: entry.vehicles.clone(),
                    },
                )
            })
            .collect();
        let file = AccountsFile { accounts };
        serde_yaml::to_string(&file)
            .map_err(|e| CoreError::ConfigParse(format!("accounts serialize: {e}")))
    }

    /// Atomic write-back of accounts.yaml (temp + fsync + rename).
    /// Memory unchanged: Ok only when serialization and disk write both succeed; on failure clean up temp and return Err.
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
                    "accounts file {}: write failed: {e}",
                    path.display()
                )))
            }
        }
    }

    /// vehicles validation: non-empty list, each item non-empty and only [A-Za-z0-9-_] (same charset as device_id).
    fn validate_vehicles(vehicles: &[String]) -> Result<(), AccountRegError> {
        if vehicles.iter().any(|v| {
            v.is_empty()
                || v.len() > 64
                || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }) {
            return Err(AccountRegError::InvalidVehicles(
                "vehicles: each item 1-64 chars [A-Za-z0-9-_]".into(),
            ));
        }
        Ok(())
    }

    /// Create account: hash_password + insert (in memory). Persisting is done by the admin API layer calling `save`.
    pub fn create_account(
        &self,
        username: &str,
        password: &str,
        role: &str,
        vehicles: &[String],
    ) -> Result<(), AccountRegError> {
        let role = CockpitRole::parse(role)
            .ok_or_else(|| AccountRegError::InvalidRole(role.to_string()))?;
        Self::validate_vehicles(vehicles)?;
        let mut inner = self.lock_write();
        if inner.accounts.contains_key(username) {
            return Err(AccountRegError::Duplicate);
        }
        inner.accounts.insert(
            username.to_string(),
            AccountEntry {
                password_hash: hash_password(username, password),
                role,
                vehicles: vehicles.to_vec(),
            },
        );
        Ok(())
    }

    /// Update account (role/vehicles/password as requested): None fields keep their old value.
    pub fn update_account(
        &self,
        username: &str,
        role: Option<&str>,
        vehicles: Option<&[String]>,
        new_password: Option<&str>,
    ) -> Result<(), AccountRegError> {
        let role = match role {
            Some(r) => Some(
                CockpitRole::parse(r).ok_or_else(|| AccountRegError::InvalidRole(r.to_string()))?,
            ),
            None => None,
        };
        if let Some(v) = vehicles {
            Self::validate_vehicles(v)?;
        }
        let mut inner = self.lock_write();
        let entry = inner.accounts.get_mut(username).ok_or(AccountRegError::Unknown)?;
        if let Some(r) = role {
            entry.role = r;
        }
        if let Some(v) = vehicles {
            entry.vehicles = v.to_vec();
        }
        if let Some(p) = new_password {
            entry.password_hash = hash_password(username, p);
        }
        Ok(())
    }

    /// Delete account.
    pub fn delete_account(&self, username: &str) -> Result<(), AccountRegError> {
        let mut inner = self.lock_write();
        inner.accounts.remove(username).map(|_| ()).ok_or(AccountRegError::Unknown)
    }

    /// Admin list (no password_hash).
    pub fn list_accounts(&self) -> Vec<AccountView> {
        let mut views: Vec<AccountView> = self
            .lock_read()
            .accounts
            .iter()
            .map(|(username, entry)| AccountView {
                username: username.clone(),
                role: entry.role.as_str().to_string(),
                vehicles: entry.vehicles.clone(),
            })
            .collect();
        views.sort_by(|a, b| a.username.cmp(&b.username));
        views
    }
}

/// I2 review: known dev placeholder account hashes (config/accounts.yaml admin123/
/// dispatch123/operator123) -- hash = sha256(username:password), with the username salt included.
/// The production startup guard (check_dev_credentials) rejects; dev compose explicitly exempts via
/// MEDIASERVO_ALLOW_DEV_CREDENTIALS=1.
pub const DEV_CREDENTIAL_HASHES: &[(&str, &str)] = &[
    ("admin", "sha256:bf6b5bdb74c79ece9fc0ad0ac9fb0359f9555d4f35a83b2e6ec69ae99e09603d"),
    ("dispatcher", "sha256:51f00e625e5fb3aff1c5a55eff96d1f2f03273afd8b0bc2514961e33dd82f8b2"),
    ("operator", "sha256:21cfc6b0fe8e257247937406f1ee83ae8acd3dc447c38b8431abecaf6d7ea437"),
];

/// sha256(username + ":" + password), hex encoded, `sha256:` prefix (username as salt).
pub fn hash_password(username: &str, password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(username.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Issue a JWT for an account identity (HS256, admin_jwt_secret -- same secret and algorithm as the admin middleware).
/// claims: {sub: username, role, vehicles, iat, exp}.
pub fn issue_account_token(
    secret: &str,
    identity: &AccountIdentity,
    ttl_secs: u64,
) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("clock error: {e}"))?
        .as_secs() as usize;
    let claims = JwtClaims {
        sub: identity.username.clone(),
        iat: now,
        exp: now + ttl_secs as usize,
        role: Some(identity.role.as_str().to_string()),
        vehicles: Some(identity.vehicles.clone()),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| format!("JWT encode error: {e}"))
}

/// Startup key-pairing check (I2 review -- fail-open fix):
/// account tokens are signed by login with `admin_jwt_secret`, but the /ws handshake verifies with `jwt_secret` --
/// when they differ, account token verification fails -> silent PSK/Legacy fallback -> matrix enforcement bypassed.
/// With accounts configured they must be equal (not equal -> Err, starter fail-fast).
pub fn validate_secret_pairing(
    accounts_configured: bool,
    jwt_secret: Option<&str>,
    admin_jwt_secret: Option<&str>,
) -> Result<(), String> {
    if !accounts_configured {
        return Ok(()); // no accounts -> no matrix to bypass (PSK/device path unaffected)
    }
    match (jwt_secret, admin_jwt_secret) {
        (Some(a), Some(b)) if a != b => Err("jwt_secret and admin_jwt_secret differ: account tokens are signed with admin_jwt_secret and             the /ws handshake verifies with jwt_secret; a mismatch makes account auth fail silently and fall back to PSK (matrix bypass).             Configure the same secret".to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> AccountRegistry {
        // password = "s3cret"; hash = sha256("carol:s3cret")
        let hash = hash_password("carol", "s3cret");
        let yaml = format!(
            "accounts:\n  carol:\n    password_hash: \"{hash}\"\n    role: operator\n    vehicles: [\"ms-car1\"]\n"
        );
        AccountRegistry::from_yaml(&yaml).unwrap()
    }

    #[test]
    fn from_yaml_parses_account_with_allowlist() {
        let reg = test_registry();
        assert_eq!(reg.len(), 1);
        let id = reg.authenticate("carol", "s3cret").unwrap();
        assert_eq!(id.username, "carol");
        assert_eq!(id.role, CockpitRole::Operator);
        assert_eq!(id.vehicles, vec!["ms-car1".to_string()]);
    }

    #[test]
    fn authenticate_unknown_user_and_wrong_password_identical_wire() {
        let reg = test_registry();
        let e_unknown = reg.authenticate("nobody", "s3cret").unwrap_err();
        let e_bad = reg.authenticate("carol", "wrong").unwrap_err();
        assert_ne!(e_unknown, e_bad, "internal distinction kept (audit)");
        assert_eq!(
            e_unknown.message(),
            e_bad.message(),
            "wire must be byte-identical (anti-enumeration)"
        );
        assert!(e_unknown.message().starts_with("account authentication failed"));
        // Empty registry: every user fails
        assert_eq!(
            AccountRegistry::empty().authenticate("carol", "s3cret").unwrap_err(),
            AccountAuthError::Unknown
        );
    }

    #[test]
    fn from_yaml_rejects_unknown_role() {
        let hash = hash_password("x", "y");
        let yaml = format!("accounts:\n  x:\n    password_hash: \"{hash}\"\n    role: superuser\n");
        let err = AccountRegistry::from_yaml(&yaml).unwrap_err();
        assert!(err.contains("unknown role"), "{err}");
    }

    #[test]
    fn from_yaml_rejects_bad_hash_scheme_and_length() {
        let yaml = "accounts:\n  x:\n    password_hash: \"md5:abc\"\n    role: viewer\n";
        assert!(AccountRegistry::from_yaml(yaml).unwrap_err().contains("unsupported"));
        let yaml = "accounts:\n  x:\n    password_hash: \"sha256:abc\"\n    role: viewer\n";
        assert!(AccountRegistry::from_yaml(yaml).unwrap_err().contains("malformed"));
    }

    #[test]
    fn hash_password_uses_username_as_salt() {
        assert_ne!(hash_password("a", "same"), hash_password("b", "same"));
        assert!(hash_password("a", "p").starts_with("sha256:"));
        // Stable vectors
        let mut h = Sha256::new();
        h.update(b"carol:s3cret");
        let expected: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash_password("carol", "s3cret"), format!("sha256:{expected}"));
    }

    #[test]
    fn issue_token_roundtrips_with_role_and_vehicles() {
        let id = AccountIdentity {
            username: "carol".into(),
            role: CockpitRole::Operator,
            vehicles: vec!["ms-car1".into()],
        };
        let token = issue_account_token("test-secret-32-bytes-min!!", &id, 3600).unwrap();
        let auth = mediaservo_common::auth::JwtAuth::new("test-secret-32-bytes-min!!");
        let claims = auth.verify(&token).unwrap();
        assert_eq!(claims.sub, "carol");
        assert_eq!(claims.role.as_deref(), Some("operator"));
        assert_eq!(claims.vehicles.as_deref(), Some(&["ms-car1".to_string()][..]));
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn secret_pairing_validation() {
        // I2 review: accounts configured + key divergence -> Err (fail-fast); equal/no accounts -> Ok
        assert!(
            validate_secret_pairing(true, Some("aaa"), Some("bbb")).is_err(),
            "accounts + key divergence must error"
        );
        let err = validate_secret_pairing(true, Some("aaa"), Some("bbb")).unwrap_err();
        assert!(err.contains("jwt_secret") && err.contains("admin_jwt_secret"), "{err}");
        assert_eq!(validate_secret_pairing(true, Some("aaa"), Some("aaa")), Ok(()));
        // No accounts -> divergence harmless (no matrix to bypass); one side missing -> cannot diverge
        assert_eq!(validate_secret_pairing(false, Some("aaa"), Some("bbb")), Ok(()));
        assert_eq!(validate_secret_pairing(true, None, Some("bbb")), Ok(()));
        assert_eq!(validate_secret_pairing(true, Some("aaa"), None), Ok(()));
        assert_eq!(validate_secret_pairing(true, None, None), Ok(()));
    }

    #[test]
    fn load_missing_file_ok_empty_and_malformed_err() {
        // I4 review: missing -> empty (non-blocking); corrupt -> Err (fail-fast, silent degradation forbidden)
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.yaml");
        assert!(AccountRegistry::load(&missing).unwrap().is_empty(), "missing = empty registry");

        let bad = dir.path().join("bad.yaml");
        std::fs::write(&bad, "accounts: [not-a-map").unwrap();
        let err = AccountRegistry::load(&bad).unwrap_err();
        assert!(err.to_string().contains("accounts file"), "{err}");
        assert!(err.to_string().contains("bad.yaml"), "{err}");

        let good = dir.path().join("good.yaml");
        let hash = hash_password("carol", "s3cret");
        std::fs::write(
            &good,
            format!("accounts:\n  carol:\n    password_hash: \"{hash}\"\n    role: viewer\n"),
        )
        .unwrap();
        assert_eq!(AccountRegistry::load(&good).unwrap().len(), 1);
    }

    #[test]
    fn admin_and_dispatcher_roles_parse() {
        let hash_adm = hash_password("adm", "p");
        let hash_disp = hash_password("disp", "p");
        let yaml = format!(
            "accounts:\n  adm:\n    password_hash: \"{hash_adm}\"\n    role: admin\n  disp:\n    password_hash: \"{hash_disp}\"\n    role: dispatcher\n"
        );
        let reg = AccountRegistry::from_yaml(&yaml).unwrap();
        assert_eq!(reg.authenticate("adm", "p").unwrap().role, CockpitRole::Admin);
        assert_eq!(reg.authenticate("disp", "p").unwrap().role, CockpitRole::Dispatcher);
        // vehicles defaults to empty
        assert!(reg.authenticate("adm", "p").unwrap().vehicles.is_empty());
    }

    // -- I2 review: dev placeholder account guard --------------------------------------------

    /// Registry containing dev placeholder hashes (same shape as accounts.yaml)
    fn dev_registry() -> AccountRegistry {
        AccountRegistry::from_yaml(
            "accounts:\n  admin:\n    password_hash: \"sha256:bf6b5bdb74c79ece9fc0ad0ac9fb0359f9555d4f35a83b2e6ec69ae99e09603d\"\n    role: admin\n  operator:\n    password_hash: \"sha256:21cfc6b0fe8e257247937406f1ee83ae8acd3dc447c38b8431abecaf6d7ea437\"\n    role: operator\n",
        )
        .unwrap()
    }

    #[test]
    fn dev_credentials_detected_by_hash_membership() {
        let reg = dev_registry();
        let found = reg.dev_credentials_present();
        assert_eq!(found.len(), 2, "should hit admin + operator: {found:?}");
        assert!(found.contains(&"admin"));
        assert!(found.contains(&"operator"));
        // Non-dev registry -> empty
        assert!(test_registry().dev_credentials_present().is_empty());
        assert!(AccountRegistry::empty().dev_credentials_present().is_empty());
    }

    #[test]
    fn dev_credentials_refuse_startup_unless_explicitly_allowed() {
        let reg = dev_registry();
        let err = reg.check_dev_credentials(false).unwrap_err();
        assert!(
            err.contains("DEVELOPMENT CREDENTIALS DETECTED"),
            "startup must reject dev placeholder accounts: {err}"
        );
        assert!(err.contains("admin123"), "error must name the dev account: {err}");
        // Explicit override (dev compose env) -> allowed
        assert_eq!(reg.check_dev_credentials(true), Ok(()));
        // Non-dev / empty registry -> always allowed
        assert_eq!(test_registry().check_dev_credentials(false), Ok(()));
        assert_eq!(AccountRegistry::empty().check_dev_credentials(false), Ok(()));
    }

    // -- unified-device-admin T5: hot-reload primitives -------------------------------------

    #[test]
    fn create_account_then_authenticate_hot() {
        let reg = AccountRegistry::empty();
        reg.create_account("bob", "pw", "operator", &["ms-car2".to_string()]).unwrap();
        // login succeeds without a restart (hot-reload semantics)
        let id = reg.authenticate("bob", "pw").unwrap();
        assert_eq!(id.role, CockpitRole::Operator);
        assert_eq!(id.vehicles, vec!["ms-car2".to_string()]);
    }

    #[test]
    fn create_duplicate_errors() {
        let reg = test_registry();
        let err = reg.create_account("carol", "x", "viewer", &[]).unwrap_err();
        assert_eq!(err, AccountRegError::Duplicate);
    }

    #[test]
    fn create_invalid_role_rejected() {
        let reg = AccountRegistry::empty();
        let err = reg.create_account("eve", "pw", "superuser", &[]).unwrap_err();
        assert!(matches!(err, AccountRegError::InvalidRole(_)));
    }

    #[test]
    fn create_invalid_vehicles_rejected() {
        let reg = AccountRegistry::empty();
        let err = reg.create_account("eve", "pw", "viewer", &["bad id!".to_string()]).unwrap_err();
        assert!(matches!(err, AccountRegError::InvalidVehicles(_)));
    }

    #[test]
    fn update_role_and_password_hot() {
        let reg = test_registry();
        reg.update_account("carol", Some("admin"), None, Some("new-pw")).unwrap();
        assert_eq!(reg.authenticate("carol", "s3cret").unwrap_err(), AccountAuthError::BadPassword);
        let id = reg.authenticate("carol", "new-pw").unwrap();
        assert_eq!(id.role, CockpitRole::Admin);
    }

    #[test]
    fn delete_account_makes_login_fail() {
        let reg = test_registry();
        reg.delete_account("carol").unwrap();
        assert_eq!(reg.authenticate("carol", "s3cret").unwrap_err(), AccountAuthError::Unknown);
    }

    #[test]
    fn delete_unknown_errors() {
        let reg = test_registry();
        assert_eq!(reg.delete_account("nobody"), Err(AccountRegError::Unknown));
    }

    #[test]
    fn list_accounts_excludes_password_hash() {
        let reg = test_registry();
        let views = reg.list_accounts();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].username, "carol");
        assert_eq!(views[0].role, "operator");
        assert_eq!(views[0].vehicles, vec!["ms-car1".to_string()]);
    }

    #[test]
    fn save_roundtrip_preserves_accounts() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ms-accounts-test-{}.yaml", uuid::Uuid::new_v4()));
        let reg = AccountRegistry::empty();
        reg.create_account("bob", "pw", "dispatcher", &[]).unwrap();
        reg.save(&path).unwrap();
        assert!(!path.with_extension("yaml.tmp").exists(), "temp must be cleaned up");
        let reloaded = AccountRegistry::load(&path).unwrap();
        let id = reloaded.authenticate("bob", "pw").unwrap();
        assert_eq!(id.role, CockpitRole::Dispatcher);
        let _ = std::fs::remove_file(&path);
    }
}
