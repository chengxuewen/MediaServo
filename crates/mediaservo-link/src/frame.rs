//! 帧元数据 + 帧引用 + 帧流（latest-slot）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// 帧元数据（定长 LE 编码，D243）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameMeta {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    /// 像素格式（0=未知, 1=I420, 2=NV12, 3=RGBA, 4=JSON 载荷——非像素数据，
    /// 如 E2 streamer 推流状态 stats/* topic）。
    pub format: u8,
    /// 元数据版本（演进用，D243）。encode 恒写 [`Self::WIRE_VERSION`]；
    /// decode 对非当前版本**拒帧**（N4：静默错解 36 字节 = 比丢帧更糟）。
    pub version: u8,
    pub is_keyframe: bool,
    pub ts_mono_ns: u64,
    pub ts_epoch_ns: u64,
}

impl FrameMeta {
    /// JSON 载荷格式标记（E2 stats topic 线格式；非像素数据）。
    pub const FORMAT_JSON: u8 = 4;

    /// 当前线格式版本。0 = 历史恒零值（全量部署字节形即 v0）——**上线零断裂**；
    /// 未来演进 bump 此常数并在 decode 加兼容窗。
    pub const WIRE_VERSION: u8 = 0;

    /// 定长编码字节数：
    /// seq(8) + width(4) + height(4) + format(1) + version(1) + keyframe(1) + reserved(1)
    /// + ts_mono_ns(8) + ts_epoch_ns(8) = 36。
    pub const WIRE_LEN: usize = 36;

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut b = [0u8; Self::WIRE_LEN];
        b[0..8].copy_from_slice(&self.seq.to_le_bytes());
        b[8..12].copy_from_slice(&self.width.to_le_bytes());
        b[12..16].copy_from_slice(&self.height.to_le_bytes());
        b[16] = self.format;
        b[17] = self.version;
        b[18] = u8::from(self.is_keyframe);
        b[19] = 0; // reserved
        b[20..28].copy_from_slice(&self.ts_mono_ns.to_le_bytes());
        b[28..36].copy_from_slice(&self.ts_epoch_ns.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self, crate::LinkError> {
        if b.len() < Self::WIRE_LEN {
            return Err(crate::LinkError::Bus(format!(
                "frame meta too short: {} < {}",
                b.len(),
                Self::WIRE_LEN
            )));
        }
        // N4 版本门：未知版本拒帧（解析后字段全错位 = 静默花屏比丢帧更坏）。
        if b[17] != Self::WIRE_VERSION {
            return Err(crate::LinkError::Bus(format!(
                "frame meta wire version {} unsupported (expected {})",
                b[17],
                Self::WIRE_VERSION
            )));
        }
        Ok(Self {
            seq: u64::from_le_bytes(b[0..8].try_into().expect("8 bytes")),
            width: u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")),
            height: u32::from_le_bytes(b[12..16].try_into().expect("4 bytes")),
            format: b[16],
            version: b[17],
            is_keyframe: b[18] != 0,
            ts_mono_ns: u64::from_le_bytes(b[20..28].try_into().expect("8 bytes")),
            ts_epoch_ns: u64::from_le_bytes(b[28..36].try_into().expect("8 bytes")),
        })
    }
}

/// 帧引用（元数据 + payload）。Phase 2 可演进为 iceoryx2 SHM 零拷贝视图。
pub struct FrameRef {
    meta: FrameMeta,
    payload: Vec<u8>,
}

impl FrameRef {
    pub fn new(meta: FrameMeta, payload: Vec<u8>) -> Self {
        Self { meta, payload }
    }
    pub fn meta(&self) -> &FrameMeta {
        &self.meta
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// 帧流内部状态（后台线程经 [`Weak`] 投递，流被丢弃时自动停线程）。
pub(crate) struct StreamInner {
    slot: Mutex<Option<FrameRef>>,
    notify: tokio::sync::Notify,
    shutdown: AtomicBool,
}

impl StreamInner {
    /// 投递最新帧（替换旧帧）。
    pub(crate) fn deliver(&self, frame: FrameRef) {
        *self.slot.lock().expect("frame slot lock") = Some(frame);
        self.notify.notify_waiters();
    }

    /// 关停（唤醒等待中的 recv 返回 None）。
    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

/// 帧流（latest-slot：慢消费者跳到最新帧，审核 H5）。
///
/// FrameBus 后台线程每收到一帧经 [`StreamInner::deliver`] 替换槽内帧并通知；
/// 消费者 [`Self::recv`] 等待通知后取最新帧。**禁用无界队列**（会重新引入积压）。
#[derive(Clone)]
pub struct FrameStream {
    inner: Arc<StreamInner>,
}

impl std::fmt::Debug for FrameStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameStream").finish()
    }
}

impl FrameStream {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(StreamInner {
                slot: Mutex::new(None),
                notify: tokio::sync::Notify::new(),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// 供后台线程获取弱引用（流被丢弃时线程自动退出）。
    pub(crate) fn weak_inner(&self) -> Weak<StreamInner> {
        Arc::downgrade(&self.inner)
    }

    /// 内部共享状态（FrameBus 用于 close 时 shutdown）。
    pub(crate) fn inner(&self) -> Arc<StreamInner> {
        Arc::clone(&self.inner)
    }

    /// 取最新帧；无帧时等待。慢消费者自动跳到最新（不积压）；关停后返回 None。
    pub async fn recv(&self) -> Option<FrameRef> {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(f) = self.inner.slot.lock().expect("frame slot lock").take() {
                return Some(f);
            }
            if self.inner.shutdown.load(Ordering::SeqCst) {
                return None; // 已关停且无帧
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u64) -> FrameRef {
        FrameRef::new(
            FrameMeta { seq, ..Default::default() },
            vec![seq as u8],
        )
    }

    #[tokio::test]
    async fn latest_slot_skips_to_newest() {
        let s = FrameStream::new();
        s.inner.deliver(frame(1));
        s.inner.deliver(frame(2));
        s.inner.deliver(frame(3));
        let f = s.recv().await.expect("frame");
        assert_eq!(f.meta().seq, 3, "慢消费者应跳到最新帧");
    }

    #[tokio::test]
    async fn recv_waits_for_delivery() {
        let s = Arc::new(FrameStream::new());
        let s2 = Arc::clone(&s);
        let h = tokio::spawn(async move {
            let f = s2.recv().await.expect("frame");
            f.meta().seq
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        s.inner.deliver(frame(42));
        assert_eq!(h.await.expect("join"), 42);
    }

    #[tokio::test]
    async fn recv_returns_none_after_shutdown() {
        let s = FrameStream::new();
        s.inner().shutdown();
        assert!(s.recv().await.is_none(), "关停后 recv 应返回 None");
    }

    #[test]
    fn decode_rejects_unknown_wire_version() {
        // N4：版本位非 0 = 拒帧且报版本（36 字节全错位比丢帧更坏）
        let mut b = FrameMeta { seq: 7, width: 4, ..Default::default() }.encode();
        assert_eq!(FrameMeta::decode(&b).unwrap().version, FrameMeta::WIRE_VERSION);
        b[17] = 9;
        let e = FrameMeta::decode(&b).expect_err("v9 must be rejected");
        assert!(e.to_string().contains("version 9"), "{e}");
    }
}