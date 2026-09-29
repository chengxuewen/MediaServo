//! C 配置结构镜像 + 校验纯函数（struct_size 前向兼容 / 必填 / 凭证恰一 / role 表）。

use std::os::raw::c_int;
use std::ptr;

use mediaservo_common::protocol::PeerRole;

use crate::errors::{
    MEDIASERVO_CLIENT_ERR_INVALID_ARG, check_struct_size, cstr, parse_role, set_last_error,
};

/// 登录配置（⊘ 保留一周期：批1b 起 login 转扁平参数形，本结构不再被消费；
/// struct_size 演进纪律下的存量形状，批2 随 ⊘ 清单一并移除）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
#[repr(C)]
pub struct mediaservo_client_login_config_t {
    pub struct_size: usize,
    pub http_base_url: *const std::os::raw::c_char,
    pub username: *const std::os::raw::c_char,
    pub password: *const std::os::raw::c_char,
}

pub const MEDIASERVO_CLIENT_LOGIN_CONFIG_MIN_SIZE: usize =
    size_of::<mediaservo_client_login_config_t>();

impl Default for mediaservo_client_login_config_t {
    fn default() -> Self {
        Self {
            struct_size: MEDIASERVO_CLIENT_LOGIN_CONFIG_MIN_SIZE,
            http_base_url: ptr::null(),
            username: ptr::null(),
            password: ptr::null(),
        }
    }
}

/// 会话配置。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
#[repr(C)]
pub struct mediaservo_client_config_t {
    pub struct_size: usize,
    pub signaling_url: *const std::os::raw::c_char,
    pub room: *const std::os::raw::c_char,
    pub jwt: *const std::os::raw::c_char,
    pub psk: *const std::os::raw::c_char,
    pub role: *const std::os::raw::c_char,
    /// 急停 HMAC 密钥文件路径（W4b；G13 语义=密钥永不走 argv/env 明文）。
    /// 文件须 0600 且非空；NULL/空 = estop 不签名（车端未配 key = 迁移放行形，
    /// 车端已配 key = 拒签正确裁决）。载荷 = 原始密钥字节（trim 尾换行）。
    pub hmac_key_file: *const std::os::raw::c_char,
    /// 设备身份实例目录（viewer-auth-matrix T1，accountless T3 布局：identity.json +
    /// etc/link/signing.pem）。**尾部 additive**：MIN_SIZE = 旧形状（本字段在覆盖区外），
    /// 老调用方 struct_size 校验照过；读取必须按 `cfg.struct_size` 判界（见
    /// [`identity_dir_of`]——老二进制传小结构 = 字段不存在，非解引用垃圾）。
    /// NULL/空 = 不启用。与 jwt/psk 可叠（server 设备认证优先，D-E3）；防呆=三凭证
    /// 齐给 INVALID_ARG。
    pub identity_dir: *const std::os::raw::c_char,
}

/// 新调用方（含 identity_dir 字段）的完整尺寸。MIN_SIZE 保持旧形状值——
/// check_struct_size(actual < MIN) 拒，[MIN, FULL) = 老形状（identity_dir 缺席）。
pub const MEDIASERVO_CLIENT_CONFIG_FULL_SIZE: usize = size_of::<mediaservo_client_config_t>();
pub const MEDIASERVO_CLIENT_CONFIG_MIN_SIZE: usize =
    MEDIASERVO_CLIENT_CONFIG_FULL_SIZE - size_of::<*const std::os::raw::c_char>();

