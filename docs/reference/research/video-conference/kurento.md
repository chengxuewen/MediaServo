# Kurento Media Server & OpenVidu Reference Research Report

> **Generated:** 2026-07-19
> **Scope:** Kurento Media Server v7.x, OpenVidu v2/v3
> **Repositories:**
> - [Kurento/kurento](https://github.com/Kurento/kurento) — main repository (monorepo, Apache 2.0, C/C++/Java)
> - [Kurento/kurento-media-server](https://github.com/Kurento/kurento-media-server) — historical repository (archived)
> - [OpenVidu/openvidu](https://github.com/OpenVidu/openvidu) — main repository of the OpenVidu platform

---

## 1. Product Profile

### Positioning
Kurento Media Server (KMS) is an open-source WebRTC media server providing full-stack capability for media transport, processing, recording, and playback. It is the core of the entire Kurento ecosystem — application developers remotely control KMS through client APIs to build media pipelines that complete complex real-time audio/video processing.

OpenVidu is a high-level framework built on top of Kurento, wrapping Kurento's low-level capabilities and providing a simplified REST API + client SDKs for quickly developing video conferencing applications.

### Core Philosophy

| Principle | Meaning |
|------|------|
| **Media/signaling separation** | The Signaling Plane (application server) and the Media Plane (KMS) are fully decoupled, communicating over WebSocket + JSON-RPC |
| **Pipeline architecture** | Media processing is realized by chaining MediaElements together, with dynamic add/remove at run time |
| **Modular black boxes** | Each MediaElement is a self-contained functional unit; developers need not understand its internals |
| **End-to-end transparency** | The automatic media adaptation layer (agnostic media adapter) inserts transcoding automatically when connecting incompatible elements |
| **Cloud native** | PaaS deployment is supported; one application can call multiple KMS instances, and one KMS can serve multiple applications |

### Technology Stack
- **Core languages:** C/C++ (the media server itself), Java/Node.js (client SDKs)
- **Media engine:** GStreamer (the underlying multimedia framework providing codecs, multiplexing, filters, etc.)
- **Communication protocol:** WebSocket + JSON-RPC (Kurento Protocol)
- **License:** Apache 2.0
- **Architecture model:** MCU + SFU dual mode (can both mix and forward)

### Key Points in the Version Evolution
- **2013:** the Kurento project started (led by Universidad Rey Juan Carlos)
- **2016:** OpenVidu created as a simplified API layer over Kurento
- **~2023:** Kurento code migrated to a monorepo (Kurento/kurento)
- **OpenVidu v2:** the classic Kurento-based architecture (three editions CE/Pro/Enterprise, CE free)
- **OpenVidu v3 (2024-2025):** a complete rebuild — the underlying stack migrated from Kurento to LiveKit + mediasoup while keeping API compatibility (via the v2compatibility module)

---

## 2. Technical Characteristics

### 2.1 Core Architecture: Three-Layer Model

```
┌─────────────────────────────────────────────────────────────┐
│  Client Application (browser/mobile)                          │
│  ├── Presentation Layer: <video>, WebRTC RTCPeerConnection     │
│  └── Client-side App Logic                                    │
├─────────────────────────────────────────────────────────────┤
│  Application Server (signaling layer)                         │
│  ├── Business logic (Java/Node.js/Python/...)                  │
│  ├── Kurento Client SDK → WebSocket/JSON-RPC                  │
│  └── Session/Connection management                             │
├─────────────────────────────────────────────────────────────┤
│  Kurento Media Server (media layer)                           │
│  ├── JSON-RPC → MediaPipeline management                       │
│  ├── GStreamer Pipeline Engine                                 │
│  └── MediaElement instantiation and orchestration              │
└─────────────────────────────────────────────────────────────┘
```

### 2.2 MediaElement Types

Kurento abstracts media processing units as **MediaElement**s, in four major categories:

#### (1) Input Endpoints

| Element | Function |
|---------|------|
| **WebRtcEndpoint** | Bidirectional WebRTC communication endpoint, SDP Offer/Answer negotiation |
| **RtpEndpoint** | Bidirectional RTP/RTCP media transport |
| **HttpPostEndpoint** | Accepts media streams uploaded via HTTP POST |
| **PlayerEndpoint** | Reads media from a filesystem/HTTP URL/RTSP URL and injects it into the pipeline |

#### (2) Filters

| Element | Function |
|---------|------|
| **GStreamerFilter** | Generic filter — can inject any GStreamer element into the pipeline (a single element only, no bins) |
| **FaceOverlayFilter** | Face detection overlay (OpenCV) |
| **ZBarFilter** | QR code / barcode detection |
| **OpenCVFilter** | Generic OpenCV filter base class |
| **ChromaFilter** | Chroma-key cutout |

#### (3) Hubs (switching hubs — manage multiple media streams)

| Hub type | Function |
|----------|------|
| **Composite** | MCU mixing: mixes all input audio, outputs video arranged as a grid |
| **Dispatcher** | SFU routing: flexible routing from any input port to any output port |
| **DispatcherOneToMany** | One-to-many broadcast: one input → all outputs |

#### (4) Output Endpoints

| Element | Function |
|---------|------|
| **RecorderEndpoint** | Records media to the filesystem (WebM/MP4) |
| **HttpEndpoint** | Exposes media streams over HTTP |

### 2.3 MediaPipeline Design

```javascript
// typical WebRTC recording + filter pipeline (pseudocode)
const pipeline    = await kurento.create('MediaPipeline');
const webRtcEp    = await pipeline.create('WebRtcEndpoint');
const recorder    = await pipeline.create('RecorderEndpoint', {uri: 'file:///tmp/rec.webm'});
const faceOverlay = await pipeline.create('FaceOverlayFilter');
const gstFilter   = await pipeline.create('GStreamerFilter', {command: 'videoscale ! video/x-raw,width=640'});

// connect the pipeline
await webRtcEp.connect(recorder);       // record the raw stream
await webRtcEp.connect(faceOverlay);    // face detection
await faceOverlay.connect(gstFilter);   // scaling
await gstFilter.connect(webRtcEp);      // send the processed stream back
```

**Key characteristics:**
- **Dynamic composition:** elements can be inserted, activated, and deactivated at any time while media is flowing
- **Automatic adaptation:** the Agnostic Media Adapter automatically handles media format incompatibilities between elements (e.g. VP8→RAW RGB transcoding)
- **Acyclic constraint:** the pipeline topology has no cycles, but feedback loops are possible (e.g. a WebRtcEndpoint output connected back to itself)

### 2.4 GStreamer Integration Mechanism

Kurento delegates all of its core media processing to GStreamer:

1. **Each KMS process maintains one or more GStreamer pipelines internally**
2. **A MediaElement maps onto a GStreamer element/bin** (e.g. WebRtcEndpoint maps onto `webrtcbin`)
3. **MediaElement.connect() maps onto GStreamer pad linking** (`gst_pad_link()`)
4. **GStreamerFilter provides an extension entry point:** allows developers to inject any GStreamer element via `gst_parse_launch()`

```cpp
// core logic of kms-filters/GStreamerFilterImpl.cpp
GStreamerFilterImpl::GStreamerFilterImpl(...) {
    // parse the user-provided GStreamer command string into an element
    filter = gst_parse_launch(command.c_str(), &error);
    // check that it is a single element (bins are rejected)
    if (GST_IS_BIN(filter)) throw ...;
    // attach it to the internal element
    g_object_set(element, "filter", filter, NULL);
}

// runtime property manipulation
void GStreamerFilterImpl::setElementProperty(const string &name, const string &value) {
    GParamSpec *pspec = g_object_class_find_property(G_OBJECT_GET_CLASS(gstElement), name);
    // convert and set according to the type (int/float/double/string/enum)
    g_object_set_property(G_OBJECT(gstElement), name, &value);
}
```

### 2.5 Plugin System (Kurento Modules)

Kurento provides a complete module development system:

#### Module Types

| Type | Based on | Applicable scenarios |
|------|------|---------|
| **GStreamer module** | GstVideoFilter/GstAudioFilter | Generic media processing (requires GStreamer knowledge) |
| **OpenCV module** | OpenCVFilter | Computer vision, augmented reality |

#### Development Flow

```bash
# 1. Install the development toolkit
sudo apt-get install kurento-media-server-dev

# 2. Generate scaffolding
kurento-module-scaffold my_filter        # GStreamer module
kurento-module-scaffold my_vision true   # OpenCV module
```

Generated file tree:
```
my_filter/
├── CMakeLists.txt
├── src/
│   ├── gst-plugins/          # GStreamer element implementation
│   │   ├── gstmyfilter.cpp   # core processing logic (transform_frame_ip)
│   │   ├── gstmyfilter.h     # element header
│   │   └── myfilter.c        # plugin registration
│   ├── server/
│   │   ├── implementation/objects/
│   │   │   ├── MyFilterImpl.cpp   # JSON-RPC API implementation
│   │   │   └── MyFilterImpl.hpp
│   │   └── CMakeLists.txt
│   └── MyFilter.kmd.json     # Kurento Module Descriptor (interface definition)
```

#### KMD (Kurento Module Descriptor)
`.kmd.json` files define a module's complete API interface:
- Constructor parameters
- Method signatures (including JSON-RPC mapping)
- Properties (read/write control)
- Events
- Custom complex types

At CMake build time, `kurento-module-creator` is invoked to generate from the KMD:
- C++ server-side stub code
- JavaScript client bindings
- Java client bindings

#### Deployment
```bash
# install onto the machine running KMS after building
cmake .. && make && make install
# verify the installation
kurento-media-server --version   # list all installed modules
kurento-media-server --list      # list all available MediaObject Factories
```

> ⚠️ **ABI compatibility constraint:** modules must be compiled on a machine using the same compiler version and OS version (e.g. Ubuntu 20.04); they cannot be installed across OS versions.

### 2.6 Communication Protocol: Kurento Protocol

- **Transport layer:** WebSocket
- **RPC framework:** JSON-RPC 2.0
- **Lifecycle management:**
  - `create` — instantiate a MediaPipeline/MediaElement
  - `invoke` — call a method
  - `subscribe` — subscribe to events
  - `release` — release resources (reference counting + GC)
  - `transaction` — batch atomic operations

### 2.7 Session Establishment Flow (WebRTC)

```
Client                App Server              Kurento MS
  │                       │                      │
  │  SDP Offer + business request                 │
  │──────────────────────►│                      │
  │                       │  create pipeline     │
  │                       │  + WebRtcEndpoint    │
  │                       │─────────────────────►│
  │                       │  processOffer(SDP)   │
  │                       │─────────────────────►│
  │                       │  ◄── SDP Answer ──── │
  │  SDP Answer           │                      │
  │◄──────────────────────│                      │
  │                       │                      │
  │══════════ ICE/DTLS/SRTP media channel ═══════│
  │                       │                      │
```

---

## 3. Key Capabilities

### 3.1 Media Processing Capabilities

| Capability | Support | Notes |
|------|---------|------|
| **WebRTC SFU** | ✅ | Routing and forwarding implemented on Hub(Dispatcher) |
| **WebRTC MCU** | ✅ | Mixing + grid layout implemented on Hub(Composite) |
| **Recording** | ✅ | RecorderEndpoint → WebM/MP4 files |
| **Playback** | ✅ | PlayerEndpoint ← file/HTTP/RTSP |
| **Transcoding** | ✅ | Automatic adaptation layer + explicit GStreamerFilter control |
| **Video filters** | ✅ | FaceOverlay, Chroma, ZBar, GStreamerFilter |
| **Computer vision** | ✅ | OpenCVFilter base class + custom OpenCV modules |
| **RTSP push/pull** | ✅ | PlayerEndpoint(RTSP), RtpEndpoint |
| **RTP transport** | ✅ | RtpEndpoint |
| **Broadcasting (live)** | ✅ | Push to RTMP via OpenVidu (YouTube/Twitch) |
| **Screen sharing** | ✅ | WebRTC getDisplayMedia |

### 3.2 Codec Support
- **Video:** VP8, VP9, H.264, H.263 (depending on the GStreamer plugins installed)
- **Audio:** OPUS, G.711, Speex, AMR

### 3.3 Network Protocols
- **Signaling:** WebSocket (JSON-RPC) + REST (OpenVidu)
- **Media transport:** SRTP/SRTCP over ICE/DTLS (WebRTC), RTP/RTCP, HTTP, RTSP
