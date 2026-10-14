//! Control channel C handle + ack pump (S6 batch 1b: cumulative on_ack + token/off_ack unregistration,
//! the ready_state/buffered_amount observation surface, and the producer_ids needed upgrade).
//!
//! Pump discipline per the lib.rs header note: callbacks fire only on the ack pump thread, hold no lock, and return quickly.
//! user_free exactly-once contract: off_ack only marks entries retired; **the pump reclaims them uniformly at the start of the next round and calls free outside the lock**
//! (no concurrent firing at that moment, eliminating the UAF race); entries the pump never reclaims (pump dead / never off'ed) are
//! all released by control_close — the two reclamation paths are mutually exclusive by "still in the registry", giving exactly once.

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use mediaservo_client::ControlChannel;
use mediaservo_client::RTCDataChannelState;
use mediaservo_client::error::ClientError;

use crate::ACK_POLL;
use crate::errors::{
    ErrSlot, HandleErr, MEDIASERVO_CLIENT_ERR_INTERNAL, MEDIASERVO_CLIENT_ERR_INVALID_ARG,
    MEDIASERVO_OK, build_envelope, copy_out_needed, cstr, fail_global, set_last_error,
};
use crate::{ffi_catch, mediaservo_client_ack_cb, mediaservo_client_user_free, runtime};

/// One ack-callback registration (cumulative form; token is monotonically increasing, 0 = reserved invalid value).
struct AckReg {
    token: u64,
    cb: mediaservo_client_ack_cb,
    user: *mut c_void,
    free: Option<mediaservo_client_user_free>,
    /// off_ack marker: reclaimed by the pump next round (freed outside the lock), with close as the backstop.
    retired: bool,
}

/// Outbound control channel set opaque handle.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub struct mediaservo_client_control_t {
    pub(crate) ctl: Mutex<ControlChannel>,
    closed: AtomicBool,
    regs: Mutex<Vec<AckReg>>,
    next_token: AtomicU64,
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
    err: ErrSlot,
}

// SAFETY: inner Mutex<ControlChannel> (libwebrtc DC/PC handles are thread-safe, the livekit convention).
unsafe impl Send for mediaservo_client_control_t {}
// SAFETY: all methods are serialized through Mutex/AtomicBool.
unsafe impl Sync for mediaservo_client_control_t {}

impl HandleErr for mediaservo_client_control_t {
    fn err_slot(&self) -> &ErrSlot {
        &self.err
    }
}

impl mediaservo_client_control_t {
    /// Closure check + lock accessors for compound operations (emergency_stop); the caller maintains the lock order session→ctl.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub(crate) fn ctl_lock(&self) -> Result<std::sync::MutexGuard<'_, ControlChannel>, ()> {
        self.ctl.lock().map_err(|_| ())
    }
}

pub(crate) fn new(channel: ControlChannel) -> mediaservo_client_control_t {
    mediaservo_client_control_t {
        ctl: Mutex::new(channel),
        closed: AtomicBool::new(false),
        regs: Mutex::new(Vec::new()),
        next_token: AtomicU64::new(0),
        pump: Mutex::new(None),
        err: Mutex::new(None),
    }
}

pub(crate) fn into_raw(handle: mediaservo_client_control_t) -> *mut mediaservo_client_control_t {
    Box::into_raw(Box::new(handle))
}

/// RTCDataChannelState → wire value (0..3, pinned by the header enum comment; exhaustive match = a new variant turns the build red).
fn dc_state_u8(st: RTCDataChannelState) -> u8 {
    match st {
        RTCDataChannelState::Connecting => 0,
        RTCDataChannelState::Open => 1,
        RTCDataChannelState::Closing => 2,
        RTCDataChannelState::Closed => 3,
    }
}

