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
use std::time::Duration;

use mediaservo_webrtc::stats::RTCStats;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::engine::PcHandle;
use crate::session::{VideoFrame, VideoStreamStats, fold_inbound_stats};

/// 换算完毕的 QOE 快照（consumer-qoe-callback：单位与 TRRO OnMediaState 对齐）。
///
/// `epoch` = 差分基线代——重协商/pc 切换后首轮回 0 值并递增代，消费方仅作
/// 断点识别，**不做业务判断**（plan §5 守门）。
#[derive(Clone, Copy, Debug, Serialize)]
pub struct QoeSnapshot {
    pub fps: u32,
    /// 码率 kbps（SDK 内差分 bytesReceived，四舍五入；首轮占位 0）
    pub kbps: u32,
    /// 丢包率 %（packets_lost / (lost + received) × 100，received=0 时 0）
    pub lost_percent: f32,
    pub decoded_total: u64,
    /// 平均解码耗时 ms/帧（ΔtotalDecodeTime×1000/ΔframesDecoded；首轮/无增量=0）
    pub avg_decode_ms: u32,
    /// 抖动 ms（inbound jitter 秒×1000 取整；多路 max）
    pub jitter_ms: u32,
    /// 往返时延 ms（candidate-pair currentRoundTripTime；后端无源=0）
    pub rtt_ms: u32,
    pub width: u32,
    pub height: u32,
    pub epoch: u32,
}

/// 差分状态机（slot 内独占——plan §2.2）。
///
/// `last_ts == 0` = 基线未建（首轮回 0 占位）；pc 切换（K1 重放）由
/// `ConsumerSlot::set_pc` 调 `bump_epoch` 递增代并重置基线。
#[derive(Debug, Default)]
pub(crate) struct QoeDiffer {
    last_bytes: u64,
    last_ts_ms: u64,
    last_decode_time: f64,
    last_frames_decoded: u64,
    epoch: u32,
}

impl QoeDiffer {
    /// 折叠最新 stats → 快照。`now_ms` 由调用方时钟注入（可测性）。
    pub(crate) fn diff(&mut self, stats: &VideoStreamStats, now_ms: u64) -> QoeSnapshot {
        let mut kbps = 0u32;
        let mut avg_decode_ms = 0u32;
        if self.last_ts_ms > 0 && now_ms > self.last_ts_ms {
            let elapsed_s = (now_ms - self.last_ts_ms) as f64 / 1000.0;
            let delta = stats.bytes_received.saturating_sub(self.last_bytes);
            kbps = ((delta as f64 * 8.0 / elapsed_s) / 1000.0).round() as u32;
            // 解码耗时差分比值（同 host-streamer avg_encode_ms 口径）：
            // ΔtotalDecodeTime×1000 / ΔframesDecoded；重协商清零/无增量 → 0。
            let delta_decoded = stats.frames_decoded.saturating_sub(self.last_frames_decoded);
            if delta_decoded > 0 {
                let delta_decode_s = (stats.total_decode_time - self.last_decode_time).max(0.0);
                avg_decode_ms =
                    (delta_decode_s * 1000.0 / delta_decoded as f64).round() as u32;
            }
        }
        self.last_bytes = stats.bytes_received;
        self.last_ts_ms = now_ms;
        self.last_decode_time = stats.total_decode_time;
        self.last_frames_decoded = stats.frames_decoded;
        let lost = stats.packets_lost;
        let total = lost + stats.packets_received;
        QoeSnapshot {
            fps: stats.frames_per_second.round() as u32,
            kbps,
            lost_percent: if total > 0 { (lost as f64 / total as f64 * 100.0) as f32 } else { 0.0 },
            decoded_total: stats.frames_decoded,
            avg_decode_ms,
            jitter_ms: (stats.jitter * 1000.0).round() as u32,
            rtt_ms: 0, // tick 处并采 connection_stats 填充（保持 diff 签名可测）
            width: stats.frame_width,
            height: stats.frame_height,
            epoch: self.epoch,
        }
    }

    pub(crate) fn bump_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.last_ts_ms = 0; // 下一轮重新占位（断点识别）
        self.last_decode_time = 0.0;
        self.last_frames_decoded = 0;
    }
}

/// 一路 consumer 的共享骨架（会话登记簿持有 Arc，app 句柄亦持有一份）。
///
/// 死活判据 = `frame_tx` 的接收端全部 drop（Consumer drop / 裸 receiver drop
/// 同形）；slot 本体随会话注册簿回收（无单路摘除，与旧 `_pcs` 语义一致）。
pub struct ConsumerSlot {
    producer_id: String,
    frame_tx: mpsc::Sender<VideoFrame>,
    /// 当前 recv pc——重放（K1）成功即整体替换；None = 建立中/已死。
    pc: Mutex<Option<Arc<dyn PcHandle>>>,
    /// QOE 差分状态（consumer-qoe-callback）——pc 切换时 bump epoch。
    pub(crate) qoe_differ: Mutex<QoeDiffer>,
}

