//! Error codes / last_error / handle-level error slots (K3) / error_t / strerror table / pure mapping functions.
//!
//! K3 contract (batch 1b §2): error text moved from the process global into each handle (one
//! `Mutex<Option<HandleError>>` each for session/control/consumer); `mediaservo_client_last_error` ⊘ stays global for one period —
//! ruling = minimal-change form: new code paths always **write the handle slot + the global fallback** ([`HandleErr::note`] is the single write point).

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

use mediaservo_client::error::ClientError;
use mediaservo_common::protocol::{ControlEnvelope, PeerRole};

// -- Error codes (0 = ok, <0 = error; MEDIASERVO_CLIENT_ERR_*, prefixed per D241) --
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

/// Global last-error message (read via ⊘ `mediaservo_client_last_error`; the K3 dual-write keeps the global alive for one period).
pub(crate) static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub(crate) fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

// -- K3: handle-level error slots --

/// One handle-level error record (storage form of the error_t wire shape + its text source).
pub(crate) struct HandleError {
    pub code: c_int,
    pub wire: u32,
    pub retryable: bool,
    pub msg: String,
}

/// Handle error slot (one each for session/control/consumer).
pub(crate) type ErrSlot = std::sync::Mutex<Option<HandleError>>;

/// The error_t wire shape (C out-param; same struct_size forward-compatibility discipline as the config structs).
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
#[repr(C)]
pub struct mediaservo_client_error_t {
    pub struct_size: usize,
    pub code: c_int,
    pub wire_code: u32,
    pub retryable: u8,
}

/// Known minimum size of error_t (currently all fields; older callers report the old value when it evolves).
pub(crate) const CLIENT_ERROR_T_SIZE: usize = size_of::<mediaservo_client_error_t>();

/// Unified entry point for handle-error writes (trait shared by the session/control/consumer forms).
pub(crate) trait HandleErr {
    fn err_slot(&self) -> &ErrSlot;

    /// The single write point: handle slot + global LAST_ERROR dual write (K3 ruling form).
    fn note(&self, err: HandleError) {
        set_last_error(err.msg.clone());
        if let Ok(mut g) = self.err_slot().lock() {
            *g = Some(err);
        }
    }

    /// ClientError → slot (code = exhaustive error_code mapping; wire/retryable machine-readable bits passed through).
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

    /// Put any code into the slot (off-table codes go through this gate; inventing new negative codes is forbidden — reuse the existing value space).
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

    /// panic safety net (end of catch_unwind; invoked via ffi_catch while the handle is still valid).
    fn fail_panic(&self, name: &str) {
        self.fail_internal(&format!("{name}: panic"));
    }
}

/// Error write for handle-less free functions (global only — the login/list_rooms/strerror/version families).
pub(crate) fn fail_global(msg: impl Into<String>, code: c_int) -> c_int {
    set_last_error(msg);
    code
}

/// Extract a C string (null → None). Invalid UTF-8 → Err.
pub(crate) fn cstr<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().map(Some).map_err(|_| ())
}

/// Copy a string into the caller's buffer (NUL-terminated). Insufficient cap → INVALID_ARG (never writes a partial string).
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

/// needed overflow-feedback contract (shared by login/list_rooms/wait_video/consumer_id/consumer_stats/
/// producer_ids/video_stats): **needed is written first** (it has a value in both the overflow and success states — the caller
/// uses it to pick the retry size), then the copy. `needed` may be NULL (no feedback wanted). Pure buffer logic, pinned by unit tests.
pub(crate) fn copy_out_needed(
    name: &str,
    s: &str,
    buf: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    let need = s.len() + 1;
    if !needed.is_null() {
        // SAFETY: already checked non-null; points at caller-owned usize storage.
        unsafe { *needed = need };
    }
    let rc = copy_out_str(s, buf, cap);
    if rc != MEDIASERVO_OK {
        set_last_error(format!("{name}: buffer too small, need {need} bytes"));
    }
    rc
}

/// Truncating text copy (last_error-family semantics: read as much as fits, always OK — a short buffer truncates without erroring).
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

/// [`ClientError`] → C error code (exhaustive match — new variants are forced to be classified at compile time).
pub(crate) fn error_code(e: &ClientError) -> c_int {
    match e {
        ClientError::Login(_) => MEDIASERVO_CLIENT_ERR_LOGIN,
        ClientError::InvalidCredentials
        | ClientError::AuthRejected { .. }
        | ClientError::RestRejected { .. } => MEDIASERVO_CLIENT_ERR_UNAUTHORIZED,
        ClientError::ControlDenied(_) => MEDIASERVO_CLIENT_ERR_DENIED,
        // StreamNotFound maps into the TIMEOUT code domain (§0 ruling: no new ABI code — the message already contains
        // "stream not found: <label>"; the C-side semantics are carried by the message)
        ClientError::Timeout { .. } | ClientError::StreamNotFound { .. } => {
            MEDIASERVO_CLIENT_ERR_TIMEOUT
        }
        ClientError::Signal(_) => MEDIASERVO_CLIENT_ERR_SIGNAL,
        ClientError::ProtocolUnsupported(_) | ClientError::ProtocolTooLow { .. } => {
            MEDIASERVO_CLIENT_ERR_PROTOCOL
        }
        ClientError::MalformedResponse(_) | ClientError::UnsupportedScheme(_) => {
            MEDIASERVO_CLIENT_ERR_MALFORMED
        }
        ClientError::InvalidState(_) => MEDIASERVO_CLIENT_ERR_STATE,
        // Generic server carrier (4012/4101/4003/4010/4011 were already split by from_wire_error into
        // ControlDenied/ProtocolUnsupported/AuthRejected and never reach here) + the fallback family.
        ClientError::Server { .. } | ClientError::WebRtc(_) | ClientError::Io(_) => {
            MEDIASERVO_CLIENT_ERR_INTERNAL
        }
    }
}

