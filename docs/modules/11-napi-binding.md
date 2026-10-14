# 11. NAPI Binding — Node.js API

> Version: 0.1 | Date: 2026-07-16 | Phase: 0

## 11.1 Design Principles

- **Session abstraction**: only the high-level Session API is exported; the Rust trait hierarchy is not exposed
- **EventEmitter**: native Node.js pattern, zero extra dependencies
- **Delivered decoded**: GStreamer decode on the Rust side → RGBA Buffer → napi-rs copy
- **Zero code generation**: napi-rs bindings hand-written, no macro dependency

## 11.2 Architecture

```
Host platform (Node.js)
│
├─ @mediaservo/napi
│   ├─ Session          ← napi-rs exported class
│   │   ├─ EventEmitter
│   │   ├─ control.send()
│   │   └─ getStats()
│   │
│   └─ types.d.ts       ← TypeScript type declarations
│
│  napi-rs FFI
│
▼
mediaservo-common (Rust)
├─ SessionManager        ← new thin wrapper
├─ MediaTransport        ← existing
├─ PluginManager         ← existing
└─ pipeline              ← existing
```

## 11.3 Session API

### Creating

```typescript
import { createSession, SessionType, SessionRole } from '@mediaservo/napi';

const session = createSession({
  type: 'teleop_cockpit',      // SessionType
  role: 'cockpit',             // SessionRole
  transport: 'libwebrtc',      // TransportBackend: 'str0m' | 'libwebrtc'

  signaling: {
    url: 'wss://signal.example.com/ws',
    token: '<JWT>',
    reconnect: {
      maxAttempts: 5,
      intervalMs: 2000,
    },
  },

  video: {
    codec: 'vp8',              // 'h264' | 'vp8' | 'vp9' | 'av1'
    width: 1280,
    height: 720,
    fps: 30,
    bitrateKbps: 2000,
  },

  audio: {
    codec: 'opus',             // Phase 1 supports opus only
    sampleRate: 48000,
    channels: 2,
    bitrateKbps: 64,
  },

  dataChannel: {
    labels: ['control', 'telemetry'],
    ordered: {
      'control': true,         // Control commands must be ordered
      'telemetry': false,      // Telemetry tolerates drops
    },
  },
});
```

### Types

```typescript
type SessionType =
  | 'remote_desktop'    // Remote desktop
  | 'teleop_cockpit'    // Teleoperation cockpit
  | 'teleop_vehicle';   // Teleoperation vehicle

> Phase 1 SessionType range: remote_desktop, teleop_cockpit, teleop_vehicle. The remaining 4 capabilities (video conferencing, streaming push/pull, surveillance cameras, video-conference) are Phase 2+ extensions.
type SessionRole =
  | 'host'              // Controlled end (screen-sharing side)
  | 'client'            // Controller (remote desktop client)
  | 'cockpit'           // Cockpit (receives video + sends control)
  | 'vehicle';          // Vehicle (sends video + receives control)

type SessionState =
  | 'idle'              // Not connected
  | 'connecting'        // Connection being established
  | 'connected'         // Connected
  | 'reconnecting'      // Reconnecting after disconnect
  | 'disconnected'      // Disconnected
  | 'failed';           // Connection failed
```

### Lifecycle

```typescript
// Start the connection (blocks until ICE completed or failure)
await session.connect();

// Wait for a specific state (for testing)
await session.waitFor('connected', { timeoutMs: 30000 });

// Graceful shutdown
session.disconnect();
```

### Events

```typescript
interface VideoFrame {
  timestamp: bigint;            // Microseconds
  width: number;
  height: number;
  format: 'rgba';              // Phase 1 supports RGBA only
  data: Buffer;                 // width × height × 4 bytes
}

interface AudioFrame {
  timestamp: bigint;
  sampleRate: number;          // 48000
  channels: number;            // 1 | 2
  format: 'pcm_s16le';         // PCM 16-bit signed little-endian
  data: Buffer;
}

interface DataChannelMessage {
  label: string;               // 'control' | 'telemetry'
  data: Buffer;
}

interface SessionStats {
  rttMs: number;
  packetLossPercent: number;
  bitrateKbps: number;
  framerate: number;
  encodeMs: number;            // Encoding latency
  decodeMs: number;            // Decoding latency
  jitterMs: number;
}

interface SessionError {
  code: string;                // 'SIGNALING_FAILED' | 'ICE_FAILED' | 'DATACHANNEL_ERROR'
  message: string;
  recoverable: boolean;
}

// Register callbacks
session.on('state_change', (state: SessionState) => {});
session.on('remote_video', (frame: VideoFrame) => {});
session.on('remote_audio', (frame: AudioFrame) => {});
session.on('data_channel', (msg: DataChannelMessage) => {});
session.on('stats', (stats: SessionStats) => {});      // Pushed every second
session.on('error', (err: SessionError) => {});
```

### Control

