//! API-key 注册表 — 免账号客户端凭证（accountless-client-auth T1）。
//!
//! 形态 = LiveKit apikey/apiSecret 同型：`POST /api/auth/exchange {key_id, secret}`
//! → 验 key → 以 `admin_jwt_secret` 现签短 JWT `{sub:"apikey:<id>", role, vehicles, exp}`
//! → 走 `/ws` 既有握手门（验签/角色/矩阵零改动，见 signaling.rs）。
//!
//! 与 [devices] 的纪律同构（有意复用其模式而非新发明）：
//! - YAML 单一事实源 + `RwLock` 热生效 + atomic save（temp+fsync+rename，失败内存不变）；
//! - 仅存 `sha256(key_id + ":" + secret)`（复用 [`crate::devices::hash_secret`]，key_id 为盐）；
//! - Unknown 与 BadSecret **逐字同消息**（防枚举，review #1 同族）；dummy 哈希垫时间等长；
//! - secret 明文仅 register 响应一次（C33 语义），丢失=吊销重发，无找回路径。
//!
//! 吊销粒度（F10）：删 key 只挡后续 exchange；已发 JWT 至 exp 自然死（窗口=TTL）。
//! `sub` 命名空间前缀 `apikey:` 与账号用户名隔离（F5，审计可归因）。

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

/// key_id 词法（防 YAML 注入/路径歧义，同 validate_device_id 纪律）。
fn valid_key_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// key_id 为**键**（非用户名语义），secret 仅哈希落盘。vehicles 缺省空
/// （viewer/operator 生效；admin/dispatcher 忽略=任意车 — 同 G3 矩阵）。
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
    /// 未知 key 时间补偿目标（随机 id 盐，长度与真实哈希恒等 — devices.rs review #1 同形）。
    dummy_hash: [u8; 32],
}

pub struct ApiKeyRegistry {
    inner: RwLock<Inner>,
}

/// register/revoke 错误（管理面 400/409/404 映射）。
#[derive(Debug, PartialEq, Eq)]
pub enum ApiKeyRegError {
    Duplicate,
    Unknown,
    Invalid(String),
}

/// exchange 认证失败（对外逐字同消息，防枚举）。
#[derive(Debug, PartialEq, Eq)]
pub enum ApiKeyAuthError {
    Unknown,
    BadSecret,
}

impl ApiKeyAuthError {
    #[must_use]
    pub fn message(&self) -> &'static str {
        // Unknown 与 BadSecret 必须逐字一致（集成测试钉）——exchange 响应不含区分信息。
        "api key authentication failed: invalid credentials"
    }
}

