# SRS Reference Analysis
> Generated: 2026-07-16 | Category: Streaming

## 1. Product Profile
- **Name**: SRS (Simple Realtime Server), branded SRS/8.0 (from May 2026)
- **Developer**: OSSRS open-source community. Core maintainer winlinvip (Yang Chengli, full-time on SRS development for over 10 years) and 120+ contributors. The community includes many Chinese internet developers
- **First release**: 2013 (v1.0), continuously developed for over 13 years. SRS/8.0 is the current main version (released 2026-05-17), the 8th major version since 2013
- **Product positioning**: a simple, efficient real-time video server. A single binary simultaneously providing multi-protocol support for RTMP/WebRTC/HLS/HTTP-FLV/HTTP-TS/SRT/MPEG-DASH/GB28181. "Simple" is the core philosophy — for deployment (one command), configuration (an INI file), and usage (HTTP API + built-in player) alike. Simultaneously supports high performance (coroutine concurrency, transmux-only, zero transcoding) and low latency (WebRTC 80ms, SRT 0.5s)
- **Target audience**: live platform developers (heavily used by Chinese internet live companies — Douyu/Huya/Kuaishou/Douyin etc.), traditional streaming architects needing RTMP compatibility, self-built small live solutions (education/enterprise/surveillance), video surveillance system integrators (GB28181 device ingestion)
- **License / business model**: MIT license (the SRS/8.0 "code Free" edition). The core code is completely free and open source. Revenue comes from the Oryx (SRS Stack) management platform (integrated Docker deployment + Web UI) and commercial technical support. Sponsors include Agora and others

## 2. Technical Characteristics
### Overall architecture
```
┌──────────────────────────────────────────────────────────────────┐
│                    SRS (C/C++ coroutine architecture)            │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │          ST (State Threads) coroutine scheduler           │  │
│  │                                                          │  │
│  │  Single process, multiple coroutines. Each network        │  │
│  │  connection is bound to one ST coroutine                  │  │
│  │  Synchronous programming model (no callback nesting).     │  │
│  │  User-space threads, million-scale concurrency            │  │
│  │  Full architecture support: Linux/macOS/ARM/RISCV/        │  │
│  │  LOONGARCH/MIPS                                           │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
│  ┌───────────────────────────────────────────────────────┐     │
│  │  Internal unified data path: RTMP stream ← core design  │     │
│  │                                                       │     │
│  │  All input protocols ──▶ internal RTMP stream ──▶ all   │     │
│  │  output protocols                                    │     │
│  │                                                       │     │
│  │  RTMP (push) ──┐                                     │     │
│  │  WebRTC (push) ─┤        ┌───────────┐               │     │
│  │  SRT (push)    ─┼───────▶│internal RTMP├───────────┐  │     │
│  │  GB28181(push) ─┘        └───────────┘           │  │     │
│  │                                                  ▼  │     │
│  │                             ┌──────────────────────┐ │     │
│  │                             │ transmux (zero transcode)│ │  │
│  │                             │                      │ │     │
│  │                             │ RTMP → HLS           │ │     │
│  │                             │ (demux FLV → mux TS)  │ │     │
│  │                             │ RTMP → HTTP-FLV      │ │     │
│  │                             │ (zero overhead, direct forward)││
│  │                             │ RTMP → DASH          │ │     │
│  │                             │ (demux FLV → mux fMP4)│ │     │
│  │                             │ RTMP → SRT           │ │     │
│  │                             │ (demux FLV → mux MPEG-TS)│ │  │
│  │                             │ WebRTC → RTMP         │ │     │
│  │                             │ (depkt RTP → mux FLV) │ │     │
│  │                             └──────────────────────┘ │     │
│  └───────────────────────────────────────────────────────┘     │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Protocol input layer                                    │  │
│  │  ┌────────┐ ┌──────────┐ ┌───────┐ ┌─────────┐          │  │
│  │  │  RTMP  │ │  WebRTC  │ │  SRT  │ │ GB28181 │          │  │
│  │  │ TCP    │ │WHIP/WHEP │ │  UDP  │ │SIP+RTP  │          │  │
│  │  │ 1935   │ │DTLS-SRTP │ │       │ │         │          │  │
│  │  │        │ │ICE-Lite  │ │Go sep.│ │         │          │  │
│  │  │        │ │ext.TURN  │ │process │ │         │          │  │
│  │  └────────┘ └──────────┘ └───────┘ └─────────┘          │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Protocol output layer (all transmuxed from the internal  │  │
│  │  RTMP stream)                                             │  │
│  │  ┌──────┐ ┌──────┐ ┌─────┐ ┌──────┐ ┌─────┐ ┌────┐     │  │
│  │  │ RTMP │ │ HLS  │ │HTTP-│ │WebRTC│ │ DASH│ │ SRT │     │  │
│  │  │ play │ │.m3u8 │ │ FLV │ │ WHEP │ │.mpd │ │ play│     │  │
│  │  │      │ │ +.ts │ │ HTTP│ │ RTP  │ │     │ │    │     │  │
│  │  │ TCP  │ │      │ │ long │ │ UDP  │ │ HTTP│ │ UDP │     │  │
│  │  └──────┘ └──────┘ └─────┘ └──────┘ └─────┘ └────┘     │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Origin-Edge cluster                                      │  │
│  │                                                           │  │
│  │  push → Origin ──┬──▶ Edge 1 ──▶ viewers (RTMP/FLV/WebRTC)│  │
│  │                 ├──▶ Edge 2 ──▶ viewers                   │  │
│  │                 └──▶ Edge N ──▶ viewers                   │  │
│  │                                                           │  │
│  │  Multi-level edge nodes (Edge-of-Edge). RTMP/HTTP-FLV/    │  │
│  │  WebRTC distribution                                      │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  HTTP Callback event system                               │  │
│  │  on_publish · on_unpublish · on_play · on_stop           │  │
│  │  on_dvr · on_hls · on_rtc_play                           │  │
│  │  → HTTP POST to an external service → auth/billing/       │  │
│  │  notification/auto-transcoding trigger                     │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Observability                                            │  │
│  │  Prometheus metrics exporter · Grafana Dashboard         │  │
│  │  HTTP API (runtime control) · built-in WebRTC/HLS player  │  │
│  │  console                                                 │  │
│  │  Docker Hub · Helm Chart (Kubernetes)                    │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
│  SRS/8.0 note: MIT license, code Free, no paid features        │
└──────────────────────────────────────────────────────────────────┘
```