impl ConsumerSlot {
    pub(crate) fn new(producer_id: &str, frame_tx: mpsc::Sender<VideoFrame>) -> Self {
        Self {
            producer_id: producer_id.to_string(),
            frame_tx,
            pc: Mutex::new(None),
            qoe_differ: Mutex::new(QoeDiffer::default()),
        }
    }

    pub(crate) fn set_pc(&self, pc: Arc<dyn PcHandle>) {
        *self.pc.lock().unwrap_or_else(|e| e.into_inner()) = Some(pc);
        // 重协商 = 媒体路径断点：差分基线重置 + 代递增（下一轮回 0 占位）。
        self.qoe_differ.lock().unwrap_or_else(|e| e.into_inner()).bump_epoch();
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
    /// 构造时（async consume() 内=runtime 上下文）捕获——on_qoe 可从任意
    /// 非 runtime 线程（C++ worker）安全注册。PIT-216: Handle::current()
    /// 在调用点取会 panic（TPHMI worker 无 runtime）。
    rt: tokio::runtime::Handle,
}

impl Consumer {
    pub(crate) fn new(
        producer_id: String,
        slot: Arc<ConsumerSlot>,
        frames: mpsc::Receiver<VideoFrame>,
    ) -> Self {
        Self {
            producer_id,
            slot,
            frames,
            rt: tokio::runtime::Handle::current(),
        }
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
    /// 不触碰 QOE 差分状态（qoe_snapshot/on_qoe 独占——拉取与推送基线互不污染）。
    #[must_use]
    pub fn stats(&self) -> VideoStreamStats {
        fold_inbound_stats(self.slot.receiver_stats())
    }

    /// 当前 QOE 快照（与 [`Self::on_qoe`] 共享差分状态；拉取即一次 tick）。
    #[must_use]
    pub fn qoe_snapshot(&self) -> QoeSnapshot {
        let stats = fold_inbound_stats(self.slot.receiver_stats());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.slot
            .qoe_differ
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diff(&stats, now)
    }

    /// QOE 周期回调（consumer-qoe-callback：对齐 TRRO OnMediaState 形态）。
    ///
    /// SDK 内置节拍/差分/换算——回调给现成 [`QoeSnapshot`]。幂等：重复注册
    /// 替换前次回调；drop Consumer 或 `close(self)` 即停。回调在 SDK 内部
    /// 节拍 task 触发——**回调内禁止阻塞**（plan §5 守门）。
    pub fn on_qoe(
        &self,
        period: Duration,
        cb: Box<dyn Fn(QoeSnapshot) + Send + Sync>,
    ) -> tokio::task::JoinHandle<()> {
        self.on_qoe_inner(period, cb)
    }

    pub(crate) fn on_qoe_inner(
        &self,
        period: Duration,
        cb: Box<dyn Fn(QoeSnapshot) + Send + Sync>,
    ) -> tokio::task::JoinHandle<()> {
        // 同步 fn——C++/FFI 侧在任意线程调用。Handle 已在 Consumer::new
        // （async consume() 上下文）捕获——此处不再取 current()（PIT-216）。
        let handle = self.rt.clone();
        let slot = Arc::clone(&self.slot);
        handle.spawn(async move {
            let mut tick = tokio::time::interval(period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                // 槽死（app 侧 receiver 全 drop）→ 停节拍
                if slot.is_dead() {
                    break;
                }
                let stats = fold_inbound_stats(slot.receiver_stats());
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let mut snap = slot
                    .qoe_differ
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .diff(&stats, now);
                // rtt 连接级并采（后端无源/无 succeeded pair → None → 0）
                if let Some(conn) = slot
                    .pc
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .and_then(|pc| pc.connection_stats())
                {
                    snap.rtt_ms = conn.rtt_ms.round() as u32;
                }
                cb(snap);
            }
        })
    }

    /// 主动撤走本路（消费 self → 接收端 drop → slot 判死，重放不再续此路）。
    /// pc 随会话回收（与旧 `_pcs` 生命周期语义一致）。
    pub fn close(self) {
        // 无操作体：self 出作用域即 drop frames 接收端 + slot Arc 引用。
    }
}


#[cfg(test)]
mod qoe_tests {
    use super::*;
    use crate::session::VideoStreamStats;
    use mediaservo_webrtc::stats::{RTCInboundRtpStreamStats, RTCStats};

