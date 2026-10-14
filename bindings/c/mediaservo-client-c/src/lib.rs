//! MediaServo client C ABI — cockpit/consumer-side SDK (login + SFU video consumption + control DataChannel).
//!
//! Contract §7 (D109/D240/D241): opaque handles + int error codes + callbacks.
//! Pattern baseline = bindings/c/mediaservo-link-c (same-shape lifecycle/struct_size/pump discipline).
//! All Rust-side capability comes from `mediaservo-client` v2 (this crate is FFI shaping only, zero business logic).
//!
//! S6 batch 1b (R1 rename + K3 handle errors + multi-consumer + state observation):
//! - full `mediaservo_client_*` prefix (zero aliases; the old `ms_client_*` is retired);
//! - dual error track = handle slots (`session_error`/`session_last_error`, machine-readable code/wire/retryable)
//!   + process-global ⊘ dual write for one period (K3 ruling: minimal-change form);
//! - multi-consumer `session_consume` (one frame pump per consumer, no cross coupling); the old `consume_video` ⊘ is
//!   behavior-remapped (wait(30s)+consume+into_receiver, the one-shot gate kept);
//! - `session_state`/`session_on_state` (cumulative registration, ConnectionState 0..3 wire values);
//! - cumulative `control_on_ack` + token (`control_off_ack` unregisters); the old replacement form is retired.
//!
//! # Lifecycle contract (inheriting link-c R2)
//! - Each handle is single-thread-owner; any API call after close is UB (idempotent close, same as link).
//! - session_close = set the closed flag → close the session (frame channels die with it) → join the video pump →
//!   join the state pump → invoke each registration's user_free → only then free.
//! - consumer handles close independently (close withdraws that consumer's receiver; the pump join is bounded ≤CONSUMER_POLL);
//!   session_close does not implicitly close consumers (same discipline as control).
//! - The control handle closes independently; each ack-pump recv_ack is bounded (1s), so the closed flag takes effect within 1s.
//! - Frame/ack/state callbacks fire only on their own pump threads; no lock is held during a callback; calling
//!   any mediaservo_client_* API (including close) from within a callback is forbidden — undefined behavior.
//! - JSON strings and frame.data pointers inside callbacks are valid only during the callback (copy if you need to keep them).
//! - user_free contract: each registration (on_state/consume/on_ack) carries an optional user_free,
//!   **invoked exactly once** — on_state at session_close; consume at consumer_close;
//!   on_ack at the pump reclamation round after off_ack or at control_close (entries never off_ack'ed are released at close).
//!
//! # runtime (the client form of audit R1)
//! `RoomSession::open_control` internally does `tokio::spawn(ack_consumer_pump)` (session.rs), and link's
//! `SignalClient::connect` spawns the WS read loop — background tasks must outlive any single
//! block_on, hence a process-level shared multi_thread runtime (OnceLock, the C28 pattern). Unlike
//! link-c's per-handle runtime: client background tasks co-own the session handle across calls,
//! so one shared instance naturally covers everything and saves threads.
//!
//! # Error mapping
//! All 14 `ClientError` variants → `MEDIASERVO_CLIENT_ERR_*` exhaustive match (pinned by unit tests;
//! adding a new variant turns the build red, so silent missed mappings are impossible). The machine-readable wire_code/retryable bits pass through
//! [`mediaservo_client_error_t`] (the C mirror of the K2 Rust surface).
//!
//! Raw pointer parameters are a signature requirement of the C ABI surface: every dereference point null-checks first and is wrapped in
//! catch_unwind, and the header documents the ownership/lifetime contracts — clippy's safe-API misuse model
//! does not apply to this crate (the same-family issues in link-c/field-c are existing debt; the workspace lint red at HEAD
//! is filed separately; this crate uses explicit allows to keep itself warning-free).
#![allow(clippy::not_unsafe_ptr_arg_deref)]

mod config;
mod consumer;
mod control;
mod errors;

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mediaservo_client::{ClientConfig, ConnectionState, Consumer, RoomSession, VideoFrame};
use tokio::sync::mpsc;
use tokio::time::timeout;

use config::{mediaservo_client_config_t, validate_session_cfg};
use errors::{
    CLIENT_ERROR_T_SIZE, ErrSlot, HandleErr, MEDIASERVO_CLIENT_ERR_INTERNAL,
    MEDIASERVO_CLIENT_ERR_INVALID_ARG, MEDIASERVO_OK, check_struct_size, copy_out_needed,
    copy_out_str, copy_out_text, cstr, error_code, fail_global, last_error_impl, set_last_error,
    strerror_text,
};

pub use config::{
    MEDIASERVO_CLIENT_CONFIG_MIN_SIZE, MEDIASERVO_CLIENT_LOGIN_CONFIG_MIN_SIZE,
    mediaservo_client_login_config_t,
};
pub use consumer::mediaservo_client_consumer_t;
pub use control::mediaservo_client_control_t;
pub use errors::{
    MEDIASERVO_CLIENT_ERR_DENIED, MEDIASERVO_CLIENT_ERR_LOGIN, MEDIASERVO_CLIENT_ERR_MALFORMED,
    MEDIASERVO_CLIENT_ERR_PROTOCOL, MEDIASERVO_CLIENT_ERR_SIGNAL, MEDIASERVO_CLIENT_ERR_TIMEOUT,
    MEDIASERVO_CLIENT_ERR_UNAUTHORIZED, mediaservo_client_error_t,
};

/// Upper bound for waiting on the room's first video producer (same value as basic.rs: the late-join NewProducer replay window).
const VIDEO_PRODUCER_WAIT: Duration = Duration::from_secs(30);
/// Ack-pump per-round recv_ack window (the upper bound for the closed flag to take effect = this value).
pub(crate) const ACK_POLL: Duration = Duration::from_secs(1);
/// ⊘ video-pump idle re-check window (polling granularity for closed / channel death).
const VIDEO_POLL: Duration = Duration::from_secs(2);

/// Process-level shared multi_thread runtime (see the runtime section in the module header).
/// Lazily installs the diagnostics subscriber (once, at the same moment as the runtime): default mediaservo=info —
/// "consume on_track arrival"/ICE state/client consume setup form the watershed evidence chain for zero-frame problems.
/// If the host already has a subscriber, try_init Err yields silently; the RUST_LOG env can override levels at any time.
/// MSRTC.log file target (set once via set_log_file; None = keep the current stdout behavior).
/// The host (TPHMI) calls this before the first runtime use — init_tracing reads it lazily.
static LOG_FILE_PATH: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

/// C ABI: set the tracing log file (MSRTC.log). Must be called before the first runtime use
/// (before any API such as the first login/consume); repeat calls are no-ops (OnceLock semantics).
/// Level filtering via the env MSRTC_LOG_LEVEL (default info); the libwebrtc noise layer
/// (mediaservo_webrtc) is independently capped at warn — set MSRTC_LOG_LEVEL=debug to open everything when deep-diving.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_set_log_file(path: *const std::ffi::c_char) -> std::ffi::c_int {
    if path.is_null() {
        return crate::errors::MEDIASERVO_CLIENT_ERR_INVALID_ARG;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(path) };
    let Ok(p) = cstr.to_str() else {
        return crate::errors::MEDIASERVO_CLIENT_ERR_INVALID_ARG;
    };
    match LOG_FILE_PATH.set(Some(std::path::PathBuf::from(p))) {
        Ok(()) => crate::errors::MEDIASERVO_OK,
        Err(_) => crate::errors::MEDIASERVO_CLIENT_ERR_STATE, // already set
    }
}

