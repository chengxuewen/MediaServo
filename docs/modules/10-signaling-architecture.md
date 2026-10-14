# 10. Signaling Architecture

> Version: 0.1 | Date: 2026-07-16 | Phase: 0
> Related decisions: D51-D54, D74 (signaling protocol) + main doc reference: [architecture doc](../architecture.md) §8

## 10.1 Core Principle

**Signaling = opaque message relay**. The signaling layer never parses SDP/ICE content — the SDP/ICE state machine belongs to `MediaTransport`. The signaling layer is only responsible for:

1. Authenticating connections
2. Routing messages (keyed by ConnId)
3. Topology-aware forwarding (P2P unicast, SFU broadcast, PubSub per subscription)

## 10.2 Architecture Layers

```
┌───────────────────────────────────┐
│  Phase 2: RoomRouter              │ Topology-aware routing (P2P/SFU/PubSub)
│  create_room / route_message      │
├───────────────────────────────────┤
│  Phase 1: SignalHandler           │ Message relay + connection pairing
│  accept / handle_input / close    │
├───────────────────────────────────┤
│  Transport: axum WebSocket        │ Managed by the framework; no trait defined
└───────────────────────────────────┘

        SignalHandler operates sans-I/O
        ┌─────────────────────────┐
        │  handle_input(conn, msg) │ ← message received
        │  → Vec<SignalOutput>     │ ← routing instructions produced
        │                         │
        │  poll_output()           │ ← periodic heartbeat/timeouts
        │  → Vec<SignalOutput>     │
        └─────────────────────────┘
                 │
                 ▼
        SignalOutput { target, msg }
        target: Direct(ConnId) | Room(RoomId) | Broadcast
```

## 10.3 SignalHandler Trait (Phase 1)

```rust
use std::collections::HashMap;

/// Connection identifier (assigned by axum WebSocket)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(u64);

/// Signaling message (client↔server)
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

/// Routing output
#[derive(Debug, Clone)]
pub struct SignalOutput {
    pub target: SignalTarget,
    pub message: SignalingMessage,
}

#[derive(Debug, Clone)]
pub enum SignalTarget {
    /// Send directly to the specified connection
    Direct(ConnId),
    /// Send to all participants in the room (excluding the sender)
    Room(RoomId, ExcludeSender),
    /// Send to the specified connection list
    Multi(Vec<ConnId>),
}

#[derive(Debug, Clone)]
pub enum ExcludeSender {
    Yes,
    No,
}

/// sans-I/O signaling handler
pub trait SignalHandler: Send {
    /// Accepts a new connection (called after auth passes)
    fn accept(&mut self, conn_id: ConnId, auth: &[u8]) -> Result<Vec<SignalOutput>>;

    /// Handles a received message
    fn handle_input(&mut self, conn_id: ConnId, msg: SignalingMessage) -> Result<Vec<SignalOutput>>;

    /// Handles connection close
    fn handle_close(&mut self, conn_id: ConnId) -> Result<Vec<SignalOutput>>;

    /// Periodic poll (heartbeat/timeouts)
    fn poll_output(&mut self) -> Result<Vec<SignalOutput>>;
}
```

## 10.4 Phase 1 Implementation: Pairing Relay

The Phase 1 remote cockpit is 1:1 P2P. The simplest implementation:

```rust
pub struct PairingRelay {
    pairs: HashMap<ConnId, ConnId>,  // A → B mapping
    pending: HashMap<String, ConnId>, // token → connection awaiting pairing
}

impl SignalHandler for PairingRelay {
    fn accept(&mut self, conn_id: ConnId, auth: &[u8]) -> Result<Vec<SignalOutput>> {
        let token = parse_token(auth)?;
        if let Some(&peer) = self.pending.remove(&token) {
            // Second connection arrives → pairing complete
            self.pairs.insert(conn_id, peer);
            self.pairs.insert(peer, conn_id);
            // Notify both sides
            Ok(vec![
                SignalOutput { target: SignalTarget::Direct(conn_id), message: SignalingMessage::JoinAccepted },
                SignalOutput { target: SignalTarget::Direct(peer), message: SignalingMessage::ParticipantJoined },
            ])
        } else {
            // First connection arrives → wait for pairing
            self.pending.insert(token, conn_id);
            Ok(vec![])
        }
    }

    fn handle_input(&mut self, conn_id: ConnId, msg: SignalingMessage) -> Result<Vec<SignalOutput>> {
        let &peer = self.pairs.get(&conn_id)
            .ok_or_else(|| anyhow::anyhow!("no peer for {:?}", conn_id))?;
        Ok(vec![SignalOutput {
            target: SignalTarget::Direct(peer),
            message: msg,  // Forwarded verbatim, never parsed
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
        Ok(vec![])  // Phase 1: no heartbeat
    }
}
```

## 10.5 Protobuf Message Definitions

Client↔server uses **separate enums** (not a shared oneof):

