# 17. WebRTC Crate — mediaservo-webrtc

> Status: Phase 3 complete | Related decisions: D11, D31, D139–D151 | Reference: webrtc-kit trait abstraction pattern

## Positioning

`mediaservo-webrtc` is MediaServo's WebRTC transport crate. It exposes standard W3C WebRTC API traits externally and dispatches across multiple backends via compile-time feature gates. The current default backend is `webrtc-sys` (libwebrtc C++ FFI).

```
┌─────────────────────────────────────────────────────┐
│                 mediaservo-webrtc                   │
│                                                     │
│  RTCEngine::create_factory()                        │
│    └→ RTCPeerConnectionFactory                      │
│         └→ RTCPeerConnection (pub trait)            │
│                                                     │
│  ┌─────────────────────────────────────────────┐    │
│  │            internal trait layer             │    │
│  │  PcBackend  │  DcBackend  │ TrackWriteBackend│   │
│  └──────────────┼─────────────┼──────────────────┘ │
│                 ▼             ▼                     │
│  ┌─────────────────────────────────────────────┐    │
│  │          compile-time type alias            │    │
│  │  ActivePc / ActiveDc / ActiveTrack          │    │
│  └──────────────┬──────────────────────────────┘ │
│                 ▼                                  │
│  ┌─────────────────────────────────────────────┐    │
│  │    webrtc-sys    │   webrtc-rs   │   stub   │    │
│  │  (libwebrtc C++) │  (pure Rust)  │ (testing)│    │
│  └──────────────────┴───────────────┴───────────┘   │
└─────────────────────────────────────────────────────┘
```

## Naming Convention

All externally visible API types use the uniform `RTC` prefix, aligned with the W3C WebRTC standard:

| Type | Name | W3C Standard |
|------|------|--------------|
| Engine entry | `RTCEngine` | — |
| Factory | `RTCPeerConnectionFactory` | — |
| Connection | `RTCPeerConnection` | `RTCPeerConnection` |
| Data channel | `RTCDataChannel` | `RTCDataChannel` |
| Session description | `RTCSessionDescription` | `RTCSessionDescription` |
| ICE candidate | `RTCIceCandidate` | `RTCIceCandidate` |
| Configuration | `RTCConfiguration` | `RTCConfiguration` |
| RTP sender | `RTCRtpSender` | `RTCRtpSender` |
| RTP receiver | `RTCRtpReceiver` | `RTCRtpReceiver` |
| Error | `RTCError` | — |
| ICE connection state | `RTCIceConnectionState` | `RTCIceConnectionState` |
| ICE gathering state | `RTCIceGatheringState` | `RTCIceGatheringState` |
| Signaling state | `RTCSignalingState` | `RTCSignalingState` |
| Offer options | `RTCOfferOptions` | `RTCOfferOptions` |
| Answer options | `RTCAnswerOptions` | `RTCAnswerOptions` |
| ICE transport policy | `RTCIceTransportPolicy` | `RTCIceTransportPolicy` |
| Stats | `RTCStats` | — |

**Internal types carry no prefix**: `TrackSender`, `TrackReceiver`, `PcBackend`, `DcBackend`, `TrackWriteBackend`. `AudioTrackConfig` and `DataChannelRx` keep their names (no W3C counterpart).

## Trait Hierarchy

Two-layer architecture:

```
┌──────────────────────────────────────────┐
│  pub trait RTCPeerConnection             │  ← external W3C API
│    create_offer / create_answer / close  │
│    add_track / on_track / get_senders... │
├──────────────────────────────────────────┤
│  RTCPeerConnectionImpl (struct)          │  ← shared logic
│    tracks: HashMap<TrackRef>             │    track registry
│    on_track_callback: Arc<Mutex<...>>    │    callback management
│    backend: ActivePc                     │    delegates to backend
├──────────────────────────────────────────┤
│  pub(crate) trait PcBackend              │  ← internal backend trait
│    create_offer / set_local_desc / ...   │
│    set_on_track / register_track / ...   │
├──────────────────────────────────────────┤
│  WebrtcSysPc │ WebrtcRsPc │ StubPc       │
└──────────────────────────────────────────┘
```