fn init_tracing() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        // Levels: MSRTC_LOG_LEVEL takes priority, default info; the webrtc_sys noise layer is independently raised to warn
        let level = std::env::var("MSRTC_LOG_LEVEL").unwrap_or_else(|_| "info".into());
        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(format!("mediaservo_webrtc=warn,mediaservo={level}"))
        });
        let file_writer: Option<std::fs::File> = LOG_FILE_PATH
            .get()
            .and_then(|o| o.as_ref())
            .and_then(|p| match std::fs::OpenOptions::new().create(true).append(true).open(p) {
                Ok(f) => Some(f),
                Err(e) => {
                    eprintln!(
                        "[msrtc] log file open failed ({}): {e} — fallback stdout",
                        p.display()
                    );
                    None
                }
            });
        match file_writer {
            Some(f) => {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .with_writer(std::sync::Mutex::new(f))
                    .with_ansi(false)
                    .try_init();
            }
            None => {
                let _ =
                    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();
            }
        }
    });
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        init_tracing();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("client-c runtime");
        // Warm up the libwebrtc C++ stack (root cause of flaky first connections): the first CreatePeerConnection
        // synchronously runs VAAPI DRM scans/PulseAudio/X11 display probing, measured at >15s — exactly enough to trigger link
        // heartbeat's 15s silent-death active disconnect → "first pull fails, retry succeeds". One background dry run spreads
        // the init cost into load time; failures are silent (the warm-up only fills caches, it is not a functional dependency).
        rt.spawn(async move {
            use mediaservo_client::engine::Engine as _;
            let engine = mediaservo_client::engine::SysEngine::new();
            match engine.create_pc().await {
                Ok(_pc) => tracing::debug!("libwebrtc warmup pc created"),
                Err(e) => tracing::warn!("libwebrtc warmup failed (non-fatal): {e}"),
            }
        });
        rt
    })
}

/// Unified catch tail for handle-carrying FFI: panic → the handle error slot (when the handle is non-null) + the global dual write.
///
/// The close family does not use this form (after Box::from_raw handle ownership is in doubt; the panic net writes only the global).
fn ffi_catch<T: HandleErr + ?Sized>(
    name: &'static str,
    h: *const T,
    body: impl FnOnce() -> c_int,
) -> c_int {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
        // SAFETY: dereferences only non-null pointers; a panic does not free the handle (except in the close family,
        // whose panic tail goes through ffi_global). Lock poisoning is absorbed by note's try-lock.
        match unsafe { h.as_ref() } {
            Some(x) => x.fail_panic(name),
            None => set_last_error(format!("{name}: panic")),
        }
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// Catch tail for handle-less free functions (global last_error only).
fn ffi_global(name: &'static str, body: impl FnOnce() -> c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
        set_last_error(format!("{name}: panic"));
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

// -- Callback types + struct mirrors --

/// Video frame callback (⊘ shared by consume_video and multi-consume; fires on each one's own pump thread;
/// the frame pointer is valid only inside the callback; the first parameter = the owning session handle).
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub type mediaservo_client_frame_cb =
    extern "C" fn(*mut mediaservo_client_session_t, *const mediaservo_client_frame_t, *mut c_void);

/// Ack callback (fires only on the ack pump thread; ack_json is valid only inside the callback).
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub type mediaservo_client_ack_cb =
    extern "C" fn(*mut mediaservo_client_control_t, *const c_char, *mut c_void);

/// Session connection-state callback (fires only on the state pump thread; see the header enum comment for state wire values).
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub type mediaservo_client_state_cb =
    extern "C" fn(*mut mediaservo_client_session_t, u8 /*state*/, *mut c_void);

/// user pointer deallocator (NULL = do not free; invoked exactly once per registration, timing in the module header).
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub type mediaservo_client_user_free = extern "C" fn(*mut c_void);

/// C-side frame description (naturally aligned, matching client.h; data is valid only inside the callback).
#[allow(non_camel_case_types)]
#[repr(C)]
pub struct mediaservo_client_frame_t {
    pub width: u32,
    pub height: u32,
    pub ts_us: i64,
    pub data: *const u8,
    pub len: usize,
}

// -- Free functions --

/// SDK version (MAJOR.MINOR.PATCH — D241 soname semantics).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_version(buf: *mut c_char, len: usize) -> c_int {
    ffi_global("mediaservo_client_version", || {
        if buf.is_null() || len == 0 {
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        copy_out_str(env!("CARGO_PKG_VERSION"), buf, len)
    })
}

/// Account login for a JWT (blocking; flat parameter form, the batch-1b signature upgrade — the old config struct form is retired).
///
/// Out-param = the jwt text (NUL-terminated). Same needed overflow contract as [`mediaservo_client_list_rooms`]
/// (needed is written first in both states; jwts are typically <2KiB, cap ≥ 4096 recommended).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_login(
    http_base: *const c_char,
    username: *const c_char,
    password: *const c_char,
    out_jwt: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_login";
    ffi_global(NAME, || {
        let (base, user, pass) = match (cstr(http_base), cstr(username), cstr(password)) {
            (Ok(Some(b)), Ok(Some(u)), Ok(Some(p)))
                if !b.is_empty() && !u.is_empty() && !p.is_empty() =>
            {
                (b, u, p)
            }
            _ => {
                return fail_global(
                    "mediaservo_client_login: http_base/username/password all required",
                    MEDIASERVO_CLIENT_ERR_INVALID_ARG,
                );
            }
        };
        if out_jwt.is_null() || cap == 0 {
            return fail_global(
                "mediaservo_client_login: null out_jwt or cap 0",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        }
        match runtime().block_on(mediaservo_client::login(base, user, pass)) {
            Ok(outcome) => copy_out_needed(NAME, &outcome.jwt, out_jwt, cap, needed),
            Err(e) => {
                set_last_error(format!("mediaservo_client_login: {e}"));
                error_code(&e)
            }
        }
    })
}

/// Exchange an API key for a short JWT (`POST {http_base}/api/auth/exchange`, blocking; a pre-session free function,
/// accountless-client-auth T2). On success out_jwt = a NUL-terminated JWT (fill it verbatim into
/// `mediaservo_client_config_t.jwt` — the credential source changes, the consumption surface does not). 401 (unknown key or
/// wrong secret; the server uses the identical message verbatim to prevent enumeration) → ERR_UNAUTHORIZED; same needed overflow contract as login.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_exchange(
    http_base: *const c_char,
    key_id: *const c_char,
    secret: *const c_char,
    out_jwt: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_exchange";
    ffi_global(NAME, || {
        let (base, kid, sec) = match (cstr(http_base), cstr(key_id), cstr(secret)) {
            (Ok(Some(b)), Ok(Some(k)), Ok(Some(s)))
                if !b.is_empty() && !k.is_empty() && !s.is_empty() =>
            {
                (b, k, s)
            }
            _ => {
                return fail_global(
                    "mediaservo_client_exchange: http_base/key_id/secret all required",
                    MEDIASERVO_CLIENT_ERR_INVALID_ARG,
                );
            }
        };
        if out_jwt.is_null() || cap == 0 {
            return fail_global(
                "mediaservo_client_exchange: null out_jwt or cap 0",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        }
        match runtime().block_on(mediaservo_client::exchange(base, kid, sec)) {
            Ok(outcome) => copy_out_needed(NAME, &outcome.jwt, out_jwt, cap, needed),
            Err(e) => {
                set_last_error(format!("mediaservo_client_exchange: {e}"));
                error_code(&e)
            }
        }
    })
}

/// PSK room discovery (`GET {http_base}/api/rooms` + `Authorization: Psk <psk>`,
/// blocking; the psk-discover path — the Legacy full view, the discovery surface for cross-room pulls). Non-2xx →
/// ERR_UNAUTHORIZED (details in last_error); out_json has the same shape as list_rooms.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_list_rooms_psk(
    http_base: *const c_char,
    psk: *const c_char,
    out_json: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_list_rooms_psk";
    ffi_global(NAME, || {
        let (base, psk) = match (cstr(http_base), cstr(psk)) {
            (Ok(Some(b)), Ok(Some(p))) if !b.is_empty() && !p.is_empty() => (b, p),
            _ => {
                return fail_global(
                    "mediaservo_client_list_rooms_psk: null/invalid http_base or psk",
                    MEDIASERVO_CLIENT_ERR_INVALID_ARG,
                );
            }
        };
        if out_json.is_null() || cap == 0 {
            return fail_global(
                "mediaservo_client_list_rooms_psk: null out_json or cap 0",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        }
        match runtime().block_on(mediaservo_client::list_rooms_psk(base, psk)) {
            Ok(rooms) => {
                let json = match serde_json::to_string(&rooms) {
                    Ok(j) => j,
                    Err(e) => {
                        return fail_global(
                            format!("mediaservo_client_list_rooms_psk: serialize: {e}"),
                            MEDIASERVO_CLIENT_ERR_MALFORMED,
                        );
                    }
                };
                copy_out_needed(NAME, &json, out_json, cap, needed)
            }
            Err(e) => {
                set_last_error(format!("mediaservo_client_list_rooms_psk: {e}"));
                error_code(&e)
            }
        }
    })
}

/// Room discovery (`GET {http_base}/api/rooms`, blocking; a **pre-session free function** — depends on no handle, F-T-8).
/// out_json = the JSON array `[{"room_id":..,"kind":..}]` (the server wire
/// passed through verbatim; this layer does not parse the content). Overflow contract (the corrected form of the producer_ids cap blind spot): when cap is insufficient,
/// `*needed` receives the required byte count (including NUL) and ERR_INVALID_ARG is returned; on success `*needed` = the actual
/// length. `needed` may be NULL (no feedback wanted). Non-2xx → ERR_UNAUTHORIZED (details in last_error).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_list_rooms(
    http_base: *const c_char,
    jwt: *const c_char,
    out_json: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_list_rooms";
    ffi_global(NAME, || {
        let (base, token) = match (cstr(http_base), cstr(jwt)) {
            (Ok(Some(b)), Ok(Some(t))) => (b, t),
            _ => {
                return fail_global(
                    "mediaservo_client_list_rooms: null/invalid http_base or jwt",
                    MEDIASERVO_CLIENT_ERR_INVALID_ARG,
                );
            }
        };
        if out_json.is_null() || cap == 0 {
            return fail_global(
                "mediaservo_client_list_rooms: null out_json or cap 0",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        }
        let rooms = match runtime().block_on(mediaservo_client::list_rooms(base, token)) {
            Ok(r) => r,
            Err(e) => {
                set_last_error(format!("mediaservo_client_list_rooms: {e}"));
                return error_code(&e);
            }
        };
        let json = match serde_json::to_string(&rooms) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(format!("mediaservo_client_list_rooms: serialize: {e}"));
                return MEDIASERVO_CLIENT_ERR_INTERNAL;
            }
        };
        copy_out_needed(NAME, &json, out_json, cap, needed)
    })
}

