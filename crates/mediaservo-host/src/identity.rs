//! 设备身份（G4 + device-enroll T6）：`identity.json` 生成/加载 + `DeviceIdentity` 装配。
//!
//! 布局（D-H13）：实例根目录 `<dir>/identity.json`，0600。形状（device-enroll §4）：
//! ```json
//! { "device_id": "ms-<12 hex>" }
//! ```
//! **无 secret**——准入凭据 = `etc/link/signing.pem`（Ed25519 PKCS#8，D-E1 一钥两用）
//! 派生的公钥指纹；serde 忽略未知字段 → 旧形状文件（含 `device_secret`）仍可解析，
//! 多余字段丢弃、不影响新链（D-E3 共存周期）。
//! - `device_id`：随机 6 字节 hex，前缀 `ms-`（稳定唯一即可——server 侧注册键）。
//! - 再生策略：`host init` 幂等——**仅缺失时生成**；覆盖会使 server 侧注册失效（G2）。
//!   存在但损坏 → 显式报错（C15），不静默覆盖。

use std::path::Path;

use mediaservo_link::DeviceIdentity;
use serde::{Deserialize, Serialize};

/// identity.json 文件名（实例根目录，D-H13）。
pub const IDENTITY_FILE: &str = "identity.json";

/// signing.pem 相对路径（实例根下，init 生成 0600，D-E1）。
pub const SIGNING_PEM: &str = "etc/link/signing.pem";

/// identity.json 文件形状（新形状仅 device_id；未知字段默认忽略 = 旧文件兼容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IdentityFile {
    device_id: String,
}

/// 生成新 device_id `<brand>-<12 hex>`（默认 "ms-"，legacy 映射见 brand.rs）。
pub fn generate_device_id() -> String {
    use rand_core::RngCore;
    let mut id_bytes = [0u8; 6];
    rand_core::OsRng.fill_bytes(&mut id_bytes);
    format!(
        "{}{}",
        mediaservo_common::brand::media_brand().device_prefix,
        hex(&id_bytes)
    )
}

/// `host init`：幂等确保 identity.json 存在（存在 → 返回其 device_id，不覆盖；
/// 缺失 → 生成并 0600 写入）。损坏文件 → Err（C15 显式报错）。
pub fn ensure_identity(dir: &Path) -> Result<String, String> {
    if let Some(existing) = load_identity(dir)? {
        return Ok(existing);
    }
    let id = IdentityFile {
        device_id: generate_device_id(),
    };
    let path = dir.join(IDENTITY_FILE);
    let json = serde_json::to_string_pretty(&id)
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    write_secret_file(&path, json.as_bytes())?;
    Ok(id.device_id)
}

