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

/// 关闭本路并释放 handle（幂等）。置 closed → 取走 Consumer（receiver drop → K1 重放
/// 判死本路）→ join 泵（≤CONSUMER_POLL）→ user_free 恰好一次 → free。
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
    fn state_line_values_pinned() {
        // 线值合同冗余钉（lib.rs state_u8 为真源，header 注释消费）——防漂移。
        assert_eq!(MEDIASERVO_CLIENT_ERR_STATE, -9);
    }
}
