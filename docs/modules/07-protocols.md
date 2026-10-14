# Protocols & Communication Reference

> MediaServo uses a multi-protocol layered design covering internal communication, external access, and media transport.

---

## 1. Protocol Layers

| Layer | Protocol | Description |
|-------|----------|-------------|
| **Internal IPC** | FlatBuffers | Zero-copy communication between plugins |
| **Third-party platforms** | C FFI | Rust → C static linking |
| **Host platform** | napi-rs | Rust → Node.js native module |
| **Backend services** | gRPC (protobuf) | Control-plane API |
| **Signaling** | WebSocket | SDP negotiation, ICE exchange |
| **Media transport** | RTP/RTCP, SRT, WebRTC | Data plane |
| **Client ⇄ backend** | gRPC + REST | Auth, permission pull, config sync |

---

## 2. Protocol Selection Guide

| Protocol | Push ingest | Playback distribution | Low latency | NAT traversal | Mobile | Recommendation |
|----------|:-----------:|:---------------------:|:-----------:|:-------------:|:------:|:--------------:|
| **RTMP** | ✅ | ⚠️ | 1-5s | ❌ | ❌ | Must support (ecosystem compatibility) |
| **SRT** | ✅ | - | 0.5-2s | ⚠️ | ❌ | High (reliable push) |
| **WebRTC/WHIP** | ✅ | ✅ | <500ms | Needs TURN | ✅ | High (low-latency interaction) |
| **HLS/LL-HLS** | - | ✅ | 2-10s | ✅ | ✅ | Must (large-scale distribution) |
| **MPEG-DASH** | - | ✅ | 2-10s | ✅ | ✅ | Recommended |
| **HTTP-FLV** | - | ✅ | 1-3s | ✅ | ⚠️ | Optional (Chinese ecosystem) |
| **RTSP** | ✅ | ✅ | 0.5-2s | ❌ | ❌ | Recommended (IPC access) |
| **Media-over-QUIC** | ✅ | ✅ | <100ms | ✅ | - | Future direction |
| **MoQ (QUIC)** | ✅ | ✅ | <100ms | ✅ | - | Future direction |

---

## 3. WebRTC RTCDataChannel Protocol (teleoperation)

### 3.1 Channel Separation Strategy

| Channel Type | ordered | Reliability mode | Typical payload | Frequency |
|--------------|---------|------------------|-----------------|-----------|
| **Control command channel** | false | maxRetransmits: 0 | Steering/throttle/brake (uint16 × N) | 50-200Hz |
| **Telemetry channel** | false | maxRetransmits: 0 | Speed/GPS/IMU/status bits | 20-50Hz |
| **Reliable command channel** | true | reliable | System restart/mode switch/config push | On demand |
| **File/log channel** | true | reliable | Log upload/firmware update | On demand |
| **Heartbeat channel** | false | maxRetransmits: 0 | Timestamp + sequence number | 1-10Hz |

### 3.2 Binary Protocol Format

**Extended control packet** (recommended for teleoperation):
```
[timestamp: uint32 LE] [seq: uint16 LE] [cmd: uint8] [flags: uint8] [payload: N bytes]
Total overhead: 8-byte header

flags:
  bit 0: emergency (e-stop)
  bit 1: heartbeat
  bit 2: ack_requested
  bit 3-7: reserved
```

**Telemetry packet**:
```
[timestamp: uint32 LE] [seq: uint16 LE]
[steering_angle: int16] [throttle: int16] [brake: uint16]
[speed: uint16] [yaw_rate: int16] [lat_accel: int16]
[gps_lat: int32] [gps_lon: int32] [heading: uint16]
[status_flags: uint16] [battery: uint8] [signal_rssi: int8]
Total overhead: 37 bytes
```

### 3.3 Core Principles
- Ordered reliable channels share one SCTP stream — one channel blocking doesn't affect others
- Real-time control must be `ordered: false` — old commands arriving late are actively dangerous
- Heartbeat on its own channel — not mixed with other data, measuring true network RTT
- Single message ≤ 16KB (browser compatibility ceiling)

---

### MQTT 5.0 (Phase 2+)

MQTT 5.0 is planned for Phase 2+ vehicle-to-cloud signaling scenarios. Phase 1 uses WebSocket exclusively. Key features: session persistence, shared subscriptions, request-response pattern. See decision D74.

---

## 4. WebRTC Backends

MediaServo supports three WebRTC backends, switched via Cargo feature gates:

| Backend | Feature Gate | Use | Phase |
|---------|--------------|-----|-------|
| **webrtc-sys** (libwebrtc FFI) | `webrtc-libwebrtc` | Default backend, full WebRTC support | Phase 1+ |
| **str0m** | `webrtc-str0m` | sans-I/O, lightweight pure Rust | Phase 1 (LAN) |
| **mediasoup-sys** | `webrtc-mediasoup` | SFU multi-party conferencing | Phase 2 |

Decision basis: D137 (libwebrtc FFI selection), D139 (webrtc-sys crate definition), D118 (mediasoup SFU plan).

### 4.1 webrtc-sys (default backend)

webrtc-sys binds Google libwebrtc via FFI, providing the full RTCPeerConnection + RTP track API.
Compiled as the default Cargo feature (`default = ["webrtc-libwebrtc"]`).
See `crates/webrtc-sys/`.
