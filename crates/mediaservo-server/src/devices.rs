//! G2 设备注册表 — server 侧设备凭证校验（D-H11 连接级身份）。
//!
//! 注册表为文件型配置（YAML，与 server.yaml 同构），格式：
//! ```yaml
//! devices:
//!   ms-0a1b2c3d4e5f:
//!     secret_hash: "sha256:<hex>"   # sha256(device_id + ":" + device_secret) — legacy secret 形
//!   ms-c3d4e5f6a7b8:
//!     public_key: "ed25519:<b64>"   # device-enroll 公钥形（base64 32B Ed25519 vk）
//!     name: "jetson-7"              # 可选显示名
//! ```
//! device-enroll（design §5.1）: 条目双形共存（D-E3 一周期），读哪个走哪条验证，save 回写保形。
//! 存储决策（G2）: 客户端经 TLS 在 wire 上明文携带 secret，注册表仅存单向哈希；
//! `sha256(device_id + ":" + device_secret)` — device_id 充当每设备盐（无需额外存储）。
//! 升级路径（H 阶段）: argon2id 替换 sha256，格式前缀 `argon2:<encoded>`。
//! 配发流程（G2 文档）: `host init` 生成 identity.json → 运维把 device_id/secret
//! 拷入 server 的 devices.yaml（`ms-field hash` 之类工具 H 阶段提供；当前用
//! `sha256sum` 手工算或本模块测试向量）。
//!
//! 热重载（unified-device-admin）: 注册表内部 `RwLock<Inner>` 化 — 外部签名
//! （`Arc<DeviceRegistry>` / `&DeviceRegistry`）不变，signaling 鉴权调用点零改动；
//! 管理操作（register/revoke/reset/list/save）运行时生效，无需重启 server。
//! 写回策略：磁盘为单一事实源 — `save` 先序列化（短临界区）后 atomic 写盘
//! （temp + fsync + rename），失败返回 Err 且内存不变。

use base64::Engine as _;
use mediaservo_common::error::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, RwLock, RwLockReadGuard};
use subtle::ConstantTimeEq;

/// 设备认证失败原因（错误码统一 4010，见 signaling.rs 认证点注释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAuthError {
    /// device_id/device_secret 恰好只带了一个（形状检查，G4 review Minor 1）。
    Incomplete,
    /// device_id 不在注册表中。
    Unknown,
    /// secret 哈希不匹配。
    BadSecret,
}

