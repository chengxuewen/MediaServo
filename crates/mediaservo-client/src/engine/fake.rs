//! FakeEngine——有状态 libwebrtc 语义模拟器（feature `engine-fake` 门控，默认 off）。
//!
//! 存在理由（K11 / mediasoup-client FakeHandler 同型）：把「引擎行为」层假面化，
//! 让 consume/control/ack 逻辑与故障分支进 CI 秒级确定性执行——真 webrtc-sys
//! 只保留在直通姿态回归里。与真实现的语义差异见各注入方法文档；**不模拟**：
//! DTLS/ICE 握手时序、SRTP 解帧、SCTP 拥塞/重传、真 SDP 语义校验。
//!
//! 锁模型：单全局 `std::sync::Mutex`（收集→释放→再动作，绝不跨 pc 锁嵌套、
//! 绝不持锁 await）。测试基座专用，无吞吐诉求。
//! ponytail: 全局锁，若演练并行化出现竞争再分 pc 锁。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

use mediaservo_webrtc::data_channel::RTCDataChannelInit;
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::stats::{RTCInboundRtpStreamStats, RTCStats};
use mediaservo_webrtc::track::{FrameSink, TrackKind};

use super::{DcHandle, Engine, EngineDcEvent, EngineTrack, PcHandle, TrackHandle};
use crate::error::ClientError;

/// 假 answer SDP（形状合法即可——fake 域无人解析）。
const FAKE_ANSWER_SDP: &str = "v=0\r\no=mediaservo-fake 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";
/// 假本地 DTLS 指纹：32 字节冒号十六进制（与 libwebrtc 出形一致，供 Connect 上行断言）。
const FAKE_LOCAL_FINGERPRINT: &str = "FA:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";
/// DC 事件广播容量（演练级流量下不构成瓶颈）。
const DC_EVENT_CAP: usize = 64;

/// 全局可变状态（故障旗标 + pc 注册表 + 出程记账）。
struct St {
    /// 剩余 create_pc 注入失败次数。
    fail_connect: usize,
    /// 剩余「已有远端描述再 set_remote」注入失败次数（批1 resume 预置）。
    fail_resume: usize,
    /// 下一次 DC send 丢弃旗标。
    drop_next_send: bool,
    /// 新建 DC 是否即刻 open（libwebrtc 常态 = 连上即开；演练 c 关掉它）。
    dc_auto_open: bool,
    /// DC id 分配器（从 1 起，模拟 libwebrtc 递增实配）。
    next_dc_id: i32,
    /// 活 pc 注册表（注入面寻址用）。
    pcs: Vec<Weak<FakePc>>,
    /// 出程记账：label → 已送达文本序列。
    sent: HashMap<String, Vec<String>>,
}

