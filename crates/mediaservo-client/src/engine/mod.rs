//! Engine 抽象层——client 实际消费的 WebRTC 原语最小面（S6 批0 / K11）。
//!
//! 边界纪律：**只圈现码真实调用点，不发明 client 用不到的方法**。mediasoup-client
//! FakeHandler 同型——把「引擎行为」层假面化，让恢复/错误/DC 逻辑进 CI 秒级确定性。
//!
//! 真实现 = [`sys::SysEngine`]（mediaservo-webrtc backend-webrtc-sys 直通，零逻辑改）；
//! 假实现 = [`fake::FakeEngine`]（feature `engine-fake` 门控，默认 off，发布构建不含）。
//!
//! 类型选择：SDP 以裸 `String` 过界（现码三处 set_remote 全是 Offer、answer 只取
//! `.sdp`）；`RTCDataChannelInit`/`RTCStats`/`TrackKind`/`FrameSink` 是
//! mediaservo-webrtc 抽象层的轻量纯数据/ trait 类型（不拉后端实例），直接复用——
//! sfu.rs 公开签名（返回 RTCDataChannelInit）因此零改动。
//!
//! # 方法 ↔ 现调用点对账（行号 = 重构前 session.rs/control.rs@HEAD）
//!
//! | 方法 | 用途 | 现调用点 |
//! |------|------|---------|
//! | `Engine::create_pc` | 建一条 ICE-Lite SFU peer connection（无 STUN） | session.rs:435-444 `create_pc()` |
//! | `PcHandle::on_track` | 注册入程 track 回调（必须先于 set_remote——晚注册丢首 track） | session.rs:247-256 |
//! | `PcHandle::add_transceiver` | 加 transceiver（现码仅 Video+Recvonly，签名镜像 libwebrtc 不裁剪） | session.rs:301-308 |
//! | `PcHandle::set_remote_offer` | 设远端 offer（mediasoup transport 参数合成 SDP 后喂入） | session.rs:310, 367, 642 |
//! | `PcHandle::create_answer` | 生成本地 answer SDP 体 | session.rs:313, 386, 645 |
//! | `PcHandle::set_local_answer` | 设本地 answer 收口协商 | session.rs:317, 390, 649 |
//! | `PcHandle::local_dtls_fingerprint` | 本地 DTLS 指纹 → ConnectWebRtcTransport dtls_parameters | session.rs:453-455 |
//! | `PcHandle::receiver_stats` | 指定 track 的 inbound-rtp stats（诊断/mini-stats 真值源） | session.rs:101-106 |
//! | `PcHandle::create_data_channel` | 建 DC（出程控制=非 negotiated；ack 消费=negotiated 带外） | session.rs:377-380, 741-744 |
//! | `DcHandle::label` / `DcHandle::id` | SCTP 流参数派生（stream_id = libwebrtc 实配 DC id） | session.rs:407 |
//! | `DcHandle::send_text` | 控制信封出程 | control.rs:93-95 |
//! | `DcHandle::events` | 入程事件流（Message/Closed） | session.rs:752, 758-775 |
//! | `DcHandle::close` | 车端 DataProducer 死亡时本地定向自拆（释 sid） | session.rs:779 |

#[cfg(feature = "engine-fake")] // 发布构建不含假引擎（K11 门控纪律）
pub mod fake;
pub mod sys;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use mediaservo_webrtc::data_channel::{RTCDataChannelInit, RTCDataChannelState};
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::stats::RTCStats;
use mediaservo_webrtc::track::{FrameSink, TrackKind};

use crate::error::ClientError;

pub use sys::SysEngine;

/// DC 入程事件（镜像 `mediaservo_webrtc::RTCDataChannelEvent` 四变体，
/// 载荷去后端化——现码消费点仅 Message/Closed，其余透传保持事件语义）。
#[derive(Debug, Clone)]
pub enum EngineDcEvent {
    Open,
    Closed,
    Message(Vec<u8>),
    Error(String),
}