impl DeviceAuthError {
    /// 面向客户端的可读消息（C15: 错误响应必须信息充分）。
    /// 防枚举（review #1）: 未知设备与错误 secret 必须返回**逐字一致**的消息 —
    /// 区分会泄漏注册表成员资格。内部区分（Unknown/BadSecret）仅保留在审计日志。
    /// 4010 单一错误码（signaling.rs 认证点常量）+ 此单一消息。
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

/// 注册表管理操作错误（管理 API 用；400/404/409 映射见 admin.rs）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceRegError {
    /// 注册时 device_id 已存在。
    Duplicate,
    /// 吊销/重置时 device_id 不存在。
    Unknown,
    /// 管理员提供的 secret 不合规（非空、8-128 字符、无空白）。
    InvalidSecret(String),
    /// enroll 收录的 public_key 不合规（非 base64(32B) 规范 Ed25519 vk）。
    InvalidPublicKey(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct RegistryFile {
    #[serde(default)]
    devices: HashMap<String, DeviceEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DeviceEntry {
    /// legacy secret 形（D-E3 共存一周期）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_hash: Option<String>,
    /// 公钥形: "ed25519:<base64 32B vk>"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_key: Option<String>,
    /// 公钥形可选显示名（auto 收录为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// 注册表条目双形（device-enroll design §5.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// secret 形: "sha256:<hex>"。
    Secret(String),
    /// 公钥形: vk = base64(32B Ed25519 verifying key)。
    PublicKey { vk: String, name: Option<String> },
}

/// 注册表内部态（RwLock 包裹；方法均为内部分发）。
#[derive(Debug)]
struct RegistryInner {
    devices: HashMap<String, Entry>, // device_id → Secret("sha256:<hex>") | PublicKey{vk,name}
    /// 未知设备的固定比较目标（启动时随机生成, 与真实哈希同长, 永不匹配）。
    /// review #1: 未知设备也必须走完整 sha256 + ct_eq 路径, 响应时间不可区分。
    dummy_hash: String,
}

impl RegistryInner {
    fn new(devices: HashMap<String, Entry>) -> Self {
        Self { devices, dummy_hash: new_dummy_hash() }
    }

    fn verify(&self, device_id: &str, secret: &str) -> Result<(), DeviceAuthError> {
        let known = self.devices.contains_key(device_id);
        // review #1 防时序: 未知设备也用 dummy_hash 走完整 sha256 + ct_eq（无提前返回）。
        // 已知/未知的响应时间不可区分; 匹配与否经 same-length ct_eq 判定。
        // 公钥形条目对 secret 比较走 dummy（与未知设备同路径——不泄漏条目形态，review #1 纪律）。
        let stored: &String = match self.devices.get(device_id) {
            Some(Entry::Secret(h)) => h,
            _ => &self.dummy_hash,
        };
        let want = hash_secret(device_id, secret);
        let matched: bool = stored.as_bytes().ct_eq(want.as_bytes()).into();
        match (matched, known) {
            (true, _) => Ok(()),
            // 内部区分保留（审计用）; 对外 wire 响应两者完全一致（见 message()）。
            (false, true) => Err(DeviceAuthError::BadSecret),
            (false, false) => Err(DeviceAuthError::Unknown),
        }
    }
}

/// 设备注册表（启动时加载；运行期可读热重载，管理操作经 RwLock 生效）。
/// 注意: 不实现 Default — dummy_hash 必须启动时随机生成（Default 会给空串,
/// 长度与真实哈希不同 → 未知设备比较路径的时序与已知设备可区分, 重开侧信道）。
/// 注意: 不再 derive Clone（std RwLock 非 Clone）— 共享一律走 Arc，使用点已确认无克隆。
#[derive(Debug)]
pub struct DeviceRegistry {
    inner: RwLock<RegistryInner>,
}

impl DeviceRegistry {
    pub fn empty() -> Self {
        Self { inner: RwLock::new(RegistryInner::new(HashMap::new())) }
    }

    /// 从 YAML 文件加载；文件缺失视为空注册表（PSK 路径不受影响）。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            CoreError::ConfigParse(format!("devices file {}: {e}", path.as_ref().display()))
        })?;
        Self::from_yaml(&content).map_err(|e| {
            CoreError::ConfigParse(format!("devices file {}: {e}", path.as_ref().display()))
        })
    }

