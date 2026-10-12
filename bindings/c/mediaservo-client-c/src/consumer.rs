//! 单路视频消费者 C handle（S6 批1b/K4 的 C 面）：每路一个独立帧泵，多路互不
//! 连坐。句柄由 `mediaservo_client_session_consume` 产出，独立 close。
//!
//! 泵纪律：std Mutex 持锁跨 `timeout(CONSUMER_POLL, recv)` 等待——stats/close 与泵
//! 争锁的最坏延迟 = CONSUMER_POLL（250ms，有界）；close 置 closed → 取走 Consumer
//! （receiver 随 drop 消亡，K1 重放判死该路）→ join 泵 → user_free 恰好一次。

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

/// 帧泵空转复查窗（closed / 通道消亡的轮询粒度；兼作 close/stats 最坏锁延迟）。
const CONSUMER_POLL: Duration = Duration::from_millis(250);

/// QOE 快照 C 布局（client.h `mediaservo_client_qoe_snapshot_t` 镜像；
/// 字段序冻结——ABI 只增纪律，新字段尾部追加）。
#[repr(C)]
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
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

/// QOE 回调 C 函数指针。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外——同 qoe_snapshot_t/consumer_t）
pub type mediaservo_client_qoe_cb =
    extern "C" fn(*const mediaservo_client_qoe_snapshot_t, *mut c_void);

/// `*mut c_void` 的 Send/Sync 包装——C 回调契约：**调用方保证 user 上下文的
/// 线程安全**（client.h QOE 节注释；SDK 节拍线程触发 cb）。
#[derive(Clone, Copy)]
struct CbUser(*mut c_void);
// SAFETY: C ABI 契约——user 上下文线程安全由调用方保证（client.h 明示）。
unsafe impl Send for CbUser {}
unsafe impl Sync for CbUser {}

/// 单路视频消费者 opaque handle。
#[allow(non_camel_case_types)] // C ABI 命名（C6 例外）
pub struct mediaservo_client_consumer_t {
    /// 帧回调第一参数透传（**永不解引用**——属主契约由调用方保证会话不早于
    /// consumer 释放；见 client.h 生命周期节）。
    session: *mut mediaservo_client_session_t,
    producer_id: String,
    consumer: Mutex<Option<Consumer>>,
    closed: AtomicBool,
    cb: mediaservo_client_frame_cb,
    user: *mut c_void,
    free: Option<mediaservo_client_user_free>,
    pump: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// QOE 节拍 task 句柄——close 时 abort 释放 trampoline 闭包
    /// （lsan.1096280 实证：进程退出 tokio runtime 不 shutdown → Box 残留 64B/次注册）。
    qoe_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    err: ErrSlot,
}

// SAFETY: consumer/cb/user 的跨线程访问全部经 Mutex/AtomicBool；session 指针只搬运
// 不解引用。
unsafe impl Send for mediaservo_client_consumer_t {}
// SAFETY: 同上。
unsafe impl Sync for mediaservo_client_consumer_t {}

impl HandleErr for mediaservo_client_consumer_t {
    fn err_slot(&self) -> &ErrSlot {
        &self.err
    }
}

/// 由 session_consume 装配：建句柄 + 启动帧泵，返回 raw handle。
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
    // 泵可能已跑完（closed 竞态/通道瞬死）——store 仍成立，join 即收敛。
    if let Ok(mut g) = unsafe { &*raw }.pump.lock() {
        *g = Some(join);
    }
    raw
}

/// 帧泵线程：本路帧通道 → C 回调（帧数据 Vec 存活跨越回调调用点，data 仅回调内有效）。
fn pump_loop(raw: usize) {
    let c = raw as *mut mediaservo_client_consumer_t;
    let h = unsafe { &*c };
    let rt = runtime();
    loop {
        if h.closed.load(Ordering::SeqCst) {
            break;
        }
        // 注意: `timeout(..)` 必须在 async 块内构造（无 context 线程的外侧实参
        //        会 panic——lib.rs video_pump 同训）。
        let frame = match h.consumer.lock() {
            Ok(mut g) => match g.as_mut() {
                Some(cons) => {
                    rt.block_on(async { timeout(CONSUMER_POLL, cons.frames().recv()).await })
                }
                None => break, // close 已取走
            },
            Err(_) => break, // poisoned = 有线程 panic 在临界区，收敛退出
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
            Ok(None) => break,  // 帧通道消亡（会话关闭/该路撤走）——正常收敛
            Err(_) => continue, // CONSUMER_POLL 空转，复查 closed
        }
    }
}

/// 本路 producer id（needed 溢出合同；id 短，cap 256 足够）。
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

/// 本路视频统计 JSON（K4 单路读数，不经会话汇总；键表 = session_video_stats 同形）。
/// needed 溢出合同同 list_rooms。
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

