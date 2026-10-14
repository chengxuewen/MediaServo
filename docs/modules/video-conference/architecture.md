# Video Conference Architecture

## 1. Component Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                        Client (Web/Native)                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────────┐  ┌─────────────┐   │
│  │ Camera   │  │ Mic      │  │ Screen Share │  │ Audio       │   │
│  │ Capture  │  │ Capture  │  │ Capture      │  │ Playback    │   │
│  └────┬─────┘  └────┬─────┘  └──────┬───────┘  └──────┬──────┘   │
│       │ VP8/AV1     │ Opus          │ VP8/AV1          │ Opus     │
│       ▼              ▼               ▼                  ▼          │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │  WebRTC Stack (Simulcast + SVC, transport-cc, TWCC)      │    │
│  └──────────────────────────┬───────────────────────────────┘    │
│                             │ SRTP/UDP                           │
└─────────────────────────────┼───────────────────────────────────┘
                              │
┌─────────────────────────────┼───────────────────────────────────┐
│                     MediaServo Backend                           │
│                              │                                   │
│  ┌──────────────────────────┴──────────────────────────────┐    │
│  │               Signaling (WebSocket + gRPC)              │    │
│  │  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐  │    │
│  │  │ Room Mgr    │  │ Participant  │  │ SFU Selector   │  │    │
│  │  │ (create/    │  │ Mgr (join/   │  │ (worker alloc, │  │    │
│  │  │  destroy)   │  │  leave)      │  │  load balance) │  │    │
│  │  └─────────────┘  └──────────────┘  └────────────────┘  │    │
│  └──────────────────────────┬───────────────────────────────┘    │
│                             │                                    │
│  ┌──────────────────────────┴──────────────────────────────┐    │
│  │              Conference Controller (Rust)               │    │
│  │  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐  │    │
│  │  │ Auth Guard  │  │ Permission   │  │ ICE/TURN Relay │  │    │
│  │  │ (token)     │  │ Check        │  │ (NAT traversal)│  │    │
│  │  └─────────────┘  └──────────────┘  └────────────────┘  │    │
│  └──────────────────────────┬───────────────────────────────┘    │
│                             │                                    │
│  ┌──────────────────────────┴──────────────────────────────┐    │
│  │              mediasoup SFU Engine                       │    │
│  │  ┌───────────┐  ┌───────────┐  ┌───────────┐            │    │
│  │  │ Worker 1  │  │ Worker 2  │  │ Worker N  │            │    │
│  │  │ Router A  │  │ Router B  │  │ Router C  │            │    │
│  │  │           │  │           │  │           │            │    │
│  │  │ Producer  │  │ Producer  │  │ Producer  │            │    │
│  │  │ Consumer  │  │ Consumer  │  │ Consumer  │            │    │
│  │  └───────────┘  └───────────┘  └───────────┘            │    │
│  └─────────────────────────────────────────────────────────┘    │
│                             │                                    │
│  ┌──────────────────────────┴──────────────────────────────┐    │
│  │              Recording / ASR Pipeline                   │    │
│  │  (RTP Forwarding: SFU -> external consumers via pipe)   │    │
│  └─────────────────────────────────────────────────────────┘    │
│                             │                                    │
│  ┌──────────────────────────┴──────────────────────────────┐    │
│  │  Redis: Room state, Pub/Sub, Node health, PSRPC         │    │
│  └─────────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────┘
```

## 2. SFU Engine

### 2.1 Choice: mediasoup

mediasoup is the core SFU engine, bound into MediaServo native-core via napi-rs.

Rationale:
- C++ implementation, best performance, process-level Worker isolation
- Signaling-agnostic; the control plane is fully customizable
- `pipeToRouter()` cascading supports same-host and cross-host distributed deployment
- Rust-FFI friendly; wrapped as a Rust crate via napi-rs or FFI

### 2.2 Worker Model

```
┌───────────────┐
│ Conference    │
│ Controller    │
│ (Rust)        │
│               │
│  ┌─────────┐  │
│  │ Worker  │  │  ← Each Worker is an independent C++ process
│  │ Pool    │  │     process isolation; a crash doesn't affect other Workers
│  │ Mgr     │  │
│  └────┬────┘  │
└───────┼───────┘
        │
  ┌─────┼─────┐
  │     │     │
  ▼     ▼     ▼
┌────┐ ┌────┐ ┌────┐
│ W1 │ │ W2 │ │ WN │  ← Independent processes; 1 Worker ≈ 1 CPU core
│ Rtr│ │ Rtr│ │ Rtr│
└───┬┘ └───┬┘ └───┬┘
    │      │      │
    └──────┴──────┘
            │ pipeToRouter() across Workers / hosts
            ▼
      ┌──────────┐
      │ Remote   │
      │ Router   │
      └──────────┘