### Internal RTMP normalization model

SRS's most core design decision is normalizing all input protocols into an internal RTMP stream:

**Normalization flow**:
1. **RTMP push** → parse FLV tags directly → internal RTMP stream (zero conversion)
2. **WebRTC WHIP push** → depacketize RTP → decode H.264/Opus → re-encode to FLV → internal RTMP stream (with codec overhead!)
3. **SRT push** → (separate SRT process) → FLV → TCP pipe → internal RTMP stream (cross-process bridge)
4. **GB28181 push** → depacketize PS/RTP → extract H.264/AAC → mux FLV → internal RTMP stream

**Transmux after normalization**:
- RTMP → HTTP-FLV: zero overhead (FLV tags forwarded as-is), lowest latency (1-3s)
- RTMP → HLS: demux FLV → mux TS segments + generate the m3u8 playlist, latency 3-10s (standard) / 2-3s (LL-HLS)
- RTMP → DASH: demux FLV → mux fMP4 segments + generate the MPD manifest, latency 3-10s
- RTMP → SRT: demux FLV → mux MPEG-TS → SRT push, latency 0.5-2s
- RTMP → WebRTC (WHEP): demux FLV → decode → repackage as RTP → WebRTC playback (with codec overhead)

### Key technical capabilities
| Capability | Details |
|------|------|
| Protocol support | Input: RTMP/RTMPS (TCP 1935), WebRTC/WHIP (UDP/TCP), SRT (UDP, separate process), GB28181 (SIP+RTP). Output: RTMP, WebRTC/WHEP, HLS (m3u8+ts), HTTP-FLV, HTTP-TS, MPEG-DASH (MPD+fMP4), SRT |
| Coding | H.264, H.265(HEVC) — Enhanced RTMP, AV1 — Enhanced RTMP, VP9/VP8 — WebRTC. Audio: AAC, Opus, G.711 (PCMA/PCMU), MP3 |
| Transport | RTMP: TCP (1935) + TLS (RTMPS); SRT: UDP + AES-128/256; WebRTC: UDP/DTLS-SRTP + ICE-Lite + external TURN; HLS: HTTP/HTTPS; HTTP-FLV: HTTP long connection; DASH: HTTP |
| Conversion | **transmux only (core design philosophy)**: RTMP → HLS / HTTP-FLV / DASH / SRT. Container format conversion only, no decode/encode. Extremely low CPU overhead |
| Recording | FLV format recording to disk. Segmented by stream and by time. The on_dvr HTTP callback notifies segment creation. DVR supports a sliding window |
| Security | RTMP token authentication (`?token=<md5_hash>` URL parameter), HTTP callback authentication, WebRTC DTLS-SRTP encryption, SRT AES-128/256 encryption |
| Transcoding | **Not built in**. Depends on an external FFmpeg process (triggered by the on_publish HTTP callback). Receives the transcoded stream as a different stream name |
| LL-HLS | Low-latency HLS, segment duration 1-2s, latency 2-3s (vs standard HLS 3-10s) |

### Performance metrics
| Test item | Data | Conditions |
|--------|------|------|
| RTMP latency | 0.4-3s | H.264 only: 0.4s; H.264+AAC: 0.6s; standard: 0.8-3s |
| WebRTC latency | 80-400ms | 80ms after optimization (SRS 4.0.87); SRS 2.0.72 benchmark 80ms |
| HLS latency | 3-10s (standard) / 2-3s (LL-HLS) | Segment duration 1-6s |
| HTTP-FLV latency | 1-3s | HTTP long connection, FLV tag passthrough |
| RTMP playback concurrency | thousands of sessions | Single machine 4 cores, no transcoding |
| WebRTC playback concurrency | 500-1000+ | 2 vCPU, external TURN required |
| Optimal HLS fragment | 3s | The balance point of latency and stability |
| CPU usage | Extremely low (transmux only) | One core can handle hundreds of RTMP connections |
| Lines of code | about 170,000 | SRS v6.0-r0 |

### Deep protocol comparison — SRS vs Fragment Model

SRS's protocol conversion matrix uses the RTMP stream as its intermediate representation; interaction with non-RTMP protocols has the following performance characteristics:

| Protocol path | SRS handling | Codec overhead | Latency range | MediaServo Fragment handling | Latency improvement |
|---------|-------------|:---------:|:-------:|---------------------------|:--------:|
| RTMP→HLS | FLV→TS transmux | none | 3-10s | Fragment→CMAF→LL-HLS | 50% improvement (1-3s) |
| RTMP→HTTP-FLV | FLV passthrough | none | 1-3s | Fragment→FLV tag generation | on par |
| RTMP→WHEP | FLV→decode→RTP | **yes** (quality lost) | 200-500ms | Fragment→RTP packaging (passthrough) | **improved** (no quality loss) |
| WHIP→RTMP | RTP→decode→FLV | **yes** (double encoding) | 200-500ms | Fragment passthrough (back to WHIP) | **improved** (no encoding) |
| WHIP→WHEP | RTP→FLV→decode→RTP | **yes** (2 codec passes) | 300-800ms | Fragment→RTP passthrough | **90% improvement** |
| SRT→HLS | MPEG-TS→FLV→TS | none (double container conversion) | 3-10s | Fragment→CMAF→HLS | improved (single step) |
| GB28181→HLS | PS→FLV→TS | none | 3-10s | PS→Fragment→CMAF→HLS | improved (single step) |

**Key findings**:
1. WHIP→WHEP in SRS involves 2 codec passes (the original best quality is already lost); it is the biggest performance pain point of the SRS architecture
2. In the Fragment Model, WHIP→WHEP is a pure RTP passthrough path with no codec operations at all
3. SRT→HLS in SRS passes through two container conversions, MPEG-TS→FLV→TS; the Fragment Model needs only one step, Fragment→CMAF

### Concurrency performance and resource consumption comparison

| Scenario | SRS (RTMP normalization) | LVQR (Fragment Model) | Difference |
|------|-----------------|----------------------|------|
| 1000 sessions RTMP→HLS | 1000 transmux channels (zero CPU) | 1000 CMAF generation channels (zero CPU) | comparable |
| 100 sessions WHIP→WHEP | 100 codec passes (100% CPU+GPU) | 100 Fragment passthroughs (<1% CPU) | SRS 100x higher |
| 500 sessions RTMP→HTTP-FLV | 500 FLV passthroughs (zero CPU) | 500 FLV tag generations (zero CPU) | comparable |
| Adding RTMP→MoQ support | requires an FLV→MoQ Object adapter (complex) | already supported (Fragment→MoQ) | Fragment advantage |
| GPU requirement | GPU mandatory for WHIP to RTMP | not needed (zero codec) | Fragment advantage |

### Technology stack
- **Language breakdown**: C/C++ (87.5% — core server), Go (5.5% — the SRSX proxy module, some tools), JavaScript (3.0% — the WebRTC player srsRTCPlayer.js, the HLS/FLV player srsPlayer.js, the admin console), Shell/Python/Lua (ops scripts)
- **Coroutine framework**: ST (State Threads) — a lightweight user-space coroutine library. Developed by Netscape in 2001, single-process multi-coroutine model. One coroutine per network connection. Context switching ≈ the cost of a function call. No async programming framework needed. Downsides: not a standard library, no native debugger support, small community
- **Internal dependencies**: self-developed first, external dependencies minimized.
  - RTMP protocol stack: a complete self-developed implementation (13 years of refinement). Enhanced RTMP support (HEVC/AV1/multi-track). FLV parsing and generation
  - WebRTC module: a self-developed RTC module. DTLS-SRTP encryption (based on OpenSSL/mbedTLS). ICE-Lite (simplified server-side ICE, no STUN negotiation needed). No built-in TURN/STUN services
  - SRT: a separate Go process module (hybrid model), communicating with the main process through an internal TCP pipe, passing FLV streams. Uses the libsrt C library
  - GB28181: a self-developed SIP signaling + RTP/PS depacketizing module
