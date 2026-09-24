//! 房间会话——link 信令之上的 SFU 消费 + 控制出程（S2 核心）。
//!
//! consume 序列本地镜像 `mediaservo-field::PullSession::subscribe` 形状
//! （C21 依赖边界：client 禁依 field）；控制序列镜像 `mediaservo-host::controller`
//! S1 出程形（Send transport → DC 先于 answer → Connect → CreateDataProducer）。
//! ICE-Lite：候选随 WebRtcTransportCreated 内联，无 candidate 交换回合；
//! `Error{code:0,"transport_connected"}` 是 server 惯例 ack，非真错误。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mediaservo_common::protocol::{
    ControlAck, ControlEnvelope, DtlsParameters, Fingerprint, IceCandidate, IceParameters,
    MediaKind, SctpStreamParameters, SignalingMessage, TransportDirection,
};
use mediaservo_link::{SignalClient, SignalEvent};
use mediaservo_webrtc::rtp::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use mediaservo_webrtc::track::TrackKind;
use tokio::sync::{Mutex, broadcast, mpsc};

use crate::config::{ClientConfig, sfu_peer_key};
use crate::consumer::{Consumer, ConsumerSlot};
use crate::control::ControlChannel;
use crate::engine::{DcHandle, Engine, EngineDcEvent, PcHandle, SysEngine};
use crate::error::{ClientError, classify_link_error, from_wire_error};
use crate::sfu;
use crate::signal::{LinkSignal, Signal};
use crate::supervisor::{self, ConnectionState, SupervisorCtx};

/// 控制域能力门（I5 首用户，S0 契约）：CreateDataProducer 的 can_control 门
/// 自方言 2 起生效——低于 2 本地预拒，不发请求（省一次 4012 往返）。
const CONTROL_MIN_PROTOCOL: u32 = 2;
/// CreateDataProducer 子协议（host controller DATA_PROTOCOL 同值）。
const DATA_PROTOCOL: &str = "sctp";
/// server 响应等待窗（transport/consumed/producer）。
const RESPONSE_WAIT: Duration = Duration::from_secs(10);

/// 解码视频帧（I420，libwebrtc 侧渲染前格式；C5 边界语义）。
/// inbound-rtp 折叠规则单一落点（求和/max 混排——自由函数形单测可钉，免触真 pc）。
pub(crate) fn fold_inbound_stats(
    items: Vec<mediaservo_webrtc::stats::RTCStats>,
) -> VideoStreamStats {
    use mediaservo_webrtc::stats::RTCStats;
    let mut out = VideoStreamStats::default();
    // 诊断（debug 级）：区分「stats 表里根本没有该 inbound-rtp（demux 注册失败）」与
    // 「有行但全零（RTP 未达 transport）」——零帧排障的两级判据（09-24 实录入册）。
    tracing::debug!(
        rows = items.len(),
        kinds = ?items.iter().map(|x| match x {
            RTCStats::InboundRtp(_) => "inbound-rtp".to_string(),
            o => format!("{o:?}").chars().take(20).collect::<String>(),
        }).collect::<Vec<_>>(),
        "fold_inbound_stats raw rows"
    );
    for st in items {
        if let RTCStats::InboundRtp(r) = st {
            out.bytes_received += r.bytes_received;
            out.packets_received += r.packets_received;
            out.packets_lost += r.packets_lost;
            out.frames_decoded += u64::from(r.frames_decoded);
            out.frame_width = out.frame_width.max(r.frame_width);
            out.frame_height = out.frame_height.max(r.frame_height);
            out.frames_per_second = out.frames_per_second.max(r.frames_per_second);
            out.jitter = out.jitter.max(r.jitter);
            out.frame_dropped += r.frame_dropped;
            out.nack_count += r.nack_count;
            out.pli_count += r.pli_count;
            out.fir_count += r.fir_count;
        }
    }
    out
}

/// [`RoomSession::video_stats_summary`] 的扁平结果（serde 键名 = C 面 JSON 契约）。
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct VideoStreamStats {
    pub bytes_received: u64,
    pub packets_received: u64,
    pub packets_lost: u64,
    pub frames_decoded: u64,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frames_per_second: f64,
    pub jitter: f64,        // 秒（多路 union 取 max）
    pub frame_dropped: u64, // 多路求和
    pub nack_count: u64,
    pub pli_count: u64,
    pub fir_count: u64,
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// 帧到达无时钟源（webrtc-sys 未暴露 rtp ts）——恒 0，上层按到达序消费。
    pub ts_us: i64,
}

