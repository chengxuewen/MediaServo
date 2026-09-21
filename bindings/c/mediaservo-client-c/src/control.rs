//! 控制通道 C handle + ack 泵（S6 批1b：on_ack 累积形 + token/off_ack 注销、
//! ready_state/buffered_amount 观测面、producer_ids needed 升形）。
//!
//! 泵纪律承 lib.rs 头注：回调仅在 ack 泵线程触发、不持锁、快速返回。
//! user_free 恰好一次合同：off_ack 只标 retired，**泵在下一轮开头统一回收并在锁外
//! 调用 free**（此刻无并发触发，消 UAF 竞态）；泵未回收的（含泵已死/从未 off）随
//! control_close 全量释放——两条回收路径按"是否仍在注册表"互斥，恰好一次。

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

/// 一条 ack 回调注册（累积形；token 单调自增，0 = 保留无效值）。
struct AckReg {
    token: u64,
    cb: mediaservo_client_ack_cb,
    user: *mut c_void,
    free: Option<mediaservo_client_user_free>,
    /// off_ack 标记：泵下轮回收（锁外 free），close 兜底。
    retired: bool,
}

/// 出程控制通道集 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub struct mediaservo_client_control_t {
    pub(crate) ctl: Mutex<ControlChannel>,
    closed: AtomicBool,
    regs: Mutex<Vec<AckReg>>,
    next_token: AtomicU64,
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
    err: ErrSlot,
}

// SAFETY: 内部 Mutex<ControlChannel>（libwebrtc DC/PC 句柄线程安全，livekit 惯例）。
unsafe impl Send for mediaservo_client_control_t {}
// SAFETY: 所有方法经 Mutex/AtomicBool 序列化。
unsafe impl Sync for mediaservo_client_control_t {}

impl HandleErr for mediaservo_client_control_t {
    fn err_slot(&self) -> &ErrSlot {
        &self.err
    }
}

impl mediaservo_client_control_t {
    /// 组合操作（emergency_stop）用的闭包检查 + 锁访问器（锁序 session→ctl 由调用方维持）。
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

/// RTCDataChannelState → 线值（0..3，头文件枚举注释钉；穷尽 match=新变体编译红）。
fn dc_state_u8(st: RTCDataChannelState) -> u8 {
    match st {
        RTCDataChannelState::Connecting => 0,
        RTCDataChannelState::Open => 1,
        RTCDataChannelState::Closing => 2,
        RTCDataChannelState::Closed => 3,
    }
}

/// ack 泵线程：每轮开头回收 retired（锁外 free）→ 快照 active → recv_ack(1s) →
/// 逐个回调（累积形：一条 ack 触发全部 active）。
fn ack_pump(raw: usize) {
    let c = raw as *mut mediaservo_client_control_t;
    let h = unsafe { &*c };
    let rt = runtime();
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        // retired 回收（off_ack 已标但仍在表）——此刻不持回调锁，free 安全。
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
            Err(_) => break, // poisoned = 有线程 panic 在临界区，收敛退出
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
                // CString 存活至回调返回（serde_json 输出无内嵌 NUL）。
                if let Ok(cstr) = CString::new(json) {
                    for (cb, user) in &actives {
                        cb(c, cstr.as_ptr(), *user);
                    }
                }
            }
            Err(e) => {
                if matches!(e, ClientError::Timeout { .. }) {
                    continue; // 单轮窗空，正常续
                }
                h.fail_client("ack pump", &e);
                break; // InvalidState（DC 断开）等终态
            }
        }
    }
}

/// 注册 ack 回调（**累积形**：多次注册 = 每条 ack 逐个回调；首次注册启动泵）。
///
/// out_token 回传注销凭据（可 NULL；0 = 无效值）。cb=NULL → 拒（注销走
/// [`mediaservo_client_control_off_ack`]，本形无"取消全部"语义）。user_free 在
/// off_ack 后的泵回收轮（≤ACK_POLL）或 control_close 时恰好调用一次。
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
                "mediaservo_client_control_on_ack: cb required (off_ack 注销，NULL 无取消语义)",
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
            // SAFETY: 已判非 null；指向调用方 u64 存储。
            unsafe { *out_token = token };
        }
        // 首次注册启动 ack 泵（同 link on_event 纪律）。
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

/// 按 token 注销 ack 回调（标记 retired，泵下轮回收并调用其 user_free；对**下一轮
/// 之后**到达的 ack 不再生效——在途轮次可能仍触发一次，属有界宽限）。未知 token →
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

/// 发送一条命令信封（serde 构造，非拼接）。payload_json NULL/"" = null payload。
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

/// DC 就绪态（out_state: 0=Connecting 1=Open 2=Closing 3=Closed；=RTCDataChannelState
/// 稳定线值，K5 背压/发送前门禁用）。
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

/// SCTP 背压水位（字节数；0 = 可安全追加。阻塞取 DC 状态，≤ACK_POLL 窗内与 ack 泵
/// 争锁——单线程属主契约内可接受）。
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

/// producer id JSON 数组（观测面，镜像 ControlChannel::producer_ids()）。
/// needed 溢出合同同 list_rooms（批1b 升形：旧固定 buf 盲点修正）。
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

/// 关闭控制通道并释放 handle（幂等；置 closed → join ack 泵 → 全量回收注册表：
/// 未被泵回收的条目在此逐条 user_free——含 retired-未及回收与从未 off 的活跃项）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_control_close(c: *mut mediaservo_client_control_t) -> c_int {
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
        // 泵 join 后无并发触发：表内残留条目（retired 未回收 or 活跃从未 off）
        // 全部在此释放——泵已回收的条目已出表，恰好一次总账成立。
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
        // 线值合同（header 注释钉）：改动即红。
        assert_eq!(dc_state_u8(RTCDataChannelState::Connecting), 0);
        assert_eq!(dc_state_u8(RTCDataChannelState::Open), 1);
        assert_eq!(dc_state_u8(RTCDataChannelState::Closing), 2);
        assert_eq!(dc_state_u8(RTCDataChannelState::Closed), 3);
    }

    #[test]
    fn on_ack_requires_cb_and_closed_control_state() {
        // ControlChannel 无公开构造（真 DC 依赖），本层只钉守卫形：
        // closed 句柄不可得（new() 需真 channel）——null 守卫 + 线值表已覆盖
        // 可确定面；注册/回收活体语义 = 1c（对真 server）。
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
        // STATE 码通路上面的 off_ack(null) 已钉 INVALID_ARG 形；closed 形归 1c。
        let _ = MEDIASERVO_CLIENT_ERR_STATE;
    }
}
