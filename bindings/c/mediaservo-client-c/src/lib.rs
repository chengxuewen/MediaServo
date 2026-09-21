//! MediaServo client C ABI — 舱端/消费侧 SDK（登录 + SFU 视频消费 + 控制 DataChannel）。
//!
//! 契约 §7（D109/D240/D241）：opaque handle + int 错误码 + 回调。
//! 模式基准 = bindings/c/mediaservo-link-c（生命周期/struct_size/pump 纪律同形）。
//! Rust 侧全部能力来自 `mediaservo-client` v2（本 crate 只做 FFI 形，零业务逻辑）。
//!
//! S6 批1b（R1 改名 + K3 句柄错误 + 多路 Consumer + 状态观测）：
//! - 全量前缀 `mediaservo_client_*`（零别名，旧 `ms_client_*` 退役）；
//! - 错误双轨 = 句柄槽（`session_error`/`session_last_error`，机读 code/wire/retryable）
//!   + 进程全局 ⊘ 双写一周期（K3 裁决：最小改动形）；
//! - `session_consume` 多路形（每 consumer 一帧泵，互不连坐）；旧 `consume_video` ⊘
//!   行为重映射（wait(30s)+consume+into_receiver，一次性闸保持）；
//! - `session_state`/`session_on_state`（累积注册，ConnectionState 0..3 线值）；
//! - `control_on_ack` 累积形 + token（`control_off_ack` 注销），旧替换形退役。
//!
//! # 生命周期契约（承 link-c R2）
//! - handle 单线程属主；close 后任何 API 调用为 UB（幂等 close 同 link 纪律）。
//! - session_close = 置 closed 标志 → 关会话（帧通道随之消亡）→ join 视频泵 →
//!   join 状态泵 → 逐条回调 user_free → 才 free。
//! - consumer handle 独立关闭（close 撤本路接收端，泵 join 有界 ≤CONSUMER_POLL）；
//!   session_close 不隐式关 consumer（与 control 同纪律）。
//! - 控制 handle 独立关闭；ack 泵每轮 recv_ack 有界（1s），closed 标志 ≤1s 内生效。
//! - 帧/ack/状态回调仅在各自泵线程触发；回调调用期间不持任何锁；回调内禁止调用
//!   任何 mediaservo_client_* API（含 close）——未定义行为。
//! - 回调内 JSON 字符串与 frame.data 指针仅在回调内有效（需保留请拷贝）。
//! - user_free 契约：每条注册（on_state/consume/on_ack）携带可选 user_free，
//!   **恰好调用一次**——on_state 在 session_close；consume 在 consumer_close；
//!   on_ack 在 off_ack 后的泵回收轮或 control_close（未 off_ack 的随 close 释放）。
//!
//! # runtime（审核 R1 的 client 形态）
//! `RoomSession::open_control` 内部 `tokio::spawn(ack_consumer_pump)`（session.rs），
//! link `SignalClient::connect` spawn WS 读循环——后台任务必须存活于任何单次
//! block_on 之外，故进程级共享 multi_thread runtime（OnceLock，C28 模式）。与
//! link-c 的 per-handle runtime 不同：client 后台任务与会话句柄跨调用共生，
//! 共享实例天然覆盖且省线程。
//!
//! # 错误映射
//! `ClientError` 全 14 变体 → `MEDIASERVO_CLIENT_ERR_*` 穷尽 match（单测钉，
//! 新变体入 enum 即编译红，静默漏映射不可能）。机读位 wire_code/retryable 经
//! [`mediaservo_client_error_t`] 透传（K2 Rust 面的 C 镜像）。
//!
//! C ABI 面 raw pointer 参数是签名刚需：全部解引用点先 null 校验、包
//! catch_unwind，头文件载明属主/生命周期契约——clippy 的 safe-API 误用模型
//! 不适用于本 crate（link-c/field-c 同族问题系存量债，工作区 lint 红在 HEAD
//! 已登记另案；本 crate 以显式 allow 保持自身零告警）。
#![allow(clippy::not_unsafe_ptr_arg_deref)]

mod config;
mod consumer;
mod control;
mod errors;

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