/// 已入房会话：视频消费 + 控制出程共用一条 link WS 信令面。
///
/// S6/K1：信令面韧性——`ctx` 持有 supervisor 上下文（重连环/帧槽/状态机），
/// 跨重连存续；`events`/`pump_events` 是 per-session 订阅（重连后 refresh）。
pub struct RoomSession {
    /// S6/K1: 信令面韧性上下文（Arc 单实例；forwarder/supervisor/重放共用）。
    ctx: Arc<SupervisorCtx>,
    /// S4/T3.5: 急停 HMAC 密钥（emergency_stop 签名用；None = 不签）。
    hmac_key: Option<String>,
    /// connect 时刻订阅（broadcast 无历史重放——接住 join 时 server 回放的
    /// late-join NewProducer）。重连后由 supervisor 刷新。
    events: Mutex<broadcast::Receiver<SignalEvent>>,
    /// S2d: open_control 专用第二流（connect 同刻订阅 = join 回放零缺口；
    /// take 后移交 ack 泵，每会话一次性）。
    pump_events: Mutex<Option<broadcast::Receiver<SignalEvent>>>,
}

impl RoomSession {
    /// S2c 诊断面：video receiver 的 inbound-rtp stats（"包没进来" vs
    /// "进而不解" 二分的定案读点；track_id = offer msid 注入的 "video"）。
    #[must_use]
    pub fn video_receiver_stats(&self) -> Vec<mediaservo_webrtc::stats::RTCStats> {
        // 全路并集（与旧 _pcs flat_map 语义逐位一致；单路读数走 Consumer::stats）。
        self.ctx
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .flat_map(|slot| slot.receiver_stats())
            .collect()
    }

    /// 消费面视频统计汇总（W3 mini-stats 数据源）：本会话全部 inbound-rtp 折叠。
    /// 形状刻意**扁平 + 稳定键名**（C ABI JSON 透传给 C++ mini-parse 消费，
    /// 不导出 webrtc 内部枚举 wire）。计数类求和；fps 取最大（多轨无求和语义）。
    pub fn video_stats_summary(&self) -> VideoStreamStats {
        fold_inbound_stats(self.video_receiver_stats())
    }

    /// S4/a4·T3.5：急停双路 —— DC 快路径（带 HMAC sig，车端 act-then-audit 主留痕）
    /// 伴随 WS `ControlAudit` 审计副本（best-effort：副本失败不影响已投递 DC 命令
    /// ——急停恰多发于 WS 降级窗，审计主落点=车端执行器，PLAN §11.6 席3 裁决）。
    pub async fn emergency_stop(
        &mut self,
        ctl: &mut ControlChannel,
        label: &str,
        seq: u64,
        payload: serde_json::Value,
    ) -> Result<(), ClientError> {
        let mut env = ControlEnvelope { seq, cmd: "estop".to_string(), payload, sig: None };
        if let Some(k) = &self.hmac_key {
            env.sig = Some(mediaservo_common::protocol::control_hmac_sign(k, &env));
        }
        ctl.send_envelope(label, &env).await?; // DC 快路径先行（急停语义 = 不等审计副本）
        let _ = self
            .ctx
            .signal
            .send(SignalingMessage::ControlAudit {
                room_id: self.room_id().to_string(),
                seq,
                cmd: env.cmd.clone(),
                sig_present: env.sig.is_some(),
            })
            .await; // WS 副本 best-effort（C15：失败静默 = 主留痕在车端 actuation）
        Ok(())
    }

    /// S4：急停密钥是否已配置（观测面；密钥值永不外泄）。
    #[must_use]
    pub fn hmac_key_debug_present(&self) -> bool {
        self.hmac_key.is_some()
    }

    /// 信令连接 + 入房（PSK 或 JWT，见 [`ClientConfig`]；link 承载全部握手）。
    /// 引擎 = 真 webrtc-sys 直通（[`Self::connect_with_engine`] 的便捷包装）。
    pub async fn connect(cfg: &ClientConfig) -> Result<Self, ClientError> {
        Self::connect_with_engine(cfg, Arc::new(SysEngine::new())).await
    }

    /// 信令连接 + 入房 + 引擎注入（S6 批0/K11：测试以 FakeEngine 走全链确定性演练）。
    ///
    /// S6/K1：创建 SupervisorCtx（持有 Arc<SignalClient> 供 supervisor 重连），
    /// 订阅事件流后 spawn epoch-1 forwarder + supervisor。
    pub async fn connect_with_engine(
        cfg: &ClientConfig,
        engine: Arc<dyn Engine>,
    ) -> Result<Self, ClientError> {
        let client = SignalClient::new(
            &cfg.signaling_url,
            cfg.psk.as_deref().unwrap_or(""),
            &cfg.room_id,
            cfg.role.clone(),
        );
        let client = if let Some(jwt) = &cfg.jwt { client.with_jwt(jwt.clone()) } else { client };
        let session = client.connect().await.map_err(classify_link_error)?;
        // 第二流与主流同刻订阅（LinkSignal::events 在 async 态 blocking_lock
        // 会 panic——订阅必须在此同步点完成）。
        let raw_events = session.events();
        let pump_events = session.events();
        let signal = Arc::new(LinkSignal::new(session, sfu_peer_key(&cfg.role).to_string()));
        let client = Arc::new(client);
        let ctx = SupervisorCtx::new(signal, client, engine, raw_events);
        let events = ctx.ev_tx.subscribe();
        supervisor::start(&ctx);
        Ok(Self {
            ctx,
            hmac_key: cfg.hmac_key.clone(),
            events: Mutex::new(events),
            pump_events: Mutex::new(Some(pump_events)),
        })
    }

