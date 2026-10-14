//! Single video-consumer C handle (the C face of S6 batch 1b/K4): one independent frame pump per consumer, no
//! cross-consumer coupling. Handles are produced by `mediaservo_client_session_consume` and closed independently.
//!
//! Pump discipline: the std Mutex is held across the `timeout(CONSUMER_POLL, recv)` wait — the worst-case lock
//! contention delay for stats/close is CONSUMER_POLL (250ms, bounded); close sets closed → takes the Consumer away
//! (the receiver dies with the drop, K1 replay declares that consumer dead) → join the pump → user_free exactly once.

use std::os::raw::{c_char, c_int, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mediaservo_client::Consumer;
use tokio::time::timeout;

use crate::errors::{
    ErrSlot, HandleErr, MEDIASERVO_CLIENT_ERR_INVALID_ARG, MEDIASERVO_OK, copy_out_needed,
    fail_global,
};
use crate::{
    ffi_catch, mediaservo_client_frame_cb, mediaservo_client_frame_t, mediaservo_client_session_t,
    mediaservo_client_user_free, runtime,
};

/// Frame-pump idle re-check window (polling granularity for closed / channel death; also the worst-case close/stats lock delay).
const CONSUMER_POLL: Duration = Duration::from_millis(250);

/// QOE snapshot C layout (mirror of client.h `mediaservo_client_qoe_snapshot_t`;
/// field order is frozen — ABI additive-only discipline; new fields append at the end).
#[repr(C)]
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub struct mediaservo_client_qoe_snapshot_t {
    pub struct_size: u32,
    pub fps: u32,
    pub kbps: u32,
    pub lost_percent: f32,
    pub decoded_total: u64,
    pub avg_decode_ms: u32,
    pub jitter_ms: u32,
    pub rtt_ms: u32,
    pub width: u32,
    pub height: u32,
    pub epoch: u32,
}

/// QOE callback C function pointer.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception — same as qoe_snapshot_t/consumer_t)
pub type mediaservo_client_qoe_cb =
    extern "C" fn(*const mediaservo_client_qoe_snapshot_t, *mut c_void);

/// Send/Sync wrapper over `*mut c_void` — the C callback contract: **the caller guarantees the thread safety
/// of the user context** (client.h QOE section note; the SDK tick thread invokes cb).
#[derive(Clone, Copy)]
struct CbUser(*mut c_void);
// SAFETY: C ABI contract — caller-side thread safety of the user context (explicit in client.h).
unsafe impl Send for CbUser {}
unsafe impl Sync for CbUser {}

/// Single video-consumer opaque handle.
#[allow(non_camel_case_types)] // C ABI naming (C6 exception)
pub struct mediaservo_client_consumer_t {
    /// First frame-callback parameter passed through (**never dereferenced** — the ownership contract makes the caller guarantee the session does not outlive
    /// the consumer; see the client.h lifecycle section).
    session: *mut mediaservo_client_session_t,
    producer_id: String,
    consumer: Mutex<Option<Consumer>>,
    closed: AtomicBool,
    cb: mediaservo_client_frame_cb,
    user: *mut c_void,
    free: Option<mediaservo_client_user_free>,
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// QOE tick task handle — close aborts it, releasing the trampoline closure
    /// (proven by lsan.1096280: on process exit the tokio runtime does not shut down → a 64B Box leaks per registration).
    qoe_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    err: ErrSlot,
}

// SAFETY: all cross-thread access to consumer/cb/user goes through Mutex/AtomicBool; the session pointer is only moved,
// never dereferenced.
unsafe impl Send for mediaservo_client_consumer_t {}
// SAFETY: same as above.
unsafe impl Sync for mediaservo_client_consumer_t {}

impl HandleErr for mediaservo_client_consumer_t {
    fn err_slot(&self) -> &ErrSlot {
        &self.err
    }
}

/// Assembled by session_consume: build the handle + start the frame pump, returning the raw handle.
pub(crate) fn spawn(
    session: *mut mediaservo_client_session_t,
    consumer: Consumer,
    producer_id: String,
    cb: mediaservo_client_frame_cb,
    user: *mut c_void,
    free: Option<mediaservo_client_user_free>,
) -> *mut mediaservo_client_consumer_t {
    let handle = Box::new(mediaservo_client_consumer_t {
        session,
        producer_id,
        consumer: Mutex::new(Some(consumer)),
        closed: AtomicBool::new(false),
        cb,
        user,
        free,
        pump: Mutex::new(None),
        qoe_task: Mutex::new(None),
        err: Mutex::new(None),
    });
    let raw = Box::into_raw(handle);
    let raw_usize = raw as usize;
    let join = std::thread::spawn(move || pump_loop(raw_usize));
    // The pump may already have finished (closed race / transient channel death) — the store still holds and the join converges.
    if let Ok(mut g) = unsafe { &*raw }.pump.lock() {
        *g = Some(join);
    }
    raw
}

/// Frame-pump thread: this consumer's frame channel → C callback (the frame data Vec outlives the callback call site; data is valid only inside the callback).
fn pump_loop(raw: usize) {
    let c = raw as *mut mediaservo_client_consumer_t;
    let h = unsafe { &*c };
    let rt = runtime();
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        // Note: `timeout(..)` must be constructed inside the async block (an outside argument on a context-less thread
        //        panics — same lesson as the lib.rs video_pump).
        let frame = match h.consumer.lock() {
            Ok(mut g) => match g.as_mut() {
                Some(cons) => {
                    rt.block_on(async { timeout(CONSUMER_POLL, cons.frames().recv()).await })
                }
                None => break, // already taken by close
            },
            Err(_) => break, // poisoned = a thread panicked inside the critical section; converge and exit
        };
        match frame {
            Ok(Some(f)) => {
                let cframe = mediaservo_client_frame_t {
                    width: f.width,
                    height: f.height,
                    ts_us: f.ts_us,
                    data: f.data.as_ptr(),
                    len: f.data.len(),
                };
                (h.cb)(h.session, &cframe, h.user);
            }
            Ok(None) => break, // frame channel gone (session closed / this consumer withdrawn) — normal convergence
            Err(_) => continue, // CONSUMER_POLL idle tick; re-check closed
        }
    }
}

