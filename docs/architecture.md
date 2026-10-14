# MediaServo Architecture Design

> Phase 0 — architecture definition | 2026-07-16 | Last synced with decisions.md: D205 (2026-07-31)

> **⚠️ MVP v2 architecture change (D118, 2026-07-17)**: the Phase 1 MVP uses the Host→Server→Remote relay three-component model (formerly Host↔Remote P2P). Both the signaling relay and the media relay are carried by mediaservo-server. The full P2P / Host↔Remote direct-connect mode is retained as a Phase 2+ architectural goal.

## 1. Overview

MediaServo is independently deployable multimedia infrastructure offering remote desktop, video conferencing, live streaming push/pull, surveillance camera access, and more. It uses a microkernel + plugin architecture and supports multi-form deployment.

### 1.1 The Seven Product Capabilities

| Capability | Phase | Description | Typical scenarios |
|------------|-------|-------------|-------------------|
| Remote desktop (Phase 1) | Phase 1 | Screen capture, GPU encoding, input injection | IT ops, remote work |
| Video conferencing (Phase 2+) | Phase 2+ | Multi-party audio/video, SFU/MCU, screen sharing | Team collaboration |
| Streaming push/pull (Phase 2+) | Phase 1 | RTMP/HLS/SRT ingest and distribution | Live streaming, content distribution |
| Surveillance cameras (Phase 2+) | Phase 2+ | ONVIF/GB28181 discovery and stream management | Security, inspection |
| WebRTC teleoperation (Phase 1) | Phase 1 | Low-latency video + RTCDataChannel control | Vehicle teleoperation, robotics |
| Vehicle push (Phase 1) | Phase 1 | Vehicle cameras streaming to the cloud | Connected vehicles |
| Recording & playback (Phase 2+) | Phase 2+ | Recording management, storage and playback (D61: deferred to Phase 2) | Compliance, audit |

> Note: WebRTC teleoperation, vehicle push, and cockpit pull — three sub-capabilities — all belong to the teleop domain, carried by SDKs such as field/client (see §7).

> 🔄 **The D126-D155 increment (2026-07-19)**: the three-layer logical abstraction model (D126), the Component architecture (D127-D134), the WebRTC architecture upgrade to webrtc-sys RTP tracks (D137), mediasoup SFU (D138), the unified Gateway (D128), multi-backend feature gates (D139-D140: webrtc-sys/webrtc-rs/stub, str0m Phase 2+), webrtc-kit design borrowing (D144-D151), and the Host monolith confirmation (D155). Implemented (Phase 3).

> ⚠️ **The D15/D118 scope change**: Phase 1 was originally Host↔Remote P2P direct connection and has changed to the Host→Server→Remote relay three-component model. See the [module doc](modules/03-client-host.md) and §2 for details.

### 1.2 Relationship to Third-Party Platforms

```
Industrial control end    ──┐              ┌── Host platform (enterprise apps)
                            ├── MediaServo ──┤
   references native crates │  media core   │ Docker module
                            │              │
                            └──────────────┘
```

- **With third-party platforms**: optional embedding via statically linked Rust crates, using only remote desktop and teleoperation capabilities
- **With host platforms**: zero hard dependency. A host platform can run MediaServo as a Docker module (analogous to Synology Surveillance Station); users/permissions are then delegated to the platform's RBAC/LDAP
- **Standalone deployment**: can run fully independently of third-party platforms, shipping a complete backend

### 1.3 Design Principles

| Principle | Description | Related decisions |
|-----------|-------------|-------------------|
| **Compile-time multi-backend dispatch** | The WebRTC multi-backend setup (webrtc-sys/webrtc-rs/stub; str0m Phase 2+) dispatches via compile-time `#[cfg(feature)]` with no runtime dyn-trait overhead. Borrows webrtc-kit's trait abstraction pattern | D144-D151 |
| **A WebRTC-free kernel** | mediaservo-common depends on no WebRTC crate; the MediaTransport trait is a pure abstraction | D150 |
| **The Component service layer** | Service-level Components are separated from pipeline-level Plugins — the three-layer abstraction model | D126 |

## 2. The Three-Layer Architecture

> 📄 See [modules/03-client-host.md](/docs/modules/03-client-host.md) for details

