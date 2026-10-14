//! MediaServo deck C ABI — capture/record/playback plane (C consumption for local monitoring/NVR scenarios).
//!
//! Contract §7 (D109/D240/D241): opaque handles + int error codes + frame callbacks.
//! Synchronous blocking style (internally a per-handle shared multi_thread runtime + pump threads).
//!
//! # Lifecycle contract (continuing audit R2)
//! - Each handle is single-thread-owner; any API call after close is UB (close is idempotent).
//! - close = set the closed flag → join the pump thread → release the handle.
//! - The mediaservo_frame_t data_* pointers are valid only inside the callback — copy if you need to keep them.
//! - Frame callbacks fire only on the pump thread; calling any mediaservo_deck_* API (including close) from within a callback is forbidden.
//! - `mediaservo_deck_recorder_record(rec, cam)`: the camera must already be `start`ed and **outlive the
//!   recording** (close order: recorder_stop/close before camera_stop/close).
//!
//! # C ABI surface
//! ```c
//! typedef struct mediaservo_deck_camera_t mediaservo_deck_camera_t;     /* opaque */
//! typedef struct mediaservo_deck_recorder_t mediaservo_deck_recorder_t; /* opaque */
//! typedef struct mediaservo_deck_player_t mediaservo_deck_player_t;     /* opaque */
//! typedef void (*mediaservo_deck_frame_cb)(const mediaservo_frame_t* frame, void* user);
//! ```
//! Error codes: MEDIASERVO_DECK_ERR_INVALID_ARG(-1)/DEVICE(-2)/RECORDER(-3)/PLAYER(-4)/STATE(-5)/INTERNAL(-6).

#![allow(clippy::not_unsafe_ptr_arg_deref)] // C ABI facade: non-null/alignment = documented contract (same crate-top precedent as link-c)
#![allow(non_camel_case_types)] // C-visible type names = snake_case mirrors (the *_*_t word shape is part of the ABI)
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mediaservo_codec::frame::VideoFrame;
use mediaservo_deck::record::{Frames, Recorder, StopSignal};
use mediaservo_deck::{
    CameraSource, CaptureOptions, DeckError, DeviceId, FrameStream, MediaDeviceKind, MediaDevices,
    Player, RecordOptions,
};
use tokio::sync::mpsc;

/// Error codes (0 = ok, <0 = error).
pub const MEDIASERVO_OK: c_int = 0;
pub const MEDIASERVO_DECK_ERR_INVALID_ARG: c_int = -1;
pub const MEDIASERVO_DECK_ERR_DEVICE: c_int = -2;
pub const MEDIASERVO_DECK_ERR_RECORDER: c_int = -3;
pub const MEDIASERVO_DECK_ERR_PLAYER: c_int = -4;
pub const MEDIASERVO_DECK_ERR_STATE: c_int = -5;
pub const MEDIASERVO_DECK_ERR_INTERNAL: c_int = -6;

/// Global last-error message (read via mediaservo_deck_last_error).
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

// -- C structs --

/// Capture options (C struct — maps to CaptureOptions).
///
/// First field `struct_size` (audit R3): the caller fills `sizeof(mediaservo_deck_capture_options_t)`;
/// the library validates `>= sizeof(known struct)` and ignores extra length. width/height/framerate all 0 = default
/// 1280x720@30。
#[allow(non_camel_case_types)] // C ABI naming (C6 exception: mediaservo_* prefix)
#[repr(C)]
pub struct mediaservo_deck_capture_options_t {
    pub struct_size: usize,
    pub width: u32,
    pub height: u32,
    pub framerate: u32,
}

/// Known prefix size of the C struct (the minimum legal value across version evolution).
pub const MEDIASERVO_DECK_CAPTURE_OPTIONS_MIN_SIZE: usize =
    size_of::<mediaservo_deck_capture_options_t>();

/// In-memory frame (I420 three planes; layout matches mediaservo_frame_t in mediaservo_common.h).
/// data_* pointers are valid only inside the callback.
#[allow(non_camel_case_types)]
#[repr(C)]
pub struct mediaservo_frame_t {
    pub width: u32,
    pub height: u32,
    pub pts_us: u64,
    pub stride_y: u32,
    pub stride_u: u32,
    pub stride_v: u32,
    pub data_y: *const u8,
    pub data_u: *const u8,
    pub data_v: *const u8,
}