- **RTCPeerConnection trait**: the external W3C API; methods use idiomatic Rust snake_case (`create_offer` / `on_track`), aligned with the webrtc-rs/str0m ecosystem
- **RTCPeerConnectionImpl**: track registry, `MAX_TRACKS` validation, callback management — shared by all three backends
- **PcBackend**: `pub(crate)` visibility; SDP/ICE generation and parsing, observer registration — implemented independently per backend

D145 originally specified a Phase 0 RTCPeerConnection struct rather than trait. That decision's scope is the PcBackend internal backend layer — backend implementations use concrete structs to bind directly to webrtc-sys without introducing a backend trait abstraction. Defining the external API layer's `RTCPeerConnection` as a pub trait provides a stable W3C contract — a different layer of design consideration than backend abstraction (revised 2026-07-22).

### Method Naming Principle

All external methods use Rust snake_case (D167); napi-rs converts them automatically to JS camelCase:

```rust
pub trait RTCPeerConnection: Send + Sync {
    async fn create_offer(&self, options: &RTCOfferOptions) -> Result<RTCSessionDescription>;
    async fn create_answer(&self, options: &RTCAnswerOptions) -> Result<RTCSessionDescription>;
    async fn set_local_description(&self, desc: &RTCSessionDescription) -> Result<()>;
    async fn set_remote_description(&self, desc: &RTCSessionDescription) -> Result<()>;
    async fn add_ice_candidate(&self, candidate: &RTCIceCandidate) -> Result<()>;
    async fn create_data_channel(&self, label: &str, init: RTCDataChannelInit) -> Result<RTCDataChannel>;

    fn connection_state(&self) -> RTCPeerConnectionState;
    fn ice_connection_state(&self) -> RTCIceConnectionState;
    fn ice_gathering_state(&self) -> RTCIceGatheringState;
    fn signaling_state(&self) -> RTCSignalingState;

    async fn close(&self);

    // Track management
    fn add_track(&self, track_id: &str, kind: TrackKind) -> Result<RTCRtpSender>;
    fn remove_track(&self, track_id: &str) -> Result<()>;
    fn get_senders(&self) -> Vec<RTCRtpSender>;
    fn get_receivers(&self) -> Vec<RTCRtpReceiver>;
    fn on_track<F>(&self, callback: F) where F: Fn(RTCRtpReceiver) + Send + Sync + 'static;
}
```

## Backend Compile-Time Dispatch

```rust
// backend/mod.rs — one backend compiled at a time, zero runtime overhead
#[cfg(feature = "backend-webrtc-sys")]
pub type ActivePc = webrtc_sys::WebrtcSysPc;

#[cfg(feature = "backend-webrtc-rs")]
pub type ActivePc = webrtc_rs::WebrtcRsPc;

#[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]
pub type ActivePc = stub::StubPc;

// Mutual-exclusion guard — enabling multiple backends at once is a compile error
#[cfg(all(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys"))]
compile_error!("Only one backend can be enabled at a time.");
```

| Backend | Feature | Dependencies | Scenario |
|---------|---------|--------------|----------|
| `webrtc-sys` | `backend-webrtc-sys` (default) | libwebrtc C++ (cmake) | Public-internet remote control, weak-network penetration |
| `webrtc-rs` | `backend-webrtc-rs` | Pure Rust webrtc crate | Future Embed upgrade |
| `stub` | When no feature set | None | Dev/test/compile checks |

`ActivePc` / `ActiveDc` / `ActiveTrack` / `ActiveFactory` work the same way — four type aliases cover every backend dispatch point.

## PcBackend trait

An internal `pub(crate)` trait defining the SDP/ICE operations each backend must implement:

