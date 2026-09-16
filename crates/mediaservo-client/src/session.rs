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
    ControlAck, DtlsParameters, Fingerprint, IceCandidate, IceParameters, MediaKind,
    SctpStreamParameters, SignalingMessage, TransportDirection, ControlEnvelope};
use mediaservo_link::{SignalClient, SignalEvent};
use mediaservo_webrtc::data_channel::RTCDataChannelEvent;
use mediaservo_webrtc::rtp::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnection,
    RTCPeerConnectionFactory, TrackKind, TrackRef,
};
use tokio::sync::{Mutex, broadcast, mpsc};

use crate::config::{sfu_peer_key, ClientConfig};
use crate::control::ControlChannel;
use crate::error::{classify_link_error, from_wire_error, ClientError};
use crate::sfu;
use crate::signal::{LinkSignal, Signal};

/// 控制域能力门（I5 首用户，S0 契约）：CreateDataProducer 的 can_control 门
/// 自方言 2 起生效——低于 2 本地预拒，不发请求（省一次 4012 往返）。
const CONTROL_MIN_PROTOCOL: u32 = 2;
/// CreateDataProducer 子协议（host controller DATA_PROTOCOL 同值）。
const DATA_PROTOCOL: &str = "sctp";
/// server 响应等待窗（transport/consumed/producer）。
const RESPONSE_WAIT: Duration = Duration::from_secs(10);

