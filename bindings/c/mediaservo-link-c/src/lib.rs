#![allow(clippy::not_unsafe_ptr_arg_deref)]
// C ABI facade shape: extern fn receiving raw pointers IS the contract; null guards live in the body (an unsafe fn form would push the obligation onto the C side)
//! MediaServo link C ABI — signaling + frame bus (device-side SDK consumption).
//!
//! Contract §7 (D109/D240/D241): opaque handles + int error codes + callbacks.
//! Pattern baseline = bindings/c/mediaservo-field-c (slice 1 was delivered and proven in live e2e).
//!
//! # Lifecycle contract (audit R2)
//! - Each handle is single-thread-owner; any API call after close is UB (except idempotent close).
//! - close = set the closed flag → release the session (wakes the event pump) → join the pump thread → only then free the handle.
//! - Event callbacks fire on a single internal pump thread; no lock is held during the callback;
//!   calling any mediaservo_link_signal_* API (including close) from within a callback is forbidden — undefined behavior.
//! - `mediaservo_link_signal_on_event` can be registered at any time after connect (re-registration replaces the callback);
//!   the first registration starts the event pump and re-synthesizes the Connected event (events sent before subscribing to the broadcast are invisible).
//! - The event JSON string is valid only inside the callback (copy it if you need to keep it).
//!
//! # runtime (audit R1)
//! `SignalClient::connect` internally does `tokio::spawn(session_task)` (WS read loop) — a per-call
//! runtime would cancel the spawned task on return, killing the session. Therefore each signal handle owns
//! a shared multi_thread runtime; all C calls block_on the same instance; the event pump reuses that runtime.
//!
//! # FrameMeta（R4）
//! mediaservo_frame_meta_t (C-side #pragma pack(1)) is a field bag: the Rust side does a 36B copy +
//! [`FrameMeta::decode`] field-by-field read; never reinterpret the whole block (padding/endianness risk).
//!
//! # C ABI surface (hand-maintained header, stable within MAJOR)
//! ```c
//! typedef struct mediaservo_link_signal_t mediaservo_link_signal_t;   /* opaque */
//! typedef int mediaservo_err_t;                               /* 0=ok, <0=error */
//!
//! mediaservo_err_t mediaservo_link_signal_connect(const mediaservo_link_signal_config_t* cfg, mediaservo_link_signal_t** out);
//! mediaservo_err_t mediaservo_link_signal_send(mediaservo_link_signal_t* s, const char* msg_json, size_t len);
//! void     mediaservo_link_signal_on_event(mediaservo_link_signal_t* s, mediaservo_link_event_cb cb, void* user);
//! mediaservo_err_t mediaservo_link_signal_close(mediaservo_link_signal_t* s);
//! mediaservo_err_t mediaservo_link_bus_attach(const char* endpoint, const char* token_pem,
//!                             const char* vk_pem, mediaservo_link_bus_t** out);
//! mediaservo_err_t mediaservo_link_bus_publish(mediaservo_link_bus_t* b, const char* topic,
//!                              const uint8_t* payload, size_t len, const mediaservo_frame_meta_t* meta);
//! mediaservo_err_t mediaservo_link_bus_subscribe(mediaservo_link_bus_t* b, const char* topic, mediaservo_link_stream_t** out);
//! mediaservo_err_t mediaservo_link_bus_recv(mediaservo_link_stream_t* st, mediaservo_frame_meta_t* out_meta,
//!                           uint8_t* out_data, size_t cap, size_t* out_len);
//! mediaservo_err_t mediaservo_link_stream_close(mediaservo_link_stream_t* st);
//! mediaservo_err_t mediaservo_link_bus_close(mediaservo_link_bus_t* b);
//! mediaservo_err_t mediaservo_link_last_error(char* buf, size_t len);
//! mediaservo_err_t mediaservo_link_version(char* buf, size_t len);
//! ```

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{
    CapabilityToken, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameRef, FrameStream, FrameTopic,
    SignalClient, SignalEvent, SignalSession,
};
use tokio::sync::{Notify, broadcast};

// -- Error codes (0 = ok, <0 = error; MEDIASERVO_LINK_ERR_*, prefixed per D241) --
pub const MEDIASERVO_OK: c_int = 0;
pub const MEDIASERVO_LINK_ERR_INVALID_ARG: c_int = -1;
pub const MEDIASERVO_LINK_ERR_CONNECT: c_int = -2;
pub const MEDIASERVO_LINK_ERR_SEND: c_int = -3;
pub const MEDIASERVO_LINK_ERR_BUS: c_int = -4;
pub const MEDIASERVO_LINK_ERR_STATE: c_int = -5;
pub const MEDIASERVO_LINK_ERR_INTERNAL: c_int = -6;
pub const MEDIASERVO_LINK_ERR_CLOSED: c_int = -7;

/// Global last-error message (read via mediaservo_link_last_error).
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

// -- Internal helpers --

/// Extract a C string (null → None). Invalid UTF-8 → Err.
fn cstr<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().map(Some).map_err(|_| ())
}

/// Create a shared multi_thread runtime (one per handle, R1).
fn new_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("link-c runtime")
}

/// C-side mediaservo_frame_meta_t → 36B copy → FrameMeta::decode (field-by-field, R4).
fn meta_from_c(meta: *const mediaservo_frame_meta_t) -> Result<FrameMeta, ()> {
    let mut buf = [0u8; FrameMeta::WIRE_LEN];
    unsafe {
        ptr::copy_nonoverlapping(meta as *const u8, buf.as_mut_ptr(), FrameMeta::WIRE_LEN);
    }
    FrameMeta::decode(&buf).map_err(|_| ())
}