/// Error code → static text (table source = the ClientError Display templates + the header error-code comments,
/// no new wording invented; unknown codes get the fixed fallback string). Copy semantics = truncate without erroring.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_strerror(code: c_int, buf: *mut c_char, cap: usize) -> c_int {
    ffi_global("mediaservo_client_strerror", || copy_out_str(strerror_text(code), buf, cap))
}

/// Last error details (⊘ process-global form, kept for one period; after K3 new code uses the handle-level
/// [`mediaservo_client_session_last_error`] / [`mediaservo_client_error_t`]）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_last_error(buf: *mut c_char, len: usize) -> c_int {
    ffi_global("mediaservo_client_last_error", || last_error_impl(buf, len))
}

// -- Session --

/// One state-callback registration (cumulative form; user_free is invoked exactly once at session_close).
struct StateReg {
    cb: mediaservo_client_state_cb,
    user: *mut c_void,
    free: Option<mediaservo_client_user_free>,
}

/// Room session opaque handle.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub struct mediaservo_client_session_t {
    /// Taken out and consumed at close (the RoomSession::close(self) signature shape).
    session: std::sync::Mutex<Option<RoomSession>>,
    /// Snapshot at connect (constant for the negotiated session's lifetime).
    negotiated: u32,
    /// T4/F6 identity label (derived locally at create, static; read via session_identity).
    identity_label: String,
    closed: AtomicBool,
    /// ⊘ consume_video one-shot gate (set on first call; rolled back on failure paths).
    video_started: AtomicBool,
    /// ⊘ frame callback (read by the pump thread each cycle; the user pointer lives inside the handle and is never moved across threads — same shape as link-c).
    video_cb: std::sync::Mutex<Option<(mediaservo_client_frame_cb, *mut c_void)>>,
    video_pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// State-callback registry (cumulative; no unregister API by design — named distinctly from on_ack's token form).
    state_regs: std::sync::Mutex<Vec<StateReg>>,
    state_pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// K3 handle error slot.
    err: ErrSlot,
}

// SAFETY: inner Mutex<Option<RoomSession>> serializes access; pump threads read only through that lock and the atomic flags.
unsafe impl Send for mediaservo_client_session_t {}
// SAFETY: all field access goes through Mutex/AtomicBool.
unsafe impl Sync for mediaservo_client_session_t {}

impl HandleErr for mediaservo_client_session_t {
    fn err_slot(&self) -> &ErrSlot {
        &self.err
    }
}

/// ConnectionState → wire value (0..3, pinned by the header enum comment; a new Rust variant turns the build red = exhaustive match).
fn state_u8(st: ConnectionState) -> u8 {
    match st {
        ConnectionState::Disconnected => 0,
        ConnectionState::Connected => 1,
        ConnectionState::Reconnecting => 2,
        ConnectionState::Failed => 3,
    }
}

/// State pump thread: on watch changes → invoke all registered callbacks (the registry is re-read every cycle — registration takes effect immediately).
fn state_pump_loop(raw: usize) {
    let s = raw as *mut mediaservo_client_session_t;
    let h = unsafe { &*s };
    let rt = runtime();
    // The observation source = the session's watch subscription (missing/poisoned session = no source, the pump exits directly;
    // the registry is kept, and close does a uniform user_free pass).
    let rx = match h.session.lock() {
        Ok(g) => g.as_ref().map(|sess| sess.subscribe_connection_state()),
        Err(_) => None,
    };
    let Some(mut rx) = rx else { return };
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        match rt.block_on(rx.changed()) {
            Ok(()) => {
                let st = state_u8(*rx.borrow_and_update());
                let regs: Vec<(mediaservo_client_state_cb, *mut c_void)> = h
                    .state_regs
                    .lock()
                    .map(|g| g.iter().map(|r| (r.cb, r.user)).collect())
                    .unwrap_or_default();
                for (cb, user) in regs {
                    cb(s, st, user);
                }
            }
            Err(_) => break, // RoomSession drop → the watch sender dies (close converges normally)
        }
    }
}