```protobuf
syntax = "proto3";
package mediaservo.signaling;

// === Client → Server ===
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

// === Server → Client ===
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

**Design rationale**:
- Separate enums guarantee type safety — a browser never sends `ParticipantJoined`; the server never sends `JoinRoom`
- Each message is independently defined, maximizing reuse (`SdpMessage` serves both offer and answer)
- `DataChannelSignal` is independent of SDP — RTCDataChannel is created in-band over SCTP; signaling only conveys intent
- `TrackRequest` three-in-one — publish/subscribe/unsubscribe share fields; the enum value distinguishes semantics

**Wire format**: WebSocket binary frames = protobuf; WebSocket text frames = JSON (for debugging, serde-derived from the same protobuf schema)

## 10.6 sans-I/O Run Loop

Same invariant as `MediaTransport`: **after every mutation, poll_output must be fully drained before the next mutation**.

```rust
// Server WebSocket event loop
loop {
    select! {
        // Channel 1: client message received
        msg = ws_rx.recv() => {
            let outputs = signal_handler.handle_input(conn_id, msg)?;
            drain_outputs(&mut signal_handler, &ws_senders, outputs).await;
        }
        // Channel 2: timer (heartbeat)
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
        // poll_output may cascade (e.g. handle_close triggering reconnects)
        outputs = handler.poll_output()?;
    }
}
```

## 10.7 Phase 2 Extension: RoomRouter

When N:M multi-party conferencing is needed, extract the RoomRouter trait:

```rust
#[derive(Debug, Clone)]
pub enum RoomTopology {
    PeerToPeer,
    /// mediasoup SFU relay (Phase 2) — the signaling layer only manages SDP exchange;
    /// the media plane is handled independently by the mediasoup Worker; the signaling layer never parses media content.
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

`route_message` branches internally by topology:

| Topology | Routing strategy |
|----------|------------------|
| PeerToPeer | Unicast to the other participant in the room |
| SfuRelay | Broadcast to all participants (excluding the sender). The signaling layer only relays SDP/ICE; the media plane is handled by the external SFU (mediasoup) |
| PublishSubscribe | Filtered by TrackSubscription |

### 10.7.1 mediasoup SFU Signaling Bridge (Phase 2)

When topology = `SfuRelay`, the signaling layer adds an SDP adaptation bridge:

```
Host ──WS──→ SignalHandler ──SDP adapter──→ mediasoup Router
  │  SDP offer        │  PlainTransport.createProducer     │
  │                   │  WebRtcTransport.createConsumer    │
Remote ←──WS── SignalHandler ←──SDP adapter── mediasoup Router
```

The **SDP adapter** converts WebRTC SDP into mediasoup's PlainTransport / WebRtcTransport parameters. The signaling layer does not create mediasoup Transports — it only passes JSON parameters to the Server-side mediasoup integration module.

**WebSocket relay keeps DC control commands**: mediasoup handles the audio/video RTP, but RTCDataChannel control commands (keyboard/mouse/gamepad) still relay through the WebSocket signaling channel. This avoids SCTP overhead on the SFU side, and since control payloads are small (<1KB), WS latency suffices.

## 10.8 Browser Client Adaptation

Both `SignalHandler` and `RoomRouter` are **connection-type agnostic** — they handle `ConnId` + `SignalingMessage`, indifferent to whether the message comes from a native client or a browser. The signaling layer does not care whether media is handled by the SFU (mediasoup) — SDP exchange semantics are transparent to the SignalHandler.

Browser clients:
- Connect to the signaling service using the JS `WebSocket` API
- Send/receive `ClientSignal`/`ServerSignal` (JSON format, same as the proto definitions)
- `RTCPeerConnection.onicecandidate` → serialize into `IceCandidateMessage` → send over WS
- WS receives `remote_ice` → `pc.addIceCandidate()`

The server uses the same `SignalHandler` for both client types, distinguishing the protocol by the `User-Agent` or a custom header at WS upgrade (binary = protobuf, text = JSON)

## 10.9 Scenario Coverage Matrix

| Scenario | Topology | SignalHandler Phase 1 | RoomRouter Phase 2 |
|----------|----------|-----------------------|--------------------|
| Remote desktop | Server relay (D96 relay-default) | ✅ Pairing relay | — |
| Remote cockpit | Server relay + RTCDataChannel (D118) | ✅ Pairing relay + dc_signal | — |
| Video conference | N:M SFU | — | ✅ SFU broadcast |
| Live streaming | 1:N PubSub | — | ✅ PubSub per subscription |
| Surveillance pull | N:1 RTSP bridge | — | ✅ Single source, multi-subscriber |
| Local capture | 0 (no signaling) | N/A | N/A |

## 10.10 Reference Projects

| Project | What We Borrow |
|---------|----------------|
| RustDesk hbbs | Protobuf RendezvousMessage oneof + 3 ports + Ed25519 peer keys |
| LiveKit | MessageSink/MessageSource abstraction + dual-format WS + JWT |
| mediasoup | Signaling-agnostic + WebRtcTransport/PlainTransport type distinction |
| str0m | sans-I/O poll_output invariant |
| webrtc-kit | RTCPeerConnectionFactory trait + cfg dispatch (applies to the independent-implementation choice for SignalHandler and RoomRouter) |

## 10.11 Phase 2+: MQTT 5.0 Signaling

> **Status revision (D289 / F13, finalized in the 2026-09-14 team review)**: MQTT is no longer the default Phase 2+ dual-track item — WS signaling hardening has already landed via S0.5 (v3 heartbeat/session resume/priority queues); MQTT is only evaluated when the "offline command delivery / large-scale presence" scenario actually appears, **with the precondition gate = completing the externalized-session-state assessment first** (this repo's "connection = session" model and the broker bus model cannot be mixed; the dual-presence-source = dual-stack same-session option was rejected in option D). The original design below is archived (unimplemented).

Phase 2+ introduces MQTT 5.0 as the Vehicle-to-Cloud signaling channel. Phase 1 uses WebSocket only. MQTT 5.0's core features (session persistence, shared subscriptions, request-response pattern) suit large-scale vehicle-connection scenarios (D74).
