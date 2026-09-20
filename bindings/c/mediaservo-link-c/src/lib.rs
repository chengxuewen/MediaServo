#![allow(clippy::not_unsafe_ptr_arg_deref)]
// C ABI 门面形态：extern fn 收裸指针是合同本身，判空守卫在体内（unsafe fn 形反而把义务推给 C 侧）
//! MediaServo link C ABI — 信令 + 帧总线（设备侧 SDK 消费）。
//!
//! 契约 §7（D109/D240/D241）：opaque handle + int 错误码 + 回调。
//! 模式基准 = bindings/c/mediaservo-field-c（slice 1 已交付并 live e2e 实证）。
//!
//! # 生命周期契约（审核 R2）
//! - handle 单线程属主；close 后任何 API 调用为 UB（除幂等 close）。
//! - close = 置 closed 标志 → 释放会话（唤醒事件泵）→ join 泵线程 → 才 free handle。
//! - 事件回调仅在一个内部泵线程触发；回调调用期间不持任何锁；
//!   回调内禁止调用任何 mediaservo_link_signal_* API（含 close）— 未定义行为。
//! - `mediaservo_link_signal_on_event` 可在 connect 后任意时刻注册（重复注册替换回调）；
//!   首次注册时启动事件泵并合成补发 Connected 事件（broadcast 订阅前的事件不可见）。
//! - 事件 JSON 字符串仅在回调内有效（需保留请拷贝）。
//!
//! # runtime（审核 R1）
//! `SignalClient::connect` 内部 `tokio::spawn(session_task)`（WS 读循环）——per-call
//! runtime 会在返回时取消 spawn 任务导致会话死亡。因此每个 signal handle 持有
//! 共享 multi_thread runtime，全部 C 调用 block_on 同一实例；事件泵复用该 runtime。
//!
//! # FrameMeta（R4）
//! mediaservo_frame_meta_t（C 侧 #pragma pack(1)）为字段袋：Rust 侧经 36B 拷贝 +
//! [`FrameMeta::decode`] 逐字段读取，禁止整块 reinterpret（填充/字节序风险）。
//!
//! # C ABI 面（手工维护头文件, MAJOR 内稳定）
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

// ── 错误码（0 = ok, <0 = error；MEDIASERVO_LINK_ERR_*，D241 前缀化）──
pub const MEDIASERVO_OK: c_int = 0;
pub const MEDIASERVO_LINK_ERR_INVALID_ARG: c_int = -1;
pub const MEDIASERVO_LINK_ERR_CONNECT: c_int = -2;
pub const MEDIASERVO_LINK_ERR_SEND: c_int = -3;
pub const MEDIASERVO_LINK_ERR_BUS: c_int = -4;
pub const MEDIASERVO_LINK_ERR_STATE: c_int = -5;
pub const MEDIASERVO_LINK_ERR_INTERNAL: c_int = -6;
pub const MEDIASERVO_LINK_ERR_CLOSED: c_int = -7;

/// 全局最近错误信息（mediaservo_link_last_error 读取）。
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = LAST_ERROR.lock() {
        *guard = Some(msg.into());
    }
}

// ── 内部辅助 ──

/// 提取 C 字符串（null → None）。非法 UTF-8 → Err。
fn cstr<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().map(Some).map_err(|_| ())
}

/// 新建共享 multi_thread runtime（每个 handle 一个，R1）。
fn new_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("link-c runtime")
}

/// C 侧 mediaservo_frame_meta_t → 36B 拷贝 → FrameMeta::decode（逐字段，R4）。
fn meta_from_c(meta: *const mediaservo_frame_meta_t) -> Result<FrameMeta, ()> {
    let mut buf = [0u8; FrameMeta::WIRE_LEN];
    unsafe {
        ptr::copy_nonoverlapping(meta as *const u8, buf.as_mut_ptr(), FrameMeta::WIRE_LEN);
    }
    FrameMeta::decode(&buf).map_err(|_| ())
}

/// FrameMeta::encode → 36B 拷贝到 C 侧 mediaservo_frame_meta_t。
fn meta_to_c(meta: &FrameMeta, out: *mut mediaservo_frame_meta_t) {
    let bytes = meta.encode();
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, FrameMeta::WIRE_LEN);
    }
}

// ── 信令 ──