/// 有状态假引擎。`Clone` 得同一世界句柄（测试侧注入面与 session 侧引擎同源）。
#[derive(Clone)]
pub struct FakeEngine {
    st: Arc<Mutex<St>>,
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            st: Arc::new(Mutex::new(St {
                fail_connect: 0,
                fail_resume: 0,
                drop_next_send: false,
                dc_auto_open: true,
                next_dc_id: 1,
                pcs: Vec::new(),
                sent: HashMap::new(),
            })),
        }
    }

    /// 测试基座内统一取锁：中毒即取内值继续（演练环境无「脏状态不可信」语义，
    /// panic 传播只会掩盖首个真实失败）。
    fn lock(&self) -> std::sync::MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 收集活 pc 强引用（释放全局锁后再动作——无锁嵌套纪律）。
    fn live_pcs(&self) -> Vec<Arc<FakePc>> {
        let mut st = self.lock();
        st.pcs.retain(|w| w.strong_count() > 0);
        st.pcs.iter().filter_map(Weak::upgrade).collect()
    }

    // ── 故障注入面（最小三件，ticket K11）─────────────────────

    /// 接下来 n 次 `create_pc` 失败（= libwebrtc factory/ICE 建连期故障）。
    /// 用途：验证引擎错误正确映射 `ClientError::WebRtc` 且不 panic、会话可续用。
    pub fn fail_next_connect(&self, n: usize) {
        self.lock().fail_connect = n;
    }

    /// 接下来 n 次「对已协商 pc 再次 set_remote_offer」失败。批1 resume 路径
    /// = 存量 transport 重协商形，此旗标为该演练预置注入点（现码无 resume 调用方，
    /// 首消费者出现前它是纯 fake 侧语义）。
    pub fn fail_next_resume(&self, n: usize) {
        self.lock().fail_resume = n;
    }

    /// 下一次 DC send「发成功但丢包」（= SCTP 中途断链吞包：send_text 返回 Ok、
    /// 对端永不出帧）。用途：验证舱端重发/超时窗不被假 ack 误导。
    pub fn drop_dc_mid_send(&self) {
        self.lock().drop_next_send = true;
    }

    // ── 世界控制面 ─────────────────────────────────────────

    /// 新建 DC 是否自动进入 open 态（默认 true；演练 c 置 false 验发送态拒绝）。
    pub fn set_dc_auto_open(&self, on: bool) {
        self.lock().dc_auto_open = on;
    }

    /// 手动把所有在册 DC 推到 open（配 `set_dc_auto_open(false)` 用）。
    pub fn open_data_channels(&self) {
        for pc in self.live_pcs() {
            for dc in pc.dcs_snapshot() {
                dc.set_open();
            }
        }
    }

    /// 向所有已注册 on_track 的 pc 投递一条 video 入程 track（= libwebrtc
    /// 「demux 出 track」时刻；track_id 恒 "video"，与 stats 查询键同源）。
    pub fn inject_video_track(&self) {
        for pc in self.live_pcs() {
            pc.fire_on_track();
        }
    }

    /// 向全部已挂 sink 推一帧零值 I420 并累计 receiver stats
    /// （= RTP 解密→解码→出帧的合并抽象；宽×高×3/2 字节）。
    pub fn inject_frame(&self, width: u32, height: u32) {
        let data = vec![0u8; (width as usize) * (height as usize) * 3 / 2];
        for pc in self.live_pcs() {
            pc.deliver_frame(&data, width, height);
        }
    }

    /// 以 label 匹配投递 DC 入程消息（ack 泵等消费侧可见）。
    /// 返回 false = 该 label 的 consumer DC 尚未建立（调用方重试，竞态归测试侧消化）。
    pub fn deliver_dc_message(&self, label: &str, bytes: Vec<u8>) -> bool {
        let mut hit = false;
        for pc in self.live_pcs() {
            for dc in pc.dcs_snapshot() {
                if dc.label_inner() == label {
                    dc.push_message(bytes.clone());
                    hit = true;
                }
            }
        }
        hit
    }

    /// 出程记账读取：该 label 已真实送达的文本序列（drop_dc_mid_send 的不计入）。
    #[must_use]
    pub fn sent_texts(&self, label: &str) -> Vec<String> {
        self.lock().sent.get(label).cloned().unwrap_or_default()
    }
}

#[async_trait]
impl Engine for FakeEngine {
    async fn create_pc(&self) -> Result<Arc<dyn PcHandle>, ClientError> {
        let mut st = self.lock();
        if st.fail_connect > 0 {
            st.fail_connect -= 1;
            return Err(ClientError::WebRtc("fake: create_peer_connection 故障注入".into()));
        }
        let pc = Arc::new(FakePc { st: self.st.clone(), inner: FakePcInner::default() });
        st.pcs.push(Arc::downgrade(&pc));
        Ok(pc)
    }
}

/// pc 可变内核（回调槽 / track / DC / stats）。
#[derive(Default)]
struct FakePcInner {
    on_track: Mutex<Option<Arc<dyn Fn(EngineTrack) + Send + Sync + 'static>>>,
    had_remote: AtomicBool,
    tracks: Mutex<Vec<Arc<FakeTrack>>>,
    dcs: Mutex<Vec<Arc<FakeDc>>>,
    stats: Mutex<PcStats>,
}

#[derive(Default)]
struct PcStats {
    frames: u32,
    bytes: u64,
    width: u32,
    height: u32,
}

struct FakePc {
    st: Arc<Mutex<St>>,
    inner: FakePcInner,
}