/// 加载 device_id：文件缺失 → `Ok(None)`（PSK 回落路径）；存在但不可解析 → Err。
pub fn load_identity(dir: &Path) -> Result<Option<String>, String> {
    let path = dir.join(IDENTITY_FILE);
    let raw = match std::fs::read(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    let id: IdentityFile = serde_json::from_slice(&raw)
        .map_err(|e| format!("{} 解析失败: {e}", path.display()))?;
    Ok(Some(id.device_id))
}

/// 装配 [`DeviceIdentity`]：identity.json(device_id) + `etc/link/signing.pem`(PKCS#8)
/// → 派生公钥指纹。identity.json 缺失 → `Ok(None)`（PSK 回落）；PEM 缺失/损坏 →
/// 显式 Err（host-agent 侧 warn 退 PSK-only）。
pub fn load_device_identity(dir: &Path) -> Result<Option<DeviceIdentity>, String> {
    let Some(device_id) = load_identity(dir)? else {
        return Ok(None);
    };
    use ed25519_dalek::pkcs8::DecodePrivateKey as _;
    let pem_path = dir.join(SIGNING_PEM);
    let pem = std::fs::read(&pem_path)
        .map_err(|e| format!("读取 {} 失败（host init 生成）: {e}", pem_path.display()))?;
    let signing = ed25519_dalek::SigningKey::from_pkcs8_pem(&String::from_utf8_lossy(&pem))
        .map_err(|e| format!("{} 解析失败（Ed25519 PKCS#8 PEM）: {e}", pem_path.display()))?;
    Ok(Some(DeviceIdentity::new(device_id, signing)))
}

/// 写凭据文件并设 0600（与 signing.pem 同纪律；幂等由调用方保证）。
fn write_secret_file(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)
        .map_err(|e| format!("创建 {} 失败: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("设置 {} 权限失败: {e}", path.display()))?;
    }
    f.write_all(data)
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

/// 小端 hex（无依赖；device_id 为固定长度）。
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写实例 signing.pem（PKCS#8 PEM，与 `host init` gen_signing_pem 同法）。
    fn write_signing_pem(dir: &Path, seed: [u8; 32]) {
        use pkcs8::EncodePrivateKey;
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        let pem = signing.to_pkcs8_pem(pkcs8::LineEnding::LF).expect("pkcs8 pem");
        let link = dir.join("etc").join("link");
        std::fs::create_dir_all(&link).expect("mkdir etc/link");
        std::fs::write(link.join("signing.pem"), pem.as_bytes()).expect("write pem");
    }

    #[test]
    fn generate_device_id_shape_and_uniqueness() {
        let a = generate_device_id();
        assert!(a.starts_with("ms-"), "device_id 应带 ms- 前缀: {a}");
        assert_eq!(a.len(), 15, "ms- + 12 hex");
        assert!(a[3..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, generate_device_id(), "随机 device_id 不得碰撞");
    }

    #[test]
    fn ensure_identity_writes_new_shape_0600_and_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = ensure_identity(dir.path()).expect("ensure");
        let path = dir.path().join(IDENTITY_FILE);
        let raw = std::fs::read(&path).expect("read identity");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "identity.json 必须 0600");
        }
        // 新形状：仅 device_id，无 secret
        let value: serde_json::Value = serde_json::from_slice(&raw).expect("parse");
        assert_eq!(value["device_id"].as_str().expect("device_id"), first);
        assert_eq!(value.get("device_secret"), None, "新形状不得写 secret");
        assert_eq!(value.as_object().expect("obj").len(), 1, "仅一个字段");
        // 幂等：已存在 → 不覆盖（内容不变，device_id 不换）
        let second = ensure_identity(dir.path()).expect("ensure again");
        assert_eq!(second, first, "已存在的身份不得再生");
        assert_eq!(std::fs::read(&path).expect("read again"), raw, "文件内容不得变化");
    }

    #[test]
    fn legacy_file_with_secret_still_loads() {
        // D-E3 兼容：旧形状 {device_id, device_secret} 可读不报错（多余字段丢弃）。
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(IDENTITY_FILE),
            br#"{"device_id": "ms-0a1b2c3d4e5f", "device_secret": "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"}"#,
        )
        .expect("write legacy identity");
        assert_eq!(
            load_identity(dir.path()).expect("legacy parse").expect("Some"),
            "ms-0a1b2c3d4e5f"
        );
        // ensure 幂等同样不覆盖旧文件
        assert_eq!(ensure_identity(dir.path()).expect("ensure legacy"), "ms-0a1b2c3d4e5f");
        let raw = std::fs::read(dir.path().join(IDENTITY_FILE)).expect("read");
        assert!(
            std::str::from_utf8(&raw).expect("utf8").contains("device_secret"),
            "ensure 不得改写旧文件（保留原字节）"
        );
    }

    #[test]
    fn load_identity_missing_returns_none_and_corrupt_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(load_identity(dir.path()), Ok(None)), "缺失 → None（PSK 回落）");
        std::fs::write(dir.path().join(IDENTITY_FILE), b"not json").expect("write corrupt");
        let err = load_identity(dir.path()).expect_err("损坏文件必须报错");
        assert!(err.contains("解析失败"), "应指出解析失败: {err}");
    }

    #[test]
    fn load_device_identity_derives_pubkey_from_pem() {
        let dir = tempfile::tempdir().expect("tempdir");
        ensure_identity(dir.path()).expect("ensure identity");
        write_signing_pem(dir.path(), std::array::from_fn(|i| i as u8));
        let ident = load_device_identity(dir.path())
            .expect("load")
            .expect("Some（identity+PEM 齐备）");
        assert_eq!(ident.device_id, load_identity(dir.path()).unwrap().unwrap());
        assert_eq!(
            ident.pubkey_b64, "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=",
            "seed(0..=31) 的 vk 指纹必须 = server sig_vector 钉值（同锚）"
        );
    }

    #[test]
    fn load_device_identity_missing_identity_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(load_device_identity(dir.path()), Ok(None)), "无 identity.json → None（PSK 回落）");
    }

    #[test]
    fn load_device_identity_missing_pem_errors_explicitly() {
        let dir = tempfile::tempdir().expect("tempdir");
        ensure_identity(dir.path()).expect("ensure identity");
        let err = load_device_identity(dir.path()).expect_err("PEM 缺失必须显式报错");
        assert!(err.contains("signing.pem"), "错误应指向 signing.pem: {err}");
    }

    #[test]
    fn load_device_identity_corrupt_pem_errors_explicitly() {
        let dir = tempfile::tempdir().expect("tempdir");
        ensure_identity(dir.path()).expect("ensure identity");
        let link = dir.path().join("etc").join("link");
        std::fs::create_dir_all(&link).expect("mkdir");
        std::fs::write(link.join("signing.pem"), b"-----BEGIN PRIVATE KEY-----\nbroken\n").expect("write");
        let err = load_device_identity(dir.path()).expect_err("坏 PEM 必须显式报错");
        assert!(err.contains("signing.pem 解析失败"), "应指出 PEM 解析失败: {err}");
    }
}