/// 解码视频帧（I420，libwebrtc 侧渲染前格式；C5 边界语义）。
/// inbound-rtp 折叠规则单一落点（求和/max 混排——自由函数形单测可钉，免触真 pc）。
fn fold_inbound_stats(items: Vec<mediaservo_webrtc::stats::RTCStats>) -> VideoStreamStats {
    use mediaservo_webrtc::stats::RTCStats;
    let mut out = VideoStreamStats::default();
    for st in items {
        if let RTCStats::InboundRtp(r) = st {
            out.bytes_received += r.bytes_received;
            out.packets_received += r.packets_received;
            out.packets_lost += r.packets_lost;
            out.frames_decoded += u64::from(r.frames_decoded);
            out.frame_width = out.frame_width.max(r.frame_width);
            out.frame_height = out.frame_height.max(r.frame_height);
            out.frames_per_second = out.frames_per_second.max(r.frames_per_second);
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
pub struct RoomSession {
    /// S2d: ack 消费后台泵与前台共用信令面 = Arc 共享。
    signal: Arc<LinkSignal>,
    /// S4/T3.5: 急停 HMAC 密钥（emergency_stop 签名用；None = 不签）。
    hmac_key: Option<String>,
    /// connect 即刻订阅（broadcast 无历史重放——接住 join 时 server 回放的
    /// late-join NewProducer）。
    events: Mutex<broadcast::Receiver<SignalEvent>>,
    /// S2d: open_control 专用第二流（connect 同刻订阅 = join 回放零缺口；
    /// take 后移交 ack 泵，每会话一次性）。
    pump_events: Mutex<Option<broadcast::Receiver<SignalEvent>>>,
    /// consume 建立的 recv PC 保活（句柄即生命周期）。
    _pcs: Vec<RTCPeerConnection>,
}

impl RoomSession {
    /// S2c 诊断面：video receiver 的 inbound-rtp stats（"包没进来" vs
    /// "进而不解" 二分的定案读点；track_id = offer msid 注入的 "video"）。
    #[must_use]
    pub fn video_receiver_stats(&self) -> Vec<mediaservo_webrtc::stats::RTCStats> {
        self._pcs
            .iter()
            .flat_map(|pc| pc.receiver_get_stats("video"))
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
        let mut env = ControlEnvelope {
            seq,
            cmd: "estop".to_string(),
            payload,
            sig: None,
        };
        if let Some(k) = &self.hmac_key {
            env.sig = Some(mediaservo_common::protocol::control_hmac_sign(k, &env));
        }
        ctl.send_envelope(label, &env).await?; // DC 快路径先行（急停语义 = 不等审计副本）
        let _ = self
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
    pub async fn connect(cfg: &ClientConfig) -> Result<Self, ClientError> {
        let mut client = SignalClient::new(
            &cfg.signaling_url,
            cfg.psk.as_deref().unwrap_or(""),
            &cfg.room_id,
            cfg.role.clone(),
        );
        if let Some(jwt) = &cfg.jwt {
            client = client.with_jwt(jwt.clone());
        }
        let session = client.connect().await.map_err(classify_link_error)?;
        let events = session.events();
        // S2d: 第二流与主流同刻订阅（LinkSignal::events 在 async 态 blocking_lock
        // 会 panic——订阅必须在此同步点完成）。
        let pump_events = session.events();
        let signal = LinkSignal::new(session, sfu_peer_key(&cfg.role).to_string());
        Ok(Self {
            signal: Arc::new(signal),
            hmac_key: cfg.hmac_key.clone(),
            events: Mutex::new(events),
            pump_events: Mutex::new(Some(pump_events)),
            _pcs: Vec::new(),
        })
    }

    /// 谈成的方言版本（S0；旧 server = 1）。
    #[must_use]
    pub fn negotiated(&self) -> u32 {
        self.signal.negotiated()
    }

    #[must_use]
    pub fn room_id(&self) -> &str {
        self.signal.room_id()
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
        .map_err(|_| ClientError::Timeout {
            what: "video NewProducer",
        })?
    }

    /// 订阅一路视频 producer：Recv transport → Consume（拿 ssrc）→ answerer
    /// 协商 → Connect → on_track 帧流。镜像 field PullSession::subscribe。
    pub async fn consume_video(
        &mut self,
        producer_id: &str,
    ) -> Result<mpsc::Receiver<VideoFrame>, ClientError> {
        let mut ev = self.events.lock().await;
        let room = self.signal.room_id().to_string();
        let peer = self.signal.sfu_peer_id().to_string();

        // 1. Recv transport
        self.signal
            .send(SignalingMessage::CreateWebRtcTransport {
                room_id: room.clone(),
                peer_id: peer.clone(),
                direction: TransportDirection::Recv,
            })
            .await?;
        let (transport_id, ice, dtls, candidates) =
            tokio::time::timeout(RESPONSE_WAIT, await_transport_created(&mut ev))
                .await
                .map_err(|_| ClientError::Timeout {
                    what: "WebRtcTransportCreated(recv)",
                })??;

        // 2. PC（mediasoup ICE-Lite，无 STUN）+ on_track 先于 set_remote（晚注册丢首 track）
        let pc = create_pc().await?;
        let (frame_tx, frame_rx) = mpsc::channel::<VideoFrame>(3);
        pc.on_track(move |receiver| {
            // S2c 诊断（常久日志）：on_track 到达 = RTP 已 demux 成 track；
            // 「ICE/DTLS 通过但零帧」的分水岭判据。
            tracing::info!(kind = ?receiver.kind, track_id = ?receiver.track_id, "consume on_track 到达");
            if let TrackRef::Receiver(r) = receiver.track {
                r.set_frame_sink(Box::new(FrameChanSink {
                    tx: frame_tx.clone(),
                }));
            }
        });

        // 3a. Router 能力查询直传（mediasoup-client Device.load 同款官方流程，C18；
        //     手拼 caps 在 H264 producer 下必拒——S2b 活体教训，PullSession 同罪另案）。
        self.signal
            .send(SignalingMessage::GetRouterRtpCapabilities { room_id: room.clone() })
            .await?;
        let router_caps = tokio::time::timeout(RESPONSE_WAIT, await_router_caps(&mut ev))
            .await
            .map_err(|_| ClientError::Timeout {
                what: "RouterRtpCapabilities",
            })??;

        // 3b. Consume（C1 显式绑 transport_id；caps = router 原样回包）
        self.signal
            .send(SignalingMessage::Consume {
                room_id: room.clone(),
                peer_id: peer.clone(),
                producer_id: producer_id.to_string(),
                rtp_capabilities: router_caps,
                transport_id: Some(transport_id.clone()),
            })
            .await?;
        let consumer_rtp = tokio::time::timeout(RESPONSE_WAIT, await_consumed(&mut ev))
            .await
            .map_err(|_| ClientError::Timeout {
                what: "Consumed",
            })??;

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
        )
        .map_err(|e| ClientError::WebRtc(format!("add_transceiver: {e}")))?;
        tracing::debug!(remote_sdp = %remote_sdp, "consume remote offer（SSRC 注入后）");
        pc.set_remote_description(&RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp))
            .await
            .map_err(|e| ClientError::WebRtc(format!("set_remote_description: {e}")))?;
        let answer = pc
            .create_answer(&RTCAnswerOptions)
            .await
            .map_err(|e| ClientError::WebRtc(format!("create_answer: {e}")))?;
        pc.set_local_description(&answer)
            .await
            .map_err(|e| ClientError::WebRtc(format!("set_local_description: {e}")))?;

        // 6. Connect（本地 DTLS 指纹，role=client）
        connect_transport(&*self.signal, &room, &peer, &transport_id, &pc).await?;

        self._pcs.push(pc);
        tracing::info!(producer_id, transport_id = %transport_id, "client consume_video 建立");
        Ok(frame_rx)
    }

    /// 开出程控制通道集（host S1 镜像）。前置 I5 门：方言 ≥2，否则本地预拒。
    pub async fn open_control(&mut self, labels: &[&str]) -> Result<ControlChannel, ClientError> {
        let got = self.signal.negotiated();
        if got < CONTROL_MIN_PROTOCOL {
            return Err(ClientError::ProtocolTooLow {
                need: CONTROL_MIN_PROTOCOL,
                got,
            });
        }
        let mut ev = self.events.lock().await;
        let room = self.signal.room_id().to_string();
        let peer = self.signal.sfu_peer_id().to_string();
        // S2d: 取 connect 同刻预订阅的第二流（建立期车端 ack announce 零缺口）。
        let pump_ev = self
            .pump_events
            .lock()
            .await
            .take()
            .ok_or_else(|| ClientError::InvalidState("控制面已开启（ack 泵每会话一次）".into()))?;

        // 1. Send transport
        self.signal
            .send(SignalingMessage::CreateWebRtcTransport {
                room_id: room.clone(),
                peer_id: peer.clone(),
                direction: TransportDirection::Send,
            })
            .await?;
        let (transport_id, ice, dtls, candidates) =
            tokio::time::timeout(RESPONSE_WAIT, await_transport_created(&mut ev))
                .await
                .map_err(|_| ClientError::Timeout {
                    what: "WebRtcTransportCreated(send)",
                })??;

        // 2. PC + 合成 offer set_remote；DC 必须先于 answer 创建（S1 时序合同）
        let pc = create_pc().await?;
        let remote_sdp = sfu::build_dc_remote_sdp(&ice, &dtls, candidates.as_ref());
        pc.set_remote_description(&RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp))
            .await
            .map_err(|e| ClientError::WebRtc(format!("set_remote_description: {e}")))?;

        // 3. 每 label 出程 DC（mediasoup 单向对模型：producer DC 无入程，回执
        //    回程 = step 6 ack 消费链路——2026-09-15 活体证 consumer 反向不透传）。
        let (ack_tx, ack_rx) = mpsc::channel::<ControlAck>(32);
        let mut dcs = HashMap::new();
        let mut order = Vec::with_capacity(labels.len());
        for label in labels {
            let dc = pc
                .create_data_channel(label, sfu::channel_init(label))
                .await
                .map_err(|e| ClientError::WebRtc(format!("create_data_channel {label}: {e}")))?;
            dcs.insert(label.to_string(), dc);
            order.push(label.to_string());
        }

        // 4. answer/connect 收口
        let answer = pc
            .create_answer(&RTCAnswerOptions)
            .await
            .map_err(|e| ClientError::WebRtc(format!("create_answer: {e}")))?;
        pc.set_local_description(&answer)
            .await
            .map_err(|e| ClientError::WebRtc(format!("set_local_description: {e}")))?;
        connect_transport(&*self.signal, &room, &peer, &transport_id, &pc).await?;

        // 5. 逐 DC CreateDataProducer announce（4012 → ControlDenied 终态）
        let mut producer_ids = Vec::with_capacity(order.len());
        for label in &order {
            let dc = &dcs[label];
            self.signal
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
                .map_err(|_| ClientError::Timeout {
                    what: "DataProducerCreated",
                })??;
            tracing::info!(label = %label, data_producer_id = %dp, "client DataProducer 已建立");
            producer_ids.push(dp);
        }

        // 6. S2d 官方单向对模型：后台消费车端 label=ack DataProducer，negotiated
        //    consumer DC 的 ControlAck 路由进 ack_rx（recv_ack 的正式供数来源）。
        tokio::spawn(ack_consumer_pump(pump_ev, self.signal.clone(), room, peer, ack_tx));

        Ok(ControlChannel::new(dcs, order, ack_rx, producer_ids, pc))
    }

    /// 关闭会话（释放 WS + 后台任务；帧/ack 流随 drop 收敛）。
    pub async fn close(self) -> Result<(), ClientError> {
        self.signal.close().await
    }
}

async fn create_pc() -> Result<RTCPeerConnection, ClientError> {
    let factory = RTCPeerConnectionFactory::new();
    factory
        .create_peer_connection(RTCConfiguration {
            ice_servers: Vec::<RTCIceServer>::new(),
            ice_transport_type: RTCIceTransportPolicy::All,
        })
        .await
        .map_err(|e| ClientError::WebRtc(format!("create_peer_connection: {e}")))
}

async fn connect_transport(
    signal: &dyn Signal,
    room: &str,
    peer: &str,
    transport_id: &str,
    pc: &RTCPeerConnection,
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
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".to_string(),
                    value: fp_hex,
                }],
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
            err @ SignalingMessage::Error { .. } => return Err(on_unexpected(err, "WebRtcTransportCreated")),
            other => {
                tracing::debug!(msg = ?other, "await(transport) 忽略无关事件，继续等");
                continue;
            }
        }
    }
}

