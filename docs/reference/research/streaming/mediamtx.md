# MediaMTX Reference Analysis

> Generated: 2026-07-16 | Category: Streaming

## 1. Product Profile

- **Name**: MediaMTX (formerly rtsp-simple-server, renamed in 2023)
- **Developer**: Bluenviron (independent developer/organization; the primary author is a single core developer). The per-protocol libraries are maintained by the same organization: gortsplib (RTSP), gortmplib (RTMP), gohlslib (HLS), mediacommon (general-purpose media utilities)
- **First release**: December 2019 (rtsp-simple-server v0.0.0), renamed to MediaMTX v1.0.0 in 2023. Under continuous development for more than 6 years, 123+ releases
- **Product positioning**: Out-of-the-box, zero-dependency real-time media server and media proxy. Positioned as a "media router", not a "streaming platform". The core idea is automatic conversion between protocols — publishers push with any protocol, viewers watch with any protocol, no configuration required. No transcoding built in; pure routing
- **Target user groups**: IoT/IP camera deployers, video surveillance system integrators, streaming architects needing multi-protocol conversion, home security DIY users (Raspberry Pi cameras), small live-streaming platforms
- **License / business model**: MIT licensed, fully free and open source. No commercial edition. All dependencies are MIT or Apache 2.0 licensed

## 2. Technical Characteristics