/// This consumer's producer id (needed overflow contract; ids are short, cap 256 is enough).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_consumer_id(
    c: *const mediaservo_client_consumer_t,
    out: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_consumer_id";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_consumer_id: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out.is_null() || cap == 0 {
            return h.fail_arg("mediaservo_client_consumer_id: null out or cap 0");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_consumer_id: consumer closed");
        }
        let rc = copy_out_needed(NAME, &h.producer_id, out, cap, needed);
        if rc != MEDIASERVO_OK {
            h.fail_arg("mediaservo_client_consumer_id: buffer too small (see needed)");
        }
        rc
    })
}

/// This consumer's video stats JSON (K4 per-consumer reading, not the session aggregate; the key table matches session_video_stats).
/// Same needed overflow contract as list_rooms.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_consumer_stats(
    c: *mut mediaservo_client_consumer_t,
    out_json: *mut c_char,
    cap: usize,
    needed: *mut usize,
) -> c_int {
    const NAME: &str = "mediaservo_client_consumer_stats";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_consumer_stats: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if out_json.is_null() || cap == 0 {
            return h.fail_arg("mediaservo_client_consumer_stats: null out or cap 0");
        }
        if h.closed.load(Ordering::SeqCst) {
            return h.fail_state("mediaservo_client_consumer_stats: consumer closed");
        }
        let guard = match h.consumer.lock() {
            Ok(g) => g,
            Err(_) => return h.fail_internal("mediaservo_client_consumer_stats: lock poisoned"),
        };
        let Some(consumer) = guard.as_ref() else {
            return h.fail_state("mediaservo_client_consumer_stats: consumer closed");
        };
        let json = match serde_json::to_string(&consumer.stats()) {
            Ok(j) => j,
            Err(e) => {
                return h
                    .fail_internal(&format!("mediaservo_client_consumer_stats: serialize: {e}"));
            }
        };
        let rc = copy_out_needed(NAME, &json, out_json, cap, needed);
        if rc != MEDIASERVO_OK {
            h.fail_arg("mediaservo_client_consumer_stats: buffer too small (see needed)");
        }
        rc
    })
}