/// ⊘ video pump thread: frame channel → C callback (first-consumer only; see the consumer module for multi-consumer).
fn video_pump(s: *mut mediaservo_client_session_t, mut rx: mpsc::Receiver<VideoFrame>) {
    let h = unsafe { &*s };
    let rt = runtime();
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        // Note: if `timeout(..)` is used as an outside argument of block_on it is constructed **on this context-less thread**
        //        (Sleep::new_timeout → Handle::current panics, proven in the spike) — it must be constructed inside the async block.
        match rt.block_on(async { timeout(VIDEO_POLL, rx.recv()).await }) {
            Ok(Some(frame)) => {
                let Some((cb, user)) = h.video_cb.lock().ok().and_then(|g| *g) else {
                    continue; // not registered / already unregistered: drop the frame (latest-value pump semantics)
                };
                let cframe = mediaservo_client_frame_t {
                    width: frame.width,
                    height: frame.height,
                    ts_us: frame.ts_us,
                    data: frame.data.as_ptr(),
                    len: frame.data.len(),
                };
                cb(s, &cframe, user);
            }
            Ok(None) => break, // session dropped → the frame channel died (normal convergence)
            Err(_) => continue, // VIDEO_POLL idle tick; re-check closed
        }
    }
}

/// Connect signaling and join the room (blocking). On success `*out` = the new handle (the caller closes).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_create(
    cfg: *const mediaservo_client_config_t,
    out: *mut *mut mediaservo_client_session_t,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_create";
    ffi_global(NAME, || {
        if cfg.is_null() || out.is_null() {
            set_last_error("mediaservo_client_session_create: null cfg/out");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let parts = match validate_session_cfg(unsafe { &*cfg }) {
            Ok(v) => v,
            Err(code) => return code,
        };
        let client_cfg = ClientConfig {
            signaling_url: parts.signaling_url.to_string(),
            room_id: parts.room.to_string(),
            psk: parts.psk.map(str::to_string),
            jwt: parts.jwt.map(str::to_string),
            role: parts.role,
            // W4b: the emergency-stop key file channel (G13 = keys never travel as plaintext argv/env). An invalid path does **not block session setup**
            // — non-safety surfaces like steer/consume keep working; only the estop call site reports INVALID_ARG
            // (hard-blocking at session setup would let one bad path take down the whole SDK session = an availability backfire; without a key on the vehicle
            // this field is irrelevant anyway).
            hmac_key: parts.hmac_key_file.and_then(|p| {
                config::load_hmac_key_file(p)
                    .map_err(|e| set_last_error(format!("mediaservo_client_session_create: {e}")))
                    .ok()
            }),
            // viewer-auth-matrix T1: the C ABI additive field (read with a struct_size bound check).
            identity_dir: parts.identity_dir.map(std::path::PathBuf::from),
        };
        // viewer-auth-matrix T4/F6: derive the identity label from **local config** before connecting (zero network).
        // Semantics match the server's ordering: Device (public key present) > jwt > psk; in the jwt form the sub is unknowable
        // (the SDK does not decode tokens), so it is labeled "jwt:<n> chars". None of the three = "(no credential)".
        let identity_label = match (parts.identity_dir, parts.jwt, parts.psk) {
            (Some(dir), _, _) => format!("Device({})", device_id_of_dir(dir)),
            (_, Some(_), _) => "jwt".to_string(),
            (_, _, Some(_)) => "legacy-psk".to_string(),
            _ => "(no credential)".to_string(),
        };
        match runtime().block_on(RoomSession::connect(&client_cfg)) {
            Ok(session) => {
                let negotiated = session.negotiated();
                let handle = Box::new(mediaservo_client_session_t {
                    session: std::sync::Mutex::new(Some(session)),
                    negotiated,
                    identity_label,
                    closed: AtomicBool::new(false),
                    video_started: AtomicBool::new(false),
                    video_cb: std::sync::Mutex::new(None),
                    video_pump: std::sync::Mutex::new(None),
                    state_regs: std::sync::Mutex::new(Vec::new()),
                    state_pump: std::sync::Mutex::new(None),
                    err: std::sync::Mutex::new(None),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_client_session_create: {e}"));
                error_code(&e)
            }
        }
    })
}

fn fallback_name(dir: &str) -> String {
    std::path::Path::new(dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string())
}

/// device_id extraction for the M4 identity label: read `<dir>/identity.json` (falls back to the directory name's tail on failure).
fn device_id_of_dir(dir: &str) -> String {
    // Parse the fixed key manually (avoiding a serde derive dependency just for this; the identity.json shape is pinned by D-H13).
    let parse_device_id = |raw: &[u8]| -> Option<String> {
        let v: serde_json::Value = serde_json::from_slice(raw).ok()?;
        v.get("device_id")?.as_str().map(str::to_string)
    };
    match std::fs::read(std::path::Path::new(dir).join("identity.json")) {
        Ok(raw) => parse_device_id(&raw).unwrap_or_else(|| fallback_name(dir)),
        Err(_) => fallback_name(dir),
    }
}

/// The negotiated dialect version (S0; an old server = 1).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_negotiated(
    s: *const mediaservo_client_session_t,
    out_protocol: *mut u32,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_negotiated";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_negotiated: null handle/out",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_protocol.is_null() {
            return h.fail_arg("mediaservo_client_session_negotiated: null out");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_session_negotiated: session closed");
        }
        unsafe { *out_protocol = h.negotiated };
        MEDIASERVO_OK
    })
}

/// The effective identity label (viewer-auth-matrix T4/F6; a static value derived locally at create, zero network).
/// Forms like "Device(ms-a1b2...)" / "jwt" / "legacy-psk" / "(no credential)".
/// Same needed overflow contract as list_rooms; readable even on a closed session (the label is a static snapshot).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_identity(
    s: *const mediaservo_client_session_t,
    out: *mut std::os::raw::c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_identity";
    ffi_global(NAME, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_identity: null session",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out.is_null() || cap == 0 {
            return fail_global(
                "mediaservo_client_session_identity: null out or cap 0",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        }
        copy_out_needed(NAME, &h.identity_label, out, cap, needed)
    })
}

/// Session connection-state snapshot (out_state: 0=Disconnected 1=Connected 2=Reconnecting 3=Failed).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_state(
    s: *const mediaservo_client_session_t,
    out_state: *mut u8,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_state";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_state: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_state.is_null() {
            return h.fail_arg("mediaservo_client_session_state: null out");
        }
        let guard = match h.session.lock() {
            Ok(g) => g,
            Err(_) => return h.fail_internal("mediaservo_client_session_state: lock poisoned"),
        };
        let Some(session) = guard.as_ref() else {
            return h.fail_state("mediaservo_client_session_state: session closed");
        };
        unsafe { *out_state = state_u8(session.connection_state()) };
        MEDIASERVO_OK
    })
}

/// Register a session connection-state callback (**cumulative form**: multiple registrations = every state change invokes each registered entry;
/// no unregister; released together at session_close with per-entry user_free).
///
/// The pump starts on first registration while the session is alive; it reports only **changes** (fetch the initial state after registering with
/// [`mediaservo_client_session_state`]). Calling this SDK's API from within a callback is forbidden (C contract).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_on_state(
    s: *mut mediaservo_client_session_t,
    cb: Option<mediaservo_client_state_cb>, // FFI-safe NPO
    user: *mut c_void,
    user_free: Option<mediaservo_client_user_free>,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_on_state";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_on_state: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        let Some(cb) = cb else {
            return h.fail_arg(
                "mediaservo_client_session_on_state: cb required (no off form; released with close)",
            );
        };
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_session_on_state: session closed");
        }
        {
            let mut guard = match h.state_regs.lock() {
                Ok(g) => g,
                Err(_) => {
                    return h.fail_internal("mediaservo_client_session_on_state: lock poisoned");
                }
            };
            guard.push(StateReg { cb, user, free: user_free });
        }
        // First registration with a live session → start the state pump (same discipline as link's on_event; with the session already closed,
        // the registration is kept but has no observation source; close still does user_free).
        let mut pump_guard = match h.state_pump.lock() {
            Ok(g) => g,
            Err(_) => {
                return h.fail_internal("mediaservo_client_session_on_state: pump lock poisoned");
            }
        };
        if pump_guard.is_none() {
            let has_session = h.session.lock().map(|g| g.is_some()).unwrap_or(false);
            if has_session {
                let raw = s as usize;
                let join = std::thread::spawn(move || state_pump_loop(raw));
                *pump_guard = Some(join);
            }
        }
        MEDIASERVO_OK
    })
}