### Overall architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                      MediaMTX (Go single binary)                  │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐     │
│  │                    Path Manager                           │     │
│  │  path → { source, publisher, readers[], auth, config }   │     │
│  │                                                           │     │
│  │  Each path corresponds to one media stream. Its lifecycle │     │
│  │  is driven by the following:                              │     │
│  │  - publisher connect/disconnect → path create/destroy     │     │
│  │  - static source config → path persists                   │     │
│  │  - sourceOnDemand → pull when readers exist, release      │     │
│  │    when no readers remain                                 │     │
│  └────────────┬──────────────────────┬───────────────────────┘     │
│               │                      │                             │
│  ┌────────────▼──────────┐  ┌────────▼──────────────────────┐     │
│  │   Publisher / Source   │  │        Reader / Consumer       │     │
│  │                        │  │                                │     │
│  │  ┌──────────────────┐  │  │  ┌──────────────────────────┐ │     │
│  │  │ Static Source     │  │  │  │ Protocol Auto-Conversion │ │     │
│  │  │ Continuously     │  │  │  │ Zero configuration,      │ │     │
│  │  │ pulls external   │  │  │  │ automatic conversion     │ │     │
│  │  │ RTSP/RTMP/SRT    │  │  │  │                          │ │     │
│  │  │ Reconnect on     │  │  │  │ push RTSP → watch WebRTC │ │     │
│  │  │ disconnect       │  │  │  │ push RTMP → watch LL-HLS │ │     │
│  │  └──────────────────┘  │  │  │ push SRT → watch HLS     │ │     │
│  │                        │  │  │ push WebRTC → watch RTSP │ │     │
│  │  ┌──────────────────┐  │  │  │ push MoQ → watch WebRTC  │ │     │
│  │  │ sourceOnDemand   │  │  │  └──────────────────────────┘ │     │
│  │  │ Lazy pull mode   │  │  │                                │     │
│  │  │ Pulls when       │  │  │  ┌──────────────────────────┐ │     │
│  │  │ viewers exist    │  │  │  │ Recording & Playback     │ │     │
│  │  │ Releases when    │  │  │  │ fMP4 / MPEG-TS segment   │ │     │
│  │  │ no viewers       │  │  │  │ retrieval via REST API   │ │     │
│  │  │ Save power/      │  │  │  │ playback mimics live     │ │     │
│  │  │ save bandwidth   │  │  │  └──────────────────────────┘ │     │
│  │  └──────────────────┘  │  │                                │     │
│  │  ┌──────────────────┐  │  │                                │     │
│  │  │ RPI Camera       │  │  │                                │     │
│  │  │ HW H.264/MJPEG   │  │  │                                │     │
│  │  │ Timestamp text   │  │  │                                │     │
│  │  │ overlay          │  │  │                                │     │
│  │  └──────────────────┘  │  │                                │     │
│  └────────────────────────┘  └────────────────────────────────     │
│                                                                   │
│  Internal data flow model:                                        │
│  Streams inside a path are managed by reader tracks. Each track   │
│  has a ring buffer holding the most recent N data units. When a  │
│  new reader connects, sending starts from the most recent key     │
│  frame in the ring buffer, guaranteeing instant playback          │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐     │
│  │ Protocol layer (each protocol has its own server/client)  │     │
│  │                                                           │     │
│  │  ┌──────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐ │     │
│  │  │  RTSP    │ │   RTMP    │ │   SRT     │ │  WebRTC   │ │     │
│  │  │gortsplib │ │ gortmplib │ │  libsrt   │ │ pion/     │ │     │
│  │  │[push/play]│ │[push/play]│ │[push/play]│ │ webrtc    │ │     │
│  │  │TCP/UDP   │ │  TCP      │ │  UDP      │ │WHIP/WHEP  │ │     │
│  │  └──────────┘ └───────────┘ └───────────┘ └───────────┘ │     │
│  │  ┌──────────┐ ┌───────────┐ ┌───────────┐               │     │
│  │  │ LL-HLS   │ │ MPEG-TS   │ │Media-over- │               │     │
│  │  │gohlslib  │ │   UDP     │ │   QUIC    │               │     │
│  │  │ [play]   │ │[push/play]│ │WebTransport│              │     │
│  │  │HTTP      │ │           │ │[push/play] │               │     │
│  │  └──────────┘ └───────────┘ └───────────┘               │     │
│  │  ┌──────────┐                                            │     │
│  │  │   RTP    │  UDP streaming (raw RTP)                  │     │
│  │  └──────────┘                                            │     │
│  └──────────────────────────────────────────────────────────┘     │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐     │
│  │ Extension layer                                          │     │
│  │  ┌────────────────┐  ┌──────────────┐  ┌──────────────┐ │     │
│  │  │ runOn*/exec     │  │ External     │  │ Read Replica │ │     │
│  │  │ hooks, FFmpeg   │  │ auth HTTP    │  │  Origin→Edge │ │     │
│  │  │ subprocess      │  │ callback     │  │              │ │     │
│  │  └────────────────┘  └──────────────┘  └──────────────┘ │     │
│  └──────────────────────────────────────────────────────────┘     │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐     │
│  │ Runtime API + Observability                              │     │
│  │ REST API: /v3/config/paths/*, /v3/recordings/*, config   │     │
│  │ Prometheus metrics, pprof, structured logging, healthz   │     │
│  └──────────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────────┘
```

### Path model in detail

The core abstraction of MediaMTX is the Path; each path is a lifecycle management unit for one media stream:

- **Path state machine**: Idle (no publisher, no source) → Ready (a publisher or source is providing data) → OnDemandWaiting (sourceOnDemand = true, viewers are waiting for the pull) → OnDemandPulling (currently pulling from the upstream stream) → Idle (publisher/source disconnected, all viewers unconnected)
- **Publisher mode**: an external client pushes to the path (RTSP ANNOUNCE/SETUP, RTMP publish, SRT caller, WHIP POST, MoQ ANNOUNCE)
- **Static Source mode**: the server actively pulls from an external stream (RTSP DESCRIBE/SETUP/PLAY, RTMP play, SRT listener), reconnecting continuously
- **sourceOnDemand mode**: only pulls from the upstream when viewers exist. After viewers disconnect, the connection is released after the onDemandCloseAfter timeout
- **Authentication and authorization**: each path can configure publisher and reader authentication separately. Supports external (HTTP callback auth), externalStart (callback only when the path starts), rtsp (RTSP Digest), jwt, internal (plaintext credentials in the config file)
- **IP allowlists**: IP/CIDR lists configured separately for publishing and reading

### Key technical capabilities

| Capability | Details |
|------|------|
| Protocol support | Inbound (push): RTSP (TCP/UDP, TLS), RTMP/RTMPS (TCP), SRT (UDP, AES-128/256 encryption), WebRTC/WHIP (UDP/TCP, DTLS-SRTP), Media-over-QUIC (QUIC/HTTP3 WebTransport), MPEG-TS over UDP, RTP over UDP. Outbound (playback): RTSP, RTMP, SRT, WebRTC/WHEP, LL-HLS, Media-over-QUIC, MPEG-TS |
| Codecs | Video: H.264, H.265(HEVC), AV1, VP9, VP8, MJPEG. Audio: Opus, MPEG-4 Audio (AAC), G.711 (PCMA/PCMU), LPCM, FLAC (v1.19.0+), G.726. **Pure passthrough mode — no decoding, no re-encoding** |
| Transport | TCP/UDP (native to each protocol), QUIC/HTTP3/WebTransport (Media-over-QUIC). RTSP supports automatic TCP/UDP switching (interleaved TCP/UDP transport fallback). RTP over UDP supports unicast and multicast |
| Recording | fMP4 (fragmented MP4) or MPEG-TS segmented recording to disk. Segmentation by time and file size. REST API retrieves the segment list by start/end time. Recording path and segment size are configurable |
| sourceOnDemand | Pulls from upstream only when viewers exist. Releases after a configurable delay following viewer disconnect. Suitable for battery-powered IP cameras or bandwidth-constrained scenarios |
| Playback | Replays recorded segments as a live stream. REST API selects the playback range by time span. Supports fast/slow playback and pause |
| API & hot reload | REST API for runtime management of paths and configuration. Config files can be hot-reloaded without interrupting existing connections (except for special cases such as port changes and encryption key changes) |
| Security | Argon2 password hashing. TLS 1.3 (HTTPS/RTMPS/RTSPS). Basic/Digest authentication. JWT. HTTP external authentication |
| RPI Camera | mediamtx-rpicamera module: Raspberry Pi camera hardware H.264 encoding and MJPEG encoding. Unified software/hardware encoding parameters (v2.8.0). Custom timestamp text overlay |

### Protocol conversion matrix

MediaMTX's automatic protocol conversion supports any protocol combination; the following combinations are verified:

| Publisher ↓ / Reader → | RTSP | RTMP | SRT | WebRTC | LL-HLS | MoQ | MPEG-TS |
|------------------------|:----:|:----:|:---:|:------:|:------:|:---:|:-------:|
| RTSP | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| RTMP | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| SRT | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| WebRTC (WHIP) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Media-over-QUIC | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| MPEG-TS (UDP) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| RTP (UDP) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

### Protocol conversion latency comparison

MediaMTX's protocol conversion requires no codec operations (pure passthrough), but end-to-end latency varies across protocol combinations depending on transport-layer characteristics:

| Push protocol | Playback protocol | End-to-end latency (LAN) | Bottleneck stage | Applicable MediaServo scenario |
|---------|---------|-------------------|---------|-------------------|
| RTSP (TCP) | RTSP (TCP) | <100ms | RTP unpack/pack | Surveillance camera → surveillance viewing |
| RTSP (TCP) | LL-HLS | 500ms-2s | HLS segment buffering | Surveillance camera → Web viewing |
| RTSP (TCP) | WebRTC | 100-200ms | ICE negotiation + RTP forwarding | Surveillance camera → real-time interaction |
| RTMP | LL-HLS | 500ms-2s | FLV→fMP4 conversion | General push → HLS distribution |
| RTMP | WebRTC | 100-300ms | FLV→RTP unpack | Push → low-latency interaction |
| SRT | WebRTC | 80-200ms | None (lowest-latency combination) | Remote push → Web playback |
| WebRTC | WebRTC | <80ms | None (peer-level RTP forwarding) | Bidirectional real-time calls |
| Media-over-QUIC | WebRTC | 50-150ms | QUIC→RTP bridging | Future low-latency links |
| MPEG-TS (UDP) | RTSP | 100-200ms | UDP multicast→unicast forwarding | Broadcast-grade signal distribution |

**Latency tiers**:

| Tier | Latency range | MediaMTX combination | MediaServo protocol routing strategy |
|------|---------|--------------|---------------------|
| Real-time | <150ms | RTSP→RTSP, WebRTC→WebRTC, SRT→WebRTC | Fragment→WHEP direct pass-through, zero buffering |
| Near-real-time | 150-500ms | RTSP→WebRTC, RTMP→WebRTC, QUIC→WebRTC | Fragment→WHEP, minimal jitter buffer |
| Low-latency | 500ms-2s | RTSP→HLS, RTMP→HLS, SRT→HLS | Fragment→CMAF→LL-HLS, 0.5s segment |
| Standard | 2-10s | Any→standard HLS/DASH | Fragment→CMAF→HLS/DASH, 3-6s segment |

### Transport layer protocol comparison

| Transport protocol | Multiplexing | Loss recovery | Connection migration | TLS encryption | Firewall traversal | MediaServo priority |
|---------|:-------:|:-------:|:-------:|:--------:|:---------:|:--------------:|
| TCP | ❌ (ordered) | Retransmission (head-of-line blocking) | ❌ | ✅ | ✅ | Phase 1 (baseline) |
| UDP | ❌ | Application layer handling | ❌ | DTLS | ⚠️ | Phase 1 (SRT/WebRTC) |
| QUIC (HTTP/3) | ✅ (parallel streams) | Per-stream retransmission | ✅ (connection migration) | ✅ (built-in) | ✅ (80/443) | Phase 4 (future) |
| WebTransport | ✅ (stream+datagram) | Per-stream retransmission + loss ignoring | ✅ | ✅ (built-in) | ✅ | Phase 4+ (cutting edge) |

The per-stream retransmission property of QUIC/WebTransport matters greatly for streaming: packet loss of one video frame does not block transmission of later frames (the TCP head-of-line blocking problem). MediaMTX v1.19.0 validated the feasibility of QUIC in streaming.

### Technology stack

- **Language**: Go (100%). Uses Go 1.22+ features
- **Core dependency libraries** (all authored and maintained by Bluenviron):
  - `gortsplib/v5`: RTSP 1.0 client and server library. Supports TCP/UDP/TLS transports. H.264 SPS/PPS parsing, RTP unpack/pack. v5 was a major refactor with significantly improved performance
  - `gortmplib`: RTMP client and server library. Supports RTMP/RTMPS. FLV parsing and generation
  - `gohlslib/v2`: HLS parsing and generation library. Supports LL-HLS, fMP4 segments, master playlist generation
  - `mediacommon/v2`: General-purpose media utilities. Codec parsing (H.264 NAL/H.265 NAL/AV1 OBU/VP9), fMP4 writing, bitrate calculation
  - `mediamtx-rpicamera`: Raspberry Pi camera hardware encoding module. Separate repository
- **WebRTC stack**: the pion project (Go implementation of the WebRTC standard, contributed by Google).
  - `pion/webrtc/v4` — WebRTC implementation (WHIP/WHEP)
  - `pion/srtp/v3` — SRTP (Secure RTP) encryption
  - `pion/dtls/v3` — DTLS (Datagram TLS) handshake
  - `pion/ice/v4` — ICE (Interactive Connectivity Establishment) NAT traversal
  - `pion/sctp` — SCTP (Stream Control Transmission Protocol) RTCDataChannel
  - `pion/turn/v5` — TURN service (only when webrtcAdditionalHosts is configured)
- **QUIC/WebTransport**: `quic-go/webtransport-go` v0.11.0. Supports QUIC/HTTP3 and the WebTransport API
- **Cryptography**: `argon2` (password hashing), `golang.org/x/crypto` (TLS/encryption)
- **Build/deploy**: Go cross-compiles to a single static binary (statically linked except for the libsrt C bindings). Docker images: ~27MB (amd64), ~26MB (arm64), ~28MB (armv6/v7). Supports darwin/amd64, darwin/arm64, linux/amd64, linux/arm/v6, linux/arm/v7, linux/arm64, windows/amd64
- **Observability**: Prometheus metrics (`/metrics` endpoint), Go pprof profiling (`/debug/pprof/*`), structured logging (slog), health check (`/healthz`)

## 3. Feature Overview

### Core feature modules

| Module | Function | Implementation |
|------|------|------|
| Path Manager | Manages all media paths. Maintains publisher/source/reader mappings. Drives the path lifecycle state machine. Handles authentication and configuration | `internal/core/path_manager.go` |
| Static Source | The server continuously connects to an external stream source. Supports RTSP/Rtmp/SRT as sources. Automatic reconnection on disconnect (exponential backoff). Supports source authentication | `internal/core/source_static.go` |
| sourceOnDemand | Lazy pulling. The first viewer triggers the pull; released after the last viewer disconnects. Configurable release delay | `internal/core/source_on_demand.go` |
| Protocol Auto-Conversion | Automatic protocol conversion. No configuration required. Media pushed by a publisher is automatically mapped to all enabled output protocols. Track-based ring buffer | `internal/core/path.go` (readPublisher method) |
| Recording | fMP4/MPEG-TS segmented recording. Segmentation by time or size. REST API retrieves segment lists. Recording path configurable | `internal/record/` |
| Playback | Plays recorded segments as live streams. REST API selects by time range. Supports pause/speed-up/slow-down | `internal/playback/` |
| Read Replica | Pulls and distributes from an origin instance. A replica node specifies the origin as its source via configuration. L4/L7 load balancing | `internal/core/source_static.go` (remote source) |
| Auth & ACL | External/RTSP Digest/JWT/Internal authentication. Per-path publish/read credentials. IP/CIDR allowlists | `internal/auth/` |
| RPI Camera | Raspberry Pi hardware H.264/MJPEG encoding. Unified configuration interface (v2.8.0). Timestamp text overlay. Performance optimizations | `mediamtx-rpicamera` (separate module) |
| API | REST API: path CRUD, recording segment queries, hot config reload, runtime status. Swagger/OpenAPI | `internal/api/` |
| Metrics | Prometheus metrics exporter. Path count, client count, publisher/reader count, bytes, recording segment count | `internal/metrics/` |

### Signature features

- **Zero-config automatic protocol conversion**: MediaMTX's killer feature. Publisher pushes RTSP → viewer watches via HLS. Publisher pushes RTMP → viewer watches via WebRTC. No conversion rules need to be configured. The internal track ring buffer plus codec passthrough realizes zero-overhead protocol conversion
- **Media-over-QUIC pioneer (v1.19.0, 2026-06-02)**: Among the first open-source media servers with native Media-over-QUIC support. Built on QUIC/HTTP3 transport and the WebTransport browser API. Slightly faster than WebRTC (no SCTP overhead), better loss recovery (native to QUIC), extra codec support (FLAC), lower routing complexity (single UDP port, no ICE/STUN negotiation)
- **sourceOnDemand lazy pulling**: An energy-saving mode for IoT/camera scenarios. Cameras transmit video only while a viewer is watching. With no viewers the network connection is fully released and the camera enters low-power mode. The release delay is configurable (prevents frequent connect/disconnect cycles). Extremely valuable for 4G/battery-powered surveillance cameras
- **RPI Camera hardware encoding**: Uses the hardware H.264/MJPEG encoder directly on the Raspberry Pi. v2.8.0 unified the configuration interface for software and hardware encoding (`rpiCameraH264Profile` and `rpiCameraH264Level` apply to both software and hardware paths). Performance optimization (frame size computed once)
- **RTSP tunneling through firewalls**: RTSP over HTTP and RTSP over WebSocket tunnels. Solves RTSP port blocking in enterprise networks. gortsplib supports both TCP (interleaved) and UDP transport, switching automatically
- **Read Replica horizontal scaling**: The simplest origin-replica architecture. The origin acts as a static source; replicas pull via remote paths. No cluster management system required (no Raft/Paxos/etcd). Read traffic is spread by an external L4 (TCP) or L7 (HTTP) load balancer

### Extensibility / plugin mechanism

MediaMTX has no traditional plugin system. Extension approaches:

- **Configuration-driven**: All behavior is controlled through YAML config files. Hot reload is supported (via `SIGHUP` signal or triggered by the API). Configuration overrides: global → protocol defaults → per-path
- **REST API**: A complete runtime management API. Path CRUD (POST/GET/PATCH/DELETE `/v3/config/paths/{name}`), recording segment list (GET `/v3/recordings/list`), recording segment detail (GET `/v3/recordings/get/{name}`), config reload (POST `/v3/config/reload`)
- **External authentication hook**: `externalAuthenticationURL` — HTTP POST callback authentication. The server sends the client's credentials and IP to an external service, which decides whether to allow the connection. Callback timeout/failure denies by default
- **runOn*/exec hooks**: Execute external commands on specific events. `runOnInit` (after the service starts), `runOnReady` (path ready), `runOnRead` (a viewer is present), `runOnRecordSegmentCreate` (recording segment created). Commonly used as a trigger for external FFmpeg transcoding
- **API events (WebSocket)**: Receive server events over WebSocket (path create/delete, publisher connect/disconnect, reader connect/disconnect). Well suited to building a custom admin UI
- **Go build tags**: Feature switches controlled by build tags at compile time. The RPI Camera module compiles only on ARM Linux
- **Static compilation**: All Go dependencies compile into a single binary. There is only one external C dependency (libsrt), which is also statically linked in the linker

