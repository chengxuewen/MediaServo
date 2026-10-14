# RustDesk Reference Analysis
> Generated: 2026-07-16 | Category: Remote Desktop

## 1. Product Profile
- **Name**: RustDesk
- **Developer**: the RustDesk open-source community (led by a Chinese team, 400+ contributors); the commercial entity is Purslane Ltd
- **First release**: 2020
- **Positioning**: an open-source self-hosted TeamViewer alternative whose core idea is "data sovereignty belongs to the user"
- **Target users**: privacy-conscious individuals, SMB IT operations, educational institutions, organizations needing self-hosted remote solutions, and government/enterprise bodies wary of closed-source remote software
- **License / business model**:
  - Open-source edition (client + OSS server): AGPLv3
  - RustDesk Server Pro: commercial license (separate from AGPL)
  - Revenue model: Pro Server subscriptions by concurrent connections or device count + enterprise support

## 2. Technical Characteristics
### Overall Architecture
```
┌──────────────────────────────────────────────────────────────────┐
│                 The RustDesk ecosystem — architecture overview   │
│                                                                  │
│  ┌─────────────────────┐      ┌─────────────────────┐            │
│  │   HBBS service       │      │   HBBR service      │            │
│  │   (Rendezvous        │      │   (Relay Server)    │            │
│  │    Server)           │      │                     │            │
│  │                      │      │                     │            │
│  │  · device reg/       │      │  · data relay       │            │
│  │    discovery         │      │  · cannot decrypt   │            │
│  │  · presence          │      │  · TCP/UDP relay    │            │
│  │  · NAT-type          │      │                     │            │
│  │    classification    │      │                     │            │
│  │  · connection        │      │                     │            │
│  │    negotiation aid   │      │                     │            │
│  │  · ID allocation     │      │                     │            │
│  │    & management      │      │                     │            │
│  └──────────┬──────────┘      └──────────┬───────────┘            │
│             │                            │                        │
│             └──────────┬─────────────────┘                        │
│                        │                                          │
│  ┌─────────────────────┴──────────────────────────────────────┐  │
│  │              RustDesk Client (single binary)               │  │
│  │                                                             │  │
│  │  ┌──────────────────────────────────────────────────────┐  │  │
│  │  │  parity-tokio-ipc (the inter-process bus)           │  │  │
│  │  │  · Windows: Named Pipe                               │  │  │
│  │  │  · Unix: Unix Domain Socket                          │  │  │
│  │  │  · Message type: Rust enum `ipc::Data`                │  │  │
│  │  └────┬──────────────┬──────────────┬────────────────────┘  │  │
│  │       │              │              │                        │  │
│  │  ┌────┴──────┐ ┌────┴──────┐ ┌─────┴──────┐                 │  │
│  │  │  Server   │ │    CM     │ │   Main     │                 │  │
│  │  │  process  │ │  process  │ │  process   │                 │  │
│  │  │           │ │           │ │            │                 │  │
│  │  │ · back-   │ │ · conn    │ │ · Flutter  │                 │  │
│  │  │   ground  │ │   mgmt UI │ │   GUI      │                 │  │
│  │  │ · system  │ │ · session │ │ · settings │                 │  │
│  │  │   tray    │ │   UI      │ │ · address  │                 │  │
│  │  │ · remote  │ │ · auth    │ │   book     │                 │  │
│  │  │   conn    │ │   dialogs │ │ · user     │                 │  │
│  │  │ · listen/ │ │ · conn    │ │   identity │                 │  │
│  │  │ · SYSTEM  │ │   state   │ │            │                 │  │
│  │  │           │ │ · user    │ │            │                 │  │
│  │  │           │ │   identity│ │            │                 │  │
│  │  └───────────┘ └───────────┘ └────────────┘                 │  │
│  └─────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  Core libraries (libs/):                                         │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────────────────┐  │
│  │ libs/scrap   │ │libs/hbb_common│ │ libs/clipboard           │  │
│  │ cross-plat   │ │ Protobuf     │ │ cross-platform           │  │
│  │ screen       │ │ protocol     │ │ clipboard                │  │
│  │ capture      │ │ · message    │ │ · text/images            │  │
│  │ · DXGI       │ │   defs       │ │ · file drag & drop       │  │
│  │ · X11/       │ │ · codecs     │ │                          │  │
│  │   KMS/DRM    │ │ · codec      │ │                          │  │
│  │ · Core-      │ │ · config/log │ │                          │  │
│  │   Graphics   │ │ · key        │ │                          │  │
│  │ · PipeWire   │ │   exchange   │ │                          │  │
│  └──────────────┘ └──────────────┘ └──────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### Connection Establishment Flow
```
Controller (Client)                 HBBS (signaling)            Controlled (Host)
    │                               │                            │
    │── 1. TCP conn + ID register ─►│◄── 1. TCP conn + ID register│
    │── 2. heartbeat ──────────────►│◄── 2. heartbeat ───────────│
    │                               │                            │
    │── 3. query target ID presence►│                            │
    │◄── 4. reply: online + net info│                            │
    │                               │                            │
    │── 5. TCP hole-punch request ─────────────────────────────────►│
    │                               │                            │
    │   [punch succeeded]           │                            │
    │◄══════ NaCl E2E-encrypted P2P ═════════════════════════════►│
    │                               │                            │
    │   [punch failed]              │                            │
    │── 6. request HBBR relay ─────►│                            │
    │◄═══ HBBR relay (NaCl) ═══════►│◄══ HBBR relay (NaCl) ═════│