```typescript
// Send a control command (RTCDataChannel)
session.control.send(Buffer.from(JSON.stringify({ steering: 15 })));

// Pull stats on demand
const stats: SessionStats = session.getStats();

// Request a keyframe (after weak-network retransmission)
session.control.requestKeyframe();
```

## 11.4 Rust-Side Implementation

```rust
// mediaservo-common/src/session.rs (new)

use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object)]
pub struct SessionConfig {
  pub session_type: String,
  pub role: String,
  pub transport: String,
  pub signaling_url: String,
  pub token: String,
  pub video_codec: String,
  pub video_width: u32,
  pub video_height: u32,
  pub video_fps: u32,
  pub video_bitrate_kbps: u32,
  pub data_channel_labels: Vec<String>,
}

#[napi]
pub struct Session {
  inner: Mutex<SessionInner>,
}

#[napi]
impl Session {
  #[napi(constructor)]
  pub fn new(config: SessionConfig) -> Result<Self> {
    // 1. Pick the MediaTransport backend based on transport
    // 2. Create the SignalingTransport (WebSocket)
    // 3. Create the pipeline (decode → format convert → callback)
    Ok(Self { inner: Mutex::new(SessionInner::new(config)?) })
  }

  #[napi]
  pub async fn connect(&self) -> Result<()> {
    // 1. WS connect
    // 2. SDP offer/answer via SignalHandler
    // 3. ICE negotiation
    // 4. Start the pipeline
    // 5. emit('state_change', 'connected')
    Ok(())
  }

  #[napi]
  pub fn disconnect(&self) -> Result<()> { /* ... */ }
}
```

## 11.5 Video Frame Pipeline (napi-rs path)

```
Rust: GStreamer pipeline
  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐
  │ appsrc   │ →  │ h264parse│ →  │ decodebin│ →  │ appsink  │
  │ (RTP)    │    │          │    │ (HW dec) │    │ (RGBA)   │
  └──────────┘    └──────────┘    └──────────┘    └────┬─────┘
                                                       │
  appsink callback: GstBuffer → RGBA bytes             │
  napi-rs: Buffer::from(rgba_bytes)                    │
                                                       ▼
  Node.js: emit('remote_video', { data: Buffer })
```

Phase 2 optimization: DMA-BUF sharing (Linux only)
- Rust outputs a DMA-BUF fd → napi-rs passes the fd → the Node.js side imports the GPU texture via an addon
- Avoids the ~8MB/frame RGBA copy

## 11.6 Memory & Performance

| Scenario | Resolution | FPS | RGBA bandwidth | Copy overhead |
|----------|------------|-----|----------------|---------------|
| Remote desktop | 1920×1080 | 30 | ~250 MB/s | 5-10ms (memcpy) |
| Teleop cockpit | 1280×720 | 30 | ~110 MB/s | 2-4ms |
| Teleop cockpit | 1280×720 | 15 | ~55 MB/s | 1-2ms |
| Video conference | 640×480 | 15 | ~18 MB/s | <1ms |

The Phase 1 teleop-cockpit scenario runs at 15fps; the 55MB/s copy overhead is acceptable.
`memcpy` of an 8MB RGBA frame costs ~2-4ms on modern CPUs (DDR4 bandwidth ~20GB/s).

## 11.7 Host-Platform Integration

```typescript
// MediaServo Sessions used inside a host-platform plugin

// plugins/teleop-cockpit/src/index.ts
import { createSession } from '@mediaservo/napi';

export class TeleopCockpitPlugin implements Plugin {
  private session: Session;

  async onLoad(ctx: PluginContext) {
    this.session = createSession({
      type: 'teleop_cockpit',
      role: 'cockpit',
      transport: 'libwebrtc',
      signaling: { url: ctx.config.signalingUrl, token: ctx.auth.token },
      video: { codec: 'vp8', width: 1280, height: 720, fps: 30, bitrateKbps: 2000 },
    });

    this.session.on('remote_video', (frame) => {
      // Render to browser Canvas / WebGL
      ctx.ui.renderVideo(this.videoElementId, frame);
    });

    this.session.on('data_channel', (msg) => {
      if (msg.label === 'telemetry') {
        ctx.emit('telemetry_update', parseTelemetry(msg.data));
      }
    });

    await this.session.connect();
  }

  // The host-platform UI can call this via PluginHost
  sendControl(steering: number, throttle: number) {
    this.session.control.send(Buffer.from(JSON.stringify({ steering, throttle })));
  }
}
```

## 11.8 Phase 1 vs Phase 2

| | Phase 1 | Phase 2 |
|---|---------|---------|
| SessionType | remote_desktop, teleop_cockpit, teleop_vehicle | + video_conference, live_streaming, surveillance_viewer |
| Video format | RGBA Buffer | + DMA-BUF fd (Linux) |
| Audio format | PCM s16le | + Opus pass-through |
| Events | 6 basic events | + track_added/removed (SFU) |
| Control | control.send() | + subscribeTrack() (SFU) |
