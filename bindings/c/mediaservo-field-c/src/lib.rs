//! MediaServo Field C ABI — push plane (vehicle-side SDK consumption).
//!
//! Contract §7 (D109/D240/D241): opaque handles + int error codes + callbacks.
//! Synchronous blocking style (internally a shared multi_thread runtime) — simplifies integration for vehicle embedded scenarios.
//!
//! # Lifecycle contract (audit R2)
//! - Each handle is single-thread-owner; any API call after close is UB (except idempotent close).
//! - close = set the closed flag → release the session → drop the runtime (cancels background tasks).
//!
//! # C ABI surface (cbindgen exports, stable within MAJOR)
//! ```c
//! typedef struct mediaservo_field_push_t mediaservo_field_push_t;   /* opaque */
//! typedef int mediaservo_err_t;                              /* 0=ok, <0=error */
//!
//! mediaservo_err_t mediaservo_field_push_connect(const mediaservo_push_config_t* cfg, mediaservo_field_push_t** out);
//! mediaservo_err_t mediaservo_field_push_publish_video(mediaservo_field_push_t* s, mediaservo_track_id_t* out_track);
//! mediaservo_err_t mediaservo_field_push_start_video_frames(mediaservo_field_push_t* s);
//! void     mediaservo_field_push_stop_video_frames(mediaservo_field_push_t* s);
//! mediaservo_err_t mediaservo_field_push_close(mediaservo_field_push_t* s);
//! mediaservo_err_t mediaservo_field_last_error(char* buf, size_t len);   /* mediaservo_last_error is a deprecated alias */
//! mediaservo_err_t mediaservo_field_version(char* buf, size_t len);
//! ```

#![allow(clippy::not_unsafe_ptr_arg_deref)] // C ABI facade (same-shape precedent as deck-c/link-c)
#![allow(non_camel_case_types)] // C-visible *_t type-name mirrors
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use mediaservo_field::{PublishOptions, PushConfig, PushSession};

/// Error codes (<0; 0 = ok). MEDIASERVO_FIELD_ERR_* is the current prefix; MEDIASERVO_ERR_* are retained compatibility aliases.
pub const MEDIASERVO_OK: c_int = 0;
pub const MEDIASERVO_ERR_INVALID_ARG: c_int = -1;
pub const MEDIASERVO_ERR_CONNECT: c_int = -2;
pub const MEDIASERVO_ERR_PUBLISH: c_int = -3;
pub const MEDIASERVO_ERR_STATE: c_int = -4;
pub const MEDIASERVO_ERR_INTERNAL: c_int = -5;
pub const MEDIASERVO_FIELD_ERR_INVALID_ARG: c_int = -1;
pub const MEDIASERVO_FIELD_ERR_CONNECT: c_int = -2;
pub const MEDIASERVO_FIELD_ERR_PUBLISH: c_int = -3;
pub const MEDIASERVO_FIELD_ERR_STATE: c_int = -4;
pub const MEDIASERVO_FIELD_ERR_INTERNAL: c_int = -5;

/// Global last-error message (read via mediaservo_field_last_error).
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

/// Push configuration (C struct — one-to-one mapping with PushConfig).
///
/// First field `struct_size` (audit R3): the caller fills `sizeof(mediaservo_push_config_t)`;
/// the library validates `>= sizeof(known struct)` and ignores extra length — struct evolution does not break binary compatibility.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception: mediaservo_* prefix)
#[repr(C)]
pub struct mediaservo_push_config_t {
    pub struct_size: usize,
    /// Signaling WS address (e.g. "ws://host:9800/ws").
    pub url: *const c_char,
    /// PSK authentication key.
    pub psk: *const c_char,
    /// Room ID.
    pub room: *const c_char,
    /// Video width (default 1280).
    pub width: u32,
    /// Video height (default 720).
    pub height: u32,
    /// Framerate (default 30).
    pub framerate: u32,
    /// Encoder bitrate kbps (default 2000).
    pub bitrate_kbps: u32,
    /// Keyframe interval in seconds (default 2).
    pub keyframe_interval: u64,
}

/// Known prefix size of the C struct (the minimum legal value across version evolution).
pub const MEDIASERVO_PUSH_CONFIG_MIN_SIZE: usize = size_of::<mediaservo_push_config_t>();