/// 等房间内首路视频 producer 上限（basic.rs 同值：late-join NewProducer 回放窗）。
const VIDEO_PRODUCER_WAIT: Duration = Duration::from_secs(30);
/// ack 泵单轮 recv_ack 窗（closed 标志生效上界 = 本值）。
pub(crate) const ACK_POLL: Duration = Duration::from_secs(1);
/// ⊘ 视频泵空转复查窗（closed / 通道消亡的轮询粒度）。
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

/// 带句柄 FFI 的统一 catch 尾：panic → 句柄错误槽（句柄非 null 时）+ 全局双写。
///
/// close 族不用本形（Box::from_raw 后句柄所有权存疑，panic 兜底仅写全局）。
fn ffi_catch<T: HandleErr + ?Sized>(
    name: &'static str,
    h: *const T,
    body: impl FnOnce() -> c_int,
) -> c_int {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
        // SAFETY: 仅在指针非 null 时解引用；panic 不释放句柄（close 族除外，
        // 其 panic 尾走 ffi_global）。锁中毒由 note 的 try-lock 消化。
        match unsafe { h.as_ref() } {
            Some(x) => x.fail_panic(name),
            None => set_last_error(format!("{name}: panic")),
        }
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

/// 无句柄自由函数的 catch 尾（仅全局 last_error）。
fn ffi_global(name: &'static str, body: impl FnOnce() -> c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
        set_last_error(format!("{name}: panic"));
        MEDIASERVO_CLIENT_ERR_INTERNAL
    })
}

// ── 回调类型 + 结构镜像 ──

/// 视频帧回调（⊘ consume_video 与多路 consume 共用；各自泵线程触发，
/// frame 指针仅回调内有效；第一参数 = 所属会话句柄）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub type mediaservo_client_frame_cb =
    extern "C" fn(*mut mediaservo_client_session_t, *const mediaservo_client_frame_t, *mut c_void);

/// ack 回调（仅在 ack 泵线程触发；ack_json 仅回调内有效）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub type mediaservo_client_ack_cb =
    extern "C" fn(*mut mediaservo_client_control_t, *const c_char, *mut c_void);

/// 会话连接态回调（仅在状态泵线程触发；state 线值见 header 枚举注释）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub type mediaservo_client_state_cb =
    extern "C" fn(*mut mediaservo_client_session_t, u8 /*state*/, *mut c_void);

/// user 指针释放器（NULL = 不释放；每条注册恰好调用一次，时机见模块头）。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub type mediaservo_client_user_free = extern "C" fn(*mut c_void);

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

// ── 自由函数 ──

/// SDK 版本 (MAJOR.MINOR.PATCH — D241 soname 语义)。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_version(buf: *mut c_char, len: usize) -> c_int {
    ffi_global("mediaservo_client_version", || {
        if buf.is_null() || len == 0 {
            return MEDIASERVO_CLIENT_ERR_INVALID_ARG;
        }
        copy_out_str(env!("CARGO_PKG_VERSION"), buf, len)
    })
}

/// 账号登录换 JWT（阻塞；扁平参数形，批1b 签名升形——旧 config 结构形退役）。
///
/// 出参 = jwt 文本（NUL 结尾）。needed 溢出合同同 [`mediaservo_client_list_rooms`]
/// （先写 needed 两态；jwt 常规 <2KiB，建议 cap ≥ 4096）。
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

/// 房间发现（`GET {http_base}/api/rooms`，阻塞；**会话前自由函数**——不依赖任何
/// handle，F-T-8）。out_json = JSON 数组 `[{"room_id":..,"kind":..}]`（服务端 wire
/// 原样透传，本层不解析内容）。溢出合同（producer_ids cap 盲点的修正形）：cap 不足时
/// `*needed` 写入必需字节数（含 NUL）并返回 ERR_INVALID_ARG；成功时 `*needed` = 实际
/// 长度。`needed` 可 NULL（不需要反馈）。非 2xx → ERR_UNAUTHORIZED（详情 last_error）。
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

/// 错误码 → 静态文案（表源 = ClientError Display 模板 + header 错误码注释，
/// 无新文案发明；未知码固定兜底串）。拷贝语义 = 截断不报错。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_strerror(code: c_int, buf: *mut c_char, cap: usize) -> c_int {
    ffi_global("mediaservo_client_strerror", || copy_out_str(strerror_text(code), buf, cap))
}