/// QOE periodic callback (consumer-qoe-callback): Rust on_qoe → C trampoline.
/// cb fires on the SDK tick task (a tokio worker); the C side must guarantee its own thread safety.
/// Re-registration: the old task is abort+join before returning — timing-safe for the caller swapping ctx (PIT-221).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_consumer_on_qoe(
    c: *mut mediaservo_client_consumer_t,
    period_ms: u32,
    cb: mediaservo_client_qoe_cb,
    user: *mut c_void,
) -> c_int {
    const NAME: &str = "mediaservo_client_consumer_on_qoe";
    ffi_catch(NAME, c, || {
        let Some(h) = (unsafe { c.as_ref() }) else {
            return fail_global(
                "mediaservo_client_consumer_on_qoe: null handle",
                MEDIASERVO_CLIENT_ERR_INVALID_ARG,
            );
        };
        if period_ms == 0 {
            return h.fail_arg("mediaservo_client_consumer_on_qoe: period_ms 0");
        }
        let guard = match h.consumer.lock() {
            Ok(g) => g,
            Err(_) => return h.fail_internal("mediaservo_client_consumer_on_qoe: lock poisoned"),
        };
        let Some(consumer) = guard.as_ref() else {
            return h.fail_state("mediaservo_client_consumer_on_qoe: consumer closed");
        };
        // C trampoline: QoeSnapshot → C struct (field order frozen; see the client.h ABI note).
        // CbUser carries a raw pointer across the Send boundary (C contract: the caller guarantees the user's thread safety).
        let ctx = std::sync::Arc::new(CbUser(user));
        let cb_cb = cb;
        let trampoline = move |snap: mediaservo_client::consumer::QoeSnapshot| {
            let c_snap = mediaservo_client_qoe_snapshot_t {
                struct_size: std::mem::size_of::<mediaservo_client_qoe_snapshot_t>() as u32,
                fps: snap.fps,
                kbps: snap.kbps,
                lost_percent: snap.lost_percent,
                decoded_total: snap.decoded_total,
                avg_decode_ms: snap.avg_decode_ms,
                jitter_ms: snap.jitter_ms,
                rtt_ms: snap.rtt_ms,
                width: snap.width,
                height: snap.height,
                epoch: snap.epoch,
            };
            cb_cb(&c_snap, ctx.0);
        };
        let qoe_task = consumer
            .on_qoe(std::time::Duration::from_millis(u64::from(period_ms)), Box::new(trampoline));
        // Idempotent replacement (same discipline as PIT-221): old task abort+join (the join happens outside the lock —
        // narrowing the deadlock surface like the close path) — after return the old callback physically cannot fire again, so the C++ layer
        // can safely delete the old qoe_ctx_. The new task is created before insertion: within the join window the new task may
        // already have fired once (with the new ctx — allocated before registration, safe).
        let old_task = h.qoe_task.lock().ok().and_then(|mut g| g.take());
        if let Some(t) = old_task {
            t.abort();
            join_qoe_task(t);
        }
        if let Ok(mut g) = h.qoe_task.lock() {
            *g = Some(qoe_task);
        }
        MEDIASERVO_OK
    })
}

/// After abort, wait for the task to really exit. The C API sync entry points are never runtime threads —
/// block_on is safe; if misused from a runtime thread (future misuse), the try_current guard
/// degrades to abort-only (no join), the contract degrades to best-effort, and it logs.
fn join_qoe_task(task: tokio::task::JoinHandle<()>) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tracing::warn!("join_qoe_task called on runtime thread — skip join (abort only)");
        return;
    }
    let _ = crate::runtime().block_on(task);
}