/// Frame callback (fires on the pump thread; the `frame` pointer is valid only inside the callback).
pub type mediaservo_deck_frame_cb = unsafe extern "C" fn(*const mediaservo_frame_t, *mut c_void);

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
        .expect("deck-c runtime")
}

/// DeckError → C error code (mapped to RECORDER/PLAYER by call domain).
#[derive(Clone, Copy)]
enum Ctx {
    Camera,
    Recorder,
    Player,
}

fn map_deck_err(e: &DeckError, ctx: Ctx) -> c_int {
    match e {
        DeckError::Device(_) => MEDIASERVO_DECK_ERR_DEVICE,
        DeckError::InvalidState(_) => MEDIASERVO_DECK_ERR_STATE,
        DeckError::NotFound(_) => match ctx {
            Ctx::Camera => MEDIASERVO_DECK_ERR_DEVICE,
            Ctx::Recorder => MEDIASERVO_DECK_ERR_RECORDER,
            Ctx::Player => MEDIASERVO_DECK_ERR_PLAYER,
        },
        DeckError::Codec(_) | DeckError::Io(_) => match ctx {
            Ctx::Camera => MEDIASERVO_DECK_ERR_INTERNAL,
            Ctx::Recorder => MEDIASERVO_DECK_ERR_RECORDER,
            Ctx::Player => MEDIASERVO_DECK_ERR_PLAYER,
        },
        // #[non_exhaustive]: future variants map uniformly to INTERNAL
        _ => MEDIASERVO_DECK_ERR_INTERNAL,
    }
}