/// Purely wait for a video producer in the room (no side effects — does not consume; the directed entry point for multi-consumer orchestration).
///
/// On success out_pid = the producer id (NUL-terminated); same needed overflow contract as list_rooms.
/// Timeout → ERR_TIMEOUT; disconnect / closed event stream → ERR_STATE.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_wait_video(
    s: *mut mediaservo_client_session_t,
    timeout_ms: u64,
    out_pid: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_wait_video";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_wait_video: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_pid.is_null() || cap == 0 {
            return h.fail_arg("mediaservo_client_session_wait_video: null out or cap 0");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_session_wait_video: session closed");
        }
        let mut guard = match h.session.lock() {
            Ok(g) => g,
            Err(_) => {
                return h.fail_internal("mediaservo_client_session_wait_video: lock poisoned");
            }
        };
        let Some(session) = guard.as_mut() else {
            return h.fail_state("mediaservo_client_session_wait_video: session closed");
        };
        let produced = match runtime()
            .block_on(session.wait_video_producer(Duration::from_millis(timeout_ms)))
        {
            Ok(p) => p,
            Err(e) => return h.fail_client("mediaservo_client_session_wait_video", &e),
        };
        let rc = copy_out_needed(NAME, &produced, out_pid, cap, needed);
        if rc != MEDIASERVO_OK {
            h.fail_arg("mediaservo_client_session_wait_video: buffer too small (see needed)");
        }
        rc
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_wait_video_labeled(
    s: *mut mediaservo_client_session_t,
    label: *const c_char,
    timeout_ms: u64,
    out_pid: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_wait_video_labeled";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_wait_video_labeled: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_pid.is_null() || cap == 0 {
            return h.fail_arg("mediaservo_client_session_wait_video_labeled: null out or cap 0");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_session_wait_video_labeled: session closed");
        }
        let mut guard = match h.session.lock() {
            Ok(g) => g,
            Err(_) => {
                return h
                    .fail_internal("mediaservo_client_session_wait_video_labeled: lock poisoned");
            }
        };
        let Some(session) = guard.as_mut() else {
            return h.fail_state("mediaservo_client_session_wait_video_labeled: session closed");
        };
        let label_opt = if label.is_null() {
            None
        } else {
            match unsafe { CStr::from_ptr(label) }.to_str() {
                Ok(l) if !l.is_empty() => Some(l.to_string()),
                _ => None,
            }
        };
        let produced =
            match runtime().block_on(session.wait_video_producer_labeled(
                Duration::from_millis(timeout_ms),
                label_opt.as_deref(),
            )) {
                Ok(p) => p,
                Err(e) => return h.fail_client("mediaservo_client_session_wait_video_labeled", &e),
            };
        let rc = copy_out_needed(NAME, &produced, out_pid, cap, needed);
        if rc != MEDIASERVO_OK {
            h.fail_arg(
                "mediaservo_client_session_wait_video_labeled: buffer too small (see needed)",
            );
        }
        rc
    })
}

/// Subscribe to one video producer (the new multi-consumer form, K4): one independent frame pump + consumer handle per producer,
/// no cross coupling. The setup sequence makes waiting explicit for the caller (first [`mediaservo_client_session_wait_video`]
/// or a known id); this call goes straight to consume.
///
/// `**out_consumer` = the new handle (the caller owns `mediaservo_client_consumer_close`;
/// session_close does not implicitly close it). user_free is invoked exactly once at consumer_close.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_consume(
    s: *mut mediaservo_client_session_t,
    producer_id: *const c_char,
    cb: Option<mediaservo_client_frame_cb>, // FFI-safe NPO
    user: *mut c_void,
    user_free: Option<mediaservo_client_user_free>,
    out_consumer: *mut *mut mediaservo_client_consumer_t,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_consume";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_consume: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        let Some(cb) = cb else {
            return h.fail_arg("mediaservo_client_session_consume: cb required");
        };
        let pid = match cstr(producer_id) {
            Ok(Some(p)) if !p.is_empty() => p,
            _ => return h.fail_arg("mediaservo_client_session_consume: producer_id required"),
        };
        let Some(out) = (unsafe { out_consumer.as_mut() }) else {
            return h.fail_arg("mediaservo_client_session_consume: null out_consumer");
        };
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_session_consume: session closed");
        }
        let consumer = {
            let guard = match h.session.lock() {
                Ok(g) => g,
                Err(_) => {
                    return h.fail_internal("mediaservo_client_session_consume: lock poisoned");
                }
            };
            let Some(session) = guard.as_ref() else {
                return h.fail_state("mediaservo_client_session_consume: session closed");
            };
            match runtime().block_on(session.consume(pid)) {
                Ok(c) => c,
                Err(e) => return h.fail_client("mediaservo_client_session_consume", &e),
            }
        };
        *out = consumer::spawn(s, consumer, pid.to_string(), cb, user, user_free);
        MEDIASERVO_OK
    })
}

/// ⊘ Consume the room's first video (first-consumer form, kept for one period): internally = wait(≤30s)+consume+
/// into_receiver remap, byte-for-byte identical to the old form (one-shot gate: a second call → ERR_STATE).
/// New code should use the wait_video + consume multi-consumer form.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_consume_video(
    s: *mut mediaservo_client_session_t,
    cb: Option<mediaservo_client_frame_cb>, // FFI-safe NPO
    user: *mut c_void,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_consume_video";
    ffi_catch(NAME, s, || {
        let Some(cb) = cb else {
            if s.is_null() {
                return fail_global(
                    "mediaservo_client_session_consume_video: cb required",
                    MEDIASERVO_CLIENT_ERR_INVALID_ARG,
                );
            }
            return unsafe { &*s }.fail_arg("mediaservo_client_session_consume_video: cb required");
        };
        let h = match unsafe { s.as_ref() } {
            Some(h) => h,
            None => {
                return fail_global(
                    "mediaservo_client_session_consume_video: null handle",
                    MEDIASERVO_CLIENT_ERR_INVALID_ARG,
                );
            }
        };
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_session_consume_video: session closed");
        }
        if h.video_started.swap(true, Ordering::SeqCst) {
            return h
                .fail_state("mediaservo_client_session_consume_video: video pump already started");
        }
        let rt = runtime();
        // The session lock is held during setup (within the single-thread-owner contract); pump threads never touch the session again.
        let frames = {
            let mut guard = match h.session.lock() {
                Ok(g) => g,
                Err(_) => {
                    h.video_started.store(false, Ordering::SeqCst);
                    return h
                        .fail_internal("mediaservo_client_session_consume_video: lock poisoned");
                }
            };
            let Some(session) = guard.as_mut() else {
                h.video_started.store(false, Ordering::SeqCst);
                return h.fail_state("mediaservo_client_session_consume_video: session closed");
            };
            let produced = match rt.block_on(session.wait_video_producer(VIDEO_PRODUCER_WAIT)) {
                Ok(p) => p,
                Err(e) => {
                    h.video_started.store(false, Ordering::SeqCst);
                    return h
                        .fail_client("mediaservo_client_session_consume_video: wait producer", &e);
                }
            };
            // Behavior remap (contract §2): consume+into_receiver ≡ the old consume_video
            // (the Rust-side consume_video is exactly the thin bridge over that composition, byte-for-byte equivalent).
            match rt.block_on(session.consume(&produced)).map(Consumer::into_receiver) {
                Ok(rx) => rx,
                Err(e) => {
                    h.video_started.store(false, Ordering::SeqCst);
                    return h.fail_client("mediaservo_client_session_consume_video: consume", &e);
                }
            }
        };
        if let Ok(mut g) = h.video_cb.lock() {
            *g = Some((cb, user));
        }
        // Raw pointers are not Send: pass the handle as usize (value semantics, address move only); the callback lives inside the handle
        // and is read each cycle — same pump discipline as link-c.
        let raw = s as usize;
        let join =
            std::thread::spawn(move || video_pump(raw as *mut mediaservo_client_session_t, frames));
        if let Ok(mut g) = h.video_pump.lock() {
            *g = Some(join);
        }
        MEDIASERVO_OK
    })
}

