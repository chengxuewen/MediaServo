# Client & Host

> MediaServo offers two client forms: Client (full-featured desktop GUI) and Host (headless remote end).

---

## Comparison Overview

| Dimension | MediaServo Client | MediaServo Host |
|-----------|-------------------|-----------------|
| **Environment** | Desktop OS | Server / edge / vehicle / headless |
| **GUI** | Tauri v2 | Embedded Web (localhost config page) |
| **SDK** | Full (produce + consume) | Produce only (capture, encode, push) |
| **Role** | Can control others, can be controlled | Produces media streams only |
| **Install** | Desktop installer | Single binary `mediaservo-host` |
| **Size** | Large (includes GUI framework) | Small (no GUI dependencies) |

**Why two apps**: the Host must run on platforms without a desktop environment (headless Linux servers, in-vehicle embedded devices).

---

## 1. Client — Full-Featured Desktop GUI Application

### Scenario
Operator desktop; can remotely control others and be remotely controlled.

### Architecture
```
┌─────────────────────────┐
│  Tauri v2 GUI           │
│  ┌───────────────────┐  │
│  │ React frontend    │  │
│  │ Permission-driven │  │
│  │ UI Module         │  │
│  └───────────────────┘  │
│  ┌───────────────────┐  │
│  │ mediaservo-common │  │
│  │ Full SDK          │  │
│  └───────────────────┘  │
└─────────────────────────┘
```

### Permission-Driven UI

At startup the client pulls the permission configuration from the backend and dynamically loads UI modules:

```typescript
const permissions = await backend.getPermissions(userId);

const modules: Module[] = [];
if (permissions.streaming)    modules.push(StreamingModule);
if (permissions.remote)       modules.push(RemoteDesktopModule);
if (permissions.conference)   modules.push(ConferenceModule);
if (permissions.surveillance) modules.push(SurveillanceModule);

// Modules without permission are never loaded
```

### Scenario Coverage
- Remote desktop (control others / be controlled)
- Video conferencing (join/host)
- Streaming push/pull (watch/manage)
- Surveillance (view/playback)
- Teleoperation (drive vehicles/robots)

---

## 2. Host — Headless Remote End

### Scenario
Vehicles, server rooms, edge devices, cameras — produces media streams only, no GUI needed.

### Architecture

Host monolith — GStreamer pipeline + WebRTC transport running in one process (D155):

```
┌──────────────────────────────────────────┐
│  mediaservo-host  (single binary)        │
│                                          │
│  ┌────────────────────────────────────┐  │
│  │  GStreamer (C, glib dynamic .so)   │  │
│  │  capture → encode → appsink        │  │
│  │             │                      │  │
│  │    H.264 byte buffer (&[u8])       │  │
│  │             ↓                      │  │
│  ├────────────────────────────────────┤  │
│  │  mediaservo-webrtc (Rust wrapper)  │  │
│  │  TrackLocal::write_frame(&[u8])    │  │
│  │             ↓                      │  │
│  ├────────────────────────────────────┤  │
│  │  webrtc-sys (C++, libwebrtc .a)    │  │
│  │  RTP packetizer → ICE → network    │  │
│  └────────────────────────────────────┘  │
│                                          │
│  ┌────────────────────────────────────┐  │
│  │  axum HTTP/WS (signaling+metrics+  │  │
│  │  config)                           │  │
│  │  Signaling + Metrics + Session     │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────────┘
```

Key design: GStreamer and libwebrtc are coupled through the pure-byte `&[u8]` interface, each using its own memory allocator (glib malloc vs C++ new); no shared memory, no symbol conflicts. GStreamer is an optional feature; embedded platforms simply exclude it.

### Characteristics
- Single binary `mediaservo-host` (~25 MB, GStreamer + libwebrtc statically linked)
- GStreamer pipeline + WebRTC transport in one process (D155, same pattern as OBS/Sunshine)
- Interface coupling: appsink → `&[u8]` byte buffer → TrackLocal::write_frame (memory isolation)
- No GUI dependencies; suitable for embedded devices
- Embedded Web config page (localhost)
- Contains only the production SDK (capture, encode, push)
