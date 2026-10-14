# Jitsi Meet Reference Analysis
> Generated: 2026-07-16 | Category: Video Conferencing

## 1. Product Profile
- **Name**: Jitsi Meet
- **Developer**: 8x8, Inc. (acquired in 2018 from Atlassian's Jitsi team). Core committers: Boris Grozev, Emil Ivov, Saúl Ibarra Corretgé, et al.
- **First release**: 2013 (Jitsi Meet web client); Jitsi Videobridge traces back to 2012
- **Product positioning**: The world's most popular open-source full-stack video-conferencing solution. A complete self-hosted offering from web client to signaling layer to SFU media layer to recording/streaming/transcription/PSTN gateway. Emphasizes three principles: "secure, simple, scalable"
- **Target user groups**:
  - Government and public sector (data sovereignty, GDPR/Schrems II compliance mandates self-hosting)
  - Higher-education institutions (universities self-hosting video teaching platforms, zero license fees)
  - Healthcare (HIPAA-compliant self-hosted setups; patient data never leaves owned infrastructure)
  - Internal enterprise communications (organizations with extremely high data-privacy requirements, e.g. law firms, audit firms)
  - Nonprofits and open-source communities (low-cost, high-quality video conferencing)
- **License / business model**: Apache 2.0 (100% open source, no closed-source components at all). 8x8 offers the "8x8 Meet" commercial SaaS (built on Jitsi). Community self-hosting is completely free, with no license fees whatsoever

## 2. Technical Features

### 2.1 Overall Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                        Client layer                           │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐             │
│  │ Web client │  │ Mobile App │  │ Electron   │             │
│  │ (React)    │  │ (RN)       │  │ Desktop    │             │
│  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘             │
│        │               │               │                     │
│  ┌─────┴───────────────┴───────────────┴─────┐               │
│  │       lib-jitsi-meet (JS media API)        │               │
│  │       WebRTC SDP/JSEP wrapper              │               │
│  │       getStats / device enum / SDP munging │               │
│  └─────────────────────┬─────────────────────┘               │
│                        │                                     │
└────────────────────────┼──────────────────────────────────────┘
                         │ WebSocket (XMPP) / HTTPS
┌────────────────────────┼──────────────────────────────────────┐
│                      Signaling layer                            │
│  ┌────────────────────┴─────────────────────┐                 │
│  │            Prosody (XMPP server)          │                 │
│  │  • XMPP server written in Lua             │                 │
│  │  • MUC (Multi-User Chat) room management  │                 │
│  │  • Jingle extensions handle SDP offer/ans.│                 │
│  │  • WebSocket/BOSH dual transport          │                 │
│  └────────────────────┬─────────────────────┘                 │
│                       │                                       │
│  ┌────────────────────┴─────────────────────┐                 │
│  │          Jicofo (JItsi COnference FOcus)  │                 │
│  │  ───────────────────────────────────────  │                 │
│  │  • One Jicofo instance per MUC room       │                 │
│  │  • Colibri2 protocol manages JVB endpoints│                 │
│  │  • Pluggable BridgeSelectionStrategy iface│                 │
│  │  • ICE candidate exchange coordination    │                 │
│  │  • Simulcast/SVC source management        │                 │
│  │  • Participant roles & permission control │                 │
│  └────────────────────┬─────────────────────┘                 │
│                       │ Colibri2 (REST over HTTP or XMPP)     │
└───────────────────────┼──────────────────────────────────────┘
                        │
┌───────────────────────┼──────────────────────────────────────┐
│                  Media layer (SFU)                             │
│  ┌────────────────────┴─────────────────────┐                 │
│  │        JVB Pools (multi-region deploy)    │                 │
│  │                                           │                 │
│  │  ┌─────────────┐  ┌─────────────┐        │                 │
│  │  │ JVB Pool A  │  │ JVB Pool B  │        │                 │
│  │  │(Frankfurt)  │  │ (London)    │        │                 │
│  │  │             │  │             │        │                 │
│  │  │ ┌─────────┐ │  │ ┌─────────┐ │        │                 │
│  │  │ │ JVB 1   │ │  │ │ JVB 1   │ │        │                 │
│  │  │ │ ─────── │ │  │ │ ─────── │ │        │                 │
│  │  │ │media:xxx│ │  │ │media:yyy│ │        │                 │
│  │  │ │colibri  │ │  │ │colibri  │ │        │                 │
│  │  │ └─────────┘ │  │ └─────────┘ │        │                 │
│  │  │ ┌─────────┐ │  │ ┌─────────┐ │        │                 │
│  │  │ │ JVB 2   │ │  │ │ JVB 2   │ │        │                 │
│  │  │ └─────────┘ │  │ └─────────┘ │        │                 │
│  │  │ ┌─────────┐ │  │ ┌─────────┐ │        │                 │
│  │  │ │ JVB N   │ │  │ │ JVB N   │ │        │                 │
│  │  │ └─────────┘ │  │ └─────────┘ │        │                 │
│  │  └──────┬──────┘  └──────┬──────┘        │                 │
│  │         │                │                │                 │
│  │         └────────┬───────┘                │                 │
│  │                  │                        │                 │
│  │   Secure Octo (JVB-to-JVB ICE/DTLS mesh)  │                 │
│  │   · Selective forwarding — only streams    │                 │
│  │     subscribers actually need              │                 │
│  │   · Encrypted cascade — replaces the old   │                 │
│  │     VPN approach                           │                 │
│  │   · Source-name filtering — unwanted        │                 │
│  │     streams never cross the bridge         │                 │
│  │                                           │                 │
│  │  ┌──────────────────────────────────────┐ │                 │
│  │  │        JVB internals (Java/Kotlin)    │ │                 │
│  │  │  ───────────────────────────────────  │ │                 │
│  │  │  • Pure RTP forwarding — no decode,   │ │                 │
│  │  │    no transcode                       │ │                 │
│  │  │  • Simulcast 3-layer adaptive select  │ │                 │
│  │  │  • VP9 SVC spatial/temporal layers    │ │                 │
│  │  │  • RTP retransmission buffer (NACK/RTX)│ │                 │
│  │  │  • Bandwidth estimation (REMB+TWCC)   │ │                 │
│  │  │  • Last-N active-speaker policy       │ │                 │
│  │  │  • Adaptive bitrate & quality ctrlrs  │ │                 │
│  │  │  • Single-port ICE — fewer fw rules   │ │                 │
│  │  └──────────────────────────────────────┘ │                 │
│  └──────────────────────────────────────────┘                 │
│                                                                │
│  ┌──────────────────────────────────────────┐                 │
│  │           Auxiliary services              │                 │
│  │  ┌──────────┐  ┌──────────┐  ┌─────────┐ │                 │
│  │  │  Jibri   │  │  Jigasi  │  │ Jitsi   │ │                 │
│  │  │ ──────── │  │ ──────── │  │ SIP GW  │ │                 │
│  │  │ record/  │  │ transcr./│  │         │ │                 │
│  │  │ stream   │  │ PSTN     │  │ SIP→    │ │                 │
│  │  │ Chrome+  │  │ Java     │  │ WebRTC  │ │                 │
│  │  │ ffmpeg   │  │ SIP UA   │  │         │ │                 │
│  │  └──────────┘  └──────────┘  └─────────┘ │                 │
│  └──────────────────────────────────────────┘                 │
└──────────────────────────────────────────────────────────────┘
```

### 2.2 Key Technical Capabilities

| Capability | Details |
|------|------|
| Architecture pattern | Pure SFU (Jitsi Videobridge). Java/Kotlin implementation. Strictly no decoding, no transcoding — RTP-packet selective forwarding only |
| Video codecs | VP8, VP9, H.264; Simulcast 3-layer concurrent encoding; VP9 SVC spatial/temporal layers |
| Transport protocols | WebRTC standards (SRTP/DTLS/SCTP); JVB-to-JVB Secure Octo (ICE/DTLS-SRTP direct cascade) |
| Recording | Jibri: headless Chrome + ffmpeg. Composite recording (grid view) and single-track recording. Can simultaneously push streams to YouTube Live |
| Platform support | Web (Chrome/Firefox/Edge/Safari), iOS SDK, Android SDK, Electron Desktop |
| Region cascading | Pools + Remote Pools tiered architecture. Secure Octo encrypted cascade. Selective forwarding filters out unwanted streams |
| Signaling protocol | XMPP/Prosody + Jicofo + Colibri2. Colibri2 supports both HTTP/JSON and XMPP transports |
| Security | SRTP + DTLS encrypted by default; E2EE (Insertable Streams API, keys managed client-side); SSO/LDAP/JWT authentication |
| Last-N policy | Forwards video of only the N most active speakers. Muted participants consume no video bandwidth at all |
| P2P fallback | Two-person calls go direct P2P (bypassing the JVB). Saves server-side resources |

### 2.3 Tech Stack

**SFU core**:
- Java / Kotlin (Jitsi Videobridge v3). The Kotlin version is progressively replacing the old Java implementation
- Netty-based async I/O framework handling RTP send/receive
- ICE implementation based on ice4j (Java ICE library)
- Memory management relies on JVM GC — GC pauses under heavy concurrency are a known hazard

**Signaling layer**:
- Prosody (written in Lua): XMPP server handling MUC rooms and Jingle SDP
- Jicofo (written in Java): conference focus server, Colibri2 protocol implementation
- Colibri2 protocol: RESTful design, two transport modes — HTTP/JSON and XMPP

**Clients**:
- Web: React 18 + TypeScript. lib-jitsi-meet wraps the WebRTC API
- Mobile: React Native (iOS + Android)
- Desktop: Electron (jitsi-meet-electron)
- SDKs: Jitsi Meet SDK for iOS/Android; IFrame API; Jitsi Meet API

**Auxiliary services**:
- Jibri: Chrome 119+ + ffmpeg 6.x + PulseAudio + Xvfb. Recording and live streaming
- Jigasi: Java SIP User Agent + real-time transcription (STT engine)
- Jitsi SIP Gateway: SIP protocol bridged to WebRTC

**Deployment & operations**:
- Docker Compose: official complete containerization recipe (8 cooperating containers)
- Kubernetes: community Helm Chart
- Configuration management: community Ansible/Terraform recipes
- Monitoring: JVB exposes Prometheus metrics at /colibri/stats
- Build: Maven (Java), Webpack (Web), fastlane (Mobile)

## 3. Feature Overview

### 3.1 Core Feature Modules

| Module | Function | Implementation |
|------|------|---------|
| **Jitsi Meet (Web)** | React SPA video-conferencing UI. The URL is the room number; join with no installation | React 18 + TypeScript + lib-jitsi-meet |
| **Jitsi Videobridge** | Java/Kotlin SFU. 100+ participants per meeting | Netty + ice4j + custom RTP engine |
| **Jicofo** | Conference focus. Manages XMPP MUC rooms, assigns JVB endpoints | Java + Colibri2 REST client |
| **Prosody** | XMPP signaling server. MUC/Jingle/WebSocket/BOSH | Lua + community modules |
| **lib-jitsi-meet** | JavaScript media API. Wraps WebRTC getStats/SDP/device management | TypeScript |
| **Jibri** | Recording/streaming service. Virtual Chrome + ffmpeg | Java + Selenium + ffmpeg |
| **Jigasi** | SIP-PSTN gateway + real-time transcription | Java + SIP stack + STT engine |
| **Etherpad** | Optional collaborative document editing embedded in the meeting | Node.js |

### 3.2 Signature Features

- **Install-free joining**: visit `https://meet.jit.si/roomname` and you're in. WebRTC-based, no software or plugin installation. Jitsi's single most important user-experience advantage
- **Pools + Remote Pools region cascading** (2022 architecture upgrade):
  - JVBs are no longer bound to a specific Shard. Each region maintains its own JVB pool with auto-scaling
  - Remote Pools: cross-region JVB pools connect to other regions' signalling nodes without full interconnection
  - Region Groups: group adjacent regions (e.g. Frankfurt + London) for intra-region proximity, avoiding trans-oceanic cascades
  - Background: redesigned after the pandemic-era fully-connected JVB mesh (50+ shards, 2000+ JVBs) collapsed in 2020
- **Secure Octo encrypted cascade**: JVBs establish encrypted connections via ICE/DTLS-SRTP, replacing the old VPN approach. Selective forwarding — only streams subscribers actually need are transmitted, filtering out unnecessary bandwidth
- **Colibri2 signaling protocol**:
  - A RESTful SFU control protocol — conference endpoints modeled as resources
  - Core operations: POST creates an endpoint, PATCH updates ICE/sources, DELETE destroys, GET queries state
  - Dual transport modes: HTTP/JSON (development/debugging) and XMPP (production)
  - Source management: Simulcast layers, SVC layers, RTP header extensions
- **Pluggable bridge selection strategy**: the Java interface `BridgeSelectionStrategy` — single-bridge mode, region-first, load-balanced, visitor separation, custom policies
- **Browser-side AI features**: background blur (TensorFlow.js, no GPU required); background replacement; noise suppression (RNNoise)
- **In-meeting YouTube sharing**: built-in synchronized YouTube video playback — all participants see the same progress
- **P2P server-free mode**: 1:1 calls go direct P2P (bypassing the JVB), needing only Prosody signaling. Usable in minimal deployments without any JVB
- **IFrame API + Jitsi Meet API**: embeddable conference component for any web page. A complete JavaScript API to control meeting behavior

### 3.3 Extensibility & Plugin Mechanisms

1. **Prosody Lua modules**: authentication extensions (LDAP/JWT/SSO/OAuth2), MUC behavior customization, visitor policies, automatic recording triggers
2. **Jicofo BridgeSelectionStrategy**: customizable JVB selection algorithm. Implement the interface to inject a new scheduling policy
3. **JVB custom plugins**: Java/Kotlin interfaces injecting custom logic into the RTP processing path
4. **IFrame API**: `api.executeCommand('toggleVideo')`, `api.addEventListener('videoConferenceJoined')`, etc.
5. **config.js / interface_config.js**: hundreds of configuration items to customize UI/logo/feature switches/toolbar/default behavior
6. **Jitsi Meet SDK**: iOS/Android SDKs embedded in your own app. React Native SDK
7. **Docker Compose environment variables**: every component's parameters configured via the `.env` file
8. **JaaS (Jitsi as a Service)**: 8x8's API-driven Jitsi cloud service (usage-based pricing)

## 4. Current Status & Ecosystem

### 4.1 Versions & Repositories

| Repository | Stars | Forks | Language | License | Notes |
|------|-------|-------|------|--------|------|
| jitsi/jitsi-meet | 29.6k | 8k | TypeScript | Apache 2.0 | Web client main repository |
| jitsi/jitsi-videobridge | 3.1k | 1k | Kotlin/Java | Apache 2.0 | SFU media server |
| jitsi/docker-jitsi-meet | 3.6k | 1.6k | Lua(Docker) | Apache 2.0 | Docker Compose deployment |
| jitsi/lib-jitsi-meet | 1.4k | 1.2k | TypeScript | Apache 2.0 | JavaScript media API |
| jitsi/jicofo | 700+ | 300+ | Java | Apache 2.0 | Conference focus server |
| jitsi/jigasi | 608 | 340 | Java | Apache 2.0 | SIP gateway + transcription |
| jitsi/jibri | 300+ | 150+ | Java/Kotlin | Apache 2.0 | Recording/streaming service |

- **GitHub Organization**: the jitsi org owns 183 public repositories — the highest repository count of any organization in the open-source video-conferencing category
- **Update frequency**: all core repos are actively updated. docker-jitsi-meet gets daily community feedback and updates
- **Current stable release**: jitsi-meet ships continuous rolling releases. JVB is mid-migration from Java to Kotlin

### 4.2 Community & Documentation

- **Official docs**: `jitsi.org` — self-hosting deployment guides, developer guides, API reference, FAQ
- **Docker Compose**: the most recommended deployment path. Three commands — `git clone && cp env.example .env && docker-compose up -d` — bring it up
- **Kubernetes**: community Helm Chart support (unofficial)
- **Community forum**: Jitsi Community Forum (community.jitsi.org) — the most active exchange channel
- **Matrix/IRC**: real-time chat channels
- **Deployment cases**: numerous European universities (e.g. ETH Zurich, TU Berlin), the French government, multiple German state governments, WHO, and others all run self-hosted Jitsi
- **Chinese localization**: the UI supports Chinese (Simplified/Traditional), but Chinese community resources are limited

### 4.3 Known Deficiencies & Limitations

1. **Java JVM memory overhead**: a single JVB instance's memory footprint is markedly higher than C++ (mediasoup) or Go (LiveKit) implementations. GC pauses can impact real-time behavior at high concurrency
2. **Fully-connected JVB mesh scalability collapse** (2020 historical lesson): with 50+ shards fully interconnected, O(N²) connection counts exploded. Thoroughly resolved in 2022 via the Pools + Remote Pools architecture
3. **Jibri recording depends on the Chrome ecosystem**: needs headless Chrome + Xvfb + PulseAudio + ffmpeg. Complex deployment, high resource consumption, reliability tied to Chrome versions
4. **Heavyweight XMPP stack**: large serialization/deserialization overhead for XML traffic. Configuring Prosody requires Lua and XMPP domain knowledge. Hard to debug (XML unreadability)
5. **No native MCU**: Jitsi is a pure SFU — all processing (decode, render, mix) happens client-side. No server-side codec conversion (e.g. H.264→VP8) or blended mixing
6. **No native AI Agent integration**: compared to LiveKit's Agents Framework, Jitsi has no built-in AI participant framework. AI capability is bolted on via external services (Jigasi transcription)
7. **Heavy build chain**: core components (JVB, Jicofo, Jibri, Jigasi) use Maven. The web client uses Webpack. Build times are long

## 5. Market Positioning

### 5.1 Primary Industries of Application

- **Government & public sector**: data-sovereignty regulations (GDPR/Schrems II) mandate data localization. Jitsi self-hosting is the only fully compliant open-source option
- **Higher education**: universities self-hosting video teaching platforms. Zero license fees + unlimited users + own hardware = very low TCO
- **Healthcare**: patient-privacy data never leaves owned infrastructure. HIPAA-compliant configurations
- **Internal enterprise communications**: law firms, audit firms, financial institutions — organizations with extremely high data-sovereignty demands
- **Nonprofits & open-source communities**: free video-conferencing infrastructure

### 5.2 Competitor Comparison Summary

| Competitor | Strengths | Weaknesses |
|------|------|------|
| mediasoup | C++ extreme performance, fully custom signaling, Worker process isolation | entire signaling layer must be self-built, 2-3 person-months investment; no ready-to-use client |
| LiveKit | Go single-binary minimal deployment, AI Agent first-class citizen, best docs | young project, under-validated at extremes (started 2021); non-standard signaling protocol |
| BigBlueButton | most complete teaching features (whiteboard/breakout/voting/recording playback); 100% open source | FS MCU audio bottleneck; 10+ deb packages make deployment complex; unstable during architecture migration |
| Zoom | fully in-house stack with extreme optimization; large-scale commercial validation; Webinar CDN separation | fully closed source, not self-hostable; non-WebRTC-standard protocols; pricing rising year over year |
| Microsoft Teams | deep M365 integration; flexible SFU/MCU switching; Azure global infrastructure | closed source, not self-hostable; web client feature-limited; Azure ecosystem dependency |
| Google Meet | pure WebRTC standards; progressive AV1 deployment strategy; virtualized media source lowers client complexity | not self-hostable; Chrome/Edge/Firefox only; recording depends on Google Cloud |

### 5.3 Pricing & Licensing

- **Community edition (Apache 2.0)**: 100% free. All core components open source. Unlimited users, unlimited meeting duration
- **8x8 Meet**: commercial SaaS. Built on Jitsi, with 8x8 providing operations and SLA
- **JaaS (Jitsi as a Service)**: API-driven cloud service. Billed by monthly active users. 25,000 MAU free tier
- **Self-hosted TCO**: servers (2-4 cores, 8-16GB RAM sustains hundreds of concurrent sessions), ops headcount, bandwidth. Typical small/medium deployment annual TCO far below Zoom/Teams license fees

## 6. Product Highlights

1. **The only 100% open-source full-stack solution** — from web client to SFU to signaling to recording to PSTN gateway, Jitsi is the only video-conferencing offering whose every component is fully open source. No stitching multiple projects together, no dependency on any closed-source component. For scenarios with strict data-sovereignty and compliance requirements (government, healthcare, law firms), this advantage is irreplaceable.

2. **Colibri2 signaling protocol** — the RESTful SFU control protocol is the most instructive design in Jitsi's architecture. Conference endpoints modeled as resources (create/update/destroy), ICE candidate exchange using trickle-ICE incremental pushes, source management supporting fine-grained control of Simulcast and SVC layers. Dual REST and XMPP transports — cURL + JSON for development/debugging, XMPP persistent connections in production. This "one protocol, two transports" design is something MediaServo's signaling layer should borrow directly.

3. **Pools + Remote Pools cascade architecture** — the 2022 architecture upgrade embodies the ops wisdom of "trip into the pit first, then evolve". During the 2020 pandemic, a 50+ shard fully-connected JVB mesh caused O(N²) connection explosion and a network-wide collapse. The fix: JVB pools grouped by region (star topology), Remote Pools single-link cross-region connections, Region Groups avoiding needless trans-oceanic cascades. This architecture is the "correct answer" for production-scale deployment.

4. **Community size and production validation** — 29.6k Stars (highest in the open-source video-conferencing category), millions of daily active users (pandemic era), a complete ecosystem of 183 GitHub repositories. Deployed by multiple European governments and dozens of universities worldwide. Jitsi is the only open-source video-conferencing solution validated for self-hosting feasibility at the "1M+ DAU" level.

5. **Pluggable bridge selection strategy** — the `BridgeSelectionStrategy` Java interface supports runtime injection of different JVB allocation policies: single bridge (small deployments), region-first (multi-datacenter), load-balanced (large scale), visitor separation (security requirements), custom (special topologies). This granularity of deployment flexibility is the core advantage of open-source solutions over commercial products.

## 7. Reference Value for MediaServo

### [Adopt] Directly Borrowable

1. **Colibri2 RESTful resource modeling**: MediaServo Conference Controller's API design should borrow directly. `POST /conferences` (create meeting + assign SFU), `PATCH /conferences/{id}` (update ICE/sources), `DELETE /conferences/{id}` (destroy), `GET /conferences/{id}/dominant-speaker` (active speaker)
2. **Pools + Remote Pools star topology**: MediaServo's distributed SFU deployment architecture should adopt it directly — partition SFU Worker pools by Region Group, cascade across regions via Remote Pools single links, avoid full interconnection
3. **BridgeSelectionStrategy → trait SfuSelector**: MediaServo should define a Rust trait `SfuSelector` with Local/RegionBased/LoadBalanced implementations
4. **P2P fallback mode**: two-person calls go straight WebRTC P2P (bypassing the SFU). MediaServo's `WebRtcPlugin` should build this in — activate `SfuRelayPlugin` only for 3+ person meetings
5. **Last-N active-speaker policy**: the SFU forwards video only for the N most active speakers. Muted participants consume no video bandwidth at all. MediaServo's `SfuRelayPlugin` should embed this logic
6. **Complete Docker Compose recipe**: MediaServo should offer a Jitsi-like `docker-compose up -d` one-click deployment experience, lowering the onboarding barrier

### [Adapt] Adopt With Modifications

1. **Signaling protocol: XMPP → WebSocket + Protobuf**: the XMPP/Prosody stack is heavyweight. MediaServo should use WebSocket + Protobuf/JSON dual mode (borrow Colibri2's API design but not the XMPP transport). Native clients over Protobuf, web clients over JSON
2. **Recording: Jibri → RTP Forwarding + standalone recording service**: Jibri's Chrome + ffmpeg recipe is too heavy for MediaServo. Use `Plain RTP Transport` (see mediasoup) + a Rust recording service instead, avoiding the Chrome dependency. Two-tier architecture — realtime RTP recording (single-track storage) + offline compositing (on demand)
3. **SFU implementation language: Java/Kotlin → Rust**: JVB's JVM memory overhead is unacceptable. MediaServo's SFU core is Rust — memory safe, no GC pauses, consistent with the native-core stack
4. **Authentication: Lua modules → AuthProvider trait**: Jitsi implements LDAP/JWT/SSO through Prosody Lua modules. MediaServo uses a Rust trait `AuthProvider` with two modes: Local (SQLite+JWT) and AUDEBase (gRPC LDAP)
5. **E2EE key management**: Jitsi achieves it via Insertable Streams + custom key exchange. MediaServo should offer a comparable optional E2EE capability — MediaServoCore provides a trait `KeyExchange`, default implementation using ECDH, replaceable by an enterprise KMS
6. **Native Kubernetes support**: Jitsi has a community Helm Chart. MediaServo should officially provide Helm Chart + Terraform Module, supporting one-click deployment across clouds

### [Avoid] Known Pitfalls / Non-Applicable Scenarios

1. **Avoid the Java/JVM stack**: mediasoup (C++) and LiveKit (Go) already prove the memory-efficiency advantage of non-JVM implementations. MediaServo has chosen Rust — do not introduce Java dependencies
2. **Avoid the XMPP stack**: XMPP is a 1999-era protocol — heavy XML serialization, hard debugging, steep learning curve. WebSocket + gRPC is the standard choice in 2026
3. **Fully-connected SFU mesh = does not scale**: Jitsi's 2020 collapse is the textbook negative case for SFU deployment. MediaServo adopts Pools star topology from Day 1
4. **Jibri Chrome recording = ops nightmare**: Chrome version upgrades, ffmpeg parameter tuning, Xvfb configuration — every link is an ops pain point. mediasoup PlainTransport + a Rust recording service avoids it entirely
5. **Cross-region cascade latency expectations**: even with Pools, inter-continental cascading (e.g. Beijing→Frankfurt) adds 80-150ms latency. MediaServo's UI layer should surface cross-region latency in connection-quality indicators

**Overall rating**: ★★★★☆ (4/5)

> Verdict: Jitsi is the benchmark of open-source video conferencing. The Colibri2 protocol design, the Pools cascade architecture, and the pluggable BridgeSelectionStrategy are all important design references for MediaServo. But the Java/XMPP stack is its historical baggage — MediaServo should take the architectural design essence and discard the technical implementation.

---

> **Sources**
> GitHub: jitsi/jitsi-meet (29.6k Stars, Apache 2.0)
> GitHub: jitsi/jitsi-videobridge (3.1k Stars, Kotlin/Java)
> GitHub: jitsi/docker-jitsi-meet (3.6k Stars)
> Official docs: jitsi.org (self-hosting guides, developer guides, API reference)
> Colibri2 protocol spec: github.com/jitsi/jitsi-videobridge/blob/master/doc/colibri.md
> Jitsi 2020 architecture collapse and recovery: Emil Ivov, "Scaling Jitsi Meet in the Cloud"
> Jitsi Pools architecture design: Boris Grozev, JVB Pools Technical Design (2022)
> Community forum: community.jitsi.org
> MediaServo: docs/research/video-conference.md


---
**Related decisions**: D50, D52, D-SFU-WORKER, Colibri2 (Phase 2 reference)

## Appendix A: Jitsi Docker Compose Complete Deployment

Jitsi's Docker Compose recipe is among the most mature containerized deployments in open-source video conferencing. The core service architecture comprises 6 containers: Nginx (reverse proxy + web statics), Prosody (XMPP signaling), Jicofo (conference focus), JVB (SFU media), Jibri (recording/streaming), Jigasi (PSTN gateway/transcription).

Core deployment steps:
```bash
git clone https://github.com/jitsi/docker-jitsi-meet
cd docker-jitsi-meet
cp env.example .env
# Edit the .env file: configure PUBLIC_URL, TZ, JVB_ADVERTISED_IP, passwords, etc.
./gen-passwords.sh
docker-compose up -d
```

MediaServo should offer a similar Docker Compose one-click deployment experience. Component mapping:
- Jitsi Web ⬄ MediaServo Client (Tauri v2 + web UI)
- Prosody ⬄ MediaServo Signaling (custom WebSocket + JSON/Protobuf signaling)
- Jicofo ⬄ MediaServo Conference Controller (Rust; manages rooms + SFU selection + ICE coordination)
- JVB ⬄ MediaServo SfuRelayPlugin (Rust + mediasoup C++ Worker or str0m)
- Jibri ⬄ MediaServo Recording Plugin (Rust + GStreamer ffmpeg pipeline)
- Jigasi ⬄ MediaServo Telephony Plugin (Rust + SIP stack bridge)

Key Jitsi deployment environment variables:
- `AUTH_TYPE`: internal (local users), jwt (JWT Token), ldap (enterprise LDAP directory)
- `ENABLE_LOBBY`: lobby feature — participants cannot enter the meeting until approved
- `ENABLE_BREAKOUT_ROOMS`: breakout rooms — split the main conference into multiple sub-conferences
- `JVB_BREWERY_MUC`: XMPP MUC room name where JVB pools register (Jicofo discovers available JVBs through this room)
- `BRIDGE_STRESS_THRESHOLD`: JVB load threshold (0.0-1.0). Above it Jicofo stops assigning new meetings to that JVB
- `JVB_OCTO_BIND_ADDRESS`: Secure Octo cascade bind address. Cross-region JVBs establish ICE/DTLS connections through this port

Jitsi scaling guidance:
- **Vertical scaling**: raise the JVB container's CPU/memory quotas. Each JVB handles hundreds of concurrent streams
- **Horizontal scaling**: `docker-compose up -d --scale jvb=3` starts 3 JVB instances. Jicofo auto-discovers new instances via JVB_BREWERY_MUC and begins assigning meetings
- **Cross-region deployment**: deploy an independent Jitsi stack per region. Configure Remote Pools for cross-region cascading
- **Ops note**: JVB uses a UDP port range (default 10000). The firewall must open UDP 10000-100xx

---

## Appendix B: Jitsi's Architecture Evolution From Full-Mesh to Pools

**Phase 1 (2013-2019): static Shard architecture**
```
Shard A                Shard B                Shard C
┌────────┐            ┌────────┐            ┌────────┐
│ JVB 1  │◄──────────►│ JVB 1  │◄──────────►│ JVB 1  │
│ JVB 2  │   Octo     │ JVB 2  │   Octo     │ JVB 2  │
│ JVB N  │ full-mesh  │ JVB N  │ full-mesh  │ JVB N  │
└────────┘            └────────┘            └────────┘
```
Problem: every JVB in every Shard must build Octo connections with every JVB of every other Shard.
Shard count × JVB count explodes connection numbers at O(N²). 50 shards × 40 JVB/shard = 2000 JVBs,
full-mesh connections = C(2000,2) ≈ 2 million ICE/DTLS connections. Collapsed during the 2020 pandemic — the textbook proof that fully-connected topologies do not scale.

**Phase 2 (2022-present): Pools + Remote Pools architecture**
```
Region Group A           Region Group B
┌──────────────────┐     ┌──────────────────┐
│  Pool A          │     │  Pool B          │
│  ┌──┐ ┌──┐ ┌──┐ │     │  ┌──┐ ┌──┐ ┌──┐ │
│  │J1│ │J2│ │JN│ │     │  │J1│ │J2│ │JN│ │
│  └──┘ └──┘ └──┘ │     │  └──┘ └──┘ └──┘ │
│       │          │     │       │          │
│  Signalling Node──┼─────┼──────Signalling  │
│       │   Remote  │     │  Pool │          │
│  ┌──┐ ┌──┐ ┌──┐ │     │  ┌──┐ ┌──┐ ┌──┐ │
│  │R1│ │R2│ │RN│ │     │  │R1│ │R2│ │RN│ │
│  └──┘ └──┘ └──┘ │     │  └──┘ └──┘ └──┘ │
│  Remote Pool     │     │  Remote Pool     │
└──────────────────┘     └──────────────────┘
```
Key improvements:
- Each JVB connects only to its local Signalling Node (no direct connections to other regions' JVBs)
- Cross-region traffic flows through Remote Pool single links between Signalling Nodes
- Connection count drops from O(N²) to O(N) — 40 JVBs need only 40 connections (instead of 780 full-mesh)
- Each Pool scales independently, elastically sized to regional traffic
- Region Groups cluster adjacent regions (Frankfurt + London communicate in-group by proximity, avoiding trans-oceanic cascades)

Core takeaway for MediaServo: adopt Pools star topology from Day 1; never design a fully-interconnected multi-node architecture. Full mesh is fine below 3 nodes, a disaster at 10, and physically impossible at 100.

---

## Appendix C: Colibri2 Protocol Message Examples

Colibri2 is Jitsi's SFU control protocol — the complete lifecycle of conference endpoints managed through a RESTful API:

**1. Create a conference endpoint**
```
POST /colibri/conferences/meeting123
Content-Type: application/json

{
  "id": "meeting123",
  "contents": [{
    "name": "audio",
    "channels": [{"expire": 60, "initiator": true, "endpoint": "abc123"}]
  }, {
    "name": "video",
    "channels": [{"expire": 60, "initiator": true, "endpoint": "abc123"}]
  }]
}
```

**2. Add a participant endpoint**
```
PATCH /colibri/conferences/meeting123
{
  "contents": [{
    "name": "audio",
    "channels": [{"id": "ch1", "expire": 60, "endpoint": "participant2"}]
  }, {
    "name": "video",
    "channels": [{"id": "ch2", "expire": 60, "endpoint": "participant2"}]
  }]
}
```
Every PATCH carries an `expire` field — the JVB automatically cleans up endpoints that fail to renew after timeout. An elegant "soft state" mechanism — default lifetime 60 seconds, the client renews with periodic (e.g. every 30 seconds) PATCHes. After a disconnect the endpoint is cleaned automatically, no explicit DELETE needed.

**3. Simulcast layer management**
```
PATCH /colibri/conferences/meeting123
{
  "channelBundles": [{
    "id": "bundle1",
    "transport": {
      "xmlns": "urn:xmpp:jingle:transports:ice-udp:1",
      "ufrag": "abc123",
      "pwd": "secret",
      "candidates": [...]
    }
  }]
}
```

**4. Query the active speaker**
```
GET /colibri/conferences/meeting123/dominant-speaker
→ {"dominantSpeakerEndpoint": "participant3"}
```

RESTful design points:
- Conference = resource, endpoint = sub-resource (Channel), ICE candidates = nested attributes
- Soft-state renewal = graceful disconnect handling by construction
- Trickle ICE = incremental candidate updates (no need to send all candidates at once)

MediaServo's Conference Controller API should be designed in a similar RESTful style — meetings and participants as resources, managed through CRUD operations. Key difference: MediaServo uses WebSocket (real-time push) + gRPC (service-to-service) dual channels, rather than Colibri2's REST + XMPP dual mode.

---

## Appendix D: Jitsi Security Best Practices

Security checklist for production Jitsi deployments:

1. **Enforce authentication**: `AUTH_TYPE=jwt` or `ldap`. Never use `internal` in production (allows anonymous account creation)
2. **Visitor access control**: configure Prosody `mod_muc_lobby_rooms` — visitors must be approved by a host to enter the meeting
3. **Force TLS 1.2+**: Nginx `ssl_protocols TLSv1.2 TLSv1.3;`
4. **Optionally enable E2EE**: `ENABLE_E2EE=true`. Supports Insertable Streams encryption
5. **STUN/TURN security**: use the `long-term-credential` mechanism rather than `anonymous` mode
6. **Rate limiting**: Prosody `mod_limits` restricts per-IP connection rates
7. **Log scrubbing**: never log PII such as Participant IDs and IP addresses (achievable via Prosody modules)
8. **Regular updates**: pin Docker images to fixed version tags (not `latest`), refresh with periodic `docker pull`

Security takeaways for MediaServo:
- AuthProvider trait mandatory — no skipping authentication to enter a meeting
- TURN service uses time-limited credentials (JWT-token-like) rather than long-lived keys
- Recording-service access control — recording files are equivalent to meeting recordings and need the same level of permission control

### 7.9 [Adopt] Deep Reference: IFrame API Embedding Pattern

Jitsi's IFrame API makes video conferencing embeddable in any web page. MediaServo's web client should offer comparable embedding capability:
- Use Shadow DOM to isolate styles and scripts, avoiding conflicts with the host page
- Provide a complete JavaScript API: join/leave/mute/unmute/shareScreen/setCamera
- Event-driven model: onParticipantJoined/onTrackAdded/onChatMessage
- Adaptive UI layouts: gallery mode, speaker mode, custom layouts

### 7.10 [Adapt] Deep Reference: Bridge Selection Strategy Implementations

Jitsi's pluggable BridgeSelectionStrategy interface is the direct reference for MediaServo's SfuSelector. MediaServo should implement these strategies:
- RegionAwareStrategy (default): selects same-region Workers based on participant and Room region configuration
- LoadBalanceStrategy: selects based on Worker load (participant count, Consumer count, CPU usage)
- LatencyAwareStrategy: selects based on network distance from each participant to each Worker
- AffinityStrategy: participants of the same Room preferentially assigned to the same Worker
- Strategies switchable dynamically at runtime, no service restart required

### 7.11 [Avoid] Deep Reference: Java GC vs Rust Zero-Cost Abstractions

Jitsi's Java implementation exposed GC-pause problems at large deployment scale — an important argument for MediaServo choosing Rust:
- Java JVB suffers 200-500ms Full GC pauses at 50+ participants, buffering RTP packets in the UDP socket
- Rust's zero-GC property guarantees latency consistency (latency jitter <5ms)
- Rust's ownership model avoids object-allocation-frequency problems
- Rust's zero-cost abstractions yield 30-50% higher CPU efficiency than Java in high-concurrency scenarios

### 7.12 [Adopt] Deep Reference: Last-N Active-Speaker Policy

Jitsi's Last-N policy is the SFU's core bandwidth-saving mechanism. MediaServo's SfuRelayPlugin should build it in:
- Default Last-N = 6 (show at most 6 videos simultaneously)
- Dynamic Last-N: adaptively tuned to participant device capability
- Smooth video transition on speaker switch (pre-warm at low resolution first, then switch to high resolution)
- Muted participants' video auto-paused (combined with the Dynacast idea)