MediaServo adopts a three-layer architecture separating the control plane from the data plane: the backend centrally manages users, devices, permissions, licenses, and signaling; the Client/Host data planes communicate with the control plane over gRPC/REST. The dual Client/Host application design comes from decision D2 — the Host must run on platforms without a desktop environment (Linux servers, vehicle edge devices), and the split reduces the Host's size.

> 🔄 **The D128 Gateway routing**: a single port :9800 exposed externally, routed internally by the Gateway Component: `/ws` → SignalingComponent, `/admin/api/*` → AdminComponent, `/admin/*` → SPA static files, `/health` → MonitorComponent.

```
┌──────────────────────────────────────────────────────────────────┐
│                     The MediaServo backend                       │
│                                                                  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐  │
│  │  User    │ │ Permiss- │ │ License  │ │ Device   │ │ Signal-│  │
│  │  mgmt    │ │ ion ctrl │ │ mgmt     │ │ mgmt     │ │ ing    │  │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └────────┘  │
│                                                                  │
│  Auth Provider (trait): Local mode | platform mode | future LDAP │
│                                                                  │
│  Control plane: auth / authorization / feature flags / stream    │
│  caps / resolution caps / expiry                                 │
└────────────────────────────┬─────────────────────────────────────┘
                             │ gRPC / REST
        ┌────────────────────┼────────────────────┐
        ▼                    ▼                    ▼
┌───────────────────┐ ┌───────────────────┐ ┌───────────────────┐
│ MediaServo Client │ │ MediaServo Host   │ │ Other clients/AI  │
│ (desktop GUI)     │ │ (headless)        │ │                   │
│                   │ │                   │ │                   │
│ Full SDK          │ │ Produce SDK only  │ │ On-demand         │
│ produce + consume │ │ capture/encode    │ │ integration       │
│                   │ │ /push             │ │                   │
│ ┌───────────────┐ │ │                   │ │                   │
│ │ Tauri v2      │ │ │ embedded web cfg  │ │                   │
│ └───────────────┘ │ │                   │ │                   │
├───────────────────┤ ├───────────────────┤ │                   │
│ Scenario: operator│ │ Scenario: vehicle/│ │                   │
│ desktop; can      │ │ server room/edge  │ │                   │
│ remotely control  │ │ devices/cameras;  │ │                   │
│ others and be     │ │ produces media    │ │                   │
│ remotely controlled│ │ streams only     │ │                   │
└───────────────────┘ └───────────────────┘ └───────────────────┘

        Client/SDK layer — data plane: capture / encode / transport / decode / render
```

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| **Backend** | Control plane: users, devices, permissions, licenses, signaling | Rust (axum/tonic) |
| **Client** | Desktop GUI, the full-featured operations end | Tauri v2 + mediaservo-common |
| **Host** | Headless remote end, producing media streams only | mediaservo-common (no GUI dependency) |
| **SDK** | Core pipelines + domain plugins | The Rust crate family |

### 2.1 Client vs Host

| | MediaServo Client | MediaServo Host |
|---|---|---|
| **Environment** | Desktop OS | Server / edge / vehicle / headless |
| **GUI** | Tauri v2 | Embedded Web (a localhost config page) |
| **SDK** | Full (produce + consume) | Produce only (capture, encode, push) |
| **Role** | Can control others, can be controlled | Produces media streams only |
| **Install** | Desktop installer | Single binary `mediaservo-host` |
| **Size** | Large (includes the GUI framework) | Small (no GUI dependencies) |

Why two applications rather than one: the Host must run on platforms without a desktop environment (headless Linux servers, in-vehicle embedded devices).

> Phase 2+ feature: the headless Host supports a virtual display, producing synthetic frames without a physical monitor (testing, CI, cloud rendering scenarios).

## 3. Accounts & Permissions

> 📄 See [modules/05-auth-permissions.md](/docs/modules/05-auth-permissions.md) for details

The Auth Provider trait implements dual-mode authentication: Local mode (embedded SQLite + JWT) for standalone deployment; platform mode delegates to the platform's RBAC/LDAP for the Docker-module scenario. The permission model covers feature flags, quota limits (stream count/bitrate/resolution/duration), and license control (trial/standard/enterprise). At startup the client SDK pulls the permission config from the backend, caches it locally, and the License Manager validates before every pipeline operation.