/// ⊘ video stats aggregate JSON (session-level union, the W3 mini-stats source; same fold semantics as the old `_pcs`,
/// reconciled in 1a). The object has the shape `{"bytes_received":..,"packets_received":..,"packets_lost":..,
/// "frames_decoded":..,"frame_width":..,"frame_height":..,"frames_per_second":..}`。
/// Same needed overflow contract as list_rooms. Use consumer_stats for precise per-consumer readings.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_video_stats(
    s: *const mediaservo_client_session_t,
    out_json: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_video_stats";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_video_stats: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_json.is_null() || cap == 0 {
            return h.fail_arg("mediaservo_client_session_video_stats: null out or cap 0");
        }
        let guard = match h.session.lock() {
            Ok(g) => g,
            Err(_) => {
                return h.fail_internal("mediaservo_client_session_video_stats: lock poisoned");
            }
        };
        let Some(session) = guard.as_ref() else {
            return h.fail_state("mediaservo_client_session_video_stats: session closed");
        };
        let json = match serde_json::to_string(&session.video_stats_summary()) {
            Ok(j) => j,
            Err(e) => {
                return h.fail_internal(&format!(
                    "mediaservo_client_session_video_stats: serialize: {e}"
                ));
            }
        };
        let rc = copy_out_needed(NAME, &json, out_json, cap, needed);
        if rc != MEDIASERVO_OK {
            h.fail_arg("mediaservo_client_session_video_stats: buffer too small (see needed)");
        }
        rc
    })
}

/// K3 handle-level machine-readable error (this handle's most recent failed call; no error = code 0).
///
/// Covers state/transport/mapping-class failures; buffer overflow (the needed contract) only produces a return code + the global last_error.
/// out.struct_size must be ≥ sizeof(mediaservo_client_error_t) (the R3 evolution discipline).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_error(
    s: *const mediaservo_client_session_t,
    out: *mut mediaservo_client_error_t,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_error";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_error: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        let Some(o) = (unsafe { out.as_mut() }) else {
            return h.fail_arg("mediaservo_client_session_error: null out");
        };
        if check_struct_size(o.struct_size, CLIENT_ERROR_T_SIZE, NAME).is_err() {
            h.fail_arg("mediaservo_client_session_error: error_t.struct_size too small (rebuild with current header)");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let guard = match h.err.lock() {
            Ok(g) => g,
            Err(_) => return h.fail_internal("mediaservo_client_session_error: lock poisoned"),
        };
        match guard.as_ref() {
            Some(e) => {
                o.code = e.code;
                o.wire_code = e.wire;
                o.retryable = u8::from(e.retryable);
            }
            None => {
                o.code = MEDIASERVO_OK;
                o.wire_code = 0;
                o.retryable = 0;
            }
        }
        MEDIASERVO_OK
    })
}

/// K3 handle-level error text (details of this handle's most recent failed call; no error = empty string).
/// Copy semantics = truncate without erroring (same as the ⊘ last_error).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_last_error(
    s: *const mediaservo_client_session_t,
    buf: *mut c_char,
    len: usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_last_error";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_last_error: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        let msg =
            h.err.lock().ok().and_then(|g| g.as_ref().map(|e| e.msg.clone())).unwrap_or_default();
        copy_out_text(&msg, buf, len)
    })
}

/// Emergency-stop dual-path delivery (the C face of W4b·S4 semantics): the DC fast path (signed with the key loaded
/// via `hmac_key_file` at session creation; unconfigured = unsigned form) + the signaling audit copy.
/// Precondition: mediaservo_client_open_control has succeeded (the ctl handle comes from that call). payload_json
/// NULL/empty = a Null payload. Returning OK only means the **delivery** succeeded (the vehicle's adjudication is the ack —
/// recv_ack receives the seq receipt), not "it executed".
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_emergency_stop(
    s: *mut mediaservo_client_session_t,
    c: *mut mediaservo_client_control_t,
    label: *const c_char,
    seq: u64,
    payload_json: *const c_char,
) -> c_int {
    const NAME: &str = "mediaservo_client_session_emergency_stop";
    ffi_catch(NAME, s, || {
        let Some(sh) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_session_emergency_stop: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        let label = match cstr(label) {
            Ok(Some(l)) if !l.is_empty() => l,
            _ => return sh.fail_arg("mediaservo_client_session_emergency_stop: label required"),
        };
        let payload = match cstr(payload_json) {
            Ok(Some(p)) if !p.is_empty() => match serde_json::from_str::<serde_json::Value>(p) {
                Ok(v) => v,
                Err(e) => {
                    return sh.fail(
                        MEDIASERVO_CLIENT_ERR_MALFORMED,
                        &format!("mediaservo_client_session_emergency_stop: payload json: {e}"),
                    );
                }
            },
            _ => serde_json::Value::Null,
        };
        let Some(ch) = (unsafe { c.as_ref() }) else {
            return sh.fail_arg("mediaservo_client_session_emergency_stop: null control handle");
        };
        if sh.closed.load(Ordering::SeqCst) || ch.is_closed() {
            return sh
                .fail_state("mediaservo_client_session_emergency_stop: session or control closed");
        }
        // Lock order session→ctl (consistent with all compound operations; no reverse-order path = no deadlock surface).
        let mut sg = match sh.session.lock() {
            Ok(g) => g,
            Err(_) => {
                return sh.fail_internal(
                    "mediaservo_client_session_emergency_stop: session lock poisoned",
                );
            }
        };
        let Some(session) = sg.as_mut() else {
            return sh.fail_state("mediaservo_client_session_emergency_stop: session closed");
        };
        let mut cg = match ch.ctl_lock() {
            Ok(g) => g,
            Err(_) => {
                return sh.fail_internal(
                    "mediaservo_client_session_emergency_stop: control lock poisoned",
                );
            }
        };
        match runtime().block_on(session.emergency_stop(&mut cg, label, seq, payload)) {
            Ok(()) => MEDIASERVO_OK,
            Err(e) => sh.fail_client("mediaservo_client_session_emergency_stop", &e),
        }
    })
}