## 4. Status & Ecosystem

- **Current version**: v1.19.2 (2026-06-28, latest stable), v1.19.0 (2026-06-02, adds Media-over-QUIC and FLAC support). Actively maintained, roughly one release per month. 123+ releases over 6 years
- **GitHub Stars / activity**: ~20,000 stars, 1,500+ forks. High commit frequency, fast issue response (usually replied to within 1-3 days). High code quality (good test coverage, well-developed CI/CD automation). The release workflow is fully public (GitHub Actions builds, blockchain-verified checksums)
- **Community size**: Mid-sized community. GitHub Issues/Discussions are active. Main use cases concentrate on IP cameras / IoT / surveillance / home security. A user community exists on Discord
- **Documentation / SDK / API ecosystem**:
  - Official website: mediamtx.org (complete documentation, including configuration guides for all protocols, quickstart, deployment guides, API reference, FAQ)
  - REST API documentation: complete OpenAPI/Swagger description. Every endpoint has curl and programming-language examples
  - Configuration documentation: every parameter has detailed description and defaults. YAML config comments are thorough
  - Docker Hub official images: bluenviron/mediamtx, multi-architecture (amd64/arm64/armv6/armv7). Built automatically, tags correspond to version numbers
  - Client integration examples: configuration examples for connecting OBS Studio, FFmpeg, GStreamer, VLC, WebRTC (pion/browser), RTSP (OpenCV/ffplay) clients to MediaMTX
  - No official SDK, but all protocols are standard. Standard client libraries for each protocol work as-is
  - Security and verification: Release checksums.sha256 + GitHub Attestations blockchain verification