/// 最近错误详情（⊘ 进程全局形，保留一周期；K3 后新代码用句柄级
/// [`mediaservo_client_session_last_error`] / [`mediaservo_client_error_t`]）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_last_error(buf: *mut c_char, len: usize) -> c_int {
    ffi_global("mediaservo_client_last_error", || last_error_impl(buf, len))
}

// ── 会话 ──

/// 一条状态回调注册（累积形；user_free 随 session_close 恰好调用一次）。
struct StateReg {
    cb: mediaservo_client_state_cb,
    user: *mut c_void,
    free: Option<mediaservo_client_user_free>,
}

/// 房间会话 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub struct mediaservo_client_session_t {
    /// close 时 take 出并消费（RoomSession::close(self) 签名形态）。
    session: std::sync::Mutex<Option<RoomSession>>,
    /// connect 时快照（negotiated 会话期恒定）。
    negotiated: u32,
    closed: AtomicBool,
    /// ⊘ consume_video 一次性闸门（首调用置位；失败路径回滚）。
    video_started: AtomicBool,
    /// ⊘ 帧回调（泵线程每轮读取；user 指针存于句柄内不跨线程搬运——link-c 同形）。
    video_cb: std::sync::Mutex<Option<(mediaservo_client_frame_cb, *mut c_void)>>,
    video_pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 状态回调注册表（累积；注销 API 不设——与 on_ack 的 token 形区分命名）。
    state_regs: std::sync::Mutex<Vec<StateReg>>,
    state_pump: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// K3 句柄错误槽。
    err: ErrSlot,
}

// SAFETY: 内部 Mutex<Option<RoomSession>> 序列化访问；泵线程仅经该锁与原子标志读取。
unsafe impl Send for mediaservo_client_session_t {}
// SAFETY: 所有字段访问经 Mutex/AtomicBool。
unsafe impl Sync for mediaservo_client_session_t {}

impl HandleErr for mediaservo_client_session_t {
    fn err_slot(&self) -> &ErrSlot {
        &self.err
    }
}

/// ConnectionState → 线值（0..3，头文件 enum 注释钉；新 Rust 变体编译红=穷尽 match）。
fn state_u8(st: ConnectionState) -> u8 {
    match st {
        ConnectionState::Disconnected => 0,
        ConnectionState::Connected => 1,
        ConnectionState::Reconnecting => 2,
        ConnectionState::Failed => 3,
    }
}

/// 状态泵线程：watch 变化 → 全部注册回调（每轮重读注册表——注册即生效）。
fn state_pump_loop(raw: usize) {
    let s = raw as *mut mediaservo_client_session_t;
    let h = unsafe { &*s };
    let rt = runtime();
    // 观测源 = 会话 watch 订阅（会话缺失/poisoned = 无源，泵直接退出；
    // 注册表保留，close 时统一 user_free）。
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
            Err(_) => break, // RoomSession drop → watch sender 消亡（close 正常收敛）
        }
    }
}

/// ⊘ 视频泵线程：帧通道 → C 回调（首路专用；多路见 consumer 模块）。
fn video_pump(s: *mut mediaservo_client_session_t, mut rx: mpsc::Receiver<VideoFrame>) {
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
            Ok(None) => break,  // 会话 drop → 帧通道消亡（正常收敛）
            Err(_) => continue, // VIDEO_POLL 空转，复查 closed
        }
    }
}