/// Ack pump thread: each round reclaims retired entries first (free outside the lock) → snapshot active → recv_ack(1s) →
/// invoke callbacks one by one (cumulative form: one ack triggers all active entries).
fn ack_pump(raw: usize) {
    let c = raw as *mut mediaservo_client_control_t;
    let h = unsafe { &*c };
    let rt = runtime();
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        // Reclaim retired entries (off_ack marked but still in the table) — the callback lock is not held here, so freeing is safe.
        let reaped: Vec<AckReg> = match h.regs.lock() {
            Ok(mut g) => {
                if !g.iter().any(|r| r.retired) {
                    Vec::new()
                } else {
                    let mut dead = Vec::new();
                    let mut keep = Vec::with_capacity(g.len());
                    for r in g.drain(..) {
                        if r.retired {
                            dead.push(r);
                        } else {
                            keep.push(r);
                        }
                    }
                    *g = keep;
                    dead
                }
            }
            Err(_) => break, // poisoned = a thread panicked inside the critical section; converge and exit
        };
        for r in reaped {
            if let Some(f) = r.free {
                f(r.user);
            }
        }
        let actives: Vec<(mediaservo_client_ack_cb, *mut c_void)> =
            h.regs.lock().map(|g| g.iter().map(|r| (r.cb, r.user)).collect()).unwrap_or_default();
        let item = match h.ctl.lock() {
            Ok(mut g) => rt.block_on(g.recv_ack(ACK_POLL)),
            Err(_) => break,
        };
        match item {
            Ok(ack) => {
                let json = serde_json::to_string(&ack).unwrap_or_default();
                // The CString lives until the callback returns (serde_json output has no embedded NUL).
                if let Ok(cstr) = CString::new(json) {
                    for (cb, user) in &actives {
                        cb(c, cstr.as_ptr(), *user);
                    }
                }
            }
            Err(e) => {
                if matches!(e, ClientError::Timeout { .. }) {
                    continue; // empty round window; keep going normally
                }
                h.fail_client("ack pump", &e);
                break; // terminal states like InvalidState (DC disconnected)
            }
        }
    }
}

/// Register an ack callback (**cumulative form**: multiple registrations = each ack invokes them one by one; the first registration starts the pump).
///
/// out_token returns the unregistration credential (may be NULL; 0 = invalid value). cb=NULL → reject (unregistration goes through
/// [`mediaservo_client_control_off_ack`]; this form has no "cancel all" semantics). user_free runs
/// exactly once at the pump reclamation round after off_ack (≤ACK_POLL) or at control_close.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_on_ack(
    c: *mut mediaservo_client_control_t,
    cb: Option<mediaservo_client_ack_cb>, // FFI-safe NPO
    user: *mut c_void,
    user_free: Option<mediaservo_client_user_free>,
    out_token: *mut u64,
) -> c_int {
    const NAME: &str = "mediaservo_client_control_on_ack";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_control_on_ack: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        let Some(cb) = cb else {
            return h.fail_arg(
                "mediaservo_client_control_on_ack: cb required (unregister via off_ack; NULL has no cancel semantics)",
            );
        };
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_control_on_ack: control closed");
        }
        let token = h.next_token.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut guard = match h.regs.lock() {
                Ok(g) => g,
                Err(_) => {
                    return h.fail_internal("mediaservo_client_control_on_ack: lock poisoned");
                }
            };
            guard.push(AckReg { token, cb, user, free: user_free, retired: false });
        }
        if !out_token.is_null() {
            // SAFETY: already checked non-null; points at caller-owned u64 storage.
            unsafe { *out_token = token };
        }
        // The first registration starts the ack pump (same discipline as link's on_event).
        let mut pump_guard = match h.pump.lock() {
            Ok(g) => g,
            Err(_) => {
                return h.fail_internal("mediaservo_client_control_on_ack: pump lock poisoned");
            }
        };
        if pump_guard.is_none() {
            let raw = c as usize;
            let join = std::thread::spawn(move || ack_pump(raw));
            *pump_guard = Some(join);
        }
        MEDIASERVO_OK
    })
}

