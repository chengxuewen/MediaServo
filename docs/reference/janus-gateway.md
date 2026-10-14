# Janus Gateway — Reference Analysis

> Research date: 2026-07-19 | Version: v1.4.0 | Repo: [meetecho/janus-gateway](https://github.com/meetecho/janus-gateway)

## 1. Product Profile

| Attribute | Value |
|-----------|-------|
| **Name** | Janus WebRTC Server (Janus Gateway) |
| **Developer** | [Meetecho](https://www.meetecho.com/) s.r.l. |
| **First release** | 2014 |
| **Current version** | v1.4.0 (2026-02-06) |
| **License** | GPL v3.0 (server), MIT (janus.js) |
| **GitHub Stars** | ~9,100 |
| **Language** | C (82.8%), JavaScript (11.7%) |
| **Positioning** | General-purpose WebRTC server — lightweight, modular, signaling-agnostic |
| **Target users** | Developers who need a self-hosted, extensible WebRTC media server |

## 2. Architectural Characteristics

### 2.1 Core + Plugin Architecture

Janus's design philosophy: **an extremely thin core; all business logic lives in plugins**.

```
┌─────────────────────────────────────────────────────────┐
│                    Janus Core                            │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐   │
│  │ Session  │  │  Handle  │  │ ICE/DTLS/SRTP/SCTP   │   │
│  │ Manager  │  │ Manager  │  │ (libnice/OpenSSL)    │   │
│  └──────────┘  └──────────┘  └──────────────────────┘   │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Transport Layer (HTTP/WS/RabbitMQ/MQTT/Nanomsg) │   │
│  └──────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Plugin Loader (dlopen .so)                      │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
         │                    │                    │
    ┌────┴────┐         ┌────┴────┐         ┌────┴────┐
    │ Echotest│         │VideoRoom│         │Streaming│  ...
    │  .so    │         │  .so    │         │  .so    │
    └─────────┘         └─────────┘         └─────────┘
```

**Core responsibilities**: only the WebRTC protocol stack (ICE/DTLS/SRTP) + plugin loading + message routing.

**Plugins never touch**: ICE negotiation, DTLS handshake, SRTP encrypt/decrypt — the core is transparent to plugins.

### 2.2 Session/Handle Model

```
Browser → 1. Create Session (session_id)
        → 2. Attach to Plugin (handle_id)
              ↳ handle ↔ janus_plugin_session (opaque)
        → 3. Messages (handle_message)
              ↳ JSEP offer/answer (optional, for WebRTC)
        → 4. Media Flow (RTP/RTCP via the handle's ICE)
        → 5. Detach/Destroy
```

One session may attach multiple handles to different plugins, letting one user use several features simultaneously.

### 2.3 Thread Model

- **One GMainLoop thread per handle** (PR #1399, 2018): replacing the old 2-threads-per-RTCPeerConnection
- **Plugin helper threads** (optional): the VideoRoom and Streaming plugins support decoupled packet receipt and dispatch
- **`event_loops` pool**: a configurable thread pool, reused rather than created per handle

### 2.4 Transport Layer

Supports 6 transport protocols, all implemented by loadable transport plugins: HTTP/HTTPS (libmicrohttpd), WebSocket (libwebsockets), RabbitMQ, MQTT, Nanomsg, Unix Sockets.

### 2.5 Plugin API

```c
// Callbacks every plugin must implement
struct janus_plugin {
    int (*init)(const char *config_path);           // at startup
    void (*destroy)(void);                           // at shutdown
    void (*create_session)(janus_plugin_session *, int *error);
    void (*destroy_session)(janus_plugin_session *, int *error);
    struct janus_plugin_result *(*handle_message)(   // message handling
        janus_plugin_session *, char *, json_t *, json_t *);
    // Optional: media callbacks
    void (*incoming_rtp)(...);   // receive RTP
    void (*incoming_rtcp)(...);
    void (*setup_media)(...);    // PC ready
    void (*hangup_media)(...);
};

// Interfaces the core exposes to plugins
struct janus_callbacks {
    int (*push_event)(...);     // push events to the client
    void (*relay_rtp)(...);     // relay RTP
    void (*relay_rtcp)(...);
    void (*close_pc)(...);
    void (*end_session)(...);
};
```

Loading: `dlopen()` + `dlsym("create_p")` obtains the plugin instance.

## 3. Key Capabilities

| Capability | Description |
|------------|-------------|
| **SFU** | VideoRoom plugin: 1 publisher → N subscribers |
| **MCU** | AudioBridge plugin: Opus audio mixing |
| **Codecs** | VP8/VP9/H.264/H.265/AV1, Opus/G.711/G.722 |
| **Simulcast** | VP8/VP9/H.264 multi spatial/temporal layers |
| **Multistream** | v1.0.0+ full Unified Plan support |
| **Recording** | Built-in .mjr format + the janus-pp-rec post-processing tool |
| **Cascading** | v1.4.0 Remote Publishers: cross-instance RTP forwarding |
| **Extension points** | 12 official plugins + custom .so development |

## 4. Plugin List (v1.4.0)

| Plugin | Purpose |
|--------|---------|
| Echo Test | Loopback testing |
| Video Room | SFU multi-party video (publish/subscribe) |
| Video Call | 1-1 video calls |
| Streaming | Broadcasting RTSP/RTMP streams to WebRTC |
| Record & Play | Recording WebRTC streams + playback |
| Text Room | RTCDataChannel chat |
| Audio Bridge | MCU audio mixing |
| SIP Gateway | WebRTC↔SIP interop |
| Lua/Duktape | Plugin authoring in Lua/JS |

## 5. Ecosystem & Market

- **Production cases**: IETF conference streaming, enterprise video conferencing, media distribution
- **Community**: active Discourse forum, 300+ contributors
- **Commercial support**: Meetecho offers consulting
- **Competition**: LiveKit (more modern but heavier), mediasoup (SFU specialist), Jitsi (complete solution)

## 6. Highlights & Limitations

### Highlights

- **Extremely flexible**: the plugin architecture places zero constraints on application logic
- **Signaling-agnostic**: ships no signaling server; you bring your own
- **Multi-transport**: 6 transport protocols, all async JSON
- **Lightweight**: pure C, few dependencies, runs on a Raspberry Pi
- **Mature**: in production since 2014

### Limitations

- **C code**: 116K lines of C, memory-safety risk (use-after-free, buffer overflows)
- **No built-in clustering**: a room lives on one instance only; clustering is built externally
- **Fragile plugin ABI**: one version bump and every .so must be recompiled
- **No built-in auth**: an external system must be layered on
- **No Windows support**: Linux/macOS only
- **RTP extensions stripped by the core**: leaving some extensions broken

## 7. Reference Value for MediaServo

### Adoptable

| Concept | Janus's approach | MediaServo application |
|---------|------------------|------------------------|
| **Thin core + plugins** | The core handles only the WebRTC protocol; all business lives in plugins | mediaservo-core microkernel + plugin system |
| **Session/Handle isolation** | opaque `janus_plugin_session` + reference counting | `Arc<SessionHandle>` + trait objects |
| **Async message pattern** | `handle_message` → returns results → `push_event` | `handle_message()` → `tokio::mpsc::Sender` |
| **Transport abstraction** | Transports are loadable plugins too | `TransportProvider` trait |
| **Signaling-agnostic** | Imposes no signaling protocol | In-house WebSocket signaling, swappable |

### Needs Adaptation

| Janus's approach | MediaServo adaptation |
|------------------|------------------------|
| `dlopen .so` runtime loading | **Compile-time traits** — `Vec<Box<dyn PluginTrait>>`, type-safe |
| C function pointer tables | **Rust traits** — checked at compile time, default impls for optional methods |
| Single-threaded GMainLoop per handle | **tokio async** — one task per handle, work-stealing scheduler |
| `json_t` messages | **serde + strongly-typed Message enums** |
| Manual refcounting | **Arc + the ownership system** — eliminates use-after-free |

### Must Avoid

- **Single-threaded blocking** → use tokio async
- **Fragile C ABI** → Rust traits + semver
- **No built-in clustering** → design cluster-awareness from Phase 2
- **Memory-only sessions** → an optional persistence layer (Redis/SQLite)
- **janus.js is a demo library** → ship a first-class client SDK
**Related decisions**: D97 (plugin architecture), D144-D145 (multi-backend traits)