    fn stats_of(bytes: u64, fps: f64, lost: u64, received: u64, decoded: u64) -> VideoStreamStats {
        VideoStreamStats {
            bytes_received: bytes,
            frames_per_second: fps,
            packets_lost: lost,
            packets_received: received,
            frames_decoded: decoded,
            frame_width: 1920,
            frame_height: 1080,
            ..VideoStreamStats::default()
        }
    }

    fn stats_with_decode(
        decoded: u64,
        total_decode_s: f64,
        jitter_s: f64,
    ) -> VideoStreamStats {
        VideoStreamStats {
            frames_decoded: decoded,
            total_decode_time: total_decode_s,
            jitter: jitter_s,
            ..VideoStreamStats::default()
        }
    }

    #[test]
    fn qoe_first_tick_emits_zero_placeholder() {
        // PIT-2026-10-09 钉：基线未建（last_ts=0）首轮回 0 占位——消费端 UI
        // 管道首轮即认识该键（旧拉取式首轮缺席+整除截断 = 码率恒 0 假象根因）。
        let mut d = QoeDiffer::default();
        let snap = d.diff(&stats_of(100_000, 30.0, 0, 1000, 60), 1_000);
        assert_eq!(snap.kbps, 0, "首轮占位 0");
        assert_eq!(snap.fps, 30, "瞬态值首轮即真实");
        assert_eq!(snap.decoded_total, 60);
        assert_eq!(snap.epoch, 0);
    }

    #[test]
    fn qoe_diff_rounds_and_scales() {
        // 300KB/1s = 2400 kbps；丢包率 2%
        let mut d = QoeDiffer::default();
        let _ = d.diff(&stats_of(0, 30.0, 0, 0, 0), 1_000);
        let snap = d.diff(&stats_of(300_000, 30.0, 20, 980, 120), 2_000);
        assert_eq!(snap.kbps, 2400);
        assert!((snap.lost_percent - 2.0).abs() < 1e-4, "lost% = 20/1000");
        assert_eq!(snap.decoded_total, 120);
    }

    #[test]
    fn qoe_epoch_bump_resets_baseline() {
        // K1 重放换 pc → bump_epoch：下一轮回 0 占位（断点识别）
        let mut d = QoeDiffer::default();
        let _ = d.diff(&stats_of(0, 30.0, 0, 0, 0), 1_000);
        let mid = d.diff(&stats_of(300_000, 30.0, 0, 0, 0), 2_000);
        assert_eq!(mid.kbps, 2400);
        d.bump_epoch();
        let after = d.diff(&stats_of(900_000, 30.0, 0, 0, 0), 3_000);
        assert_eq!(after.epoch, 1);
        assert_eq!(after.kbps, 0, "重协商后首轮回 0 占位");
        let next = d.diff(&stats_of(1_200_000, 30.0, 0, 0, 0), 4_000);
        assert_eq!(next.kbps, 2400);
    }

    #[test]
    fn qoe_jitter_passes_through_ms() {
        // qoe-full-metrics: jitter 秒→ms 透传取整（0.0037s → 4ms）
        let mut d = QoeDiffer::default();
        let _ = d.diff(&stats_with_decode(60, 1.0, 0.0037), 1_000);
        let snap = d.diff(&stats_with_decode(120, 2.0, 0.0037), 2_000);
        assert_eq!(snap.jitter_ms, 4, "jitter*1000 四舍五入");
    }

    #[test]
    fn qoe_avg_decode_ratio() {
        // Δdecode=1.0s / Δframes=100 → 10ms/帧；首轮占位 0
        let mut d = QoeDiffer::default();
        let first = d.diff(&stats_with_decode(0, 0.0, 0.0), 1_000);
        assert_eq!(first.avg_decode_ms, 0, "首轮占位");
        let snap = d.diff(&stats_with_decode(100, 1.0, 0.0), 2_000);
        assert_eq!(snap.avg_decode_ms, 10);
        // 无帧增量 → 0（除零防护）
        let flat = d.diff(&stats_with_decode(100, 1.5, 0.0), 3_000);
        assert_eq!(flat.avg_decode_ms, 0);
        // 重协商清零（totalDecodeTime 回退）→ 负差分钳 0
        d.bump_epoch();
        let _ = d.diff(&stats_with_decode(200, 0.5, 0.0), 4_000);
        let after = d.diff(&stats_with_decode(300, 0.7, 0.0), 5_000);
        assert_eq!(after.avg_decode_ms, 2, "(0.7-0.5)s/100f");
    }

    #[test]
    fn qoe_rtt_none_yields_zero() {
        // diff 层 rtt 恒 0（tick 处由 connection_stats 填充）；None → 保持 0
        let mut d = QoeDiffer::default();
        let snap = d.diff(&stats_of(300_000, 30.0, 0, 1000, 60), 2_000);
        assert_eq!(snap.rtt_ms, 0);
    }
}
