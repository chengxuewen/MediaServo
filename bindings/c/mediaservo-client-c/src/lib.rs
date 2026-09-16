//! MediaServo client C ABI — 舱端/消费侧 SDK（登录 + SFU 视频消费 + 控制 DataChannel）。
//!
//! 契约 §7（D109/D240/D241）：opaque handle + int 错误码 + 回调。
//! 模式基准 = bindings/c/mediaservo-link-c（生命周期/struct_size/pump 纪律同形）。
//! Rust 侧全部能力来自 `mediaservo-client` v2（本 crate 只做 FFI 形，零业务逻辑）。
//!
//! # 生命周期契约（承 link-c R2）
//! - handle 单线程属主；close 后任何 API 调用为 UB（幂等 close 同 link 纪律）。
//! - session_close = 置 closed 标志 → 关会话（帧通道随之消亡）→ join 视频泵 → 才 free。
//! - 控制 handle 独立关闭；ack 泵每轮 recv_ack 有界（1s），closed 标志 ≤1s 内生效。
//! - 帧/ack 回调仅在各自泵线程触发；回调调用期间不持任何锁；回调内禁止调用
//!   任何 ms_client_* API（含 close）——未定义行为。
//! - 回调内 JSON 字符串与 frame.data 指针仅在回调内有效（需保留请拷贝）。
//!
//! # runtime（审核 R1 的 client 形态）
//! `RoomSession::open_control` 内部 `tokio::spawn(ack_consumer_pump)`（session.rs），
//! link `SignalClient::connect` spawn WS 读循环——后台任务必须存活于任何单次
//! block_on 之外，故进程级共享 multi_thread runtime（OnceLock，C28 模式）。与
//! link-c 的 per-handle runtime 不同：client 后台任务与会话句柄跨调用共生，
//! 共享实例天然覆盖且省线程。
//!
//! # 错误映射
//! [`ClientError`] 全 14 变体 → `MEDIASERVO_CLIENT_ERR_*` 穷尽 match（单测钉，
//! 新变体入 enum 即编译红，静默漏映射不可能）。
//!
//! C ABI 面 raw pointer 参数是签名刚需：全部解引用点先 null 校验、包
//! catch_unwind，头文件载明属主/生命周期契约——clippy 的 safe-API 误用模型
//! 不适用于本 crate（link-c/field-c 同族问题系存量债，工作区 lint 红在 HEAD
//! 已登记另案；本 crate 以显式 allow 保持自身零告警）。
#![allow(clippy::not_unsafe_ptr_arg_deref)]

mod config;
mod errors;

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use mediaservo_client::error::ClientError;
use mediaservo_client::{ClientConfig, ControlChannel, RoomSession, VideoFrame};
use tokio::sync::mpsc;
use tokio::time::timeout;

use config::{
    ms_client_config_t, ms_client_login_config_t, validate_login_cfg, validate_session_cfg,
};
use errors::{
    MEDIASERVO_CLIENT_ERR_INVALID_ARG, MEDIASERVO_CLIENT_ERR_INTERNAL,
    MEDIASERVO_CLIENT_ERR_STATE, MEDIASERVO_OK, build_envelope, copy_out_str, cstr, error_code,
    last_error_impl, set_last_error,
};

pub use config::{MEDIASERVO_CLIENT_CONFIG_MIN_SIZE, MS_CLIENT_LOGIN_CONFIG_MIN_SIZE};
pub use errors::{
    MEDIASERVO_CLIENT_ERR_DENIED, MEDIASERVO_CLIENT_ERR_LOGIN, MEDIASERVO_CLIENT_ERR_MALFORMED,
    MEDIASERVO_CLIENT_ERR_PROTOCOL, MEDIASERVO_CLIENT_ERR_SIGNAL, MEDIASERVO_CLIENT_ERR_TIMEOUT,
    MEDIASERVO_CLIENT_ERR_UNAUTHORIZED,
};