### 3.1 The Auth Provider Modes

```
                    ┌──────────────────────────┐
                    │    MediaServo Backend    │
                    │                          │
                    │    AuthProvider (trait)  │
                    │    ┌───────────────────┐ │
                    │    │ authenticate()    │ │
                    │    │ authorize()       │ │
                    │    │ getPermissions()  │ │
                    │    └───────┬───────────┘ │
                    │            │             │
                    │    ┌───────┴───────┐     │
                    │    ▼               ▼     │
                    │  Local         Platform  │
                    │  (SQLite+JWT)  (gRPC LDAP)│
                    └──────────────────────────┘
```

```rust
#[async_trait]
trait AuthProvider: Send + Sync {
    async fn authenticate(&self, credential: &Credential) -> Result<User, AuthError>;
    async fn authorize(&self, user_id: &str, permission: &Permission) -> Result<bool, AuthError>;
    async fn get_permissions(&self, user_id: &str) -> Result<HashSet<Permission>, AuthError>;
}
```

### 3.2 The Two Modes

| | Local (standalone) | Platform mode |
|---|---|---|
| **User storage** | Embedded SQLite | Delegated to platform RBAC |
| **Authentication** | Local JWT | A host-platform-issued token |
| **Permissions** | A local RBAC table | Host-platform LDAP group mapping |
| **Config** | `auth.mode: "local"` | `auth.mode: "aude"` |
| **Scenario** | Standalone deployment | The Docker platform module |

### 3.3 The Permission Model

```typescript
interface Permission {
  // Feature flags
  capabilities: {
    streaming:  { push: boolean; pull: boolean };
    remote:     { control: boolean; controllable: boolean };
    conference: { host: boolean; join: boolean };
    surveillance: boolean;
    teleop:     { operator: boolean; vehicle: boolean };
  };
  
  // Quota limits
  quotas: {
    max_streams: number;        // max stream count
    max_bitrate: number;        // max bitrate (kbps)
    max_resolution: "720p" | "1080p" | "4k";
    max_duration: number;       // max session duration (seconds)
  };
  
  // License
  license: {
    type: "trial" | "standard" | "enterprise";
    expires_at: string;          // ISO 8601
    features: string[];          // the premium feature list
  };
}
```

The permission flow: at startup the client SDK pulls the permission config from the backend → caches it locally → the License Manager validates before each pipeline operation.

### 3.4 The Reference Model

Analogous to Synology DSM: MediaServo installed as a Docker module on a host platform, using the host platform's user/permission system (like Jira installed on a Synology using DSM's LDAP accounts).

## 4. The Plugin System

> 📄 See [modules/06-plugin-system.md](/docs/modules/06-plugin-system.md) for details

The mediaservo-common microkernel hosts the five core components: PluginManager, LicenseManager, ProtocolBroker, PipelineEngine, and AuthProvider. Plugins are grouped by domain into production (Host capture/encode/push), consumption (Client decode/render/pull), protocol (RTMP/HLS/SRT/RTSP/WebRTC), and relay (STUN/TURN/SFU). The three-layer trait hierarchy (Plugin → MediaSource/Processor/Sink → concrete implementations) provides clean extension boundaries.

### 4.0 The Component Service Layer (D126-D134)

> 📄 See [modules/15-component-architecture.md](/docs/modules/15-component-architecture.md) for details

Distinct from D1's deployment-topology three layers, D126 defines a three-layer logical abstraction in the code dimension:

| Layer | Abstraction | Managed by | Phase 1 instances |
|-------|-------------|-----------|-------------------|
| **Layer 3 Process** | An OS process | systemd/K8s | single process |
| **Layer 2 Component** | Service-level units | ComponentManager | Gateway / Signaling / Relay / Admin / Auth / Monitor |
| **Layer 1 Plugin** | Pipeline elements | PipelineEngine | ScreenCapture / HardwareEncoder / RtmpPublisher (Phase 2+) |

- **Plugin** answers "how" (how to encode H.264) — communicates via MediaPort (frame queues)
- **Component** answers "what" (manages WebRTC signaling) — communicates via ComponentBus (messages)
- A Component may internally hold Plugin instances (through the PipelineEngine)

**The Component/Plugin/Process deployment mapping**:

| Logical layer | Deployment home | Phase 1 examples |
|---------------|-----------------|-------------------|
| **Plugin layer** | Inside any Component | ScreenCapture, H264Encoder |
| **Component layer** | Host / Server / Remote processes | SignalingComponent, RelayComponent, GatewayComponent |
| **Process layer** | OS processes (systemd/K8s) | hostd, mediaservo-server, mediaservo-client |

The Component trait lives in its own crate `mediaservo-component` (D127) with an init→run→shutdown three-stage lifecycle (D131); ComponentBus supports dual-mode RPC + Event routing (D132-D133); ComponentManager provides single-level supervision + crash-loop protection (D134).

### 4.1 The Microkernel Architecture

```
mediaservo-common (the microkernel)
├── PluginManager     — plugin registration, lifecycle
├── LicenseManager    — permission validation, quota control
├── ProtocolBroker    — internal protocol routing (FlatBuffers)
├── PipelineEngine    — media pipeline scheduling
└── AuthProvider      — the auth interface (trait)

The plugin layer (by domain)
├── Production (Host)
│   ├── ScreenCapture     — screen capture (DXGI/PipeWire/CoreGraphics)
│   ├── CameraCapture     — camera capture (V4L2/AVFoundation/DirectShow)
│   ├── AudioCapture      — audio capture
│   ├── HardwareEncoder   — GPU encoding (NVENC/VAAPI/VideoToolbox/QSV)
│   ├── StreamPublisher   — streaming push (RTMP/SRT/WHIP)
│   └── InputReceiver     — input receiving (keyboard-mouse/touch)
│
├── Consumption (Client)
│   ├── ScreenRender      — frame rendering
│   ├── VideoDecode       — video decoding
│   ├── AudioPlayback     — audio playback
│   ├── StreamSubscriber  — stream pulling
│   └── InputForwarder    — input forwarding
│
├── Protocol
│   ├── RtmpPlugin        — RTMP ingest/distribution
│   ├── HlsPlugin         — HLS packaging
│   ├── SrtPlugin         — SRT transport
│   ├── RtspPlugin        — RTSP ingest
│   ├── WebRtcPlugin      — WebRTC P2P/SFU
│   └── DataChannelPlugin — WebRTC RTCDataChannel
│
└── Relay
    ├── StunTurnPlugin    — NAT traversal
    └── SfuRelayPlugin    — SFU media forwarding
```

### 4.2 The Core Traits

```rust
/// The plugin base trait
#[async_trait]
trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> (u16, u16, u16);
    fn category(&self) -> PluginCategory;
    async fn init(&mut self, ctx: &PluginContext) -> Result<()>;
    async fn shutdown(&mut self) -> Result<()>;
}

/// Media source (production)
#[async_trait]
trait MediaSource: Plugin {
    fn output_port(&self) -> &MediaPort;
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
}

/// Media processor
#[async_trait]
trait MediaProcessor: Plugin {
    fn input_port(&self) -> &MediaPort;
    fn output_port(&self) -> &MediaPort;
    async fn process(&mut self, frame: MediaFrame) -> Result<MediaFrame>;
}

/// Media sink (consumption)
#[async_trait]
trait MediaSink: Plugin {
    fn input_port(&self) -> &MediaPort;
    async fn consume(&mut self, frame: MediaFrame) -> Result<()>;
}
```

/// The multi-backend transport abstraction (ponytail: mediaservo-common defines this trait, with mediaservo-webrtc's RTCPeerConnection as the implementation layer. The MediaTransport trait migrates into mediaservo-common in Phase 2+ to decouple from the webrtc crate. See docs/modules/17-webrtc-crate.md)
/// mediaservo-common depends on no WebRTC crate, guaranteeing independent compilation and testing
#[async_trait]
pub trait MediaTransport: Send + Sync {
    async fn create_video_track(&self, source_id: &str) -> Result<TrackLocal, TransportError>;
    async fn create_audio_track(&self) -> Result<TrackLocal, TransportError>;
    async fn on_remote_track(&self, callback: Box<dyn Fn(TrackRemote) + Send + Sync>);
}

# ponytail: MediaTransport trait — Phase 0 has webrtc-sys as the main backend, webrtc-rs has struct skeletons, str0m in Phase 2+. The full multi-backend trait system lands Phase 2+.