async fn await_consumed(ev: &mut broadcast::Receiver<SignalEvent>) -> Result<serde_json::Value, ClientError> {
    loop {
        match next_msg(ev).await? {
            SignalingMessage::Consumed {
                rtp_parameters, ..
            } => return Ok(rtp_parameters),
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
            SignalingMessage::DataProducerCreated {
                data_producer_id,
                ..
            } => return Ok(data_producer_id),
            SignalingMessage::Error { message, .. } if message == "transport_connected" => {}
            err @ SignalingMessage::Error { .. } => return Err(on_unexpected(err, "DataProducerCreated")),
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
            SignalingMessage::DataConsumed {
                sctp_stream_parameters,
                label,
                protocol,
                ..
            } => {
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
    room: &str,
    peer: &str,
) -> Result<(RTCPeerConnection, String), ClientError> {
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
            .map_err(|_| ClientError::Timeout {
                what: "WebRtcTransportCreated(recv-ack)",
            })??;
    let pc = create_pc().await?;
    let remote_sdp = sfu::build_dc_remote_sdp(&ice, &dtls, candidates.as_ref());
    pc.set_remote_description(&RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp))
        .await
        .map_err(|e| ClientError::WebRtc(format!("set_remote(recv-ack): {e}")))?;
    let answer = pc
        .create_answer(&RTCAnswerOptions)
        .await
        .map_err(|e| ClientError::WebRtc(format!("create_answer(recv-ack): {e}")))?;
    pc.set_local_description(&answer)
        .await
        .map_err(|e| ClientError::WebRtc(format!("set_local(recv-ack): {e}")))?;
    connect_transport(signal, room, peer, &transport_id, &pc).await?;
    Ok((pc, transport_id))
}

/// 后台泵：消费车端 ack producer → negotiated DC → ControlAck 路由进 `ack_tx`。
/// 循环等后续 announce（车端重启/多车实例自适应）；transport 首建复用。
/// `ack_tx` 全 drop（ControlChannel 释放）或事件流 Closed 时退出。
async fn ack_consumer_pump(
    mut ev: broadcast::Receiver<SignalEvent>,
    signal: Arc<LinkSignal>,
    room: String,
    peer: String,
    ack_tx: mpsc::Sender<ControlAck>,
) {
    let mut transport: Option<(RTCPeerConnection, String)> = None;
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
            None => match open_dc_recv_transport(&mut ev, &signal, &room, &peer).await {
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
        match pc
            .create_data_channel(&label, sfu::negotiated_init(&sp, &protocol))
            .await
        {
            Ok(dc) => {
                tracing::info!(
                    label,
                    stream_id = sp.stream_id,
                    "ack 泵: negotiated consumer DC 已建，回执接入 recv_ack"
                );
                let tx = ack_tx.clone();
                let mut rx = dc.spool().await;
                let (purge_tx, mut purge_rx) = mpsc::unbounded_channel();
                consumers.insert(dp_id.clone(), purge_tx);
                tokio::spawn(async move {
                    let mut _dc = dc; // 通道生命周期锚（drop 即关）
                    loop {
                        tokio::select! {
                            item = rx.recv() => match item {
                                Some(RTCDataChannelEvent::Message(m)) => {
                                    match serde_json::from_slice::<ControlAck>(&m.data) {
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
                                Some(RTCDataChannelEvent::Closed) => break,
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
async fn next_msg(ev: &mut broadcast::Receiver<SignalEvent>) -> Result<SignalingMessage, ClientError> {
    match ev.recv().await {
        Ok(SignalEvent::Message(msg)) => Ok(msg),
        Ok(SignalEvent::Disconnected { reason }) => Err(ClientError::InvalidState(format!(
            "等待期信令断开: {reason}"
        ))),
        Ok(SignalEvent::Error(e)) => {
            Err(ClientError::Signal(mediaservo_link::LinkError::Signal(e)))
        }
        Ok(SignalEvent::Connected { .. }) => Err(ClientError::InvalidState(
            "等待期连接事件（resume 重挂未支持）".into(),
        )),
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
        let _ = self.tx.try_send(VideoFrame {
            width,
            height,
            data: data.to_vec(),
            ts_us: 0,
        });
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
        })
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