/// 引擎侧 track 句柄（sys=真 receiver / fake=注入槽），对 EngineTrack 使用者不透明。
pub(crate) trait TrackHandle: Send + Sync {
    /// 挂载帧出口（泵线程回调，非 async；满丢 = latest 语义由 sink 侧决定）。
    fn attach(&self, sink: Box<dyn FrameSink>);
}

/// `on_track` 回调载荷：一条入程远端 track。
pub struct EngineTrack {
    pub track_id: String,
    pub kind: TrackKind,
    handle: Arc<dyn TrackHandle>,
}

impl EngineTrack {
    pub(crate) fn new(track_id: String, kind: TrackKind, handle: Arc<dyn TrackHandle>) -> Self {
        Self { track_id, kind, handle }
    }

    /// 挂载 I420 帧 sink（现码唯一消费者 = session.rs FrameChanSink）。
    pub fn set_frame_sink(&self, sink: Box<dyn FrameSink>) {
        self.handle.attach(sink);
    }
}

/// WebRTC 引擎工厂面。client 全部引擎接触的唯一入口；
/// `RoomSession::connect_with_engine` 注入点。
#[async_trait]
pub trait Engine: Send + Sync + 'static {
    /// 建一条 SFU peer connection（ICE-Lite、空 ice_servers、policy=All）。
    /// [session.rs:435-444]
    async fn create_pc(&self) -> Result<Arc<dyn PcHandle>, ClientError>;
}

/// 单条 peer connection 操作面（句柄即生命周期：最后一个 Arc drop 即断）。
#[async_trait]
pub trait PcHandle: Send + Sync + 'static {
    /// 注册入程 track 回调。[session.rs:247]
    fn on_track(&self, cb: Box<dyn Fn(EngineTrack) + Send + Sync + 'static>);
    /// 加 transceiver。[session.rs:301]
    fn add_transceiver(
        &self,
        kind: TrackKind,
        init: RTCRtpTransceiverInit,
    ) -> Result<(), ClientError>;
    /// 设远端 offer。[session.rs:310, 367, 642]
    async fn set_remote_offer(&self, sdp: String) -> Result<(), ClientError>;
    /// 生成本地 answer（SDP 体）。[session.rs:313, 386, 645]
    async fn create_answer(&self) -> Result<String, ClientError>;
    /// 设本地 answer。[session.rs:317, 390, 649]
    async fn set_local_answer(&self, sdp: &str) -> Result<(), ClientError>;
    /// 本地 DTLS 指纹（sha-256 冒号十六进制）。[session.rs:453]
    fn local_dtls_fingerprint(&self) -> Option<String>;
    /// 指定 track 的 receiver stats。[session.rs:104]
    fn receiver_stats(&self, track_id: &str) -> Vec<RTCStats>;
    /// 建数据通道。[session.rs:377, 741]
    async fn create_data_channel(
        &self,
        label: &str,
        init: RTCDataChannelInit,
    ) -> Result<Arc<dyn DcHandle>, ClientError>;
}

/// 单条 DataChannel 操作面。
#[async_trait]
pub trait DcHandle: Send + Sync + 'static {
    /// 通道 label（sctp_stream_params 派生输入）。[session.rs:407]
    fn label(&self) -> &str;
    /// libwebrtc 实配 DC id = SCTP stream_id。[session.rs:407]
    fn id(&self) -> i32;
    /// 文本发送（控制信封）。[control.rs:93]
    async fn send_text(&self, text: &str) -> Result<(), ClientError>;
    /// S6/K5: 通道就绪态（open/closing/closed + connecting——背压/可用性判据）。
    fn state(&self) -> RTCDataChannelState;
    /// S6/K5: 待发队列字节数（急停投递前水位预检用）。
    async fn buffered_amount(&self) -> u64;
    /// 订阅入程事件流（每次调用独立订阅端，同 spool 语义）。[session.rs:752]
    async fn events(&self) -> mpsc::UnboundedReceiver<EngineDcEvent>;
    /// 主动关闭（本地自拆）。[session.rs:779]
    async fn close(&self);
}