/// Open the outbound control channel set (once per session — one ack pump per session, gated on the Rust side).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_open_control(
    s: *mut mediaservo_client_session_t,
    labels: *const *const c_char,
    labels_len: usize,
    out: *mut *mut mediaservo_client_control_t,
) -> c_int {
    const NAME: &str = "mediaservo_client_open_control";
    ffi_catch(NAME, s, || {
        let Some(h) = (unsafe { s.as_ref() }) else {
            return fail_global(
                "mediaservo_client_open_control: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out.is_null()
            || (!labels.is_null() && labels_len == 0)
            || (labels.is_null() && labels_len > 0)
        {
            return h.fail_arg("mediaservo_client_open_control: null out/labels-mismatch");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_open_control: session closed");
        }
        let mut label_vec: Vec<&str> = Vec::with_capacity(labels_len);
        for i in 0..labels_len {
            match cstr(unsafe { *labels.add(i) }) {
                Ok(Some(l)) if !l.is_empty() => label_vec.push(l),
                _ => {
                    return h.fail_arg(&format!(
                        "mediaservo_client_open_control: label[{i}] missing or invalid UTF-8"
                    ));
                }
            }
        }
        let channel = {
            let mut guard = match h.session.lock() {
                Ok(g) => g,
                Err(_) => return h.fail_internal("mediaservo_client_open_control: lock poisoned"),
            };
            let Some(session) = guard.as_mut() else {
                return h.fail_state("mediaservo_client_open_control: session closed");
            };
            match runtime().block_on(session.open_control(&label_vec)) {
                Ok(ctl) => ctl,
                Err(e) => return h.fail_client("mediaservo_client_open_control", &e),
            }
        };
        unsafe { *out = control::into_raw(control::new(channel)) };
        MEDIASERVO_OK
    })
}

/// Close the session and release the handle (idempotent). Order: the closed flag → close the session (frame channels die with it, waking
/// the ⊘ pump; the watch sender dying wakes the state pump) → join the video pump → join the state pump → then
/// user_free → free. join blocks while a pump is inside a callback — callbacks must return quickly.
/// consumer/control handles are not reclaimed by this call (independent close discipline).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_close(s: *mut mediaservo_client_session_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(s) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent (same double-close discipline as link-c)
        }
        let session = handle.session.lock().ok().and_then(|mut g| g.take());
        if let Some(session) = session
            && let Err(e) = runtime().block_on(session.close())
        {
            set_last_error(format!("mediaservo_client_session_close: {e}"));
        }
        if let Some(join) = handle.video_pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join();
        }
        if let Some(join) = handle.state_pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join();
        }
        // State-callback reclamation: user_free exactly once per entry (the pump has been joined, no concurrent firing).
        let regs =
            handle.state_regs.lock().ok().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
        for r in regs {
            if let Some(f) = r.free {
                f(r.user);
            }
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_client_session_close: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consumer::mediaservo_client_consumer_close;
    use crate::control::mediaservo_client_control_close;
    use crate::errors::MEDIASERVO_CLIENT_ERR_STATE;
    use std::ptr;
    use std::sync::atomic::AtomicUsize;

    // -- FFI entry null guards (no network) --
    #[test]
    fn exchange_null_args_fail() {
        // accountless-client-auth T2: all three surfaces missing + null out are rejected locally (no network).
        let mut buf = [0u8; 64];
        let mut need = 0usize;
        let rc = mediaservo_client_exchange(
            ptr::null(),
            c"k".as_ptr(),
            c"s".as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut need,
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        let rc = mediaservo_client_exchange(
            c"http://127.0.0.1:9".as_ptr(),
            c"".as_ptr(),
            c"s".as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut need,
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        let rc = mediaservo_client_exchange(
            c"http://127.0.0.1:9".as_ptr(),
            c"k".as_ptr(),
            c"s".as_ptr(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn login_null_args_fail() {
        let mut buf = [0u8; 64];
        let mut need = 0usize;
        let rc = mediaservo_client_login(
            ptr::null(),
            c"u".as_ptr(),
            c"p".as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut need,
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // Empty strings are treated as missing (the old validate_login_cfg semantics preserved).
        let rc = mediaservo_client_login(
            c"".as_ptr(),
            c"u".as_ptr(),
            c"p".as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut need,
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // null out_jwt / cap 0 — rejected locally (no network).
        let rc = mediaservo_client_login(
            c"http://127.0.0.1:9".as_ptr(),
            c"u".as_ptr(),
            c"p".as_ptr(),
            ptr::null_mut(),
            64,
            ptr::null_mut(),
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn list_rooms_guards_fail_before_network() {
        let mut buf = [0u8; 128];
        let base = std::ffi::CString::new("http://127.0.0.1:9").unwrap();
        let jwt = std::ffi::CString::new("j").unwrap();
        // null http_base / null jwt / null out / cap 0 — all rejected locally (no network;
        // the assertion point precedes block_on: if it hit the network we would see Io/Timeout rather than INVALID_ARG).
        let cases = [
            (ptr::null(), jwt.as_ptr(), buf.as_mut_ptr() as *mut c_char, buf.len()),
            (base.as_ptr(), ptr::null(), buf.as_mut_ptr() as *mut c_char, buf.len()),
            (base.as_ptr(), jwt.as_ptr(), ptr::null_mut(), buf.len()),
            (base.as_ptr(), jwt.as_ptr(), buf.as_mut_ptr() as *mut c_char, 0),
        ];
        for (b, j, o, c) in cases {
            assert_eq!(
                mediaservo_client_list_rooms(b, j, o, c, ptr::null_mut()),
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
                "case b={b:?} c={c}"
            );
        }
    }

    #[test]
    fn strerror_ffi_roundtrip() {
        let mut buf = [0u8; 64];
        let rc = mediaservo_client_strerror(
            MEDIASERVO_CLIENT_ERR_STATE,
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        assert_eq!(rc, MEDIASERVO_OK);
        let s =
            unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(s.contains("invalid state"), "strerror(-9) = {s}");
    }

    #[test]
    fn session_create_null_fails() {
        let rc = mediaservo_client_session_create(ptr::null(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn video_stats_null_and_cap_guards() {
        let mut buf = [0u8; 256];
        assert_eq!(
            mediaservo_client_session_video_stats(
                ptr::null(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                ptr::null_mut()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // Non-null but cap 0: an invalid handle still may not touch the network or dereference — parameter guard first.
        assert_eq!(
            mediaservo_client_session_video_stats(
                ptr::null(),
                buf.as_mut_ptr() as *mut c_char,
                0,
                ptr::null_mut()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }

    #[test]
    fn emergency_stop_null_guards_before_network() {
        let label = std::ffi::CString::new("chassis").unwrap();
        assert_eq!(
            mediaservo_client_session_emergency_stop(
                ptr::null_mut(),
                ptr::null_mut(),
                label.as_ptr(),
                900,
                ptr::null()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // label null pointer (an invalid session side also gets the parameter guard first).
        let dummy_ctl = ptr::null_mut::<mediaservo_client_control_t>();
        assert_eq!(
            mediaservo_client_session_emergency_stop(
                ptr::null_mut(),
                dummy_ctl,
                ptr::null(),
                1,
                ptr::null()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }

    #[test]
    fn session_negotiated_null_fails() {
        let mut p = 0u32;
        let rc = mediaservo_client_session_negotiated(ptr::null(), &mut p);
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn consume_video_null_cb_fails() {
        let rc = mediaservo_client_session_consume_video(ptr::null_mut(), None, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn open_control_null_fails() {
        let rc = mediaservo_client_open_control(ptr::null_mut(), ptr::null(), 0, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // labels non-null but len=0 → reject
        let labels = [c"chassis".as_ptr()];
        let rc =
            mediaservo_client_open_control(ptr::null_mut(), labels.as_ptr(), 0, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn close_null_is_ok() {
        assert_eq!(mediaservo_client_session_close(ptr::null_mut()), MEDIASERVO_OK);
        assert_eq!(mediaservo_client_control_close(ptr::null_mut()), MEDIASERVO_OK);
        assert_eq!(mediaservo_client_consumer_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn version_roundtrip() {
        let mut buf = [0u8; 32];
        let rc = mediaservo_client_version(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s =
            unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(s.starts_with("0.1."), "version: {s}");
    }

    #[test]
    fn last_error_ffi_roundtrip() {
        set_last_error("client-c ffi error");
        let mut buf = [0u8; 64];
        let rc = mediaservo_client_last_error(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
    }

    // -- batch 1b: K3 handle error slots (the deterministic session=None failure form, no network needed) --

    /// Test-only: a bare handle with no session (session=None = the "closed" form — state/stats calls
    /// fail deterministically, exercising the handle-slot path without a real server).
    fn test_session_handle() -> *mut mediaservo_client_session_t {
        Box::into_raw(Box::new(mediaservo_client_session_t {
            session: std::sync::Mutex::new(None),
            negotiated: 3,
            identity_label: "jwt".to_string(),
            closed: AtomicBool::new(false),
            video_started: AtomicBool::new(false),
            video_cb: std::sync::Mutex::new(None),
            video_pump: std::sync::Mutex::new(None),
            state_regs: std::sync::Mutex::new(Vec::new()),
            state_pump: std::sync::Mutex::new(None),
            err: std::sync::Mutex::new(None),
        }))
    }

    extern "C" fn noop_state_cb(_s: *mut mediaservo_client_session_t, _st: u8, _u: *mut c_void) {}

    #[test]
    fn session_error_t_roundtrip_and_double_write() {
        let s = test_session_handle();
        // Deterministic failure: session=None → STATE (handle slot + global dual write).
        let mut st = 9u8;
        assert_eq!(mediaservo_client_session_state(s, &mut st), MEDIASERVO_CLIENT_ERR_STATE);
        let mut e = mediaservo_client_error_t {
            struct_size: size_of::<mediaservo_client_error_t>(),
            code: 99,
            wire_code: 99,
            retryable: 99,
        };
        assert_eq!(mediaservo_client_session_error(s, &mut e), MEDIASERVO_OK);
        assert_eq!(e.code, MEDIASERVO_CLIENT_ERR_STATE);
        assert_eq!(e.wire_code, 0); // a local state error has no wire code
        assert_eq!(e.retryable, 0);
        // Handle-level text is non-empty (carries the failure context).
        let mut buf = [0u8; 128];
        assert_eq!(
            mediaservo_client_session_last_error(s, buf.as_mut_ptr() as *mut c_char, buf.len()),
            MEDIASERVO_OK
        );
        let msg =
            unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(msg.contains("session closed"), "handle msg: {msg}");
        // ⊘ the global fallback dual write (the K3 one-period contract).
        let mut gbuf = [0u8; 128];
        assert_eq!(
            mediaservo_client_last_error(gbuf.as_mut_ptr() as *mut c_char, gbuf.len()),
            MEDIASERVO_OK
        );
        assert_eq!(mediaservo_client_session_close(s), MEDIASERVO_OK);
    }

    #[test]
    fn session_error_struct_size_guard() {
        let s = test_session_handle();
        let mut e =
            mediaservo_client_error_t { struct_size: 1, code: 0, wire_code: 0, retryable: 0 };
        assert_eq!(mediaservo_client_session_error(s, &mut e), MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        assert_eq!(mediaservo_client_session_close(s), MEDIASERVO_OK);
    }

    #[test]
    fn session_error_no_error_is_code_zero() {
        let s = test_session_handle();
        let mut e = mediaservo_client_error_t {
            struct_size: size_of::<mediaservo_client_error_t>(),
            code: 42,
            wire_code: 42,
            retryable: 42,
        };
        assert_eq!(mediaservo_client_session_error(s, &mut e), MEDIASERVO_OK);
        assert_eq!(e.code, MEDIASERVO_OK);
        assert_eq!(mediaservo_client_session_close(s), MEDIASERVO_OK);
    }

    #[test]
    fn on_state_cumulative_and_user_free_exactly_once() {
        static FREES: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn count_free(_u: *mut c_void) {
            FREES.fetch_add(1, Ordering::SeqCst);
        }
        let s = test_session_handle();
        // Cumulative registration: three entries (two with free, one NULL free). session=None → the pump does not start,
        // but registration still succeeds (semantics = register for close-time reclamation); the live conversion surface = 1c.
        for _ in 0..2 {
            assert_eq!(
                mediaservo_client_session_on_state(
                    s,
                    Some(noop_state_cb),
                    ptr::null_mut(),
                    Some(count_free)
                ),
                MEDIASERVO_OK
            );
        }
        assert_eq!(
            mediaservo_client_session_on_state(s, Some(noop_state_cb), ptr::null_mut(), None),
            MEDIASERVO_OK
        );
        // cb=NULL → reject (the cumulative form has no "unregister" semantics).
        assert_eq!(
            mediaservo_client_session_on_state(s, None, ptr::null_mut(), None),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        let before = FREES.load(Ordering::SeqCst);
        assert_eq!(mediaservo_client_session_close(s), MEDIASERVO_OK);
        // user_free exactly once per entry (the two registrations that carry free).
        assert_eq!(FREES.load(Ordering::SeqCst), before + 2);
    }

    #[test]
    fn consume_video_second_call_hits_gate() {
        // ⊘ one-shot gate pin: video_started already set → STATE (the gate is checked before session access,
        // so a session=None handle suffices to test it).
        let s = test_session_handle();
        unsafe { &*s }.video_started.store(true, Ordering::SeqCst);
        assert_eq!(
            mediaservo_client_session_consume_video(s, Some(noop_frame_cb), ptr::null_mut()),
            MEDIASERVO_CLIENT_ERR_STATE
        );
        assert_eq!(mediaservo_client_session_close(s), MEDIASERVO_OK);
    }

    extern "C" fn noop_frame_cb(
        _s: *mut mediaservo_client_session_t,
        _f: *const mediaservo_client_frame_t,
        _u: *mut c_void,
    ) {
    }

    extern "C" fn noop_user_free(_u: *mut c_void) {}

    #[test]
    fn wait_video_guards_before_network() {
        let mut buf = [0u8; 64];
        // null handle → global INVALID_ARG (no network).
        assert_eq!(
            mediaservo_client_session_wait_video(
                ptr::null_mut(),
                10,
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                ptr::null_mut()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // Handle present but the session is closed → STATE (before any waiting).
        let s = test_session_handle();
        unsafe { &*s }.closed.store(true, Ordering::SeqCst);
        assert_eq!(
            mediaservo_client_session_wait_video(
                s,
                10,
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                ptr::null_mut()
            ),
            MEDIASERVO_CLIENT_ERR_STATE
        );
        // The closed handle is not freed (close semantics are covered by other cases); this leak stays inside the test process.
    }

    #[test]
    fn session_consume_guards() {
        let s = test_session_handle();
        let mut out: *mut mediaservo_client_consumer_t = ptr::null_mut();
        let pid = c"p1";
        // cb is required (the cumulative multi-consumer form has no "cancel" semantics).
        assert_eq!(
            mediaservo_client_session_consume(
                s,
                ptr::null(),
                None,
                ptr::null_mut(),
                None,
                &mut out
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // producer_id is required.
        assert_eq!(
            mediaservo_client_session_consume(
                s,
                ptr::null(),
                Some(noop_frame_cb),
                ptr::null_mut(),
                None,
                &mut out
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // out is required.
        assert_eq!(
            mediaservo_client_session_consume(
                s,
                pid.as_ptr(),
                Some(noop_frame_cb),
                ptr::null_mut(),
                Some(noop_user_free),
                ptr::null_mut()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(mediaservo_client_session_close(s), MEDIASERVO_OK);
    }
}