- **Known defects or limitations**:
  - **Go GC latency risk**: under extremely high concurrency (tens of thousands of connections) Go GC can produce millisecond-scale pauses. A risk for sub-50ms low-latency scenarios
  - **No transcoding built in**: pure media router positioning. No internal codec engine. ABR adaptive bitrate always requires external FFmpeg/GStreamer. Managing subprocesses forked by the runOnReady hook is an operational burden
  - **No built-in cluster management**: Read Replica relies on manual configuration. No automatic service discovery, failover, or node orchestration
  - **Pure Go ecosystem limits**: cannot call GPU hardware encoding APIs (NVENC/VAAPI/VT). Unsuitable for embedded C FFI scenarios
  - **LL-HLS + CDN cache conflicts**: partial segments of low-latency HLS must bypass the CDN cache. Configuration differs per CDN provider and needs case-by-case handling
  - **Recording does not support cloud storage**: only local filesystem recording. No direct integration with S3/OSS/MinIO object storage
  - **MJPEG supported only by RPI Camera**: generic MJPEG stream handling and distribution is unimplemented. Supported only in the RPI Camera module via hardware encoders
  - **RTMP Enhanced (HEVC/AV1) support is incomplete**: gortmplib is a lightweight implementation, less complete in features than SRS/librtmp

