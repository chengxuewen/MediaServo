//! G3 舱端账号注册表（D-H11 双类身份之操作员侧）— 登录认证 + token 签发。
//!
//! 文件型配置（YAML，与 devices.yaml 同构），格式:
//! ```yaml
//! accounts:
//!   carol:                       # 用户名（JWT sub）
//!     password_hash: "sha256:<hex>"   # sha256(username + ":" + password)
//!     role: operator             # viewer|operator|admin|dispatcher
//!     vehicles: ["ms-car1"]      # 车×舱白名单（admin/dispatcher 可省略）
//! ```
//! 存储决策同 G2 devices: 仅单向哈希（username 充当盐）；subtle 常量时间比较；
//! 未知用户与错误密码 wire 响应逐字一致（防枚举）。升级路径同 G2: argon2id 前缀。
//! token 设计（G3 采用 D-H11 选项②）: 登录成功签发 JWT
//! `{sub: username, role, vehicles, iat, exp}`，与 admin JWT 同 secret（admin_jwt_secret）
//! 同算法（HS256）— 复用既有 JwtAuth/中间件机制，无第二套签名体系。

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

/// 账号认证失败原因（wire 统一 401，内部区分仅进审计日志 — 防枚举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAuthError {
    /// 用户不在注册表中。
    Unknown,
    /// 密码哈希不匹配。
    BadPassword,
}

impl AccountAuthError {
    /// 面向客户端的可读消息 — 未知用户与错误密码必须逐字一致（防枚举，同 G2）。
    pub fn message(&self) -> &'static str {
        match self {
            AccountAuthError::Unknown | AccountAuthError::BadPassword => {
                "account authentication failed: invalid credentials"
            }
        }
    }
}

/// 账号注册表管理操作错误（管理 API 用；400/404/409 映射见 admin.rs）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountRegError {
    /// 创建时用户名已存在。
    Duplicate,
    /// 更新/删除时用户名不存在。
    Unknown,
    /// 角色不是 viewer|operator|admin|dispatcher。
    InvalidRole(String),
    /// vehicles 含空项或非法字符。
    InvalidVehicles(String),
}

/// 管理列表视图（不含 password_hash — 密码哈希绝不外泄）。
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

/// 舱端账号注册表（启动时加载，只读；空 = 无账号，PSK/设备路径不受影响）。
/// 注册表内部态（RwLock 包裹）。
#[derive(Debug)]
struct AccountRegistryInner {
    accounts: HashMap<String, AccountEntry>,
}

impl AccountRegistryInner {
    fn new(accounts: HashMap<String, AccountEntry>) -> Self {
        Self { accounts }
    }
}

