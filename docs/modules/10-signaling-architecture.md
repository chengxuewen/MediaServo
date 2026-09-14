# 10. 信令架构

> 版本: 0.1 | 日期: 2026-07-16 | Phase: 0
> 关联决策: D51-D54, D74（信令协议） + 主文档引用: [架构文档](../architecture.md) §8

## 10.1 核心原则

**信令 = 不透明消息中继**。信令层绝不解析 SDP/ICE 内容 — SDP/ICE 状态机属于 `MediaTransport`。信令层只负责：

1. 认证连接
2. 路由消息（按 ConnId 键）
3. 感知拓扑转发（P2P 单播、SFU 广播、PubSub 按订阅）

## 10.2 架构分层

```
┌───────────────────────────────────┐
│  Phase 2: RoomRouter             │  拓扑感知路由（P2P/SFU/PubSub）
│  create_room / route_message     │
├───────────────────────────────────┤
│  Phase 1: SignalHandler          │  消息中继 + 连接配对
│  accept / handle_input / close   │
├───────────────────────────────────┤
│  Transport: axum WebSocket       │  由框架管理，不定义 trait
└───────────────────────────────────┘

        SignalHandler operates sans-I/O
        ┌─────────────────────────┐
        │  handle_input(conn, msg) │ ← 收到消息
        │  → Vec<SignalOutput>     │ ← 产出路由指令
        │                         │
        │  poll_output()           │ ← 定时心跳/超时
        │  → Vec<SignalOutput>     │
        └─────────────────────────┘
                 │
                 ▼
        SignalOutput { target, msg }
        target: Direct(ConnId) | Room(RoomId) | Broadcast
```

## 10.3 SignalHandler Trait（Phase 1）

```rust
use std::collections::HashMap;

/// 连接标识符（axum WebSocket 分配）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(u64);

/// 信令消息（客户端↔服务端）
#[derive(Debug, Clone)]
pub enum SignalingMessage {
    JoinRoom(JoinRoomRequest),
    LeaveRoom(LeaveRoomRequest),
    SdpOffer(SdpMessage),
    SdpAnswer(SdpMessage),
    RTCIceCandidate(IceCandidateMessage),
    PublishTrack(PublishTrackRequest),
    SubscribeTrack(SubscribeTrackRequest),
    UnsubscribeTrack(UnsubscribeTrackRequest),
    RTCDataChannel(DataChannelSignal),
    Ping,
    Error(ErrorInfo),
}

/// 路由输出
#[derive(Debug, Clone)]
pub struct SignalOutput {
    pub target: SignalTarget,
    pub message: SignalingMessage,
}

#[derive(Debug, Clone)]
pub enum SignalTarget {
    /// 直接发给指定连接
    Direct(ConnId),
    /// 发给房间内所有参与者（不含发送者）
    Room(RoomId, ExcludeSender),
    /// 发给指定连接列表
    Multi(Vec<ConnId>),
}

#[derive(Debug, Clone)]
pub enum ExcludeSender {
    Yes,
    No,
}

/// sans-I/O 信令处理器
pub trait SignalHandler: Send {
    /// 接受新连接（认证通过后调用）
    fn accept(&mut self, conn_id: ConnId, auth: &[u8]) -> Result<Vec<SignalOutput>>;

    /// 处理接收到的消息
    fn handle_input(&mut self, conn_id: ConnId, msg: SignalingMessage) -> Result<Vec<SignalOutput>>;

    /// 处理连接关闭
    fn handle_close(&mut self, conn_id: ConnId) -> Result<Vec<SignalOutput>>;

    /// 定时轮询（心跳/超时）
    fn poll_output(&mut self) -> Result<Vec<SignalOutput>>;
}
```

## 10.4 Phase 1 实现：配对中继

Phase 1 遥控座舱是 1:1 P2P。最简实现：