/// Capture the lock (poison → INTERNAL).
fn lock<'a, T>(
    guard: Result<
        std::sync::MutexGuard<'a, T>,
        std::sync::PoisonError<std::sync::MutexGuard<'a, T>>,
    >,
) -> Result<std::sync::MutexGuard<'a, T>, c_int> {
    guard.map_err(|_| {
        set_last_error("deck-c: mutex poisoned");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// VideoFrame → mediaservo_frame_t (data pointers point into the frame planes, valid only inside the callback).
fn to_mediaservo_frame(f: &VideoFrame) -> mediaservo_frame_t {
    mediaservo_frame_t {
        width: f.format.width,
        height: f.format.height,
        pts_us: f.pts,
        stride_y: f.plane_stride(0).unwrap_or(0),
        stride_u: f.plane_stride(1).unwrap_or(0),
        stride_v: f.plane_stride(2).unwrap_or(0),
        data_y: f.plane_data(0).map(|d| d.as_ptr()).unwrap_or(ptr::null()),
        data_u: f.plane_data(1).map(|d| d.as_ptr()).unwrap_or(ptr::null()),
        data_v: f.plane_data(2).map(|d| d.as_ptr()).unwrap_or(ptr::null()),
    }
}

// -- camera (capture) --

/// camera shared state (pump thread + C calls access it via Arc).
struct CameraInner {
    src: Mutex<Option<CameraSource>>,
    opts: CaptureOptions,
    /// Frame callback (registered via frames_cb; fetched by the pump thread each cycle).
    cb: Mutex<Option<(mediaservo_deck_frame_cb, *mut c_void)>>,
    /// Recording-bridge sender (registered via recorder_record; the pump thread sends per frame).
    rec_tx: Mutex<Option<mpsc::UnboundedSender<VideoFrame>>>,
    /// Pump-thread stop flag (set by camera_stop).
    stop: AtomicBool,
    started: AtomicBool,
    closed: AtomicBool,
    rt: tokio::runtime::Runtime,
}

// SAFETY: cb's user pointer is used only on the pump thread (contract: no API calls inside callbacks,
// close joins the pump thread before releasing the handle); the remaining fields are thread-safe types.
unsafe impl Send for CameraInner {}
// SAFETY: same as above; all mutable access is serialized through Mutex/Atomic.
unsafe impl Sync for CameraInner {}

/// Camera opaque handle.
pub struct mediaservo_deck_camera_t {
    inner: Arc<CameraInner>,
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// Pump-thread body: polls stop/closed every 50ms (generator.stop does not release the sink →
/// recv never returns None, so a timeout poll is required); frames fan out to cb + the recording bridge.
fn camera_pump_loop(shared: Arc<CameraInner>, mut stream: FrameStream) {
    loop {
        if shared.stop.load(Ordering::SeqCst) || shared.closed.load(Ordering::SeqCst) {
            break;
        }
        match shared.rt.block_on(async {
            tokio::time::timeout(Duration::from_millis(50), stream.recv()).await
        }) {
            Ok(Some(frame)) => deliver_frame(&shared, frame),
            Ok(None) => break, // frame stream ended
            Err(_) => {}       // timeout: back to the loop to check stop/closed
        }
    }
}

/// Single-frame fan-out: cb first (borrow), then the recording bridge (move ownership).
fn deliver_frame(shared: &CameraInner, frame: VideoFrame) {
    if let Ok(guard) = shared.cb.lock()
        && let Some((cb, user)) = *guard
    {
        let mf = to_mediaservo_frame(&frame);
        unsafe { cb(&mf, user) };
    }
    if let Ok(guard) = shared.rec_tx.lock()
        && let Some(tx) = guard.clone()
        && tx.send(frame).is_err()
    {
        // The recording task has ended (rx dropped) → self-heal clear
        if let Ok(mut g) = shared.rec_tx.lock() {
            *g = None;
        }
    }
}

/// Enumerate devices (two-call pattern): the first call with `out_ids=NULL` returns the required length (excluding NUL,
/// snprintf convention; negative on error), the second fills the buffer (also returns the required length on truncation).
/// kind: 0=Camera 1=Audio 2=Screen; multiple devices separated by '\n'.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_devices_enumerate(
    kind: c_int,
    out_ids: *mut c_char,
    cap: usize,
    out_len: *mut usize,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let kind = match kind {
            0 => MediaDeviceKind::Camera,
            1 => MediaDeviceKind::Audio,
            2 => MediaDeviceKind::Screen,
            _ => {
                set_last_error(
                    "mediaservo_deck_devices_enumerate: invalid kind (0=Camera 1=Audio 2=Screen)",
                );
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
        };
        let ids =
            MediaDevices::enumerate(kind).into_iter().map(|d| d.0).collect::<Vec<_>>().join("\n");
        if !out_len.is_null() {
            unsafe { *out_len = ids.len() };
        }
        if !out_ids.is_null() {
            if cap == 0 {
                set_last_error("mediaservo_deck_devices_enumerate: cap == 0 with out_ids");
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
            let n = ids.len().min(cap - 1);
            unsafe {
                ptr::copy_nonoverlapping(ids.as_ptr(), out_ids as *mut u8, n);
                *out_ids.add(n) = 0;
            }
        }
        ids.len() as c_int
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_devices_enumerate: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Open the camera (blocking, local initialization only). `opts` must not be null; `dev_id` must exist in the enumeration result.
/// On success `*out` points to the new handle (the caller owns `mediaservo_deck_camera_close`).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_camera_open(
    dev_id: *const c_char,
    opts: *const mediaservo_deck_capture_options_t,
    out: *mut *mut mediaservo_deck_camera_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if dev_id.is_null() || opts.is_null() || out.is_null() {
            set_last_error("mediaservo_deck_camera_open: null dev_id/opts/out");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let opts_ref = unsafe { &*opts };
        if opts_ref.struct_size < MEDIASERVO_DECK_CAPTURE_OPTIONS_MIN_SIZE {
            set_last_error(format!(
                "mediaservo_deck_camera_open: opts.struct_size {} < {} (rebuild with current header)",
                opts_ref.struct_size,
                MEDIASERVO_DECK_CAPTURE_OPTIONS_MIN_SIZE
            ));
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let dev = match cstr(dev_id) {
            Ok(Some(s)) => s.to_owned(),
            Ok(None) => {
                set_last_error("mediaservo_deck_camera_open: dev_id required");
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
            Err(_) => {
                set_last_error("mediaservo_deck_camera_open: invalid UTF-8 in dev_id");
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
        };
        // width/height/framerate all 0 = default 1280x720@30
        let (w, h) = if opts_ref.width > 0 && opts_ref.height > 0 {
            (opts_ref.width, opts_ref.height)
        } else {
            (1280, 720)
        };
        let fps = if opts_ref.framerate > 0 { opts_ref.framerate } else { 30 };
        let copts = CaptureOptions {
            resolution: Some((w, h)),
            framerate: Some(fps),
            format: None, // CameraSource::start does not use format
        };
        match CameraSource::open(DeviceId(dev), copts.clone()) {
            Ok(src) => {
                let handle = Box::new(mediaservo_deck_camera_t {
                    inner: Arc::new(CameraInner {
                        src: Mutex::new(Some(src)),
                        opts: copts,
                        cb: Mutex::new(None),
                        rec_tx: Mutex::new(None),
                        stop: AtomicBool::new(false),
                        started: AtomicBool::new(false),
                        closed: AtomicBool::new(false),
                        rt: new_runtime(),
                    }),
                    pump: Mutex::new(None),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_deck_camera_open: {e}"));
                map_deck_err(&e, Ctx::Camera)
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_camera_open: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Start producing frames (uses the opts from open; only allowed once, repeat calls → STATE) and start the pump thread.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_camera_start(c: *mut mediaservo_deck_camera_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            set_last_error("mediaservo_deck_camera_start: null handle");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*c };
        if handle.inner.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_camera_start: camera closed");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        if handle.inner.started.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_camera_start: already started");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        let opts = handle.inner.opts.clone();
        let mut src_guard = match lock(handle.inner.src.lock()) {
            Ok(g) => g,
            Err(rc) => return rc,
        };
        let Some(src) = src_guard.as_mut() else {
            set_last_error("mediaservo_deck_camera_start: camera not open");
            return MEDIASERVO_DECK_ERR_STATE;
        };
        let stream = match src.start(&opts) {
            Ok(s) => s,
            Err(e) => {
                set_last_error(format!("mediaservo_deck_camera_start: {e}"));
                return map_deck_err(&e, Ctx::Camera);
            }
        };
        let mut pump_guard = match lock(handle.pump.lock()) {
            Ok(g) => g,
            Err(rc) => return rc,
        };
        let shared = Arc::clone(&handle.inner);
        let pump = match std::thread::Builder::new()
            .name("deck-camera-pump".into())
            .spawn(move || camera_pump_loop(shared, stream))
        {
            Ok(p) => p,
            Err(e) => {
                set_last_error(format!("mediaservo_deck_camera_start: spawn pump: {e}"));
                return MEDIASERVO_DECK_ERR_INTERNAL;
            }
        };
        *pump_guard = Some(pump);
        handle.inner.started.store(true, Ordering::SeqCst);
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_camera_start: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Register the frame callback (fires per frame on the pump thread; a repeat call replaces the old callback).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_camera_frames_cb(
    c: *mut mediaservo_deck_camera_t,
    cb: Option<mediaservo_deck_frame_cb>,
    user: *mut c_void,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            set_last_error("mediaservo_deck_camera_frames_cb: null handle");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let Some(cb) = cb else {
            set_last_error("mediaservo_deck_camera_frames_cb: null cb");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        };
        let handle = unsafe { &*c };
        if handle.inner.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_camera_frames_cb: camera closed");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        let mut guard = match lock(handle.inner.cb.lock()) {
            Ok(g) => g,
            Err(rc) => return rc,
        };
        *guard = Some((cb, user));
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_camera_frames_cb: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Stop producing frames (idempotent): set the stop flag → the pump thread exits within 50ms → stop the frame source.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_camera_stop(c: *mut mediaservo_deck_camera_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            set_last_error("mediaservo_deck_camera_stop: null handle");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*c };
        handle.inner.stop.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = handle.inner.src.lock()
            && let Some(src) = guard.as_mut()
        {
            src.stop();
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_camera_stop: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Close the camera and release the handle (idempotent): set closed → join the pump thread → release
/// (the Drop chain stops the frame source). Calling during a frame callback is UB (documented contract).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_camera_close(c: *mut mediaservo_deck_camera_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            return MEDIASERVO_OK;
        }
        if unsafe { &*c }.inner.closed.load(Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent: already closed
        }
        let handle = unsafe { Box::from_raw(c) };
        handle.inner.closed.store(true, Ordering::SeqCst);
        handle.inner.stop.store(true, Ordering::SeqCst);
        if let Some(pump) = handle.pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = pump.join();
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_camera_close: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

// -- recorder (recording) --

/// Frame bridge: async frame source on the recording-task side (camera pump → unbounded channel).
struct FrameRx {
    rx: mpsc::UnboundedReceiver<VideoFrame>,
}

impl Frames for FrameRx {
    #[allow(clippy::manual_async_fn)] // the pump-thread entry returns an explicit Future: making it async moves thread boundaries, benefit < risk
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send {
        async move { self.rx.recv().await }
    }
}

/// recorder shared state.
struct RecorderInner {
    recorder: Mutex<Option<Recorder>>,
    /// Recording task (started by record; close does block_on join to complete flush+trailer).
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// External stop signal (set by stop → the recording loop exits within 50ms and flushes).
    stop_signal: Mutex<Option<StopSignal>>,
    closed: AtomicBool,
    rt: tokio::runtime::Runtime,
}

/// Recorder opaque handle.
pub struct mediaservo_deck_recorder_t {
    inner: Arc<RecorderInner>,
}

/// Create the recorder (RecordOptions defaults to h264/mp4; not started).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_recorder_new(
    path: *const c_char,
    out: *mut *mut mediaservo_deck_recorder_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if path.is_null() || out.is_null() {
            set_last_error("mediaservo_deck_recorder_new: null path/out");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let path = match cstr(path) {
            Ok(Some(s)) => s.to_owned(),
            Ok(None) => {
                set_last_error("mediaservo_deck_recorder_new: path required");
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
            Err(_) => {
                set_last_error("mediaservo_deck_recorder_new: invalid UTF-8 in path");
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
        };
        match Recorder::new(PathBuf::from(path), RecordOptions::default()) {
            Ok(rec) => {
                let handle = Box::new(mediaservo_deck_recorder_t {
                    inner: Arc::new(RecorderInner {
                        recorder: Mutex::new(Some(rec)),
                        task: Mutex::new(None),
                        stop_signal: Mutex::new(None),
                        closed: AtomicBool::new(false),
                        rt: new_runtime(),
                    }),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_deck_recorder_new: {e}"));
                map_deck_err(&e, Ctx::Recorder)
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_recorder_new: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Bridge recording: the camera's started frame pump → the recorder's recording task (internally finalizes with a flush).
/// Contract: the camera must outlive the recording (close order: recorder before camera).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_recorder_record(
    r: *mut mediaservo_deck_recorder_t,
    c: *mut mediaservo_deck_camera_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if r.is_null() || c.is_null() {
            set_last_error("mediaservo_deck_recorder_record: null recorder/camera");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let rec_handle = unsafe { &*r };
        if rec_handle.inner.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_recorder_record: recorder closed");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        let cam = unsafe { &*c };
        if cam.inner.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_recorder_record: camera closed");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        if !cam.inner.started.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_recorder_record: camera not started");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        if cam.inner.stop.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_recorder_record: camera stopped");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        let mut rec_guard = match lock(rec_handle.inner.recorder.lock()) {
            Ok(g) => g,
            Err(rc) => return rc,
        };
        let Some(mut recorder) = rec_guard.take() else {
            set_last_error(
                "mediaservo_deck_recorder_record: recorder not open or already recording",
            );
            return MEDIASERVO_DECK_ERR_STATE;
        };
        let mut tx_guard = match lock(cam.inner.rec_tx.lock()) {
            Ok(g) => g,
            Err(rc) => {
                *rec_guard = Some(recorder);
                return rc;
            }
        };
        let mut task_guard = match lock(rec_handle.inner.task.lock()) {
            Ok(g) => g,
            Err(rc) => {
                *rec_guard = Some(recorder);
                return rc;
            }
        };
        let mut stop_guard = match lock(rec_handle.inner.stop_signal.lock()) {
            Ok(g) => g,
            Err(rc) => {
                *rec_guard = Some(recorder);
                return rc;
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        *tx_guard = Some(tx);
        let stop_signal = recorder.stop_signal();
        // record() blocks until stop/stream end → must be spawned onto the shared runtime (continuing R1)
        let task = rec_handle.inner.rt.spawn(async move {
            if let Err(e) = recorder.record(FrameRx { rx }).await {
                tracing::error!("deck-c recorder task: {e}");
            }
        });
        *task_guard = Some(task);
        *stop_guard = Some(stop_signal);
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_recorder_record: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Request stop recording (idempotent): set running=false → the recording loop exits within 50ms and flushes.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_recorder_stop(r: *mut mediaservo_deck_recorder_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if r.is_null() {
            set_last_error("mediaservo_deck_recorder_stop: null handle");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let handle = unsafe { &*r };
        if handle.inner.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_recorder_stop: recorder closed");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        if let Some(signal) = handle.inner.stop_signal.lock().ok().and_then(|mut g| g.take()) {
            signal.stop();
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_recorder_stop: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Close the recorder and release the handle (idempotent): join the recording task (flush + trailer complete).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_recorder_close(r: *mut mediaservo_deck_recorder_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if r.is_null() {
            return MEDIASERVO_OK;
        }
        if unsafe { &*r }.inner.closed.load(Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent: already closed
        }
        let handle = unsafe { Box::from_raw(r) };
        handle.inner.closed.store(true, Ordering::SeqCst);
        if let Some(task) = handle.inner.task.lock().ok().and_then(|mut g| g.take()) {
            match handle.inner.rt.block_on(task) {
                Ok(()) => MEDIASERVO_OK,
                Err(e) => {
                    set_last_error(format!("mediaservo_deck_recorder_close: recorder task: {e}"));
                    MEDIASERVO_DECK_ERR_INTERNAL
                }
            }
        } else {
            MEDIASERVO_OK
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_recorder_close: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

// -- player (playback) --

/// player shared state.
struct PlayerInner {
    player: Mutex<Option<Player>>,
    /// Decode pump thread (started by frames_cb; close sets closed → the thread exits within one frame → join).
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
    closed: AtomicBool,
}

/// Player opaque handle.
pub struct mediaservo_deck_player_t {
    inner: Arc<PlayerInner>,
}

/// Open a media file (demux + decoder ready; unsupported files → PLAYER error).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_player_open(
    path: *const c_char,
    out: *mut *mut mediaservo_deck_player_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if path.is_null() || out.is_null() {
            set_last_error("mediaservo_deck_player_open: null path/out");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let path = match cstr(path) {
            Ok(Some(s)) => s.to_owned(),
            Ok(None) => {
                set_last_error("mediaservo_deck_player_open: path required");
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
            Err(_) => {
                set_last_error("mediaservo_deck_player_open: invalid UTF-8 in path");
                return MEDIASERVO_DECK_ERR_INVALID_ARG;
            }
        };
        match Player::open(PathBuf::from(path)) {
            Ok(player) => {
                let handle = Box::new(mediaservo_deck_player_t {
                    inner: Arc::new(PlayerInner {
                        player: Mutex::new(Some(player)),
                        pump: Mutex::new(None),
                        closed: AtomicBool::new(false),
                    }),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("mediaservo_deck_player_open: {e}"));
                map_deck_err(&e, Ctx::Player)
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_player_open: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Per-frame decode callback pump (synchronous next_frame loop; exits on EOF or close).
/// Only allowed once (repeat calls → STATE).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_player_frames_cb(
    p: *mut mediaservo_deck_player_t,
    cb: Option<mediaservo_deck_frame_cb>,
    user: *mut c_void,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if p.is_null() {
            set_last_error("mediaservo_deck_player_frames_cb: null handle");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        }
        let Some(cb) = cb else {
            set_last_error("mediaservo_deck_player_frames_cb: null cb");
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
        };
        let handle = unsafe { &*p };
        if handle.inner.closed.load(Ordering::SeqCst) {
            set_last_error("mediaservo_deck_player_frames_cb: player closed");
            return MEDIASERVO_DECK_ERR_STATE;
        }
        let mut guard = match lock(handle.inner.player.lock()) {
            Ok(g) => g,
            Err(rc) => return rc,
        };
        let Some(mut player) = guard.take() else {
            set_last_error("mediaservo_deck_player_frames_cb: already pumping or closed");
            return MEDIASERVO_DECK_ERR_STATE;
        };
        let mut pump_guard = match lock(handle.inner.pump.lock()) {
            Ok(g) => g,
            Err(rc) => {
                *guard = Some(player); // put back
                return rc;
            }
        };
        let _shared = Arc::clone(&handle.inner);
        // *mut c_void is not Send → round-trip as usize inside the thread closure (FFI convention)
        let user_tag = user as usize;
        let pump =
            match std::thread::Builder::new().name("deck-player-pump".into()).spawn(move || {
                let user = user_tag as *mut c_void;
                // The pump runs until EOF (or a decode error) ends it naturally — close does not abort it:
                // close semantics = join (blocks until decoding finishes); otherwise calling close right after frames_cb returns
                // would race-kill a pump that has not decoded any frames yet (zero-frame output).
                loop {
                    match player.next_frame() {
                        Ok(Some(frame)) => {
                            let mf = to_mediaservo_frame(&frame);
                            unsafe { cb(&mf, user) };
                        }
                        Ok(None) => break, // EOF
                        Err(e) => {
                            set_last_error(format!("mediaservo_deck_player_frames_cb: {e}"));
                            break;
                        }
                    }
                }
            }) {
                Ok(p) => p,
                Err(e) => {
                    // player has been moved into the closure (dropped with it if spawn fails — an unrecoverable resource-exhaustion-class error)
                    set_last_error(format!("mediaservo_deck_player_frames_cb: spawn pump: {e}"));
                    return MEDIASERVO_DECK_ERR_INTERNAL;
                }
            };
        *pump_guard = Some(pump);
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_player_frames_cb: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

/// Close the player and release the handle (idempotent): set closed → join the decode pump (blocks until EOF/error) → release.
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_player_close(p: *mut mediaservo_deck_player_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if p.is_null() {
            return MEDIASERVO_OK;
        }
        if unsafe { &*p }.inner.closed.load(Ordering::SeqCst) {
            return MEDIASERVO_OK; // idempotent: already closed
        }
        let handle = unsafe { Box::from_raw(p) };
        handle.inner.closed.store(true, Ordering::SeqCst);
        if let Some(pump) = handle.inner.pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = pump.join();
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_deck_player_close: panic");
        MEDIASERVO_DECK_ERR_INTERNAL
    })
}

// -- Common --

/// Details of the last error (thread-safe; returns an empty string when no error).
fn last_error_impl(buf: *mut c_char, len: usize) -> c_int {
    if buf.is_null() || len == 0 {
        return MEDIASERVO_DECK_ERR_INVALID_ARG;
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
pub extern "C" fn mediaservo_deck_last_error(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| last_error_impl(buf, len)))
        .unwrap_or(MEDIASERVO_DECK_ERR_INTERNAL)
}

/// Version info (MAJOR.MINOR.PATCH — D241 soname semantics).
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_version(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if buf.is_null() || len == 0 {
            return MEDIASERVO_DECK_ERR_INVALID_ARG;
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
    .unwrap_or(MEDIASERVO_DECK_ERR_INTERNAL)
}

#[cfg(test)]
mod tests {
    /// Tests in this file share the process-level static LAST_ERROR (C ABI semantics) — null-handle negative
    /// cases write it implicitly, so explicit-read assertions stomp on each other = flaky (same surgery as field-c 09-20; link-c 09-20 gate
    /// caught last_error_roundtrip racing; deck-c prevented in the same batch). All tests in this file serialize through this lock.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::*;

    /// Helper: open a default camera (the caller owns close on success).
    fn open_camera() -> *mut mediaservo_deck_camera_t {
        let dev = c"stub:test-camera";
        let opts = mediaservo_deck_capture_options_t {
            struct_size: MEDIASERVO_DECK_CAPTURE_OPTIONS_MIN_SIZE,
            width: 0,
            height: 0,
            framerate: 0,
        };
        let mut out: *mut mediaservo_deck_camera_t = ptr::null_mut();
        let rc = mediaservo_deck_camera_open(dev.as_ptr(), &opts, &mut out);
        assert_eq!(rc, MEDIASERVO_OK, "open failed");
        out
    }

    #[test]
    fn last_error_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Global state races across tests: clear before setting (do not assume other tests haven't written)
        set_last_error("");
        set_last_error("deck test error");
        let mut buf = [0u8; 64];
        let rc = mediaservo_deck_last_error(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert_eq!(s, "deck test error");
    }

    #[test]
    fn version_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = [0u8; 32];
        let rc = mediaservo_deck_version(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(s.starts_with("0.1."), "version: {s}");
    }

    #[test]
    fn enumerate_two_call_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // First call: length (including separators, excluding NUL); second call: identical content
        let mut len1: usize = 0;
        let rc = mediaservo_deck_devices_enumerate(0, ptr::null_mut(), 0, &mut len1);
        assert!(rc > 0, "rc={rc}");
        assert_eq!(rc as usize, len1);
        let mut buf = [0u8; 64];
        let mut len2: usize = 0;
        let rc2 = mediaservo_deck_devices_enumerate(
            0,
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut len2,
        );
        assert_eq!(rc2, rc, "second call length mismatch");
        assert_eq!(len2, len1);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert_eq!(s, "stub:test-camera");
        assert_eq!(s.len(), len1);
    }

    #[test]
    fn enumerate_empty_kind() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Audio/Screen have no devices → length 0
        let mut len: usize = 0;
        let rc = mediaservo_deck_devices_enumerate(1, ptr::null_mut(), 0, &mut len);
        assert_eq!(rc, 0);
        assert_eq!(len, 0);
    }

    #[test]
    fn enumerate_invalid_kind() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_deck_devices_enumerate(7, ptr::null_mut(), 0, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn camera_open_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_deck_camera_open(ptr::null(), ptr::null(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn camera_open_small_struct_size_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dev = c"stub:test-camera";
        let opts =
            mediaservo_deck_capture_options_t { struct_size: 1, width: 0, height: 0, framerate: 0 };
        let mut out: *mut mediaservo_deck_camera_t = ptr::null_mut();
        let rc = mediaservo_deck_camera_open(dev.as_ptr(), &opts, &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn camera_open_unknown_device_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dev = c"stub:nonexistent";
        let opts = mediaservo_deck_capture_options_t {
            struct_size: MEDIASERVO_DECK_CAPTURE_OPTIONS_MIN_SIZE,
            width: 0,
            height: 0,
            framerate: 0,
        };
        let mut out: *mut mediaservo_deck_camera_t = ptr::null_mut();
        let rc = mediaservo_deck_camera_open(dev.as_ptr(), &opts, &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_DEVICE);
        assert!(out.is_null());
    }

    #[test]
    fn camera_open_close_roundtrip() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cam = open_camera();
        assert!(!cam.is_null());
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
        // C-consumer convention: set to NULL after close (closing the same pointer twice is UB — header contract)
        assert_eq!(mediaservo_deck_camera_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn camera_double_start_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cam = open_camera();
        assert_eq!(mediaservo_deck_camera_start(cam), MEDIASERVO_OK);
        let rc = mediaservo_deck_camera_start(cam);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_STATE);
        assert_eq!(mediaservo_deck_camera_stop(cam), MEDIASERVO_OK);
        assert_eq!(mediaservo_deck_camera_stop(cam), MEDIASERVO_OK); // idempotent
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
    }

    #[test]
    fn camera_frames_cb_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_deck_camera_frames_cb(ptr::null_mut(), None, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        let cam = open_camera();
        let rc = mediaservo_deck_camera_frames_cb(cam, None, ptr::null_mut()); // null cb
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
    }

    #[test]
    fn recorder_new_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: *mut mediaservo_deck_recorder_t = ptr::null_mut();
        let rc = mediaservo_deck_recorder_new(ptr::null(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn recorder_new_missing_parent_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Parent directory missing → NotFound → RECORDER
        let mut out: *mut mediaservo_deck_recorder_t = ptr::null_mut();
        let p = c"/tmp/opencode/no-such-dir-xyz/deck_test.mp4";
        let rc = mediaservo_deck_recorder_new(p.as_ptr(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_RECORDER);
        assert!(out.is_null());
    }

    #[test]
    fn recorder_record_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_deck_recorder_record(ptr::null_mut(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn recorder_record_camera_not_started_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cam = open_camera();
        let mut rec: *mut mediaservo_deck_recorder_t = ptr::null_mut();
        let p = c"/tmp/opencode/deck_test_never_written.mp4";
        let rc = mediaservo_deck_recorder_new(p.as_ptr(), &mut rec);
        assert_eq!(rc, MEDIASERVO_OK);
        let rc = mediaservo_deck_recorder_record(rec, cam); // camera not started
        assert_eq!(rc, MEDIASERVO_DECK_ERR_STATE);
        assert_eq!(mediaservo_deck_recorder_close(rec), MEDIASERVO_OK);
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
    }

    #[test]
    fn recorder_close_null_is_ok() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mediaservo_deck_recorder_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn player_open_missing_file_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: *mut mediaservo_deck_player_t = ptr::null_mut();
        let p = c"/tmp/opencode/no-such-file-xyz.mp4";
        let rc = mediaservo_deck_player_open(p.as_ptr(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_PLAYER);
        assert!(out.is_null());
    }

    #[test]
    fn player_open_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: *mut mediaservo_deck_player_t = ptr::null_mut();
        let rc = mediaservo_deck_player_open(ptr::null(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn player_close_null_is_ok() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mediaservo_deck_player_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn recorder_stop_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_deck_recorder_stop(ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn player_frames_cb_null_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let rc = mediaservo_deck_player_frames_cb(ptr::null_mut(), None, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }
}