impl Default for mediaservo_push_config_t {
    fn default() -> Self {
        Self {
            struct_size: MEDIASERVO_PUSH_CONFIG_MIN_SIZE,
            url: ptr::null(),
            psk: ptr::null(),
            room: ptr::null(),
            width: 1280,
            height: 720,
            framerate: 30,
            bitrate_kbps: 2000,
            keyframe_interval: 2,
        }
    }
}

/// Push session opaque handle.
pub struct mediaservo_field_push_t {
    inner: std::sync::Mutex<Option<PushSession>>,
    cfg: PushConfig,
    /// Shared multi_thread runtime (audit R1): the session background task (WS read loop)
    /// lives on this runtime; all C calls block_on the same instance — a per-call runtime
    /// would cancel the spawned task on return, killing the session.
    rt: tokio::runtime::Runtime,
    /// Closed flag (close idempotence + entry validation).
    closed: AtomicBool,
}

// SAFETY: the handle holds Mutex<Option<PushSession>> (thread-safe) + an immutable cfg + a Send runtime.
unsafe impl Send for mediaservo_field_push_t {}
// SAFETY: all methods access the inner session serialized through the Mutex.
unsafe impl Sync for mediaservo_field_push_t {}

// -- Internal helpers --

/// Extract a C string (null → None). Invalid UTF-8 → error.
fn cstr<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().map(Some).map_err(|_| ())
}

/// Create a shared multi_thread runtime (one per handle).
fn new_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("field-c runtime")
}

// ── C ABI ──

/// Connect signaling and create the push session (blocking).
///
/// `cfg` must not be null; `url/psk/room` are required; `cfg.struct_size` must be
/// `>= sizeof(mediaservo_push_config_t)` (callers compiled against an older header get an explicit error).
/// On success `*out` points to the new handle (the caller owns `mediaservo_field_push_close`).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_field_push_connect(
    cfg: *const mediaservo_push_config_t,
    out: *mut *mut mediaservo_field_push_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if cfg.is_null() || out.is_null() {
            set_last_error("mediaservo_field_push_connect: null cfg/out");
            return MEDIASERVO_FIELD_ERR_INVALID_ARG;
        }
        let cfg_ref = unsafe { &*cfg };
        if cfg_ref.struct_size < MEDIASERVO_PUSH_CONFIG_MIN_SIZE {
            set_last_error(format!(
                "mediaservo_field_push_connect: cfg.struct_size {} < {} (rebuild with current header)",
                cfg_ref.struct_size,
                MEDIASERVO_PUSH_CONFIG_MIN_SIZE
            ));
            return MEDIASERVO_FIELD_ERR_INVALID_ARG;
        }
        let (url, psk, room) = match (
            cstr(cfg_ref.url),
            cstr(cfg_ref.psk),
            cstr(cfg_ref.room),
        ) {
            (Ok(Some(u)), Ok(Some(p)), Ok(Some(r))) => (u, p, r),
            (Ok(None), _, _) => {
                set_last_error("mediaservo_field_push_connect: url required");
                return MEDIASERVO_FIELD_ERR_INVALID_ARG;
            }
            (_, Ok(None), _) => {
                set_last_error("mediaservo_field_push_connect: psk required");
                return MEDIASERVO_FIELD_ERR_INVALID_ARG;
            }
            (_, _, Ok(None)) => {
                set_last_error("mediaservo_field_push_connect: room required");
                return MEDIASERVO_FIELD_ERR_INVALID_ARG;
            }
            _ => {
                set_last_error("mediaservo_field_push_connect: invalid UTF-8 in config");
                return MEDIASERVO_FIELD_ERR_INVALID_ARG;
            }
        };

        let mut push_cfg = PushConfig::new(url, psk, room);
        if cfg_ref.width > 0 {
            push_cfg.width = cfg_ref.width;
        }
        if cfg_ref.height > 0 {
            push_cfg.height = cfg_ref.height;
        }
        if cfg_ref.framerate > 0 {
            push_cfg.framerate = cfg_ref.framerate;
        }
        if cfg_ref.bitrate_kbps > 0 {
            push_cfg.bitrate_kbps = cfg_ref.bitrate_kbps;
        }
        if cfg_ref.keyframe_interval > 0 {
            push_cfg.keyframe_interval = cfg_ref.keyframe_interval;
        }

        let rt = new_runtime();
        match rt.block_on(PushSession::connect(push_cfg.clone())) {
            Ok((session, _events)) => {
                let handle = Box::new(mediaservo_field_push_t {
                    inner: std::sync::Mutex::new(Some(session)),
                    cfg: push_cfg,
                    rt,
                    closed: AtomicBool::new(false),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_field_push_connect: {e}"));
                MEDIASERVO_FIELD_ERR_CONNECT
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_field_push_connect: panic");
        MEDIASERVO_FIELD_ERR_INTERNAL
    })
}

/// Publish the video track (blocking negotiation; on success writes the track id string into the `out_track` buffer).
///
/// `out_track` needs at least `out_track_len` bytes (track ids are short; 64 is enough).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_field_push_publish_video(
    s: *mut mediaservo_field_push_t,
    out_track: *mut c_char,
    out_track_len: usize,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() || out_track.is_null() {
            set_last_error("mediaservo_field_push_publish_video: null handle/out_track");
            return MEDIASERVO_FIELD_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*s };
        if handle.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_field_push_publish_video: session closed");
            return MEDIASERVO_FIELD_ERR_STATE;
        }
        let mut guard = match handle.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("mediaservo_field_push_publish_video: lock poisoned");
                return MEDIASERVO_FIELD_ERR_INTERNAL;
            }
        };
        let Some(session) = guard.as_mut() else {
            set_last_error("mediaservo_field_push_publish_video: session closed");
            return MEDIASERVO_FIELD_ERR_STATE;
        };

        let opts = PublishOptions::default();
        match handle.rt.block_on(session.publish_video(&handle.cfg, &opts)) {
            Ok(track_id) => {
                let bytes = track_id.as_bytes();
                let n = bytes.len().min(out_track_len.saturating_sub(1));
                unsafe {
                    ptr::copy_nonoverlapping(bytes.as_ptr(), out_track as *mut u8, n);
                    *out_track.add(n) = 0;
                }
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_field_push_publish_video: {e}"));
                MEDIASERVO_FIELD_ERR_PUBLISH
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_field_push_publish_video: panic");
        MEDIASERVO_FIELD_ERR_INTERNAL
    })
}