## 5. Market Positioning

- **Primary application industries**: Video surveillance — IP camera stream forwarding and multi-protocol distribution (the most core scenario); IoT video device management — connecting edge device video streams to the cloud; home security DIY — Raspberry Pi cameras integrated with Home Assistant; small streaming platforms — live distribution relying on external transcoding; multi-protocol conversion gateways — upgrading legacy RTSP systems to WebRTC/HLS
- **Competitor comparison summary**:

| Dimension | MediaMTX | SRS | nginx-rtmp | Ant Media | Wowza | LiveKit |
|------|----------|-----|------------|-----------|-------|---------|
| Deployment complexity | ⭐ (single binary) | ⭐⭐ (needs TURN) | ⭐⭐ (needs nginx) | ⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐ |
| Docker size | ~27MB | ~100MB+ | ~50MB+ | ~200MB+ | ~500MB+ | ~150MB+ |
| Built-in transcoding | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| Cluster | Read Replica | Origin-Edge | push/pull | Built-in | Built-in | Built-in |
| Media-over-QUIC | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| RTSP support | ⭐⭐⭐⭐⭐ | ⚠️ | ❌ | ⭐⭐⭐ | ⭐⭐⭐⭐ | ❌ |
| on-demand pulling | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| RPI Camera optimization | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| IP camera scenario | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐ | ⭐⭐ | ⭐⭐⭐ | ❌ |
| ABR transcoding | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| Language | Go | C/C++ | C | Java | Java | Go/TS |
| Stars | 20K | 29K | 14K | 3K | N/A | 30K+ |
| License | MIT | MIT | BSD-2 | Commercial | Commercial | Apache 2.0 |
| Release year | 2019 | 2013 | 2012 | 2018 | 2007 | 2021 |