/// 等房间内首路视频 producer 上限（basic.rs 同值：late-join NewProducer 回放窗）。
const VIDEO_PRODUCER_WAIT: Duration = Duration::from_secs(30);
/// ack 泵单轮 recv_ack 窗（closed 标志生效上界 = 本值）。
const ACK_POLL: Duration = Duration::from_secs(1);
/// 视频泵空转复查窗（closed / 通道消亡的轮询粒度）。
const VIDEO_POLL: Duration = Duration::from_secs(2);

/// 进程级共享 multi_thread runtime（见模块头 runtime 节）。
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("client-c runtime")
    })
}

// ── 会话 ──

/// 房间会话 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub struct ms_client_session_t {
    /// close 时 take 出并消费（RoomSession::close(self) 签名形态）。
    session: std::sync::Mutex<Option<RoomSession>>,
    /// connect 时快照（negotiated 会话期恒定）。
    negotiated: u32,
    closed: AtomicBool,
    /// 视频泵一次性闸门（首调用置位；失败路径回滚）。
    video_started: AtomicBool,
    /// 帧回调（泵线程每轮读取；user 指针存于句柄内不跨线程搬运——link-c 同形）。
    video_cb: std::sync::Mutex<Option<(ms_client_frame_cb, *mut c_void)>>,
    video_pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

// SAFETY: 内部 Mutex<Option<RoomSession>> 序列化访问；泵线程仅经该锁与原子标志读取。
unsafe impl Send for ms_client_session_t {}
// SAFETY: 所有字段访问经 Mutex/AtomicBool。
unsafe impl Sync for ms_client_session_t {}