- **Configuration system**: INI-format configuration files (`conf/srs.conf`). Per-vhost and per-app configuration isolation. Supports the include directive to reference multiple configuration files
- **Monitoring and operations**: Prometheus metrics exporter (HTTP `/metrics`), Grafana JSON dashboard templates, HTTP API runtime status queries and control, built-in WebRTC/HLS/HTTP-FLV players (testing and demos), containerized deployment (Docker Hub + Helm Chart)
- **Multi-architecture support**: x86_64, ARMv7 (32-bit ARM), AARCH64 (ARM64/Apple M1/M2), RISCV (RISC-V 64-bit), LOONGARCH (Loongson), MIPS (MIPS64). Domestic CPU support is an important competitive strength in its Chinese market
- **Client ecosystem**: srsRTCPlayer.js (WebRTC player, provided via GitHub releases), srsPlayer.js (HTTP-FLV/HLS player), community SDKs (Java/C#/Python/Go), Oryx/SRS Stack (an integrated Docker management platform + Web UI)

## 3. Feature Overview
### Core feature modules
| Module | Function | Implementation language |
|------|------|----------|
| RTMP Core | A complete RTMP/RTMPS implementation. Push + playback. Enhanced RTMP: H.265/AV1/multi-track. GOP cache: caches the most recent GOP so new viewers get a picture immediately. FLV tag passthrough (HTTP-FLV output) | C/C++ |
| WebRTC | WHIP push + WHEP playback. DTLS-SRTP. ICE-Lite (simplified ICE). Requires an external TURN for NAT traversal. Latency 80-400ms | C/C++ |
| HLS | Real-time HLS generation: FLV → TS segments + m3u8 playlist. Standard HLS (3-10s) + LL-HLS (2-3s). HLS over HTTPS | C/C++ |
| HTTP-FLV | HTTP long-connection FLV stream distribution. Latency 1-3s. Zero overhead (FLV tag passthrough). Not supported on iOS Safari | C/C++ |
| DASH | FLV → fMP4 segments + MPD manifest generation. ABR multi-bitrate (manually configured multi-stream MPD). Latency comparable to HLS | C/C++ |
| SRT | A separate Go process. Internal TCP pipe connecting to the main process. Supports push and playback. AES encryption. Latency 0.5-2s | Go (separate process) |
| GB28181 | The national GB/T 28181 standard. SIP signaling (register/keepalive/Invite) + RTP/PS streaming. Surveillance camera ingestion. The only open-source implementation | C/C++ |
| Origin-Edge Cluster | Origin push → Edge distribution. Multi-level Edge. Supports RTMP/HTTP-FLV/WebRTC. Simple configuration | C/C++ |
| HTTP Callback | Stream lifecycle event notifications. on_publish/unpublish/play/stop/dvr/hls. HTTP POST calls to external services. Auth/billing/transcode triggering | C/C++ |
| Transmux | Pure container format conversion. No decode/encode. RTMP → HLS/DASH/HTTP-FLV/SRT. Extremely low CPU overhead | C/C++ |
| Security | RTMP token authentication. HTTP callback authentication. DTLS-SRTP (WebRTC). AES encryption (SRT). Per-vhost/app authentication rules | C/C++ |
| Monitoring | Prometheus metrics. HTTP API. Built-in WebRTC/HLS player debug tools. Grafana Dashboard | C/C++ + JS |

### Signature features
- **A deep implementation of the RTMP protocol**: an RTMP protocol stack refined continuously for 13 years. Enhanced RTMP supports HEVC/AV1 coding and multiple audio/video tracks. OBS Studio uses SRS as its RTMP compatibility benchmark. This deep accumulation is the result of long-term investment
- **The simplicity of the ST coroutine architecture**: single process, multiple coroutines. The synchronous programming model (no async/await keywords, no Future/Promise). One coroutine per connection, so the code is as simple to write as synchronous code, while the bottom layer is efficient coroutine scheduling. Unique in the C/C++ ecosystem
- **The Transmux zero-transcode philosophy**: no decode/encode, only container format conversion. Extremely low CPU overhead (a single 4-core machine can handle thousands of RTMP sessions). This decision is the optimal architecture in scenarios that do not need ABR adaptive bitrate
- **The only open-source support for the GB28181 national standard**: a native open-source implementation of the Chinese surveillance industry standard protocol. SIP signaling + RTP/PS media transport. Out-of-the-box support for GB28181 devices (Chinese surveillance cameras). This is SRS's unique competitive strength in the Chinese surveillance market
- **A comprehensive Chinese community ecosystem**: complete Chinese documentation, tutorials, Wiki, GitHub Issues (Chinese-friendly), WeChat groups. For Chinese users, SRS is the open-source streaming server with the lowest barrier to entry. Oryx (SRS Stack) provides a one-click deployment web management interface
- **Multi-architecture CPU support**: x86_64, ARMv7, AARCH64 (Apple M1/M2), RISCV, LOONGARCH (Loongson), MIPS. Domestic CPU support gives it a unique advantage in the Chinese xinchuang market

### Extensibility / plugin mechanism
SRS has no official plugin system. Extension mechanisms:
- **HTTP Callback event system**: defines a set of stream lifecycle events (on_publish, on_unpublish, on_play, on_stop, on_dvr, on_hls, on_rtc_play). When an event occurs, an HTTP POST is made to the configured URL. Used for:
  - Authentication: verify tokens and permissions in the on_publish/on_play callback, controlling allow/deny via the HTTP status code
  - Billing: record viewing duration in the on_stop callback
  - Notification: notify that recording is complete in the on_dvr callback
  - Automatic transcoding: trigger an FFmpeg subprocess for ABR transcoding in the on_publish callback
- **External FFmpeg transcoding**: automatically starts an FFmpeg process via the on_publish HTTP callback. FFmpeg outputs new RTMP streams (such as `stream_720p`, `stream_480p`). SRS receives these streams as independent paths. Then the HLS master playlist or DASH MPD is manually configured to include multiple bitrates
- **Origin-Edge cluster**: the upstream Origin of Edge nodes is specified in the configuration file. Edge nodes connect to the Origin on startup and pull the stream. Dynamic stream allocation (Edge pulls the stream from Origin only when a viewer requests it)
- **HTTP API**: runtime queries (stream list, client list, server status, statistics), runtime control (kick publishers/viewers), configuration queries
- **SRT hybrid module (hybrid model)**: SRT runs as a separate Go process. It communicates with the C/C++ main process over an internal TCP connection. This is a historical legacy design and not the recommended extension model

## 4. Status & Ecosystem
- **Current version**: SRS/8.0 (2026-05-17, code Free). The previous latest tagged release was v6.0-r0 (2025-12-03, 170,962 lines of code). Semantic versioning has not been used since v6.0; it switched to brand versioning
- **GitHub Stars / activity**: about 29,000 stars, 5,600+ forks. Sustained high activity, 120+ contributors. About 45 open issues. Core maintainer winlinvip has worked full-time on SRS development for over 10 years. Release cycle: v5.x about one release per quarter, v6.x about one alpha/beta per month
- **Community size**: possibly the most active open-source streaming community on the Chinese internet. The Chinese documentation is thorough (Wiki + the official documentation sites ossrs.io/ossrs.net). English documentation is also continuously improving. Reddit, GitHub Issues/Discussions are active. WeChat groups. An Oryx developer community
- **Documentation / SDK / API ecosystem**:
  - Official websites: ossrs.io (international), ossrs.net (China). Complete Chinese and English documentation
  - Wiki: the GitHub Wiki contains detailed architecture documents, configuration guides, performance test reports, FAQ
  - HTTP API documentation: a complete REST API reference
  - Configuration documentation: detailed descriptions for every parameter. Rich example configuration files
  - Client SDKs: srsRTCPlayer.js (WebRTC player), srsPlayer.js (HTTP-FLV/HLS player). Community-contributed Java/C#/Python/Go SDKs
  - Player compatibility: FFmpeg (push/pull), OBS Studio (push), VLC (RTMP/HLS playback), browser WebRTC (WHEP playback)
  - Operations tooling: Prometheus + Grafana monitoring templates, Docker Compose (docker-compose.yml), Helm Chart (Kubernetes)
  - Oryx (SRS Stack): an integrated management platform. One-click Docker deployment. A web UI management interface. Supports starting a complete live platform with one docker run command
- **Known defects or limitations**:
  - **The internal model is fixed as the RTMP stream (the biggest architectural limitation)**: all inputs are normalized to RTMP and all outputs are derived from RTMP. Adding a new protocol (such as MoQ/QUIC's subgroup and object concepts) requires twisting the adaptation to RTMP semantics. This is SRS's Achilles' heel
  - **No QUIC/Media-over-QUIC support**: MediaMTX v1.19.0 already supports it, SRS has no plans. An RTMP-centric architecture is hard to adapt to QUIC's multiplexing and object model
  - **No built-in TURN/STUN**: WebRTC NAT traversal depends entirely on an external coturn server. This adds deployment complexity and failure points
  - **The SRT hybrid process model**: a separate Go process + TCP pipe bridging adds latency, maintenance complexity, and failure points
  - **No GUI for cluster management**: Origin-Edge configuration is all specified manually in configuration files. No graphical management or dynamic orchestration
  - **An aging C code base**: the core code dates from 2013, 170k lines of C/C++. Accumulated technical debt. High difficulty for newcomers to start
  - **Relatively high HLS latency**: LL-HLS's 2-3s latency seems slow in the WebRTC era (<100ms)
  - **WebRTC push to RTMP has codec overhead**: WHIP push (H.264/Opus) → decode → re-encode to FLV (H.264/AAC) → RTMP. This breaks the pure transmux performance advantage
  - **Complex ABR configuration**: multi-bitrate distribution requires manually configured FFmpeg transcoding + manually editing the HLS master playlist. No automated ABR ladder

## 5. Market Positioning
- **Main application industries**: Chinese internet live streaming — platforms such as Douyu/Huya/Kuaishou/Douyin/Bilibili heavily use SRS as edge/relay servers (the largest user base); online education live streaming — real-time interaction on course platforms; corporate live/training — internal training streams; surveillance — GB28181 camera ingestion and stream distribution; small and medium live platforms overseas — RTMP/HLS/LHLS distribution
- **Competitive comparison summary**:
| Dimension | SRS | MediaMTX | nginx-rtmp | Ant Media | Wowza | LiveKit |
|------|-----|----------|------------|-----------|-------|---------|
| Core language | C/C++ | Go | C | Java | Java | Go/TS |
| Number of protocols | 8 | 7 | 3 | 6+ | 10+ | 1 (WebRTC) |
| Internal model | **RTMP normalization** | Path + no unified model | no unified model | RTMP normalization | multi-format | Room/Track |
| Built-in transcoding | ❌ | ❌ | ❌ | ✅ (H.264) | ✅ (multi-format) | ✅ |
| MoQ/QUIC | ❌ | ✅ v1.19 | ❌ | ❌ | ❌ | ❌ |
| Cluster | Origin-Edge | Read Replica | push/pull | built-in cluster | built-in cluster | built-in SFU |
| TURN built in | ❌ (requires coturn) | ❌ | ❌ | ✅ | ✅ | ✅ |
| GB28181 | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| RTMP depth | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ❌ |
| HLS optimization | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐ | ❌ |
| Stars | 29K | 20K | 14K | 3K | N/A (closed source) | 30K+ |
| Chinese support | ⭐⭐⭐⭐⭐ | ⭐ | ⭐ | ⭐ | ⭐ | ⭐ |
| License | MIT | MIT | BSD-2 | commercial | commercial | Apache 2.0 |
| Release year | 2013 | 2019 | 2012 | 2018 | 2007 | 2021 |
| Code size | 170k lines | ~50k lines | ~30k lines | ~200k lines | ~500k lines | ~100k lines |
- **Pricing / license**: MIT free open source (SRS/8.0 code Free). The Oryx (SRS Stack) platform has a free tier and a paid tier (advanced features such as multi-user management, recording management, statistics panel). Enterprises can purchase commercial technical support

## 6. Product Highlights
1. **The pinnacle of open-source RTMP protocol implementations**: 13 years of continuous refinement. Enhanced RTMP (HEVC/AV1/multi-track) ahead of the industry. OBS Studio as the compatibility benchmark. 170k lines of C/C++ carry all the lessons of the RTMP era
2. **Engineering practice of the ST coroutine architecture**: single process, multiple coroutines; the synchronous programming model avoids the complexity of C/C++ async programming. In 2013 it already delivered today's async/await user experience, but with a 2001 coroutine library
3. **The Transmux zero-transcode architecture**: no decode/encode, only container format conversion. A single 4-core machine can handle thousands of RTMP sessions. Extremely low CPU usage. No GPU needed. This is the optimal architecture for simple scenarios
4. **The only open-source support for the GB28181 national standard**: the national standard protocol of the Chinese surveillance industry. SIP signaling + RTP/PS media transport. The standard open-source solution for Chinese surveillance camera ingestion
5. **A comprehensive Chinese community ecosystem**: possibly the open-source streaming server with the best Chinese documentation in the world. A complete Chinese Wiki, blogs, WeChat groups. Oryx one-click deployment web UI. Zero-barrier onboarding
- **Version evolution**:
  - 2013: SRS v1.0 — RTMP only, C language, the ST coroutine architecture established
  - 2015: SRS v2.0 — HLS support, HTTP Callback, DVR recording
  - 2017: SRS v3.0 — HTTP-FLV, multi-process architecture, DASH support
  - 2020: SRS v4.0 — WebRTC (WHIP/WHEP) support (a major milestone), SRT support (Go separate process)
  - 2021: SRS v5.0 — H.265/AV1 Enhanced RTMP, LL-HLS, GB28181 (surveillance)
  - 2023: SRS v6.0 — Centrifuge cascading cluster, SRT push, initial WebTransport
  - 2025: SRS v6.0-r0 — 170,962 lines of code, the last semantic version
  - 2026-05: SRS/8.0 — brand rename, MIT licensed "code Free" edition
- **Contributor structure**: 120+ contributors, core maintainer winlinvip developing full-time for over 10 years. Corporate sponsors include Agora and other Chinese real-time audio/video companies
- **Chinese market share**: about 30-40% of the RTMP server market in Chinese internet live streaming scenarios (estimated); Oryx (SRS Stack) Docker image downloads in the millions

## 7. Reference Value for MediaServo
### [Adopt] Directly Adoptable
1. **The Transmux zero-transcode fast path**: MediaServo should support a "fast transmux path": when the Fragment payload is fMP4/CMAF, outputting HLS/DASH/HTTP-FLV reuses the payload directly or does container format conversion only, with no decode/encode. This is the performance-optimal path. Start the encoding pipeline only when ABR is needed
2. **The HTTP Callback event system**: the stream lifecycle callback pattern of `on_publish`/`on_play`/`on_unpublish`/`on_stop`/`on_dvr` is concise and practical. MediaServo's `StreamingSDK` needs the same hook system for authentication, billing, notification, and automated transcoding triggers
3. **The Origin-Edge cluster topology**: a multi-level edge node cluster model validated at large scale. Suited to CDN edge distribution scenarios. MediaServo's `ClusterPlugin` can adopt a similar topology early on — simple and effective
4. **The GOP Cache mechanism**: caching the most recent GOP lets new viewers get a picture immediately. This is a basic requirement of online live streaming. MediaServo's HLS/HTTP-FLV output layer must implement it. See SRS's ring buffer + keyframe boundary strategy
5. **A Prometheus + Grafana observability reference**: SRS's metrics design (stream count/client count/bitrate/latency distribution) is a basic reference. MediaServo should add Fragment-level metrics on top of it (Fragment throughput, Observer latency, SLO violation count)
6. **The Docker + Helm deployment model**: Docker images + Helm Charts are standard in the K8s era. MediaServo should reference SRS's deployment configuration (multi-architecture Dockerfile + K8s values.yaml)

### [Adapt] Needs Modification Before Adoption
1. **RTMP normalization → Unified Fragment Model**: SRS normalizes all inputs to RTMP. MediaServo should borrow the normalization idea but replace RTMP with Fragment. Fragment has none of RTMP's semantic limitations (no fixed audio/video interleaving, has unordered object IDs, has a subgroup concept) and can map naturally onto new protocols such as MoQ/QUIC
2. **The coroutine model → Tokio async/await**: ST coroutines are a non-standard technology. MediaServo uses Rust's Tokio (the industry standard), with identical functionality but a more complete ecosystem. The synchronous-style async/await code is equally readable
3. **The RTMP protocol implementation as a reference**: SRS's RTMP implementation can serve as the functional specification and compatibility benchmark for `mediaservo-ingest-rtmp`. But the code cannot be ported directly (incompatible language and license). It must be reimplemented in Rust, with a focus on Enhanced RTMP compatibility
4. **A reference implementation of the GB28181 protocol**: MediaServo's `SurveillanceSDK` should consider GB28181 in Phase 3+. SRS's SIP signaling + RTP/PS depacketizing is the only referenceable open-source implementation. Attention is needed to SIP registration, keepalive, catalog query, and invite signaling flows
5. **HTTP-FLV output**: the HTTP-FLV protocol unique to the Chinese live ecosystem. MediaServo's `HlsPlugin` can add HTTP-FLV output capability (generating FLV tag sequences from the Fragment payload). This is a lightweight format conversion
6. **The Oryx management platform philosophy**: the one-click Docker deployment + Web UI model is a reference for MediaServo's deployment shape. But MediaServo's Client is a desktop application and Host's embedded configuration page is axum + static HTML, not SRS's web SPA

### [Avoid] Known Pitfalls / Not Applicable
1. **Do not use RTMP as the internal unified representation (the core lesson)**: RTMP's semantic limitations (fixed audio/video interleaving, no subgroup, no object ID) make adding new protocols difficult. MediaServo must replace RTMP normalization with the Fragment Model. This is the most important lesson learned from SRS
2. **Do not use ST coroutines or other non-standard concurrency technologies**: ST coroutine debugging tools are immature, the community is small, and hiring is hard. MediaServo should use Tokio async/await (Rust's standard async runtime)
3. **Do not depend entirely on external transcoding**: SRS has no built-in transcoding and depends on an external FFmpeg process. MediaServo needs built-in transmux (mandatory) and an optional encoding pipeline (for ABR, format transcoding, and similar scenarios). The `Transcoder` trait should support multiple backends
4. **Do not use a multi-language mixed-process architecture**: SRS's separate SRT Go process + TCP pipe bridging is a historical burden. Cross-process communication adds latency and failure points. MediaServo should keep all critical paths inside a single Rust process
5. **The technical debt lesson of 170k lines of C code**: the maintenance challenge of a large single-repository code base. MediaServo should keep clear crate boundaries from the start (see LVQR's 29 crates) and avoid a monolithic code base
6. **The operational burden of separately deploying TURN servers**: SRS's dependence on an external coturn is a persistent pain point. MediaServo should make TURN/STUN an optional built-in module (`StunTurnPlugin`) to simplify deployment
7. **The codec overhead of WebRTC push to RTMP**: SRS's WHIP push to RTMP requires decode→re-encode, destroying transmux performance. With the Fragment Model, MediaServo's WHIP input produces Fragments directly, with no re-encoding needed

**Overall rating**: ★★★☆☆ (3/5)

SRS is the benchmark of the RTMP era — unmatched in protocol compatibility, production stability, transport performance, and Chinese community support. But an RTMP-centric normalization model is its fundamental architectural flaw, limiting evolution toward modern protocols such as MoQ/QUIC. MediaServo should learn from SRS's transmux philosophy, HTTP Callback event system, Origin-Edge cluster topology, and observability design, but must replace RTMP normalization with the Fragment Model to fundamentally solve the architectural legacy problem from 13 years ago.

---
**Related decisions**: D5 (Unified Fragment), D6, D19, D-STREAM-TOPOLOGY, D-GOP-CACHE

## Appendix A: Detailed Explanation of the ST Coroutine Architecture

### A.1 The ST (State Threads) coroutine principle

ST is a user-space threading library developed by Netscape in 2001. SRS has used ST
as its only concurrency model since 2013.

**Core characteristics**:
- User-space coroutines: scheduling happens outside the kernel, with no system calls for context switches
- M:1 model: many ST coroutines map to 1 kernel thread, running in a single process
- Cooperative scheduling: coroutines yield voluntarily (usually on I/O operations)
- Context switch cost approximately that of a function call (saving/restoring registers)
- One coroutine per connection: on a new TCP connection, an ST coroutine is created to handle all of that connection's I/O

**Scheduling flow**:
```
                   ST Scheduler
                   
  Ready Queue: [coro_1, coro_3, coro_7]
  Wait Queue:  [coro_2 (waiting for fd=5)]
               [coro_4 (waiting for timer)]
               [coro_5 (waiting for fd=9)]

  Loop:
  1. Take a coroutine from the Ready Queue, resume execution
  2. The coroutine runs to an I/O operation -> yield -> added to the Wait Queue
  3. select() waits for I/O events -> ready coroutines move to Ready
  4. Back to step 1
```

### A.2 Comparison with Tokio async/await

| Dimension | ST coroutines | Tokio async/await |
|------|---------|-------------------|
| Language | C (non-standard) | Rust (language-level support) |
| Scheduling model | Cooperative (manual yield) | Cooperative (automatic yield at await points) |
| Context switch | Nearly a function call | Nearly a function call |


### [Adopt] Supplementary — detailed design of the HTTP Callback event system

**10. HTTP Callback event types and their MediaServo mapping**:

| SRS Callback | Trigger timing | Return control | MediaServo equivalent |
|-------------|---------|---------|---------------|
| on_publish | publisher push request | HTTP 200=allow, otherwise=deny | MediaSource::on_publish |
| on_unpublish | publisher disconnects | none (notification) | MediaSource::on_unpublish |
| on_play | viewer starts playback | HTTP 200=allow, otherwise=deny | MediaSink::on_subscribe |
| on_stop | viewer stops playback | none (notification) | MediaSink::on_unsubscribe |
| on_dvr | recording segment complete | none (notification) | RecordSink::on_segment_closed |
| on_hls | HLS segment generated | none (notification) | HlsSink::on_partial_ready |
| on_rtc_play | WebRTC playback starts | HTTP 200=allow, otherwise=deny | WhepSink::on_subscribe |

MediaServo's callback system should be designed as an `EventHook` trait supporting two modes: HTTP callbacks (remote services) and local callbacks (built-in auth/billing plugins).

### [Adapt] Supplementary — MediaServo implementation of the Origin-Edge cluster

**8. Implementation strategy of the Origin-Edge topology in the ClusterPlugin**:

MediaServo's Phase 1-2 cluster model can adopt the Origin-Edge topology directly (identical to SRS):

```
Origin node                          Edge node
(PipelineEngine)                     (PipelineEngine)
├── MediaSource (RTMP)               ├── MediaSource (pulls from Origin)
├── MediaSink (HLS)                  ├── MediaSink (HLS, local viewers)
├── MediaSink (WHEP)                 ├── MediaSink (WHEP, local viewers)
└── ClusterPlugin (Origin role)      └── ClusterPlugin (Edge role)
     ├── receives pushes                    ├── pulls Fragment streams from the
     └── FragmentBroadcaster                  Origin's FragmentBroadcaster
                                               (gRPC/QUIC)
          └── non-local edges via remote pull └── broadcasts to local Sinks
```

Key difference: SRS's Edge pulls an RTMP stream from the Origin (requiring codec awareness), while MediaServo's Edge pulls a Fragment stream from the Origin (protocol-agnostic). This means the data format an Edge can cache and distribute is not restricted by protocol — the same Fragment stream can be consumed independently by an Edge's HLS, DASH, and WHEP Sinks.

### [Avoid] Supplementary — more not-applicable scenarios

**8. The lifecycle of RTMP as a streaming protocol**: RTMP was publicly specified by Adobe in 2009, over 15 years ago. Although RTMP compatibility is irreplaceable in the live streaming ecosystem (OBS→RTMP→SRS is currently the most mature push chain), Adobe Flash Player was retired in 2020. RTMP is gradually being replaced at the transport layer by SRT/WebRTC/MoQ. MediaServo must support RTMP (Phase 1, because OBS and the push toolchain depend on it), but should not make it the architectural core. The Fragment Model makes RTMP one equal protocol among others rather than the center of the architecture.

**9. The maintenance cost of 170k lines of C code**: SRS's monolithic code base (170k lines of C/C++) makes the barrier to new contributors extremely high. MediaServo must avoid this problem from Phase 0 — keep every crate at 2000-5000 lines of Rust code, plus clear crate boundaries and trait isolation, so newcomers can complete work within a single crate without understanding the whole code base.

| Multi-core utilization | Single threaded (M:1) | Multi-threaded work-stealing (M:N) |
| Debugger support | No native debugger | Rust IDE/tracing |
| Ecosystem maturity | Extremely small | Very large (the whole Rust async ecosystem) |
| I/O wakeup | select() | epoll/kqueue/iocp (mio) |
| Code style | Written synchronously (no keywords) | Explicit async/await markers |
| Hiring difficulty | Nearly impossible | Easy (all Rust developers know it) |

The historical reason SRS uses ST: in 2013, C/C++ had no standard async model.
Today Rust's async/await is the better choice. MediaServo's use of Tokio is correct.

---

## Appendix B: RTMP Normalization vs Fragment Model Comparison

### B.1 Codec overhead of SRS's RTMP normalization

SRS incurs double encoding when normalizing a WebRTC WHIP push to RTMP:

```
WebRTC WHIP (H.264+Opus)
  |
  v
RTP depacketizer -> H.264 NAL + Opus frames
  |
  v
H.264 decoder -> YUV -> H.264 encoder -> H.264 NAL (re-encode!)
Opus decoder -> PCM -> AAC encoder -> AAC frames (re-encode!)
  |
  v
FLV muxer -> RTMP stream (internal representation)
  |
  v
Transmux from RTMP to each output protocol (HLS/DASH/HTTP-FLV/SRT/WHEP)
```

Problems:
1. The WHIP input's H.264 is already encoded, but SRS must decode and re-encode it, wasting CPU and losing quality
2. The WHIP input's Opus audio has higher quality than AAC; after normalization to RTMP it becomes AAC, losing information
3. Every WHIP push incurs codec overhead, and the transmux zero-transcode advantage is lost

### B.2 How the Fragment Model solves this

```
WebRTC WHIP (H.264+Opus)
  |
  v
RTP depacketizer -> H.264 NAL + Opus frames
  |
  v
Pack directly into Fragment { payload: H.264 NAL + Opus frames }
  |
  v
FragmentBroadcasterRegistry -> Observer taps
  |
  +---> HLS: generate a CMAF chunk from the Fragment (no decode!)
  +---> DASH: generate an fMP4 segment from the Fragment (no decode!)
  +---> WHEP: generate RTP packets from the Fragment (no decode!)
  +---> MoQ: push the Fragment directly (no decode!)
  +---> Recording: write to fMP4 directly (no decode!)
```

Advantages:
1. Zero codec: the Fragment carries raw encoded data, and all outputs reuse it directly
2. No quality loss: H.264 passthrough
3. The original encoder information is preserved, letting output protocols implement optimal packetization strategies

### B.3 Migration path comparison

| Component | RTMP normalization (SRS) | Fragment Model (MediaServo) |
|------|-------------------|---------------------------|
| Internal representation | FLV tag stream | Fragment stream |
| RTMP input | Zero conversion (FLV-FLV) | Small conversion (FLV-Fragment) |
| WHIP input | Decode-encode-FLV | RTP depacket-Fragment |
| SRT input | (separate process) FLV bridge | MPEG-TS-Fragment |
| HLS output | FLV-TS segment | Fragment-CMAF chunk |
| DASH output | FLV-fMP4 segment | Fragment-fMP4 segment |
| WHEP output | FLV-decode-RTP | Fragment-RTP (no decode!) |
| HTTP-FLV output | FLV passthrough | Fragment-FLV tag |

---

## Appendix C: SRS Core Configuration Parameters

SRS's INI configuration file structure:

```ini
listen              1935;
max_connections     1000;
daemon              off;

vhost __defaultVhost__ {
    # HTTP Callback
    http_hooks {
        enabled         on;
        on_publish      http://127.0.0.1:8085/api/v1/callback/on_publish;
        on_play         http://127.0.0.1:8085/api/v1/callback/on_play;
        on_stop         http://127.0.0.1:8085/api/v1/callback/on_stop;
        on_dvr          http://127.0.0.1:8085/api/v1/callback/on_dvr;
    }

    # HLS configuration
    hls {
        enabled         on;
        hls_fragment    3;      # fragment duration (seconds)
        hls_window      12;     # number of retained fragments
        hls_path        ./objs/nginx/html;
    }

    # DVR recording
    dvr {
        enabled         on;
        dvr_path        ./objs/nginx/html/dvr/[app]/[stream]/[timestamp].flv;
        dvr_plan        segment;
        dvr_duration    600;    # 10 minutes per segment
    }

    # HTTP-FLV
    http_remux {
        enabled     on;
        mount       [vhost]/[app]/[stream].flv;
    }
}
```

Implications for MediaServo:
- The INI format is traditional, but the configuration parameter design is concise (hls_fragment needs only one number)
- HTTP Callback URL patterns use parameterized paths such as [app]/[stream]
- dvr_plan: segment, time-based segmentation, is the standard recording strategy
- Global limits such as max_connections are necessary production-grade protection measures
