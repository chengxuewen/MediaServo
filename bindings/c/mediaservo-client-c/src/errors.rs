//! 错误码 / last_error / 句柄级错误槽（K3）/ error_t / strerror 表 / 纯映射函数。
//!
//! K3 合同（批1b §2）：错误文本从进程全局迁至句柄内（session/control/consumer 各一
//! `Mutex<Option<HandleError>>`）；`mediaservo_client_last_error` ⊘ 保留全局一周期——
//! 裁决=最小改动形：新代码路径一律**句柄槽 + 全局兜底双写**（[`HandleErr::note`] 单点）。

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

/// 全局最近错误信息（⊘ `mediaservo_client_last_error` 读取；K3 双写兜底保一周期）。
pub(crate) static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub(crate) fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

// ── K3: 句柄级错误槽 ──

/// 一次句柄级错误记录（error_t 线形 + 文本源的存储形）。
pub(crate) struct HandleError {
    pub code: c_int,
    pub wire: u32,
    pub retryable: bool,
    pub msg: String,
}

/// 句柄错误槽（session/control/consumer 各持一个）。
pub(crate) type ErrSlot = std::sync::Mutex<Option<HandleError>>;

/// error_t 线形（C 出参；struct_size 前向兼容纪律同 config 结构）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
#[repr(C)]
pub struct mediaservo_client_error_t {
    pub struct_size: usize,
    pub code: c_int,
    pub wire_code: u32,
    pub retryable: u8,
}

/// error_t 的已知最小尺寸（当前 = 全字段；演进时旧调用方报旧值）。
pub(crate) const CLIENT_ERROR_T_SIZE: usize = size_of::<mediaservo_client_error_t>();

/// 句柄错误写入的统一入口（trait——session/control/consumer 三形共享）。
pub(crate) trait HandleErr {
    fn err_slot(&self) -> &ErrSlot;

    /// 唯一写点：句柄槽 + 全局 LAST_ERROR 双写（K3 裁决形）。
    fn note(&self, err: HandleError) {
        set_last_error(err.msg.clone());
        if let Ok(mut g) = self.err_slot().lock() {
            *g = Some(err);
        }
    }

    /// ClientError → 槽（code=error_code 穷尽映射；wire/retryable 机读位透传）。
    fn fail_client(&self, ctx: &str, e: &ClientError) -> c_int {
        let code = error_code(e);
        self.note(HandleError {
            code,
            wire: e.wire_code().unwrap_or(0),
            retryable: e.is_retryable(),
            msg: format!("{ctx}: {e}"),
        });
        code
    }

    /// 任意码入槽（表外码走此门，禁发明新负数码——复用既有值域）。
    fn fail(&self, code: c_int, msg: &str) -> c_int {
        self.note(HandleError { code, wire: 0, retryable: false, msg: msg.to_string() });
        code
    }

    fn fail_arg(&self, msg: &str) -> c_int {
        self.fail(MEDIASERVO_CLIENT_ERR_INVALID_ARG, msg)
    }

    fn fail_state(&self, msg: &str) -> c_int {
        self.fail(MEDIASERVO_CLIENT_ERR_STATE, msg)
    }

    fn fail_internal(&self, msg: &str) -> c_int {
        self.fail(MEDIASERVO_CLIENT_ERR_INTERNAL, msg)
    }

    /// panic 兜底（catch_unwind 尾；句柄仍有效时经 ffi_catch 调用）。
    fn fail_panic(&self, name: &str) {
        self.fail_internal(&format!("{name}: panic"));
    }
}