```rust
pub struct PairingRelay {
    pairs: HashMap<ConnId, ConnId>,  // A → B 映射
    pending: HashMap<String, ConnId>, // token → 等待配对的连接
}

impl SignalHandler for PairingRelay {
    fn accept(&mut self, conn_id: ConnId, auth: &[u8]) -> Result<Vec<SignalOutput>> {
        let token = parse_token(auth)?;
        if let Some(&peer) = self.pending.remove(&token) {
            // 第二个连接到达 → 配对完成
            self.pairs.insert(conn_id, peer);
            self.pairs.insert(peer, conn_id);
            // 通知双方
            Ok(vec![
                SignalOutput { target: SignalTarget::Direct(conn_id), message: SignalingMessage::JoinAccepted },
                SignalOutput { target: SignalTarget::Direct(peer), message: SignalingMessage::ParticipantJoined },
            ])
        } else {
            // 第一个连接到达 → 等待配对
            self.pending.insert(token, conn_id);
            Ok(vec![])
        }
    }

    fn handle_input(&mut self, conn_id: ConnId, msg: SignalingMessage) -> Result<Vec<SignalOutput>> {
        let &peer = self.pairs.get(&conn_id)
            .ok_or_else(|| anyhow::anyhow!("no peer for {:?}", conn_id))?;
        Ok(vec![SignalOutput {
            target: SignalTarget::Direct(peer),
            message: msg,  // 原样转发，不解析
        }])
    }

    fn handle_close(&mut self, conn_id: ConnId) -> Result<Vec<SignalOutput>> {
        if let Some(&peer) = self.pairs.remove(&conn_id) {
            self.pairs.remove(&peer);
            Ok(vec![SignalOutput {
                target: SignalTarget::Direct(peer),
                message: SignalingMessage::Error(ErrorInfo { code: "PEER_DISCONNECTED".into(), message: ".".into() }),
            }])
        } else {
            Ok(vec![])
        }
    }

    fn poll_output(&mut self) -> Result<Vec<SignalOutput>> {
        Ok(vec![])  // Phase 1: 无心跳
    }
}
```

## 10.5 Protobuf 消息定义

客户端↔服务端使用**分离枚举**（非共享 oneof）：

```protobuf
syntax = "proto3";
package mediaservo.signaling;

// === 客户端 → 服务端 ===
message ClientSignal {
  oneof body {
    JoinRoomRequest join_room = 1;
    SdpMessage sdp_offer = 2;
    SdpMessage sdp_answer = 3;
    IceCandidateMessage ice_candidate = 4;
    TrackRequest publish_track = 5;
    TrackRequest subscribe_track = 6;
    TrackRequest unsubscribe_track = 7;
    DataChannelSignal data_channel = 8;
    Empty ping = 9;
  }
}

// === 服务端 → 客户端 ===
message ServerSignal {
  oneof body {
    JoinResponse join_accepted = 1;
    JoinResponse join_rejected = 2;
    SdpMessage remote_offer = 3;
    SdpMessage remote_answer = 4;
    IceCandidateMessage remote_ice = 5;
    ParticipantInfo participant_joined = 6;
    ParticipantInfo participant_left = 7;
    TrackInfo track_published = 8;
    TrackInfo track_unpublished = 9;
    DataChannelSignal data_channel = 10;
    Empty pong = 11;
    ErrorInfo error = 12;
  }
}

message SdpMessage {
  string sdp_type = 1;  // "offer" | "answer"
  string sdp = 2;
}

message IceCandidateMessage {
  string candidate = 1;
  string sdp_mid = 2;
  int32 sdp_m_line_index = 3;
}

message TrackRequest {
  string track_id = 1;
  string kind = 2;  // "audio" | "video"
}

message DataChannelSignal {
  string label = 1;
  bool ordered = 2;
  sint32 max_retransmits = 3;  // -1 = reliable
}

message JoinRoomRequest {
  string room_id = 1;
  string token = 2;
}

message JoinResponse {
  string room_id = 1;
  string participant_id = 2;
}

message ParticipantInfo {
  string participant_id = 1;
  repeated string track_ids = 2;
}

message TrackInfo {
  string track_id = 1;
  string kind = 2;
  string participant_id = 3;
}

message ErrorInfo {
  string code = 1;
  string message = 2;
}

message Empty {}
```