# mediaservo-common has zero WebRTC dependency (D150).

### 4.3 Plugin Loading Modes

✅ Settled: compile-time inventory + runtime dlopen dual modes (D13, D29)

> ⚠️ A risk assessment of runtime dlopen: dynamic loading carries ABI-compatibility risks (plugin/host Rust version mismatches, dependency version conflicts); Phase 1 prefers the compile-time inventory mode. dlopen stays a Phase 2+ advanced feature, requiring a plugin version-negotiation protocol and isolation sandboxing.
- ⚠️ Additional risk: runtime plugin dynamic loading has limited production precedent in real-time media systems. OBS/LVQR/MediaMTX all use compile-time composition (the Phase 1 choice).

## 5. The Pipeline Model

> 📄 See [modules/08-pipeline-model.md](/docs/modules/08-pipeline-model.md) for details

Adopts LVQR's Unified MediaFragment Model, with one internal unified format avoiding the N×M protocol-conversion matrix. GStreamer handles mature hot paths (protocol parsing, codec, packaging); homegrown paths cover GPU encode bridging, the sans-I/O WebRTC core, the RTCDataChannel control protocol, and the input-injection pipeline.

### 5.1 The Design Choice

Rather than GStreamer's directed-graph model, we follow **LVQR's Unified Fragment Model**:

```
┌──────────────────────────────────────────────────┐
│  The input adaptation layer                      │
│  RTMP · SRT · RTSP · WHIP · screen · camera      │
│          │                                       │
│          ▼                                       │
│  ┌──────────────────────────────┐                │
│  │    Unified MediaFragment     │  ← one internal format │
│  │    (encoded frames + metadata)│                │
│  └──────────────────────────────┘                │
│          │                                       │
│          ▼                                       │
│  PipelineEngine (a programmable processing chain)│
│          │                                       │
│          ▼                                       │
│  The output adaptation layer                     │
│  HLS · DASH · WHEP · WebRTC · render · record    │
└──────────────────────────────────────────────────┘
```

Advantages:
- Avoids the N×M protocol-conversion matrix
- Each input/output protocol is just a thin adapter
- The intermediate processing chain composes programmatically

Phase 2+: support Simulcast/SVC multi-layer encoding — a single input produces multiple quality layers, adaptively selected by SFU/client per bandwidth (Zoom/Jitsi/LiveKit all have this).

### 5.2 GStreamer Hot Paths

GStreamer is the implementation engine in these scenarios:
- Protocol parsing (RTMP/RTSP/SRT parsing)
- Codec (via merged `videoconvertscale` transforms)
- HLS/DASH packaging
- `webrtcbin2` (rewritten in Rust, saving 5 threads per session)

### 5.3 Homegrown Hot Paths