/// 信令连接 + 入房（阻塞）。成功 `*out` = 新 handle（调用方 close）。
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
            // W4b：急停密钥文件通道（G13=密钥永不 argv/env 明文）。路径无效**不拦建会话**
            // ——steer/consume 等非安全面照常工作，estop 调用点才报 INVALID_ARG
            // （建会话期硬拦会让一个坏路径拖垮整个 SDK 会话=可用性反噬；车端无 key
            // 时本字段本就无关）。
            hmac_key: parts.hmac_key_file.and_then(|p| {
                config::load_hmac_key_file(p)
                    .map_err(|e| set_last_error(format!("mediaservo_client_session_create: {e}")))
                    .ok()
            }),
        };
        match runtime().block_on(RoomSession::connect(&client_cfg)) {
            Ok(session) => {
                let negotiated = session.negotiated();
                let handle = Box::new(mediaservo_client_session_t {
                    session: std::sync::Mutex::new(Some(session)),
                    negotiated,
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

/// 谈成的方言版本（S0；旧 server = 1）。
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

/// 会话连接态快照（out_state: 0=Disconnected 1=Connected 2=Reconnecting 3=Failed）。
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

/// 注册会话连接态回调（**累积形**：多次注册 = 状态每次变化逐个回调全部注册项；
/// 不可注销，随 session_close 统一释放并逐条 user_free）。
///
/// 泵在首次注册且会话存活时启动；只报**变化**（注册后的初始态用
/// [`mediaservo_client_session_state`] 自取）。回调内禁止调用本 SDK API（C 契约）。
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
                "mediaservo_client_session_on_state: cb required (off 不提供，随 close 释放)",
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
        // 首次注册且会话存活 → 启动状态泵（同 link on_event 纪律；会话已关则
        // 注册保留但无观测源，close 时照常 user_free）。
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

/// 纯等待房间内的视频 producer（无副作用——不 consume；多路编排的定向入口）。
///
/// 成功 out_pid = producer id（NUL 结尾），needed 溢出合同同 list_rooms。
/// 超时 → ERR_TIMEOUT；断链/事件流关闭 → ERR_STATE。
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

/// 订阅一路视频 producer（多路新形，K4）：每路一个独立帧泵 + consumer 句柄，
/// 互不连坐。建立序列 = wait 由调用方显式化（先 [`mediaservo_client_session_wait_video`]
/// 或已知 id），本调用直达 consume。
///
/// `**out_consumer` = 新 handle（调用方 `mediaservo_client_consumer_close`，
/// session_close 不隐式关）。user_free 在 consumer_close 时恰好调用一次。
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

/// ⊘ 消费房间第一路视频（首路形，保留一周期）：内部 = wait(≤30s)+consume+
/// into_receiver 重映射，行为与旧形逐字节一致（一次性闸：二次调用 → ERR_STATE）。
/// 新代码用 wait_video + consume 多路形。
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
        // 建立期持会话锁（单线程属主契约内）；泵线程不再触碰会话。
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
            // 行为重映射（合同 §2）：consume+into_receiver ≡ 旧 consume_video
            // （Rust 面 consume_video 即该组合的薄桥，逐字节等价）。
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
        // 裸指针非 Send：句柄经 usize 传递（值语义，仅地址搬运），回调存于句柄内
        // 每轮读取 —— 同 link-c 泵纪律。
        let raw = s as usize;
        let join =
            std::thread::spawn(move || video_pump(raw as *mut mediaservo_client_session_t, frames));
        if let Ok(mut g) = h.video_pump.lock() {
            *g = Some(join);
        }
        MEDIASERVO_OK
    })
}

/// ⊘ 视频统计汇总 JSON（会话级 union，W3 mini-stats 源；与旧 `_pcs` 折叠语义一致，
/// 1a 已对表）。对象形 `{"bytes_received":..,"packets_received":..,"packets_lost":..,
/// "frames_decoded":..,"frame_width":..,"frame_height":..,"frames_per_second":..}`。
/// needed 溢出合同同 list_rooms。多路精确读数用 consumer_stats。
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

/// K3 句柄级机读错误（本句柄最近一次失败调用；无错 = code 0）。
///
/// 覆盖状态/传输/映射类失败；缓冲溢出（needed 合同）只走返回码+全局 last_error。
/// out.struct_size 须 ≥ sizeof(mediaservo_client_error_t)（R3 演进纪律）。
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

/// K3 句柄级错误文本（本句柄最近一次失败调用的详情；无错 = 空串）。
/// 拷贝语义 = 截断不报错（同 last_error ⊘）。
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

/// 急停双路投递（W4b·S4 语义的 C 面化）：DC 快路径（会话建会话时经
/// `hmac_key_file` 加载的密钥签名；未配置=未签形）+ 信令审计副本。
/// 前置：mediaservo_client_open_control 已成功（ctl 句柄来自该调用）。payload_json
/// NULL/空 = Null 载荷。返回 OK 仅表示**投递**成功（车端裁决看 ack——
/// recv_ack 收 seq 回执），非"已执行"。
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
        // 锁序 session→ctl（与所有组合操作一致；无逆序路径=无死锁面）。
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

/// 开出程控制通道集（每会话一次性——ack 泵每会话一条，Rust 侧闸门）。
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

/// 关闭会话并释放 handle（幂等）。顺序：closed 标志 → 关会话（帧通道随之消亡唤醒
/// ⊘ 泵；watch sender 消亡唤醒状态泵）→ join 视频泵 → join 状态泵 → 逐条
/// user_free → free。泵正在回调中时 join 阻塞——回调必须快速返回。
/// consumer/control 句柄不受本调用回收（独立 close 纪律）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_session_close(s: *mut mediaservo_client_session_t) -> c_int {
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
            set_last_error(format!("mediaservo_client_session_close: {e}"));
        }
        if let Some(join) = handle.video_pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join();
        }
        if let Some(join) = handle.state_pump.lock().ok().and_then(|mut g| g.take()) {
            let _ = join.join();
        }
        // 状态回调回收：user_free 每条恰好一次（泵已 join，无并发触发）。
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

    // ── FFI 入口 null 守卫（不触网）──
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
        // 空串按缺失处理（旧 validate_login_cfg 语义保持）。
        let rc = mediaservo_client_login(
            c"".as_ptr(),
            c"u".as_ptr(),
            c"p".as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            &mut need,
        );
        assert_eq!(rc, MEDIASERVO_CLIENT_ERR_INVALID_ARG);
        // null out_jwt / cap 0 —— 本地拒（不出网）。
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
        // null http_base / null jwt / null out / cap 0——全部本地拒（不出网，
        // 断言点先于 block_on：若出网则是 Io/Timeout 而非 INVALID_ARG）。
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
        // 非 null 但 cap 0：句柄不合法也不许触网/解引用——先参数守卫。
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
        // label 空指针（session 侧不合法同样先参数守卫）。
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
        // labels 非空但 len=0 → 拒
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

    // ── 批1b: K3 句柄错误槽（session=None 的确定失败形，免触网）──

    /// 测试专用：无会话的裸句柄（session=None = "已关"形——状态/统计类调用
    /// 确定失败，走句柄槽路径，免真 server）。
    fn test_session_handle() -> *mut mediaservo_client_session_t {
        Box::into_raw(Box::new(mediaservo_client_session_t {
            session: std::sync::Mutex::new(None),
            negotiated: 3,
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
        // 确定失败：session=None → STATE（句柄槽 + 全局双写）。
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
        assert_eq!(e.wire_code, 0); // 本地状态错无 wire 码
        assert_eq!(e.retryable, 0);
        // 句柄级文本非空（含失败上下文）。
        let mut buf = [0u8; 128];
        assert_eq!(
            mediaservo_client_session_last_error(s, buf.as_mut_ptr() as *mut c_char, buf.len()),
            MEDIASERVO_OK
        );
        let msg =
            unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char) }.to_str().unwrap();
        assert!(msg.contains("session closed"), "handle msg: {msg}");
        // ⊘ 全局兜底双写（K3 一周期合同）。
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
        // 累积注册：三条（两条带 free，一条 NULL free）。session=None → 泵不启动，
        // 注册仍成功（语义=登记待 close 回收）；活体转换面 = 1c。
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
        // cb=NULL → 拒（累积形无"取消注册"语义）。
        assert_eq!(
            mediaservo_client_session_on_state(s, None, ptr::null_mut(), None),
            MEDIASERVO_CLIENT_ERR_INVALID_ARG
        );
        let before = FREES.load(Ordering::SeqCst);
        assert_eq!(mediaservo_client_session_close(s), MEDIASERVO_OK);
        // user_free 恰好每条一次（两条带 free 的注册）。
        assert_eq!(FREES.load(Ordering::SeqCst), before + 2);
    }

    #[test]
    fn consume_video_second_call_hits_gate() {
        // ⊘ 一次性闸钉：video_started 已置位 → STATE（闸检查先于会话访问，
        // session=None 句柄即可测）。
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
        // null handle → 全局 INVALID_ARG（不出网）。
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
        // 句柄在但会话已关 → STATE（先于任何等待）。
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
        // closed 句柄不 free（close 语义由其他用例覆盖），此处泄漏仅测试进程内。
    }

    #[test]
    fn session_consume_guards() {
        let s = test_session_handle();
        let mut out: *mut mediaservo_client_consumer_t = ptr::null_mut();
        let pid = c"p1";
        // cb 必填（累积多路形无"取消"语义）。
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
        // producer_id 必填。
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
        // out 必填。
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