**设计理由**：
- 分离枚举保证类型安全 — 浏览器不会发送 `ParticipantJoined`，服务端不会发送 `JoinRoom`
- 每个 message 独立定义，复用性强（`SdpMessage` 同时用于 offer 和 answer）
- `DataChannelSignal` 独立于 SDP — RTCDataChannel 是带内 SCTP 创建，信令只传达意图
- `TrackRequest` 三合一 — publish/subscribe/unsubscribe 共享字段，通过枚举值区分语义

**传输格式**：WebSocket binary 帧 = protobuf，WebSocket text 帧 = JSON（调试用，由同一个 protobuf schema 的 serde 派生）

## 10.6 sans-I/O 运行循环

与 `MediaTransport` 一致的不变式：**每次变更后必须完全排空 poll_output 再进行下一次变更**。

```rust
// 服务端 WebSocket 事件循环
loop {
    select! {
        // 通道 1: 收到客户端消息
        msg = ws_rx.recv() => {
            let outputs = signal_handler.handle_input(conn_id, msg)?;
            drain_outputs(&mut signal_handler, &ws_senders, outputs).await;
        }
        // 通道 2: 定时器（心跳）
        _ = tick.tick() => {
            let outputs = signal_handler.poll_output()?;
            drain_outputs(&mut signal_handler, &ws_senders, outputs).await;
        }
    }
}

async fn drain_outputs(
    handler: &mut dyn SignalHandler,
    senders: &HashMap<ConnId, WsSender>,
    mut outputs: Vec<SignalOutput>,
) -> Result<()> {
    while !outputs.is_empty() {
        for out in &outputs {
            match out.target {
                SignalTarget::Direct(conn_id) => {
                    senders[&conn_id].send(out.message.clone()).await?;
                }
                SignalTarget::Room(room_id, exclude) => {
                    for (conn_id, sender) in senders {
                        if exclude == ExcludeSender::Yes && conn_id == sender_conn_id { continue; }
                        sender.send(out.message.clone()).await?;
                    }
                }
                SignalTarget::Multi(ref ids) => {
                    for &conn_id in ids {
                        senders[&conn_id].send(out.message.clone()).await?;
                    }
                }
            }
        }
        // poll_output 可能有级联输出（例如 handle_close 触发需要重连）
        outputs = handler.poll_output()?;
    }
}
```

## 10.7 Phase 2 扩展：RoomRouter

当需要 N:M 多方会议时，提取 RoomRouter trait：

```rust
#[derive(Debug, Clone)]
pub enum RoomTopology {
    PeerToPeer,
    /// mediasoup SFU relay (Phase 2) — 信令层仅管理 SDP 交换,
    /// 媒体 plane 由 mediasoup Worker 独立处理，信令层不解析媒体内容。
    SfuRelay(SfuConfig),
    PublishSubscribe(PubSubConfig),
}

pub trait RoomRouter: Send {
    fn create_room(&mut self, config: RoomConfig) -> Result<(RoomId, Vec<SignalOutput>)>;
    fn join_room(&mut self, room_id: RoomId, conn_id: ConnId, info: ParticipantInfo) -> Result<Vec<SignalOutput>>;
    fn leave_room(&mut self, room_id: RoomId, conn_id: ConnId) -> Result<Vec<SignalOutput>>;
    fn route_message(&mut self, room_id: RoomId, from: ConnId, msg: SignalingMessage) -> Result<Vec<SignalOutput>>;
}
```

`route_message` 内部按 topology 分支：

| Topology | 路由策略 |
|----------|---------|
| PeerToPeer | 单播给房间内另一个参与者 |
| SfuRelay | 广播给所有参与者（不含发送者）。信令层仅转发 SDP/ICE，媒体 plane 由外部 SFU (mediasoup) 处理 |
| PublishSubscribe | 按 TrackSubscription 过滤 |

### 10.7.1 mediasoup SFU 信令桥接 (Phase 2)