/// 信令配置（C 结构 — 与 SignalClient 参数映射）。
///
/// 首字段 `struct_size`（审核 R3）：调用方填 `sizeof(mediaservo_link_signal_config_t)`，
/// 库校验 `>= sizeof(已知结构)`、超长忽略 —— 结构演进不破坏二进制兼容。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外：mediaservo_* 前缀）
#[repr(C)]
pub struct mediaservo_link_signal_config_t {
    pub struct_size: usize,
    /// 信令 WS 地址（如 "ws://host:9800/ws"）。
    pub url: *const c_char,
    /// PSK 认证密钥。
    pub psk: *const c_char,
    /// 房间 ID。
    pub room: *const c_char,
    /// 角色：`"Host"`/`"Pusher"` → Host，`"Client"`/`"Puller"` → Remote；NULL/空 = Host。
    pub role: *const c_char,
}

/// C 结构已知前缀尺寸（版本演进时的最小合法值）。
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

/// C 侧角色字符串 → PeerRole。
fn parse_role(s: &str) -> Result<PeerRole, ()> {
    match s {
        "Host" | "Pusher" => Ok(PeerRole::Host),
        "Client" | "Puller" => Ok(PeerRole::Remote),
        _ => Err(()),
    }
}

/// 信令会话 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外：mediaservo_* 前缀）
pub struct mediaservo_link_signal_t {
    session: std::sync::Mutex<Option<SignalSession>>,
    /// 共享 multi_thread runtime（R1）：session 后台任务（WS 读循环）
    /// 存活于本 runtime，全部 C 调用 block_on 同一实例。
    rt: tokio::runtime::Runtime,
    /// 已关闭标志（close 幂等 + 入口校验）。
    closed: AtomicBool,
    /// connect 时的房间 ID（泵合成 Connected 事件用）。
    room_id: String,
    /// connect 时立即订阅的事件接收器（缓冲 connect→on_event 之间的事件，
    /// broadcast 只保留订阅后发送的消息）；泵启动时接管。
    events_rx: std::sync::Mutex<Option<broadcast::Receiver<SignalEvent>>>,
    /// 事件回调（泵线程每轮读取；重复注册替换）。
    cb: std::sync::Mutex<Option<(mediaservo_link_event_cb, *mut c_void)>>,
    /// 事件泵线程（首次 on_event 注册时启动，R2）。
    pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

// SAFETY: handle 内部为 Mutex<Option<SignalSession>>（线程安全）+ Send runtime。
unsafe impl Send for mediaservo_link_signal_t {}
// SAFETY: 所有方法经 Mutex 序列化访问内部会话。
unsafe impl Sync for mediaservo_link_signal_t {}

/// 信令事件回调（C ABI）。event_json 仅在回调内有效。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外：mediaservo_* 前缀）
pub type mediaservo_link_event_cb =
    extern "C" fn(*mut mediaservo_link_signal_t, *const c_char, *mut c_void);

/// SignalEvent → opaque JSON 字符串（v1）。
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
        // #[non_exhaustive]：新变体加在事件 JSON 契约演进（opaque v1 未知类型）
        _ => serde_json::json!({"type": "unknown"}),
    };
    v.to_string()
}

/// 调 C 回调（不持任何锁 — cb 克隆后释放 guard，R2）。
fn deliver_event(handle: *mut mediaservo_link_signal_t, json: String) {
    let h = unsafe { &*handle };
    let cb = h.cb.lock().ok().and_then(|g| *g);
    if let Some((cb, user)) = cb {
        // CString 存活至回调返回（serde_json 输出无内嵌 NUL）。
        if let Ok(cstr) = CString::new(json) {
            cb(handle, cstr.as_ptr(), user);
        }
    }
}

/// 事件泵线程（R2）：先合成补发 Connected（broadcast 订阅前的事件不可见），
/// 然后循环转发事件。会话关闭（广播 sender 全 drop）或 closed 标志 → 退出。
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
            Err(broadcast::error::RecvError::Closed) => break, // 会话已关闭
            Err(broadcast::error::RecvError::Lagged(_)) => continue, // 溢出丢弃（文档）
        }
    }
}