```
pub(crate) trait PcBackend: Send + Sync + 'static {
    // ── Required (10 methods) ──
    async fn create_offer(&self, &RTCOfferOptions) -> Result<RTCSessionDescription>;
    async fn create_answer(&self, &RTCAnswerOptions) -> Result<RTCSessionDescription>;
    async fn set_local_description(&self, &RTCSessionDescription) -> Result<()>;
    async fn set_remote_description(&self, &RTCSessionDescription) -> Result<()>;
    async fn add_ice_candidate(&self, &RTCIceCandidate) -> Result<()>;
    async fn close(&self);
    fn connection_state(&self) -> RTCPeerConnectionState;
    fn ice_connection_state(&self) -> RTCIceConnectionState;
    fn ice_gathering_state(&self) -> RTCIceGatheringState;
    fn signaling_state(&self) -> RTCSignalingState;

    // ── Default / optional (8 methods) ──
    fn set_on_data_channel(&self, cb) {}     // Default no-op
    fn set_on_track(&self, cb) {}            // Default no-op — overridden by RealObserver
    fn set_on_ice_candidate(&self, cb) {}    // Default no-op
    fn gather_complete(&self) -> Result { Ok(()) }
    fn get_stats(&self) -> Vec<RTCRtcStats> { vec![] }
    fn add_transceiver(&self, ...) -> Result { Err("not supported") }
    fn register_track(&self, id, kind) -> Result { Ok(()) }
}
```

### Current Coverage Status

| Method | WebrtcSysPc | WebrtcRsPc | StubPc |
|--------|:-----------:|:----------:|:------:|
| `create_offer` / `create_answer` | ✅ FFI | ✅ webrtc-rs | ✅ empty SDP |
| `set_local_description` / `set_remote_description` | ✅ FFI | ✅ webrtc-rs | ✅ no-op |
| `add_ice_candidate` | ✅ FFI | ✅ webrtc-rs | ✅ no-op |
| `connection_state` and other state methods | ✅ | ✅ | ✅ |
| `close` | ✅ FFI | ✅ | ✅ |
| `set_on_track` | ✅ RealObserver (D173) | ❌ default no-op | ❌ default no-op |
| `set_on_data_channel` | ✅ RealObserver | ❌ default no-op | ❌ default no-op |
| `set_on_ice_candidate` | ✅ RealObserver | ❌ default no-op | ❌ default no-op |

Updated: 2026-07-31 — the webrtc-sys backend implemented the three callbacks via RealObserver (D173); coverage table corrected.

## Frame Send Path

```
TrackSender::write_raw_i420(data, w, h)
  → TrackWriteBackend::write_raw_i420()
    → WebrtcSysTrack (webrtc-sys)
      → I420Buffer::data_y/data_u/data_v (raw pointer fill)
      → VideoFrameBuilder → set_video_frame_buffer → build()
      → VideoTrackSource::on_captured_frame(&frame, &metadata)
    → libwebrtc internal encoding (VP8/H.264)
```

## Frame Receive Path (Phase 0-1 in progress)

```
libwebrtc on_track event
  → RealObserver::on_track(transceiver)
    → transceiver.receiver().track()  → MediaStreamTrack
    → media_to_video(media_track)     → VideoTrack
    → new_native_video_sink(FrameSinkAdapter)
    → video_track.add_sink(&native_sink)
    → VideoSink::on_frame(VideoFrame) → FrameSinkAdapter
    → user FrameSink::on_frame(&VideoFrame)
```

### FrameSink trait

Mirrors the `mediaservo-media::VideoSink<F>` callback pattern:

```rust
pub trait FrameSink: Send + Sync {
    fn on_frame(&self, frame: &VideoFrame) -> Result<(), RTCError>;
    fn is_active(&self) -> bool { true }
}
```

Users may wrap it as a channel / stream themselves:

```rust
struct ChannelSink { tx: mpsc::UnboundedSender<VideoFrame> }
impl FrameSink for ChannelSink {
    fn on_frame(&self, f: &VideoFrame) -> Result<(), RTCError> {
        self.tx.send(f.clone()).ok();
        Ok(())
    }
}
```

### RealObserver replaces NoOpObserver