    /// 从 YAML 文本解析（测试与加载共用）。
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
                    return Err(format!(
                        "device {id}: entry must carry secret_hash or public_key"
                    ));
                }
            };
            devices.insert(id, parsed);
        }
        Ok(Self { inner: RwLock::new(RegistryInner::new(devices)) })
    }

    /// 锁 poison 恢复标准做法：unpoisoned 读锁；poison 时取回写者遗留的一致值。
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

    /// registry 内全部 device_id（管理列表用）。
    pub fn device_ids(&self) -> Vec<String> {
        self.lock_read().devices.keys().cloned().collect()
    }

    fn verify(&self, device_id: &str, secret: &str) -> Result<(), DeviceAuthError> {
        self.lock_read().verify(device_id, secret)
    }

    /// 序列化为 YAML 文件内容（与 from_yaml 格式互逆，round-trip 稳定）。
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

    /// Atomic 写回 devices.yaml（temp + fsync + rename）。
    /// **内存不变**：仅在序列化与写盘全部成功后返回 Ok；失败清理 temp 并返回 Err。
    /// 调用方（管理 API 层）持有此函数的调用权 — 写路径低频、短临界区。
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
                let _ = std::fs::remove_file(&tmp); // 清理残留（成功路径无 tmp）
                Err(CoreError::ConfigParse(format!(
                    "devices file {}: write failed: {e}",
                    path.display()
                )))
            }
        }
    }
    /// sha256(device_id + ":" + device_secret)，hex 编码，`sha256:` 前缀。

    /// 注册新设备：默认生成随机 secret。
    /// 返回 `(secret_hash, secret)` — secret 是**唯一一次明文**，调用方负责传递；
    /// 后续仅可经 reset_secret 更换。落盘由管理 API 层调 `save` 完成。
    pub fn register(&self, device_id: &str) -> Result<(String, String), DeviceRegError> {
        self.register_with_secret(device_id, None)
    }

    /// 注册新设备：`secret` 为 `Some` 时使用管理员提供的 secret（配发流程可先配置
    /// host 再注册，secret 全程管理员掌控、不丢失 — 方案 A）; `None` 则服务器生成。
    /// 校验：非空、8-128 字符、无空白（质量由管理员负责，仅拦明显错误）。
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
                        "secret: 8-128 字符且不含空白".into(),
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

    /// 吊销设备：从注册表移除（内存）。下次接入鉴权即 Unknown → 4010。
    /// 存量在线连接不受影响（鉴权仅发生在接入时）— 运营语义见 proposal。
    pub fn revoke(&self, device_id: &str) -> Result<(), DeviceRegError> {
        let mut inner = self.lock_write();
        inner.devices.remove(device_id).map(|_| ()).ok_or(DeviceRegError::Unknown)
    }

    /// 重置设备 secret：旧 secret 立即失效，返回新 secret（唯一一次明文）。
    pub fn reset_secret(&self, device_id: &str) -> Result<(String, String), DeviceRegError> {
        let mut inner = self.lock_write();
        // 公钥形条目无 secret 可重置 → 按未注册（404）处理；迁移处置 = web 删除重录（§8）。
        // 不拦截则 reset 会把公钥形静默改写回 secret 形（准入语义被旁路）。
        if !matches!(inner.devices.get(device_id), Some(Entry::Secret(_))) {
            return Err(DeviceRegError::Unknown);
        }
        let secret = new_secret();
        let hash = hash_secret(device_id, &secret);
        inner.devices.insert(device_id.to_string(), Entry::Secret(hash.clone()));
        Ok((hash, secret))
    }

    // ── device-enroll: 公钥准入面（design §5.1）─────────────────────────────

    /// 条目查询（signaling 状态机分形决策 / admin 断言用）。
    pub fn entry_of(&self, device_id: &str) -> Option<Entry> {
        self.lock_read().devices.get(device_id).cloned()
    }

    /// 公钥形设备验签（§5.3，登记 vk 为准）。未知 → Unknown；secret 形 → BadSecret
    /// （signaling 在发挑战前已拒 secret 形接入，此处防御性兜底）。
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

    /// 验签通过后收录（enroll_auto = 手动档 admin approve 与 auto 档共用入口）：入
    /// registry public_key 形（name 可缺省）。落盘由调用方 save（同一 Arc = C33 热生效链）。
    /// vk 来自 wire → 边界校验（base64 32B 规范 key）。
    pub fn enroll_auto(
        &self,
        device_id: &str,
        vk_b64: &str,
        name: Option<&str>,
    ) -> Result<(), DeviceRegError> {
        decode_vk(vk_b64)
            .map_err(|e| DeviceRegError::InvalidPublicKey(format!("public_key 不合规: {e}")))?;
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

/// 生成新设备 secret：uuid v4（122-bit CSPRNG 熵）36 字符 — 与 G2 哈希格式兼容，
/// 无需新增依赖（design: uuid 兜底方案；H 阶段如需更强熵换 getrandom + 32B hex）。
fn new_secret() -> String {
    uuid::Uuid::new_v4().to_string()
}
/// sha256(device_id + ":" + device_secret)，hex 编码，`sha256:` 前缀。
/// device_id 充当每设备盐 — 无需额外 salt 存储（G2 存储决策，文档见模块头）。
pub fn hash_secret(device_id: &str, secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(device_id.as_bytes());
    hasher.update(b":");
    hasher.update(secret.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// 启动时生成一次的 dummy 比较目标（review #1）: 随机 device_id 保证与任何
/// 注册设备不冲突（uuid v4），长度与真实哈希一致（71 字符）保证 ct_eq 路径恒等。
fn new_dummy_hash() -> String {
    hash_secret(&format!("ms-dummy-{}", uuid::Uuid::new_v4()), "dummy")
}

/// base64 → 32B Ed25519 vk 解码校验（wire 输入边界；design §3: base64 standard 无换行）。
fn decode_vk(vk_b64: &str) -> Result<[u8; 32], &'static str> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(vk_b64)
        .map_err(|_| "not valid base64")?;
    let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| "decoded length != 32 bytes")?;
    ed25519_dalek::VerifyingKey::from_bytes(&arr).map_err(|_| "invalid ed25519 public key")?;
    Ok(arr)
}

/// device-enroll §3 密码合同: sig = Ed25519( nonce_raw(32B) ‖ device_id ‖ room_id )，
/// verify_strict 防签名可延展。登记/陌生共用同一函数（§5.3）。
/// 一切失败映射 BadSecret（wire 统一消息——防枚举纪律同 review #1，区分只在调用方日志）。
pub fn verify_signature(
    vk_b64: &str,
    nonce_b64: &str,
    device_id: &str,
    room_id: &str,
    sig_b64: &str,
) -> Result<(), DeviceAuthError> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let vk_arr = decode_vk(vk_b64).map_err(|_| DeviceAuthError::BadSecret)?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&vk_arr)
        .map_err(|_| DeviceAuthError::BadSecret)?;
    let mut msg = b64.decode(nonce_b64).map_err(|_| DeviceAuthError::BadSecret)?;
    if msg.len() != 32 {
        return Err(DeviceAuthError::BadSecret);
    }
    let sig_bytes = b64.decode(sig_b64).map_err(|_| DeviceAuthError::BadSecret)?;
    let sig_arr: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| DeviceAuthError::BadSecret)?;
    msg.extend_from_slice(device_id.as_bytes());
    msg.extend_from_slice(room_id.as_bytes());
    vk.verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&sig_arr))
        .map_err(|_| DeviceAuthError::BadSecret)
}