/// 舱端账号注册表（启动时加载；运行期可读写热重载）。
/// 空 = 无账号，PSK/设备路径不受影响。
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

    /// 锁 poison 恢复标准做法：unpoisoned 读锁；poison 时取回写者遗留的一致值。
    fn lock_read(&self) -> RwLockReadGuard<'_, AccountRegistryInner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_write(&self) -> std::sync::RwLockWriteGuard<'_, AccountRegistryInner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 从 YAML 文件加载。
    ///
    /// 文件**缺失** → 空注册表（`Ok` — PSK/设备路径不受影响，I4 review 维持）；
    /// 文件**存在但损坏** → `Err`（fail-fast: 启动方必须拒绝继续——损坏的注册表
    /// 静默降级 = 授权强制被静默禁用，与 identity.json 损坏显式报错同纪律）。
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

    /// 从 YAML 文本解析（测试与加载共用）。未知角色 → 解析错误（账号不可用）。
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

    /// 认证决策（login 处理调用）: 校验通过返回账号身份（进 JWT claims）。
    fn verify(&self, username: &str, password: &str) -> Result<AccountIdentity, AccountAuthError> {
        let inner = self.lock_read();
        let known = inner.accounts.contains_key(username);
        // 未知用户也走完整 sha256 + ct_eq 路径（防时序，同 G2 dummy 机制 —
        // 此处以固定字符串为目标，长度与真实哈希一致）。
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

    /// 登录入口（admin.rs login handler 调用）。
    pub fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AccountIdentity, AccountAuthError> {
        self.verify(username, password)
    }

    /// I2 review: 检测注册表是否含已知开发占位哈希（命中用户名列表）。
    pub fn dev_credentials_present(&self) -> Vec<&'static str> {
        let inner = self.lock_read();
        DEV_CREDENTIAL_HASHES
            .iter()
            .filter(|(_, hash)| inner.accounts.values().any(|e| e.password_hash == *hash))
            .map(|(user, _)| *user)
            .collect()
    }

    /// I2 review: 启动守卫 — 含开发占位账号 → Err（fail-fast，同损坏注册表纪律）；
    /// `allow_dev` = 显式覆盖（env MEDIASERVO_ALLOW_DEV_CREDENTIALS=1，仅 dev compose 设置）。
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

    /// 序列化为 YAML 文件内容（与 from_yaml 格式互逆）。
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

    /// Atomic 写回 accounts.yaml（temp + fsync + rename）。
    /// 内存不变：序列化与写盘全部成功才返回 Ok；失败清理 temp 并返回 Err。
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

    /// vehicles 校验：非空列表项、每项非空且仅 [A-Za-z0-9-_]（与 device_id 同字符集）。
    fn validate_vehicles(vehicles: &[String]) -> Result<(), AccountRegError> {
        if vehicles.iter().any(|v| {
            v.is_empty()
                || v.len() > 64
                || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }) {
            return Err(AccountRegError::InvalidVehicles(
                "vehicles: 每项 1-64 字符 [A-Za-z0-9-_]".into(),
            ));
        }
        Ok(())
    }

    /// 创建账号：hash_password + 插入（内存）。落盘由管理 API 层调 `save` 完成。
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

    /// 更新账号（role/vehicles/密码按需）：None 字段保持原值。
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

    /// 删除账号。
    pub fn delete_account(&self, username: &str) -> Result<(), AccountRegError> {
        let mut inner = self.lock_write();
        inner.accounts.remove(username).map(|_| ()).ok_or(AccountRegError::Unknown)
    }

    /// 管理列表（不含 password_hash）。
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

/// I2 review: 已知开发占位账号哈希（config/accounts.docker.yaml 的 admin123/
/// dispatch123/operator123）— 哈希即 sha256(username:password)，含 username 盐。
/// 生产部署启动守卫（check_dev_credentials）拒绝；dev compose 经
/// MEDIASERVO_ALLOW_DEV_CREDENTIALS=1 显式豁免。
pub const DEV_CREDENTIAL_HASHES: &[(&str, &str)] = &[
    ("admin", "sha256:bf6b5bdb74c79ece9fc0ad0ac9fb0359f9555d4f35a83b2e6ec69ae99e09603d"),
    ("dispatcher", "sha256:51f00e625e5fb3aff1c5a55eff96d1f2f03273afd8b0bc2514961e33dd82f8b2"),
    ("operator", "sha256:21cfc6b0fe8e257247937406f1ee83ae8acd3dc447c38b8431abecaf6d7ea437"),
];