/// 连接信令并创建会话（阻塞）。
///
/// `cfg` 不可为 null；`url/psk/room` 必填；`role` 可空（默认 Host）；
/// `cfg.struct_size` 必须 `>= sizeof(mediaservo_link_signal_config_t)`（R3）。
/// 成功后 `*out` 指向新 handle（调用方负责 `mediaservo_link_signal_close`）。
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
            Ok(None) => PeerRole::Host, // 车端默认
            Err(()) => {
                set_last_error("mediaservo_link_signal_connect: invalid UTF-8 in role");
                return MEDIASERVO_LINK_ERR_INVALID_ARG;
            }
        };

        let rt = new_runtime();
        let client = SignalClient::new(url, psk, room, role);
        match rt.block_on(client.connect()) {
            Ok(session) => {
                // 立即订阅：缓冲 connect→on_event 之间的事件（广播只保留订阅后消息）。
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

/// 发送一条信令消息（JSON 字符串 + 字节长度；阻塞入队）。
///
/// 消息类型为 [`SignalingMessage`]（type 标签 snake_case）。解析失败 → SEND 错误。
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

/// 注册事件回调（connect 后任意时刻；重复注册替换）。
///
/// 首次注册时启动事件泵线程：先合成补发 Connected 事件，再循环转发。
/// 回调仅在一个泵线程触发；回调内禁止调用任何 mediaservo_link_signal_* API（含 close）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_signal_on_event(
    s: *mut mediaservo_link_signal_t,
    cb: Option<mediaservo_link_event_cb>, // NULL = 取消注册（FFI-safe NPO）
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
            *guard = cb.map(|cb| (cb, user)); // NULL 回调 = 取消注册
        }
        // 首次注册时启动事件泵（R2）。
        let mut pump_guard = match handle.pump.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("mediaservo_link_signal_on_event: lock poisoned");
                return;
            }
        };
        if pump_guard.is_none() {
            // NonNull<T>: T: Send 时 Send（mediaservo_link_signal_t 已 unsafe impl Send）。
            // 裸指针非 Send：经 usize 传递（值语义，仅地址搬运）。
            let raw = s as usize;
            let join =
                std::thread::spawn(move || signal_pump(raw as *mut mediaservo_link_signal_t));
            *pump_guard = Some(join);
        }
    }));
}

/// 关闭信令会话并释放 handle（幂等）。
///
/// 顺序（R2）：置 closed 标志 → 释放会话（广播 sender 全 drop，泵 recv 返回 Closed
/// 退出）→ join 事件泵线程 → drop runtime → 释放 handle 内存。
/// 泵正在回调中时 join 阻塞——回调必须快速返回，回调内禁止调 close。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_signal_close(s: *mut mediaservo_link_signal_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if s.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(s) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等：已关闭
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

// ── 帧总线 ──

/// 帧总线 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外：mediaservo_* 前缀）
pub struct mediaservo_link_bus_t {
    bus: std::sync::Mutex<Option<FrameBus>>,
    closed: AtomicBool,
}

// SAFETY: 内部为 Mutex<Option<FrameBus>>（iceoryx2 ipc_threadsafe 线程安全）。
unsafe impl Send for mediaservo_link_bus_t {}
unsafe impl Sync for mediaservo_link_bus_t {}

/// 帧流 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外：mediaservo_* 前缀）
pub struct mediaservo_link_stream_t {
    stream: FrameStream,
    /// 每流一个 runtime（recv 阻塞取帧用；帧状态在共享 inner，不依赖本 runtime）。
    rt: tokio::runtime::Runtime,
    /// 关停信号：stream_close 唤醒阻塞中的 recv → None → CLOSED。
    shutdown: Arc<Notify>,
    closed: AtomicBool,
}

// SAFETY: stream 内部为 Arc<StreamInner>（Mutex + Notify）+ Send runtime。
unsafe impl Send for mediaservo_link_stream_t {}
unsafe impl Sync for mediaservo_link_stream_t {}

/// recv 实现：取帧或关停（select 两路，bus close / stream close 均唤醒）。
async fn stream_recv_impl(stream: &FrameStream, shutdown: &Notify) -> Option<FrameRef> {
    tokio::select! {
        f = stream.recv() => f,
        _ = shutdown.notified() => None,
    }
}

