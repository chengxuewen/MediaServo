# 14. Remote Application Architecture

> Phase 0 — architecture definition | 2026-07-17
> Related decisions: D66-D69, D70-D72, D76, D80, D118 + main doc reference: [architecture doc](../architecture.md) §2, §7

## 14.1 Overview

mediaservo-client is the cockpit-side application: it pulls the video stream from the Server, decodes and renders it to screen, and sends control commands over RTCDataChannel. Remote only pulls and controls; it never pushes.

```
Server (mediaservo-server)
  │ WebRTC forward (RTP/SRTP)
  │ Signaling (WS /ws, JSON SDP/ICE)
  ▼
mediaservo-client
  ┌────────────────────────────────────────────┐
  │  Signaling Client                          │
  │  axum WS, join room, SDP/ICE exchange      │
  ├────────────────────────────────────────────┤
  │  Pull Engine (libwebrtc RTCPeerConnection) │
  │  Subscribes to the Host stream the Server  │
  │  forwards                                  │
  │  H.264/H.265 RTP stream                    │
  ├────────────────────────────────────────────┤
  │  Decoder Pipeline                          │
  │  libwebrtc built-in (codec) / FFmpeg       │
  │  (str0m)                                   │
  │  HW decode: NVDEC/VAAPI/VideoToolbox       │
  │  SW fallback: VP8 libvpx / H.264 openh264  │
  ├────────────────────────────────────────────┤
  │  Render                                    │
  │  Phase 1: CPU readback → Canvas            │
  │  Phase 2: GPU direct (wgpu interop)        │
  ├────────────────────────────────────────────┤
  │  RTCDataChannel Control                    │
  │  Control command sending (steer/brake/     │
  │  throttle)                                 │
  │  HMAC-SHA256 signed, unordered=0 retrans   │
  ├────────────────────────────────────────────┤
  │  Monitoring                                │
  │  /health /ready /metrics                   │
  │  Decode performance, control-frame loss    │
  └────────────────────────────────────────────┘
```

## 14.2 Core Components

| Component | Responsibility | Decisions |
|-----------|----------------|-----------|
| Signaling Client | Connects the Server WS, SDP/ICE exchange, room management | D52, D54 |
| Pull Engine | libwebrtc RTCPeerConnection pull, subscribing to the Host video | D66, D118 |
| Decoder | Decodes H.264/H.265 frames. libwebrtc built-in codecs or FFmpeg | D70-D72 |
| Render | Renders decoded frames to screen. Phase 1 CPU readback, Phase 2 GPU direct | D47 |
| RTCDataChannel Control | Sends control commands (steer/brake/throttle), HMAC-SHA256 signed | D66, D117 |
| Config | remote.conf (YAML), read-only local config | D80 |

## 14.3 Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| WebRTC pull | libwebrtc (via webrtc-sys) | Built-in VP8/VP9/H.264 codecs, weak-network resilience |
| Decode | libwebrtc built-in codecs (default) / FFmpeg (str0m backend) | D70-D72 |
| Render | CPU readback appsink → Canvas (Phase 1) | Minimal implementation |
| Control | RTCDataChannel (unordered, maxRetransmits=0) | Low-latency control commands |
| Signing | HMAC-SHA256, DTLS-SRTP derived key | D117 |
| HTTP | axum 0.7 (/health /ready /metrics) | Unified with Server/Host |
| Config | serde_yaml + env override | Global unified pattern |
| Logging | tracing-subscriber (JSON stdout) | Unified with Server/Host |
| Platforms | macOS / Linux / Windows | D118 cross-platform desktop baseline |

## 14.4 Data Flow

```
Server → WebRTC RTP (H.264/H.265)
  │
  ▼
libwebrtc RTCPeerConnection (pull)
  │ Decoded frames (I420/NV12)
  ▼
Decoder (libwebrtc built-in / FFmpeg)
  │ Decoded RGBA buffer
  ▼
Render (CPU fallback → GPU direct Phase 2)
  │ Frame displayed in the window
  │
  ─── RTCDataChannel (control commands, flowing back to Host)
       steering / brake / throttle
       HMAC-SHA256 signed
```

## 14.5 Remote vs Client

| Dimension | mediaservo-client (remote/SDK) | mediaservo-client (GUI) |
|-----------|--------------------------------|--------------------------|
| GUI | No GUI (pure SDK) / simple render window | Tauri v2 full-featured desktop app |
| SDK form | mediaservo-client-c (.a + .so + .h) | Direct Rust crate dependency |
| Scenario | C/C++ embedding (ROS/autonomous driving/mobile) | Operator desktop app |
| Decode | libwebrtc built-in / FFmpeg | Same as remote |
| Control | RTCDataChannel control commands | Full-featured GUI operation |
| Config | remote.conf (read-only local) | GUI settings panel |

## 14.6 Platform Support

| Platform | Capture | Decode | HW acceleration |
|----------|---------|--------|-----------------|
| macOS | — | VideoToolbox | VT H.264/H.265 |
| Linux | — | VAAPI / NVDEC | VAAPI H.264, NVDEC |
| Windows | — | D3D11VA / NVDEC | NVDEC H.264/H.265 |

Remote needs no capture. Decoding prefers hardware acceleration; when no GPU is available it falls back to software decoding (libvpx VP8 / openh264 H.264).

## 14.7 State Machine

```
INIT → CONNECTING (connecting to Server) → RECEIVING (receiving + rendering)
  │                                             │
  └── shutdown ────────────────────────────────┘
                                                │
                          (disconnect) → CONNECTING (exponential backoff reconnect)
```

## 14.8 Cross References

- Remote as the SDK layer: [SDK layers](04-sdk-layers.md)
- Client vs Host comparison: [client & host](03-client-host.md)
- Signaling protocol: [signaling architecture](10-signaling-architecture.md)
- Security model: [architecture doc](../architecture.md) §3 (Auth)
- Server relay: [server architecture](13-server-architecture.md)