    /// 谈成的方言版本（S0；旧 server = 1）。
    #[must_use]
    pub fn negotiated(&self) -> u32 {
        self.ctx.signal.negotiated()
    }

    #[must_use]
    pub fn room_id(&self) -> &str {
        self.ctx.signal.room_id()
    }

    /// 等待房间内的视频 producer（join 时 server 对存量 producer 回放 NewProducer）。
    pub async fn wait_video_producer(&mut self, wait: Duration) -> Result<String, ClientError> {
        let mut ev = self.events.lock().await;
        tokio::time::timeout(wait, async {
            loop {
                match ev.recv().await {
                    Ok(SignalEvent::Message(SignalingMessage::NewProducer {
                        producer_id,
                        kind: MediaKind::Video,
                        ..
                    })) => return Ok(producer_id),
                    Ok(SignalEvent::Disconnected { reason }) => {
                        return Err(ClientError::InvalidState(format!(
                            "等待 producer 期信令断开: {reason}"
                        )));
                    }
                    Ok(_) => {}
                    Err(_) => {
                        return Err(ClientError::InvalidState("信令事件流关闭".into()));
                    }
                }
            }
        })
        .await
        .map_err(|_| ClientError::Timeout { what: "video NewProducer" })?
    }

    /// S6/K4: 订阅一路视频 producer，返回 [`Consumer`] 句柄（多路注册）。
    ///
    /// 序列 = 旧 consume_video 原体（挪入 [`consume_sequence`]，零复制）；差异 =
    /// pc 挂注册槽：帧通道 frame_tx 随槽存续，K1 重连重放换 pc 不换槽，app
    /// 手里的 receiver 跨重连续流（无感）。与重放经 consume_lock 串行（K1）。
    pub async fn consume(&self, producer_id: &str) -> Result<Consumer, ClientError> {
        let _seq = self.ctx.consume_lock.lock().await;
        let mut ev = self.events.lock().await;
        let (frame_tx, frame_rx) = mpsc::channel::<VideoFrame>(3);
        let pc = consume_sequence(
            producer_id,
            &self.ctx.engine,
            &self.ctx.signal,
            &mut ev,
            frame_tx.clone(),
        )
        .await?;
        let slot = Arc::new(ConsumerSlot::new(producer_id, frame_tx));
        slot.set_pc(pc);
        self.ctx.slots.lock().unwrap_or_else(|e| e.into_inner()).push(slot.clone());
        Ok(Consumer::new(producer_id.to_string(), slot, frame_rx))
    }

    /// 旧形桥（R3 行为重映射）：等价 `consume(p).await?.into_receiver()`，
    /// 返回裸帧流接收端，行为与旧 consume_video 逐字节一致。
    pub async fn consume_video(
        &mut self,
        producer_id: &str,
    ) -> Result<mpsc::Receiver<VideoFrame>, ClientError> {
        self.consume(producer_id).await.map(Consumer::into_receiver)
    }