/// 无句柄自由函数的错误写入（仅全局——login/list_rooms/strerror/version 族）。
pub(crate) fn fail_global(msg: impl Into<String>, code: c_int) -> c_int {
    set_last_error(msg);
    code
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

/// needed 溢出反馈合同（login/list_rooms/wait_video/consumer_id/consumer_stats/
/// producer_ids/video_stats 共用）：**needed 先写**（溢出/成功两态都有值——调用方
/// 凭此决定重试尺寸），再拷贝。`needed` 可 NULL（不需要反馈）。纯缓冲逻辑，单测钉。
pub(crate) fn copy_out_needed(
    name: &str,
    s: &str,
    buf: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    let need = s.len() + 1;
    if !needed.is_null() {
        // SAFETY: 已判非 null；指向调用方 usize 存储。
        unsafe { *needed = need };
    }
    let rc = copy_out_str(s, buf, cap);
    if rc != MEDIASERVO_OK {
        set_last_error(format!("{name}: buffer too small, need {need} bytes"));
    }
    rc
}

/// 截断式文本拷贝（last_error 族语义：读多少算多少，恒 OK——缓冲不足截断不报错）。
pub(crate) fn copy_out_text(msg: &str, buf: *mut c_char, len: usize) -> c_int {
    if buf.is_null() || len == 0 {
        return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
    }
    let bytes = msg.as_bytes();
    let n = bytes.len().min(len - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    MEDIASERVO_OK
}

/// [`ClientError`] → C 错误码（穷尽 match——新变体编译期强制归类）。
pub(crate) fn error_code(e: &ClientError) -> c_int {
    match e {
        ClientError::Login(_) => MEDIASERVO_CLIENT_ERR_LOGIN,
        ClientError::InvalidCredentials
        | ClientError::AuthRejected { .. }
        | ClientError::RestRejected { .. } => MEDIASERVO_CLIENT_ERR_UNAUTHORIZED,
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

/// code → 静态文案（`mediaservo_client_strerror` 表源）。
///
/// 禁新文案发明：每条取自既有真源——`ClientError` Display 模板（error.rs）与
/// client.h 错误码注释的并集，剥去动态参数位。未知码 → 固定兜底串。
#[must_use]
pub(crate) fn strerror_text(code: c_int) -> &'static str {
    match code {
        MEDIASERVO_OK => "ok",
        MEDIASERVO_CLIENT_ERR_INVALID_ARG => "invalid argument",
        MEDIASERVO_CLIENT_ERR_LOGIN => "login failed",
        MEDIASERVO_CLIENT_ERR_UNAUTHORIZED => "invalid credentials / auth rejected / rest rejected",
        MEDIASERVO_CLIENT_ERR_DENIED => "control denied (4012)",
        MEDIASERVO_CLIENT_ERR_TIMEOUT => "timed out waiting",
        MEDIASERVO_CLIENT_ERR_SIGNAL => "signal error",
        MEDIASERVO_CLIENT_ERR_PROTOCOL => "negotiated protocol rejected (4101 / too low)",
        MEDIASERVO_CLIENT_ERR_MALFORMED => "malformed response / unsupported scheme",
        MEDIASERVO_CLIENT_ERR_STATE => "invalid state: closed",
        MEDIASERVO_CLIENT_ERR_INTERNAL => "internal error",
        _ => "unknown error code",
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
        sig: None, // C 面暂不产急停签名（W2-C 增票与 hmac_key 同批；见 lib.rs ClientConfig 注）
    })
}

/// last_error 实现（⊘ 全局形，mediaservo_client_last_error 与单测共用）。
pub(crate) fn last_error_impl(buf: *mut c_char, len: usize) -> c_int {
    let msg = LAST_ERROR.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    copy_out_text(&msg, buf, len)
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
            (ClientError::InvalidCredentials, MEDIASERVO_CLIENT_ERR_UNAUTHORIZED),
            (
                ClientError::AuthRejected { code: 4010, message: "x".into() },
                MEDIASERVO_CLIENT_ERR_UNAUTHORIZED,
            ),
            (
                ClientError::RestRejected { code: 401, message: "x".into() },
                MEDIASERVO_CLIENT_ERR_UNAUTHORIZED,
            ),
            (ClientError::ControlDenied("x".into()), MEDIASERVO_CLIENT_ERR_DENIED),
            (ClientError::Timeout { what: "x" }, MEDIASERVO_CLIENT_ERR_TIMEOUT),
            (ClientError::Signal(LinkError::Signal("x".into())), MEDIASERVO_CLIENT_ERR_SIGNAL),
            (ClientError::ProtocolUnsupported("x".into()), MEDIASERVO_CLIENT_ERR_PROTOCOL),
            (ClientError::ProtocolTooLow { need: 2, got: 1 }, MEDIASERVO_CLIENT_ERR_PROTOCOL),
            (ClientError::MalformedResponse("x".into()), MEDIASERVO_CLIENT_ERR_MALFORMED),
            (ClientError::UnsupportedScheme("https".into()), MEDIASERVO_CLIENT_ERR_MALFORMED),
            (ClientError::InvalidState("x".into()), MEDIASERVO_CLIENT_ERR_STATE),
            (
                ClientError::Server { code: 5000, message: "x".into() },
                MEDIASERVO_CLIENT_ERR_INTERNAL,
            ),
            (ClientError::WebRtc("x".into()), MEDIASERVO_CLIENT_ERR_INTERNAL),
            (ClientError::Io(std::io::Error::other("x")), MEDIASERVO_CLIENT_ERR_INTERNAL),
        ];
        for (e, want) in cases {
            assert_eq!(error_code(&e), want, "variant: {e:?}");
        }
    }

    // ── strerror 表（全码覆盖 + 兜底）──
    #[test]
    fn strerror_covers_all_codes_and_falls_back() {
        let codes = [-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, 0];
        for c in codes {
            assert!(!strerror_text(c).is_empty(), "code {c} 无文案");
        }
        // 同码稳定（表源静态，非随机/动态）。
        assert_eq!(strerror_text(-9), strerror_text(MEDIASERVO_CLIENT_ERR_STATE));
        assert_eq!(strerror_text(12345), "unknown error code");
        assert_eq!(strerror_text(1), "unknown error code");
    }

    // ── needed 溢出合同两态（先写 needed，溢出/成功都有值）──
    #[test]
    fn copy_out_needed_two_states() {
        let mut buf = [0u8; 8];
        let mut need = 0usize;
        // 溢出态：needed = 必需字节数（含 NUL），不写半截，rc=INVALID_ARG。
        let rc = copy_out_needed(
            "t",
            "abcdefghij",
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut need,
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        assert_eq!(need, 11);
        // 成功态：needed = 实际长度，缓冲 NUL 结尾。
        let rc = copy_out_needed("t", "abc", buf.as_mut_ptr() as *mut c_char, buf.len(), &mut need);
        assert_eq!(rc, MEDIASERVO_OK);
        assert_eq!(need, 4);
        assert_eq!(
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap(),
            "abc"
        );
        // needed 可 NULL。
        assert_eq!(
            copy_out_needed("t", "ab", buf.as_mut_ptr() as *mut c_char, buf.len(), ptr::null_mut()),
            MEDIASERVO_OK
        );
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
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap(),
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
        let s =
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().expect("valid utf8");
        assert!(!s.is_empty());
    }

    #[test]
    fn last_error_null_buf_fails() {
        assert_eq!(last_error_impl(ptr::null_mut(), 64), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        let mut buf = [0u8; 4];
        assert_eq!(
            last_error_impl(buf.as_mut_ptr() as *mut c_char, 0),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }

    #[test]
    fn copy_out_text_truncates_not_rejects() {
        let mut buf = [0u8; 4];
        // 截断语义（last_error 族）：短缓冲不报错，读 len-1 字节 + NUL。
        assert_eq!(
            copy_out_text("abcdefgh", buf.as_mut_ptr() as *mut c_char, buf.len()),
            MEDIASERVO_OK
        );
        assert_eq!(
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap(),
            "abc"
        );
    }
}