/// 待批准条目（design §5.1；D-E5 内存不落盘——重启即清、设备重连自然重报）。
#[derive(Debug, Clone)]
pub struct PendingEntry {
    pub vk: String,
    /// 首见时刻（unix ms）。
    pub first_seen_ms: u64,
    /// 验签是否已过（当前唯一写入点在验签通过后 → 恒 true；字段保留 pending 语义面）。
    pub verified: bool,
}

/// 手动档待批准表：陌生 pubkey 验签过、未 admin approve 期间的等待队列。
/// 同 device_id 换 vk 重报 → 覆盖 + WARN（design §5.1）。
#[derive(Debug, Default, Clone)]
pub struct PendingTable {
    inner: Arc<dashmap::DashMap<String, PendingEntry>>,
}

impl PendingTable {
    /// 登记/覆盖（verified 由调用方裁决——当前唯一调用点在验签通过后传 true）。
    pub fn insert(&self, device_id: &str, vk: &str, verified: bool) {
        let entry = PendingEntry { vk: vk.to_string(), first_seen_ms: now_ms(), verified };
        let prev = self.inner.insert(device_id.to_string(), entry);
        if let Some(prev) = prev
            && prev.vk != vk
        {
            tracing::warn!("pending device {device_id} 换钥重报（覆盖）: {} → {}", prev.vk, vk);
        }
    }

