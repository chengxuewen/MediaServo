//! 房间会话——link 信令之上的 SFU 消费 + 控制出程（S2 核心）。
//!
//! consume 序列本地镜像 `mediaservo-field::PullSession::subscribe` 形状
//! （C21 依赖边界：client 禁依 field）；控制序列镜像 `mediaservo-host::controller`
//! S1 出程形（Send transport → DC 先于 answer → Connect → CreateDataProducer）。
//! ICE-Lite：候选随 WebRtcTransportCreated 内联，无 candidate 交换回合；
//! `Error{code:0,"transport_connected"}` 是 server 惯例 ack，非真错误。

use std::collections::HashMap;
use std::time::Duration;

use mediaservo_common::protocol::{
    ControlAck, DtlsParameters, Fingerprint, IceCandidate, IceParameters, MediaKind,
    SignalingMessage, TransportDirection,
};
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
    signal: LinkSignal,
    /// connect 即刻订阅（broadcast 无历史重放——接住 join 时 server 回放的
    /// late-join NewProducer）。
    events: Mutex<broadcast::Receiver<SignalEvent>>,
    /// consume 建立的 recv PC 保活（句柄即生命周期）。
    _pcs: Vec<RTCPeerConnection>,
}

impl RoomSession {
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
        let signal = LinkSignal::new(session, sfu_peer_key(&cfg.role).to_string());
        Ok(Self {
            signal,
            events: Mutex::new(events),
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
        connect_transport(&self.signal, &room, &peer, &transport_id, &pc).await?;

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

        // 3. 每 label create_data_channel + 回声回执路由（非 ControlAck 载荷 warn 丢弃）
        let (ack_tx, ack_rx) = mpsc::channel::<ControlAck>(32);
        let mut dcs = HashMap::new();
        let mut order = Vec::with_capacity(labels.len());
        for label in labels {
            let dc = pc
                .create_data_channel(label, sfu::channel_init(label))
                .await
                .map_err(|e| ClientError::WebRtc(format!("create_data_channel {label}: {e}")))?;
            let tx = ack_tx.clone();
            let mut rx = dc.spool().await;
            let lbl = label.to_string();
            tokio::spawn(async move {
                while let Some(item) = rx.recv().await {
                    match item {
                        RTCDataChannelEvent::Message(m) => {
                            match serde_json::from_slice::<ControlAck>(&m.data) {
                                Ok(ack) => {
                                    if tx.send(ack).await.is_err() {
                                        break; // 句柄已 drop
                                    }
                                }
                                Err(e) => tracing::warn!(label = %lbl, "DC 非 ControlAck 载荷丢弃: {e}"),
                            }
                        }
                        RTCDataChannelEvent::Closed => break,
                        RTCDataChannelEvent::Open | RTCDataChannelEvent::Error(_) => {}
                    }
                }
            });
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
        connect_transport(&self.signal, &room, &peer, &transport_id, &pc).await?;

        // 5. 逐 DC CreateDataProducer announce（4012 → ControlDenied 终态）
        drop(ack_tx); // 仅路由任务持发送端
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
            other => return Err(on_unexpected(other, "WebRtcTransportCreated")),
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
            other => return Err(on_unexpected(other, "Consumed"))?,
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
            other => return Err(on_unexpected(other, "DataProducerCreated"))?,
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