当 topology = `SfuRelay` 时，信令层引入 SDP 适配桥接：

```
Host ──WS──→ SignalHandler ──SDP adapter──→ mediasoup Router
  │  SDP offer        │  PlainTransport.createProducer        │
  │                   │  WebRtcTransport.createConsumer      │
Remote ←──WS── SignalHandler ←──SDP adapter── mediasoup Router
```

**SDP adapter** 将 WebRTC SDP 转换为 mediasoup 的 PlainTransport / WebRtcTransport 参数。信令层不创建 mediasoup Transport — 仅传递 JSON 参数给 Server 端的 mediasoup 集成模块。

**WebSocket relay 保留 DC 控制命令**: mediasoup 处理音视频 RTP，但 RTCDataChannel 控制命令（键盘/鼠标/手柄）仍通过 WebSocket 信令通道透传。这避免了 SFU 侧的 SCTP 开销，且控制命令负载小 (<1KB)，WS 延迟足够。

## 10.8 浏览器客户端适配

`SignalHandler` 和 `RoomRouter` 都是**连接类型无关**的 — 它们处理 `ConnId` + `SignalingMessage`，不关心消息来自原生客户端还是浏览器。信令层不关心媒体是否由 SFU (mediasoup) 处理 — SDP 交换的语义对 SignalHandler 透明。

浏览器客户端：
- 使用 JS `WebSocket` API 连接信令服务
- 发送/接收 `ClientSignal`/`ServerSignal`（JSON 格式，同 proto 定义）
- `RTCPeerConnection.onicecandidate` → 序列化为 `IceCandidateMessage` → WS 发送
- WS 收到 `remote_ice` → `pc.addIceCandidate()`

服务端用同一个 `SignalHandler` 处理两种客户端，通过 WS upgrade 时的 `User-Agent` 或自定义 header 区分协议（binary = protobuf，text = JSON）

## 10.9 场景覆盖矩阵

| 场景 | 拓扑 | SignalHandler Phase 1 | RoomRouter Phase 2 |
|------|------|----------------------|-------------------|
| 远程桌面 | Server relay (D96 relay-default) | ✅ 配对中继 | — |
| 遥控座舱 | Server relay + RTCDataChannel (D118) | ✅ 配对中继 + dc_signal | — |
| 视频会议 | N:M SFU | — | ✅ SFU 广播 |
| 直播推流 | 1:N PubSub | — | ✅ PubSub 按订阅 |
| 监控拉流 | N:1 RTSP 桥接 | — | ✅ 单源多订阅 |
| 本地采集 | 0（无信令） | N/A | N/A |

## 10.10 参考项目

| 项目 | 借鉴点 |
|------|--------|
| RustDesk hbbs | Protobuf RendezvousMessage oneof + 3端口 + Ed25519 对等密钥 |
| LiveKit | MessageSink/MessageSource 抽象 + 双格式 WS + JWT |
| mediasoup | 信令无关 + WebRtcTransport/PlainTransport 类型区分 |
| str0m | sans-I/O poll_output 不变式 |
| webrtc-kit | RTCPeerConnectionFactory trait + cfg 调度（适用于 SignalHandler 与 RoomRouter 的独立实现选择） |

## 10.11 Phase 2+: MQTT 5.0 信令

> **状态改判（D289 / F13，2026-09-14 团队评审定稿）**：MQTT 不再是默认 Phase 2+ 双轨项——WS 信令硬化已由 S0.5 落地（v3 心跳/会话续期/优先级队列），MQTT 仅在「离线指令下发 / 大规模 presence」场景出现时触发评估，**前置闸=先完成会话状态外置改造评估**（本仓「连接=会话」模型与 broker 总线模型不可混用；presence 双源=同会话双栈已否决 D 项）。以下为原设计存档（未实施）。

Phase 2+ 引入 MQTT 5.0 作为 Vehicle-to-Cloud 信令通道。Phase 1 仅使用 WebSocket。MQTT 5.0 的核心特性（session persistence、shared subscriptions、request-response pattern）适用于大规模车联场景（D74）。
