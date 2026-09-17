//! C 配置结构镜像 + 校验纯函数（struct_size 前向兼容 / 必填 / 凭证恰一 / role 表）。

use std::os::raw::c_int;
use std::ptr;

use mediaservo_common::protocol::PeerRole;

use crate::errors::{
    MEDIASERVO_CLIENT_ERR_INVALID_ARG, check_struct_size, cstr, parse_role, set_last_error,
};

/// 登录配置。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
#[repr(C)]
pub struct ms_client_login_config_t {
    pub struct_size: usize,
    pub http_base_url: *const std::os::raw::c_char,
    pub username: *const std::os::raw::c_char,
    pub password: *const std::os::raw::c_char,
}

pub const MS_CLIENT_LOGIN_CONFIG_MIN_SIZE: usize = size_of::<ms_client_login_config_t>();

impl Default for ms_client_login_config_t {
    fn default() -> Self {
        Self {
            struct_size: MS_CLIENT_LOGIN_CONFIG_MIN_SIZE,
            http_base_url: ptr::null(),
            username: ptr::null(),
            password: ptr::null(),
        }
    }
}

/// 会话配置。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
#[repr(C)]
pub struct ms_client_config_t {
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
}

pub const MEDIASERVO_CLIENT_CONFIG_MIN_SIZE: usize = size_of::<ms_client_config_t>();

impl Default for ms_client_config_t {
    fn default() -> Self {
        Self {
            struct_size: MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
            signaling_url: ptr::null(),
            room: ptr::null(),
            jwt: ptr::null(),
            psk: ptr::null(),
            role: ptr::null(),
            hmac_key_file: ptr::null(),
        }
    }
}

/// 登录配置校验（纯函数，单测钉）。空串按缺失处理。
pub(crate) fn validate_login_cfg(
    cfg: &ms_client_login_config_t,
) -> Result<(&str, &str, &str), c_int> {
    check_struct_size(
        cfg.struct_size,
        MS_CLIENT_LOGIN_CONFIG_MIN_SIZE,
        "ms_client_login",
    )?;
    let required = |p: *const std::os::raw::c_char| -> Option<&str> {
        cstr(p).ok().flatten().filter(|s| !s.is_empty())
    };
    let (Some(base), Some(user), Some(pass)) = (
        required(cfg.http_base_url),
        required(cfg.username),
        required(cfg.password),
    ) else {
        set_last_error("ms_client_login: http_base_url/username/password all required");
        return Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    };
    Ok((base, user, pass))
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
}

/// 会话配置校验（纯函数，单测钉）：url/room 必填；jwt/psk 恰一非空；role 可空。
pub(crate) fn validate_session_cfg(cfg: &ms_client_config_t) -> Result<SessionCfg<'_>, c_int> {
    check_struct_size(
        cfg.struct_size,
        MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
        "ms_client_session_create",
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
        set_last_error("ms_client_session_create: signaling_url/room required");
        return Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    };
    let jwt = opt_nonempty(cfg.jwt);
    let psk = opt_nonempty(cfg.psk);
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
            })
        }
        (Some(j), Some(p)) if clean(j) && clean(p) => {
            set_last_error(
                "ms_client_session_create: exactly one of jwt/psk required (both given)",
            );
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
        (Some(_), _) | (_, Some(_)) => {
            set_last_error("ms_client_session_create: invalid UTF-8 in jwt/psk");
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
        (None, None) => {
            set_last_error(
                "ms_client_session_create: exactly one of jwt/psk required (neither)",
            );
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
    }
}

fn session_hmac_key_file(cfg: &ms_client_config_t) -> Option<&str> {
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

fn session_role(cfg: &ms_client_config_t) -> Result<PeerRole, c_int> {
    match cstr(cfg.role) {
        Ok(Some(r)) if !r.is_empty() => parse_role(r).map_err(|()| {
            set_last_error(format!(
                "ms_client_session_create: unknown role '{r}' (Client/Viewer/Remote/Host)"
            ));
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        }),
        Ok(Some(_)) | Ok(None) => Ok(PeerRole::Consumer), // NULL/空 = "Client" 缺省
        Err(()) => {
            set_last_error("ms_client_session_create: invalid UTF-8 in role");
            Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn login_cfg_small_struct_size_rejected() {
        let cfg = ms_client_login_config_t {
            struct_size: 1,
            ..Default::default()
        };
        assert_eq!(
            validate_login_cfg(&cfg).unwrap_err(),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }

    #[test]
    fn login_cfg_missing_fields_rejected() {
        let cfg = ms_client_login_config_t::default();
        assert_eq!(
            validate_login_cfg(&cfg).unwrap_err(),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        let user = c"op";
        let cfg = ms_client_login_config_t {
            struct_size: MS_CLIENT_LOGIN_CONFIG_MIN_SIZE,
            http_base_url: c"http://h:9800".as_ptr(),
            username: user.as_ptr(),
            password: ptr::null(),
        };
        assert_eq!(
            validate_login_cfg(&cfg).unwrap_err(),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }

    #[test]
    fn login_cfg_ok_path() {
        let cfg = ms_client_login_config_t {
            struct_size: MS_CLIENT_LOGIN_CONFIG_MIN_SIZE,
            http_base_url: c"http://h:9800".as_ptr(),
            username: c"op".as_ptr(),
            password: c"pw".as_ptr(),
        };
        let (base, user, pass) = validate_login_cfg(&cfg).expect("valid");
        assert_eq!((base, user, pass), ("http://h:9800", "op", "pw"));
    }

    #[test]
    fn session_cfg_requires_exactly_one_credential() {
        let mk = |jwt: *const std::os::raw::c_char, psk: *const std::os::raw::c_char| {
            ms_client_config_t {
                struct_size: MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
                signaling_url: c"ws://h:9800/ws".as_ptr(),
                room: c"r".as_ptr(),
                jwt,
                psk,
                role: ptr::null(),
                hmac_key_file: ptr::null(),
            }
        };
        // 双凭证 → 拒
        let both = mk(c"j".as_ptr(), c"p".as_ptr());
        assert_eq!(
            validate_session_cfg(&both).unwrap_err(),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // 无凭证 → 拒
        let none = mk(ptr::null(), ptr::null());
        assert_eq!(
            validate_session_cfg(&none).unwrap_err(),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // jwt 单 → 过
        let jwt = mk(c"j".as_ptr(), ptr::null());
        let parts = validate_session_cfg(&jwt).expect("valid");
        assert_eq!(parts.jwt, Some("j"));
        assert_eq!(parts.role, PeerRole::Consumer); // NULL role = "Client" 缺省
    }

    #[test]
    fn session_cfg_bad_role_rejected() {
        let cfg = ms_client_config_t {
            struct_size: MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
            signaling_url: c"ws://h:9800/ws".as_ptr(),
            room: c"r".as_ptr(),
            jwt: c"j".as_ptr(),
            psk: ptr::null(),
            role: c"Bogus".as_ptr(),
            hmac_key_file: ptr::null(),
        };
        assert_eq!(
            validate_session_cfg(&cfg).unwrap_err(),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
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
        let cfg = ms_client_config_t {
            struct_size: MEDIASERVO_CLIENT_CONFIG_MIN_SIZE,
            signaling_url: c"ws://h:9800/ws".as_ptr(),
            room: c"r".as_ptr(),
            jwt: c"j".as_ptr(),
            psk: ptr::null(),
            role: ptr::null(),
            hmac_key_file: c"/tmp/k".as_ptr(),
        };
        assert_eq!(validate_session_cfg(&cfg).unwrap().hmac_key_file, Some("/tmp/k"));
    }
}
