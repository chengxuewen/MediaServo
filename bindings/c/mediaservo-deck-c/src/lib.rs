//! MediaServo deck C ABI — 采集/录制/回放面（本地监控/NVR 场景 C 消费）。
//!
//! 契约 §7（D109/D240/D241）：opaque handle + int 错误码 + 帧回调。
//! 同步阻塞式（内部 per-handle 共享 multi_thread runtime + 泵线程）。
//!
//! # 生命周期契约（审核 R2 延续）
//! - handle 单线程属主；close 后任何 API 调用为 UB（close 幂等）。
//! - close = 置 closed 标志 → join 泵线程 → 释放 handle。
//! - `mediaservo_frame_t` 的 data_* 指针仅在回调内有效 —— 需要保留必须拷贝。
//! - 帧回调仅在泵线程触发；回调内禁止调用任何 mediaservo_deck_* API（含 close）。
//! - `mediaservo_deck_recorder_record(rec, cam)`：camera 必须已 `start` 且**活到录制
//!   结束**（关闭顺序：recorder_stop/close 先于 camera_stop/close）。
//!
//! # C ABI 面
//! ```c
//! typedef struct mediaservo_deck_camera_t mediaservo_deck_camera_t;     /* opaque */
//! typedef struct mediaservo_deck_recorder_t mediaservo_deck_recorder_t; /* opaque */
//! typedef struct mediaservo_deck_player_t mediaservo_deck_player_t;     /* opaque */
//! typedef void (*mediaservo_deck_frame_cb)(const mediaservo_frame_t* frame, void* user);
//! ```
//! 错误码：MEDIASERVO_DECK_ERR_INVALID_ARG(-1)/DEVICE(-2)/RECORDER(-3)/PLAYER(-4)/STATE(-5)/INTERNAL(-6)。

#![allow(clippy::not_unsafe_ptr_arg_deref)] // C ABI 门面：非空/对齐 = 文档契约（link-c 同形 crate-top 先例）
#![allow(non_camel_case_types)] // C 可见类型名 = snake_case 镜像（*_*_t 词形是 ABI 一部分）
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

/// 错误码（0 = ok, <0 = error）。
pub const MEDIASERVO_OK: c_int = 0;
pub const MEDIASERVO_DECK_ERR_INVALID_ARG: c_int = -1;
pub const MEDIASERVO_DECK_ERR_DEVICE: c_int = -2;
pub const MEDIASERVO_DECK_ERR_RECORDER: c_int = -3;
pub const MEDIASERVO_DECK_ERR_PLAYER: c_int = -4;
pub const MEDIASERVO_DECK_ERR_STATE: c_int = -5;
pub const MEDIASERVO_DECK_ERR_INTERNAL: c_int = -6;

/// 全局最近错误信息（mediaservo_deck_last_error 读取）。
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

// ── C 结构 ──

/// 采集选项（C 结构 — 与 CaptureOptions 映射）。
///
/// 首字段 `struct_size`（审核 R3）：调用方填 `sizeof(mediaservo_deck_capture_options_t)`，
/// 库校验 `>= sizeof(已知结构)`、超长忽略。width/height/framerate 全 0 = 默认
/// 1280x720@30。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外：mediaservo_* 前缀）
#[repr(C)]
pub struct mediaservo_deck_capture_options_t {
    pub struct_size: usize,
    pub width: u32,
    pub height: u32,
    pub framerate: u32,
}

/// C 结构已知前缀尺寸（版本演进时的最小合法值）。
pub const MEDIASERVO_DECK_CAPTURE_OPTIONS_MIN_SIZE: usize =
    size_of::<mediaservo_deck_capture_options_t>();

/// 内存帧（I420 三平面；布局与 mediaservo_common.h 的 mediaservo_frame_t 一致）。
/// data_* 指针仅在回调内有效。
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

/// 帧回调（泵线程触发；`frame` 指针仅回调内有效）。
pub type mediaservo_deck_frame_cb = unsafe extern "C" fn(*const mediaservo_frame_t, *mut c_void);

// ── 内部辅助 ──

/// 提取 C 字符串（null → None）。非法 UTF-8 → 错误。
fn cstr<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().map(Some).map_err(|_| ())
}

/// 新建共享 multi_thread runtime（每个 handle 一个）。
fn new_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("deck-c runtime")
}

