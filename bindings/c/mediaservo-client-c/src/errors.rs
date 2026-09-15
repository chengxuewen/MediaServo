//! 错误码 / last_error / 纯映射函数（ClientError → C 码、信封构造、辅助提取）。

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

use mediaservo_client::error::ClientError;
use mediaservo_common::protocol::{ControlEnvelope, PeerRole};

// ── 错误码（0 = ok, <0 = error；MEDIASERVO_CLIENT_ERR_*，D241 前缀化）──
pub const MEDIASERVO_OK: c_int = 0;
pub const MEDIASERVO_CLIENT_ERR_INVALID_ARG: c_int = -1;
pub const MEDIASERVO_CLIENT_ERR_LOGIN: c_int = -2;
pub const MEDIASERVO_CLIENT_ERR_UNAUTHORIZED: c_int = -3;
pub const MEDIASERVO_CLIENT_ERR_DENIED: c_int = -4;
pub const MEDIASERVO_CLIENT_ERR_TIMEOUT: c_int = -5;
pub const MEDIASERVO_CLIENT_ERR_SIGNAL: c_int = -6;
pub const MEDIASERVO_CLIENT_ERR_PROTOCOL: c_int = -7;
pub const MEDIASERVO_CLIENT_ERR_MALFORMED: c_int = -8;
pub const MEDIASERVO_CLIENT_ERR_STATE: c_int = -9;
pub const MEDIASERVO_CLIENT_ERR_INTERNAL: c_int = -10;

/// 全局最近错误信息（ms_client_last_error 读取）。
pub(crate) static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub(crate) fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

/// 提取 C 字符串（null → None）。非法 UTF-8 → Err。
pub(crate) fn cstr<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().map(Some).map_err(|_| ())
}

/// 字符串拷入调用方缓冲（NUL 结尾）。cap 不足 → INVALID_ARG（不写半截）。
pub(crate) fn copy_out_str(s: &str, buf: *mut c_char, cap: usize) -> c_int {
    if buf.is_null() || cap == 0 {
        set_last_error("output buffer is null or cap is 0");
        return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
    }
    let bytes = s.as_bytes();
    if bytes.len() + 1 > cap {
        set_last_error(format!(
            "output buffer too small: need {} bytes, got {cap}",
            bytes.len() + 1
        ));
        return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
        *buf.add(bytes.len()) = 0;
    }
    MEDIASERVO_OK
}

/// [`ClientError`] → C 错误码（穷尽 match——新变体编译期强制归类）。
pub(crate) fn error_code(e: &ClientError) -> c_int {
    match e {
        ClientError::Login(_) => MEDIASERVO_CLIENT_ERR_LOGIN,
        ClientError::InvalidCredentials | ClientError::AuthRejected { .. } => {
            MEDIASERVO_CLIENT_ERR_UNAUTHORIZED
        }
        ClientError::ControlDenied(_) => MEDIASERVO_CLIENT_ERR_DENIED,
        ClientError::Timeout { .. } => MEDIASERVO_CLIENT_ERR_TIMEOUT,
        ClientError::Signal(_) => MEDIASERVO_CLIENT_ERR_SIGNAL,
        ClientError::ProtocolUnsupported(_) | ClientError::ProtocolTooLow { .. } => {
            MEDIASERVO_CLIENT_ERR_PROTOCOL
        }
        ClientError::MalformedResponse(_) | ClientError::UnsupportedScheme(_) => {
            MEDIASERVO_CLIENT_ERR_MALFORMED
        }
        ClientError::InvalidState(_) => MEDIASERVO_CLIENT_ERR_STATE,
        // Server 通用承载（4012/4101/4003/4010/4011 已被 from_wire_error 分入
        // ControlDenied/ProtocolUnsupported/AuthRejected，不会落到这里）+ 兜底族。
        ClientError::Server { .. } | ClientError::WebRtc(_) | ClientError::Io(_) => {
            MEDIASERVO_CLIENT_ERR_INTERNAL
        }
    }
}

/// C 侧角色字符串 → PeerRole。
/// Client/Consumer → Consumer（cockpit 消费形，basic.rs 实盘同值）；
/// Viewer → Consumer（消费权限差异由 server 账号 can_control 门裁决，本地同形）；
/// Remote → Remote。
pub(crate) fn parse_role(s: &str) -> Result<PeerRole, ()> {
    match s {
        "Client" | "Consumer" | "Viewer" => Ok(PeerRole::Consumer),
        "Remote" => Ok(PeerRole::Remote),
        "Host" => Ok(PeerRole::Host),
        _ => Err(()),
    }
}

/// struct_size 前向兼容校验（R3）。
pub(crate) fn check_struct_size(actual: usize, min: usize, fn_name: &str) -> Result<(), c_int> {
    if actual < min {
        set_last_error(format!(
            "{fn_name}: cfg.struct_size {actual} < {min} (rebuild with current header)"
        ));
        return Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }
    Ok(())
}

/// 信封构造（NEVER 字符串拼接——serde 序列化由 ControlChannel::send_envelope 承担）。
/// payload_json None/"" → Null；非法 JSON → INVALID_ARG。纯函数，单测钉。
pub(crate) fn build_envelope(
    seq: u64,
    cmd: &str,
    payload_json: Option<&str>,
) -> Result<ControlEnvelope, c_int> {
    let payload = match payload_json {
        None | Some("") => serde_json::Value::Null,
        Some(s) => serde_json::from_str(s).map_err(|e| {
            set_last_error(format!("payload_json invalid: {e}"));
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        })?,
    };
    Ok(ControlEnvelope {
        seq,
        cmd: cmd.to_string(),
        payload,
    })
}