```

Worker allocation strategy:
- Each Worker binds one CPU core
- New conferences are assigned to the least-loaded Worker
- Large rooms get a dedicated Worker; small rooms share Workers
- Default `auto` policy: `ceil((nproc * 0.8) + (max(0, nproc - 32) / 2))`

### 2.3 Producer/Consumer Model

mediasoup uses a publish/subscribe model. MediaServo has two Producer origins:

**Browser Producer**: captures camera/mic via getUserMedia and creates a Producer through mediasoup-client (JS).

**Native Host Producer**: mediaservo-host establishes the ICE/DTLS connection via mediaservo-webrtc (webrtc-sys/libwebrtc)
and creates a Producer through a WebRtcTransport to inject the video stream. The Host has no browser dependency; suitable for vehicle/edge native pushing.

```
Producer (publisher)          Consumer (subscriber)
┌──────────────────┐       ┌──────────────────┐
│ Router           │       │ Router           │
│  │               │       │  │               │
│  ▼               │       │  ▼               │
│ Producer         │       │ Consumer         │
│ • Browser (JS)   │       │ • Browser (JS)   │
│ • Host (native)  │       │ • RTP stream out │
│ - RTP stream in  │       │ - Layer choice   │
│ - Simulcast/SVC  │       │ - Retransmission │
│ - Encoding info  │       │   buffer         │
└──────────────────┘       └──────────────────┘

### 2.4 Cascading

Cross-region conferences cascade via `pipeToRouter()`:

```
Zone A (Beijing)              Zone B (Singapore)
┌──────────────────┐         ┌──────────────────┐
│ Worker 1         │         │ Worker 2         │
│ Router A1        │◄────────│ Router B1        │
│ Participants     │ pipe    │ Participants     │
│  A, B, C         │         │  D, E, F         │
└──────────────────┘         └──────────────────┘
        │                            │
    ┌───────┐                   ┌───────┐
    │ Local │                   │ Local │
    │ SFU   │                   │ SFU   │
    └───────┘                   └───────┘
```

Cascading decisions:
- Same-region participants are assigned to the same Router
- Cross-region cascades via `pipeToRouter()`
- Cascading forwards only currently active streams (PLI/FIR aggregated control)
- Avoid the full-mesh model (the Jitsi 2020 lesson)

## 3. Audio Strategy

### 3.1 Design Choice

**SFU forwarding + client-side mixing as the main path; MCU mixing as a supplement.**

```
Normal conference:
  The SFU forwards all participants' Opus audio
  → the client decodes multiple audio streams and mixes locally
  → the server doesn't mix, consuming no CPU

PSTN bridging:
  SFU forwarding + FreeSWITCH MCU mixing
  → PSTN users receive a single mixed audio stream
  → optional server-side mixing at recording time

Silent-listening mode (borrowed from BBB):
  → muted users' audio channels hang up automatically
  → only active speakers' audio is forwarded
  → saves SFU resources
```

### 3.2 Active Speaker Detection

Borrowing Google Meet's virtual media stream model:

- The server detects the 3 loudest current speakers' audio
- Active-speaker changes are flagged via the RTP CSRC field
- The client keeps listening to a fixed number of audio streams; CSRC switches dynamically
- No client renegotiation needed

## 4. Codec Strategy

### 4.1 Video Codecs

| Codec | Role | Scenario |
|-------|------|----------|
| VP8 | Baseline | All-platform compatibility |
| VP9 | SVC primary | Browser-preferred; VP9 SVC L3T3_KEY |
| H.264 | Hardware compatibility | Mobile, hardware encoders |
| AV1 | Progressive rollout | Warm-up traffic + optional main codec |

### 4.2 Simulcast + SVC

- Simulcast by default (VP8/H.264): 3 layers (low/mid/high)
- VP9 enables SVC mode: temporal + spatial layering
- Dynamic switching: SVC when few participants (<16), Simulcast when many (>16)
- Dynacast (encode on demand): only encode layers with subscribers

### 4.3 Bandwidth Management

- Clients feed bandwidth estimates via transport-cc and TWCC
- The SFU adaptively selects forwarding layers based on bandwidth
- Adaptive congestion control: delay-based for latency-sensitive networks, loss-based for lossy networks
- Audio first: under heavy loss, keep audio and drop video

## 5. Recording

### 5.1 Two-Layer Architecture

```
Live recording:
  Producer → pipeToRouter() → Recording Consumer → per-track storage

Async compositing:
  After the conference ends, batch-composite the tracks into final video files
  GPU/CPU compositing, multiple layouts supported
```

### 5.2 RTP Forwarding

Borrowing Janus's RTP Forwarding model:

- The SFU duplicates the publisher's RTP/RTCP stream
- Pushes it via pipe to an external recording service
- The recording service is a non-WebRTC endpoint receiving raw RTP
- Supports both per-track and composited recording

## 6. Security

- SRTP + DTLS encrypted media transport
- Token authentication (JWT)
- Optional end-to-end encryption (Insertable Streams)
- Permission control: who may speak, share screen, record
- SM (Chinese national) cipher support (China deployment scenario)

## 7. Reference Architecture Comparison

| Dimension | MediaServo VC | Zoom | Jitsi | LiveKit |
|-----------|---------------|------|-------|---------|
| SFU engine | mediasoup (C++) | MMR (in-house) | JVB (Java) | LiveKit (Go) |
| Signaling | In-house WebSocket+gRPC | In-house | XMPP+Colibri2 | In-house WebSocket |
| Cascading | pipeToRouter | Backbone cascade | Secure Octo | Redis Router |
| Audio | SFU+client mixing | SFU | SFU+client mixing | SFU+client mixing |
| Recording | RTP Forwarding | Native | Jibri | Egress |
| Deployment | Multiple forms | Not self-hostable | Self-hosted | Self-hosted+cloud |

See the [research doc](../../research/video-conference.md) for the full research analysis.