/// DeckError → C 错误码（按调用域区分 RECORDER/PLAYER 归属）。
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
        // #[non_exhaustive]：未来变体统一映射 INTERNAL
        _ => MEDIASERVO_DECK_ERR_INTERNAL,
    }
}

/// 捕获锁（poison → INTERNAL）。
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

/// VideoFrame → mediaservo_frame_t（data 指针指向帧内 planes，仅回调内有效）。
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

// ── camera（采集）──

/// camera 共享状态（泵线程 + C 调用经 Arc 访问）。
struct CameraInner {
    src: Mutex<Option<CameraSource>>,
    opts: CaptureOptions,
    /// 帧回调（frames_cb 注册；泵线程每次取用）。
    cb: Mutex<Option<(mediaservo_deck_frame_cb, *mut c_void)>>,
    /// 录制桥接发送端（recorder_record 注册；泵线程逐帧 send）。
    rec_tx: Mutex<Option<mpsc::UnboundedSender<VideoFrame>>>,
    /// 泵线程停止标志（camera_stop 置位）。
    stop: AtomicBool,
    started: AtomicBool,
    closed: AtomicBool,
    rt: tokio::runtime::Runtime,
}

// SAFETY: cb 的 user 指针仅在泵线程使用（契约：回调内禁止调用 API、
// close 先 join 泵线程后释放 handle）；其余字段均为线程安全类型。
unsafe impl Send for CameraInner {}
// SAFETY: 同上，所有可变访问经 Mutex/Atomic 序列化。
unsafe impl Sync for CameraInner {}

/// 相机 opaque handle。
pub struct mediaservo_deck_camera_t {
    inner: Arc<CameraInner>,
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// 泵线程主体：50ms 轮询 stop/closed（generator.stop 不释放 sink →
/// recv 永不返回 None，必须超时轮询），帧扇出到 cb + 录制桥。
fn camera_pump_loop(shared: Arc<CameraInner>, mut stream: FrameStream) {
    loop {
        if shared.stop.load(Ordering::SeqCst) || shared.closed.load(Ordering::SeqCst) {
            break;
        }
        match shared.rt.block_on(async {
            tokio::time::timeout(Duration::from_millis(50), stream.recv()).await
        }) {
            Ok(Some(frame)) => deliver_frame(&shared, frame),
            Ok(None) => break, // 帧流结束
            Err(_) => {}       // 超时：回到循环检查 stop/closed
        }
    }
}

/// 单帧扇出：先 cb（借用），再录制桥（move 所有权）。
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
        // 录制任务已结束（rx 已 drop）→ 自愈清除
        if let Ok(mut g) = shared.rec_tx.lock() {
            *g = None;
        }
    }
}

/// 枚举设备（双调用模式）：第一次 `out_ids=NULL` 返回所需长度（不含 NUL，
/// snprintf 约定；错误为负值），第二次填缓冲（截断时同样返回所需长度）。
/// kind: 0=Camera 1=Audio 2=Screen；多设备 '\n' 分隔。
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