impl FakePc {
    fn lock_st(&self) -> std::sync::MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn dcs_snapshot(&self) -> Vec<Arc<FakeDc>> {
        self.inner.dcs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn fire_on_track(&self) {
        let cb = self.inner.on_track.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(cb) = cb else { return };
        let track = Arc::new(FakeTrack::default());
        self.inner.tracks.lock().unwrap_or_else(|e| e.into_inner()).push(track.clone());
        cb(EngineTrack::new("video".into(), TrackKind::Video, track));
    }

    fn deliver_frame(&self, data: &[u8], width: u32, height: u32) {
        let tracks: Vec<Arc<FakeTrack>> =
            self.inner.tracks.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for t in &tracks {
            t.deliver(data, width, height);
        }
        let mut stats = self.inner.stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.frames = stats.frames.saturating_add(1);
        stats.bytes = stats.bytes.saturating_add(data.len() as u64);
        stats.width = width;
        stats.height = height;
    }
}

#[async_trait]
impl PcHandle for FakePc {
    fn on_track(&self, cb: Box<dyn Fn(EngineTrack) + Send + Sync + 'static>) {
        *self.inner.on_track.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::from(cb));
    }

    /// 记录式接受（fake 不校验方向/类型——现码唯一调用形 = Video+Recvonly）。
    fn add_transceiver(
        &self,
        _kind: TrackKind,
        _init: RTCRtpTransceiverInit,
    ) -> Result<(), ClientError> {
        Ok(())
    }

    async fn set_remote_offer(&self, _sdp: String) -> Result<(), ClientError> {
        if self.inner.had_remote.load(Ordering::Acquire) {
            let mut st = self.lock_st();
            if st.fail_resume > 0 {
                st.fail_resume -= 1;
                return Err(ClientError::WebRtc("fake: 重协商（resume 形）故障注入".into()));
            }
        }
        self.inner.had_remote.store(true, Ordering::Release);
        Ok(())
    }

    async fn create_answer(&self) -> Result<String, ClientError> {
        Ok(FAKE_ANSWER_SDP.to_string())
    }

    async fn set_local_answer(&self, _sdp: &str) -> Result<(), ClientError> {
        Ok(())
    }

    fn local_dtls_fingerprint(&self) -> Option<String> {
        Some(FAKE_LOCAL_FINGERPRINT.to_string())
    }

    /// 注入帧的累计账（零帧 = 空表，同真 receiver 未收流形态）。
    fn receiver_stats(&self, track_id: &str) -> Vec<RTCStats> {
        let stats = self.inner.stats.lock().unwrap_or_else(|e| e.into_inner());
        if stats.frames == 0 {
            return Vec::new();
        }
        vec![RTCStats::InboundRtp(RTCInboundRtpStreamStats {
            id: format!("fake-inbound-{track_id}"),
            timestamp: 0.0,
            ssrc: 1,
            kind: "video".into(),
            packets_received: u64::from(stats.frames),
            packets_lost: 0,
            bytes_received: stats.bytes,
            frames_decoded: stats.frames,
            frame_width: stats.width,
            frame_height: stats.height,
            frames_per_second: 30.0,
        })]
    }

    async fn create_data_channel(
        &self,
        label: &str,
        init: RTCDataChannelInit,
    ) -> Result<Arc<dyn DcHandle>, ClientError> {
        let (id, auto_open) = {
            let mut st = self.lock_st();
            // negotiated 形沿用入参 id（server 分配 stream_id 语义）；
            // 非 negotiated = libwebrtc 实配递增 id。
            let id = if init.negotiated { init.id } else { st.next_dc_id };
            if !init.negotiated {
                st.next_dc_id += 1;
            }
            (id, st.dc_auto_open)
        };
        let (tx, _rx) = broadcast::channel(DC_EVENT_CAP);
        let dc = Arc::new(FakeDc {
            label: label.to_string(),
            id,
            open: AtomicBool::new(auto_open),
            tx,
            st: self.st.clone(),
        });
        if auto_open {
            dc.push_open();
        }
        self.inner.dcs.lock().unwrap_or_else(|e| e.into_inner()).push(dc.clone());
        Ok(dc)
    }
}

/// 注入 track 的 sink 槽（inject_frame 的投递目标）。
#[derive(Default)]
struct FakeTrack {
    sink: Mutex<Option<Box<dyn FrameSink>>>,
}

