# Video Conference Product Research Report

> Generated: 2026-07-16
> Purpose: provide references for SFU architecture and signaling design selection for the MediaServo conference module

---

## Table of Contents

- [Architecture Classification Overview](#architecture-classification-overview)
- [SFU Architecture Products](#sfu-architecture-products)
  - [Zoom](#1-zoom)
  - [Google Meet](#2-google-meet)
  - [Microsoft Teams](#3-microsoft-teams)
  - [Jitsi Meet](#4-jitsi-meet)
  - [LiveKit](#5-livekit)
  - [mediasoup](#6-mediasoup)
  - [Janus](#7-janus-gateway)
- [SFU+MCU Hybrid Architecture Products](#sfumcu-hybrid-architecture-products)
  - [BigBlueButton](#8-bigbluebutton)
  - [Tencent Meeting](#9-tencent-meeting)
- [SD-RTN Network-Layer Products](#sd-rtn-network-layer-products)
  - [Agora](#10-agora)
- [Synthesis](#synthesis)

---

## Architecture Classification Overview

| Architecture type | Products |
|---------|------|
| **Pure SFU** | Zoom, Google Meet, Jitsi, LiveKit, mediasoup, Janus |
| **SFU+MCU hybrid** | Microsoft Teams, BigBlueButton, Tencent Meeting |
| **SD-RTN (global private network)** | Agora |

---

# SFU Architecture Products

## 1. Zoom

### Overview

Zoom is one of the video conferencing platforms with the highest market share worldwide. In April 2020 daily meeting participants peaked at 300 million (including duplicate counts). It offers a full UCaaS product line including Meetings, Webinars, Phone, and Contact Center. The core of the architecture is the proprietary MMR (Multi-Media Router), which is essentially an SFU. It operates dozens of data centers globally, with 99.9% availability.

### Architecture

**SFU (MMR)** — Zoom's MMR is a distributed subscriber-style SFU that performs no server-side transcoding or mixing. The core architecture has three layers:
- **MMR (Meeting Server)**: the media routing node, responsible for selectively forwarding RTP/SRTP packets
- **Zone Controller**: manages load balancing and health status of the MMR cluster within a zone
- **Global Cloud Controller**: cross-zone scheduling, routing users to the nearest available Zone based on geolocation

**Cascading strategy**: participants in the same zone are assigned to the same MMR; cross-zone meetings cascade MMRs over Zoom's global backbone network. Zoom Node supports hybrid deployment (local MMR + cloud MMR). Adaptive transport layer: UDP preferred, with seamless fallback to TCP/TLS (443). Screen sharing uses Reliable UDP.

### Tech stack

- **Signaling**: proprietary protocol over HTTPS/WebSocket; the Zone Controller assigns the optimal MMR
- **Codecs**: proprietary adaptive codec, multiple simultaneous stream sends, the client dynamically selects the layer. Calls remain usable at ~45% packet loss
- **Bandwidth adaptation**: monitors bandwidth, packet loss, latency, and jitter plus CPU/memory/IO, and dynamically adjusts frame rate and resolution
- **Security**: SRTP + DTLS, with optional E2EE

### Latency & Scalability

- 2 participants use P2P; 3+ participants switch to SFU. Webinar mode sends large-scale viewers over HLS/DASH CDN
- Backend is deployed across multiple global regions, with geographic nearest-edge access and cross-zone backbone interconnection
- **Lesson**: Zoom Web SDK video goes over DataChannel (TCP), which has observable freezing problems on moderate WiFi

### Pros & Cons

**Pros**: full-stack proprietary with end-to-end optimization; large-scale validated SFU architecture; audio-first policy protects call continuity
**Cons**: closed source, cannot be self-hosted; not a native WebRTC protocol, isolated from the open-source ecosystem; Web SDK performance is worse than native clients

### Adoptable Designs

- **MMR three-layer architecture**: the layered scheduling pattern of Global Controller → Zone Controller → MMR
- **Adaptive transport**: UDP/TCP fallback + multiple simultaneous streams + client-side layer selection
- **Hybrid deployment**: the Zoom Node model allows local + cloud MMR to coexist
- **Audio-first**: guarantees audio at 45% packet loss while degrading video

---

## 2. Google Meet

### Overview

Google Meet is the core video conferencing product of Google Workspace, formerly Google Hangouts. It is based on the WebRTC standard and runs entirely in the browser. It is deployed globally, leveraging Google's global network infrastructure. Rollout of the AV1 codec began in 2023.

### Architecture

**Pure SFU + Virtual Media Streams** — Meet uses a unique "virtual media stream" architecture:
- Each client maintains **exactly 3 audio transceivers** and **1~3 video transceivers** (fixed count, unchanged over the lifecycle)
- The SFU multiplexes media from multiple participants onto these few fixed streams, using the RTP CSRC (Contributing Source) field to identify the real source
- Audio: the SFU always forwards the audio of the 3 loudest current speakers; the SSRC stays fixed while the CSRC switches dynamically
- This is equivalent to doing **logical mixing at the SFU layer** — clients only need to handle a fixed number of streams, reducing client load

**Codecs**: supports VP8, VP9, AV1. Video uses VP9 SVC (L3T3_KEY mode), with SVC dynamically enabled/disabled according to participant count. AV1 is tested via gradual rollout during the pre-join phase (sending a 320×180 warm-up stream to obtain a bandwidth estimate), falling back to VP9 SVC for the actual call.

### Signaling

- Signaling goes through the Google Meet REST API (HTTPS)
- SDP offer/answer model: the client always sends the offer, and the Meet server replies with the answer
- Must support the `transport-cc` extension and Opus audio encoding
- The data channel (SCTP) is used for `session-control` and `media-stats`

### Latency & Scalability

- Leverages Google's global network infrastructure, with edge nodes for nearest-edge access
- AV1 codec can reduce bitrate by 30-50% (vs VP9/H.264), keeping video alive at 40kbps
- **Lesson**: when VP9 SVC is unavailable on a hardware decoder it falls back to software decoding, increasing client CPU load

### Pros & Cons

**Pros**: pure WebRTC standard, deeply integrated with browsers; virtual media streams reduce client complexity; robust AV1 rollout strategy (progressive gradual rollout); backed by Google's global network infrastructure
**Cons**: cannot be self-hosted; only supports Chrome/Edge/Firefox; advanced features such as recording depend on Google Cloud

### Adoptable Designs

- **Virtual Media Streams**: fixed-count SSRC + dynamic CSRC switching, so clients only need to handle 3+3 streams
- **Gradual AV1 rollout**: first use AV1 for warm-up traffic to obtain a bandwidth estimate without exposing user video
- **Dynamic SVC**: toggle SVC according to participant count (fewer participants → no SVC, avoiding software decoding cost)
- **CSRC active-speaker switching**: the server decides the loudest 3 speakers, no renegotiation needed on the client

---

## 3. Microsoft Teams

### Overview

Microsoft Teams is the core collaboration platform of the Microsoft 365 ecosystem, integrating chat, meetings, calls, and file collaboration. The meeting architecture is based on the Azure Communication Services infrastructure and evolved from Skype for Business. It supports up to 1000 interactive participants + 10000 view-only participants.

### Architecture

**SFU+MCU hybrid** — the Teams Media Processor can switch between SFU and MCU modes depending on the scenario:
- **1:1 calls**: prefer direct P2P connection (via ICE/STUN/TURN); media does not pass through the server
- **Multi-party meetings**: SFU mode, using the Azure media processor
- **PSTN access**: MCU mode, doing codec conversion and mixing
- The media processor also handles transcription, recording, and live captions at the same time

**Transport layer**: SRTP over UDP preferred, with TCP fallback. ICE integrity checks determine the best path. Transport Relay supports traversal of enterprise firewalls.

### Signaling

- Signaling uses HTTPS REST (TLS 1.2)
- SDP offer/answer model
- PSTN scenarios use the SIP protocol to interconnect with SBCs
- In-meeting video subscription management may go through SCTP/DataChannel

### Codecs & Security

- Supports H.264, H.265/HEVC, AV1 (newer versions)
- Audio uses Opus and SILK
- SRTP encryption (SDES preferred over DTLS for backward compatibility with SBCs)
- Enterprise compliance: data residency, eDiscovery, retention policies

### Latency & Scalability

- Nearest-edge access via Azure global data centers
- Lowest latency with direct P2P; SFU forwarding adds a small amount of latency
- The media processor scales elastically and is assigned according to the meeting's region
- **Lesson**: 1:1 calls and meetings use different media paths (DataChannel subscription management); in VDI-optimized scenarios the bidirectional SCTP channel must be kept clear

### Pros & Cons

**Pros**: deeply integrated with the M365 ecosystem; flexible SFU/MCU switching covers all scenarios; enterprise-grade security and compliance; native PSTN support
**Cons**: closed source, cannot be self-hosted; complex architecture (session border controller, media processor, transport relay — three layers); limited Web client features; depends on Azure infrastructure

### Adoptable Designs

- **Dynamic SFU/MCU switching**: adaptively choose the media path according to the scenario
- **ICE/STUN/TURN relay architecture**: a complete NAT traversal solution
- **DataChannel subscription management**: WebRTC DataChannel used as signaling control for SFU video subscriptions
- **Direct Routing**: an SBC-based media bypass scheme for PSTN interconnection

---

## 4. Jitsi Meet

### Overview

Jitsi Meet is one of the most popular open-source video conferencing solutions, maintained by 8x8. It is fully based on the WebRTC standard and offers self-hosted deployment. It consists of three core components: Jitsi Videobridge (JVB), Jicofo, and Prosody. The community is active, and it absorbed enormous traffic growth during the pandemic.

### Architecture

**Pure SFU (Jitsi Videobridge)** — JVB is a Java-implemented SFU that performs no transcoding or mixing:
- **JVB (Jitsi Videobridge)**: the SFU media server, implemented in pure Java. Each JVB instance can handle hundreds of conferences. It ingests streams on UDP port 10000 and supports Simulcast, VP8/VP9/H.264
- **Jicofo (JItsi COnference FOcus)**: the signaling focus. Manages XMPP MUC rooms, selects the optimal JVB for each participant, and allocates media endpoints through the Colibri2 protocol
- **Prosody**: the XMPP server, providing the signaling channel and MUC room management

**Cascading and region routing** (new architecture since 2022):
- **JVB Pools**: JVBs are no longer bound to a specific shard. Each region has an independent JVB pool with automatic scaling
- **Remote Pools**: cross-zone JVB pools connect to signaling nodes in other regions, avoiding full-mesh explosion
- **Region Groups**: groups nearby regions (e.g. Frankfurt + London) to avoid unnecessary cascading
- **Secure Octo**: JVBs connect to each other via ICE/DTLS-SRTP, replacing the old VPN scheme. Unneeded streams can be filtered

### Signaling

- **XMPP + Jingle**: participants coordinate through XMPP MUC rooms
- **Colibri2**: the RESTful signaling protocol between Jicofo and JVB, used for endpoint allocation, ICE candidate exchange, and source management
- Both HTTP/JSON and XMPP Colibri transports are supported

### Latency & Scalability

- Cross-zone cascading adds 80-150ms of latency
- Nearest-edge regional access reduces next-hop RTT by about 29% (from 223ms down to 158ms in cross-continent scenarios)
- Horizontal scaling: JVB pools scale independently, signaling nodes are deployed on demand
- **Lesson**: in 2020 the full-mesh JVB design collapsed at 50+ shards and 2000+ JVBs during the pandemic; recovery only came after the redesign into the Pools + Remote Pools architecture

### Pros & Cons

**Pros**: fully open source; elegant signaling architecture (Colibri2 RESTful API); mature multi-region Bridge Cascading; decoupled components that scale independently
**Cons**: Java implementation consumes a lot of memory; the XMPP protocol stack is heavy; recording (Jibri) depends on Chrome + ffmpeg virtual frame buffers; lacks a native MCU

### Adoptable Designs

- **Colibri2 protocol**: a RESTful SFU control protocol supporting create/modify/destroy of conference endpoints, ICE candidate exchange, and source management
- **Pools + Remote Pools cascading architecture**: group JVBs by region, with remote pools crossing zones, avoiding full interconnection
- **Secure Octo**: secure direct ICE/DTLS links between JVBs, no VPN needed
- **Bridge Selection Strategy**: pluggable bridge selection strategies (single bridge / region / load balancing / visitor separation)

---

## 5. LiveKit

### Overview

LiveKit is an emerging open-source SFU implementation written in Go. It is positioned as an "opinionated SFU" — with built-in signaling protocol, distributed coordination, and an AI Agent framework. The founding team comes from Twitch/Amazon. It offers both open-source self-hosted and cloud-hosted modes. It has gained substantial community adoption (especially in AI voice scenarios).

### Architecture

**Go-implemented SFU** — layered architecture, six layers by design:
- **Service Layer**: HTTP/Twirp API, the signaling entry point
- **Routing Layer**: the Router interface abstraction (LocalRouter / RedisRouter), room-to-node mapping
- **Core RTC Layer**: Room/participant lifecycle management
- **SFU Pipeline**: WebRTCReceiver → Buffer → Forwarder/DownTrack (three-stage pipeline)
- **Storage Layer**: ObjectStore (RedisStore/LocalStore)
- **Infrastructure**: Wire DI, Telemetry, Config

**Distributed coordination**: Redis as the shared state store and message bus. PSRPC (PubSub RPC) implements inter-node RPC. Node health is monitored through Keepalive heartbeats.

### Media Processing

- **Ingestion**: WebRTCReceiver receives RTP packets; the Buffer caches them and handles retransmissions
- **Processing**: the Forwarder does adaptive layer selection; VideoAllocation allocates based on TWCC/REMB bandwidth estimates
- **Egress**: DownTrack does RTP header munging (SSRC/SN/timestamp rewriting) and per-subscriber pacing
- **Dynacast**: monitors the number of layer subscribers, pauses encoding layers with no subscribers, saving publisher uplink bandwidth
- Supports VP8/VP9/H.264/AV1, with SVC support for VP9/AV1

### Signaling

- Proprietary WebSocket signaling protocol
- Client SDKs encapsulate the signaling details, with multi-platform support
- Token-based authentication mechanism

### Latency & Scalability

- A single SFU hop adds 10-30ms of latency (vs P2P)
- A single node can route hundreds of simultaneous video streams
- Redis Router provides multi-node distribution with nearest-edge regional access
- Prometheus metrics monitoring: participant_count, packet_loss_rate, forwarded_rtp_total

### Pros & Cons

**Pros**: written in Go with memory safety and simple deployment (single binary); built-in signaling works out of the box; forward-looking AI Agent framework; Dynacast reduces bandwidth waste; excellent documentation
**Cons**: relatively young, less production validation than mediasoup/Jitsi; strong dependency on Redis; non-standard signaling protocol; community ecosystem still growing

### Adoptable Designs

- **Dynacast**: closed-loop bitrate management of encoding on demand, where publishers only encode layers that have subscribers
- **PSRPC**: a type-safe RPC framework based on Redis Pub/Sub
- **Router abstraction**: LocalRouter/RedisRouter interface-based routing for flexible deployment
- **Single binary**: a Go build artifact with simple operations

---

## 6. mediasoup

### Overview

mediasoup is a high-performance SFU library with a C++ implementation for core media processing, wrapped for Node.js as a server-side module. It is positioned as "minimalist" — handling only the media layer and binding to no signaling protocol. It is widely used in independently built video conferencing systems (Topology, the video part of BigBlueButton, and so on). v3 is the current mainstream version.

### Architecture

**C++ Worker + Node.js application layer**:
- **Worker**: one C++ subprocess per CPU core, implementing a true SFU (Router)
- **Router**: behaves as a strict SFU — pure RTP packet forwarding, no decoding or transcoding. Supports Simulcast/SVC layer selection, RTP retransmission cache, and PLI/FIR request aggregation
- **Transport**: WebRTC or Plain RTP connection endpoints
- **Producer/Consumer**: publish/subscribe model

**Performance model**: a single Worker (~1 CPU) can handle about 500 Consumers. The Consumer count in an N-person fully interactive room = N × (N-1) × 2 (each person consumes everyone else's audio + video), growing quadratically.

### Signaling

- **Completely unbound from any signaling protocol** — developers must implement the signaling layer themselves
- The server calls mediasoup methods through the Node.js API (createRouter, createTransport, pipeToRouter, etc.)
- The `pipeToRouter()` API implements Router-to-Router cascading (same host or across hosts)

### Scalability

- **Same-host scaling**: multiple Workers share the load, with Routers assigned per room
- **Cross-host cascading**: `pipeToRouter()` connects Routers on different hosts. The application layer is responsible for cross-host signaling mediation (e.g. Redis Pub/Sub or REST)
- **Large-room strategy**: conservative limit of ~16 people per Router. When exceeded, create a new Router and cascade via pipeToRouter
- **Lesson**: fully interactive large rooms (>50 people) are a weakness of mediasoup; the Community explicitly recommends limiting the number of simultaneous speakers rather than scaling without limit

### Pros & Cons

**Pros**: extreme performance (C++ SFU core); signaling-agnostic with maximum flexibility; Worker isolation, so a single Worker crash does not affect the others; Lightweight — CPU consumption far lower than MCU solutions
**Cons**: provides no signaling layer, requiring a large amount of development work; scaling requires building your own orchestration system; limited large-room capability (recommended not to exceed 16 people fully interactive); Node.js calling C++ incurs FFI overhead

### Adoptable Designs

- **Router.pipeToRouter()**: an elegant cascading abstraction, similar to a Unix pipe
- **PLI/FIR aggregation**: aggregate multiple Consumers' keyframe requests into a single upstream request, debounced by 500ms~1s
- **Worker isolation**: each Worker is an independent C++ process, isolating crashes
- **Signaling agnosticism**: the design philosophy of separating the media layer from the control layer

---

## 7. Janus Gateway

### Overview

Janus is an open-source general-purpose WebRTC server developed by Meetecho, written in C. Its core idea is a "plugin architecture" — the core implements only the WebRTC protocol stack (JSEP/SDP, ICE, DTLS-SRTP, DataChannel), and all application logic is implemented by plugins. It is one of the longest-standing WebRTC SFUs and is widely used in academia and among small and medium enterprises.

### Architecture

**C core + plugin system**:
- **Core**: implements the complete WebRTC protocol stack, and manages memory and CPU
- **Plugins**: business logic loaded as dynamic libraries. Each plugin provides one function — EchoTest, VideoRoom (SFU), AudioBridge (MCU mixing), Streaming (RTSP/RTMP injection), SIP Gateway
- **API Transports**: multiple transport layers — HTTP/REST, WebSocket, RabbitMQ, MQTT, Nanomsg, Unix Sockets

### VideoRoom plugin (SFU)

- Publish/subscribe model: participants publish media streams and other participants subscribe
- Supports multistream (subscribing to multiple streams on a single PeerConnection) and legacy mode (a separate PeerConnection per subscription)
- Publishing and subscribing use different PeerConnections (a design choice that avoids renegotiation complexity)
- RTP Forwarding: forwards a publisher's RTP/RTCP stream to an external service (recording, transcoding, analytics)

### Signaling

- Multi-layer APIs: HTTP REST, WebSocket, RabbitMQ, MQTT, and more
- The janus.js client library encapsulates the interaction
- Plugins can be combined to implement complex scenarios (e.g. VideoRoom + AudioBridge for a conference with mixing)

### Latency & Scalability

- Excellent C performance with small per-process overhead
- Note: VideoRoom does not support putting subscribers and publishers on the same PeerConnection (a design limit)
- Can be deployed on edge devices (Raspberry Pi, drones, etc.) for IoT scenarios
- **Lesson**: the plugin architecture is flexible, but the complexity of combining plugins is easily underestimated

### Pros & Cons

**Pros**: extremely lightweight C implementation; highly extensible plugin architecture; rich choice of API transport layers; friendly to academic research (argued in ACM papers); deployable on resource-constrained devices
**Cons**: relatively dated development experience (C language); steep learning curve for the plugin API; separating VideoRoom publish/subscribe PeerConnections increases client complexity; documentation is Doxygen-generated with mediocre readability

### Adoptable Designs

- **Plugin architecture**: fully decouples application logic from the WebRTC protocol stack
- **RTP Forwarding**: SFU streams can be forwarded to external non-WebRTC endpoints (recording services, AI analytics, etc.)
- **Multi-API transport layers**: the same core supports HTTP/WS/MQTT/RabbitMQ control protocols
- **Function composition**: combining different plugins implements complex scenarios (VideoRoom + AudioBridge + SIP)

---

# SFU+MCU Hybrid Architecture Products

## 8. BigBlueButton

### Overview

BigBlueButton is the most popular open-source online education / virtual classroom platform in the world. Maintained by Blindside Networks, it is deeply optimized for teaching scenarios — whiteboard, presentation, group discussion, voting, recording and playback. The current stable release is 3.0. It is deployed on Ubuntu servers with components managed via deb packages.

### Architecture

**SFU + MCU dual media architecture**:
- **Audio (FreeSWITCH — MCU)**: a traditional MCU architecture handles voice conferencing. All participants' audio is mixed on the FreeSWITCH server, and each person downloads one mixed audio stream. Supports PSTN dial-in. Problem: CPU intensive — a 50-person meeting consumes a lot of resources even if most are muted
- **Video (mediasoup — SFU)**: uses mediasoup to handle camera video and screen sharing. Forwarded directly without decoding. Supports VP8/H.264
- **In transition**: audio is being migrated from the FreeSWITCH MCU to the mediasoup SFU. The experimental "transparent listen-only mode" is already available — FreeSWITCH channels are hung up automatically when muted, saving CPU

### Component System

- **bbb-webrtc-sfu (Node.js)**: signaling server, bridging clients to mediasoup/FreeSWITCH
- **bbb-apps-akka (Scala)**: the core application service, maintaining conference state
- **Redis Pub/Sub**: the inter-component communication bus
- **bbb-webrtc-recorder**: the recording component, supporting WebM format
- **Kurento**: the old media server (deprecated, migrated to mediasoup)

### Codecs & Configuration

- Audio: Opus 48kHz, 2ch, 30kbps max, FEC enabled
- Video: VP8/H.264, 1500kbps default cap
- Worker allocation: the `auto` strategy = `ceil((min(nproc,32) * 0.8) + (max(0, nproc - 32) / 2))`
- Supports assigning dedicated Workers by media type (independent pools for audio/main/content)

### Latency & Scalability

- A single server can handle several medium-sized conferences (depending on mediasoup Worker and FreeSWITCH capacity)
- Horizontal scaling requires a custom load balancer (the community provides example solutions)
- **Lesson**: FreeSWITCH MCU audio is the biggest bottleneck — a 50-person meeting may consume more CPU than a 200-person listen-only meeting; the migration to a full-SFU mediasoup architecture is underway at full pace

### Pros & Cons

**Pros**: the most complete feature set for teaching scenarios (whiteboard, polling, group discussion rooms, recording playback); fully open source and self-hostable; excellent mediasoup video architecture performance; Workers isolated by media type
**Cons**: the FreeSWITCH MCU audio bottleneck is obvious; many components make deployment complex (10+ deb packages); realtime communication latency is worse than pure SFU; the architecture is fragile during the migration transition period

### Adoptable Designs

- **Isolating Workers by media type**: audio/main/content use independent mediasoup Worker pools
- **Transparent Listen Only mode**: smart audio management that automatically releases server resources when muted
- **Pluggable Recording Adapter**: supports multiple recording backends — Kurento/native/bbb-webrtc-recorder
- **FS Bridge Mode**: bridging FreeSWITCH in dual RTP/WebRTC modes, adapting flexibly to deployment environments

---

## 9. Tencent Meeting

### Overview

Tencent Meeting is built on Tencent's 20 years of accumulated audio/video technology and launched at the end of 2019. It uses the proprietary xCast engine (the third-generation audio/video engine; the previous two generations were the QQ TRAE engine and the OpenSDK engine). It supports all platforms, holds one of the highest market shares in the Chinese market, and supports 100,000-party meetings. Local deployment options are provided to meet compliance requirements.

### Architecture

**SFU+MCU hybrid, proprietary xCast engine**:
- **Pere transport layer**: the network transport layer of xCast. Not a WebRTC implementation, but a proprietary RTP protocol stack. Supports multiple transports — DataChannel/WebSocket/WebTransport
- **Media server**: the SFU forwards video (layered encoding, no decoding or transcoding); the MCU mixes audio (configurable for server-side or client-side mixing)
- **Signaling**: proprietary protocol, dynamically distributing stream control parameters through the Spear cloud configuration system
- **Multi-point distributed deployment**: nearest-edge access, multi-server collaboration, dynamic adjustment, and mutual backup. Supports 100,000-party meetings with automatic migration on disconnect

### QoE Optimization System (ARC)

- **Configuration layer**: the Spear cloud parameter system pushes codec/3A/loss-resistant parameters in real time
- **Dynamic control**: stream control (ARC) dynamically adjusts FEC/retransmission/jitter buffer based on network jitter and packet loss
- **Congestion control**: based on an improved GCC, switching adaptively between latency-first (most networks) and loss-first (cross-carrier networks)
- **Preferential retransmission**: when latency allows, prefer retransmission (saving bandwidth); when insufficient, supplement with FEC

### Codecs

- Video: H.264/H.265/AV1 (proprietary codecs integrated in xCast)
- SVC is used for downstream stream control — temporal/spatial layer selection
- The Web client runs proprietary codecs in WebAssembly instead of the browser's built-in WebRTC codecs

### Security & Compliance

- National cryptographic standards
- Dedicated-network meetings (media data stays local the entire time, never leaving the public network) plus public-network meetings — dual mode
- 1+1 cluster backup architecture with automatic failover to a remote disaster recovery site
- SSO login restrictions, intranet access permissions, audio/video watermarking, and audit trails

### Pros & Cons

**Pros**: end-to-end optimization of the proprietary engine; deep QoE optimization system; flexible SFU/MCU switching; 100,000-party meeting capability; local deployment meets compliance
**Cons**: completely closed source; non-WebRTC standard protocols; network conditions for overseas scenarios are inferior to global SD-RTN products; limited public technical details

### Adoptable Designs

- **End-to-end QoE optimization**: model video conferencing as a constrained optimization problem (latency, bandwidth) with unified decision-making across modules
- **Realtime adaptive congestion control**: different algorithms for different network types (delay-based for delay-sensitive networks, loss-based for loss-sensitive networks)
- **Spear cloud configuration**: a stream control system that distributes parameters dynamically without client upgrades
- **Tiered deployment**: a dual-track dedicated-network + public-network mode covering both compliance and flexibility needs

---

# SD-RTN Network-Layer Products

## 10. Agora

### Overview

Agora is a globally leading realtime engagement (RTE) PaaS platform. It does not directly build video conferencing products but provides underlying SDKs for developers to build their own RTC applications. It reaches 50+ billion monthly call minutes, covering 200+ countries and regions. Its core moat is the self-built global SD-RTN™ private network.

### Architecture

**SD-RTN™ (Software-Defined Real-Time Network)**:
- 200+ global POP (Points of Presence) nodes with full-mesh measurement
- Every POP is both an access point and a routing node
- **Intelligent routing**: measures the latency of every path in real time, with ML algorithms selecting the optimal route. Three-way redundant sending (data is transmitted simultaneously over the three best paths, and the first packet to arrive is used)
- **Large Channel**: for large-scale broadcast scenarios, routing is optimized so that as many participants as possible share the path
- Essentially a network-layer implementation of the SFU idea — forwarding without decoding, but routing optimization is done at the network layer rather than the application layer

### Performance Metrics

- Global median latency <76ms
- A single channel supports 1+ million simultaneous participants
- 99.99% availability
- Zero full-system outages in company history
- Same-region latency 32ms (50th percentile), 33ms (95th)
- Cross North America–Europe: 62ms (50th), 83ms (95th)

### Signaling & SDK

- The signaling SDK is independent of the media SDK, supporting three channel types — pub/sub, 1:1, and stream
- Token-based authentication system (App ID + App Certificate)
- SDK coverage across all major platforms (iOS, Android, Web, Windows, macOS, Unity, Flutter, React Native, Electron)
- The Web SDK uses a proprietary alternative to WebRTC — running proprietary codecs in WASM to bypass browser limitations

### Latency & Scalability

- Compared to public-network CDN (1-30 seconds of latency), SD-RTN provides sub-400ms latency
- Dynamic scaling: nodes can be added quickly at the local data center or continental level
- Intelligent monitoring + automatic routing: data center failures are routed around automatically
- **Lesson**: public BGP routing suffers from staleness (routing table updates cannot keep up with actual network changes); SD-RTN bypasses this problem with realtime measurement over a full-mesh topology

### Pros & Cons

**Pros**: lowest global network latency; the widest SDK coverage; extremely high availability (99.99%); pay-per-usage with no upfront investment; the proprietary Web SDK bypasses WebRTC limitations
**Cons**: the PaaS model cannot be self-hosted; costs grow linearly with usage; data flows over a third-party network; there are compliance differences between the mainland China and international editions

### Adoptable Designs

- **Three-way redundant transport**: push three paths at once and use the fastest arriving packet, effectively reducing tail latency
- **SD-RTN as an overlay**: building a low-latency overlay network on top of the public internet
- **Full-mesh + realtime measurement**: every POP measures path quality to all other POPs
- **Large Channel**: routing path optimization sharing for large-scale scenarios

---

# Synthesis

## SFU Architecture Comparison Table

| Dimension | mediasoup | Janus | Jitsi JVB | LiveKit | Zoom MMR | Google Meet |
|------|-----------|-------|-----------|---------|----------|-------------|
| **Language** | C++/Node.js | C | Java | Go | Closed source (C/C++ presumed) | Closed source (C/C++) |
| **Open source** | MIT | GPLv3 | Apache 2.0 | Apache 2.0 | No | No |
| **Signaling** | None (self-implemented) | HTTP/WS/RabbitMQ/MQTT | Colibri2 (REST/XMPP) | WebSocket (proprietary) | Proprietary | REST (SDP) |
| **Simulcast** | ✅ | ✅ VideoRoom | ✅ | ✅ | ✅ (proprietary multi-stream) | ❌ (uses SVC) |
| **SVC** | ✅ | ❌ | ✅ | ✅ (VP9/AV1) | ❌ | ✅ (VP9/AV1) |
| **Cascading** | pipeToRouter | ❌ | Secure Octo | Redis Router | Backbone cascading | Network layer |
| **Recording** | Self-build required | RTP Forward | Jibri (Chrome) | Egress | Native | Native |
| **E2EE** | Optional | Optional | Optional | ❌ | Optional (Insertable Streams) | ❌ |
| **Worker isolation** | ✅ (process) | ❌ | ❌ | ❌ | ✅ (node) | ✅ (node) |
| **AI Agent** | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ |
| **Deployment complexity** | High (requires self-built signaling) | Medium | Medium | Low (single binary) | Cannot self-host | Cannot self-host |

## MediaServo Technology Selection Recommendations

### SFU Selection

**Recommendation**: **mediasoup** as the core SFU engine

Rationale:
1. **Best performance**: C++ implementation, Worker process isolation, extreme CPU efficiency
2. **Maximum flexibility**: signaling-agnostic, the control plane can be fully customized
3. **Cascading ecosystem**: `pipeToRouter()` supports same-host/cross-host cascading, suitable for MediaServo's multi-scenario needs
4. **Production validation**: validated by large-scale deployments such as BigBlueButton
5. **Rust FFI friendly**: can be wrapped as a Rust crate via napi-rs or FFI and integrated into MediaServo's native-core

### Signaling Design

**Recommendation: build a proprietary signaling protocol, borrowing the Colibri2 + PSRPC pattern**:

```
Suggested signaling layer design:
├── Transport: WebSocket (realtime) + gRPC (inter-service)
├── Protocol: JSON/Protobuf dual mode (JSON for Web, Protobuf for native)
├── State sync: Redis Pub/Sub (borrowing LiveKit PSRPC)
├── Room management: borrowing Jicofo's pluggable Bridge Selection Strategy pattern
└── Endpoint management: borrowing the Colibri2 RESTful API design
    - POST /conferences → create conference + assign SFU
    - PATCH /conferences/{id} → update endpoints/ICE/sources
    - DELETE /conferences/{id} → destroy
    - GET /conferences/{id}/dominant-speaker → active speakers
```

### Audio Solution

**Recommendation**: SFU forwarding + client-side mixing as the primary approach, with MCU mixing as a supplement (PSTN/recording bridging)

Borrowing the BigBlueButton lesson: MCU audio is the scalability bottleneck. Prefer SFU-forwarded audio with clients decoding multiple streams and mixing locally. Use MCU only for PSTN bridging and recording scenarios.

### Recording Solution

**Recommendation: a two-layer architecture**:
- **Realtime recording**: like Janus RTP Forwarding — the SFU duplicates a stream and pushes it to a recording service (see the mediasoup Producer → external Consumer pattern)
- **Asynchronous composition**: like Zoom's post-meeting batch composition — store each track independently and compose the final video on GPU/CPU after the meeting ends

### Key Lessons Summary

1. **Do not carry media over TCP**: Zoom Web SDK's DataChannel video is a counterexample
2. **PLI/FIR aggregation is essential**: 100 subscribers requesting keyframes simultaneously = an encoder disaster
3. **Separate the signaling layer**: mediasoup's success — decoupling the media layer from the control layer is key to long-term maintainability
4. **Worker isolation**: process isolation > thread isolation > no isolation
5. **Fully interactive large rooms are a false requirement**: mediasoup community experience — limiting the number of simultaneous speakers is more practical than pursuing unlimited scaling
6. **A full-mesh is not scalable**: Jitsi's 2020 lesson — a full-mesh at 50+ shards collapsed, the Pools model is mandatory
7. **Gradual AV1 rollout**: the Google Meet strategy — test a new codec with warm-up traffic without exposing user data
8. **Estimate bandwidth via simulated sending**: Google Meet's pre-join sends a 320×180 stream to obtain an accurate bandwidth estimate
9. **E2EE is compatible with SFU forwarding**: you only need read/write access to the RTP header (decrypt SRTP and re-encrypt), leaving the payload untouched
10. **Congestion control must be adaptive**: Tencent Meeting experience — different algorithms for different network types, latency-sensitive vs loss-sensitive

### Architecture Blueprint: MediaServo Conference Module

```
┌─────────────────────────────────────────────────────┐
│                    Client (Web/Native)               │
│  ┌─────────┐  ┌──────────┐  ┌────────────────────┐  │
│  │ Camera  │  │ Microphone│  │ Screen Capture     │  │
│  └────┬────┘  └────┬─────┘  └─────────┬──────────┘  │
│       │ VP8/AV1    │ Opus           │ VP8/AV1       │
│       ▼            ▼                ▼               │
│  ┌─────────────────────────────────────────────┐    │
│  │         WebRTC Encoder (Simulcast + SVC)    │    │
│  └────────────────────┬────────────────────────┘    │
│                       │ SRTP/UDP                     │
└───────────────────────┼─────────────────────────────┘
                        │
┌───────────────────────┼─────────────────────────────┐
│                 Signaling (WebSocket)                │
│  ┌────────────────────┴────────────────────────┐    │
│  │         Conference Controller                │    │
│  │  (Rust: room mgmt, SFU selection, ICE relay) │    │
│  └────────────────────┬────────────────────────┘    │
│                       │                              │
│  ┌────────────────────┴────────────────────────┐    │
│  │         mediasoup SFU Engine                 │    │
│  │  (C++ workers, napi-rs binding)              │    │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  │    │
│  │  │ Worker 1 │  │ Worker 2 │  │ Worker N │  │    │
│  │  │ Router A │  │ Router B │  │ Router C │  │    │
│  │  └──────────┘  └──────────┘  └──────────┘  │    │
│  └─────────────────────────────────────────────┘    │
│                       │                              │
│  ┌────────────────────┴────────────────────────┐    │
│  │    Recording / ASR / Analytics Pipeline      │    │
│  │    (RTP Forwarding → external consumers)     │    │
│  └─────────────────────────────────────────────┘    │
│                       │                              │
│  ┌────────────────────┴────────────────────────┐    │
│  │    Redis: state sync, pub/sub, room routing  │    │
│  └─────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────┘
```

---

> **Next steps**: based on the conclusions of this research, refine the component selection and interface design of the conference module.

## Related Decisions

| Research finding | Related decision |
|---------|---------|
| SFU forwards the loudest N streams (Google Meet) | D50 |
| mediasoup Worker-per-core (Jitsi/mediasoup) | D-SFU-WORKER |
| Audio-first QoS degradation chain (Zoom/Jitsi) | D-QOS-AUDIO |
| Colibri2 RESTful resource modeling (Jitsi) | Phase 2 design reference |
| LiveKit pure-SFU pluginization | D14, D97 |
| GOP Cache (SRS/MediaMTX) | D-GOP-CACHE |
| Scenario-aware Simulcast/Dynacast | D-SIMULCAST |
| Unified Room model (LiveKit) | D53 |