The current `NoOpObserver` (webrtc_sys.rs:517-602) implements `PeerConnectionObserver` but **all 20+ callbacks are empty**. Phase 0-1 replaces it with `RealObserver`:

```rust
struct RealObserver {
    on_track_cb: Mutex<Option<Box<dyn Fn(TrackReceiver) + Send + Sync + 'static>>>,
    on_data_channel_cb: Mutex<Option<Box<dyn Fn(RTCDataChannel) + Send + Sync + 'static>>>,
    on_ice_candidate_cb: Mutex<Option<Box<dyn Fn(String, i32, String) + Send + Sync + 'static>>>,
}
```

## Factory Entry

```rust
pub struct RTCEngine;
impl RTCEngine {
    pub fn create_factory() -> impl RTCPeerConnectionFactory;
}

pub trait RTCPeerConnectionFactory: Send + Sync {
    async fn create_peer_connection(&self, config: RTCConfiguration)
        -> Result<impl RTCPeerConnection>;
    fn create_video_track(&self, id: &str) -> TrackSender;
    fn create_audio_track(&self, id: &str, config: RTCAudioTrackConfig) -> TrackSender;
}
```

Usage example:

```rust
use mediaservo_webrtc::{RTCEngine, RTCConfiguration};

let factory = RTCEngine::create_factory();
let pc = factory.create_peer_connection(RTCConfiguration::default()).await?;

pc.onTrack(|receiver| {
    println!("remote track: {:?}", receiver.kind);
});
```

## File Structure

```
crates/mediaservo-webrtc/
├── src/
│   ├── lib.rs              re-exports
│   ├── engine.rs           RTCEngine::create_factory()
│   ├── peer.rs             RTCPeerConnection trait + impl
│   ├── channel.rs          RTCDataChannel
│   ├── track.rs            TrackSender / TrackReceiver / FrameSink
│   ├── rtp.rs              RTCRtpSender / RTCRtpReceiver
│   ├── sdp.rs              RTCSessionDescription
│   ├── stats.rs            RTCRtcStats
│   ├── rtp_params.rs       RTP parameter types
│   └── backend/
│       ├── mod.rs          PcBackend / DcBackend trait + type aliases
│       ├── webrtc_sys.rs   WebrtcSysPc + RealObserver
│       ├── webrtc_rs.rs    WebrtcRsPc
│       └── stub.rs         StubPc
├── examples/
│   └── webrtc_loopback_egui.rs  P2P loopback demo
└── tests/
    └── w3c_api_tests.rs    W3C API tests
```

## Related Documents

- [09. Transport architecture & trait design](09-transport-architecture.md) — the `MediaTransport` trait (upper abstraction)
- [10. Signaling architecture](10-signaling-architecture.md) — the signaling layer, calls this crate to create connections
- [14. Remote architecture](14-remote-architecture.md) — Remote Client/Host use this crate
- [15. Component architecture](15-component-architecture.md) — Components communicate through this crate
- [Decision records D144–D151](../../.agents/memorys/decisions.md) — the architecture decision chain

## Current Status

| Capability | Status |
|------------|--------|
| Sender I420 frames → libwebrtc | ✅ Implemented |
| SDP negotiation (createOffer/createAnswer) | ✅ Implemented |
| ICE candidate management | ✅ Implemented |
| RTCDataChannel creation and sending | ✅ Implemented |
| RTCDataChannel event receiving (spool) | ⚠️ stub in webrtc-sys |
| Receiver onTrack callback | ✅ Implemented (RealObserver, D173) |
| Receiver FrameSink frame receipt | ✅ Implemented (VideoSink bridge, D173) |
| webrtc-rs backend | ⚠️ structs exist, trait adaptation pending |
| str0m backend | 🔲 Phase 2+ |
| RTC prefix rename | ✅ Done (D170) |
| backends/ → backend/ directory unification | ✅ Done (D171) |
| core → media module migration | ✅ Done (D169) |
| RawPixelFormat → PixelFormat merge | ✅ Done |