    /// 开出程控制通道集（host S1 镜像）。前置 I5 门：方言 ≥2，否则本地预拒。
    pub async fn open_control(&mut self, labels: &[&str]) -> Result<ControlChannel, ClientError> {
        let got = self.ctx.signal.negotiated();
        if got < CONTROL_MIN_PROTOCOL {
            return Err(ClientError::ProtocolTooLow { need: CONTROL_MIN_PROTOCOL, got });
        }
        let mut ev = self.events.lock().await;
        let room = self.ctx.signal.room_id().to_string();
        let peer = self.ctx.signal.sfu_peer_id().to_string();
        // S2d: 取 connect 同刻预订阅的第二流（建立期车端 ack announce 零缺口）。
        let pump_ev =
            self.pump_events.lock().await.take().ok_or_else(|| {
                ClientError::InvalidState("控制面已开启（ack 泵每会话一次）".into())
            })?;

        // 1. Send transport
        self.ctx
            .signal
            .send(SignalingMessage::CreateWebRtcTransport {
                room_id: room.clone(),
                peer_id: peer.clone(),
                direction: TransportDirection::Send,
            })
            .await?;
        let (transport_id, ice, dtls, candidates) =
            tokio::time::timeout(RESPONSE_WAIT, await_transport_created(&mut ev))
                .await
                .map_err(|_| ClientError::Timeout { what: "WebRtcTransportCreated(send)" })??;

        // 2. PC + 合成 offer set_remote；DC 必须先于 answer 创建（S1 时序合同）
        let pc = self.ctx.engine.create_pc().await?;
        let remote_sdp = sfu::build_dc_remote_sdp(&ice, &dtls, candidates.as_ref());
        pc.set_remote_offer(remote_sdp).await?;

        // 3. 每 label 出程 DC（mediasoup 单向对模型：producer DC 无入程，回执
        //    回程 = step 6 ack 消费链路——2026-09-15 活体证 consumer 反向不透传）。
        let (ack_tx, ack_rx) = mpsc::channel::<ControlAck>(32);
        let mut dcs: HashMap<String, Arc<dyn DcHandle>> = HashMap::new();
        let mut order = Vec::with_capacity(labels.len());
        for label in labels {
            let dc = pc.create_data_channel(label, sfu::channel_init(label)).await?;
            dcs.insert(label.to_string(), dc);
            order.push(label.to_string());
        }

        // 4. answer/connect 收口
        let answer = pc.create_answer().await?;
        pc.set_local_answer(&answer).await?;
        connect_transport(&*self.ctx.signal, &room, &peer, &transport_id, &pc).await?;

        // 5. 逐 DC CreateDataProducer announce（4012 → ControlDenied 终态）
        let mut producer_ids = Vec::with_capacity(order.len());
        for label in &order {
            let dc = &dcs[label];
            self.ctx
                .signal
                .send(SignalingMessage::CreateDataProducer {
                    room_id: room.clone(),
                    peer_id: peer.clone(),
                    transport_direction: TransportDirection::Send,
                    label: label.clone(),
                    protocol: DATA_PROTOCOL.into(),
                    sctp_stream_parameters: Some(
                        sfu::sctp_stream_params(dc.label(), dc.id())
                            .map_err(ClientError::WebRtc)?,
                    ),
                    transport_id: Some(transport_id.clone()),
                })
                .await?;
            let dp = tokio::time::timeout(RESPONSE_WAIT, await_data_producer_created(&mut ev))
                .await
                .map_err(|_| ClientError::Timeout { what: "DataProducerCreated" })??;
            tracing::info!(label = %label, data_producer_id = %dp, "client DataProducer 已建立");
            producer_ids.push(dp);
        }

        // 6. S2d 官方单向对模型：后台消费车端 label=ack DataProducer，negotiated
        //    consumer DC 的 ControlAck 路由进 ack_rx（recv_ack 的正式供数来源）。
        tokio::spawn(ack_consumer_pump(
            pump_ev,
            self.ctx.signal.clone(),
            self.ctx.engine.clone(),
            room,
            peer,
            ack_tx,
        ));

        Ok(ControlChannel::new(dcs, order, ack_rx, producer_ids, pc))
    }

    /// 关闭会话（shutdown supervisor + 释放 WS；帧/ack 流随 drop 收敛）。
    pub async fn close(self) -> Result<(), ClientError> {
        self.ctx.shutdown();
        self.ctx.signal.close().await
    }

    /// S6/K1: 信令连接态观测（单一 watch 真源；重连/终态实时可读）。
    #[must_use]
    pub fn connection_state(&self) -> ConnectionState {
        self.ctx.state()
    }

    /// S6/K1: 信令连接态变更流（subscriber 语义；重连/终态自动推送）。
    #[must_use]
    pub fn subscribe_connection_state(&self) -> tokio::sync::watch::Receiver<ConnectionState> {
        self.ctx.subscribe_state()
    }

    /// S6/K1: 动态切换自动重连（D273 红牌语义：Failed = auth 族终态，不受此开关控制）。
    ///
    /// **DEVIATION**：设计最初考虑为 ClientConfig 字段，但 client-c lib.rs:238
    /// 的 ClientConfig 是 exhaustive 字面量，新增字段 = C ABI break（本批禁触），
    /// 故改为运行时方法（K5 亦用运行时 setter 形）。
    pub fn set_auto_reconnect(&mut self, on: bool) {
        self.ctx.set_auto_reconnect(on);
    }
}

impl Drop for RoomSession {
    fn drop(&mut self) {
        self.ctx.shutdown();
    }
}