/// 附加帧总线（验签 + ACL + iceoryx2 节点，阻塞）。
///
/// `endpoint` 为 Phase 1 预留（可传空串）；`token_pem`/`vk_pem` 为 Ed25519
/// 能力令牌/验证密钥的 PEM 字符串（必须非空）。
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
        let token = CapabilityToken::from_raw(token_pem); // JWT 字符串（from_pem 仅密钥）
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

/// 发布一帧（ACL 检查 + SHM loan + send；阻塞）。
///
/// `meta` 为共享头文件 mediaservo_frame_meta_t（36B 字段袋，R4 逐字段读取）。
/// `payload` 可为 NULL 当且仅当 `len == 0`（纯元数据帧）。
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

/// 订阅一个 topic，创建帧流 handle（阻塞）。
///
/// 成功后 `*out` 指向新 handle（调用方负责 `mediaservo_link_stream_close`）。
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

/// 阻塞取帧：元数据拷入 `out_meta`，载荷拷入 `out_data`（最多 `cap` 字节），
/// `*out_len` = 实际拷贝字节数（帧大于 cap 时截断）。
///
/// 帧到达或关停时返回：关停（stream_close / bus_close）→ MEDIASERVO_LINK_ERR_CLOSED。
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

/// 关闭帧流 handle（幂等；唤醒阻塞中的 recv 使其返回 CLOSED）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_stream_close(st: *mut mediaservo_link_stream_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if st.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(st) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等：已关闭
        }
        handle.shutdown.notify_waiters();
        MEDIASERVO_OK
    }))
    .unwrap_or_else(|_| {
        set_last_error("mediaservo_link_stream_close: panic");
        MEDIASERVO_LINK_ERR_INTERNAL
    })
}

/// 关闭帧总线 handle（幂等；shutdown 全部流，recv 返回 None → CLOSED）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_bus_close(b: *mut mediaservo_link_bus_t) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        if b.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(b) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等：已关闭
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

// ── 通用 ──

/// 最近一次错误的详情（线程安全；无错误时返回空串）。
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

/// 最近错误详情。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_link_last_error(buf: *mut c_char, len: usize) -> c_int {
    catch_unwind(AssertUnwindSafe(|| last_error_impl(buf, len)))
        .unwrap_or(MEDIASERVO_LINK_ERR_INTERNAL)
}

/// 版本信息（MAJOR.MINOR.PATCH — D241 soname 语义）。
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

// ── 共享 C 类型镜像（mediaservo_common.h，R4 字段袋）──

