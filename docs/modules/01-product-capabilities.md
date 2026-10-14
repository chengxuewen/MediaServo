# Product Capability Overview

> MediaServo offers seven product capabilities covering remote desktop, video conferencing, live streaming push/pull, surveillance access, teleoperation, and more.

---

## 1. The Seven Product Capabilities

| Capability | Description | Typical Scenarios |
|------------|-------------|-------------------|
| **Remote Desktop** | Screen capture, GPU encoding (H.264/H.265), input injection, <100ms latency | IT ops, remote work |
| **Video Conferencing** | Multi-party audio/video calls, SFU/MCU, screen sharing | Team collaboration |
| **Streaming (push/pull)** | RTMP/HLS/SRT ingest and distribution, live transcoding | Live streaming, content distribution |
| **Surveillance Access** | ONVIF/GB28181 camera discovery and stream management | Security, inspection |
| **WebRTC Teleoperation** | Low-latency video + RTCDataChannel control | Vehicle teleoperation, robotics |
| **Vehicle Push** | Vehicle cameras streaming to the cloud | Connected vehicles |
| **Cockpit Pull** | In-cockpit screens viewing remote video | Remote driving |

---

## 2. Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **Core language** | Rust (edition 2024) | Zero-cost abstractions, memory safety, cross-platform |
| **WebRTC stack** | webrtc-sys (libwebrtc FFI, default) / str0m (LAN) / mediasoup-sys (Phase 2, D118) | MVP Phase 1: libwebrtc P2P + RTP track-level API |
| **GPU encoding** | libloading bridging NVENC/VAAPI/VT | Avoids compile-time binding to GPU SDKs |
| **Codec** | GStreamer (gst-plugins-rs) | Broad coverage, mature ecosystem |
| **Signaling** | WebSocket (long-term; MQTT = trigger clause — D289/F13 revision of D74's dual-track promise) | D74→D289 |
| **Internal protocol** | FlatBuffers | Zero-copy, multi-language |
| **Desktop GUI** | Tauri v2 + React | Lightweight, Rust backend, cross-platform |
| **Embedded Web** | axum + static HTML | Host config page, no framework dependency |
| **Build** | Cargo workspace | Unified dependency management |

---

## 3. Key Design Decisions

### 3.1 MVP v2 Relay Model (D118)

The MVP uses a 3-component relay architecture: Host → Server(relay) → Client. The Server handles signaling relay and media forwarding; P2P penetration is only enabled in the third-party platform Studio LAN scenario. mediasoup SFU is the Phase 2 multi-party conferencing backend (D118 appendix); MVP Phase 1 prioritizes completing 1:1 relay.

Industry consensus — all products use this connection strategy. The default goes through SFU/TURN relay (D118 relay-default).

### 3.2 UDP as the First-Priority Transport

TCP head-of-line blocking is fatal in real-time scenarios. Parsec and ToDesk experience shows UDP + a custom reliability layer is the optimal solution.

### 3.3 Hardware Codec First + Software Fallback

Balances performance and compatibility. Hardware encoding provides low latency (NVENC median ~5.8ms); software encoding serves as fallback on older devices.

### 3.4 Multiprocess Privilege Separation

The network layer and UI layer run as separate processes (validated by AnyDesk/RustDesk/Chrome Remote Desktop).

### 3.5 Protobuf for All Protocols

Type safety, multi-language support, evolution-friendly (RustDesk experience).

### 3.6 Unified Fragment Model

Modeled on LVQR: all input protocols produce a unified Fragment; all output protocols consume Fragments. One internal media type; adding a new protocol takes only ~50 lines of bridge code.

---

## 4. Reference Projects

| Project | Reference Points | Language |
|---------|------------------|----------|
| RustDesk | Remote desktop architecture, P2P hole punching, single binary multi-role | Rust |
| LVQR | Unified Fragment Model, multi-protocol adaptation, 29-crate workspace | Rust |
| str0m | sans-I/O WebRTC core | Rust |
| webrtc-rs | W3C-compatible WebRTC, runtime agnostic | Rust |
| MediaMTX | Protocol routing (zero transcoding), Media-over-QUIC | Go |
| Parsec | Low-latency remote desktop, GPU encoding pipeline | C++ |
| tether-rally | RTCDataChannel teleoperation binary protocol | JS/C++ |

---

## 5. Performance Benchmarks

| Scenario | Video G2G latency | Control latency | Notes |
|----------|-------------------|-----------------|-------|
| LAN/WiFi | ~100ms | <15ms | Ideal environment |
| 4G LTE | 150-250ms | 15-40ms | Cellular network |
| 5G NSA | 100-200ms | 10-30ms | Current commercial |
| 5G SA + MEC | <100ms | <10ms | Edge computing |
| 5G NR theoretical | <50ms | <1ms | Theoretical limit |
