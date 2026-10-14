# mediasoup Reference Analysis
> Generated: 2026-07-16 | Category: Video Conferencing

## 1. Product Profile
- **Name**: mediasoup
- **Developer**: Versatica (community-driven project; core maintainers Iñaki Baz Castillo, José Luis Millán, Nazar Mokrynskyi)
- **Core maintainers**: ibc (Iñaki Baz Castillo, project founder), jmillan (José Luis Millán), nazar-pc (Nazar Mokrynskyi)
- **First release**: December 2014 (first GitHub commit). The v3 line became mainstream in 2018
- **Product positioning**: A minimalist SFU (Selective Forwarding Unit) library. Strictly scoped to "media layer only" — it binds no signaling protocol and ships no out-of-the-box video-conferencing features. Positioned as an infrastructure component embedded in larger applications, not a standalone product
- **Target user groups**: developers and architects who need to build their own real-time AV infrastructure. Requires deep team knowledge of the WebRTC stack (SDP/ICE/DTLS/SRTP), signaling-system development capability, and distributed-systems operations experience. Typical users: BigBlueButton (the world's largest open-source online-education platform) as an SFU backend, Topology (enterprise collaboration platform)
- **License / business model**: ISC License (ISC — Internet Systems Consortium). One of the most permissive open-source licenses around (shorter than MIT, fewer restrictions than Apache 2.0). Pure community project, no commercial SaaS or enterprise edition. No commercial services of any kind. The community relies entirely on open-source contributions and sponsorship
- **npm weekly downloads**: 33.2K/week (July 2026)

## 2. Technical Features

### 2.1 Overall Architecture Design

mediasoup uses the classic two-tier "C++ Worker core + upper-language bindings" architecture. This is the foundation of its high performance and flexibility.

```
┌──────────────────────────────────────────────────────────┐
│              Application layer (Node.js / Rust)           │
│                                                           │
│  ┌────────────────────────────────────────────────────┐  │
│  │     Custom signaling layer (fully developer-owned)  │  │
│  │                                                     │  │
│  │  ┌──────────────┐  ┌──────────────┐                │  │
│  │  │ Room mgmt    │  │ Participant  │                │  │
│  │  │ · create/    │  │ mgmt         │                │  │
│  │  │   destroy    │  │ · join/leave │                │  │
│  │  │ · config mgmt│  │ · roles/perms│                │  │
│  │  │ · lifecycle  │  │ · reconnect  │                │  │
│  │  └──────────────┘  └──────────────┘                │  │
│  │  ┌──────────────┐  ┌──────────────┐                │  │
│  │  │ Load         │  │ Recording    │                │  │
│  │  │ balancing    │  │ scheduling   │                │  │
│  │  │ · Worker     │  │ · RTP export │                │  │
│  │  │   assign     │  │ · storage    │                │  │
│  │  │ · Router sel.│  │   mgmt       │                │  │
│  │  │ · health chk │  │ · post-proc  │                │  │
│  │  │              │  │   composite  │                │  │
│  │  └──────────────┘  └──────────────┘                │  │
│  └────────────────────────────────────────────────────┘  │
│                          │                                │
│  ┌────────────────────────────────────────────────────┐  │
│  │      mediasoup Node.js / Rust API module            │  │
│  │                                                      │  │
│  │  API surface:                                        │  │
│  │  · createWorker()     — spawn C++ Worker process     │  │
│  │  · createRouter()     — create Router in Worker      │  │
│  │  · createWebRtcTransport() — WebRTC endpoint         │  │
│  │  · createPlainTransport()  — raw RTP endpoint        │  │
│  │  · createPipeTransport()   — Router cascade endpoint │  │
│  │  · createProducer()  — create media publisher        │  │
│  │  · createConsumer()  — create media subscriber       │  │
│  │  · pipeToRouter()    — cascade between Routers       │  │
│  └──────────────────┬───────────────────────────────┘    │
│                     │                                    │
└─────────────────────┼────────────────────────────────────┘
                      │ Unix Domain Socket / TCP (Channel Protocol)
                      │ libuv-pipe-based binary communication protocol
┌─────────────────────┼────────────────────────────────────┐
│                     │  C++ Worker layer                   │
│                                                           │
│  ┌──────────────────┴───────────────────────────────┐    │
│  │  Worker 1 (C++ subprocess)  Worker 2 (C++ subproc)│   │
│  │  PID: 12345               PID: 12346               │    │
│  │  CPU: 0                   CPU: 1                   │    │
│  │                                                    │    │
│  │  ┌──────────────────┐    ┌──────────────────┐     │    │
│  │  │     Router       │    │     Router       │     │    │
│  │  │  ─────────────── │    │  ─────────────── │     │    │
│  │  │  · Producer Map  │    │  · Producer Map  │     │    │
│  │  │  · Consumer Map  │    │  · Consumer Map  │     │    │
│  │  │  · RTX Buffer    │    │  · RTX Buffer    │     │    │
│  │  │  · PLI Aggregator│    │  · PLI Aggregator│     │    │
│  │  │  · BWE Engine    │    │  · BWE Engine    │     │    │
│  │  │                                                  │    │
│  │  │  ┌────────────┐        ┌────────────┐           │    │
│  │  │  │ Transport  │        │ Transport  │           │    │
│  │  │  │ (WebRTC)   │        │ (PlainRTP) │           │    │
│  │  │  │ ICE/DTLS   │        │ UDP socket  │           │    │
│  │  │  │ SRTP enc   │        │ Raw RTP     │           │    │
│  │  │  └────────────┘        └────────────┘           │    │
│  │  │  ┌────────────┐        ┌────────────┐           │    │
│  │  │  │ Producer   │        │ Consumer   │           │    │
│  │  │  │ · Audio    │        │ · SSRC A   │           │    │
│  │  │  │ · Video Lo │        │ · SSRC B   │           │    │
│  │  │  │ · Video Med│        │ · SSRC C   │           │    │
│  │  │  │ · Video Hi │        │            │           │    │
│  │  │  └────────────┘        └────────────┘           │    │
│  │  └──────────────────┘    └──────────────────┘     │    │
│  │                                                    │    │
│  │    libuv (C event loop)    libuv (C event loop)    │    │
│  │    · epoll/kqueue/IOCP     · epoll/kqueue/IOCP      │    │
│  │    · single-threaded,      · single-threaded,       │    │
│  │      lock-free               lock-free              │    │
│  │    · zero context          · zero context           │    │
│  │      switches                switches               │    │
│  └──────────────────────────────────────────────────┘    │
│                                                           │
│  Dependencies:                                            │
│  · OpenSSL 3.x — DTLS handshake + SRTP encrypt/decrypt   │
│  · libuv 1.x   — cross-platform async I/O + event loop    │
│  · Custom SCTP stack — replaces usrsctp (v3.20+)          │
│  · Google Abseil — C++ utility library                    │
│  · FlatBuffers — internal serialization (optional)        │
└──────────────────────────────────────────────────────────┘
```

### 2.2 Worker Model & Performance

**Worker creation & binding**:
Each Worker is an independent OS process (created via fork, not a thread). The main process creates Workers via `createWorker()`, with options:
- `logLevel`: log level
- `rtcMinPort` / `rtcMaxPort`: ICE port range
- `dtlsCertificateFile` / `dtlsPrivateKeyFile`: custom TLS certificate
- `libwebrtcFieldTrials`: WebRTC experimental feature switches

**Performance formulas & constraints**:
- 1 Worker ≈ 1 CPU core at full load ≈ at most ~500 Consumers (concurrently active RTP downlink streams)
- Consumer count in an N-person fully-interactive room = N × (N-1) × 2 (each person consumes everyone else's audio + video)
- Theoretical maximum Consumers on an 8-core server = 8 × 500 = 4,000
- Typical load: dozens of small/medium meetings (2-20 people each), not a single large meeting
- The community explicitly advises: large fully-interactive rooms (>50 people, all video) do not suit pure SFU mode

**Process-isolation advantages**:
- Crash isolation: a SIGSEGV kills only the current Worker; meetings on other Workers are unaffected
- Memory isolation: no risk of stray pointers corrupting across Workers
- CPU affinity: each Worker pinned to a specific CPU core (CPU affinity)
- Resource quotas: per-Worker CPU/memory independently limited via Linux cgroups
- Main-process supervision: health checks, heartbeat monitoring, automatic restart on anomalies

### 2.3 Key Technical Capabilities

| Capability | Details |
|------|------|
| Architecture pattern | Pure SFU (Selective Forwarding Unit). Strictly no decoding, no transcoding — pure RTP-packet-level selective forwarding. Versus MCU: 5-10× lower CPU consumption, 50-80ms lower latency |
| Video codecs | VP8, VP9, H.264, AV1 all supported. Simulcast 3-layer concurrent encoding (high/medium/low resolution independent streams). SVC spatial/temporal layering (VP9/AV1). Each Consumer independently selects its spatial and temporal layer |
| Transport protocols | WebRTC Transport (ICE/DTLS/SRTP over UDP/TCP); Plain RTP Transport (raw RTP over UDP, unencrypted); Pipe Transport (Router-to-Router cascade, Unix Socket on the same host or TCP across hosts); SCTP RTCDataChannel over DTLS or over UDP |
| Recording | No built-in recording. Streams must be forwarded via Plain RTP Transport to external recording services (FFmpeg/GStreamer/custom recording pipeline) |
| Platform support | Server: Linux (primary production platform), macOS (dev/testing), Windows (experimental). Clients: browsers (mediasoup-client v3, written in TypeScript), native (libmediasoupclient, written in C++), Python (mediasoup-client-aiortc). Dual-language server bindings: Node.js and Rust |
| Cascading routing | pipeToRouter() API — a single method call connects two Routers. Supports same-host (Unix Socket, zero network overhead) and cross-host (TCP/TLS Socket). The application layer owns the transport connection-parameter management |
| Worker isolation | Each Worker is an independent C++ process; crashes do not affect one another. The main process owns lifecycle management |
| Keyframe aggregation | When multiple Consumers request PLI/FIR simultaneously, requests are automatically merged into one upstream request. Subsequent duplicate requests within a 500ms~1s debounce window are ignored. Prevents N subscribers simultaneously demanding keyframes from overloading the encoder |
| Adaptive bitrate | Based on REMB (receiver bandwidth estimation) and TWCC (Transport-Wide Congestion Control). Each Consumer runs its own bandwidth estimate and automatically picks the optimal video layer |
| Security | Multiple SRTP cipher suites (AES_CM_128, AEAD_AES_256_GCM, AEAD_AES_128_GCM). DTLS 1.2 handshake. E2EE supported (Insertable Streams API, key exchange implemented at the application layer) |

### 2.4 Tech Stack Details

**Language distribution**:
- C++ (67.6%): the Worker core engine, built on the libuv event loop
- Rust (20.4%): the mediasoup-rust crate. Started in 2023, providing a Rust API equivalent to the Node.js binding
- TypeScript (10.1%): the Node.js API layer + the browser client SDK (mediasoup-client)
- JavaScript (0.8%): build toolchain, test helpers
- Other (CMake/Meson/Python): build-system and CI/CD configuration

**Core dependency libraries**:
- **libuv 1.x**: cross-platform async I/O library. Provides the event loop (epoll/kqueue/IOCP wrappers), TCP/UDP sockets, pipes, timers, signals, subprocess management, etc. The same event-driven engine that powers Node.js
- **OpenSSL 3.x**: DTLS handshake (client and server certificate management) and SRTP encrypt/decrypt (multiple cipher suites)
- **Custom SCTP stack** (v3.20+): replaces usrsctp (a C library called via FFI). Advantages: no FFI overhead, DirectTransport mode supported (in-process SCTP message send/receive), fine-grained configuration (maxSendMessageSize, maxReceiveMessageSize)
- **Google Abseil**: C++ utility library providing string, status, time and other base components
- **Meson**: cross-platform C++ build system. A modern alternative to CMake

**Protocol coverage**:
- ICE-Lite (server mode, recommended) and ICE-Full (client mode) dual modes
- DTLS 1.2 (self-signed or custom certificates) fully automatic handshake management
- Multiple SRTP cipher suites (AES_CM_128_HMAC_SHA1_80, AES_CM_128_HMAC_SHA1_32, AEAD_AES_256_GCM, AEAD_AES_128_GCM)
- STUN (Binding Request/Response, Keepalive)
- TURN (client mode, connecting to external TURN services)
- Full RTP/RTCP stack (RTP packet parsing, SSRC management, RTCP SR/RR, NACK, PLI, FIR, REMB, TWCC)
- SCTP over DTLS and SCTP over UDP dual modes

## 3. Feature Overview

### 3.1 Core Domain Model

mediasoup's core abstraction is four entities: Worker → Router → Transport → Producer/Consumer. A pure domain model — each entity has a single responsibility in a clear hierarchy.

**Worker**: represents an OS process. One per CPU core. Manages Router creation and destruction. Handles no media business logic itself.

**Router**: represents an independent media routing space. Maintains a Producer Map and a Consumer Map. Performs selective RTP packet forwarding. Three key subsystems built in:
1. **RTP retransmission buffer**: each Producer keeps a ring buffer indexed by SSRC + sequence number, caching ~1000+ RTP packets by default. When a Consumer reports a lost packet via RTCP NACK, the buffer answers directly — no need to consult the upstream Producer
2. **PLI/FIR request aggregator**: simultaneous keyframe requests from multiple Consumers of the same Producer are merged into one PLI/FIR request. Subsequent duplicate requests within a 500ms~1s debounce window are ignored. Prevents 100 subscribers simultaneously requesting keyframes from spiking the encoder into overload
3. **Bandwidth estimation engine**: each Consumer independently runs a REMB or TWCC estimator. The Router automatically selects the best Simulcast/SVC layer for each Consumer. The application layer can intervene manually via `setConsumerPreferredLayers()`

**Transport**: three types:
1. **WebRtcTransport**: standard WebRTC connection endpoint. Full ICE-Lite/ICE-Full, DTLS handshake, SRTP encrypt/decrypt. v3.20 added WebRtcServer mode (single-port multiplexing — distinct connections differentiated by ICE ufrag, greatly reducing firewall port requirements)
2. **PlainTransport**: raw RTP over UDP. No ICE/DTLS/SRTP. Supports comedia mode (auto-learns the peer address). Manual SRTP mode optional. Typical scenarios: FFmpeg recording pipelines, GStreamer processing chains, external RTP stream injection
3. **PipeTransport**: dedicated to Router-to-Router cascading. API: `routerA.pipeToRouter({ producerId, router: routerB })`. Same host uses a Unix Socket (zero-copy); cross-host uses TCP/TLS Sockets. The application layer owns transport connection parameters (IP:Port)

**Producer**: represents one upstream media source. Created by a client through a Transport. The Router records the RTP parameters (SSRC, payload type, codec, Simulcast layer structure or SVC structure). Supports the paused state — connection stays up while RTP packet forwarding pauses

**Consumer**: represents one downstream media subscription. The server calls `createConsumer()` per subscriber. The Router automatically starts forwarding RTP packets matching the negotiated format. Each Consumer independently manages its own RTCP send/receive — SR/RR, REMB, NACK, PLI, FIR. The Consumer is the SFU scalability bottleneck — an N-person room yields up to N×(N-1)×2 Consumers

### 3.2 Signature Features

- **Simulcast + SVC independent layer selection**: mediasoup's killer feature. Each Consumer selects its video layer independently based on its own bandwidth estimate. Spatial layers (720p/360p/180p) and temporal layers (30fps/15fps/7.5fps) can be chosen orthogonally. Different participants in the same meeting receive completely different quality layers — Zhang San may be on mobile 4G watching 180p/7.5fps while Li Si is on desktop WiFi at 720p/30fps. No mutual interference.
- **pipeToRouter() pipeline philosophy**: the Unix pipe design philosophy realized in the WebRTC world. `routerA.pipeToRouter({ producerId, router: routerB })` — one API call connects two Routers (whether on the same machine or across data centers). Three topologies supported: tree (split a large room into sub-rooms), star (central Router + per-region nearest access), chain (multi-datacenter forwarding chain)
- **WebRtcServer single-port multiplexing** (v3.20 milestone feature): traditionally each WebRTC Transport needed its own port pair (RTP + RTCP). Large-scale deployment meant thousands of open ports and complex firewall rules. WebRtcServer carries all Transports on a single UDP port — connections differentiated by ICE ufrag. The firewall needs only 1 open port. A key ops-friendly improvement
- **PLI/FIR intelligent aggregation**: the answer to the "invisible cost killer" in large-scale SFU deployments. I-frame (keyframe) size is 5-20× that of a P-frame. 100 participants simultaneously requesting keyframes within 500ms → instant encoder overload (100 I-frames to encode) → network congestion → more loss → more PLI requests → vicious cycle. mediasoup's aggregator: auto-merge → 500ms-1s debounce → send a single PLI. Any production-grade SFU implementing Simulcast/SVC must have a similar mechanism
- **Custom SCTP stack** (v3.20): farewell to usrsctp (external C library, FFI calls). Better performance (no cross-FFI-boundary data copies). DirectTransport mode — SCTP messages sent/received directly inside the Node.js/Rust process without touching the network stack. Fine-grained configuration: maxSendMessageSize, maxReceiveMessageSize, sendBufferSize

### 3.3 Deployment & Scaling Patterns

**Three horizontal-scaling strategies**:
1. **Multiple Workers on one host**: Worker count = CPU core count (auto-detected via `os.cpus().length`). Routers assigned to Workers by room-ID hash. The simplest, most effective same-host scaling approach
2. **Cross-host pipeToRouter**: Routers on different hosts cascaded via pipeToRouter(). The application layer owns cross-host signaling mediation (Redis Pub/Sub, etcd, NATS). Requires a custom orchestration system
3. **Large-room partitioned cascading**: a single Router is conservatively limited to ~16 people fully interactive. Beyond the limit, create a new Router and cascade via pipeToRouter. Consumers within each partition = N_partition × (N_partition-1) × 2; cascade-link Consumers = N_partition × N_other_partitions × 2. Effectively reduces single-Router load

**Cost and benefit of signaling-agnosticism**:
- Benefit: complete freedom to choose the signaling protocol (WebSocket/gRPC/HTTP REST/MQTT/XMPP); deep integration with existing systems (e.g. MediaServo C-FFI, AUDEBase napi-rs); no protocol vendor lock-in
- Cost: a production-grade signaling layer is at least 2-3 person-months of development (room management, ICE negotiation, participant state machine, reconnect logic, authentication/authorization, monitoring metrics); any signaling-layer bug is a full-stack bug — requires understanding both WebRTC protocol and the mediasoup API

## 4. Current Status & Ecosystem

### 4.1 Version & Activity

- **Current version**: v3.21.1 (released 2026-07-14). 119 cumulative releases since the v3 line
- **Release cadence**: averaging 1-2 minor versions per month. Actively maintained
- **GitHub Stars**: 7,290. Forks: 1,242
- **Contributors**: ~60. Core maintainers ~5
- **Open Issues**: 29 (extremely low — issue management is very strict)
- **npm total versions**: 405 published package versions
- **npm weekly downloads**: 33,215
- **Last commit**: 2026-07-16 (continuously active development)

### 4.2 Version Evolution History

- **v3.0-v3.10 (2018-2021) — core stabilization**: established the Router/Transport/Producer/Consumer four-layer domain model. Continuous browser WebRTC compatibility follow-up. Basic Simulcast support. Basic bandwidth estimation and layer selection
- **v3.11-v3.15 (2022-2024) — Simulcast/SVC enhancement period**: VP9 SVC spatial/temporal layer support. Bandwidth estimation algorithms matured. Continuous server-side performance optimization. PlainTransport comedia mode
- **v3.16-v3.21 (2025-2026) — modernization period**: custom SCTP stack replacement (v3.20). WebRtcServer single-port multiplexing (v3.20). The Rust crate binding officially released and maturing. CI matrix extended to arm64 and Windows 2025. AV1 codec support. RTCDataChannel enhancements (DirectTransport SCTP)

### 4.3 Community & Documentation

- **Discourse forum**: `mediasoup.discourse.group` — the main channel for community Q&A and technical discussion
- **Bluesky**: `@mediasoup-sfu.bsky.social` — official social media account
- **GitHub Discussions**: release announcements, feature discussions, community support
- **Official docs `mediasoup.org`**: complete v3 API documentation — every class, every method, every parameter, every event, every type definition documented in detail
- **CHANGELOG.md**: 830+ lines of detailed change records from v3.1 to v3.21
- **Demo project**: `mediasoup-demo` (official), `demo.mediasoup.org` (online experience)
- **Client SDKs**: `mediasoup-client` (TypeScript browser), `libmediasoupclient` (C++ native), `mediasoup-client-aiortc` (Python)
- **Third-party ecosystem**: GStreamer plugin (community), FFmpeg Plain RTP interop recipes (community docs), React Native adaptation (community)
- **Chinese support**: none. All documentation is English-only. Adds extra learning cost for Chinese-speaking teams

### 4.4 Known Deficiencies & Limitations

1. **No signaling layer provided** — mediasoup's biggest paradox: the strongest SFU engine, yet developers must build the entire control plane from scratch. Room management, participant authentication, ICE negotiation, reconnection — at least 2-3 person-months of work
2. **Limited large fully-interactive rooms** — the Consumer count N×(N-1)×2 quadratic growth is an inherent constraint of the SFU architecture, not a mediasoup-specific problem. But mediasoup offers no MCU mixing as a fallback
3. **Scaling requires a self-built orchestration system** — no built-in load balancing, health checking, auto-scaling, or service discovery. For multi-host deployments the application layer needs complete orchestration logic
4. **Node.js FFI overhead** — serialization/deserialization overhead exists on the Channel between the C++ Worker and the Node.js main process. Under heavy load (thousands of Consumers changing state simultaneously) it can become a bottleneck
5. **English-only documentation** — all docs, forums, and issues are in English. Extra language and comprehension cost for Chinese-speaking teams
6. **No built-in recording** — recording must be entirely DIY. PlainTransport provides RTP export, but there is a lot of engineering between RTP packets and a playable MP4 file
7. **Rust crate relatively young** — started in 2023; API stability, documentation completeness, and community case studies all trail the maturity of the Node.js version

## 5. Market Positioning

### 5.1 Primary Industries of Application

- **Online education**: the official SFU backend of BigBlueButton 3.0 (the world's largest open-source online-education platform). mediasoup's largest production deployment case
- **Enterprise communication platforms**: self-hosted video conferencing driven by data sovereignty and compliance (benchmarking Jitsi but with better performance)
- **Interactive live streaming**: PlainRTP → FFmpeg → RTMP/HLS second-development recipes. Suits live scenarios requiring extremely low-latency interactive co-streaming
- **Web3/decentralized video**: ISC license + no centralized dependencies + combinable with blockchain signaling
- **IoT/edge video**: PlainRTP supports non-browser sources (RTSP cameras, drone video links, industrial sensor streams)

### 5.2 Competitor Comparison Summary

| Dimension | mediasoup | Janus Gateway | Jitsi JVB | LiveKit |
|------|-----------|---------------|-----------|---------|
| Core language | C++ / Rust / TypeScript | C | Kotlin / Java | Go |
| License | ISC (most permissive) | GPLv3 | Apache 2.0 | Apache 2.0 |
| Signaling | None (developer-built) | HTTP/WS/RabbitMQ/MQTT etc. multi-protocol | Colibri2 + XMPP | Built-in WebSocket binary protocol |
| Out of the box | No (pure library) | Needs development (plugin framework exists) | Yes (complete full-stack product) | Yes (semi-full-stack product) |
| Simulcast | ✅ Full support | ✅ VideoRoom plugin | ✅ | ✅ |
| SVC | ✅ VP9/AV1 full | ❌ | ✅ VP9 | ✅ VP9/AV1 |
| Cascading | ✅ pipeToRouter | ❌ | ✅ Secure Octo | ✅ Redis Router |
| Process isolation | ✅ Worker processes | ❌ | ❌ | ❌ |
| CPU efficiency | ★★★★★ | ★★★★☆ | ★★★☆☆ | ★★★★☆ |
| Deployment complexity | Highest (build everything) | Fairly high | Medium | Lowest |
| Rust support | Official Rust crate | None | None | Client Rust SDK |
| Recording support | DIY required | RTP Forward | Jibri (Chrome) | Built-in Egress |
| AI Agent | ❌ | ❌ | ❌ | ✅ first-class citizen |
| Community size | Medium (7.3k Stars) | Medium | Large (29.6k Stars) | Fast growing (19.7k Stars) |

### 5.3 Pricing & Licensing

- **License**: ISC License — nearly equivalent to public domain. Permits commercial use, modification, closed-source redistribution, patent use. Only requires retaining the original copyright notice. One of the most permissive open-source licenses around
- **Cost**: completely free. No SaaS cloud service. No enterprise edition. No commercial charges of any kind
- **TCO (total cost of ownership)**: server costs (VPS/bare metal, configured as needed) + ops headcount (1-2 engineers familiar with WebRTC) + bandwidth (usage-based billing)

## 6. Product Highlights

1. **Signaling-agnosticism delivers maximum flexibility** — the only design in its class that binds no signaling protocol whatsoever. Developers may use WebSocket/gRPC/HTTP/MQTT/XMPP, any or all. MediaServo can implement dedicated signaling in Rust and integrate natively and deeply with MediaServo and AUDEBase through napi-rs or C-FFI. This is a degree of flexibility that LiveKit (bound to a proprietary binary protocol) or Jitsi (bound to XMPP) cannot offer

2. **Extreme performance from C++ Worker + libuv event loop** — single-threaded lock-free design + independent process isolation. Zero context switches inside the Worker — all RTP packet receive, route, and forward work completes in one libuv event loop. CPU efficiency far exceeds the JVM (Jitsi JVB, GC pauses) and Go (LiveKit, goroutine scheduling overhead). A single Worker crash does not affect other Workers (process isolation > thread isolation)

3. **pipeToRouter()'s Unix pipeline philosophy** — `routerA.pipeToRouter({ producerId, router: routerB })` — the Unix design philosophy (each tool does one thing well, connected by pipes) perfectly realized in the WebRTC world. One API call achieves cross-core / cross-host / cross-datacenter cascading. Tree/star/chain topologies combined freely. The application layer fully controls cascade topology — no black-box routing, no vendor lock-in

4. **The Rust crate path aligns exactly with MediaServo's strategy** — mediasoup's Rust crate (started 2023) validates the "C++ core + Rust safe binding + upper-language wrapper" technology path. MediaServo's native-core (Rust → napi-rs → Node.js, Rust → FFI → C) can reuse this pattern directly. mediasoup-rust proved this three-tier architecture's feasibility and performance advantage

5. **PLI/FIR intelligent aggregation — a necessity for production-grade SFUs** — a feature that looks tiny but is actually critical. 100 participants all requesting keyframes within 500ms = the encoder must produce 100 I-frames in an extremely short time = instant CPU overload + network congestion. mediasoup's aggregator auto-merges + debounces = one PLI/FIR sent = one I-frame produced. Any self-built SFU (in any language or framework) must implement equivalent logic — this is production experience, not an optional optimization

## 7. Reference Value for MediaServo

### [Adopt] Directly Borrowable

1. **Worker process isolation pattern**: MediaServo's SFU Workers should adopt a similar design — one independent Rust process per CPU core (spawned via `std::process::Command`), IPC over Unix Domain Sockets. An order of magnitude better in stability and operability than a single-process multi-threaded design
2. **Router → Transport → Producer/Consumer three-tier domain model**: use directly as the domain-model design blueprint of the `mediaservo-conference` crate. Producer = source, Consumer = subscription, Transport = connection type — the elegance of this abstraction has been validated by mediasoup over many years
3. **PLI/FIR aggregation + 500ms-1s debounce**: built into `SfuRouter`. The first line of defense for a production-grade SFU — without it, any participant's network jitter in a 100-person meeting triggers a cascading keyframe-request storm
4. **The design philosophy of fully separating signaling and media**: MediaServo's architecture.md already embodies this — mediasoup proves with a complete project that "separation works at extreme performance"
5. **Rust crate binding pattern**: mediasoup-rust validates the feasibility of dual-language server bindings. MediaServo's two paths — napi-rs (Node.js) and C-FFI (MediaServo) — can follow its API-mapping rules directly

### [Adapt] Adopt With Modifications

1. **Simulcast layer-selection algorithm**: mediasoup's REMB/TWCC baseline bandwidth estimation can be adapted into MediaServo PipelineEngine. But scenario awareness must be added — remote desktop (steady high bandwidth) vs video conferencing (fluctuating medium bandwidth) vs vehicle push (highly unstable bandwidth) vs teleoperation (latency-sensitive bandwidth) — different scenarios use different layer-selection strategies
2. **pipeToRouter() cascade protocol**: mediasoup implements cascading with plain TCP/UDP socket connections. MediaServo's cascade signaling needs a control plane above the transport — topology management, routing-table synchronization, health checks, circuit breakers, recovery from tripped breakers. gRPC is better suited than raw sockets for control-plane communication
3. **WebRtcServer single-port multiplexing**: this feature added in v3.20 has direct reference value for MediaServo's network policy design. At large scale the firewall opens very few ports. But porting it to the Rust ecosystem requires a self-built equivalent design (str0m and webrtc-rs do not currently include single-port multiplexing)
4. **RTCDataChannel → mediaservo-teleop control link**: mediasoup's DataProducer/DataConsumer are generic SCTP data channels. MediaServo's teleop module must layer a control protocol on top — an unordered, maxRetransmits=0 low-latency mode plus an ordered reliable mode, dual channels

### [Avoid] Known Pitfalls & Non-Applicable Scenarios

1. **Not usable as an out-of-the-box service** — mediasoup is not a product, it is an engine. It assumes your team can build signaling, recording, monitoring and orchestration themselves. If you expect a video-conferencing service in three commands, use Jitsi or LiveKit
2. **Large fully-interactive rooms (>50 people all video) are infeasible** — quadratic Consumer growth is a mathematical constraint of the SFU architecture, not an implementation issue. Solutions: limit concurrent speakers (Jitsi Last-N style) + webinar mode (Zoom CDN distribution style)
3. **Fully-connected Router mesh does not scale** — Jitsi's 2020 lesson applies equally to mediasoup deployments. Multi-node Routers must cascade in a Pools star topology; never fully interconnect
4. **Node.js FFI overhead must be quantified** — under heavy load (thousands of Consumers changing state simultaneously), the Channel between the Node.js main process and the C++ Worker can become the bottleneck. Prefer the Rust crate path (zero FFI overhead)
5. **No MCU mixing = an independent service needed** — PSTN bridging, recording compositing, multi-stream blended layouts — these all need MCU capability. MediaServo needs a separate mixing/compositing service (GStreamer-based or a custom rendering pipeline)

**Overall rating**: ★★★★☆ (4/5)

> Verdict: mediasoup is the core SFU reference engine selected by MediaServo. Its advantages in performance, flexibility and code quality are irreplaceable. But "signaling-agnostic" is both its most powerful weapon and its steepest barrier — it demands that the MediaServo team reach the WebRTC engineering level of designing a complete control plane independently. As long as that premise holds, mediasoup's reference value is the highest among the four options.

---

> **Sources**
> GitHub: versatica/mediasoup (7,290 Stars, ISC License, v3.21.1)
> npm: mediasoup (33.2K weekly downloads, 405 versions)
> Official site: mediasoup.org (complete v3 API docs)
> Community: mediasoup.discourse.group
> Bluesky: @mediasoup-sfu.bsky.social
> CHANGELOG.md: 830+ lines of records from v3.1 to v3.21
> mediasoup-rust crate: github.com/versatica/mediasoup-rust
> MediaServo: docs/research/video-conference.md

---
**Related decisions**: D97, D138, D-SFU-WORKER, D-SIMULCAST, D50

## Appendix A: mediasoup Demo Deployment Steps

mediasoup-demo is the official complete reference implementation, including a Node.js signaling layer. Deployment overview:

```bash
# 1. Clone the demo project
git clone https://github.com/versatica/mediasoup-demo.git
cd mediasoup-demo

# 2. Install dependencies (server + client + browser)
cd server && npm install
cd ../app && npm install

# 3. Configure
cp server/config.example.js server/config.js
# Edit config.js: set TLS cert paths, listen IP/Port, announced IP

# 4. Start the server
cd server && node server.js

# 5. Build and start the browser client
cd ../app && npm start  # dev mode, default http://localhost:3000
```

The demo's core value is a working reference implementation of the signaling layer — signaling based on WebSocket + JSON message format, with fully traceable room management, ICE negotiation, and Producer/Consumer creation flows. MediaServo's signaling layer can draw inspiration from the demo's signaling message formats and state-machine design.

Key signaling message types:
- `getRouterRtpCapabilities` → obtain all RTP capabilities the Router can handle
- `createWebRtcTransport` → create a WebRTC transport endpoint
- `connectWebRtcTransport` → connect the transport after the DTLS handshake completes
- `produce` → publish a media stream (audio/video/screen share)
- `consume` → subscribe to other participants' media streams
- `resumeConsumer` → resume a paused subscription
- `changeProducerPause` → pause/resume publishing

---

## Appendix B: mediasoup Rust Crate Usage Example

The mediasoup-rust crate (started 2023) provides a Rust API equivalent to the Node.js version:

```rust
use mediasoup::worker::{Worker, WorkerSettings};
use mediasoup::router::{Router, RouterOptions};
use mediasoup::webrtc_transport::{WebRtcTransportOptions, TransportListenIps};

// 1. Create the Worker (C++ subprocess)
let worker = Worker::new(WorkerSettings {
    log_level: Some(LogLevel::Debug),
    rtc_ports_range: Some((10000, 10100)),
    ..Default::default()
})?;

// 2. Create a Router
let router = worker.create_router(RouterOptions::default())?;

// 3. Create a WebRtcTransport
let transport = router.create_webrtc_transport(
    WebRtcTransportOptions {
        listen_ips: vec![TransportListenIp { ip: "0.0.0.0".parse()?, announced_ip: Some("1.2.3.4".parse()?) }],
        ..Default::default()
    }
)?;

// 4. Get DTLS parameters and ICE candidates (send to the client to complete SDP negotiation)
let dtls_params = transport.dtls_parameters();
let ice_candidates = transport.ice_candidates();
let ice_params = transport.ice_parameters();

// 5. After the client connects, create a Producer
// (after handling the transport's connect event)
let producer = transport.create_producer(ProducerOptions {
    kind: MediaKind::Video,
    rtp_parameters: client_provided_rtp_params,
    ..Default::default()
})?;

// 6. Create a Consumer for another participant
let consumer = transport2.create_consumer(ConsumerOptions {
    producer_id: producer.id(),
    rtp_capabilities: client2_rtp_capabilities,
    ..Default::default()
})?;

// 7. Cascade between Routers
let pipe = router_a.pipe_to_router(
    PipeToRouterOptions {
        producer_id: some_producer_id,
        router: router_b.clone(),
    }
)?;
```

MediaServo's native-core should follow this API design — the Worker/Router/Transport/Producer/Consumer type system in the Rust crate maps directly onto MediaServo's domain model.

---

## Appendix C: Deep Comparison with LiveKit / Jitsi / Janus

**Scenarios where mediasoup is the right choice**:
- Fully custom signaling protocol needed (any of WebSocket/gRPC/MQTT/XMPP)
- The team has deep WebRTC engineering capability (SDP/ICE/DTLS/SRTP stack experience)
- Extreme performance demands (C++ core + Worker process isolation)
- Rust binding needed (integration with MediaServo napi-rs)
- Firm on the "media layer and control layer fully separated" architecture philosophy

**Scenarios where mediasoup is the wrong choice**:
- Need to ship fast (2 weeks vs 2 months development-cycle gap) → pick LiveKit
- No signaling-layer development resources → pick Jitsi Meet (open-source full stack)
- MCU mixing capability needed → pick Janus + the AudioBridge plugin
- Fully hosted needed → pick Agora / LiveKit Cloud

**Hybrid strategy** (recommended for MediaServo):
mediasoup as the core SFU engine + custom signaling (borrowing Colibri2's RESTful design + LiveKit PSRPC + Zoom MMR three-tier scheduling) + Rust bindings (napi-rs → AUDEBase + C-FFI → MediaServo). Take the best of each reference project while avoiding any single project's full dependency set.


---

## Appendix D: mediasoup Performance-Tuning Practices

Key tuning parameters for production mediasoup deployments:

**Worker configuration tuning**:
- `rtcMinPort` / `rtcMaxPort`: bound the ICE port range to shrink firewall rules. Recommended 40000-41000
- `dtlsCertificateFile` / `dtlsPrivateKeyFile`: use long-lived certificates (instead of self-generated ephemeral ones) to avoid certificate-verification overhead in the DTLS handshake
- CPU affinity: pin Worker processes to specific CPU cores via `taskset`, reducing L1/L2 cache misses

**Router configuration tuning**:
- Rooms per Router: recommended ≤ 100 rooms/Router. Create new Routers beyond that
- Max participants per room: conservatively 16 fully interactive. If 50+ meetings are expected, configure Last-N=5 (forward only 5 active speakers' video)
- `rtpRetransmissionBufferSize`: default 1000 packets. On high-loss networks (>5%) raise it to 2000

**Transport configuration tuning**:
- Use ICE-Lite (server mode) for WebRtcTransport — saves one ICE handshake round trip
- Production: `enableUdp: true, enableTcp: false` (TCP fallback is only needed behind corporate firewalls)
- WebRtcServer single-port multiplexing (v3.20+) — the large-deployment best practice

**Producer/Consumer tuning**:
- Simulcast with 3 layers (1080p/720p/360p or 720p/360p/180p). Avoid 4 layers — the 4th layer's CPU cost does not pay for its bandwidth savings
- Call Consumer `setPreferredLayers(spatialLayer, temporalLayer)` dynamically from the bandwidth-estimation callback
- PLI interval: default 1s debounce. On high-loss networks it can be lowered to 500ms

**Network-layer tuning**:
- Raise Linux kernel `net.core.rmem_max` / `net.core.wmem_max` to 26214400 (25MB)
- Raise `net.core.netdev_max_backlog` to 5000
- Disable `tcp_slow_start_after_idle` (TCP fallback in WebRTC scenarios needs no slow start)

**Node.js main-process tuning**:
- Use `--max-old-space-size` to bound the V8 heap size
- Use `--optimize-for-size` to reduce JIT compilation memory footprint
- Never run CPU-heavy work in the main process — all media processing happens in the C++ Workers

**Monitoring metrics**:
- Worker process CPU/memory usage
- Router-level Consumer counts (forecast scaling needs)
- Transport-level packet loss, jitter, RTT
- PLI/FIR request frequency (high frequency = encoder or network problem)
- Per-Consumer bandwidth estimate (locate bandwidth anomalies)

---

## Appendix E: Integration Paths Between mediasoup and the MediaServo WebRTC Stack

MediaServo has technology paths for integrating mediasoup's media capability:

**Path A: use mediasoup directly**
- Node.js calls the mediasoup npm package → C++ Worker subprocess
- Pro: full SFU capability at zero R&D cost. A production-grade recipe validated by BigBlueButton
- Con: Node.js FFI overhead; the signaling layer (Node.js) and MediaServo's backend (Rust/Golang) are not in the same runtime
- Fit: AUDEBase calls the Node.js version of mediasoup through napi-rs

**Path B: Rust secondary wrapper (recommended)**
- mediasoup-rust crate → Rust Worker wrapper → MediaServo native-core
- Pro: consistent with MediaServo's Rust stack. Zero FFI overhead. Signaling layer (Rust) and media layer in the same runtime
- Con: the mediasoup-rust crate is less mature than the Node.js version (started 2023)
- Fit: MediaServo via C-FFI static linking + AUDEBase via napi-rs

**Path C: borrow the design, build your own**
- Borrow mediasoup's Worker/Router/Transport/Producer/Consumer domain model
- Use str0m (sans-I/O WebRTC) or webrtc-rs as the WebRTC stack
- Implement the SFU routing logic yourself
- Pro: fully self-controlled. No external C++ dependency. Pure Rust ecosystem
- Con: the longest R&D cycle. str0m/webrtc-rs Simulcast/SVC support is not as mature as mediasoup's
- Fit: Phase 1.0+ long-term goal

Phase 0 recommended path: mainly Path B (Rust secondary wrapper), with Path C (custom design) as the long-term reserve.