/// C 侧帧元数据（#pragma pack(1) 36B，与 FrameMeta::encode 线格式一致）。
///
/// 生产路径一律经 36B 拷贝 + FrameMeta::decode/encode 逐字段读写；
/// 禁止整块 reinterpret（packed 未对齐 + 字节序风险）。
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
    /// 本文件测试共享进程级 static LAST_ERROR（C ABI 语义在册）——null-handle 负例
    /// 隐式写它，显式读断言并互踩 = flaky（field-c 09-20 同型手术；link-c 09-20 gate
    /// 实抓 last_error_roundtrip 竞跑覆写；deck-c 预防同批）。测试域全文件经此锁串行。
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::*;

    /// 合成一个未连接的信令 handle（会话 None，仅用于 closed/STATE 路径测试）。
    /// 合成一个未连接的信令 handle（会话 None；closed 标志可预设）。
    /// 注意：close 后调用为 UB（契约），测试对同一指针只 close 一次。
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

    /// 合成一个未 attach 的总线 handle（bus None；closed 标志可预设）。
    fn bus_handle(closed: bool) -> *mut mediaservo_link_bus_t {
        Box::into_raw(Box::new(mediaservo_link_bus_t {
            bus: std::sync::Mutex::new(None),
            closed: AtomicBool::new(closed),
        }))
    }

    /// 合成一个未 attach 的总线 handle（bus None）。
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
        // 全局状态跨测试竞争: 先清空再设（不依赖其他测试未写）
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
        // 旧头文件编译的调用方：struct_size 过小 → 明确错误（R3）
        let cfg = mediaservo_link_signal_config_t { struct_size: 1, ..Default::default() };
        let mut out: *mut mediaservo_link_signal_t = ptr::null_mut();
        let rc = mediaservo_link_signal_connect(&cfg, &mut out);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn connect_missing_required_fails() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // struct_size 合法但 url/psk/room 为空 → 必填错误
        let cfg = mediaservo_link_signal_config_t::default();
        let mut out: *mut mediaservo_link_signal_t = ptr::null_mut();
        let rc = mediaservo_link_signal_connect(&cfg, &mut out);
        assert_eq!(rc, MEDIASERVO_LINK_ERR_INVALID_ARG);
        assert!(out.is_null());
    }

    #[test]
    fn connect_bad_role_fails_before_network() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 角色字符串非法 → 连接前即拒绝（不触网）
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
        // 合法 SignalingMessage（frame）但会话缺失 → STATE
        let msg = c"{\"type\":\"frame\",\"room_id\":\"r\",\"codec\":\"h264\",\"sequence\":1,\"is_keyframe\":true,\"data_base64\":\"\"}";
        let rc = mediaservo_link_signal_send(s, msg.as_ptr(), msg.to_bytes().len());
        assert_eq!(rc, MEDIASERVO_LINK_ERR_STATE);
        assert_eq!(mediaservo_link_signal_close(s), MEDIASERVO_OK);
    }

    #[test]
    fn send_after_closed_flag_returns_state() {
        let _ser = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = signal_handle(true);
        // closed 标志已置 → STATE（不触会话）
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
        // R4: repr(C, packed) 镜像结构逐字段填 → 36B 拷贝 + decode →
        // 与 Rust FrameMeta::encode 逐字节比对
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
        // 走真实生产路径：36B 拷贝 + decode（与 meta_from_c 同机制）
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
        // 回写路径（meta_to_c 同机制）也逐字节一致
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

    /// bus 正向链路: attach(验签) → subscribe → publish → recv 断言 meta/payload。
    /// 双节点（ACL 矩阵: Capture 仅发布 camera/*, Processor 仅订阅 camera/*）；
    /// 订阅先于发布（iceoryx2 新订阅者不重放历史帧）。
    /// 复用 mediaservo-link e2e 测试密钥对（D235 测试 fixture）。
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

        // 1. Capture 发布端
        let mut bus: *mut mediaservo_link_bus_t = ptr::null_mut();
        let rc = mediaservo_link_bus_attach(
            endpoint.as_ptr(),
            token_for("ct-capture", Role::Capture).as_ptr(),
            vk_s.as_ptr(),
            &mut bus,
        );
        assert_eq!(rc, MEDIASERVO_OK, "attach capture");
        assert!(!bus.is_null());

        // 2. Processor 订阅端
        let mut proc_bus: *mut mediaservo_link_bus_t = ptr::null_mut();
        let rc = mediaservo_link_bus_attach(
            endpoint.as_ptr(),
            token_for("ct-processor", Role::Processor).as_ptr(),
            vk_s.as_ptr(),
            &mut proc_bus,
        );
        assert_eq!(rc, MEDIASERVO_OK, "attach processor");
        assert!(!proc_bus.is_null());

        // 3. 订阅先于发布（历史帧不重放）
        let mut stream: *mut mediaservo_link_stream_t = ptr::null_mut();
        let rc = mediaservo_link_bus_subscribe(proc_bus, topic.as_ptr(), &mut stream);
        assert_eq!(rc, MEDIASERVO_OK, "subscribe");
        assert!(!stream.is_null());

        // 4. 发布
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

        // 5. 接收并断言 meta/payload
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
        // packed 结构字段读取必须 read_unaligned（E0793）
        unsafe {
            assert_eq!(ptr::addr_of!(out_meta.seq).read_unaligned(), 7);
            assert_eq!(ptr::addr_of!(out_meta.width).read_unaligned(), 640);
            assert_eq!(ptr::addr_of!(out_meta.height).read_unaligned(), 480);
            assert_eq!(ptr::addr_of!(out_meta.format).read_unaligned(), 1);
            assert_eq!(ptr::addr_of!(out_meta.is_keyframe).read_unaligned(), 1);
        }
        assert_eq!(out_len, 64);
        assert_eq!(&buf[..64], &payload);

        // 6. 关闭
        assert_eq!(mediaservo_link_stream_close(stream), MEDIASERVO_OK);
        assert_eq!(mediaservo_link_bus_close(proc_bus), MEDIASERVO_OK);
        assert_eq!(mediaservo_link_bus_close(bus), MEDIASERVO_OK);
    }
}