/// last_error 实现（ms_client_last_error 与单测共用，纯缓冲写入）。
pub(crate) fn last_error_impl(buf: *mut c_char, len: usize) -> c_int {
    if buf.is_null() || len == 0 {
        return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
    }
    let msg = LAST_ERROR
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();
    let bytes = msg.as_bytes();
    let n = bytes.len().min(len - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    MEDIASERVO_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_link::LinkError;
    use std::ptr;

    // ── (a) ClientError → 错误码 穷尽映射表 ──
    #[test]
    fn error_code_covers_every_variant() {
        let cases: Vec<(ClientError, c_int)> = vec![
            (ClientError::Login("x".into()), MEDIASERVO_CLIENT_ERR_LOGIN),
            (
                ClientError::InvalidCredentials,
                MEDIASERVO_CLIENT_ERR_UNAUTHORIZED,
            ),
            (
                ClientError::AuthRejected {
                    code: 4010,
                    message: "x".into(),
                },
                MEDIASERVO_CLIENT_ERR_UNAUTHORIZED,
            ),
            (
                ClientError::ControlDenied("x".into()),
                MEDIASERVO_CLIENT_ERR_DENIED,
            ),
            (
                ClientError::Timeout { what: "x" },
                MEDIASERVO_CLIENT_ERR_TIMEOUT,
            ),
            (
                ClientError::Signal(LinkError::Signal("x".into())),
                MEDIASERVO_CLIENT_ERR_SIGNAL,
            ),
            (
                ClientError::ProtocolUnsupported("x".into()),
                MEDIASERVO_CLIENT_ERR_PROTOCOL,
            ),
            (
                ClientError::ProtocolTooLow { need: 2, got: 1 },
                MEDIASERVO_CLIENT_ERR_PROTOCOL,
            ),
            (
                ClientError::MalformedResponse("x".into()),
                MEDIASERVO_CLIENT_ERR_MALFORMED,
            ),
            (
                ClientError::UnsupportedScheme("https".into()),
                MEDIASERVO_CLIENT_ERR_MALFORMED,
            ),
            (
                ClientError::InvalidState("x".into()),
                MEDIASERVO_CLIENT_ERR_STATE,
            ),
            (
                ClientError::Server {
                    code: 5000,
                    message: "x".into(),
                },
                MEDIASERVO_CLIENT_ERR_INTERNAL,
            ),
            (ClientError::WebRtc("x".into()), MEDIASERVO_CLIENT_ERR_INTERNAL),
            (
                ClientError::Io(std::io::Error::other("x")),
                MEDIASERVO_CLIENT_ERR_INTERNAL,
            ),
        ];
        for (e, want) in cases {
            assert_eq!(error_code(&e), want, "variant: {e:?}");
        }
    }

    // ── 信封构造纯函数 ──
    #[test]
    fn build_envelope_null_payloads() {
        for p in [None, Some("")] {
            let env = build_envelope(7, "steer", p).expect("valid");
            assert_eq!(env.seq, 7);
            assert_eq!(env.cmd, "steer");
            assert_eq!(env.payload, serde_json::Value::Null);
        }
    }

    #[test]
    fn build_envelope_object_payload() {
        let env = build_envelope(1, "steer", Some("{\"deg\":10.5}")).expect("valid");
        assert_eq!(env.payload["deg"], serde_json::json!(10.5));
        // 线形 = ControlEnvelope serde（seq/cmd/payload）
        let text = serde_json::to_string(&env).unwrap();
        let back: ControlEnvelope = serde_json::from_str(&text).unwrap();
        assert_eq!(back.seq, 1);
        assert_eq!(back.cmd, "steer");
    }

    #[test]
    fn build_envelope_bad_json_is_invalid_arg() {
        let rc = build_envelope(1, "steer", Some("{not-json")).unwrap_err();
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn parse_role_table() {
        assert_eq!(parse_role("Client").unwrap(), PeerRole::Consumer);
        assert_eq!(parse_role("Viewer").unwrap(), PeerRole::Consumer);
        assert_eq!(parse_role("Remote").unwrap(), PeerRole::Remote);
        assert_eq!(parse_role("Host").unwrap(), PeerRole::Host);
        assert!(parse_role("God").is_err());
    }

    #[test]
    fn copy_out_str_rejects_truncation() {
        let mut buf = [0u8; 4];
        let rc = copy_out_str("abcdefgh", buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        let rc = copy_out_str("abc", buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        assert_eq!(
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
                .to_str()
                .unwrap(),
            "abc"
        );
    }

    #[test]
    fn last_error_roundtrip() {
        // 全局状态跨测试竞争: 设独有标记后立即读（同 link-c 接受窗口）
        set_last_error("client-c errors test error");
        let mut buf = [0u8; 64];
        let rc = last_error_impl(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        // 内容竞争容忍: 读得回合法 C 串即达标（link-c 同纪律，独有标记可能被并发测试覆写）
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_str()
            .expect("valid utf8");
        assert!(!s.is_empty());
    }

    #[test]
    fn last_error_null_buf_fails() {
        assert_eq!(
            last_error_impl(ptr::null_mut(), 64),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        let mut buf = [0u8; 4];
        assert_eq!(
            last_error_impl(buf.as_mut_ptr() as *mut c_char, 0),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }
}