/// Start video frame generation (squares + timestamp watermark; blocking, local start only).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_field_push_start_video_frames(
    s: *mut mediaservo_field_push_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            set_last_error("mediaservo_field_push_start_video_frames: null handle");
            return MEDIASERVO_FIELD_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*s };
        if handle.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_field_push_start_video_frames: session closed");
            return MEDIASERVO_FIELD_ERR_STATE;
        }
        let mut guard = match handle.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("mediaservo_field_push_start_video_frames: lock poisoned");
                return MEDIASERVO_FIELD_ERR_INTERNAL;
            }
        };
        let Some(session) = guard.as_mut() else {
            set_last_error("mediaservo_field_push_start_video_frames: session closed");
            return MEDIASERVO_FIELD_ERR_STATE;
        };
        // WebRtcTrackSink needs a tokio runtime context (Handle::try_current) —
        // the sync method provides it via shared-rt block_on (continuing audit R1).
        match handle.rt.block_on(async { session.start_video_frames(&handle.cfg) }) {
            Ok(()) => MEDIASERVO_OK,
            Err(e) => {
                set_last_error(format!("mediaservo_field_push_start_video_frames: {e}"));
                MEDIASERVO_FIELD_ERR_STATE
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_field_push_start_video_frames: panic");
        MEDIASERVO_FIELD_ERR_INTERNAL
    })
}

/// Stop video frame generation (idempotent).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_field_push_stop_video_frames(s: *mut mediaservo_field_push_t) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            return;
        }
        let handle = unsafe { &*s };
        if handle.closed.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(mut guard) = handle.inner.lock()
            && let Some(session) = guard.as_mut()
        {
            session.stop_video_frames();
        }
    }));
}

/// Close the push session and release the handle (idempotent).
///
/// Order (audit R2): set the closed flag → release the session (block_on close) →
/// drop the runtime (cancels leftover background tasks) → free the handle memory.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_field_push_close(s: *mut mediaservo_field_push_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(s) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent: already closed
        }
        let session = handle.inner.lock().ok().and_then(|mut g| g.take());
        if let Some(session) = session {
            match handle.rt.block_on(session.close()) {
                Ok(()) => MEDIASERVO_OK,
                Err(e) => {
                    set_last_error(format!("mediaservo_field_push_close: {e}"));
                    MEDIASERVO_FIELD_ERR_INTERNAL
                }
            }
        } else {
            MEDIASERVO_OK
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_field_push_close: panic");
        MEDIASERVO_FIELD_ERR_INTERNAL
    })
}