/// K4/K1: 单路视频收流建立全序列（Recv transport → router caps → Consume →
/// answerer 协商 → Connect）。自旧 consume_video 原体抽出 = K1 重放共用，零复制。
/// 帧出口由 `frame_tx` 注入（槽持有的长寿命 sender；on_track 每 sink 克隆一份）。
pub(crate) async fn consume_sequence(
    producer_id: &str,
    engine: &Arc<dyn Engine>,
    signal: &LinkSignal,
    ev: &mut broadcast::Receiver<SignalEvent>,
    frame_tx: mpsc::Sender<VideoFrame>,
) -> Result<Arc<dyn PcHandle>, ClientError> {
    let room = signal.room_id().to_string();
    let peer = signal.sfu_peer_id().to_string();

    // 1. Recv transport
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: room.clone(),
            peer_id: peer.clone(),
            direction: TransportDirection::Recv,
        })
        .await?;
    let (transport_id, ice, dtls, candidates) =
        tokio::time::timeout(RESPONSE_WAIT, await_transport_created(ev))
            .await
            .map_err(|_| ClientError::Timeout { what: "WebRtcTransportCreated(recv)" })??;

    // 2. PC（mediasoup ICE-Lite，无 STUN）+ on_track 先于 set_remote（晚注册丢首 track）
    let pc = engine.create_pc().await?;
    pc.on_track(Box::new(move |track| {
        // S2c 诊断（常久日志）：on_track 到达 = RTP 已 demux 成 track；
        // 「ICE/DTLS 通过但零帧」的分水岭判据。
        tracing::info!(kind = ?track.kind, track_id = ?track.track_id, "consume on_track 到达");
        track.set_frame_sink(Box::new(FrameChanSink { tx: frame_tx.clone() }));
    }));

    // 3a. Router 能力查询直传（mediasoup-client Device.load 同款官方流程，C18；
    //     手拼 caps 在 H264 producer 下必拒——S2b 活体教训，PullSession 同罪另案）。
    signal.send(SignalingMessage::GetRouterRtpCapabilities { room_id: room.clone() }).await?;
    let router_caps = tokio::time::timeout(RESPONSE_WAIT, await_router_caps(ev))
        .await
        .map_err(|_| ClientError::Timeout { what: "RouterRtpCapabilities" })??;

    // 3b. Consume（C1 显式绑 transport_id；caps = router 原样回包）
    signal
        .send(SignalingMessage::Consume {
            room_id: room.clone(),
            peer_id: peer.clone(),
            producer_id: producer_id.to_string(),
            rtp_capabilities: router_caps,
            transport_id: Some(transport_id.clone()),
        })
        .await?;
    let consumer_rtp = tokio::time::timeout(RESPONSE_WAIT, await_consumed(ev))
        .await
        .map_err(|_| ClientError::Timeout { what: "Consumed" })??;

    // 4. remote SDP：codec 取自 consumer rtp_parameters，注入 ssrc 供 demux
    let (pt, name, clock, fmtp) = sfu::codec_from_consumer(&consumer_rtp).ok_or_else(|| {
        ClientError::MalformedResponse("Consumed rtp_parameters 无 video codec".into())
    })?;
    let remote_sdp = sfu::build_recv_video_sdp(
        &ice,
        &dtls,
        candidates.as_ref(),
        pt,
        &name,
        clock,
        fmtp.as_deref(),
    );
    let remote_sdp = sfu::inject_remote_ssrc(&remote_sdp, &consumer_rtp);

    // 5. answerer 收口：add_transceiver(recvonly) → set_remote(offer) → answer
    pc.add_transceiver(
        TrackKind::Video,
        RTCRtpTransceiverInit {
            direction: RTCRtpTransceiverDirection::Recvonly,
            ..Default::default()
        },
    )?;
    tracing::debug!(remote_sdp = %remote_sdp, "consume remote offer（SSRC 注入后）");
    pc.set_remote_offer(remote_sdp).await?;
    let answer = pc.create_answer().await?;
    pc.set_local_answer(&answer).await?;

    // 6. Connect（本地 DTLS 指纹，role=client）
    connect_transport(signal, &room, &peer, &transport_id, &pc).await?;

    tracing::info!(producer_id, transport_id = %transport_id, "client consume_video 建立");
    Ok(pc)
}

async fn connect_transport(
    signal: &dyn Signal,
    room: &str,
    peer: &str,
    transport_id: &str,
    pc: &Arc<dyn PcHandle>,
) -> Result<(), ClientError> {
    let fp_hex = pc
        .local_dtls_fingerprint()
        .ok_or_else(|| ClientError::WebRtc("无本地 DTLS 指纹".into()))?;
    signal
        .send(SignalingMessage::ConnectWebRtcTransport {
            room_id: room.to_string(),
            peer_id: peer.to_string(),
            transport_id: transport_id.to_string(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
                role: "client".to_string(),
            },
        })
        .await
}

// ── 事件等待（broadcast 单流顺序消费；transport_connected ack 豁免）──

async fn await_transport_created(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<(String, IceParameters, DtlsParameters, Option<Vec<IceCandidate>>), ClientError> {
    loop {
        match next_msg(ev).await? {
            SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            } => return Ok((transport_id, ice_parameters, dtls_parameters, ice_candidates)),
            SignalingMessage::Error { message, .. } if message == "transport_connected" => {}
            // 并发事件容忍（车房常态：他人 producer 广播与 transport 应答同窗）；
            // 仅 Error 终态（C15 带上下文）。
            err @ SignalingMessage::Error { .. } => {
                return Err(on_unexpected(err, "WebRtcTransportCreated"));
            }
            other => {
                tracing::debug!(msg = ?other, "await(transport) 忽略无关事件，继续等");
                continue;
            }
        }
    }
}