- GPU encode bridging (calling NVENC/VAAPI directly via `libloading`, avoiding GStreamer's codec overhead)
- The zero-copy path: capture→encode→network entirely in GPU memory (dmabuf/CUDA handles), benchmarked against Parsec's 7ms end-to-end latency
- The sans-I/O WebRTC core (str0m / webrtc-rs rtc)
- The RTCDataChannel control protocol (a custom binary protocol)
- The input-injection pipeline (low-latency keyboard-mouse/touch forwarding)

**The data-boundary constraint (C5/D155)**: only `&[u8]` bytes may pass between GStreamer and WebRTC. Passing GStreamer buffers directly into libwebrtc is forbidden (incompatible allocators: glib malloc vs C++ new).

### 5.4 Data Semantics

| Data class | Semantics | Notes |
|------------|-----------|-------|
| Video frames | at-most-once | Frame drops acceptable; keyframe+delta frames may be skipped |
| Control commands | at-least-once | Must arrive; retransmit + dedup |
| Emergency stop | exactly-once (best-effort) | A standalone UDP channel, redundantly sent on multiple paths |

## 6. Deployment Forms

> 📄 See [modules/02-deployment-modes.md](/docs/modules/02-deployment-modes.md) for details

Four deployment forms cover everything from embedded linking to standalone deployment: Embed (~5 plugins, third-party platform embedding), Sidecar (~12 plugins, a host-platform container sidecar), Standalone (all plugins + Web UI), and the platform module (a Docker container delegating auth to the platform).

```
┌─────────────────────────────────────────────────────────────┐
│  Form           Architecture             Suited scenarios   │
├─────────────────────────────────────────────────────────────┤
│  Embed         statically linked Rust    third-party        │
│                crates → ~5 plugins       platforms embed    │
│                                          desktop + teleop   │
│                                                             │
│  Sidecar       container + napi-rs       host-platform      │
│                bindings → ~12 plugins    enterprise apps    │
│                                                             │
│  Standalone    standalone processes +    standalone         │
│                full backend → all        deployment         │
│                plugins + Web UI                             │
│                                                             │
│  Platform      a Docker container        blends into the    │
│  module        → auth delegated to       host platform      │
│                the platform                                 │
└─────────────────────────────────────────────────────────────┘
```

## 7. The SDK Layers

> 📄 See [modules/04-sdk-layers.md](/docs/modules/04-sdk-layers.md) for details

The four-SDK master architecture (D222-D232): **link** (connection) / **field** (composed sessions) / **client** (cockpit-side consumption orchestration) / **deck** (media data). It replaces the early field/remote dual-facade model (D65-D69/D82 repealed).

- **link** (the connection face, pure Rust): the frame_bus frame bus, the signal WS signaling client, auth integration (reusing common PSK/JWT), dc (Phase 2, webrtc-rs)
- **field** (the composed SDK): push/pull (webrtc-sys, C12) + re-export link (signaling/bus/auth) + re-export deck (capture: MediaDevices/VideoSource) — a one-line dependency, complete closed loop; field→deck **static direct-link by default** (D231; FFmpeg verified openssl-free)
- **client** (cockpit-side consumption orchestration): VideoRenderer rendering, multi-session orchestration, Input Forward/telemetry, deck playback integration; depends on field (D232)
- **deck** (the media data face, dual form): source capture (GStreamer) / codec (FFmpeg static) / record / playback

Dependencies are strictly one-way and acyclic: `field→(webrtc, link, deck)`; `client→field`; `deck→(codec, media, common)`. mediaservo-common/webrtc/media/codec are the underlying foundations.

```
mediaservo-link (connection face, pure Rust)        mediaservo-deck (media data face, dual form)
  ▲ rlib                                              ├─ rlib: standalone app use
  │                                                   ├─ rlib full: field's static dep (default)
mediaservo-webrtc (abstraction layer, C12)            └─ cdylib: deck-full.so (optional dlopen, OTA)
  ▲ backend-webrtc-sys / backend-webrtc-rs
  │
mediaservo-field (composed SDK = webrtc + link + deck)
  ├─ Media: push/pull (backend-webrtc-sys)
  ├─ Comms: signal/frame_bus/auth (re-export link)
  └─ Capture: MediaDevices/VideoSource (re-export deck[source])
      │ static dependency (merged at build time, self-contained at runtime)
      ▼
mediaservo-client (cockpit consumption orchestration) ── depends on field ── (+ deck playback integration)
```

Phase 2+: video-conference / live / surveillance sub-scenarios layer scenario APIs over the field session face; the binding system c/cxx/py dual backend (see modules/04-sdk-layers.md §8).

### 7.1 The Client's UI Modules

The MediaServo Client is a unified desktop app. UI modules load dynamically per the permissions the backend returns:

```typescript
const permissions = await backend.getPermissions(userId);

// Permission-driven UI
const modules: Module[] = [];
if (permissions.streaming) modules.push(StreamingModule);    // the streaming tab
if (permissions.remote)    modules.push(RemoteDesktopModule); // the remote-desktop tab
if (permissions.conference) modules.push(ConferenceModule);   // the conference tab
if (permissions.surveillance) modules.push(SurveillanceModule); // the surveillance tab

// Modules without permission are never loaded
```

## 8. Protocols & Communication

> 📄 See [modules/07-protocols.md](/docs/modules/07-protocols.md) for details

The protocol stack has five layers: internal IPC (FlatBuffers zero-copy), third-party platform integration (C FFI static linking), host-platform integration (napi-rs native modules), the backend control plane (gRPC + REST), and the media data plane (RTP/SRT/WebRTC). Signaling uses in-house WebSocket with long-term evolution (v2 = the control domain, v3 = session resume; S0/S0.5 landed); the MQTT 5.0 channel **is no longer a default dual-track promise** — evaluation triggers only when an offline-command-delivery/large-scale-presence scenario appears, with the precondition gate = the "externalized session state" rework (this repo's "connection = session" model and the broker bus model cannot be mixed). Verdict = F13/D289 (booked as reserved).

| Layer | Protocol | Description |
|-------|----------|-------------|
| **Internal IPC** | FlatBuffers | Zero-copy communication between plugins |
| **Third-party platforms** | C FFI | Rust → C static linking |
| **Host platform** | napi-rs | Rust → Node.js native module |
| **Backend services** | gRPC (protobuf) | Control-plane API |
| **Signaling** | In-house WebSocket (long-term; MQTT = a trigger clause, D289 revising D74's dual-track promise) | Room management, SDP/ICE exchange; see the [signaling architecture doc](modules/10-signaling-architecture.md) |
| **Media transport** | RTP/RTCP, SRT, WebRTC | The data plane |
| **Client ⇄ backend** | gRPC + REST | Auth, permission pull, config sync |
| **Future exploration** | MoQ (Media-over-QUIC) | Phase 3 evaluation: low-latency media transport, WebTransport |


## 9. The Cargo Workspace Structure

The current workspace contains 7 Cargo member crates (the D126-D155 increment): mediaservo-common (the microkernel), mediaservo-host, mediaservo-client, mediaservo-server, mediaservo-webrtc (added by D137, the RTP track API), mediaservo-media (the media pipeline engine), mediaservo-codec (the three codec backends: stub+FFmpeg+GStreamer).

```
crates/
├── mediaservo-common/       the microkernel (PluginManager, LicenseManager, PipelineEngine, AuthProvider trait)
├── mediaservo-host/         the Host app (headless; capture+encode+push+signaling+config; the D155 monolith)
├── mediaservo-client/       the Remote app (pull+decode+render+control)
├── mediaservo-server/       the Server app (signaling relay+monitoring+session management, mediasoup SFU; C21: mediasoup only in this crate's dependency tree — host/client must not depend on it)
├── mediaservo-webrtc/       the WebRTC wrapper (the W3C API; the webrtc-sys default backend/libwebrtc; multi-backend feature gates; C12: all client crates use WebRTC only through this layer, never linked with mediasoup)
├── mediaservo-media/        the media pipeline (PipelineEngine, BroadcastEngine, Transform interfaces)
└── mediaservo-codec/        codec (stub + FFmpeg statically linked + GStreamer via pixi)
  
Phase 2+ planned crates: mediaservo-component, mediaservo-transport, mediaservo-signaling, mediaservo-pipeline, mediaservo-auth, mediaservo-field, mediaservo-field-c, mediaservo-client-c, mediaservo-napi
```

## 10. Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **Core language** | Rust (edition 2024) | Zero-cost abstractions, memory safety, cross-platform |
| **WebRTC stack** | webrtc-sys (libwebrtc FFI, default, D137+D139) → webrtc-rs (Phase 0 struct skeletons, pure Rust, D32) → str0m (Phase 2+, Embed, D11) | Multi-backend compile-time feature-gate dispatch (D139-D140, D144-D151) |
| **GPU encoding** | libloading bridging NVENC/VAAPI/VT | Avoids compile-time binding to GPU SDKs |
| **Codec** | GStreamer (gst-plugins-rs) | Broad coverage, mature ecosystem |
| **Signaling** | In-house WebSocket (long-term; MQTT = a trigger clause, D289) | Unified Room + Protobuf dual formats (D52-D53) |
| **Internal protocol** | FlatBuffers | Zero-copy, multi-language |
| **Desktop GUI** | Tauri v2 + React | Lightweight, Rust backend, cross-platform |
| **Embedded Web** | axum + static HTML | The Host config page, no framework dependency |
| **Build** | Cargo workspace | Unified dependency management |
| **Web UI** | React 19 + Ant Design 5 | The server admin panel; reuses the host-platform ecosystem (D87) |
| **Database** | sqlx + SQLite (D101) | Compile-time SQL validation, async, lightweight |
| **Config** | serde_yaml + env vars (D101) | Sensitive values overridden via env (JWT_SECRET etc.) |
| **Observability** | tracing JSON stdout + prometheus-client (D99) | Docker logs collection + the /metrics endpoint |
| **CI/CD** | GitHub Actions three stages (D-CI-01, D101) | check → test → build+docker push |
| **Deployment** | tarball + install.sh (Phase 1, D104) → Docker Compose (Phase 2, D110) | A progressive deployment strategy |
| **Host IPC** | tokio::mpsc (Phase 1) → iceoryx2 SHM (Phase 2, D102) | Phase 1 single-process internal communication |
| **SDK build** | cbindgen + hand-written CMake/pc (D109; Phase 2: cargo-c) | .a + .so + .h (cbindgen) + .pc (pkg-config) |
| **SDK install** | /usr/local (D106) | FHS standard; pkg-config's default search path |
|- Phase 2+: Alertmanager for Prometheus alert routing |
| **SFU engine** | mediasoup-sys 0.14.1 (mediasoup 0.24.1) | Production-grade Rust bindings; the Router/Transport/Producer/Consumer model (D138) |
| **Component Bus** | tokio::mpsc (Phase 1) → Zenoh (Phase 2, D129) | InProcessBus; dual-mode RPC+Event (D132-D133) |

## 11. Settled Decisions (formerly pending)

| # | Topic | Status |
|---|-------|--------|
| 1 | Plugin loading — compile-time inventory + runtime dlopen | ✅ D13, D29 |
| 2 | The signaling service — a single axum HTTP+WS in-process | ✅ D12, D52 |
| 3 | The Cargo workspace structure — 7 crates (now) → 10+ crates (Phase 2+) | ✅ D16, D82, D127, D142 |
| 4 | Inter-plugin communication — the FlatBuffers schema design | ✅ D10 |
| 5 | Record/playback — fMP4 + splitmuxsink as a first-class pipeline path in Phase 2+ (D34-D40) | ✅ D34-D40 |
| 6 | Host packaging — a tarball + a cargo-c SDK tarball + CMake | ✅ D102-D106 |
| 7 | The three-layer logical abstraction — Plugin/Component/Process separation | ✅ D126 |
| 8 | The WebRTC architecture upgrade — RTCDataChannel→RTP tracks, the mediasoup SFU | ✅ D137-D138 |
| 9 | The multi-backend feature-gate strategy — Phase 0 implements webrtc-sys only; webrtc-rs struct skeletons; str0m Phase 2+ | ✅ D139-D140, D144-D151 |
| 10 | The unified Gateway pattern — a single port :9800 routing | ✅ D128 |
| 11 | The Component framework — its own crate, a 3-stage lifecycle, a dual-mode bus | ✅ D127, D131-D134 |

- Phase 2+: Docker + Alertmanager configuration for production alerting

## 12. Appendix

### A. Reference Projects

| Project | Reference points | Language |
|---------|------------------|----------|
| RustDesk | Remote-desktop architecture, P2P hole punching, single binary multi-role | Rust |
| LVQR | the Unified Fragment Model, multi-protocol adaptation, a 29-crate workspace | Rust |
| str0m | the sans-I/O WebRTC core | Rust |
| webrtc-rs | W3C-compatible WebRTC, runtime agnostic | Rust |
| MediaMTX | Protocol routing (zero transcoding) | Go |
| Parsec | Low-latency remote desktop, the GPU encode pipeline | C++ |
| tether-rally | RTCDataChannel teleoperation binary protocol | JS/C++ |
| QUIC / MoQ / WebTransport | Low-latency media transport standards | IETF |

- Multi-SIM bonding (Phase 3): Vay's 4-SIM parallel carrier aggregation

### B. Key WebRTC Teleoperation Metrics

| Metric | Value | Source |
|--------|-------|--------|
| Video latency threshold | <150ms, the MVP acceptance bar (D93) (the task-completion 93%→50% inflection) | the LAVT study |
| Control latency | <50ms, the MVP acceptance target (D93) | tether-rally measurements |
| RTCDataChannel | unordered, maxRetransmits=0 (control) | RFC 8831 |
| Emergency stop | a standalone UDP path (no WebRTC dependency) | CallSphere production experience |
| 4G/5G end-to-end | ~200ms video, ~300ms control | Provost et al. 2026 |