- **Pricing / license**: MIT, free and open source. No commercial edition. Release binaries are free to download

## 6. Product Highlights

1. **Zero-dependency single binary**: Go compiles to one static binary with no runtime dependencies. Docker image is 27MB (amd64). Deployment is download/start. No external Java/JRE (Ant Media/Wowza), no external nginx (nginx-rtmp). This is the ultimate expression of "simplicity"
2. **Automatic protocol conversion**: "Push one protocol, and it is automatically available on all protocols". No conversion rules or input/output pairs to configure. A publisher pushes RTSP, and viewers can play via HLS/WebRTC/SRT/RTMP. Zero-overhead conversion realized by the internal ring buffer plus codec passthrough
3. **sourceOnDemand lazy pulling**: Energy-saving optimization for IoT/battery-powered scenarios. Cameras work only while someone is watching. Resources are fully released when no viewer is present. This is a demand specific to the surveillance domain that no other general streaming server implements
4. **Media-over-QUIC pioneer**: Supported since v1.19.0 in June 2026. Ahead of all competitors — SRS (13-year project), nginx-rtmp (14-year project), Ant Media (8-year project). The QUIC transport layer is superior to TCP in loss recovery, connection migration, and multiplexing
5. **One-stop solution for IP camera scenarios**: RPI Camera hardware encoding + RTSP as the primary protocol + on-demand pulling + multi-protocol distribution. The complete chain from camera to viewer, with nearly zero configuration

## 7. Reference Value for MediaServo

### [Adopt] Directly Adoptable

1. **Path abstraction model**: the `path → { publisher/source, readers[], auth, config }` stream lifecycle management model maps directly onto MediaServo's Stream/Channel concepts. A path as the meeting point between publisher and reader fits MediaServo's Stream concept closely. The state machine design (Idle → Ready → OnDemand*) is also worth reusing directly
2. **sourceOnDemand on-demand pulling pattern**: one of the core capabilities for connecting surveillance cameras to MediaServo. The `start()`/`stop()` lifecycle methods of the `MediaSource` trait naturally support this pattern. Release the RTSP connection to the camera when no `StreamSubscriber` is connected
3. **Zero-configuration philosophy of automatic protocol conversion**: MediaMTX proves that explicit "RTSP → HLS" conversion rules need not be configured. MediaServo's Unified Fragment Model naturally supports this pattern: all inputs produce Fragments → all outputs consume Fragments → all protocol combinations are automatically available
4. **Read Replica horizontal scaling**: origin-replica is the simplest horizontal scaling pattern. No Raft/etcd required, achieved with an external LB. MediaServo Phase 1-2 can start with this pattern and upgrade to a fuller cluster design later
5. **Hot config reload**: updating configuration without interrupting existing connections is a basic requirement for production servers. MediaServo's PluginManager should be designed with runtime configuration reload capability (via a `ConfigUpdate` trait method or signal handling)
- **Version evolution**:
  - 2019-12: rtsp-simple-server v0.0.0 — RTSP push and pull only, written in Go
  - 2020-02: v0.2.0 — adds RTMP playback
  - 2021-05: v0.16.0 — adds HLS playback
  - 2022-03: v0.18.0 — adds WebRTC playback (pion/webrtc)
  - 2023-06: v1.0.0 — renamed MediaMTX, architecture comprehensively rebuilt
  - 2024-09: v1.15.0 — adds recording (fMP4/MPEG-TS)
  - 2025-11: v1.18.0 — adds WHIP pushing, Read Replica mode
  - 2026-06: v1.19.0 — Media-over-QUIC support, FLAC support (milestone release)
  - 2026-06: v1.19.2 — latest stable, QUIC stability improvements
- **Docker download volume**: over 200 million pulls (from public Docker Hub data), one of the most widely deployed containers in the streaming space
- **Sponsorship / dependency relations**: the core author is employed by Bluenviron for full-time MediaMTX development. All protocol libraries (gortsplib/gortmplib/gohlslib/mediacommon) are maintained by the same author

### [Adapt] Needs Modification Before Adoption