async fn await_consumed(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<serde_json::Value, ClientError> {
    loop {
        match next_msg(ev).await? {
            SignalingMessage::Consumed { rtp_parameters, .. } => return Ok(rtp_parameters),
            SignalingMessage::Error { message, .. } if message == "transport_connected" => {}
            err @ SignalingMessage::Error { .. } => return Err(on_unexpected(err, "Consumed")),
            other => {
                tracing::debug!(msg = ?other, "await(consumed) 忽略无关事件，继续等");
                continue;
            }
        }
    }
}

async fn await_data_producer_created(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<String, ClientError> {
    loop {
        match next_msg(ev).await? {
            SignalingMessage::DataProducerCreated { data_producer_id, .. } => {
                return Ok(data_producer_id);
            }
            SignalingMessage::Error { message, .. } if message == "transport_connected" => {}
            err @ SignalingMessage::Error { .. } => {
                return Err(on_unexpected(err, "DataProducerCreated"));
            }
            other => {
                tracing::debug!(msg = ?other, "await(producer) 忽略无关事件，继续等");
                continue;
            }
        }
    }
}

/// S2b：consume 前查 router 真实 caps——手拼"仅 VP8"声明被 H264 producer 的
/// can_consume 直拒（5000 No compatible media codecs，09-15 活体实锤）。
async fn await_router_caps(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<serde_json::Value, ClientError> {
    match next_msg(ev).await? {
        SignalingMessage::RouterRtpCapabilities { capabilities, .. } => Ok(capabilities),
        other => Err(on_unexpected(other, "RouterRtpCapabilities")),
    }
}

// ── S2d ack 消费链路（舱端 = 车端 ack producer 的 consumer）──────────

/// 等车端 ack producer 公告（非 ack label / 自身回显跳过；`None` = 事件流终结）。
/// S4′: 途经 `ProducerClosed(Data)` 定向拆除对应本地 consumer DC（释放 libwebrtc
/// stream id 占用 = `StreamId reserved` 消费天花板的舱端解法）。
async fn await_ack_producer(
    ev: &mut broadcast::Receiver<SignalEvent>,
    self_peer: &str,
    consumers: &mut HashMap<String, mpsc::UnboundedSender<()>>,
) -> Option<String> {
    loop {
        match ev.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                peer_id,
                ..
            })) => {
                if label == sfu::ACK_LABEL && peer_id != self_peer {
                    return Some(data_producer_id);
                }
            }
            Ok(SignalEvent::Message(SignalingMessage::ProducerClosed {
                producer_id,
                kind: mediaservo_common::protocol::MediaKind::Data,
                ..
            })) => {
                if let Some(tx) = consumers.remove(&producer_id) {
                    let _ = tx.send(());
                    tracing::info!(producer_id, "ack 泵: 车端 DataProducer 死亡—本地 DC 定向拆除");
                }
            }
            Ok(SignalEvent::Message(_)) | Ok(SignalEvent::Connected { .. }) => {}
            Ok(SignalEvent::Error(e)) => tracing::warn!("ack 泵: 信令事件错误（忽略续等）: {e}"),
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("ack 泵: 队列溢出丢 {n} 事件，续");
            }
            Err(_) => return None, // Closed = 会话终结
        }
    }
}

/// 等 DataConsumed 的 S2d 参数三元组（Error 终态，其余无关事件忽略续等）。
async fn await_data_consumed(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<(SctpStreamParameters, String, String), ClientError> {
    loop {
        match next_msg(ev).await? {
            // connect ack（code=0 豁免形，同 await_transport_created——泵恰跨 connect 窗）。
            SignalingMessage::Error { ref message, .. } if message == "transport_connected" => {}
            SignalingMessage::DataConsumed { sctp_stream_parameters, label, protocol, .. } => {
                let sp = sctp_stream_parameters.ok_or_else(|| {
                    ClientError::MalformedResponse(
                        "DataConsumed 缺 sctp 参数（server 过旧，无 negotiated 依据）".into(),
                    )
                })?;
                return Ok((sp, label, protocol));
            }
            err @ SignalingMessage::Error { .. } => {
                return Err(on_unexpected(err, "DataConsumed"));
            }
            other => {
                tracing::debug!(msg = ?other, "ack 泵 await 期忽略无关事件");
                continue;
            }
        }
    }
}

/// 建 DC-only recv transport 并 connect（S1 车端 setup_recv_side 同形；negotiated
/// DC 带外协商不受「DC 先于 answer」时序合同约束）。
async fn open_dc_recv_transport(
    ev: &mut broadcast::Receiver<SignalEvent>,
    signal: &LinkSignal,
    engine: &Arc<dyn Engine>,
    room: &str,
    peer: &str,
) -> Result<(Arc<dyn PcHandle>, String), ClientError> {
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: room.to_string(),
            peer_id: peer.to_string(),
            direction: TransportDirection::Recv,
        })
        .await?;
    let (transport_id, ice, dtls, candidates) =
        tokio::time::timeout(RESPONSE_WAIT, await_transport_created(ev))
            .await
            .map_err(|_| ClientError::Timeout { what: "WebRtcTransportCreated(recv-ack)" })??;
    let pc = engine.create_pc().await?;
    let remote_sdp = sfu::build_dc_remote_sdp(&ice, &dtls, candidates.as_ref());
    // 错误串归一（原 "(recv-ack)" 局部前缀并入引擎统一映射——仅文本差，类型/路径不变）。
    pc.set_remote_offer(remote_sdp).await?;
    let answer = pc.create_answer().await?;
    pc.set_local_answer(&answer).await?;
    connect_transport(signal, room, peer, &transport_id, &pc).await?;
    Ok((pc, transport_id))
}