/// 打开相机（阻塞仅本地初始化）。`opts` 不可为 null；`dev_id` 必须存在于枚举结果。
/// 成功后 `*out` 指向新 handle（调用方负责 `mediaservo_deck_camera_close`）。
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
        // width/height/framerate 全 0 = 默认 1280x720@30
        let (w, h) = if opts_ref.width > 0 && opts_ref.height > 0 {
            (opts_ref.width, opts_ref.height)
        } else {
            (1280, 720)
        };
        let fps = if opts_ref.framerate > 0 { opts_ref.framerate } else { 30 };
        let copts = CaptureOptions {
            resolution: Some((w, h)),
            framerate: Some(fps),
            format: None, // CameraSource::start 不使用 format
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

/// 开始产帧（用 open 时的 opts；只允许一次，重复调用 → STATE）并启动泵线程。
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

/// 注册帧回调（泵线程逐帧触发；重复调用替换旧回调）。
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

/// 停止产帧（幂等）：置停止标志 → 泵线程 ≤50ms 退出 → 停止帧源。
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

/// 关闭相机并释放 handle（幂等）：置 closed → join 泵线程 → 释放
/// （Drop 链停止帧源）。帧回调期间调用为 UB（契约文档）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_camera_close(c: *mut mediaservo_deck_camera_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            return MEDIASERVO_OK;
        }
        if unsafe { &*c }.inner.closed.load(Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等：已关闭
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

// ── recorder（录制）──

/// 帧桥：录制任务侧的 async 帧源（camera 泵 → unbounded channel）。
struct FrameRx {
    rx: mpsc::UnboundedReceiver<VideoFrame>,
}

impl Frames for FrameRx {
    #[allow(clippy::manual_async_fn)] // 泵线程入口返回显式 Future：async 化牵动线程边界，收益 < 风险
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send {
        async move { self.rx.recv().await }
    }
}

/// recorder 共享状态。
struct RecorderInner {
    recorder: Mutex<Option<Recorder>>,
    /// 录制任务（record 启动；close 时 block_on join 完成 flush+trailer）。
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 外部停止信号（stop 时置位 → 录制循环 50ms 内退出并 flush）。
    stop_signal: Mutex<Option<StopSignal>>,
    closed: AtomicBool,
    rt: tokio::runtime::Runtime,
}

/// 录制器 opaque handle。
pub struct mediaservo_deck_recorder_t {
    inner: Arc<RecorderInner>,
}

/// 创建录制器（RecordOptions 默认 h264/mp4；不启动）。
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

/// 桥接录制：camera 已 start 的帧泵 → recorder 录制任务（内部 flush 收尾）。
/// 契约：camera 必须活到录制结束（关闭顺序 recorder 先于 camera）。
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
        // record() 阻塞到 stop/流结束 → 必须 spawn 到共享 runtime（R1 延续）
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

/// 请求停止录制（幂等）：置 running=false → 录制循环 50ms 内退出并 flush。
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

/// 关闭录制器并释放 handle（幂等）：join 录制任务（flush + trailer 完成）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_recorder_close(r: *mut mediaservo_deck_recorder_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if r.is_null() {
            return MEDIASERVO_OK;
        }
        if unsafe { &*r }.inner.closed.load(Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等：已关闭
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

// ── player（回放）──

/// player 共享状态。
struct PlayerInner {
    player: Mutex<Option<Player>>,
    /// 解码泵线程（frames_cb 启动；close 置 closed → 线程 ≤1 帧退出 → join）。
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
    closed: AtomicBool,
}

/// 回放器 opaque handle。
pub struct mediaservo_deck_player_t {
    inner: Arc<PlayerInner>,
}

/// 打开媒体文件（demux + 解码器就绪；不支持的文件 → PLAYER 错误）。
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

/// 逐帧解码回调泵（同步 next_frame 循环；EOF 或 close 后退出）。
/// 只允许一次（重复调用 → STATE）。
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
                *guard = Some(player); // 放回
                return rc;
            }
        };
        let _shared = Arc::clone(&handle.inner);
        // *mut c_void 非 Send → 线程闭包内经 usize 往返（FFI 惯例）
        let user_tag = user as usize;
        let pump =
            match std::thread::Builder::new().name("deck-player-pump".into()).spawn(move || {
                let user = user_tag as *mut c_void;
                // 泵运行至 EOF（或解码错误）自然结束 — 不被 close 中止：
                // close 语义 = join（阻塞至解码完成），否则 frames_cb 返回后
                // 立即 close 会竞态杀死尚未解码任何帧的泵（0 帧输出）。
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
                    // player 已移入闭包（spawn 失败时随闭包 drop — 资源耗尽级错误，不可恢复）
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

/// 关闭回放器并释放 handle（幂等）：置 closed → join 解码泵（阻塞至 EOF/错误）→ 释放。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_player_close(p: *mut mediaservo_deck_player_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if p.is_null() {
            return MEDIASERVO_OK;
        }
        if unsafe { &*p }.inner.closed.load(Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等：已关闭
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

// ── 通用 ──

/// 最近一次错误的详情（线程安全；无错误时返回空串）。
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

/// 最近错误详情。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_deck_last_error(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| last_error_impl(buf, len)))
        .unwrap_or(MEDIASERVO_DECK_ERR_INTERNAL)
}

/// 版本信息（MAJOR.MINOR.PATCH — D241 soname 语义）。
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
    use super::*;

    /// 辅助：open 一台默认相机（成功后调用方负责 close）。
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
        // 全局状态跨测试竞争: 先清空再设（不依赖其他测试未写）
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
        let mut buf = [0u8; 32];
        let rc = mediaservo_deck_version(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(s.starts_with("0.1."), "version: {s}");
    }

    #[test]
    fn enumerate_two_call_roundtrip() {
        // 第一次: 长度（含分隔符，不含 NUL）；第二次: 内容一致
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
        // Audio/Screen 无设备 → 长度 0
        let mut len: usize = 0;
        let rc = mediaservo_deck_devices_enumerate(1, ptr::null_mut(), 0, &mut len);
        assert_eq!(rc, 0);
        assert_eq!(len, 0);
    }

    #[test]
    fn enumerate_invalid_kind() {
        let rc = mediaservo_deck_devices_enumerate(7, ptr::null_mut(), 0, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn camera_open_null_fails() {
        let rc = mediaservo_deck_camera_open(ptr::null(), ptr::null(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn camera_open_small_struct_size_fails() {
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
        let cam = open_camera();
        assert!(!cam.is_null());
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
        // C 消费者惯例: close 后置 NULL（重复 close 同一指针为 UB — 头文件契约）
        assert_eq!(mediaservo_deck_camera_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn camera_double_start_fails() {
        let cam = open_camera();
        assert_eq!(mediaservo_deck_camera_start(cam), MEDIASERVO_OK);
        let rc = mediaservo_deck_camera_start(cam);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_STATE);
        assert_eq!(mediaservo_deck_camera_stop(cam), MEDIASERVO_OK);
        assert_eq!(mediaservo_deck_camera_stop(cam), MEDIASERVO_OK); // 幂等
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
    }

    #[test]
    fn camera_frames_cb_null_fails() {
        let rc = mediaservo_deck_camera_frames_cb(ptr::null_mut(), None, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        let cam = open_camera();
        let rc = mediaservo_deck_camera_frames_cb(cam, None, ptr::null_mut()); // null cb
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
    }

    #[test]
    fn recorder_new_null_fails() {
        let mut out: *mut mediaservo_deck_recorder_t = ptr::null_mut();
        let rc = mediaservo_deck_recorder_new(ptr::null(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn recorder_new_missing_parent_fails() {
        // 父目录不存在 → NotFound → RECORDER
        let mut out: *mut mediaservo_deck_recorder_t = ptr::null_mut();
        let p = c"/tmp/opencode/no-such-dir-xyz/deck_test.mp4";
        let rc = mediaservo_deck_recorder_new(p.as_ptr(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_RECORDER);
        assert!(out.is_null());
    }

    #[test]
    fn recorder_record_null_fails() {
        let rc = mediaservo_deck_recorder_record(ptr::null_mut(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn recorder_record_camera_not_started_fails() {
        let cam = open_camera();
        let mut rec: *mut mediaservo_deck_recorder_t = ptr::null_mut();
        let p = c"/tmp/opencode/deck_test_never_written.mp4";
        let rc = mediaservo_deck_recorder_new(p.as_ptr(), &mut rec);
        assert_eq!(rc, MEDIASERVO_OK);
        let rc = mediaservo_deck_recorder_record(rec, cam); // camera 未 start
        assert_eq!(rc, MEDIASERVO_DECK_ERR_STATE);
        assert_eq!(mediaservo_deck_recorder_close(rec), MEDIASERVO_OK);
        assert_eq!(mediaservo_deck_camera_close(cam), MEDIASERVO_OK);
    }

    #[test]
    fn recorder_close_null_is_ok() {
        assert_eq!(mediaservo_deck_recorder_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn player_open_missing_file_fails() {
        let mut out: *mut mediaservo_deck_player_t = ptr::null_mut();
        let p = c"/tmp/opencode/no-such-file-xyz.mp4";
        let rc = mediaservo_deck_player_open(p.as_ptr(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_PLAYER);
        assert!(out.is_null());
    }

    #[test]
    fn player_open_null_fails() {
        let mut out: *mut mediaservo_deck_player_t = ptr::null_mut();
        let rc = mediaservo_deck_player_open(ptr::null(), &mut out);
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn player_close_null_is_ok() {
        assert_eq!(mediaservo_deck_player_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn recorder_stop_null_fails() {
        let rc = mediaservo_deck_recorder_stop(ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }

    #[test]
    fn player_frames_cb_null_fails() {
        let rc = mediaservo_deck_player_frames_cb(ptr::null_mut(), None, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_DECK_ERR_INVALID_ARG);
    }
}