/// FrameMeta::encode → 36B copy into the C-side mediaservo_frame_meta_t.
fn meta_to_c(meta: &FrameMeta, out: *mut mediaservo_frame_meta_t) {
    let bytes = meta.encode();
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, FrameMeta::WIRE_LEN);
    }
}

// -- Signaling --

/// Signaling configuration (C struct — maps to the SignalClient parameters).
///
/// First field `struct_size` (audit R3): the caller fills `sizeof(mediaservo_link_signal_config_t)`;
/// the library validates `>= sizeof(known struct)` and ignores extra length — struct evolution does not break binary compatibility.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception: mediaservo_* prefix)
#[repr(C)]
pub struct mediaservo_link_signal_config_t {
    pub struct_size: usize,
    /// Signaling WS address (e.g. "ws://host:9800/ws").
    pub url: *const c_char,
    /// PSK authentication key.
    pub psk: *const c_char,
    /// Room ID.
    pub room: *const c_char,
    /// Role: `"Host"`/`"Pusher"` → Host, `"Client"`/`"Puller"` → Remote; NULL/empty = Host.
    pub role: *const c_char,
}

/// Known prefix size of the C struct (the minimum legal value across version evolution).
pub const MEDIASERVO_LINK_SIGNAL_CONFIG_MIN_SIZE: usize =
    size_of::<mediaservo_link_signal_config_t>();

impl Default for mediaservo_link_signal_config_t {
    fn default() -> Self {
        Self {
            struct_size: MEDIASERVO_LINK_SIGNAL_CONFIG_MIN_SIZE,
            url: ptr::null(),
            psk: ptr::null(),
            room: ptr::null(),
            role: ptr::null(),
        }
    }
}

/// C-side role string → PeerRole.
fn parse_role(s: &str) -> Result<PeerRole, ()> {
    match s {
        "Host" | "Pusher" => Ok(PeerRole::Host),
        "Client" | "Puller" => Ok(PeerRole::Remote),
        _ => Err(()),
    }
}

/// Signaling session opaque handle.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception: mediaservo_* prefix)
pub struct mediaservo_link_signal_t {
    session: std::sync::Mutex<Option<SignalSession>>,
    /// Shared multi_thread runtime (R1): the session background task (WS read loop)
    /// lives on this runtime; all C calls block_on the same instance.
    rt: tokio::runtime::Runtime,
    /// Closed flag (close idempotence + entry validation).
    closed: AtomicBool,
    /// Room ID captured at connect (used for the pump-synthesized Connected event).
    room_id: String,
    /// Event receiver subscribed at connect time (buffers events between connect→on_event;
    /// the broadcast only keeps messages sent after subscribing); the pump takes it over at startup.
    events_rx: std::sync::Mutex<Option<broadcast::Receiver<SignalEvent>>>,
    /// Event callback (read by the pump thread each cycle; re-registration replaces).
    cb: std::sync::Mutex<Option<(mediaservo_link_event_cb, *mut c_void)>>,
    /// Event pump thread (started on first on_event registration, R2).
    pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

// SAFETY: the handle holds Mutex<Option<SignalSession>> (thread-safe) + a Send runtime.
unsafe impl Send for mediaservo_link_signal_t {}
// SAFETY: all methods access the inner session serialized through the Mutex.
unsafe impl Sync for mediaservo_link_signal_t {}

/// Signaling event callback (C ABI). event_json is valid only inside the callback.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception: mediaservo_* prefix)
pub type mediaservo_link_event_cb =
    extern "C" fn(*mut mediaservo_link_signal_t, *const c_char, *mut c_void);

/// SignalEvent → opaque JSON string (v1).
fn event_to_json(ev: &SignalEvent) -> String {
    let v = match ev {
        SignalEvent::Connected { room_id } => {
            serde_json::json!({"type": "connected", "room_id": room_id})
        }
        SignalEvent::Message(m) => serde_json::json!({"type": "message", "message": m}),
        SignalEvent::Disconnected { reason } => {
            serde_json::json!({"type": "disconnected", "reason": reason})
        }
        SignalEvent::Error(e) => serde_json::json!({"type": "error", "error": e}),
        // #[non_exhaustive]: new variants land via the event-JSON contract evolution (opaque v1 unknown type)
        _ => serde_json::json!({"type": "unknown"}),
    };
    v.to_string()
}

/// Invoke the C callback (holding no lock — the guard is released after cloning cb, R2).
fn deliver_event(handle: *mut mediaservo_link_signal_t, json: String) {
    let h = unsafe { &*handle };
    let cb = h.cb.lock().ok().and_then(|g| *g);
    if let Some((cb, user)) = cb {
        // CString lives until the callback returns (serde_json output has no embedded NUL).
        if let Ok(cstr) = CString::new(json) {
            cb(handle, cstr.as_ptr(), user);
        }
    }
}

/// Event pump thread (R2): first re-synthesize Connected (events sent before subscribing to the broadcast are invisible),
/// then forward events in a loop. Session closed (all broadcast senders dropped) or the closed flag → exit.
fn signal_pump(handle: *mut mediaservo_link_signal_t) {
    let h = unsafe { &*handle };
    deliver_event(
        handle,
        serde_json::json!({"type": "connected", "room_id": h.room_id}).to_string(),
    );
    let mut rx = match h.events_rx.lock().ok().and_then(|mut g| g.take()) {
        Some(rx) => rx,
        None => return,
    };
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        match h.rt.block_on(rx.recv()) {
            Ok(ev) => deliver_event(handle, event_to_json(&ev)),
            Err(broadcast::error::RecvError::Closed) => break, // session closed
            Err(broadcast::error::RecvError::Lagged(_)) => continue, // dropped on overflow (documented)
        }
    }
}

