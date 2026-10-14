//! C config struct mirrors + validation pure functions (struct_size forward compat / required fields / exactly-one credential / role table).

use std::os::raw::c_int;
use std::ptr;

use mediaservo_common::protocol::PeerRole;

use crate::errors::{
    MEDIASERVO_CLIENT_ERR_INVALID_ARG, check_struct_size, cstr, parse_role, set_last_error,
};

/// Login config (⊘ kept for one period: since batch 1b login uses flat parameters, this struct is no longer consumed;
/// a legacy shape under the struct_size evolution discipline; batch 2 removes it with the ⊘ list).
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
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

/// Session configuration.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
#[repr(C)]
pub struct mediaservo_client_config_t {
    pub struct_size: usize,
    pub signaling_url: *const std::os::raw::c_char,
    pub room: *const std::os::raw::c_char,
    pub jwt: *const std::os::raw::c_char,
    pub psk: *const std::os::raw::c_char,
    pub role: *const std::os::raw::c_char,
    /// Emergency-stop HMAC key file path (W4b; G13 semantics = keys never travel as plaintext argv/env).
    /// The file must be 0600 and non-empty; NULL/empty = estop is not signed (no key on the vehicle = migration-allow form;
    /// key present = refusing to sign is the correct ruling). Payload = the raw key bytes (trailing newline trimmed).
    pub hmac_key_file: *const std::os::raw::c_char,
    /// Device identity instance directory (viewer-auth-matrix T1, accountless T3 layout: identity.json +
    /// etc/link/signing.pem). **Trailing additive**: MIN_SIZE = the old shape (this field is outside the covered region),
    /// so old callers still pass the struct_size check; reads must bound-check against `cfg.struct_size` (see
    /// [`identity_dir_of`] — an old binary passing the small struct means the field is absent, not garbage to dereference).
    /// NULL/empty = disabled. Composable with jwt/psk (server device auth takes priority, D-E3); a safety rule = all three credentials
    /// together → INVALID_ARG.
    pub identity_dir: *const std::os::raw::c_char,
}

/// Full size for new callers (including the identity_dir field). MIN_SIZE keeps the old-shape value —
/// check_struct_size(actual < MIN) rejects; [MIN, FULL) = old shape (identity_dir absent).
pub const MEDIASERVO_CLIENT_CONFIG_FULL_SIZE: usize = size_of::<mediaservo_client_config_t>();
pub const MEDIASERVO_CLIENT_CONFIG_MIN_SIZE: usize =
    MEDIASERVO_CLIENT_CONFIG_FULL_SIZE - size_of::<*const std::os::raw::c_char>();

/// Safe read of the additive field: the field exists only when the caller's struct_size ≥ FULL (None = absent).
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

/// Session config validation output.
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

/// Session config validation (pure function, pinned by unit tests): url/room required; exactly one of jwt/psk non-empty; role optional.
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
            Err(()) => Some("\u{0}invalid-utf8"), // sentinel: distinguishes "missing" from "invalid"
        }
    };
    let (Some(url), Some(room)) = (required(cfg.signaling_url), required(cfg.room)) else {
        set_last_error("mediaservo_client_session_create: signaling_url/room required");
        return Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    };
    let jwt = opt_nonempty(cfg.jwt);
    let psk = opt_nonempty(cfg.psk);
    // The additive field is read with a struct_size bound check (old callers have no such field → None, not garbage).
    let identity_dir = identity_dir_of(cfg);
    // Safety rule (T1 contract): all three credentials at once = INVALID_ARG. jwt/psk still exactly-one (unchanged);
    // identity_dir composes with either (server device auth takes priority, D-E3).
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
    // cstr lifetime = the cfg borrow lifetime (SessionCfg lifetime follows cfg). Invalid UTF-8 and missing share a path
    // = this path does not error at session setup (optional field); it surfaces as "file unreadable" when estop uses it.
    match cstr(cfg.hmac_key_file) {
        Ok(Some(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// W4b: read the emergency-stop key file — thin delegation to the common source of truth (same discipline on vehicle and cockpit, preventing dual-implementation drift).
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
        Ok(Some(_)) | Ok(None) => Ok(PeerRole::Consumer), // NULL/empty = "Client" default
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
        // Both credentials → reject
        let both = mk(c"j".as_ptr(), c"p".as_ptr());
        assert_eq!(validate_session_cfg(&both).unwrap_err(), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // No credentials → reject
        let none = mk(ptr::null(), ptr::null());
        assert_eq!(validate_session_cfg(&none).unwrap_err(), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // jwt only → pass
        let jwt = mk(c"j".as_ptr(), ptr::null());
        let parts = validate_session_cfg(&jwt).expect("valid");
        assert_eq!(parts.jwt, Some("j"));
        assert_eq!(parts.role, PeerRole::Consumer); // NULL role = "Client" default
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
        assert_eq!(load_hmac_key_file(p.to_str().unwrap()).unwrap(), "s3cret"); // trim pin
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_hmac_key_file(p.to_str().unwrap()).unwrap_err().contains("too permissive"));
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&p, b"\n").unwrap();
        assert!(load_hmac_key_file(p.to_str().unwrap()).unwrap_err().contains("empty key"));
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

    // -- viewer-auth-matrix T1: identity_dir additive field discrimination --
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
        // An old caller (MIN_SIZE) passing the small struct: the field slot does not exist → None (not garbage to dereference).
        let mut cfg = base_cfg();
        cfg.struct_size = MEDIASERVO_CLIENT_CONFIG_MIN_SIZE;
        cfg.identity_dir = c"/evil".as_ptr(); // memory outside the slot — must be ignored
        let parts = validate_session_cfg(&cfg).expect("legacy shape still valid");
        assert_eq!(parts.identity_dir, None);
    }

    #[test]
    fn identity_dir_passed_through_when_present() {
        let mut cfg = base_cfg();
        cfg.identity_dir = c"/tmp/inst".as_ptr();
        let parts = validate_session_cfg(&cfg).expect("valid");
        assert_eq!(parts.identity_dir, Some("/tmp/inst"));
        // Empty string = disabled
        cfg.identity_dir = c"".as_ptr(); // empty string = disabled (filtered by identity_dir_of)
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