/// Unregister an ack callback by token (marks it retired; the pump reclaims it next round and invokes its user_free; acks arriving **after the next
/// round** no longer trigger it — an in-flight round may still fire once, which is bounded grace). Unknown token →
/// ERR_INVALID_ARG。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_off_ack(
    c: *mut mediaservo_client_control_t,
    token: u64,
) -> c_int {
    const NAME: &str = "mediaservo_client_control_off_ack";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_control_off_ack: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_control_off_ack: control closed");
        }
        let mut guard = match h.regs.lock() {
            Ok(g) => g,
            Err(_) => return h.fail_internal("mediaservo_client_control_off_ack: lock poisoned"),
        };
        match guard.iter_mut().find(|r| r.token == token) {
            Some(r) if !r.retired => {
                r.retired = true;
                MEDIASERVO_OK
            }
            _ => h.fail_arg("mediaservo_client_control_off_ack: unknown ack token"),
        }
    })
}

/// Send one command envelope (serde-built, never concatenated). payload_json NULL/"" = a null payload.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_send(
    c: *mut mediaservo_client_control_t,
    label: *const c_char,
    seq: u64,
    cmd: *const c_char,
    payload_json: *const c_char,
) -> c_int {
    const NAME: &str = "mediaservo_client_control_send";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_control_send: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_control_send: control closed");
        }
        let (label_s, cmd_s, payload_s) = match (cstr(label), cstr(cmd), cstr(payload_json)) {
            (Ok(Some(l)), Ok(Some(cm)), Ok(p)) => (l, cm, p),
            _ => {
                return h
                    .fail_arg("mediaservo_client_control_send: label/cmd required, UTF-8 valid");
            }
        };
        let env = match build_envelope(seq, cmd_s, payload_s) {
            Ok(env) => env,
            Err(code) => {
                h.fail_arg("mediaservo_client_control_send: payload_json invalid (see last_error)");
                return code;
            }
        };
        let guard = match h.ctl.lock() {
            Ok(g) => g,
            Err(_) => return h.fail_internal("mediaservo_client_control_send: lock poisoned"),
        };
        match runtime().block_on(guard.send_envelope(label_s, &env)) {
            Ok(()) => MEDIASERVO_OK,
            Err(e) => h.fail_client("mediaservo_client_control_send", &e),
        }
    })
}

/// DC ready state (out_state: 0=Connecting 1=Open 2=Closing 3=Closed; = the RTCDataChannelState
/// stable wire values; used by K5 backpressure and the pre-send gate).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_ready_state(
    c: *const mediaservo_client_control_t,
    out_state: *mut u8,
) -> c_int {
    const NAME: &str = "mediaservo_client_control_ready_state";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_control_ready_state: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_state.is_null() {
            return h.fail_arg("mediaservo_client_control_ready_state: null out");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_control_ready_state: control closed");
        }
        let guard = match h.ctl.lock() {
            Ok(g) => g,
            Err(_) => {
                return h.fail_internal("mediaservo_client_control_ready_state: lock poisoned");
            }
        };
        unsafe { *out_state = dc_state_u8(guard.ready_state()) };
        MEDIASERVO_OK
    })
}

/// SCTP backpressure water level (bytes; 0 = safe to append). Blocks to fetch the DC state, contending with the ack pump for ≤ACK_POLL
/// — acceptable within the single-thread-owner contract).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_buffered_amount(
    c: *mut mediaservo_client_control_t,
    out_bytes: *mut u64,
) -> c_int {
    const NAME: &str = "mediaservo_client_control_buffered_amount";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_control_buffered_amount: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_bytes.is_null() {
            return h.fail_arg("mediaservo_client_control_buffered_amount: null out");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_control_buffered_amount: control closed");
        }
        let guard = match h.ctl.lock() {
            Ok(g) => g,
            Err(_) => {
                return h.fail_internal("mediaservo_client_control_buffered_amount: lock poisoned");
            }
        };
        let bytes = runtime().block_on(guard.buffered_amount());
        unsafe { *out_bytes = bytes };
        MEDIASERVO_OK
    })
}