/// Close this consumer and release the handle (idempotent). Set closed → take the Consumer away (receiver drop → K1 replay
/// declares it dead) → join the pump (≤CONSUMER_POLL) → user_free exactly once → free.
/// QOE task join discipline (PIT-221): after return the old callback cannot fire again (hard abort+join guarantee);
/// re-entering close / on_qoe from within a callback is forbidden (the join has been moved out of the lock critical section, but re-entry is still a contract violation,
/// deadlock risk is the caller's — the contract remains "no blocking inside callbacks").
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_consumer_close(c: *mut mediaservo_client_consumer_t) -> c_int {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if c.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(c) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent (same double-close discipline as link-c)
        }
        // Take-and-drop: the frame channel's receiver dies, the pump's next recv → None → break.
        if let Ok(mut g) = handle.consumer.lock() {
            drop(g.take());
        }
        // QOE tick task abort + join — after return the old callback physically cannot fire again.
        // (PIT-221: abort does not interrupt a callback executing synchronously — crash_d036 proved the client.hpp:436
        // (*fn)(q) call hit an already-deleted ctx. block_on uses the process-level runtime() — the C API sync entry points
        // are never runtime threads; see join_qoe_task for the try_current guard. lsan.1096280:
        // a 64B-per-registration leak — release semantics unchanged. The join runs outside the lock (the guard drops right after the take) —
        // a contract-violating callback re-entering close/on_qoe just gets the closed-state error and never deadlocks against the join.)
        let task = handle.qoe_task.lock().ok().and_then(|mut g| g.take());
        if let Some(t) = task {
            t.abort();
            join_qoe_task(t);
        }
        if let Some(join) = handle.pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join();
        }
        if let Some(f) = handle.free {
            f(handle.user);
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        crate::errors::set_last_error("mediaservo_client_consumer_close: panic");
        crate::errors::MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::MEDIASERVO_CLIENT_ERR_STATE;
    use std::ptr;

    #[test]
    fn consumer_guards_null() {
        let mut buf = [0u8; 64];
        let mut need = 0usize;
        assert_eq!(
            mediaservo_client_consumer_id(
                ptr::null(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                &mut need
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(
            mediaservo_client_consumer_stats(
                ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                &mut need
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(mediaservo_client_consumer_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn consumer_close_joins_pump_and_frees_once() {
        // user_free exactly-once pin (contract): bounded join within the pump idle window (CONSUMER_POLL).
        // Double close is not tested — after close, touching handle memory is UB per contract (the swap idempotent branch is defense in depth,
        // not a use-after-free exemption; same discipline as link-c).
        static FREES: AtomicBool = AtomicBool::new(false);
        extern "C" fn mark_free(_u: *mut c_void) {
            FREES.store(true, Ordering::SeqCst);
        }
        extern "C" fn noop_frame(
            _s: *mut mediaservo_client_session_t,
            _f: *const mediaservo_client_frame_t,
            _u: *mut c_void,
        ) {
        }
        // Consumer has no public constructor (needs a real pc) — spawn injection is not feasible, so assemble manually
        // (same-crate private field access): the consumer=None form = "channel withdrawn", the pump converges immediately.
        let handle = Box::new(mediaservo_client_consumer_t {
            session: ptr::null_mut(),
            producer_id: "p-test".to_string(),
            consumer: Mutex::new(None),
            closed: AtomicBool::new(false),
            cb: noop_frame,
            user: ptr::null_mut(),
            free: Some(mark_free),
            pump: Mutex::new(None),
            qoe_task: Mutex::new(None),
            err: Mutex::new(None),
        });
        let raw = Box::into_raw(handle);
        let raw_usize = raw as usize;
        let join = std::thread::spawn(move || pump_loop(raw_usize));
        unsafe { &*raw }.pump.lock().unwrap_or_else(|e| e.into_inner()).replace(join);

        assert_eq!(mediaservo_client_consumer_close(raw), MEDIASERVO_OK);
        assert!(FREES.load(Ordering::SeqCst), "user_free was not called");
    }

    #[test]
    fn consumer_close_joins_qoe_task_no_callback_after_return() {
        use std::sync::atomic::AtomicUsize;
        static FIRES: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn noop_frame(
            _s: *mut mediaservo_client_session_t,
            _f: *const mediaservo_client_frame_t,
            _u: *mut c_void,
        ) {
        }
        // Tick task: tick → 30ms sync segment (simulating "a callback executing synchronously" — the segment abort cannot interrupt),
        // widening the race window from microseconds to ~75% of the period → really increments FIRES.
        let task = crate::runtime().spawn(async {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(10));
            loop {
                tick.tick().await;
                std::thread::sleep(std::time::Duration::from_millis(30));
                FIRES.fetch_add(1, Ordering::SeqCst);
            }
        });
        let handle = Box::new(mediaservo_client_consumer_t {
            session: ptr::null_mut(),
            producer_id: "p-qoe".to_string(),
            consumer: Mutex::new(None),
            closed: AtomicBool::new(false),
            cb: noop_frame,
            user: ptr::null_mut(),
            free: None,
            pump: Mutex::new(None),
            qoe_task: Mutex::new(Some(task)),
            err: Mutex::new(None),
        });
        let raw = Box::into_raw(handle);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(mediaservo_client_consumer_close(raw), MEDIASERVO_OK);
        let after = FIRES.load(Ordering::SeqCst);
        // join discipline (PIT-221): close returning = the task has ended — the in-flight callback's increment is already counted
        // in `after`, and it can physically never grow again. Strict equality (no tolerance — tolerance would mask regressions).
        std::thread::sleep(std::time::Duration::from_millis(100));
        let settled = FIRES.load(Ordering::SeqCst);
        assert_eq!(
            settled, after,
            "callback still fired after close returned: after={after} settled={settled}"
        );
    }

    #[test]
    fn on_qoe_replace_joins_old_task() {
        // The replacement path reuses join_qoe_task (behavior pinned by the close test) — here we pin the helper's
        // abort+join immediate convergence for a long-sleep task, guarding against a future async-join change regressing replacement.
        let old = crate::runtime().spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        old.abort();
        join_qoe_task(old);
        // join returning proves the task ended (abort worked — a 1h sleep cannot end naturally)
    }
}