1. **Fusing the Path model with the Fragment model**: MediaMTX's path model lacks a unified internal media representation (internally it is only a codec-aware ring buffer). MediaServo should layer the Fragment model on top of the path model — the path manages stream lifecycle and permissions, the Fragment manages the media data representation and cross-protocol mapping
2. **Recording strategy**: MediaMTX records fMP4/MPEG-TS segments to local disk. MediaServo should use CMAF/fMP4 uniformly (consistent with the Fragment payload), support multiple recording backends (local, S3/OSS, NFS), and write directly from the FragmentObserver
3. **Integration approach for external transcoding**: the MediaMTX `runOnReady` hook + FFmpeg subprocess pattern is functionally correct but operationally complex. MediaServo should design a `Transcoder` trait supporting both built-in transcoding (preferred) and external transcoding (fallback) backends, with unified lifecycle management
4. **The idea behind unifying RPI Camera configuration**: MediaMTX v2.8.0's approach of unifying software/hardware encoding parameters is worth adopting. MediaServo's `HardwareEncoder` trait should provide a unified configuration interface that hides the differences between NVENC/VAAPI/VT/QSV
5. **RTSP tunneling**: RTSP over HTTP/WebSocket firewall traversal has value for surveillance scenarios. MediaServo's `RtspPlugin` should support multiple transports (TCP interleaved, UDP, HTTP tunnel)
6. **QUIC/WebTransport transport layer**: MediaMTX v1.19.0 validated the feasibility of QUIC in streaming. MediaServo Phase 4+ should make QUIC/WebTransport the unified transport layer to reduce the number of protocols

### [Avoid] Known Pitfalls / Non-applicable scenarios