/// 视频泵线程：帧通道 → C 回调（帧数据 Vec 存活跨越回调调用点，data 仅回调内有效）。
fn video_pump(s: *mut ms_client_session_t, mut rx: mpsc::Receiver<VideoFrame>) {
    let h = unsafe { &*s };
    let rt = runtime();
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        // 注意: `timeout(..)` 若作 block_on 的外侧实参会在**无 context 的本线程**构造
        //        （Sleep::new_timeout → Handle::current panic，spike 实锤）——必须在 async 块内构造。
        match rt.block_on(async { timeout(VIDEO_POLL, rx.recv()).await }) {
            Ok(Some(frame)) => {
                let Some((cb, user)) = h.video_cb.lock().ok().and_then(|g| *g) else {
                    continue; // 未注册/已取消注册：帧丢弃（latest 语义泵）
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
            Ok(None) => break, // 会话 drop → 帧通道消亡（正常收敛）
            Err(_) => continue, // VIDEO_POLL 空转，复查 closed
        }
    }
}

/// 账号登录换 JWT（阻塞；JWT 拷入 out_token，NUL 结尾）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_login(
    cfg: *const ms_client_login_config_t,
    out_token: *mut c_char,
    cap: usize,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if cfg.is_null() {
            set_last_error("ms_client_login: null cfg");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let (base, user, pass) = match validate_login_cfg(unsafe { &*cfg }) {
            Ok(v) => v,
            Err(code) => return code,
        };
        if out_token.is_null() {
            set_last_error("ms_client_login: null out_token");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        match runtime().block_on(mediaservo_client::login(base, user, pass)) {
            Ok(outcome) => copy_out_str(&outcome.jwt, out_token, cap),
            Err(e) => {
                set_last_error(format!("ms_client_login: {e}"));
                error_code(&e)
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_login: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 信令连接 + 入房（阻塞）。成功 `*out` = 新 handle（调用方 close）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_session_create(
    cfg: *const ms_client_config_t,
    out: *mut *mut ms_client_session_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if cfg.is_null() || out.is_null() {
            set_last_error("ms_client_session_create: null cfg/out");
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
            // C 面暂不暴露急停 HMAC key（W2-C 增票：ms_client_config 扩字段+0600 文件形，G13 语义）——
            // None = C 侧 estop 不签名，与 S4 前行为一致（车端有 key 时拒签=正确裁决，非静默）。
            hmac_key: None,
        };
        match runtime().block_on(RoomSession::connect(&client_cfg)) {
            Ok(session) => {
                let negotiated = session.negotiated();
                let handle = Box::new(ms_client_session_t {
                    session: std::sync::Mutex::new(Some(session)),
                    negotiated,
                    closed: AtomicBool::new(false),
                    video_started: AtomicBool::new(false),
                    video_cb: std::sync::Mutex::new(None),
                    video_pump: std::sync::Mutex::new(None),
                });
                unsafe { *out = Box::into_raw(handle) };
                MEDIASERVO_OK
            }
            Err(e) => {
                set_last_error(format!("ms_client_session_create: {e}"));
                error_code(&e)
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_session_create: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 谈成的方言版本（S0；旧 server = 1）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_session_negotiated(
    s: *const ms_client_session_t,
    out_protocol: *mut u32,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() || out_protocol.is_null() {
            set_last_error("ms_client_session_negotiated: null handle/out");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let h = unsafe { &*s };
        if h.closed.load(Ordering::SeqCst) {
            set_last_error("ms_client_session_negotiated: session closed");
            return MEDIASERVO_CLIENT_ERR_STATE;
        }
        unsafe { *out_protocol = h.negotiated };
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_session_negotiated: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 消费房间第一路视频（阻塞至 consume 建立，≤30s producer 等待窗）。
/// 首调用启动帧泵线程；重复调用 → STATE（泵不双开，无重放语义）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_session_consume_video(
    s: *mut ms_client_session_t,
    cb: Option<ms_client_frame_cb>, // FFI-safe NPO
    user: *mut c_void,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(cb) = cb else {
            set_last_error("ms_client_session_consume_video: cb required");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        };
        if s.is_null() {
            set_last_error("ms_client_session_consume_video: null handle");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let h = unsafe { &*s };
        if h.closed.load(Ordering::SeqCst) {
            set_last_error("ms_client_session_consume_video: session closed");
            return MEDIASERVO_CLIENT_ERR_STATE;
        }
        if h.video_started.swap(true, Ordering::SeqCst) {
            set_last_error("ms_client_session_consume_video: video pump already started");
            return MEDIASERVO_CLIENT_ERR_STATE;
        }
        let rt = runtime();
        // 建立期持会话锁（单线程属主契约内）；泵线程不再触碰会话。
        let frames = {
            let mut guard = match h.session.lock() {
                Ok(g) => g,
                Err(_) => {
                    h.video_started.store(false, Ordering::SeqCst);
                    set_last_error("ms_client_session_consume_video: lock poisoned");
                    return MEDIASERVO_CLIENT_ERR_INTERNAL;
                }
            };
            let Some(session) = guard.as_mut() else {
                h.video_started.store(false, Ordering::SeqCst);
                set_last_error("ms_client_session_consume_video: session closed");
                return MEDIASERVO_CLIENT_ERR_STATE;
            };
            let produced = match rt.block_on(session.wait_video_producer(VIDEO_PRODUCER_WAIT)) {
                Ok(p) => p,
                Err(e) => {
                    h.video_started.store(false, Ordering::SeqCst);
                    set_last_error(format!("ms_client_session_consume_video: wait producer: {e}"));
                    return error_code(&e);
                }
            };
            match rt.block_on(session.consume_video(&produced)) {
                Ok(rx) => rx,
                Err(e) => {
                    h.video_started.store(false, Ordering::SeqCst);
                    set_last_error(format!("ms_client_session_consume_video: consume: {e}"));
                    return error_code(&e);
                }
            }
        };
        if let Ok(mut g) = h.video_cb.lock() {
            *g = Some((cb, user));
        }
        // 裸指针非 Send：句柄经 usize 传递（值语义，仅地址搬运），回调存于句柄内
        // 每轮读取 —— 同 link-c 泵纪律。
        let raw = s as usize;
        let join = std::thread::spawn(move || video_pump(raw as *mut ms_client_session_t, frames));
        if let Ok(mut g) = h.video_pump.lock() {
            *g = Some(join);
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_session_consume_video: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 关闭会话并释放 handle（幂等）。顺序：closed 标志 → 关会话（帧通道消亡唤醒泵）
/// → join 视频泵 → free。泵正在回调中时 join 阻塞——回调必须快速返回。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_session_close(s: *mut ms_client_session_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(s) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等（同 link-c 双 close 纪律）
        }
        let session = handle.session.lock().ok().and_then(|mut g| g.take());
        if let Some(session) = session
            && let Err(e) = runtime().block_on(session.close())
        {
            set_last_error(format!("ms_client_session_close: {e}"));
        }
        if let Some(join) = handle.video_pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join();
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_session_close: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

// ── 控制通道 ──

/// 控制通道集 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub struct ms_client_control_t {
    ctl: std::sync::Mutex<ControlChannel>,
    closed: AtomicBool,
    cb: std::sync::Mutex<Option<(ms_client_ack_cb, *mut c_void)>>,
    pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

// SAFETY: 内部 Mutex<ControlChannel>（libwebrtc DC/PC 句柄线程安全，livekit 惯例）。
unsafe impl Send for ms_client_control_t {}
// SAFETY: 所有方法经 Mutex/AtomicBool 序列化。
unsafe impl Sync for ms_client_control_t {}

/// ack 泵线程：recv_ack(1s) 轮询 → 当前回调（每轮重读 cb——重复注册替换即时生效）。
fn ack_pump(c: *mut ms_client_control_t) {
    let h = unsafe { &*c };
    let rt = runtime();
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        let item = match h.ctl.lock() {
            Ok(mut g) => rt.block_on(g.recv_ack(ACK_POLL)),
            Err(_) => break, // poisoned = 有线程 panic 在临界区，收敛退出
        };
        match item {
            Ok(ack) => {
                let json = serde_json::to_string(&ack).unwrap_or_default();
                let cb = h.cb.lock().ok().and_then(|g| *g);
                if let Some((cb, user)) = cb {
                    // CString 存活至回调返回（serde_json 输出无内嵌 NUL）。
                    if let Ok(cstr) = CString::new(json) {
                        cb(c, cstr.as_ptr(), user);
                    }
                }
            }
            Err(e) => {
                if matches!(e, ClientError::Timeout { .. }) {
                    continue; // 单轮窗空，正常续
                }
                set_last_error(format!("ack pump: {e}"));
                break; // InvalidState（DC 断开）等终态
            }
        }
    }
}

/// 开出程控制通道集（每会话一次性——ack 泵每会话一条，Rust 侧闸门）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_open_control(
    s: *mut ms_client_session_t,
    labels: *const *const c_char,
    labels_len: usize,
    out: *mut *mut ms_client_control_t,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null()
            || out.is_null()
            || (!labels.is_null() && labels_len == 0)
            || (labels.is_null() && labels_len > 0)
        {
            set_last_error("ms_client_open_control: null handle/labels-mismatch");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let h = unsafe { &*s };
        if h.closed.load(Ordering::SeqCst) {
            set_last_error("ms_client_open_control: session closed");
            return MEDIASERVO_CLIENT_ERR_STATE;
        }
        let mut label_vec: Vec<&str> = Vec::with_capacity(labels_len);
        for i in 0..labels_len {
            match cstr(unsafe { *labels.add(i) }) {
                Ok(Some(l)) if !l.is_empty() => label_vec.push(l),
                _ => {
                    set_last_error(format!(
                        "ms_client_open_control: label[{i}] missing or invalid UTF-8"
                    ));
                    return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
                }
            }
        }
        let channel = {
            let mut guard = match h.session.lock() {
                Ok(g) => g,
                Err(_) => {
                    set_last_error("ms_client_open_control: lock poisoned");
                    return MEDIASERVO_CLIENT_ERR_INTERNAL;
                }
            };
            let Some(session) = guard.as_mut() else {
                set_last_error("ms_client_open_control: session closed");
                return MEDIASERVO_CLIENT_ERR_STATE;
            };
            match runtime().block_on(session.open_control(&label_vec)) {
                Ok(ctl) => ctl,
                Err(e) => {
                    set_last_error(format!("ms_client_open_control: {e}"));
                    return error_code(&e);
                }
            }
        };
        let handle = Box::new(ms_client_control_t {
            ctl: std::sync::Mutex::new(channel),
            closed: AtomicBool::new(false),
            cb: std::sync::Mutex::new(None),
            pump: std::sync::Mutex::new(None),
        });
        unsafe { *out = Box::into_raw(handle) };
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_open_control: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 注册 ack 回调（首次注册启动泵；重复注册替换。cb=NULL 取消注册且不再触发）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_control_on_ack(
    c: *mut ms_client_control_t,
    cb: Option<ms_client_ack_cb>, // FFI-safe NPO
    user: *mut c_void,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            set_last_error("ms_client_control_on_ack: null handle");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let h = unsafe { &*c };
        if h.closed.load(Ordering::SeqCst) {
            set_last_error("ms_client_control_on_ack: control closed");
            return MEDIASERVO_CLIENT_ERR_STATE;
        }
        {
            let mut guard = match h.cb.lock() {
                Ok(g) => g,
                Err(_) => {
                    set_last_error("ms_client_control_on_ack: lock poisoned");
                    return MEDIASERVO_CLIENT_ERR_INTERNAL;
                }
            };
            *guard = cb.map(|cb| (cb, user));
        }
        // 首次注册非空回调时启动 ack 泵（同 link on_event 纪律）。
        if cb.is_some() {
            let mut pump_guard = match h.pump.lock() {
                Ok(g) => g,
                Err(_) => {
                    set_last_error("ms_client_control_on_ack: lock poisoned");
                    return MEDIASERVO_CLIENT_ERR_INTERNAL;
                }
            };
            if pump_guard.is_none() {
                let raw = c as usize;
                let join = std::thread::spawn(move || ack_pump(raw as *mut ms_client_control_t));
                *pump_guard = Some(join);
            }
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_control_on_ack: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 发送一条命令信封（serde 构造，非拼接）。payload_json NULL/"" = null payload。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_control_send(
    c: *mut ms_client_control_t,
    label: *const c_char,
    seq: u64,
    cmd: *const c_char,
    payload_json: *const c_char,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            set_last_error("ms_client_control_send: null handle");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let h = unsafe { &*c };
        if h.closed.load(Ordering::SeqCst) {
            set_last_error("ms_client_control_send: control closed");
            return MEDIASERVO_CLIENT_ERR_STATE;
        }
        let (label_s, cmd_s, payload_s) = match (cstr(label), cstr(cmd), cstr(payload_json)) {
            (Ok(Some(l)), Ok(Some(cm)), Ok(p)) => (l, cm, p),
            _ => {
                set_last_error("ms_client_control_send: label/cmd required, UTF-8 valid");
                return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
            }
        };
        let env = match build_envelope(seq, cmd_s, payload_s) {
            Ok(env) => env,
            Err(code) => return code,
        };
        let guard = match h.ctl.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("ms_client_control_send: lock poisoned");
                return MEDIASERVO_CLIENT_ERR_INTERNAL;
            }
        };
        match runtime().block_on(guard.send_envelope(label_s, &env)) {
            Ok(()) => MEDIASERVO_OK,
            Err(e) => {
                set_last_error(format!("ms_client_control_send: {e}"));
                error_code(&e)
            }
        }
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_control_send: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// producer id JSON 数组（观测面，镜像 ControlChannel::producer_ids()）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_control_producer_ids(
    c: *const ms_client_control_t,
    out_json: *mut c_char,
    cap: usize,
) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() || out_json.is_null() {
            set_last_error("ms_client_control_producer_ids: null handle/out");
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        let h = unsafe { &*c };
        if h.closed.load(Ordering::SeqCst) {
            set_last_error("ms_client_control_producer_ids: control closed");
            return MEDIASERVO_CLIENT_ERR_STATE;
        }
        let guard = match h.ctl.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("ms_client_control_producer_ids: lock poisoned");
                return MEDIASERVO_CLIENT_ERR_INTERNAL;
            }
        };
        let json = match serde_json::to_string(guard.producer_ids()) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(format!("ms_client_control_producer_ids: serialize: {e}"));
                return MEDIASERVO_CLIENT_ERR_INTERNAL;
            }
        };
        copy_out_str(&json, out_json, cap)
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_control_producer_ids: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 关闭控制通道并释放 handle（幂等；置 closed → join ack 泵 → free）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_control_close(c: *mut ms_client_control_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if c.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(c) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等
        }
        if let Some(join) = handle.pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join(); // 泵单轮 ≤ACK_POLL，join 有界
        }
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("ms_client_control_close: panic");
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

// ── 回调类型 + 帧结构镜像 ──

/// 视频帧回调（仅在 consume 泵线程触发；frame 指针仅回调内有效）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub type ms_client_frame_cb =
    extern "C" fn(*mut ms_client_session_t, *const mediaservo_client_frame_t, *mut c_void);

/// ack 回调（仅在 ack 泵线程触发；ack_json 仅回调内有效）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub type ms_client_ack_cb = extern "C" fn(*mut ms_client_control_t, *const c_char, *mut c_void);

/// C 侧帧描述（自然对齐，与 client.h 一致；data 仅回调内有效）。
#[allow(non_camel_case_types)]
#[repr(C)]
pub struct mediaservo_client_frame_t {
    pub width: u32,
    pub height: u32,
    pub ts_us: i64,
    pub data: *const u8,
    pub len: usize,
}

// ── 通用 ──

/// 最近错误详情。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_last_error(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| last_error_impl(buf, len)))
        .unwrap_or(MEDIASERVO_CLIENT_ERR_INTERNAL)
}

/// 版本信息（MAJOR.MINOR.PATCH — D241 soname 语义）。
#[unsafe(no_mangle)]
pub extern "C" fn ms_client_version(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if buf.is_null() || len == 0 {
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        copy_out_str(env!("CARGO_PKG_VERSION"), buf, len)
    }))
    .unwrap_or(MEDIASERVO_CLIENT_ERR_INTERNAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FFI 入口 null 守卫（不触网）──
    #[test]
    fn login_null_cfg_fails() {
        let mut buf = [0u8; 64];
        let rc = ms_client_login(ptr::null(), buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn session_create_null_fails() {
        let rc = ms_client_session_create(ptr::null(), ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn session_negotiated_null_fails() {
        let mut p = 0u32;
        let rc = ms_client_session_negotiated(ptr::null(), &mut p);
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn consume_video_null_cb_fails() {
        let rc = ms_client_session_consume_video(ptr::null_mut(), None, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn open_control_null_fails() {
        let rc = ms_client_open_control(ptr::null_mut(), ptr::null(), 0, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // labels 非空但 len=0 → 拒
        let labels = [c"chassis".as_ptr()];
        let rc = ms_client_open_control(ptr::null_mut(), labels.as_ptr(), 0, ptr::null_mut());
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    }

    #[test]
    fn control_apis_null_fails() {
        let mut buf = [0u8; 64];
        assert_eq!(
            ms_client_control_on_ack(ptr::null_mut(), None, ptr::null_mut()),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(
            ms_client_control_send(
                ptr::null_mut(),
                c"chassis".as_ptr(),
                1,
                c"steer".as_ptr(),
                ptr::null()
            ),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        assert_eq!(
            ms_client_control_producer_ids(ptr::null_mut(), buf.as_mut_ptr() as *mut c_char, 64),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
    }

    #[test]
    fn close_null_is_ok() {
        assert_eq!(ms_client_session_close(ptr::null_mut()), MEDIASERVO_OK);
        assert_eq!(ms_client_control_close(ptr::null_mut()), MEDIASERVO_OK);
    }

    #[test]
    fn version_roundtrip() {
        let mut buf = [0u8; 32];
        let rc = ms_client_version(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
        let s = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_str()
            .unwrap();
        assert!(s.starts_with("0.1."), "version: {s}");
    }

    #[test]
    fn last_error_ffi_roundtrip() {
        set_last_error("client-c ffi error");
        let mut buf = [0u8; 64];
        let rc = ms_client_last_error(buf.as_mut_ptr() as *mut c_char, buf.len());
        assert_eq!(rc, MEDIASERVO_OK);
    }
}