```

### Key Technical Capabilities
| Capability | Details |
|------------|---------|
| Video encoding | Software: VP8 / VP9 / AV1 (all royalty-free); hardware: H.264 / H.265 (NVENC / VAAPI / MediaCodec / VideoToolbox / AMF / QSV) |
| Encoding policy | Hardware acceleration first → software codec fallback; encoder priority and enable lists configured via Cargo feature flags |
| Transport protocol | TCP hole punching (P2P direct preferred) + TCP/UDP relay fallback (HBBR); Protobuf defines all network messages |
| End-to-end encryption | NaCl (libsodium): curve25519 key exchange + xsalsa20-poly1305 stream cipher; relay servers cannot decrypt |
| Identity & auth | one-time password (OTP) + public-key auth; password protection and key-file binding; Pro Server adds LDAP/OIDC |
| Platform matrix | Windows 7+ / macOS 10.12+ / Linux (X11 + Wayland) / iOS / Android / Web |
| Deployment | single install-free binary (Windows) / system-service install (Linux) / one-command Docker server deployment |
| Permission control | independent toggles across four dimensions: keyboard input, clipboard read/write, audio transport, file transfer |

### Process Roles in Detail
| Role | Startup flag | Runs as | Core responsibility |
|------|-------------|---------|---------------------|
| Server | `--server` | SYSTEM / root | listens for remote connections, manages connection lifecycle, system tray icon, service registration |
| CM | `--cm` | user identity | connection-management UI, authorization dialogs, password input, session state |
| CM-No-UI | `--cm-no-ui` | user identity | headless connection management (Linux servers/embedded devices), pure protocol interaction |
| Main | (no flag) | user identity | the Flutter GUI, settings pages, address-book management, initiating connections |

### Screen-Capture Backend Matrix
| Capture method | Windows | Linux | macOS |
|----------------|---------|-------|-------|
| DXGI Desktop Duplication | ✅ | ➖ | ➖ |
| X11 SHM | ➖ | ✅ | ➖ |
| KMS/DRM | ➖ | ✅ | ➖ |
| PipeWire | ➖ | ✅ | ➖ |
| CoreGraphics | ➖ | ➖ | ✅ |
| ScreenCaptureKit (macOS 13+) | ➖ | ➖ | ✅ |

### Technology Stack
- **Core language**: Rust (67.6%), leveraging the ownership system for memory- and thread-safety
- **Cross-platform UI**: Dart / Flutter (24.1%), one codebase across 6 platforms
- **Platform-adaptation layer**: C++ / C wrapping native APIs (screen capture, encoders, input injection)
- **IPC**: parity-tokio-ipc (a unified abstraction over Windows named pipes + Unix domain sockets)
- **Protocol definition**: Protobuf (messages in `libs/hbb_common/src/protos/`)
- **Crypto**: libsodium (a NaCl descendant providing curve25519 + xsalsa20-poly1305)
- **Async runtime**: tokio (the Rust async standard)
- **Screen capture**: `libs/scrap` (trait abstraction + per-platform implementations)
- **Video-render supplement**: flutter_gpu_texture_renderer (direct GPU-texture rendering)
- **Input simulation**: cross-platform keyboard/mouse injection (Windows `SendInput`/Linux `libxdo`/macOS `CGEvent`)

### Codec Capability Matrix
| Codec | Type | Royalty-free | Platform support | Notes |
|-------|------|--------------|------------------|-------|
| VP8 | software | ✅ | all platforms | the lowest-common encoder, best compatibility |
| VP9 | software | ✅ | all platforms | ~30% better compression than VP8 |
| AV1 | software | ✅ | all platforms | the newest generation, highest compression |
| H.264 NVENC | hardware | ❌ | Windows/Linux (NVIDIA) | lowest hardware encode latency |
| H.264 VAAPI | hardware | ❌ | Linux (Intel/AMD) | the standard Linux hardware encoder |
| H.264 VideoToolbox | hardware | ❌ | macOS | Apple's hardware encoder |
| H.264 MediaCodec | hardware | ❌ | Android | Android's hardware encoder |
| H.265 NVENC | hardware | ❌ | Windows/Linux (NVIDIA) | ~30% bandwidth savings over H.264 |

## 3. Feature Overview
### Core Feature Modules
- **Remote desktop control**: a complete remote-desktop experience with multi-monitor switching (separate windows or merged display), adaptive resolution scaling, fullscreen mode, smoothness-first/quality-first mode switching, true-color (32-bit) support, and adaptive network bandwidth
- **File transfer**: bidirectional, directory-level transfers with resume, live rate/progress display, drag-and-drop sending, and queue management
- **TCP tunneling (port forwarding)**: maps any remote TCP port locally, proxying intranet services (HTTP/SSH/RDP/databases); simultaneous multi-port forwarding; bidirectional local→remote and remote→local tunnels
- **Terminal access**: a remote command-line terminal (SSH-like) with no desktop environment — suited to headless Linux servers or embedded devices
- **Remote camera viewing**: live view of the controlled end's cameras (USB/built-in) — for remote monitoring and environment confirmation
- **Clipboard sync**: bidirectional sync of text, RTF rich text, and images (PNG/BMP)
- **Audio transport**: real-time system audio from the remote host to the controller

### Distinctive Features
- **Self-hosted infrastructure**: HBBS (the rendezvous/signaling server) + HBBR (the relay server) deploy with one Docker Compose command. Users fully control ID allocation and data routing without depending on any official RustDesk server
- **Built-in address book**: remote devices organized by group, with aliases, live presence, and quick connect; JSON/YAML import/export for cross-device sync and team sharing; favorites and recent-connection history
- **Single-binary portable mode**: on Windows a single `.exe` runs with no registry writes and no install residue — carry it on a USB stick, ideal for ad-hoc support. An installed edition (system service, autostart, auto-update) is also offered
- **Privacy screen**: blacks out the controlled display during a session so no one nearby can watch — suited to sensitive operations and compliance. Also supports remotely locking the input devices (keyboard/mouse)
- **Unattended access**: permanent passwords or keypair authentication, configured as a system service with autostart — for servers, industrial PCs, POS terminals. Access can be limited by IP allowlist
- **Multilingual i18n**: 30+ community-contributed translations covering the UI, docs, and website; the Chinese translation is maintained by the core team
- **Custom ID server**: enterprises can run their own HBBS inside the intranet so device IDs stay fully internal, enabling air-gapped deployment. Custom ID prefixes supported (e.g. `custom@server`)
- **Session recording**: Pro Server records and replays whole remote sessions for compliance audits
- **Monitoring wall**: Pro Server batch-monitors live screens of many devices (like a security video wall)

### Extensibility / Plugin Mechanisms
- **The relay server interface is public**: rustdesk-server-demo provides a reference implementation; third parties can build compatible rendezvous/relay servers in any language
- **A screen-capture abstraction layer**: `libs/scrap` defines the unified `Capturer` trait; new platforms only implement the trait
- **Pluggable encoders**: Cargo feature flags enable/disable specific hardware encoders, deciding the codec capability matrix at compile time; the `vram` flag enables GPU-memory optimizations
- **Web client**: browser-based remote control via WebRTC, working in nearly all modern browsers
- **REST API** (Pro): batch device management, statistics reports, LDAP integration, OIDC/OAuth2 SSO
- **White labeling** (Pro): the client and UI customizable with enterprise branding (logo, theme colors, installer name)
- **Custom plugin system** (Pro): Python script plugins for automated ops tasks

- **The web-client architecture (Pro)**: WebRTC-based, reusing one port for signaling and data. The web client uses VP8/VP9 (natively supported by browsers), a different protocol stack from the native client's H.264/H.265 — the source of this dual-stack technical debt
- **Batch deployment tooling (Pro)**: MSI install + GPO policy + silent configuration (preset password, designated ID server)
- **Health checks & monitoring (Pro)**: real-time device online/offline monitoring, CPU/memory/disk usage collection

### Key RustDesk Version Milestones
| Version | Date | Key updates |
|---------|------|-------------|
| v0.x | 2020 | the first proof of concept: TCP-hole-punch P2P + basic remote desktop |
| v1.0 | 2021 | the first stable release, introducing the Flutter UI (replacing the initial native UI) |
| v1.1 | 2021-2022 | file transfer, TCP tunneling (port forwarding), terminal access, the Android client |
| v1.2 | 2022-2023 | VP9 software encoding, experimental AV1, the iOS client, early Wayland support |
| v1.3 | 2024 | macOS ScreenCaptureKit, PipeWire audio capture, experimental Vulkan Video encoding |
| v1.4 | 2024-2025 | flutter_gpu_texture_renderer for 4K rendering, the vram feature flag, the web client (WebRTC) |
| v1.4.9 | 2026-07 | the latest stable release: Wayland-compatibility fixes, more robust Android background services |

### GitHub Repository Activity Analysis
| Repo | Stars | Forks | Contributors | Open Issues | Last Commit |
|------|-------|-------|--------------|-------------|-------------|
| rustdesk/rustdesk | 118,000+ | 10,000+ | 400+ | ~1,200 | daily |
| rustdesk/rustdesk-server-demo | 7,500+ | 2,200+ | 30+ | ~80 | weekly |
| rustdesk/rustdesk-server-pro | private | - | internal | - | continuous |
| community forks & plugin repos | many | - | - | - | - |

Technology-stack evolution trends:
- The Rust share rose from 60% to 67.6% (core logic continuing to consolidate into Rust)
- Flutter's share stabilized at ~24% (the cross-platform UI reached steady state)
- C/C++'s share shrank (more platform adaptation calls system APIs directly through Rust FFI)

## 4. Status & Ecosystem
- **Current version**: v1.4.9 (released 2026-07-06), actively iterating with stable monthly releases
- **GitHub stars / activity**: 118,000+ stars; 400+ contributors; multiple daily commits; 10,000+ forks; 39 releases; issue responses typically within 1-3 days
- **Community size**:
  - Discord: multilingual active channels with developers resident and healthy peer help
  - Reddit (r/rustdesk): usage stories, self-hosting tutorials, and support threads
  - Twitter (@rustdesk): release announcements and community news
  - YouTube: video tutorials and feature demos
- **Docs / SDK / API ecosystem**:
  - The official doc site: doc.rustdesk.com (multilingual, community-maintained)
  - Thorough self-hosting guides (Docker/docker-compose/bare metal/Kubernetes)
  - The protocol is public (Protobuf); third parties can independently implement clients
  - Pro Server ships an admin REST API with documentation
  - FAQ and troubleshooting guides
- **Known defects or limitations**:
  1. **Limited TCP punch success rates**: under symmetric NAT and multi-layer carrier NAT (e.g. Chinese mobile 4G / Unicom broadband), TCP hole punching succeeds less often, leaning heavily on the HBBR relay. The official docs acknowledge this
  2. **Flutter 4K rendering overhead**: at 4K the GPU-texture handoff between the Flutter GUI and the encode layer needs `flutter_gpu_texture_renderer` and the `vram` feature flag, otherwise there is visible overhead
  3. **The AGPLv3 compliance bar**: AGPLv3 requires network-use derivatives to stay open (the SaaS scenario), a significant obstacle to commercial integration
  4. **iOS background limits**: iOS restricts long-running background execution, so the remote-access experience is incomplete
  5. **The web/native dual stack**: the web client is WebRTC-based and its protocol doesn't fully match the native client, so feature coverage differs
  6. **Audio cross-platform consistency**: capture/playback solutions differ substantially per platform, so experiences vary

  7. **Official public server capacity limits**: the free public HBBS/HBBR servers show clear bottlenecks at peak (slow connection setup, throttled relay bandwidth). The official statement is that free servers are unsuited for production
  8. **Wayland compatibility still evolving**: Linux Wayland support rides modern APIs (PipeWire + XDG Desktop Portal) but behaves inconsistently across compositors (KDE KWin vs wlroots vs GNOME Mutter)
  9. **Opaque Pro Server pricing**: Pro pricing requires contacting sales — no public page — a adoption barrier for small teams and startups

## 5. Market Positioning
- **Primary industries**:
  - IT operations and support (the free TeamViewer/AnyDesk replacement)
  - Remote work (SMB data sovereignty, self-hostable)
  - Online education (remote demos and tutoring, low-bandwidth needs)
  - Personal/home remote access (NAS admin, home PCs)
  - Government and military (data stays onshore, fully self-hosted, auditable)
  - Embedded/IoT (remote management and monitoring of Linux embedded devices)
- **Competitor quick comparison**:

| Competitor | Strengths | Weaknesses |
|------------|-----------|------------|
| TeamViewer | 300M+ MAU devices, deep enterprise penetration, 1000+ global relay nodes | closed source, centralized, thousands of dollars/year, trust damaged by the 2016 incident |
| AnyDesk | DeskRT codec <16ms, a 3.7MB client, an Erlang carrier-grade backend | closed source, unsigned updates (security risk), scarce Erlang talent |
| Parsec | 7ms ultra-low latency, a GPU zero-copy pipeline, the gaming/creative benchmark | hardware encoding only (old devices excluded), a closed BUD protocol, Windows-leaning |
| ToDesk | outstanding domestic-network optimization (SD-WAN+RTC), weak-network loss tolerance <30% | closed source, opaque core algorithms, implicit QoS limits on the free tier |

- **Pricing / licensing**:
  - **Open-source edition (AGPLv3)**: entirely free, all basic remote-desktop features, self-hosted or the official free public servers
  - **RustDesk Server Pro**: subscription by concurrent connections or managed devices
    - LDAP/AD integration, white labeling, batch deployment, audit logs
    - OIDC/OAuth2 SSO, session recording, a Python plugin system
    - Priority support, SLA guarantees
    - Pricing by contacting sales
  - **Official public servers**: free with performance and bandwidth ceilings — suited to individuals and small teams

## 6. Product Highlights
1. **A technology stack closely aligned with MediaServo**: RustDesk is the only production-grade remote-desktop project written in Rust. Its Rust + Protobuf + cross-platform screen-capture abstraction matches MediaServo's design direction. Its engineering proves Rust is viable and efficient for remote desktop (network I/O, codec bridging, cross-platform system calls)
2. **The elegant single-binary multi-process design**: one binary assumes 4 process roles via command-line flags, with IPC between processes. The design resolves both privilege isolation (Server as SYSTEM for network I/O) and deployment simplification (ship one file). Directly relevant to MediaServo's Client/Host dual-app architecture
3. **The self-hosted data-sovereignty model**: the HBBS (signaling) + HBBR (relay) split cleanly separates control and data planes. One-command Docker deployment lowers the self-hosting barrier. All network data is NaCl end-to-end encrypted — relays cannot read content. An ideal reference for MediaServo's standalone + platform-delegated dual modes
4. **The Rust abstraction of cross-platform screen capture**: `libs/scrap` unifies Windows DXGI Desktop Duplication, Linux X11 SHM/KMS-DRM, and macOS CoreGraphics/ScreenCaptureKit behind the `Capturer` trait. A simple interface (`capture()` returns a `Frame`), freely implementable by third parties. The best-practice template for MediaServo's ScreenCapture plugin
5. **Protobuf-driven version evolution**: all network messages are Protobuf-defined, giving compile-time type checks and forward/backward runtime compatibility — vital in open-source collaboration where hundreds of contributors add fields without breaking compatibility. MediaServo also uses Protobuf; RustDesk's proto organization is worth emulating

## 7. Reference Value for MediaServo

### [Adopt] Directly Borrowable
- The cross-platform screen-capture trait abstraction design and interface shape of `libs/scrap`
- The organization of Protobuf-over-all-protocol-messages (message-type enums, oneof polymorphic messages, field-number management strategy)
- The HBBS/HBBR separation of signaling and relay responsibilities — natural control/data plane isolation
- NaCl (libsodium) end-to-end encryption: the key-exchange and stream-cipher scheme
- Process roles switched by command-line argument + IPC communication for multi-form deployment from one codebase
- The per-platform strategy of the screen-capture backend matrix (trait + conditional compilation)

### [Adapt] Needs Modification Before Adoption
- Single-binary multi-process → MediaServo's microkernel + dynamic plugin loading (compile-time feature flags → runtime dlopen/dynamic registration)
- Four-dimension permission toggles → full RBAC + session quotas (duration/resolution/bitrate caps) + feature-license validation
- Cargo feature-flag encoder selection → a runtime plugin registry supporting hot load/unload of encoder plugins
- Flutter UI → Tauri/Electron (desktop) + Flutter (mobile optional, reusing the community ecosystem)
- TCP hole punching → the ICE/STUN/TURN standard protocol family (the WebRTC stack, aligned with the Parsec/CRD route)

### [Avoid] Known Pitfalls / Not Applicable
- **Low TCP-punch success**: never hand-roll hole punching in MediaServo; adopt the ICE/STUN/TURN standard family
- **The AGPLv3 license boundary**: AGPLv3 code cannot merge into an Apache 2.0 project. Borrow architecture ideas only, with strict code audits against license contamination
- **Flutter 4K rendering overhead**: MediaServo's render path should use native GPU APIs directly (DirectX/Metal/Vulkan), avoiding texture copies between a GUI framework and the GPU
- **The web/native dual stack**: unify the signaling and data-plane protocols from the design stage — web and native on one WebRTC stack

- **Official public servers carry no SLA**: free infrastructure bottlenecks at peak. If MediaServo offers free public relays, the terms of service must state the SLA boundary
- **iOS background limits**: iOS grants no long-running background execution to third-party apps, so remote access is incomplete. If MediaServo needs iOS remote desktop, state this constraint in requirements analysis

### RustDesk's Reference for MediaServo's Code Organization

RustDesk's repo organization directly informs MediaServo:
```
A MediaServo monorepo modeled on RustDesk's structure:
mediaservo/
├── crates/
│   ├── core/              # the microkernel (like RustDesk's src/laminar)
│   ├── screen-capture/    # the cross-platform capture trait (like libs/scrap)
│   ├── protocol/          # Protobuf message defs (like libs/hbb_common/src/protos)
│   ├── crypto/            # the crypto module (like secure_connection)
│   ├── input/             # cross-platform input injection (like libs/enigo)
│   ├── codec/             # the codec factory (like RustDesk's encoder mgmt)
│   ├── transport/         # the transport abstraction (like socket_helpers)
│   └── clipboard/         # cross-platform clipboard (like libs/clipboard)
├── plugins/               # MediaServo-specific: dynamically loaded plugins
├── apps/                  # application entry points
│   ├── client/            # the Tauri/Electron client
│   └── host/              # the headless daemon
└── bindings/              # napi-rs / C FFI bindings
```

RustDesk's crate granularity (~8 core crates) is a good benchmark — not too few (single oversized crates), not more than ~15 (avoiding compile-time and dependency-management blow-ups).

**Overall rating**: ★★★★★ (5/5)
**Comment**: the reference project whose stack sits closest to MediaServo. It provides irreplaceable first-hand experience in Rust remote-desktop engineering, self-hosted architecture, and cross-platform screen-capture abstraction — the primary technical reference for Phase 0 architecture design.
<!-- -->
**Related decisions**: D3, D51, D52, D81

## Appendix: RustDesk Key Files & Code Structure

```
rustdesk/
├── src/
│   ├── main.rs              # the entry: parses args to pick the process role
│   ├── server/              # the Server process: background service, connection listening
│   │   ├── connection.rs    # remote-connection handling
│   │   └── video_service.rs # the video-stream service
│   ├── ui/                  # the Flutter GUI integration
│   │   └── session.rs       # session-management UI
│   └── platform/            # platform-specific code
│       ├── windows.rs       # Windows specifics (named-pipe IPC)
│       └── linux.rs         # Linux specifics (Unix-socket IPC)
├── libs/
│   ├── scrap/               # cross-platform screen capture
│   │   ├── src/
│   │   │   ├── dxgi.rs      # Windows DXGI Desktop Duplication
│   │   │   ├── x11.rs       # Linux X11 SHM
│   │   │   ├── wayland.rs   # Linux PipeWire/Wayland
│   │   │   └── mod.rs       # the Capturer trait definition
│   ├── hbb_common/          # shared protocol and utilities
│   │   ├── src/
│   │   │   ├── protos/      # Protobuf message definitions
│   │   │   ├── config.rs    # configuration management
│   │   │   └── socket_helpers.rs # networking helpers
│   ├── clipboard/           # cross-platform clipboard
│   └── enigo/               # cross-platform input simulation
├── flutter/                 # Flutter UI code
│   ├── lib/
│   │   ├── pages/           # pages
│   │   └── models/          # data models
└── res/                     # resource files
```

## Appendix: RustDesk Server Pro vs OSS Server

| Feature | OSS Server | Pro Server |
|---------|------------|------------|
| Device registration/discovery (HBBS) | supported | supported |
| Data relay (HBBR) | supported | supported |
| Docker deployment | supported | supported |
| Web admin UI | not supported | supported |
| LDAP/AD integration | not supported | supported |
| OIDC/OAuth2 SSO | not supported | supported |
| Batch device management | not supported | supported |
| Session recording & replay | not supported | supported |
| White-label branding | not supported | supported |
| Audit logs | not supported | supported |
| Usage statistics reports | not supported | supported |
| Python plugin system | not supported | supported |
| Priority support | not supported | supported |

## Appendix: RustDesk's Influence on MediaServo's Architecture Decisions

As the closest-stack reference project, RustDesk's influence spans these key areas:
1. Protocol layer: Protobuf's evolution friendliness is validated by RustDesk's 400+ contributor collaboration
2. Abstraction layer: trait-based cross-platform screen capture is the right direction (validated by the scrap library)
3. Deployment layer: single binary multi-process → MediaServo evolves toward microkernel + plugins
4. Security layer: NaCl E2E → MediaServo can upgrade to a DTLS-SRTP + NaCl hybrid
5. UI layer: Flutter's cross-platform reach → MediaServo weighs a Tauri-desktop + Flutter-mobile hybrid

6. Release layer: RustDesk's release engineering (39 releases of versioning experience) informs MediaServo's CI/CD pipeline design

## Appendix: RustDesk's Native-Dependency Minimization Analysis

RustDesk keeps its core dependency tree rigorously minimal — engineering discipline MediaServo should adopt:
```
Core direct dependencies (key Cargo.toml entries):
├── tokio (async runtime) — remote desktop is I/O-bound; tokio is the Rust standard
├── protobuf (serialization) — protocol definition, replacing JSON/MessagePack
├── libsodium (crypto) — the NaCl implementation: curve25519 + xsalsa20-poly1305
├── scrap (screen capture) — in-house, no third-party capture library
├── enigo (input simulation) — the Rust-native cross-platform input library
├── hbb_common (shared utilities) — in-house protocol + config + logging + net tools
└── parity-tokio-ipc (IPC) — lightweight inter-process communication