/// 后台泵：消费车端 ack producer → negotiated DC → ControlAck 路由进 `ack_tx`。
/// 循环等后续 announce（车端重启/多车实例自适应）；transport 首建复用。
/// `ack_tx` 全 drop（ControlChannel 释放）或事件流 Closed 时退出。
async fn ack_consumer_pump(
    mut ev: broadcast::Receiver<SignalEvent>,
    signal: Arc<LinkSignal>,
    engine: Arc<dyn Engine>,
    room: String,
    peer: String,
    ack_tx: mpsc::Sender<ControlAck>,
) {
    let mut transport: Option<(Arc<dyn PcHandle>, String)> = None;
    // S4′: dp_id → 本地 ack consumer DC 拆除通道。
    let mut consumers: HashMap<String, mpsc::UnboundedSender<()>> = HashMap::new();
    // 预订阅流与主流各见全量广播 = 开局积压混有 step1 send transport 的
    // Created/connect ack（泵直接 await 会抓错旧应答 → 对 send 槽二次 connect
    // = worker "connect() already called"，2026-09-15 活体实证）。排空积压：
    // ack announce 入 seed，其余丢弃；此后队列只剩本泵轮次的应答。
    let mut seed: Vec<String> = Vec::new();
    'drain: loop {
        match ev.try_recv() {
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                peer_id,
                ..
            })) => {
                if label == sfu::ACK_LABEL && peer_id != peer {
                    seed.push(data_producer_id);
                }
            }
            Ok(_) => {}
            Err(broadcast::error::TryRecvError::Lagged(_)) => continue 'drain,
            Err(broadcast::error::TryRecvError::Empty) => break 'drain,
            Err(broadcast::error::TryRecvError::Closed) => return,
        }
    }
    while !ack_tx.is_closed() {
        let dp_id = match seed.pop() {
            Some(dp) => dp,
            None => {
                let Some(dp) = await_ack_producer(&mut ev, &peer, &mut consumers).await else {
                    return;
                };
                dp
            }
        };
        tracing::info!(data_producer_id = %dp_id, "ack 泵: 发现车端 DataProducer，消费");
        let (pc, tid) = match transport.clone() {
            Some(t) => t,
            None => match open_dc_recv_transport(&mut ev, &signal, &engine, &room, &peer).await {
                Ok(t) => {
                    transport = Some(t.clone());
                    t
                }
                Err(e) => {
                    tracing::error!("ack 泵: recv transport 建立失败: {e}");
                    continue;
                }
            },
        };
        if let Err(e) = signal
            .send(SignalingMessage::ConsumeData {
                room_id: room.clone(),
                peer_id: peer.clone(),
                transport_direction: TransportDirection::Recv,
                data_producer_id: dp_id.clone(),
                transport_id: Some(tid),
            })
            .await
        {
            tracing::error!("ack 泵: ConsumeData 发送失败: {e}");
            continue;
        }
        let sp = match tokio::time::timeout(RESPONSE_WAIT, await_data_consumed(&mut ev)).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                tracing::error!("ack 泵: DataConsumed 异常: {e}");
                continue;
            }
            Err(_) => {
                tracing::error!("ack 泵: DataConsumed 等待超时");
                continue;
            }
        };
        let (sp, label, protocol) = sp;
        match pc.create_data_channel(&label, sfu::negotiated_init(&sp, &protocol)).await {
            Ok(dc) => {
                tracing::info!(
                    label,
                    stream_id = sp.stream_id,
                    "ack 泵: negotiated consumer DC 已建，回执接入 recv_ack"
                );
                let tx = ack_tx.clone();
                let mut rx = dc.events().await;
                let (purge_tx, mut purge_rx) = mpsc::unbounded_channel();
                consumers.insert(dp_id.clone(), purge_tx);
                tokio::spawn(async move {
                    let _dc = dc; // 通道生命周期锚（Arc drop 即关）
                    loop {
                        tokio::select! {
                            item = rx.recv() => match item {
                                Some(EngineDcEvent::Message(data)) => {
                                    match serde_json::from_slice::<ControlAck>(&data) {
                                        Ok(ack) => {
                                            if tx.send(ack).await.is_err() {
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("ack DC 非 ControlAck 载荷丢弃: {e}")
                                        }
                                    }
                                }
                                Some(EngineDcEvent::Closed) => break,
                                Some(_) => {}
                                None => break,
                            },
                            // S4′: 车端 ack dp 死亡（泵定向）→ 本地关闭释放 sid 占用。
                            _ = purge_rx.recv() => {
                                tracing::info!("ack 泵: 本地 consumer DC 自拆（车端 DataProducer 死亡）");
                                _dc.close().await;
                                break;
                            }
                        }
                    }
                });
            }
            Err(e) => tracing::error!("ack 泵: negotiated DC 创建失败: {e}"),
        }
    }
}