impl FakeTrack {
    fn deliver(&self, data: &[u8], width: u32, height: u32) {
        let guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sink) = guard.as_ref() {
            sink.on_frame(data, width, height);
        }
    }
}

impl TrackHandle for FakeTrack {
    fn attach(&self, sink: Box<dyn FrameSink>) {
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }
}

struct FakeDc {
    label: String,
    id: i32,
    open: AtomicBool,
    tx: broadcast::Sender<EngineDcEvent>,
    st: Arc<Mutex<St>>,
}

impl FakeDc {
    fn label_inner(&self) -> &str {
        &self.label
    }

    fn set_open(&self) {
        if !self.open.swap(true, Ordering::AcqRel) {
            self.push_open();
        }
    }

    fn push_open(&self) {
        // 无订阅者 = 尚未 events()——Open 语义由后续 send 态检查承载，丢之无害。
        let _ = self.tx.send(EngineDcEvent::Open);
    }

    fn push_message(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(EngineDcEvent::Message(bytes));
    }
}

#[async_trait]
impl DcHandle for FakeDc {
    fn label(&self) -> &str {
        &self.label
    }

    fn id(&self) -> i32 {
        self.id
    }

    async fn send_text(&self, text: &str) -> Result<(), ClientError> {
        if !self.open.load(Ordering::Acquire) {
            return Err(ClientError::WebRtc(format!("DC {} send: not open (fake)", self.label)));
        }
        let mut st = self.st.lock().unwrap_or_else(|e| e.into_inner());
        if st.drop_next_send {
            st.drop_next_send = false;
            tracing::warn!(label = %self.label, "fake: drop_dc_mid_send 命中——send 返 Ok 但未达对端");
            return Ok(());
        }
        st.sent.entry(self.label.clone()).or_default().push(text.to_string());
        Ok(())
    }

    async fn events(&self) -> mpsc::UnboundedReceiver<EngineDcEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut brx = self.tx.subscribe();
        tokio::spawn(async move {
            loop {
                match brx.recv().await {
                    Ok(ev) => {
                        if tx.send(ev).is_err() {
                            break; // 订阅端 drop
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        rx
    }

    async fn close(&self) {
        self.open.store(false, Ordering::Release);
        let _ = self.tx.send(EngineDcEvent::Closed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 引擎自语义钉（不经 session 层）：故障旗标一次性消耗 + DC 态门 + 记账。
    #[tokio::test]
    async fn fault_knobs_are_one_shot() {
        let eng = FakeEngine::new();
        eng.fail_next_connect(1);
        assert!(matches!(eng.create_pc().await, Err(ClientError::WebRtc(_))));
        assert!(eng.create_pc().await.is_ok(), "注入应一次性消耗");
    }

    #[tokio::test]
    async fn dc_send_requires_open_and_drop_mid_send_skips_ledger() {
        let eng = FakeEngine::new();
        eng.set_dc_auto_open(false);
        let pc = eng.create_pc().await.unwrap();
        let dc = pc.create_data_channel("chassis", RTCDataChannelInit::default()).await.unwrap();
        assert!(
            matches!(dc.send_text("x").await, Err(ClientError::WebRtc(m)) if m.contains("not open")),
            "未 open 应拒发"
        );
        eng.open_data_channels();
        eng.drop_dc_mid_send();
        assert!(dc.send_text("dropped").await.is_ok(), "丢弃注入应仍返 Ok（假送达）");
        assert!(eng.sent_texts("chassis").is_empty());
        assert!(dc.send_text("real").await.is_ok());
        assert_eq!(eng.sent_texts("chassis"), ["real"]);
    }

    #[tokio::test]
    async fn resume_injection_hits_only_re_negotiation() {
        let eng = FakeEngine::new();
        eng.fail_next_resume(1);
        let pc = eng.create_pc().await.unwrap();
        // 首次协商不受 resume 旗标影响（resume = 存量 pc 重协商语义）。
        assert!(pc.set_remote_offer("offer1".into()).await.is_ok());
        assert!(
            matches!(pc.set_remote_offer("offer2".into()).await, Err(ClientError::WebRtc(_))),
            "二次 set_remote 应被 resume 注入命中"
        );
        assert!(pc.set_remote_offer("offer3".into()).await.is_ok(), "注入一次性消耗");
    }
}