/// sha256(username + ":" + password)，hex 编码，`sha256:` 前缀（username 充当盐）。
pub fn hash_password(username: &str, password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(username.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// 为账号身份签发 JWT（HS256，admin_jwt_secret — 与 admin 中间件同 secret 同算法）。
/// claims: {sub: username, role, vehicles, iat, exp}。
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

/// 启动期密钥配对校验（I2 review — fail-open 修复）:
/// 账号 token 由 login 用 `admin_jwt_secret` 签发，但 /ws 握手用 `jwt_secret` 验签 —
/// 两者不一致时账号 token 验签失败 → 静默回退 PSK/Legacy → 矩阵强制被绕过。
/// 有账号配置时必须一致（不相等 → Err，启动方 fail-fast）。
pub fn validate_secret_pairing(
    accounts_configured: bool,
    jwt_secret: Option<&str>,
    admin_jwt_secret: Option<&str>,
) -> Result<(), String> {
    if !accounts_configured {
        return Ok(()); // 无账号 → 无矩阵可绕过（PSK/设备路径不受影响）
    }
    match (jwt_secret, admin_jwt_secret) {
        (Some(a), Some(b)) if a != b => Err(format!(
            "jwt_secret 与 admin_jwt_secret 不一致: 账号 token 经 admin_jwt_secret 签发、             /ws 握手经 jwt_secret 验签，不一致会使账号认证静默失败并回退 PSK（矩阵绕过）。             请配置为同一 secret"
        )),
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
        assert_ne!(e_unknown, e_bad, "内部区分保留（审计）");
        assert_eq!(e_unknown.message(), e_bad.message(), "wire 必须逐字一致（防枚举）");
        assert!(e_unknown.message().starts_with("account authentication failed"));
        // 空注册表: 任何用户都失败
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
        // 稳定向量
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
        // I2 review: 账号配置 + 密钥分歧 → Err（fail-fast）; 一致/无账号 → Ok
        assert!(
            validate_secret_pairing(true, Some("aaa"), Some("bbb")).is_err(),
            "有账号 + 密钥分歧必须报错"
        );
        let err = validate_secret_pairing(true, Some("aaa"), Some("bbb")).unwrap_err();
        assert!(err.contains("jwt_secret") && err.contains("admin_jwt_secret"), "{err}");
        assert_eq!(validate_secret_pairing(true, Some("aaa"), Some("aaa")), Ok(()));
        // 无账号 → 分歧无害（无矩阵可绕过）; 单边缺失 → 无法分歧
        assert_eq!(validate_secret_pairing(false, Some("aaa"), Some("bbb")), Ok(()));
        assert_eq!(validate_secret_pairing(true, None, Some("bbb")), Ok(()));
        assert_eq!(validate_secret_pairing(true, Some("aaa"), None), Ok(()));
        assert_eq!(validate_secret_pairing(true, None, None), Ok(()));
    }

    #[test]
    fn load_missing_file_ok_empty_and_malformed_err() {
        // I4 review: 缺失 → 空（不阻断）; 损坏 → Err（fail-fast，禁静默降级）
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.yaml");
        assert!(AccountRegistry::load(&missing).unwrap().is_empty(), "缺失=空注册表");

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
        // vehicles 缺省空
        assert!(reg.authenticate("adm", "p").unwrap().vehicles.is_empty());
    }

    // ── I2 review: 开发占位账号守卫 ──────────────────────────────────────────

    /// 含 dev 占位哈希的注册表（accounts.docker.yaml 同构）
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
        assert_eq!(found.len(), 2, "应命中 admin + operator: {found:?}");
        assert!(found.contains(&"admin"));
        assert!(found.contains(&"operator"));
        // 非 dev 注册表 → 空
        assert!(test_registry().dev_credentials_present().is_empty());
        assert!(AccountRegistry::empty().dev_credentials_present().is_empty());
    }

    #[test]
    fn dev_credentials_refuse_startup_unless_explicitly_allowed() {
        let reg = dev_registry();
        let err = reg.check_dev_credentials(false).unwrap_err();
        assert!(
            err.contains("DEVELOPMENT CREDENTIALS DETECTED"),
            "启动必须拒绝开发占位账号: {err}"
        );
        assert!(err.contains("admin123"), "错误应点名 dev 账号: {err}");
        // 显式覆盖（dev compose env）→ 放行
        assert_eq!(reg.check_dev_credentials(true), Ok(()));
        // 非 dev / 空注册表 → 恒放行
        assert_eq!(test_registry().check_dev_credentials(false), Ok(()));
        assert_eq!(AccountRegistry::empty().check_dev_credentials(false), Ok(()));
    }

    // ── unified-device-admin T5: 热重载原语 ────────────────────────────────

    #[test]
    fn create_account_then_authenticate_hot() {
        let reg = AccountRegistry::empty();
        reg.create_account("bob", "pw", "operator", &["ms-car2".to_string()]).unwrap();
        // 无需重启即登录成功（热重载语义）
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
        assert!(!path.with_extension("yaml.tmp").exists(), "temp 必须清理");
        let reloaded = AccountRegistry::load(&path).unwrap();
        let id = reloaded.authenticate("bob", "pw").unwrap();
        assert_eq!(id.role, CockpitRole::Dispatcher);
        let _ = std::fs::remove_file(&path);
    }
}