/// Details of the last error (thread-safe; returns an empty string when no error).
fn last_error_impl(buf: *mut c_char, len: usize) -> c_int {
    if buf.is_null() || len == 0 {
        return MEDIASERVO_FIELD_ERR_INVALID_ARG;
    }
    let msg = LAST_ERROR.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    let bytes = msg.as_bytes();
    let n = bytes.len().min(len - 1);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    MEDIASERVO_OK
}

/// Last error details (current prefix; recommended).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_field_last_error(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| last_error_impl(buf, len)))
        .unwrap_or(MEDIASERVO_FIELD_ERR_INTERNAL)
}

/// Last error details (deprecated alias, kept additive-only).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_last_error(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| last_error_impl(buf, len)))
        .unwrap_or(MEDIASERVO_FIELD_ERR_INTERNAL)
}

/// Version info (MAJOR.MINOR.PATCH — D241 soname semantics).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_field_version(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if buf.is_null() || len == 0 {
            return MEDIASERVO_FIELD_ERR_INVALID_ARG;
        }
        let ver = CString::new(env!("CARGO_PKG_VERSION")).unwrap_or_default();
        let bytes = ver.as_bytes();
        let n = bytes.len().min(len - 1);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
            *buf.add(n) = 0;
        }
        MEDIASERVO_OK
    }))
    .unwrap_or(MEDIASERVO_FIELD_ERR_INTERNAL)
}

#[cfg(test)]
mod tests {
    /// Tests in this file share the process-level static LAST_ERROR (C ABI semantics) — any test that walks an API error path
    /// writes it implicitly (null-handle forms xN), and parallel runs stomp on each other = flaky (caught by the 09-21 gate:
    /// the explicit-guard merged test was still overwritten by a racing null-handle test). All tests in this file serialize through this lock.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::*;

    /// Global last_error is process-level state; two tests running in parallel always race (caught by the 09-18 gate).
    /// Merged into one test that serially verifies both the new name and the alias paths.
    #[test]
    fn last_error_roundtrip_and_alias() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = [0u8; 64];
        set_last_error("test error");
        let rc = mediaservo_field_last_error(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        assert_eq!(
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap(),
            "test error"
        );
        set_last_error("alias error");
        let rc = mediaservo_last_error(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        assert_eq!(
            unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap(),
            "alias error"
        );
    }

    #[test]
    fn version_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = [0u8; 32];
        let rc = mediaservo_field_version(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(s.starts_with("0.1."), "version: {s}");
    }

    #[test]
    fn connect_null_cfg_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_field_push_connect(ptr::null(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_FIELD_ERR_INVALID_ARG);
    }

    #[test]
    fn connect_small_struct_size_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Caller compiled against an older header: struct_size too small → explicit error (R3)
        let cfg = mediaservo_push_config_t { struct_size: 1, ..Default::default() };
        let mut out: *mut mediaservo_field_push_t = ptr::null_mut();
        let rc = mediaservo_field_push_connect(&cfg, &mut out);
        assert_eq!(rc, MEDIASERVO_FIELD_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn connect_missing_required_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // struct_size valid but url/psk/room empty → required-field error
        let cfg = mediaservo_push_config_t::default();
        let mut out: *mut mediaservo_field_push_t = ptr::null_mut();
        let rc = mediaservo_field_push_connect(&cfg, &mut out);
        assert_eq!(rc, MEDIASERVO_FIELD_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn publish_null_handle_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut track = [0u8; 64];
        let rc = mediaservo_field_push_publish_video(
            ptr::null_mut(),
            track.as_mut_ptr() as *mut c_char,
            64,
        );
        assert_eq!(rc, MEDIASERVO_FIELD_ERR_INVALID_ARG);
    }

    #[test]
    fn close_null_is_ok() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mediaservo_field_push_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn start_video_frames_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_field_push_start_video_frames(ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_FIELD_ERR_INVALID_ARG);
    }

    #[test]
    fn stop_video_frames_null_noop() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        mediaservo_field_push_stop_video_frames(ptr::null_mut()); // void: passing = no crash
    }
}