    pub fn get(&self, device_id: &str) -> Option<PendingEntry> {
        self.inner.get(device_id).map(|e| e.clone())
    }

    pub fn remove(&self, device_id: &str) -> Option<PendingEntry> {
        self.inner.remove(device_id).map(|(_, v)| v)
    }

    /// 全部待批准（admin 列表；按 device_id 稳定排序）。
    pub fn list(&self) -> Vec<(String, PendingEntry)> {
        let mut out: Vec<(String, PendingEntry)> = self
            .inner
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
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

/// 设备认证决策点（RoomJoin 处理调用；纯函数便于单测）。
///
/// 返回 `None` = 未携带任何设备凭证 → PSK 路径（保持原流程）。
/// `Some(Err)` = 形状不完整或凭证校验失败 → Error 4010（见 `DeviceAuthError::message`）。
/// `Some(Ok)` = 设备认证通过 → 连接级身份绑定（peer_id → device_id，D-H11）。
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
        assert_ne!(a, b, "device_id 必须参与哈希（盐）");
        assert!(a.starts_with("sha256:") && a.len() == "sha256:".len() + 64);
        // 稳定向量: sha256("ms-a:same-secret")
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
        // 双缺 = PSK 路径（None）
        assert_eq!(authenticate(&reg, None, None), None);
        // 半带 = Incomplete
        assert_eq!(
            authenticate(&reg, Some("ms-0a1b2c3d4e5f"), None),
            Some(Err(DeviceAuthError::Incomplete))
        );
        assert_eq!(
            authenticate(&reg, None, Some("s3cret")),
            Some(Err(DeviceAuthError::Incomplete))
        );
        // 全带 = 校验
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
        // C15: 消息可读；4010 单一错误码（signaling.rs 常量）+ 单一消息。
        assert!(DeviceAuthError::Incomplete.message().contains("both device_id"));
        assert!(DeviceAuthError::Incomplete.message().contains("device authentication failed"));
        // review #1: 未知设备与错误 secret 的 wire 消息必须逐字一致（防枚举）。
        assert_eq!(DeviceAuthError::Unknown.message(), DeviceAuthError::BadSecret.message());
        assert!(DeviceAuthError::Unknown.message().contains("invalid device credentials"));
    }