impl ApiKeyRegistry {
    pub fn empty() -> Self {
        Self {
            inner: RwLock::new(Inner { keys: HashMap::new(), dummy_hash: new_dummy_hash() }),
        }
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
            let role = CockpitRole::parse(&fe.role)
                .ok_or_else(|| CoreError::ConfigParse(format!(
                    "api_keys[{id}]: role {:?} not in viewer|operator|admin|dispatcher", fe.role
                )))?;
            let hash = decode_hash(&fe.secret_hash).ok_or_else(|| {
                CoreError::ConfigParse(format!("api_keys[{id}]: bad secret_hash"))
            })?;
            keys.insert(id, ApiKeyEntry {
                secret_hash: hash,
                role,
                vehicles: fe.vehicles.unwrap_or_default(),
                label: fe.label,
            });
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
                                e.secret_hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
                            ),
                            role: e.role.as_str().to_string(),
                            vehicles: if e.vehicles.is_empty() { None } else { Some(e.vehicles.clone()) },
                            label: e.label.clone(),
                        },
                    )
                })
                .collect(),
        };
        serde_yaml::to_string(&file)
            .map_err(|e| CoreError::ConfigParse(format!("api_keys serialize: {e}")))
    }

    /// Atomic 写回 api_keys.yaml（temp + fsync + rename；失败内存不变 — devices.rs 同纪律）。
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

    /// 注册：服务器生成 secret（uuid v4 36 字符，同 devices 熵源），明文**仅此一次**返回。
    /// role 闭集校验（非法 → Invalid，deploy 期拦，不静默存矩阵外角色）。
    pub fn register(
        &self,
        key_id: &str,
        role: &str,
        vehicles: &[String],
        label: Option<&str>,
    ) -> Result<String, ApiKeyRegError> {
        if !valid_key_id(key_id) {
            return Err(ApiKeyRegError::Invalid(
                "key_id: 1-64 字符 [A-Za-z0-9-_]".into(),
            ));
        }
        let role = CockpitRole::parse(role)
            .ok_or_else(|| ApiKeyRegError::Invalid(
                "role: viewer|operator|admin|dispatcher".into(),
            ))?;
        let secret = uuid::Uuid::new_v4().to_string();
        let hash = decode_hash(&hash_secret(key_id, &secret))
            .expect("hash_secret 输出恒可解码");
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

    /// 列表（管理面；secret_hash 绝不出内存——map 成明文安全形）。
    #[must_use]
    pub fn list(&self) -> Vec<(String, String, Vec<String>, Option<String>)> {
        let inner = self.lock_read();
        let mut out: Vec<_> = inner
            .keys
            .iter()
            .map(|(id, e)| (id.clone(), e.role.as_str().to_string(), e.vehicles.clone(), e.label.clone()))
            .collect();
        out.sort();
        out
    }

    /// exchange 认证决策点：secret 恒定时间比对；未知 key 走 dummy 等长比对（时间不泄露存在性）。
    pub fn verify(&self, key_id: &str, secret: &str) -> Result<ApiKeyEntry, ApiKeyAuthError> {
        let inner = self.lock_read();
        // hash_secret 输出即 "sha256:<hex>"；decode 失败（理论不可达）= 全 0 垫底永不匹配。
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
                let _ = inner.dummy_hash.ct_eq(&provided); // 时间补偿
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

/// exchange 用签发：与 accounts::issue_account_token 同签名基建（admin_jwt_secret），
/// 差异 = `sub` 加 `apikey:` 命名空间前缀（F5 审计归因 + 与用户名隔离）。
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

/// `sha256:<64 hex>` → 32B；形状不符 = None（load 期拒，运行期不猜）。
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

/// 启动时生成一次的 dummy 比较目标（随机 key_id 盐保证与任何在册 key 不冲突）。
fn new_dummy_hash() -> [u8; 32] {
    let h = hash_secret(&format!("apikey-dummy-{}", uuid::Uuid::new_v4()), "dummy");
    decode_hash(&h).expect("hash_secret 输出恒可解码")
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
        let reg = ApiKeyRegistry::from_yaml(&entry_yaml("cockpit-1", "s3cret", "viewer"))
            .expect("load");
        let yaml = reg.to_yaml().unwrap();
        let back = ApiKeyRegistry::from_yaml(&yaml).unwrap();
        assert_eq!(back.verify("cockpit-1", "s3cret").unwrap().role.as_str(), "viewer");
        // 盐= key_id：同 secret 不同 key → 不同 hash（devices 同钉）。
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
        assert!(matches!(reg.register("bad id!!", "viewer", &[], None), Err(ApiKeyRegError::Invalid(_))));
    }

    #[test]
    fn register_returns_secret_once_and_verify_passes() {
        let reg = ApiKeyRegistry::empty();
        let secret = reg.register("ci-1", "viewer", &["vehicle_a".into()], Some("ci 集成")).unwrap();
        assert!(matches!(reg.register("ci-1", "viewer", &[], None), Err(ApiKeyRegError::Duplicate)));
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
        assert_eq!(claims.sub, "apikey:k", "F5: sub 必须 apikey: 命名空间");
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