/// 取下一条信令消息（断流/断链/非消息事件报错；Error 帧由调用方分类）。
async fn next_msg(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<SignalingMessage, ClientError> {
    match ev.recv().await {
        Ok(SignalEvent::Message(msg)) => Ok(msg),
        Ok(SignalEvent::Disconnected { reason }) => {
            Err(ClientError::InvalidState(format!("等待期信令断开: {reason}")))
        }
        Ok(SignalEvent::Error(e)) => {
            Err(ClientError::Signal(mediaservo_link::LinkError::Signal(e)))
        }
        Ok(SignalEvent::Connected { .. }) => {
            Err(ClientError::InvalidState("等待期连接事件（resume 重挂未支持）".into()))
        }
        Ok(_) => Err(ClientError::InvalidState("未知信令事件".into())),
        Err(_) => Err(ClientError::InvalidState("信令事件流关闭".into())),
    }
}

/// 等待目标消息外的事件：Error{code} → typed 终态，其余 = 协议错位。
fn on_unexpected(msg: SignalingMessage, want: &str) -> ClientError {
    match msg {
        SignalingMessage::Error { code, message } => from_wire_error(code, &message),
        other => ClientError::MalformedResponse(format!("期望 {want}, got {other:?}")),
    }
}

/// FrameSink → bounded mpsc（满则丢 = latest 语义，同 field PullFrameSink）。
struct FrameChanSink {
    tx: mpsc::Sender<VideoFrame>,
}

impl mediaservo_webrtc::track::FrameSink for FrameChanSink {
    fn on_frame(&self, data: &[u8], width: u32, height: u32) {
        let _ = self.tx.try_send(VideoFrame { width, height, data: data.to_vec(), ts_us: 0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_webrtc::stats::{RTCInboundRtpStreamStats, RTCStats};

    fn mk(bytes: u64, w: u32, fps: f64) -> RTCStats {
        RTCStats::InboundRtp(RTCInboundRtpStreamStats {
            id: "x".into(),
            timestamp: 0.0,
            ssrc: 1,
            kind: "video".into(),
            packets_received: bytes,
            packets_lost: bytes,
            bytes_received: bytes,
            frames_decoded: 10,
            frame_width: w,
            frame_height: 720,
            frames_per_second: fps,
            jitter: 0.0,
            frame_dropped: 0,
            nack_count: 0,
            pli_count: 0,
            fir_count: 0,
        })
    }

    #[test]
    fn fold_inbound_unions_new_w3c_fields() {
        // 09-24 扩面语义钉：jitter 多路取 max，dropped/nack/pli/fir 求和
        let mut mk2 = mk(100, 1280, 30.0);
        if let RTCStats::InboundRtp(r) = &mut mk2 {
            r.jitter = 0.004;
            r.frame_dropped = 7;
            r.nack_count = 5;
            r.pli_count = 2;
            r.fir_count = 1;
        }
        let mut mk3 = mk(100, 1280, 30.0);
        if let RTCStats::InboundRtp(r) = &mut mk3 {
            r.jitter = 0.009;
            r.frame_dropped = 3;
            r.nack_count = 4;
        }
        let out = fold_inbound_stats(vec![mk2, mk3]);
        assert!((out.jitter - 0.009).abs() < 1e-9, "jitter=max");
        assert_eq!(out.frame_dropped, 10);
        assert_eq!(out.nack_count, 9);
        assert_eq!(out.pli_count, 2);
        assert_eq!(out.fir_count, 1);
    }

    #[test]
    fn fold_inbound_sums_counts_maxes_size_and_fps() {
        let out = fold_inbound_stats(vec![mk(100, 1280, 24.0), mk(50, 1920, 30.0)]);
        assert_eq!(out.bytes_received, 150);
        assert_eq!(out.packets_lost, 150);
        assert_eq!(out.frames_decoded, 20);
        assert_eq!(out.frame_width, 1920);
        assert_eq!(out.frames_per_second, 30.0);
        assert_eq!(fold_inbound_stats(vec![]), VideoStreamStats::default());
    }
}