    #[test]
    fn unknown_vs_bad_secret_wire_response_identical() {
        // review #1 TDD: 两种失败路径的完整 wire 响应（code=4010 + message）必须一致。
        // code 由 signaling.rs 认证点统一为 4010；此处锁定 message 层等价。
        let reg = test_registry();
        let e_unknown =
            authenticate(&reg, Some("ms-nope"), Some("x")).expect("creds present").unwrap_err();
        let e_bad = authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some("wrong"))
            .expect("creds present")
            .unwrap_err();
        assert_eq!(e_unknown, DeviceAuthError::Unknown);
        assert_eq!(e_bad, DeviceAuthError::BadSecret);
        // 内部错误类型不同（审计可区分）但 wire 消息相同 — 防枚举。
        assert_ne!(e_unknown, e_bad);
        assert_eq!(e_unknown.message(), e_bad.message());
        // 两路径都必须走"设备认证失败"家族消息（客户端按 4010+前缀识别）。
        assert!(e_unknown.message().starts_with("device authentication failed"));
    }

    #[test]
    fn dummy_hash_is_per_instance_random_and_same_length() {
        // review #1: dummy 每次启动生成、与真实哈希同长（ct_eq 路径恒等）。
        let a = DeviceRegistry::empty();
        let b = DeviceRegistry::empty();
        assert_ne!(dummy_hash_of(&a), dummy_hash_of(&b), "dummy 必须每实例随机");
        assert_eq!(dummy_hash_of(&a).len(), "sha256:".len() + 64, "dummy 与真实哈希同长");
        assert_eq!(dummy_hash_of(&a).len(), hash_secret("ms-x", "s").len());
        // 未知设备仍走 verify 全路径（返回 Unknown 但内部已完成 sha256+ct_eq）。
        assert_eq!(a.verify("ms-anyone", "x"), Err(DeviceAuthError::Unknown));
    }

    #[test]
    fn save_roundtrip_preserves_entries_and_no_temp_leftover() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ms-devices-test-{}.yaml", uuid::Uuid::new_v4()));
        let reg = test_registry();
        reg.save(&path).unwrap();
        // 无 temp 残留
        assert!(!path.with_extension("yaml.tmp").exists(), "temp 文件必须被清理");
        // reload round-trip 保持鉴权语义
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
        assert_eq!(secret.len(), 36, "uuid v4 36 字符");
        // 注册后立即 authenticate（无重启 = 热重载语义）
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
        assert_eq!(secret, "admin-chosen-secret-1", "自备 secret 原样返回");
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
                DeviceRegError::InvalidSecret("secret: 8-128 字符且不含空白".into()),
                "{bad:?}"
            );
        }
        // 合法边界：8 字符
        assert!(reg.register_with_secret("ms-ok8", Some("12345678")).is_ok());
        // 128 字符
        assert!(reg.register_with_secret("ms-ok128", Some(&"x".repeat(128))).is_ok());
    }

    #[test]
    fn register_with_provided_secret_duplicate_priority() {
        let reg = test_registry();
        // 已存在 → Duplicate 优先于校验
        assert_eq!(
            reg.register_with_secret("ms-0a1b2c3d4e5f", Some("bad-short")).unwrap_err(),
            DeviceRegError::Duplicate
        );
    }

    // ─── device-enroll T2: 签名字节合同（design §3）─────────────────────────────
    // sig = base64( Ed25519::sign( nonce_raw(32B) ‖ device_id ‖ room_id ) )；验签 verify_strict。
    // 本向量 = host/server 交叉复验锚：批3 host 侧（T6）以 seed=bytes(0..=31)、
    // nonce=bytes(0x40..=0x5f)、同 device_id/room_id 复现同一签名。

    #[test]
    fn sig_vector_binds_nonce_device_room_and_verifies_strict() {
        use base64::Engine as _;
        use ed25519_dalek::Signer; // verify_strict = VerifyingKey 内建方法，无需 Verifier trait

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

        // 钉①: vk base64 = devices.yaml public_key 形指纹（与 protocol.rs pubkey 用例同值）
        assert_eq!(b64.encode(vk.to_bytes()), "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=");

        // 钉②: 字节合同签名钉死（host 批3 交叉复验同向量 = 两侧同 seed/nonce/ids 必出此值）
        let sig = signing.sign(&msg);
        assert_eq!(
            b64.encode(sig.to_bytes()),
            "Gnz2kGCFH6igsOfv5QW0+8aRyu/lP5ytAa8fJA0CPYP3fIX5UsYr6uTjFjqOEEFBUB2scnDffIZ1WfP9O2ECCg=="
        );

        // 钉③: 登记 vk 验签过（verify_strict —— §3 防签名 malleability 批注）
        vk.verify_strict(&msg, &sig).unwrap();

        // 钉④: 换 room_id 重放 = D-E7 跨房间绑定 → 拒
        let mut other_room = nonce.clone();
        other_room.extend_from_slice(device_id.as_bytes());
        other_room.extend_from_slice(b"vehicle_cam1");
        assert!(vk.verify_strict(&other_room, &sig).is_err());
    }

    // ─── device-enroll T3: 双形 registry / verify_pubkey / PendingTable / enroll_auto ──
    // 复用 sig_vector 常量（seed=bytes(0..=31) 的 vk/nonce/sig，批3 host 交叉复验同锚）。

    const TEST_VK: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";
    const TEST_SIG_CAM0: &str = "Gnz2kGCFH6igsOfv5QW0+8aRyu/lP5ytAa8fJA0CPYP3fIX5UsYr6uTjFjqOEEFBUB2scnDffIZ1WfP9O2ECCg==";

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
            text.contains("secret_hash") && text.contains("ed25519:") && text.contains("name: jetson-7"),
            "{text}"
        );
        // 回写保形: 双形不互窜（public_key 条目不得长出 secret_hash 行）
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
        // 换房间重放（D-E7 绑房）→ 拒
        assert_eq!(
            reg.verify_pubkey("ms-0a1b2c3d4e5f", &n, "vehicle_cam1", TEST_SIG_CAM0),
            Err(DeviceAuthError::BadSecret)
        );
        // 换 nonce（一次一用的跨连接重放面）→ 拒
        let other_nonce = base64::engine::general_purpose::STANDARD.encode([0xABu8; 32]);
        assert_eq!(
            reg.verify_pubkey("ms-0a1b2c3d4e5f", &other_nonce, "vehicle_cam0", TEST_SIG_CAM0),
            Err(DeviceAuthError::BadSecret)
        );
        // 未登记 → Unknown；secret 形条目 → BadSecret（wire 消息同家族，防枚举）
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
        // 陌生设备: 验签用其上报 vk（enroll 前置，§5.3 同一函数）。
        assert_eq!(
            verify_signature(TEST_VK, &test_nonce_b64(), "ms-0a1b2c3d4e5f", "vehicle_cam0", TEST_SIG_CAM0),
            Ok(())
        );
        // 垃圾 vk / 垃圾 sig → 统一 BadSecret（不外泄失败种类）。
        assert_eq!(
            verify_signature("!!!", &test_nonce_b64(), "ms-0a1b2c3d4e5f", "vehicle_cam0", TEST_SIG_CAM0),
            Err(DeviceAuthError::BadSecret)
        );
        assert_eq!(
            verify_signature(TEST_VK, &test_nonce_b64(), "ms-0a1b2c3d4e5f", "vehicle_cam0", "notb64"),
            Err(DeviceAuthError::BadSecret)
        );
    }

    #[test]
    fn enroll_auto_persists_and_rejects_bad_inputs() {
        // ms-0a1b2c3d4e5f = sig_vector 绑定设备（sig 消息含 device_id，D-E7）。
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
            DeviceRegError::InvalidPublicKey("public_key 不合规: not valid base64".into())
        );
        // save+reload 后公钥形仍可直接验签（热生效链 = 同一 Arc 实例运行期即生效，落盘保重启）
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
        // 同 id 换 vk 重报 → 覆盖 + WARN（first_seen 刷新 = 重报时刻，D-E5 语义）
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
        // secret 面对公钥形条目: authenticate=BadSecret 家族 / reset_secret=Unknown / register=Duplicate
        let reg = DeviceRegistry::from_yaml(&pubkey_yaml()).unwrap();
        assert_eq!(
            authenticate(&reg, Some("ms-0a1b2c3d4e5f"), Some("whatever")),
            Some(Err(DeviceAuthError::BadSecret))
        );
        assert_eq!(reg.reset_secret("ms-0a1b2c3d4e5f").unwrap_err(), DeviceRegError::Unknown);
        assert_eq!(reg.register("ms-0a1b2c3d4e5f").unwrap_err(), DeviceRegError::Duplicate);
        // 时序面: 未知设备与公钥形条目都走 dummy 全路径（响应时间不可区分，review #1）
        assert_eq!(
            authenticate(&reg, Some("ms-ghost"), Some("x")),
            Some(Err(DeviceAuthError::Unknown))
        );
    }
}