1. **Go GC low-latency risk**: millisecond-scale GC pauses are unacceptable for sub-50ms scenarios. MediaServo's Rust core has no GC, which is the right choice
2. **Architectural limitation of not building in transcoding**: MediaMTX relies entirely on external processes for transcoding. MediaServo needs a built-in transmux path (Fragment container format conversion) and an optional encoding pipeline. It should not depend on external processes
3. **No GPU hardware encoding capability**: pure Go cannot call GPU APIs. MediaServo's screen capture and encoding path must call NVENC/VAAPI/VT directly via Rust FFI or `libloading`
4. **Read Replica has no failover**: no automatic failover. If MediaServo needs high availability, it needs a more complete cluster design (see LVQR's gossip cluster)
5. **Single publisher limit**: one publisher per path. MediaServo video conferencing needs multiple publishers → audiomixer/compositor → output. A room/conference abstraction must be added on top of the path model
6. **Bus factor of dependencies such as gortsplib**: all of MediaMTX's protocol libraries are maintained by a single person. If the author stops maintaining them, the entire ecosystem collapses

**Overall rating**: ★★★☆☆ (3/5)

MediaMTX is a reference for protocol routing — excellent in zero-configuration protocol conversion, IoT/surveillance edge scenarios, and QUIC transport. The Path abstraction and the sourceOnDemand pattern have direct reference value for MediaServo. But the Go GC limitation, lack of transcoding, and lack of GPU encoding make its implementation unsuitable for MediaServo's direct needs. Take its design ideas (Path management + automatic conversion + on-demand pulling) and reimplement them in Rust + Fragment Model.

---
**Related decisions**: D5 (Unified Fragment), D21 (timestamps), D-STREAM-TOPOLOGY, D152 (nginx hooks)

## Appendix A: sourceOnDemand pattern in detail

### A.1 State machine

The core of the sourceOnDemand pattern is a state machine with a timeout:

```
  set sourceOnDemand = yes
                          
    Idle  viewer connects  Connecting  pulling  Active
   (released) ----------->  (connecting to upstream) -------> (pushing)
       A                                      |
       |    all viewers disconnect + after timeout              |
       +--------------------------------------+
```

**State descriptions**:
- Idle: no viewers, no upstream connection. The camera may enter low-power mode
- Connecting: the first viewer arrived, connecting to the upstream source
- Active: viewers exist, the upstream connection is active, media data is being pushed
- Back to Idle: all viewers disconnected, released after the onDemandCloseAfter timeout

**Debounce mechanism**:
- onDemandCloseAfter defaults to 10s. Avoids immediate reconnection after a brief viewer disconnect
- If a viewer reconnects before the timeout, the state returns to Active and the connection is not released
- After the timeout the connection is released and the camera may enter sleep

### A.2 Impact on the camera

Battery-powered IP camera scenario:
- Idle state: the camera stops encoding and transmitting entirely, power consumption drops to standby
- Active state: the camera encodes and transmits normally, at normal working power
- If the surveillance scenario is watched only occasionally, sourceOnDemand can save 95%+ of power

### A.3 Configuration example

```yaml
paths:
  front_door:
    source: rtsp://192.168.1.100:554/stream
    sourceOnDemand: yes
    sourceOnDemandCloseAfter: 30s  # release 30s after the last viewer leaves
    sourceOnDemandRetryInterval: 5s  # reconnect interval
```

### [Adopt] Supplement — hot config reload design

**6. Runtime hot configuration reload**: MediaMTX supports hot-reloading the YAML configuration file via a SIGHUP signal or a POST API without interrupting existing connections. MediaServo's `PluginManager` needs a similar mechanism:

```rust
pub trait ConfigurablePlugin: Send + Sync {
    /// Apply a new configuration. Returns the diff of the changed configuration
    async fn apply_config(&self, new: &serde_json::Value) -> Result<ConfigDiff>;
    /// Snapshot of the current configuration
    async fn current_config(&self) -> Result<serde_json::Value>;
}
```

Handling the boundaries of hot reload: a port change requires restarting the listener, a codec change does not affect existing encoding sessions, a path configuration change takes effect immediately but does not kick out existing viewers. MediaServo's PipelineEngine should iterate over all plugins' apply_config() methods in reload_config().

### [Adapt] Supplement — authentication and authorization model

**7. Multi-level authentication**: MediaMTX supports per-path external/RTSP Digest/JWT/internal authentication, plus IP allowlists. MediaServo's authentication model should be simpler:

| Auth level | MediaMTX approach | MediaServo approach | Difference |
|---------|-------------|---------------|---------|
| Level 0 | No auth (noauth) | `AuthMode::None` | Internal testing, development environments |
| Level 1 | Static password (internal) | `AuthMode::Static` | Single token/password, simple deployments |
| Level 2 | HTTP callback (external) | `AuthMode::Webhook` | External auth service, supports RBAC |
| Level 3 | JWT | `AuthMode::Jwt` | Stateless tokens, suited to microservices |
| Level 4 | Multi-factor + IP ACL | `AuthMode::Composite` | Security-sensitive scenarios |

MediaServo should use an `AuthMode` enum + `AuthBackend` trait, allowing plugins to implement custom authentication backends (LDAP/OAuth/OIDC).

### [Avoid] Supplement — more known pitfalls

**7. Reachability problems with QUIC transport**: MediaMTX's Media-over-QUIC requires clients that support HTTP/3 (WebTransport API). In enterprise networks, QUIC (UDP 443) may be blocked by firewalls (TLS/TCP 443 allowed but UDP 443 denied). If MediaServo adopts QUIC transport, it must keep TCP/Long-Polling/WebSocket as fallback transport layers.

**8. Limitations of single path, single publisher**: MediaMTX allows only one publisher per path. That suits surveillance cameras and general pushing, but not video conferencing (which needs multiparty compositing). MediaServo needs a Room/Conference model layered on top of the Path model — a Room mixes audio from multiple publishers and composites video, producing a single Fragment stream output.

### Implications for MediaServo:
- mediaservo-surveillance SDK's CameraSource trait should support onDemand: bool
- start() is the pull action, stop() is the release action
- PipelineEngine needs to be aware of viewer connection count to trigger start/stop
- The debounce timeout must be configurable

---

## Appendix B: Read Replica architecture in detail

### B.1 Topology

```
               Load Balancer
              (L4 TCP / L7 HTTP)
                    |
          +---------+---------+
          |                   |
      Replica 1           Replica 2
       (read-only)         (read-only)
      pulls from Origin   pulls from Origin
          |                   |
          +---------+---------+
                    |
                  Origin
            (accepts push + recording + API)
```

### B.2 Configuration examples

**Origin (accepts push)**:
```yaml
paths:
  live_stream:
    # Publishers push here
```

**Replica (distributes playback)**:
```yaml
paths:
  live_stream:
    source: rtsp://origin-ip:8554/live_stream  # pull from Origin
    sourceOnDemand: yes  # optional
```

### B.3 Limitations and applicable scenarios

**Limitations**:
- All paths configured manually (no automatic synchronization)
- No automatic failover: if Origin goes down, all replicas lose the stream
- No automatic service discovery: LB configuration must be updated manually
- A Replica only distributes read traffic (no push/recording capability)

### Implications for MediaServo:
- Phase 1-2 can adopt Read Replica as a simple horizontal scaling solution
- Phase 3+ upgrade to an LVQR gossip cluster (automatic discovery + failover)
- mediaservo-cluster should implement two backend traits to support incremental upgrades

---

## Appendix C: MediaMTX core configuration parameters

MediaMTX configuration file structure:

```yaml
logLevel: info
rtspAddress: :8554
rtmpAddress: :1935
hlsAddress: :8888
webrtcAddress: :8889
srtAddress: :8890

paths:
  # Publisher mode
  live:
    runOnReady: ffmpeg -i rtsp://localhost:$RTSP_PORT/$MTX_PATH ...
    runOnReadyRestart: yes

  # Static Source mode
  ip_camera:
    source: rtsp://admin:password@192.168.1.64:554/stream
    sourceProtocol: tcp
    sourceOnDemand: no

  # SourceOnDemand mode
  battery_camera:
    source: rtsp://192.168.1.65:554/stream
    sourceOnDemand: yes
    sourceOnDemandCloseAfter: 1m

  # Recording configuration
  recorded_stream:
    record: yes
    recordPath: /recordings/$MTX_PATH/%Y-%m-%d_%H-%M-%S
    recordFormat: fmp4
    recordSegmentDuration: 1h
```

Core parameter notes:
- sourceProtocol: tcp forces TCP transport, avoiding UDP issues with certain cameras
- The runOnReady hook executes a shell command when a path becomes ready, commonly used to trigger transcoding
- recordSegmentDuration sets the recording segment length, avoiding overly large single files
- The sourceOnDemand + sourceOnDemandCloseAfter combination implements energy-saving mode

### Implications for MediaServo:
- Configuration format should consider YAML (human-friendly) rather than INI (SRS) or TOML (Xiu)
- The runOnReady hook pattern is a practical solution for simple scenarios; production applications should use trait abstraction
- Basic recording configuration (path/format/segment duration) is part of the MVP
- Protocol ports should be disableable (enable only the protocols needed) to reduce the attack surface