/// additive 字段安全读取：调用方 struct_size ≥ FULL 才有该字段（返回 None = 缺席）。
pub(crate) fn identity_dir_of(cfg: &mediaservo_client_config_t) -> Option<&str> {
    if cfg.struct_size < MEDIASERVO_CLIENT_CONFIG_FULL_SIZE {
        return None;
    }
    match crate::errors::cstr(cfg.identity_dir) {
        Ok(Some(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

impl Default for mediaservo_client_config_t {
    fn default() -> Self {
        Self {
            struct_size: MEDIASERVO_CLIENT_CONFIG_FULL_SIZE,
            signaling_url: ptr::null(),
            room: ptr::null(),
            jwt: ptr::null(),
            psk: ptr::null(),
            role: ptr::null(),
            hmac_key_file: ptr::null(),
            identity_dir: ptr::null(),
        }
    }
}

/// 会话配置校验产物。
#[derive(Debug)]
pub(crate) struct SessionCfg<'a> {
    pub signaling_url: &'a str,
    pub room: &'a str,
    pub jwt: Option<&'a str>,
    pub psk: Option<&'a str>,
    pub role: PeerRole,
    pub hmac_key_file: Option<&'a str>,
    pub identity_dir: Option<&'a str>,
}

/// 会话配置校验（纯函数，单测钉）：url/room 必填；jwt/psk 恰一非空；role 可空。
pub(crate) fn validate_session_cfg(
    cfg: &mediaservo_client_config_t,
) -> Result<SessionCfg<'_>, c_int> {
    check_struct_size(
        cfg.struct_size,
        MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
        "mediaservo_client_session_create",
    )?;
    let required = |p: *const std::os::raw::c_char| -> Option<&str> {
        cstr(p).ok().flatten().filter(|s| !s.is_empty())
    };
    let opt_nonempty = |p: *const std::os::raw::c_char| -> Option<&str> {
        match cstr(p) {
            Ok(Some(s)) if !s.is_empty() => Some(s),
            Ok(_) => None,
            Err(()) => Some("\u{0}invalid-utf8"), // 哨兵：区分「缺失」与「非法」
        }
    };
    let (Some(url), Some(room)) = (required(cfg.signaling_url), required(cfg.room)) else {
        set_last_error("mediaservo_client_session_create: signaling_url/room required");
        return Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    };
    let jwt = opt_nonempty(cfg.jwt);
    let psk = opt_nonempty(cfg.psk);
    // additive 字段按 struct_size 判界读取（老调用方无此字段=None，非垃圾）。
    let identity_dir = identity_dir_of(cfg);
    // 防呆（T1 合同）：三凭证齐给 = INVALID_ARG。jwt/psk 仍恰一（不变）；
    // identity_dir 与任一可叠（server 设备认证优先，D-E3）。
    if identity_dir.is_some() && jwt.is_some() && psk.is_some() {
        set_last_error(
            "mediaservo_client_session_create: identity_dir with both jwt and psk (pick two at most)",
        );
        return Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }
    let clean = |s: &str| !s.starts_with('\u{0}');
    match (jwt, psk) {
        (Some(j), None) if clean(j) => {
            let role = session_role(cfg)?;
            Ok(SessionCfg {
                signaling_url: url,
                room,
                jwt: Some(j),
                psk: None,
                role,
                hmac_key_file: session_hmac_key_file(cfg),
                identity_dir,
            })
        }
        (None, Some(p)) if clean(p) => {
            let role = session_role(cfg)?;
            Ok(SessionCfg {
                signaling_url: url,
                room,
                jwt: None,
                psk: Some(p),
                role,
                hmac_key_file: session_hmac_key_file(cfg),
                identity_dir,
            })
        }
        (Some(j), Some(p)) if clean(j) && clean(p) => {
            set_last_error(
                "mediaservo_client_session_create: exactly one of jwt/psk required (both given)",
            );
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
        (Some(_), _) | (_, Some(_)) => {
            set_last_error("mediaservo_client_session_create: invalid UTF-8 in jwt/psk");
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
        (None, None) => {
            set_last_error(
                "mediaservo_client_session_create: exactly one of jwt/psk required (neither)",
            );
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
    }
}

fn session_hmac_key_file(cfg: &mediaservo_client_config_t) -> Option<&str> {
    // cstr 生命周期 = cfg 借用期（SessionCfg 生命周期随 cfg）。非法 UTF-8 与缺失同路
    // = 该路径在会话建立期不报错（可选字段），estop 使用时按"文件不可读"暴露。
    match cstr(cfg.hmac_key_file) {
        Ok(Some(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// W4b：读急停密钥文件——薄转调 common 真源（车舱同纪律，防双实现漂移）。
pub(crate) fn load_hmac_key_file(path: &str) -> Result<String, String> {
    mediaservo_common::protocol::control_hmac_key_from_file(path)
}

fn session_role(cfg: &mediaservo_client_config_t) -> Result<PeerRole, c_int> {
    match cstr(cfg.role) {
        Ok(Some(r)) if !r.is_empty() => parse_role(r).map_err(|()| {
            set_last_error(format!(
                "mediaservo_client_session_create: unknown role '{r}' (Client/Viewer/Remote/Host)"
            ));
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        }),
        Ok(Some(_)) | Ok(None) => Ok(PeerRole::Consumer), // NULL/空 = "Client" 缺省
        Err(()) => {
            set_last_error("mediaservo_client_session_create: invalid UTF-8 in role");
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn session_cfg_requires_exactly_one_credential() {
        let mk = |jwt: *const std::os::raw::c_char, psk: *const std::os::raw::c_char| {
            mediaservo_client_config_t {
                struct_size: MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
                signaling_url: c"ws://h:9800/ws".as_ptr(),
                room: c"r".as_ptr(),
                jwt,
                psk,
                role: ptr::null(),
                hmac_key_file: ptr::null(),
            identity_dir: ptr::null(),
            }
        };
        // 双凭证 → 拒
        let both = mk(c"j".as_ptr(), c"p".as_ptr());
        assert_eq!(validate_session_cfg(&both).unwrap_err(), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // 无凭证 → 拒
        let none = mk(ptr::null(), ptr::null());
        assert_eq!(validate_session_cfg(&none).unwrap_err(), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // jwt 单 → 过
        let jwt = mk(c"j".as_ptr(), ptr::null());
        let parts = validate_session_cfg(&jwt).expect("valid");
        assert_eq!(parts.jwt, Some("j"));
        assert_eq!(parts.role, PeerRole::Consumer); // NULL role = "Client" 缺省
    }

    #[test]
    fn session_cfg_bad_role_rejected() {
        let cfg = mediaservo_client_config_t {
            struct_size: MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
            signaling_url: c"ws://h:9800/ws".as_ptr(),
            room: c"r".as_ptr(),
            jwt: c"j".as_ptr(),
            psk: ptr::null(),
            role: c"Bogus".as_ptr(),
            hmac_key_file: ptr::null(),
            identity_dir: ptr::null(),
        };
        assert_eq!(validate_session_cfg(&cfg).unwrap_err(), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }
    #[test]
    fn hmac_key_file_gate_and_trim() {
        let dir = std::env::temp_dir().join(format!("msw4b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("k.key");
        std::fs::write(&p, b"s3cret\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(load_hmac_key_file(p.to_str().unwrap()).unwrap(), "s3cret"); // trim 钉
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_hmac_key_file(p.to_str().unwrap()).unwrap_err().contains("过宽"));
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&p, b"\n").unwrap();
        assert!(load_hmac_key_file(p.to_str().unwrap()).unwrap_err().contains("空密钥"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_cfg_reads_hmac_key_file_optional() {
        let cfg = mediaservo_client_config_t {
            struct_size: MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
            signaling_url: c"ws://h:9800/ws".as_ptr(),
            room: c"r".as_ptr(),
            jwt: c"j".as_ptr(),
            psk: ptr::null(),
            role: ptr::null(),
            hmac_key_file: c"/tmp/k".as_ptr(),
            identity_dir: ptr::null(),
        };
        assert_eq!(validate_session_cfg(&cfg).unwrap().hmac_key_file, Some("/tmp/k"));
    }

    // ── viewer-auth-matrix T1: identity_dir additive 字段判别 ──
    fn base_cfg() -> mediaservo_client_config_t {
        mediaservo_client_config_t {
            struct_size: MEDIASERVO_CLIENT_CONFIG_FULL_SIZE,
            signaling_url: c"ws://h:9800/ws".as_ptr(),
            room: c"r".as_ptr(),
            jwt: c"j".as_ptr(),
            psk: ptr::null(),
            role: ptr::null(),
            hmac_key_file: ptr::null(),
            identity_dir: ptr::null(),
        }
    }

    #[test]
    fn identity_dir_absent_on_legacy_struct_size() {
        // 老调用方（MIN_SIZE）传小结构：字段槽不存在 → None（非解引用垃圾）。
        let mut cfg = base_cfg();
        cfg.struct_size = MEDIASERVO_CLIENT_CONFIG_MIN_SIZE;
        cfg.identity_dir = c"/evil".as_ptr(); // 槽外内存——必须被忽略
        let parts = validate_session_cfg(&cfg).expect("legacy shape still valid");
        assert_eq!(parts.identity_dir, None);
    }

    #[test]
    fn identity_dir_passed_through_when_present() {
        let mut cfg = base_cfg();
        cfg.identity_dir = c"/tmp/inst".as_ptr();
        let parts = validate_session_cfg(&cfg).expect("valid");
        assert_eq!(parts.identity_dir, Some("/tmp/inst"));
        // 空串 = 不启用
        cfg.identity_dir = c"".as_ptr(); // 空串 = 不启用（identity_dir_of 过滤）
        let parts = validate_session_cfg(&cfg).expect("valid");
        assert_eq!(parts.identity_dir, None);
    }

    #[test]
    fn identity_dir_with_both_credentials_rejected() {
        let mut cfg = base_cfg();
        cfg.identity_dir = c"/tmp/inst".as_ptr();
        cfg.psk = c"p".as_ptr();
        assert_eq!(validate_session_cfg(&cfg).unwrap_err(), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }
}