/// code → static text (the `mediaservo_client_strerror` table source).
///
/// Inventing new wording is forbidden: every entry comes from the existing sources — the union of the `ClientError` Display templates (error.rs) and
/// the client.h error-code comments, with dynamic parameter slots stripped. Unknown code → fixed fallback string.
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

/// C-side role string → PeerRole.
/// Client/Consumer → Consumer (the cockpit consumption form; same value as the live basic.rs);
/// Viewer → Consumer (consumption-permission differences are adjudicated by the server account can_control gate; locally the same form);
/// Remote → Remote。
pub(crate) fn parse_role(s: &str) -> Result<PeerRole, ()> {
    match s {
        "Client" | "Consumer" | "Viewer" => Ok(PeerRole::Consumer),
        "Remote" => Ok(PeerRole::Remote),
        "Host" => Ok(PeerRole::Host),
        _ => Err(()),
    }
}

/// struct_size forward-compatibility check (R3).
pub(crate) fn check_struct_size(actual: usize, min: usize, fn_name: &str) -> Result<(), c_int> {
    if actual < min {
        set_last_error(format!(
            "{fn_name}: cfg.struct_size {actual} < {min} (rebuild with current header)"
        ));
        return Err(MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }
    Ok(())
}

/// Envelope construction (NEVER string concatenation — serde serialization is handled by ControlChannel::send_envelope).
/// payload_json None/"" → Null; invalid JSON → INVALID_ARG. Pure function, pinned by unit tests.
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
        sig: None, // the C surface does not yet produce emergency-stop signatures (the W2-C follow-up lands with hmac_key; see the lib.rs ClientConfig note)
    })
}

/// last_error implementation (⊘ global form; shared by mediaservo_client_last_error and the unit tests).
pub(crate) fn last_error_impl(buf: *mut c_char, len: usize) -> c_int {
    let msg = LAST_ERROR.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    copy_out_text(&msg, buf, len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_link::LinkError;
    use std::ptr;

    // -- (a) ClientError → error code exhaustive mapping table --
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

    // -- strerror table (full code coverage + fallback) --
    #[test]
    fn strerror_covers_all_codes_and_falls_back() {
        let codes = [-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, 0];
        for c in codes {
            assert!(!strerror_text(c).is_empty(), "code {c} has no text");
        }
        // Stable for the same code (the table source is static, not random/dynamic).
        assert_eq!(strerror_text(-9), strerror_text(MEDIASERVO_CLIENT_ERR_STATE));
        assert_eq!(strerror_text(12345), "unknown error code");
        assert_eq!(strerror_text(1), "unknown error code");
    }

    // -- The two states of the needed overflow contract (needed written first, with values in both overflow and success states) --
    #[test]
    fn copy_out_needed_two_states() {
        let mut buf = [0u8; 8];
        let mut need = 0usize;
        // Overflow state: needed = required byte count (including NUL), no partial write, rc=INVALID_ARG.
        let rc = copy_out_needed(
            "t",
            "abcdefghij",
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut need,
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        assert_eq!(need, 11);
        // Success state: needed = actual length, buffer NUL-terminated.
        let rc = copy_out_needed("t", "abc", buf.as_mut_ptr() as *mut c_char, buf.len(), &mut need);
        assert_eq!(rc, MEDIASERVO_OK);
        assert_eq!(need, 4);
        assert_eq!(
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap(),
            "abc"
        );
        // needed may be NULL.
        assert_eq!(
            copy_out_needed("t", "ab", buf.as_mut_ptr() as *mut c_char, buf.len(), ptr::null_mut()),
            MEDIASERVO_OK
        );
    }

    // -- Envelope construction pure functions --
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
        // The wire shape = ControlEnvelope serde (seq/cmd/payload)
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
        // Global state races across tests: set a unique marker and read immediately (same acceptance window as link-c)
        set_last_error("client-c errors test error");
        let mut buf = [0u8; 64];
        let rc = last_error_impl(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        // Content-race tolerance: getting back a valid C string is the bar (same discipline as link-c; a unique marker may be overwritten by concurrent tests)
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
        // Truncation semantics (last_error family): a short buffer does not error; read len-1 bytes + NUL.
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