/// QOE 周期回调（consumer-qoe-callback）：Rust on_qoe → C trampoline。
/// cb 在 SDK 节拍 task 触发（tokio worker），C 侧须自行保证线程安全。
/// 重复注册：旧 task abort+join 后才返回——调用方换 ctx 时序安全（PIT-221）。
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
        // C trampoline：QoeSnapshot → C struct（字段序冻结，client.h ABI 注释）。
        // CbUser 携带裸指针过 Send 边界（C 契约：调用方保证 user 线程安全）。
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
        let qoe_task = consumer.on_qoe(
            std::time::Duration::from_millis(u64::from(period_ms)),
            Box::new(trampoline),
        );
        // 幂等替换（PIT-221 同款纪律）：旧 task abort+join（join 在锁外——
        // 同 close 路径死锁面收窄）——返回后旧回调物理不可能再触发，C++ 层
        // delete 旧 qoe_ctx_ 时序安全。新 task 先建后插：join 窗内新 task 可能
        // 已触发一次（用新 ctx——注册前已分配，安全）。
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


/// abort 后等待 task 真退出。C API 同步入口均为非 runtime 线程——
/// block_on 安全；若误从 runtime 线程调用（未来误用），try_current 守卫
/// 降级为只 abort（不 join），契约降级为「尽力而为」并打日志。
fn join_qoe_task(task: tokio::task::JoinHandle<()>) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tracing::warn!("join_qoe_task called on runtime thread — skip join (abort only)");
        return;
    }
    let _ = crate::runtime().block_on(task);
}

/// 关闭本路并释放 handle（幂等）。置 closed → 取走 Consumer（receiver drop → K1 重放
/// 判死本路）→ join 泵（≤CONSUMER_POLL）→ user_free 恰好一次 → free。
/// QOE task join 纪律（PIT-221）：返回后旧回调必不再触发（abort+join 硬保证）；
/// 回调内禁止重入 close / on_qoe（join 虽已移出锁临界区，重入仍属契约违反，
/// 死锁风险自担——契约沿用「回调内禁止阻塞」）。
#[unsafe(no_mangle)]
pub extern "C" fn mediaservo_client_consumer_close(c: *mut mediaservo_client_consumer_t) -> c_int {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if c.is_null() {
            return MEDIASERVO_OK;
        }
        let handle = unsafe { Box::from_raw(c) };
        if handle.closed.swap(true, Ordering::SeqCst) {
            return MEDIASERVO_OK; // 幂等（同 link-c 双 close 纪律）
        }
        // 取走即 drop：帧通道接收端消亡，泵下轮 recv → None → break。
        if let Ok(mut g) = handle.consumer.lock() {
            drop(g.take());
        }
        // QOE 节拍 task abort + join——返回后旧回调物理不可能再触发。
        // （PIT-221: abort 不打断同步执行中的回调——crash_d036 实证 client.hpp:436
        // (*fn)(q) 调已 delete ctx。block_on 用进程级 runtime()——C API 同步入口
        // 均为非 runtime 线程；try_current 守卫见 join_qoe_task。lsan.1096280:
        // 64B/次注册残留——释放语义不变。join 在锁外（take 后 guard 即 drop）——
        // 回调违规重入 close/on_qoe 时只拿 closed 状态错误，不与 join 互锁。）
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
        // user_free 恰好一次钉（合同）：泵空转窗（CONSUMER_POLL）内有界 join。
        // 双 close 不测——close 后句柄内存按契约即 UB（swap 幂等分支是纵深防御，
        // 不是 use-after-free 豁免；link-c 同纪律）。
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
        // Consumer 无公开构造（真 pc 依赖）——用 spawn 注入不可行，改为手工装配
        // （同 crate 私有字段可见）：consumer=None 形 = "通道已撤"泵立即收敛。
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
        assert!(FREES.load(Ordering::SeqCst), "user_free 未调用");
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
        // 节拍 task：tick → 30ms 同步段（模拟「同步执行中的回调」——abort 打不断
        // 的那一段，把竞态窗从微秒拉宽到 ~75% 周期）→ 真实递增 FIRES。
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
        // join 纪律（PIT-221）：close 返回 = task 已终结——在途回调的递增已计入
        // after，此后物理不可能再涨。严格相等（无宽容度——宽容即遮蔽回归）。
        std::thread::sleep(std::time::Duration::from_millis(100));
        let settled = FIRES.load(Ordering::SeqCst);
        assert_eq!(
            settled, after,
            "close 返回后回调仍触发: after={after} settled={settled}"
        );
    }

    #[test]
    fn on_qoe_replace_joins_old_task() {
        // 替换路径复用 join_qoe_task（close 测试已钉行为）——此处钉 helper 对
        // 长 sleep task 的 abort+join 即时收敛，防未来改异步 join 致替换回归。
        let old = crate::runtime().spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        old.abort();
        join_qoe_task(old);
        // join 返回即证 task 终结（abort 生效——1h sleep 不可能自然结束）
    }
}