Total: core functionality in ~10 direct Rust dependencies (excluding transitive)
Contrast: an Electron+WebRTC stack carries ~50-80 direct dependencies

Takeaway: MediaServo's core crates should follow the same discipline — every dependency
needs a clear, irreplaceable justification
```

## Appendix: RustDesk Signaling Protocol Message-Type Analysis

RustDesk's Protobuf message organization informs MediaServo's protocol design:
```protobuf
// The RustDesk message-structure pattern (simplified, based on public proto files)
message Message {
  oneof message {
    RegisterPeer register_peer = 1;      // device registration
    LoginRequest login_request = 2;       // login/authentication
    PunchHoleRequest punch_hole = 5;      // hole-punch information exchange
    TestNatRequest test_nat = 7;          // NAT type probing
    VideoFrame video_frame = 10;          // video frame
    AudioFrame audio_frame = 11;          // audio frame
    Clipboard clipboard = 15;             // clipboard sync
    FileTransfer file_transfer = 20;      // file transfer
    PortForward port_forward = 25;        // port forwarding
    Heartbeat heartbeat = 30;             // keep-alive
  }
}
```
Key design decisions:
- The oneof polymorphic message: all message types on one connection share an outer container → multiplexing over a single TCP connection
- Messages numbered by functional module (1-10 auth, 10-20 media, 20-30 extensions) → a field-number management strategy
- MediaServo should adopt a similar oneof pattern + modular field-number ranges

## Appendix: RustDesk vs MediaServo Stack Alignment

RustDesk is the closest reference to MediaServo's stack; key alignments:

| Dimension | RustDesk today | MediaServo plan | Alignment |
|-----------|----------------|-----------------|-----------|
| Core language | Rust (67.6%) | Rust (100% of core) | fully aligned |
| Cross-platform UI | Flutter (24.1%) | Tauri/Electron + Flutter mobile | highly aligned |
| Protocol definition | Protobuf | Protobuf | fully aligned |
| Signaling service | HBBS (Rust) | the signaling plugin (Rust) | fully aligned |
| Relay service | HBBR (Rust) | TURN/in-house relay (Rust) | fully aligned |
| Screen capture | libs/scrap (trait) | the ScreenCapture plugin (trait) | fully aligned |
| Crypto scheme | NaCl (libsodium) | DTLS-SRTP + NaCl hybrid | highly aligned |
| Encoder management | Cargo feature flags | runtime plugin registration | partially aligned |
| Deployment model | single binary multi-role | microkernel + dynamically loaded plugins | partially aligned |
| NAT traversal | TCP hole punching | ICE/STUN/TURN | converging |
| License | AGPLv3 | Apache 2.0 | incompatible |

Key divergence analysis:
1. Encoder management: compile-time feature flags vs runtime dynamic loading (more flexible, more complex)
2. Deployment model: single binary vs microkernel + plugins (more modular, higher load overhead)
3. NAT traversal: in-house TCP punching vs standard ICE (more standard, but possibly weaker than in-house on some networks)
4. License: AGPLv3 vs Apache 2.0 (no code reuse; independent implementation required)

## Appendix: Lessons from RustDesk's Engineering Decisions
1. The Protobuf oneof polymorphic message pattern is MediaServo's first choice for protocol design (validated by 400+ contributors)
2. Trait abstraction + conditional compilation is the right direction for cross-platform screen capture (validated by scrap)
3. ~8 core crates is a good split granularity (neither too fine nor too fat)
4. Single-binary multi-role deployment simplification is clever engineering
5. Open-source + Pro Server dual licensing is a sustainable open-source business model

### RustDesk's Technical Debt (for MediaServo to Avoid)
1. Limited TCP-punch success under symmetric/multi-layer NAT → MediaServo uses ICE/STUN/TURN
2. Flutter 4K rendering overhead on GPU-texture handoff → MediaServo renders via native GPU APIs
3. AGPLv3 licensing bars commercial integration → MediaServo uses Apache 2.0
4. iOS background limits degrade remote access → state this at requirements stage
5. The web/native dual stack yields feature gaps → MediaServo unifies on one WebRTC stack
6. Official public servers lack SLA guarantees
7. Wayland compatibility still evolving, inconsistent across compositors
8. Opaque Pro Server pricing, no public page
9. Cross-platform audio inconsistency across capture/playback approaches

### Summary: RustDesk's Core Value to MediaServo
RustDesk is the closest-stack reference for MediaServo — Rust + Protobuf + self-hosting + the cross-platform screen-capture trait. Its engineering proves:
- Rust is viable and efficient for remote desktop (network I/O, codec bridging, cross-platform system calls)
- Protobuf's evolution friendliness is validated by 400+ contributor collaboration
- The single-binary multi-process model is an elegant balance between deployment simplification and privilege isolation
- The self-hosted data-sovereignty model has clear market demand
- ~8 core crates is a reference granularity for MediaServo's workspace organization

RustDesk's AGPLv3 means MediaServo cannot reuse its code but may borrow its architecture ideas and engineering practice (independently implemented under Apache 2.0).

## Appendix: RustDesk's Influence on MediaServo's Feature Priorities
Based on RustDesk's feature set and technical validation, the suggested MediaServo development priority:

Phase 0 (architecture definition): Protobuf protocol definitions (the oneof pattern), the Capturer-trait screen-capture abstraction, the HBBS/HBBR signaling-relay separation model
Phase 1 (MVP): P2P + ICE-relay remote desktop, VP8/VP9 software encoding, H.264/H.265 hardware encoding, file transfer
Phase 2 (enhancements): TCP-tunnel port forwarding, terminal access, multi-monitor support, clipboard image sync
Phase 3 (enterprise): LDAP/OIDC integration, audit logs, white labeling, batch deployment tooling
Phase 4 (extensions): the web client (a unified WebRTC stack), the Python plugin system, camera viewing

RustDesk validated the commercial viability of the AGPLv3+Pro dual-license model, a reference case for MediaServo's open-source + support business model.

## Appendix: Data Sources

This analysis draws on:
- The official RustDesk GitHub repository (rustdesk/rustdesk, 118k+ stars)
- The official docs (doc.rustdesk.com)
- RustDesk Server Pro official documentation
- RustDesk Discord and Reddit community discussions
- Source analysis (the libs/scrap screen-capture abstraction, the hbb_common protocol definitions)
- Community contributors' technical blogs and tutorials

Data currency: all technical details as of July 2026. RustDesk ships stable monthly releases (currently v1.4.9) with continuously active commits.

The license boundary between MediaServo and RustDesk: RustDesk is AGPLv3, MediaServo Apache 2.0. All technical analysis in this document is based on public information (source reading, doc analysis, community discussion) and copies or derives no AGPLv3 code.

---

> This document is reference material for MediaServo's Phase 0 architecture-definition stage.
> As the project progresses, analyses and recommendations may adjust to actual requirements and constraints.
> All external product data, versions, and community statistics are as of July 2026.
> RustDesk is a trademark of Purslane Ltd. RustDesk source is licensed under AGPLv3.
> All technical analysis herein is based on publicly available information and contains no proprietary code.

> Architecture borrowing: this analysis of RustDesk's technology is intended as a reference for MediaServo's architecture design.

> Technical validation: RustDesk proved production-grade feasibility of Rust for remote desktop (network I/O, codec bridging, cross-platform system calls).

> Final conclusion: RustDesk is MediaServo's primary technical reference for architecture design, though its AGPLv3 code cannot be reused directly.