/// Connect signaling and create the session (blocking).
///
/// `cfg` must not be null; `url/psk/room` are required; `role` may be NULL (default Host);
/// `cfg.struct_size` must be `>= sizeof(mediaservo_link_signal_config_t)` (R3).
/// On success `*out` points to the new handle (the caller owns `mediaservo_link_signal_close`).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_signal_connect(
    cfg: *const mediaservo_link_signal_config_t,
    out: *mut *mut mediaservo_link_signal_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if cfg.is_null() || out.is_null() {
            set_last_error("mediaservo_link_signal_connect: null cfg/out");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        let cfg_ref = unsafe { &*cfg };
        if cfg_ref.struct_size < MEDIASERVO_LINK_SIGNAL_CONFIG_MIN_SIZE {
            set_last_error(format!(
                "mediaservo_link_signal_connect: cfg.struct_size {} < {} (rebuild with current header)",
                cfg_ref.struct_size,
                MEDIASERVO_LINK_SIGNAL_CONFIG_MIN_SIZE
            ));
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        let (url, psk, room) = match (
            cstr(cfg_ref.url),
            cstr(cfg_ref.psk),
            cstr(cfg_ref.room),
        ) {
            (Ok(Some(u)), Ok(Some(p)), Ok(Some(r))) => (u, p, r),
            (Ok(None), _, _) => {
                set_last_error("mediaservo_link_signal_connect: url required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
            (_, Ok(None), _) => {
                set_last_error("mediaservo_link_signal_connect: psk required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
            (_, _, Ok(None)) => {
                set_last_error("mediaservo_link_signal_connect: room required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
            _ => {
                set_last_error("mediaservo_link_signal_connect: invalid UTF-8 in config");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
        };
        let role = match cstr(cfg_ref.role) {
            Ok(Some(r)) => match parse_role(r) {
                Ok(role) => role,
                Err(()) => {
                    set_last_error(format!(
                        "mediaservo_link_signal_connect: unknown role '{r}' (Host/Pusher/Client/Puller)"
                    ));
                    return MEDIASERVO_LINK_ERR_INVALID_ARG;
                }
            },
            Ok(None) => PeerRole::Host, // vehicle-side default
            Err(()) => {
                set_last_error("mediaservo_link_signal_connect: invalid UTF-8 in role");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
        };

        let rt = new_runtime();
        let client = SignalClient::new(url, psk, room, role);
        match rt.block_on(client.connect()) {
            Ok(session) => {
                // Subscribe immediately: buffer events between connect→on_event (the broadcast keeps only post-subscription messages).
                let events_rx = session.events();
                let room_id = session.room_id().to_string();
                let handle = Box::new(mediaservo_link_signal_t {
                    session: std::sync::Mutex::new(Some(session)),
                    rt,
                    closed: AtomicBool::new(false),
                    room_id,
                    events_rx: std::sync::Mutex::new(Some(events_rx)),
                    cb: std::sync::Mutex::new(None),
                    pump: std::sync::Mutex::new(None),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_link_signal_connect: {e}"));
                MEDIASERVO_LINK_ERR_CONNECT
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_signal_connect: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// Send one signaling message (JSON string + byte length; blocking enqueue).
///
/// The message type is [`SignalingMessage`] (type tag is snake_case). Parse failure → SEND error.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_signal_send(
    s: *mut mediaservo_link_signal_t,
    msg_json: *const c_char,
    len: usize,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() || msg_json.is_null() {
            set_last_error("mediaservo_link_signal_send: null handle/msg");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        if len == 0 {
            set_last_error("mediaservo_link_signal_send: empty message");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*s };
        if handle.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_link_signal_send: session closed");
            return MEDIASERVO_LINK_ERR_STATE;
        }
        let bytes = unsafe { std::slice::from_raw_parts(msg_json as *const u8, len) };
        let msg: SignalingMessage = match serde_json::from_slice(bytes) {
            Ok(m) => m,
            Err(e) => {
                set_last_error(format!("mediaservo_link_signal_send: parse: {e}"));
                return MEDIASERVO_LINK_ERR_SEND;
            }
        };
        let guard = match handle.session.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("mediaservo_link_signal_send: lock poisoned");
                return MEDIASERVO_LINK_ERR_INTERNAL;
            }
        };
        let Some(session) = guard.as_ref() else {
            set_last_error("mediaservo_link_signal_send: session closed");
            return MEDIASERVO_LINK_ERR_STATE;
        };
        match handle.rt.block_on(session.send(msg)) {
            Ok(()) => MEDIASERVO_OK,
            Err(e) => {
                set_last_error(format!("mediaservo_link_signal_send: {e}"));
                MEDIASERVO_LINK_ERR_SEND
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_signal_send: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// Register the event callback (any time after connect; re-registration replaces).
///
/// The first registration starts the event pump thread: re-synthesize Connected first, then forward events in a loop.
/// Callbacks fire on a single pump thread; calling any mediaservo_link_signal_* API (including close) from within a callback is forbidden.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_signal_on_event(
    s: *mut mediaservo_link_signal_t,
    cb: Option<mediaservo_link_event_cb>, // NULL = unregister (FFI-safe NPO)
    user: *mut c_void,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            set_last_error("mediaservo_link_signal_on_event: null handle");
            return;
        }
        let handle = unsafe { &*s };
        if handle.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_link_signal_on_event: session closed");
            return;
        }
        {
            let mut guard = match handle.cb.lock() {
                Ok(g) => g,
                Err(_) => {
                    set_last_error("mediaservo_link_signal_on_event: lock poisoned");
                    return;
                }
            };
            *guard = cb.map(|cb| (cb, user)); // NULL callback = unregister
        }
        // Start the event pump on first registration (R2).
        let mut pump_guard = match handle.pump.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("mediaservo_link_signal_on_event: lock poisoned");
                return;
            }
        };
        if pump_guard.is_none() {
            // NonNull<T> is Send when T: Send (mediaservo_link_signal_t already has unsafe impl Send).
            // Raw pointers are not Send: pass as usize (value semantics, address move only).
            let raw = s as usize;
            let join =
                std::thread::spawn(move || signal_pump(raw as *mut mediaservo_link_signal_t));
            *pump_guard = Some(join);
        }
    }));
}

/// Close the signaling session and release the handle (idempotent).
///
/// Order (R2): set the closed flag → release the session (all broadcast senders drop, pump recv returns Closed
/// and exits) → join the event pump thread → drop the runtime → free the handle memory.
/// join blocks while the pump is inside a callback — callbacks must return quickly; calling close from a callback is forbidden.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_signal_close(s: *mut mediaservo_link_signal_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(s) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent: already closed
        }
        let session = handle.session.lock().ok().and_then(|mut g| g.take());
        if let Some(session) = session
            && let Err(e) = handle.rt.block_on(session.close())
        {
            set_last_error(format!("mediaservo_link_signal_close: {e}"));
        }
        if let Some(join) = handle.pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join();
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_signal_close: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

// -- Frame bus --

/// Frame bus opaque handle.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception: mediaservo_* prefix)
pub struct mediaservo_link_bus_t {
    bus: std::sync::Mutex<Option<FrameBus>>,
    closed: AtomicBool,
}

// SAFETY: the inner is Mutex<Option<FrameBus>> (iceoryx2 ipc_threadsafe is thread-safe).
unsafe impl Send for mediaservo_link_bus_t {}
unsafe impl Sync for mediaservo_link_bus_t {}

/// Frame stream opaque handle.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception: mediaservo_* prefix)
pub struct mediaservo_link_stream_t {
    stream: FrameStream,
    /// One runtime per stream (recv uses it to block for frames; the frame state lives in the shared inner, not on this runtime).
    rt: tokio::runtime::Runtime,
    /// Shutdown signal: stream_close wakes a blocked recv → None → CLOSED.
    shutdown: Arc<Notify>,
    closed: AtomicBool,
}

// SAFETY: the stream holds Arc<StreamInner> (Mutex + Notify) + a Send runtime.
unsafe impl Send for mediaservo_link_stream_t {}
unsafe impl Sync for mediaservo_link_stream_t {}

/// recv implementation: take a frame or shut down (two-way select; bus close / stream close both wake).
async fn stream_recv_impl(stream: &FrameStream, shutdown: &Notify) -> Option<FrameRef> {
    tokio::select! {
        f = stream.recv() => f,
        _ = shutdown.notified() => None,
    }
}

/// Attach the frame bus (signature verification + ACL + iceoryx2 node, blocking).
///
/// `endpoint` is reserved for Phase 1 (an empty string is fine); `token_pem`/`vk_pem` are the Ed25519
/// capability token / verification key PEM strings (must be non-empty).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_bus_attach(
    endpoint: *const c_char,
    token_pem: *const c_char,
    vk_pem: *const c_char,
    out: *mut *mut mediaservo_link_bus_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if endpoint.is_null() || token_pem.is_null() || vk_pem.is_null() || out.is_null() {
            set_last_error("mediaservo_link_bus_attach: null args");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        let (endpoint, token_pem, vk_pem) = match (cstr(endpoint), cstr(token_pem), cstr(vk_pem)) {
            (Ok(Some(e)), Ok(Some(t)), Ok(Some(v))) => {
                (e.to_string(), t.to_string(), v.to_string())
            }
            (Ok(None), _, _) => {
                set_last_error("mediaservo_link_bus_attach: endpoint required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
            (_, Ok(None), _) => {
                set_last_error("mediaservo_link_bus_attach: token_pem required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
            (_, _, Ok(None)) => {
                set_last_error("mediaservo_link_bus_attach: vk_pem required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
            _ => {
                set_last_error("mediaservo_link_bus_attach: invalid UTF-8");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
        };
        let token = CapabilityToken::from_raw(token_pem); // JWT string (from_pem is for keys only)
        let vk = Ed25519VerifyingKey::from_pem(vk_pem.as_bytes());
        match FrameBus::attach(&endpoint, &token, &vk) {
            Ok(bus) => {
                let handle = Box::new(mediaservo_link_bus_t {
                    bus: std::sync::Mutex::new(Some(bus)),
                    closed: AtomicBool::new(false),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_link_bus_attach: {e}"));
                MEDIASERVO_LINK_ERR_BUS
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_bus_attach: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// Publish one frame (ACL check + SHM loan + send; blocking).
///
/// `meta` is the shared-header mediaservo_frame_meta_t (36B field bag, read field-by-field per R4).
/// `payload` may be NULL if and only if `len == 0` (metadata-only frame).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_bus_publish(
    b: *mut mediaservo_link_bus_t,
    topic: *const c_char,
    payload: *const u8,
    len: usize,
    meta: *const mediaservo_frame_meta_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if b.is_null() || topic.is_null() || meta.is_null() {
            set_last_error("mediaservo_link_bus_publish: null handle/topic/meta");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        if payload.is_null() && len > 0 {
            set_last_error("mediaservo_link_bus_publish: null payload with len > 0");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*b };
        if handle.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_link_bus_publish: bus closed");
            return MEDIASERVO_LINK_ERR_STATE;
        }
        let topic_str = match cstr(topic) {
            Ok(Some(t)) => t.to_string(),
            _ => {
                set_last_error("mediaservo_link_bus_publish: topic required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
        };
        let meta = match meta_from_c(meta) {
            Ok(m) => m,
            Err(()) => {
                set_last_error("mediaservo_link_bus_publish: invalid meta");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
        };
        let payload_slice =
            if len > 0 { unsafe { std::slice::from_raw_parts(payload, len) } } else { &[] };
        let guard = match handle.bus.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("mediaservo_link_bus_publish: lock poisoned");
                return MEDIASERVO_LINK_ERR_INTERNAL;
            }
        };
        let Some(bus) = guard.as_ref() else {
            set_last_error("mediaservo_link_bus_publish: bus closed");
            return MEDIASERVO_LINK_ERR_STATE;
        };
        match bus.publish(&FrameTopic::new(topic_str), payload_slice, &meta) {
            Ok(()) => MEDIASERVO_OK,
            Err(e) => {
                set_last_error(format!("mediaservo_link_bus_publish: {e}"));
                MEDIASERVO_LINK_ERR_BUS
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_bus_publish: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// Subscribe to a topic and create a frame-stream handle (blocking).
///
/// On success `*out` points to the new handle (the caller owns `mediaservo_link_stream_close`).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_bus_subscribe(
    b: *mut mediaservo_link_bus_t,
    topic: *const c_char,
    out: *mut *mut mediaservo_link_stream_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if b.is_null() || topic.is_null() || out.is_null() {
            set_last_error("mediaservo_link_bus_subscribe: null handle/topic/out");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*b };
        if handle.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_link_bus_subscribe: bus closed");
            return MEDIASERVO_LINK_ERR_STATE;
        }
        let topic_str = match cstr(topic) {
            Ok(Some(t)) => t.to_string(),
            _ => {
                set_last_error("mediaservo_link_bus_subscribe: topic required");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
        };
        let guard = match handle.bus.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("mediaservo_link_bus_subscribe: lock poisoned");
                return MEDIASERVO_LINK_ERR_INTERNAL;
            }
        };
        let Some(bus) = guard.as_ref() else {
            set_last_error("mediaservo_link_bus_subscribe: bus closed");
            return MEDIASERVO_LINK_ERR_STATE;
        };
        match bus.subscribe(&FrameTopic::new(topic_str)) {
            Ok(stream) => {
                let stream_handle = Box::new(mediaservo_link_stream_t {
                    stream,
                    rt: new_runtime(),
                    shutdown: Arc::new(Notify::new()),
                    closed: AtomicBool::new(false),
                });
                unsafe { *out = Box::into_raw(stream_handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_link_bus_subscribe: {e}"));
                MEDIASERVO_LINK_ERR_BUS
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_bus_subscribe: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// Blocking frame fetch: metadata is copied into `out_meta`, the payload into `out_data` (up to `cap` bytes),
/// `*out_len` = bytes actually copied (truncated when the frame exceeds cap).
///
/// Returns when a frame arrives or on shutdown: shutdown (stream_close / bus_close) → MEDIASERVO_LINK_ERR_CLOSED.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_bus_recv(
    st: *mut mediaservo_link_stream_t,
    out_meta: *mut mediaservo_frame_meta_t,
    out_data: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if st.is_null() || out_meta.is_null() || out_data.is_null() || out_len.is_null() {
            set_last_error("mediaservo_link_bus_recv: null stream/out");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        if cap == 0 {
            set_last_error("mediaservo_link_bus_recv: cap must be > 0");
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*st };
        if handle.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_link_bus_recv: stream closed");
            return MEDIASERVO_LINK_ERR_CLOSED;
        }
        match handle.rt.block_on(stream_recv_impl(&handle.stream, &handle.shutdown)) {
            Some(frame) => {
                meta_to_c(frame.meta(), out_meta);
                let payload = frame.payload();
                let n = payload.len().min(cap);
                unsafe {
                    ptr::copy_nonoverlapping(payload.as_ptr(), out_data, n);
                    *out_len = n;
                }
                MEDIASERVO_OK
            }
            None => {
                set_last_error("mediaservo_link_bus_recv: stream closed");
                MEDIASERVO_LINK_ERR_CLOSED
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_bus_recv: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// Close the frame-stream handle (idempotent; wakes a blocked recv so it returns CLOSED).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_stream_close(st: *mut mediaservo_link_stream_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if st.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(st) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent: already closed
        }
        handle.shutdown.notify_waiters();
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_stream_close: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// Close the frame-bus handle (idempotent; shuts down all streams, recv returns None → CLOSED).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_bus_close(b: *mut mediaservo_link_bus_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if b.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(b) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent: already closed
        }
        let bus = handle.bus.lock().ok().and_then(|mut g| g.take());
        match bus {
            Some(bus) => match bus.close() {
                Ok(()) => MEDIASERVO_OK,
                Err(e) => {
                    set_last_error(format!("mediaservo_link_bus_close: {e}"));
                    MEDIASERVO_LINK_ERR_BUS
                }
            },
            None => MEDIASERVO_OK,
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_bus_close: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

// -- Common --

/// Details of the last error (thread-safe; returns an empty string when no error).
fn last_error_impl(buf: *mut c_char, len: usize) -> c_int {
    if buf.is_null() || len == 0 {
        return MEDIASERVO_LINK_ERR_INVALID_ARG;
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

/// Last error details.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_last_error(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| last_error_impl(buf, len)))
        .unwrap_or(MEDIASERVO_LINK_ERR_INTERNAL)
}

/// Version info (MAJOR.MINOR.PATCH — D241 soname semantics).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_version(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if buf.is_null() || len == 0 {
            return MEDIASERVO_LINK_ERR_INVALID_ARG;
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
    .unwrap_or(MEDIASERVO_LINK_ERR_INTERNAL)
}

// -- Shared C type mirror (mediaservo_common.h, R4 field bag) --

/// C-side frame metadata (#pragma pack(1) 36B, same wire format as FrameMeta::encode).
///
/// Production paths always go through a 36B copy + FrameMeta::decode/encode field-by-field read/write;
/// never reinterpret the whole block (packed misalignment + endianness risk).
#[allow(non_camel_case_types)]
#[repr(C, packed)]
pub struct mediaservo_frame_meta_t {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub format: u8,
    pub version: u8,
    pub is_keyframe: u8,
    pub reserved: u8,
    pub ts_mono_ns: u64,
    pub ts_epoch_ns: u64,
}

#[cfg(test)]
mod tests {
    /// Tests in this file share the process-level static LAST_ERROR (C ABI semantics) — null-handle negative
    /// cases write it implicitly, so explicit-read assertions stomp on each other = flaky (same surgery as field-c 09-20; link-c 09-20 gate
    /// caught last_error_roundtrip racing; deck-c prevented in the same batch). All tests in this file serialize through this lock.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::*;

    /// Synthesize an unconnected signal handle (session None; for closed/STATE path tests only).
    /// Synthesize an unconnected signal handle (session None; closed flag can be preset).
    /// Note: calling after close is UB (contract); each test closes a given pointer only once.
    fn signal_handle(closed: bool) -> *mut mediaservo_link_signal_t {
        Box::into_raw(Box::new(mediaservo_link_signal_t {
            session: std::sync::Mutex::new(None),
            rt: new_runtime(),
            closed: AtomicBool::new(closed),
            room_id: "test".into(),
            events_rx: std::sync::Mutex::new(None),
            cb: std::sync::Mutex::new(None),
            pump: std::sync::Mutex::new(None),
        }))
    }

    /// Synthesize an unattached bus handle (bus None; closed flag can be preset).
    fn bus_handle(closed: bool) -> *mut mediaservo_link_bus_t {
        Box::into_raw(Box::new(mediaservo_link_bus_t {
            bus: std::sync::Mutex::new(None),
            closed: AtomicBool::new(closed),
        }))
    }

    /// Synthesize an unattached bus handle (bus None).
    fn bus_handle_closed() -> *mut mediaservo_link_bus_t {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        Box::into_raw(Box::new(mediaservo_link_bus_t {
            bus: std::sync::Mutex::new(None),
            closed: AtomicBool::new(false),
        }))
    }

    #[test]
    fn last_error_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Global state races across tests: clear before setting (do not assume other tests haven't written)
        set_last_error("");
        set_last_error("test error");
        let mut buf = [0u8; 64];
        let rc = mediaservo_link_last_error(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert_eq!(s, "test error");
    }

    #[test]
    fn version_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = [0u8; 32];
        let rc = mediaservo_link_version(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(s.starts_with("0.1."), "version: {s}");
    }

    #[test]
    fn connect_null_cfg_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_link_signal_connect(ptr::null(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
    }

    #[test]
    fn connect_small_struct_size_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Caller compiled against an older header: struct_size too small → explicit error (R3)
        let cfg = mediaservo_link_signal_config_t { struct_size: 1, ..Default::default() };
        let mut out: *mut mediaservo_link_signal_t = ptr::null_mut();
        let rc = mediaservo_link_signal_connect(&cfg, &mut out);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn connect_missing_required_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // struct_size valid but url/psk/room empty → required-field error
        let cfg = mediaservo_link_signal_config_t::default();
        let mut out: *mut mediaservo_link_signal_t = ptr::null_mut();
        let rc = mediaservo_link_signal_connect(&cfg, &mut out);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn connect_bad_role_fails_before_network() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Invalid role string → rejected before connecting (no network touch)
        let url = c"ws://127.0.0.1:1/ws";
        let psk = c"psk";
        let room = c"room";
        let role = c"Bogus";
        let cfg = mediaservo_link_signal_config_t {
            struct_size: MEDIASERVO_LINK_SIGNAL_CONFIG_MIN_SIZE,
            url: url.as_ptr(),
            psk: psk.as_ptr(),
            room: room.as_ptr(),
            role: role.as_ptr(),
        };
        let mut out: *mut mediaservo_link_signal_t = ptr::null_mut();
        let rc = mediaservo_link_signal_connect(&cfg, &mut out);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn send_null_handle_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_link_signal_send(ptr::null_mut(), c"{}".as_ptr(), 2);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
    }

    #[test]
    fn send_empty_message_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = signal_handle(false);
        let rc = mediaservo_link_signal_send(s, c"{}".as_ptr(), 0);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
        unsafe { drop(Box::from_raw(s)) };
    }

    #[test]
    fn send_without_session_returns_state() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = signal_handle(false);
        // Valid SignalingMessage (frame) but no session → STATE
        let msg = c"{\"type\":\"frame\",\"room_id\":\"r\",\"codec\":\"h264\",\"sequence\":1,\"is_keyframe\":true,\"data_base64\":\"\"}";
        let rc = mediaservo_link_signal_send(s, msg.as_ptr(), msg.to_bytes().len());
        assert_eq!(rc, MEDIASERVO_LINK_ERR_STATE);
        assert_eq!(mediaservo_link_signal_close(s), MEDIASERVO_OK);
    }

    #[test]
    fn send_after_closed_flag_returns_state() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = signal_handle(true);
        // closed flag already set → STATE (does not touch the session)
        let rc = mediaservo_link_signal_send(s, c"{}".as_ptr(), 2);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_STATE);
        assert_eq!(mediaservo_link_signal_close(s), MEDIASERVO_OK);
    }

    #[test]
    fn close_null_is_ok() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mediaservo_link_signal_close(ptr::null_mut()), MEDIASERVO_OK);
        assert_eq!(mediaservo_link_bus_close(ptr::null_mut()), MEDIASERVO_OK);
        assert_eq!(mediaservo_link_stream_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn on_event_null_handle_noop() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        mediaservo_link_signal_on_event(ptr::null_mut(), None, ptr::null_mut());
    }

    #[test]
    fn bus_attach_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: *mut mediaservo_link_bus_t = ptr::null_mut();
        let rc =
            mediaservo_link_bus_attach(ptr::null(), c"token".as_ptr(), c"vk".as_ptr(), &mut out);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn bus_publish_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_link_bus_publish(
            ptr::null_mut(),
            c"camera/0".as_ptr(),
            ptr::null(),
            0,
            ptr::null(),
        );
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
    }

    #[test]
    fn bus_publish_without_bus_returns_state() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let b = bus_handle(false);
        let meta = mediaservo_frame_meta_t {
            seq: 1,
            width: 320,
            height: 240,
            format: 1,
            version: 0,
            is_keyframe: 1,
            reserved: 0,
            ts_mono_ns: 0,
            ts_epoch_ns: 0,
        };
        let rc = mediaservo_link_bus_publish(b, c"camera/0".as_ptr(), ptr::null(), 0, &meta);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_STATE);
        assert_eq!(mediaservo_link_bus_close(b), MEDIASERVO_OK);
    }

    #[test]
    fn bus_publish_after_closed_flag_returns_state() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let b = bus_handle(true);
        let meta = mediaservo_frame_meta_t {
            seq: 1,
            width: 320,
            height: 240,
            format: 1,
            version: 0,
            is_keyframe: 1,
            reserved: 0,
            ts_mono_ns: 0,
            ts_epoch_ns: 0,
        };
        let rc = mediaservo_link_bus_publish(b, c"camera/0".as_ptr(), ptr::null(), 0, &meta);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_STATE);
        assert_eq!(mediaservo_link_bus_close(b), MEDIASERVO_OK);
    }

    #[test]
    fn bus_subscribe_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc =
            mediaservo_link_bus_subscribe(ptr::null_mut(), c"camera/0".as_ptr(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
    }

    #[test]
    fn bus_recv_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_link_bus_recv(
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
        );
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
    }

    #[test]
    fn frame_meta_c_layout_matches_wire() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // R4: fill a repr(C, packed) mirror struct field-by-field → 36B copy + decode →
        // compare byte-for-byte against Rust FrameMeta::encode
        assert_eq!(size_of::<mediaservo_frame_meta_t>(), FrameMeta::WIRE_LEN);
        let c = mediaservo_frame_meta_t {
            seq: 0x0102030405060708,
            width: 1920,
            height: 1080,
            format: 1,
            version: 0,
            is_keyframe: 1,
            reserved: 0,
            ts_mono_ns: 0xAABBCCDDEEFF0011,
            ts_epoch_ns: 0x1122334455667788,
        };
        let rust = FrameMeta {
            seq: c.seq,
            width: c.width,
            height: c.height,
            format: c.format,
            version: c.version,
            is_keyframe: c.is_keyframe != 0,
            ts_mono_ns: c.ts_mono_ns,
            ts_epoch_ns: c.ts_epoch_ns,
        };
        // Go through the real production path: 36B copy + decode (same mechanism as meta_from_c)
        let mut buf = [0u8; FrameMeta::WIRE_LEN];
        unsafe {
            ptr::copy_nonoverlapping(
                &c as *const _ as *const u8,
                buf.as_mut_ptr(),
                FrameMeta::WIRE_LEN,
            );
        }
        let decoded = FrameMeta::decode(&buf).expect("decode");
        assert_eq!(decoded, rust);
        assert_eq!(decoded.encode(), rust.encode());
        // The write-back path (same mechanism as meta_to_c) is also byte-for-byte identical
        let mut back = mediaservo_frame_meta_t {
            seq: 0,
            width: 0,
            height: 0,
            format: 0,
            version: 0,
            is_keyframe: 0,
            reserved: 0,
            ts_mono_ns: 0,
            ts_epoch_ns: 0,
        };
        let mut out_buf = [0u8; FrameMeta::WIRE_LEN];
        meta_to_c(&decoded, &mut back);
        unsafe {
            ptr::copy_nonoverlapping(
                &back as *const _ as *const u8,
                out_buf.as_mut_ptr(),
                FrameMeta::WIRE_LEN,
            );
        }
        assert_eq!(out_buf, decoded.encode());
    }

    #[test]
    fn stream_close_null_ok() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mediaservo_link_stream_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn bus_close_null_ok() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mediaservo_link_bus_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    /// bus positive path: attach (signature verify) → subscribe → publish → recv asserting meta/payload.
    /// Two nodes (ACL matrix: Capture publishes camera/* only, Processor subscribes camera/* only);
    /// Subscribe before publishing (iceoryx2 does not replay history frames to new subscribers).
    /// Reuse the mediaservo-link e2e test key pair (D235 test fixture).
    #[test]
    fn bus_attach_publish_subscribe_recv_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
        const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";
        use mediaservo_link::{
            CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, NodeAcl, NodeId, Role,
        };

        let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
        let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
        let endpoint = CString::new("").unwrap();
        let suffix = std::process::id();
        let topic = CString::new(format!("camera/ct/{suffix}/front/raw")).unwrap();
        let vk_s = CString::new(vk.pem()).unwrap();
        let token_for = |node: &str, role: Role| {
            let acl = NodeAcl::for_role(NodeId::new(node), role);
            CString::new(CapabilityToken::sign(&acl, 3600, &sk).expect("sign").as_str()).unwrap()
        };

        // 1. Capture publisher
        let mut bus: *mut mediaservo_link_bus_t = ptr::null_mut();
        let rc = mediaservo_link_bus_attach(
            endpoint.as_ptr(),
            token_for("ct-capture", Role::Capture).as_ptr(),
            vk_s.as_ptr(),
            &mut bus,
        );
        assert_eq!(rc, MEDIASERVO_OK, "attach capture");
        assert!(!bus.is_null());

        // 2. Processor subscriber
        let mut proc_bus: *mut mediaservo_link_bus_t = ptr::null_mut();
        let rc = mediaservo_link_bus_attach(
            endpoint.as_ptr(),
            token_for("ct-processor", Role::Processor).as_ptr(),
            vk_s.as_ptr(),
            &mut proc_bus,
        );
        assert_eq!(rc, MEDIASERVO_OK, "attach processor");
        assert!(!proc_bus.is_null());

        // 3. Subscribe before publishing (no history-frame replay)
        let mut stream: *mut mediaservo_link_stream_t = ptr::null_mut();
        let rc = mediaservo_link_bus_subscribe(proc_bus, topic.as_ptr(), &mut stream);
        assert_eq!(rc, MEDIASERVO_OK, "subscribe");
        assert!(!stream.is_null());

        // 4. Publish
        let meta = mediaservo_frame_meta_t {
            seq: 7,
            width: 640,
            height: 480,
            format: 1,
            version: FrameMeta::WIRE_VERSION,
            is_keyframe: 1,
            reserved: 0,
            ts_mono_ns: 1000,
            ts_epoch_ns: 2000,
        };
        let payload = [0xAAu8; 64];
        let rc = mediaservo_link_bus_publish(
            bus,
            topic.as_ptr(),
            payload.as_ptr(),
            payload.len(),
            ptr::addr_of!(meta),
        );
        assert_eq!(rc, MEDIASERVO_OK, "publish");

        // 5. Receive and assert meta/payload
        let mut out_meta = mediaservo_frame_meta_t {
            seq: 0,
            width: 0,
            height: 0,
            format: 0,
            version: 0,
            is_keyframe: 0,
            reserved: 0,
            ts_mono_ns: 0,
            ts_epoch_ns: 0,
        };
        let mut buf = [0u8; 128];
        let mut out_len: usize = 0;
        let rc = mediaservo_link_bus_recv(
            stream,
            ptr::addr_of_mut!(out_meta),
            buf.as_mut_ptr(),
            buf.len(),
            &mut out_len,
        );
        assert_eq!(rc, MEDIASERVO_OK, "recv");
        // Reading packed-struct fields requires read_unaligned (E0793)
        unsafe {
            assert_eq!(ptr::addr_of!(out_meta.seq).read_unaligned(), 7);
            assert_eq!(ptr::addr_of!(out_meta.width).read_unaligned(), 640);
            assert_eq!(ptr::addr_of!(out_meta.height).read_unaligned(), 480);
            assert_eq!(ptr::addr_of!(out_meta.format).read_unaligned(), 1);
            assert_eq!(ptr::addr_of!(out_meta.is_keyframe).read_unaligned(), 1);
        }
        assert_eq!(out_len, 64);
        assert_eq!(&buf[..64], &payload);

        // 6. Close
        assert_eq!(mediaservo_link_stream_close(stream), MEDIASERVO_OK);
        assert_eq!(mediaservo_link_bus_close(proc_bus), MEDIASERVO_OK);
        assert_eq!(mediaservo_link_bus_close(bus), MEDIASERVO_OK);
    }
}
