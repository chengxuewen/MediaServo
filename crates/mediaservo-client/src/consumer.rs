//! S6/K4：Consumer 对象模型——每路订阅一个句柄，多路互不连坐。
//!
//! 形状对账（frozen design）：
//! - [`ConsumerSlot`] = 会话侧登记簿条目（帧出口 + 当前 recv pc），K1 重挂重放
//!   换 pc 不换 slot——app 手里的 [`Consumer::frames`] 接收端跨重连连续；
//! - [`Consumer`] = app 侧句柄（**无 Drop impl**）：`close(self)` 消费自身即
//!   撤走接收端，注册簿槽经 `frame_tx.is_closed()` 判死（重放跳过该路）。
//!
//! 旧形 `consume_video` 保留为桥（返回裸 receiver），行为逐字节不变（R3）。

use std::sync::{Arc, Mutex};

use mediaservo_webrtc::stats::RTCStats;
use tokio::sync::mpsc;

use crate::engine::PcHandle;
use crate::session::{VideoFrame, VideoStreamStats, fold_inbound_stats};

/// 一路 consumer 的共享骨架（会话登记簿持有 Arc，app 句柄亦持有一份）。
///
/// 死活判据 = `frame_tx` 的接收端全部 drop（Consumer drop / 裸 receiver drop
/// 同形）；slot 本体随会话注册簿回收（无单路摘除，与旧 `_pcs` 语义一致）。
pub struct ConsumerSlot {
    producer_id: String,
    frame_tx: mpsc::Sender<VideoFrame>,
    /// 当前 recv pc——重放（K1）成功即整体替换；None = 建立中/已死。
    pc: Mutex<Option<Arc<dyn PcHandle>>>,
}

impl ConsumerSlot {
    pub(crate) fn new(producer_id: &str, frame_tx: mpsc::Sender<VideoFrame>) -> Self {
        Self { producer_id: producer_id.to_string(), frame_tx, pc: Mutex::new(None) }
    }

    pub(crate) fn set_pc(&self, pc: Arc<dyn PcHandle>) {
        *self.pc.lock().unwrap_or_else(|e| e.into_inner()) = Some(pc);
    }

    #[must_use]
    pub(crate) fn producer_id(&self) -> &str {
        &self.producer_id
    }

    /// 接收端已全 drop = 该路 app 侧已撤走（K1 重放跳过判据）。
    #[must_use]
    pub(crate) fn is_dead(&self) -> bool {
        self.frame_tx.is_closed()
    }

    /// 帧出口克隆（重放新 pc 的 on_track sink 供料；槽持有原件 = 存续锚）。
    pub(crate) fn frame_tx(&self) -> mpsc::Sender<VideoFrame> {
        self.frame_tx.clone()
    }

    /// 当前 pc 的 "video" inbound-rtp 读数（无 pc = 空表）。
    pub(crate) fn receiver_stats(&self) -> Vec<RTCStats> {
        let guard = self.pc.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(pc) => pc.receiver_stats("video"),
            None => Vec::new(),
        }
    }
}

/// app 侧单路视频消费句柄（[`crate::RoomSession::consume`] 的输出）。
///
/// 帧流与 stats 同源于注册簿 slot：重连重放后帧从**同一个** receiver 续流
/// （app 无感），stats 自动切到新 pc。无 Drop impl——撤走即 `close(self)`
/// 或直接 drop 句柄/裸 receiver（语义同：接收端 closed → 重放判死）。
pub struct Consumer {
    producer_id: String,
    slot: Arc<ConsumerSlot>,
    frames: mpsc::Receiver<VideoFrame>,
}

impl Consumer {
    pub(crate) fn new(
        producer_id: String,
        slot: Arc<ConsumerSlot>,
        frames: mpsc::Receiver<VideoFrame>,
    ) -> Self {
        Self { producer_id, slot, frames }
    }

    /// 被消费的 producer id。
    #[must_use]
    pub fn id(&self) -> &str {
        &self.producer_id
    }

    /// 帧接收端借用（latest 语义，容量 3——满丢由泵侧 try_send 承载）。
    pub fn frames(&mut self) -> &mut mpsc::Receiver<VideoFrame> {
        &mut self.frames
    }

    /// 移交裸 receiver（旧 `consume_video` 返回形的等价物）。
    ///
    /// 重放 deregistration 语义：本方法消费句柄并交出接收端——注册簿槽的
    /// 死活判据是 `frame_tx` **接收端 closed**，即交出的 receiver 被 drop 之刻
    /// 该路才判死（交出本身不脱挂；持有 receiver = 保持重放资格）。
    #[must_use]
    pub fn into_receiver(self) -> mpsc::Receiver<VideoFrame> {
        self.frames
    }

    /// 本路视频统计（K4 增益：单路读数，不经会话汇总）。
    #[must_use]
    pub fn stats(&self) -> VideoStreamStats {
        fold_inbound_stats(self.slot.receiver_stats())
    }

    /// 主动撤走本路（消费 self → 接收端 drop → slot 判死，重放不再续此路）。
    /// pc 随会话回收（与旧 `_pcs` 生命周期语义一致）。
    pub fn close(self) {
        // 无操作体：self 出作用域即 drop frames 接收端 + slot Arc 引用。
    }
}