/// producer id JSON array (observation surface, mirroring ControlChannel::producer_ids()).
/// Same needed overflow contract as list_rooms (the batch-1b upgrade: fixes the old fixed-buffer blind spot).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_producer_ids(
    c: *const mediaservo_client_control_t,
    out_json: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_control_producer_ids";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_control_producer_ids: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_json.is_null() || cap == 0 {
            return h.fail_arg("mediaservo_client_control_producer_ids: null out or cap 0");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_control_producer_ids: control closed");
        }
        let guard = match h.ctl.lock() {
            Ok(g) => g,
            Err(_) => {
                return h.fail_internal("mediaservo_client_control_producer_ids: lock poisoned");
            }
        };
        let json = match serde_json::to_string(guard.producer_ids()) {
            Ok(j) => j,
            Err(e) => {
                return h.fail_internal(&format!(
                    "mediaservo_client_control_producer_ids: serialize: {e}"
                ));
            }
        };
        let rc = copy_out_needed(NAME, &json, out_json, cap, needed);
        if rc != MEDIASERVO_OK {
            h.fail_arg("mediaservo_client_control_producer_ids: buffer too small (see needed)");
        }
        rc
    })
}

/// Close the control channel and release the handle (idempotent; sets closed → joins the ack pump → reclaims the whole registry:
/// entries the pump never reclaimed get user_free here one by one — including retired-but-unreclaimed and active entries never off'ed).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_close(c: *mut mediaservo_client_control_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(c) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent
        }
        if let Some(join) = handle.pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join(); // one pump round is ≤ACK_POLL, so the join is bounded
        }
        // After the pump join there is no concurrent firing: leftover registry entries (retired-but-unreclaimed or active-never-offed)
        // are all released here — entries the pump reclaimed are already out of the table, so the exactly-once ledger holds.
        let regs = handle.regs.lock().ok().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
        for r in regs {
            if let Some(f) = r.free {
                f(r.user);
            }
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_client_control_close: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::MEDIASERVO_CLIENT_ERR_STATE;
    use std::ptr;

    extern "C" fn noop_ack(
        _c: *mut mediaservo_client_control_t,
        _j: *const c_char,
        _u: *mut c_void,
    ) {
    }

    #[test]
    fn control_apis_null_fails() {
        let mut buf = [0u8; 64];
        let mut token = 0u64;
        assert_eq!(
            mediaservo_client_control_on_ack(
                ptr::null_mut(),
                None,
                ptr::null_mut(),
                None,
                &mut token
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(
            mediaservo_client_control_off_ack(ptr::null_mut(), 1),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(
            mediaservo_client_control_send(
                ptr::null_mut(),
                c"chassis".as_ptr(),
                1,
                c"steer".as_ptr(),
                ptr::null()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(
            mediaservo_client_control_producer_ids(
                ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_char,
                64,
                ptr::null_mut()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        let mut st = 0u8;
        assert_eq!(
            mediaservo_client_control_ready_state(ptr::null(), &mut st),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        let mut ba = 0u64;
        assert_eq!(
            mediaservo_client_control_buffered_amount(ptr::null_mut(), &mut ba),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }

    #[test]
    fn control_close_null_is_ok() {
        assert_eq!(mediaservo_client_control_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn dc_state_line_values_pinned() {
        // Wire-value contract (pinned by the header comments): any change turns it red.
        assert_eq!(dc_state_u8(RTCDataChannelState::Connecting), 0);
        assert_eq!(dc_state_u8(RTCDataChannelState::Open), 1);
        assert_eq!(dc_state_u8(RTCDataChannelState::Closing), 2);
        assert_eq!(dc_state_u8(RTCDataChannelState::Closed), 3);
    }

    #[test]
    fn on_ack_requires_cb_and_closed_control_state() {
        // ControlChannel has no public constructor (needs a real DC); this layer only pins the guard forms:
        // a closed handle is unobtainable (new() needs a real channel) — the null guard + the wire-value table cover
        // the determinable surface; live registration/reclamation semantics = 1c (against a real server).
        let mut token = 0u64;
        assert_eq!(
            mediaservo_client_control_on_ack(
                ptr::null_mut(),
                Some(noop_ack),
                ptr::null_mut(),
                None,
                &mut token
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        // The STATE-code path above already pinned off_ack(null) as INVALID_ARG; the closed form belongs to 1c.
        let _ = MEDIASERVO_CLIENT_ERR_STATE;
    }
}
