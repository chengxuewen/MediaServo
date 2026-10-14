# Pipeline Model & Trait Design

> Phase 0 architecture definition — detailed trait design and the pipeline model. References the LVQR Unified Fragment Model + GStreamer's state machine + OBS's callback pattern.

---

## 1. Core Types (native-core — zero dependency)

### 1.1 Public API Types (SDK layer — type-safe)

```rust
// ── Raw video frames (screen capture output, renderer consumption) ────────────────

pub struct RawFrame {
    pub track_id: String,
    pub timing: FrameTiming,
    pub format: RawPixelFormat,        // NV12, I420, BGRA, P010
    pub planes: Vec<Plane>,
    pub texture: Option<TextureHandle>, // GPU path handle
}

// ── Encoded fragments (encoder output, protocol adapter/recorder consumption) ────

pub struct EncodedFragment {
    pub track_id: String,
    pub timing: FrameTiming,
    pub flags: FragmentFlags,
    pub codec: CodecId,                // RFC 6381 string
    pub init_data: Option<Bytes>,      // SPS/PPS, vorbis headers
    pub payload: Bytes,                // NAL, fMP4 moof+mdat, Opus packets
}

pub struct FrameTiming {
    pub dts: u64,                      // track timebase units
    pub pts: u64,
    pub duration: u64,
    pub wall_clock: Option<Instant>,   // NTP wall clock (optional)
}

pub struct FragmentFlags {
    pub keyframe: bool,
    pub independent: bool,
    pub discardable: bool,             // D22: SFU per-frame QoS + partial HLS segments
}

pub enum RawPixelFormat { Nv12, I420, Bgra, P010 }
```

### 1.2 Internal Pipeline Types (engine layer — unified path)

```rust
/// Internal unified enum — the pipeline engine's routing unit.
/// GStreamer's GstBuffer + GstMapInfo model.

pub enum InternalPacket {
    Encoded(EncodedFragment),
    Raw(RawFrame),
    Metadata(PacketMetadata),
}

pub struct PacketMetadata {
    pub track_id: String,
    pub event: MetadataEvent,
}

pub enum MetadataEvent {
    TrackStarted { codec: CodecId, timescale: u32 },
    TrackEnded,
    QualityChanged { target_bitrate: u32 },
}
```

### 1.3 TextureHandle Borrow Model (D20)

```rust
pub struct TextureHandle { /* internal Arc<InnerTexture> */ }

impl TextureHandle {
    pub fn map_readable(&self) -> Result<TextureView>;
    pub fn map_writable(&mut self) -> Result<TextureView>;
}
// Multiple consumers may map_readable concurrently; map_writable is exclusive
// See GStreamer's GstBuffer + gst_buffer_ref
```

### 1.4 Design Basis

| Type | Reference Project | Reason |
|------|-------------------|--------|
| RawFrame + EncodedFragment | OBS (obs_source_frame + encoder_packet) | Type safety: SDK users cannot confuse encoded frames with raw frames |
| InternalPacket | GStreamer (GstBuffer), LVQR (Fragment) | Unified pipeline path: engine-internal routing avoids duplicated code |
| FrameTiming (hybrid NTP) | MediaMTX + GStreamer | Precise lip sync for push streams; graceful fallback for screen capture |
| TextureHandle (borrow) | GStreamer (gst_buffer_ref) | Supports forked pipelines (render+encode simultaneously), supports write paths |

---

## 2. Pipeline Node Hierarchy

### 2.1 Node Base trait (D23)

```rust
/// Base of all pipeline nodes. Declares metadata and capabilities.

trait NodeInfo: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> NodeCapability;
}

/// Lifecycle of runnable nodes (GStreamer's change_state model, simplified to two states in Phase 1).

trait PipelineNode: NodeInfo {
    fn on_start(&mut self) -> Result<()>;   // Allocate resources, activate I/O
    fn on_stop(&mut self) -> Result<()>;    // Release resources, deactivate I/O
}

pub struct NodeCapability {
    pub input: FormatSpec,     // Input format requirement
    pub output: FormatSpec,    // Output format declaration
}

pub struct FormatSpec {
    pub media_type: MediaType,         // Encoded, Raw, Both
    pub codecs: Option<Vec<CodecId>>,   // None = wildcard
    pub pixel_formats: Vec<RawPixelFormat>,
}

pub enum MediaType { Encoded, Raw, Both }
```

### 2.2 Three Pipeline Roles (D24)

```rust
/// ── Producer: produces media data (cameras, screens, network sources) ──

trait MediaSource: PipelineNode {
    type Output;

    /// Polls the next data unit. None = no data right now (non-blocking).
    /// LVQR's FragmentStream::next_fragment pattern, adapted to str0m's poll_output.

    fn poll_fragment(&mut self) -> Result<Option<Self::Output>>;
}

/// ── Processor: transforms media data (encode, decode, color convert) ──

trait MediaProcessor: PipelineNode {
    type Input;
    type Output;

    /// Synchronous transform. CPU/GPU work — may internally spawn onto a separate thread.
    /// GStreamer's BaseTransformImpl::transform pattern.

    fn process(&mut self, input: Self::Input) -> Result<Self::Output>;
}

/// ── Sink: consumes media data (render, push, record) ──

trait MediaSink: PipelineNode {
    type Input;

    /// Push-style consumption. OBS's filter_video / encoder_packet callback pattern.

    fn on_fragment(&mut self, fragment: Self::Input) -> Result<()>;
}
```

### 2.3 async Model (D27)

| Role | Pattern | Reason |
|------|---------|--------|
| `poll_fragment` | async | Waits on external data (network, camera) — futures-based |
| `process` | sync | CPU/GPU transform — avoids async overhead and Pin<Box<Future>> complexity |
| `on_fragment` | sync | Push-style consumption — no futures needed |

For CPU-intensive execution, nodes may optionally offload via `tokio::spawn` onto a separate thread.

---

## 3. Fan-Out Mechanism (D25)

### 3.1 FragmentBroadcaster

```rust
/// Single-producer, multi-subscriber broadcaster.
/// Based on tokio::sync::broadcast (LVQR pattern).

pub struct FragmentBroadcaster<P> {
    tx: broadcast::Sender<P>,
    meta: BroadcasterMeta,
}

impl<P: Clone + Send + 'static> FragmentBroadcaster<P> {
    /// Publishes a packet. Returns the current subscriber count. Producers never block.
    pub fn emit(&self, packet: P) -> usize;

    /// Creates a new subscriber. The returned stream can be chained into a pipeline as a MediaSource.
    pub fn subscribe(&self) -> BroadcastStream<P>;

    /// Returns broadcaster metadata.
    pub fn meta(&self) -> &BroadcasterMeta;
}

/// Subscriber stream — implements the MediaSource trait.

impl<P> MediaSource for BroadcastStream<P> {
    type Output = P;

    fn poll_fragment(&mut self) -> Result<Option<P>> {
        // tokio::sync::broadcast::Receiver::recv()
        // RecvError::Lagged → skip + log (LVQR backpressure policy)
    }
}
```

**Key constraints**:
- Default buffer: 1024 packets (~1-2 seconds at 60fps)
- Slow consumers get `RecvError::Lagged` and skip — publishers never block
- Subscribers are MediaSources — chainable into pipeline chains

**Note — backpressure policy**: Phase 1 uses simple FIFO skip-on-overflow (drop old packets when the broadcast channel fills). Phase 2 introduces backpressure from encoder to capture source (D-OPS-03): when downstream encoding or transport rate falls short, upstream is told to drop frames or reduce quality.
### 3.2 PipelineRegistry (D26)

```rust
/// The central pipeline registry. Manages dynamic source add/remove.

pub struct PipelineRegistry {
    broadcasters: RwLock<HashMap<(String, String), Arc<dyn AnyBroadcaster>>>,
    on_created: RwLock<Vec<Box<dyn Fn(&RegistryEvent) + Send + Sync>>>,
    on_removed: RwLock<Vec<Box<dyn Fn(&RegistryEvent) + Send + Sync>>>,
}

impl PipelineRegistry {
    /// Gets or creates a broadcaster. Called by producers (RTMP sources, cameras).
    pub fn get_or_create(
        &self, source_id: &str, track_id: &str, meta: BroadcasterMeta,
    ) -> Arc<FragmentBroadcaster>;

    /// Removes a broadcaster. Called when a producer disconnects.
    pub fn remove(&self, source_id: &str, track_id: &str);

    /// Lifecycle hooks — fired outside the lock (critical: consumers can safely subscribe inside callbacks without deadlock).
    /// LVQR's FragmentBroadcasterRegistry::on_entry_created/removed pattern.

    pub fn on_source_added<F>(&self, cb: F)
        where F: Fn(&RegistryEvent) + Send + Sync + 'static;

    pub fn on_source_removed<F>(&self, cb: F)
        where F: Fn(&RegistryEvent) + Send + Sync + 'static;
}
```

---

## 4. Pipeline Composition Examples

### 4.1 Remote-Cockpit Streaming (Phase 1 milestone)

```
┌─────────────────────────────────────────────────────────────┐
│  Vehicle side                                               │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐   │
│  │ CameraCapture│───▶│ NvencEncoder │───▶│ WebRtcPush   │   │
│  │ MediaSource  │    │ MediaProc    │    │ MediaSink    │   │
│  │ <RawFrame>   │    │ Raw→Encoded  │    │ <EncodedFrag>│   │
│  └──────────────┘    └──────────────┘    └──────────────┘   │
│                                                             │
│  InternalPacket::Raw    InternalPacket::Encoded             │
│                                                             │
│  Cockpit side                                               │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐   │
│  │ RtmpSubscribe│───▶│ VtDecoder    │───▶│ ScreenRender │   │
│  │ MediaSource  │    │ MediaProc    │    │ MediaSink    │   │
│  │<EncodedFrag> │    │ Encoded→Raw  │    │ <RawFrame>   │   │
│  └──────────────┘    └──────────────┘    └──────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### 4.2 Fan-Out Example: render + encode + record simultaneously

```
ScreenCapture ─── FragmentBroadcaster<RawFrame>
                      ├── ScreenRender (MediaSink<RawFrame>)
                      ├── NvencEncoder (MediaProcessor<RawFrame, EncodedFragment>)
                      │       └── RtmpPublisher (MediaSink<EncodedFragment>)
                      └── Recorder (MediaSink<RawFrame>)
```

### 4.3 Type Boundaries

Encoders and decoders are transformation nodes of the InternalPacket types:

| Node | Input | Output | Notes |
|------|-------|--------|-------|
| NvencEncoder | RawFrame | EncodedFragment | GPU zero-copy encode |
| VaapiEncoder | RawFrame | EncodedFragment | Linux hardware encode |
| VtDecoder | EncodedFragment | RawFrame | macOS hardware decode |
| SoftwareH264 | RawFrame | EncodedFragment | Software encode (compatibility fallback) |
| Passthrough | EncodedFragment | EncodedFragment | SFU passthrough (zero-copy) |
| ColorConvert | RawFrame | RawFrame | Color space conversion |

---

## 5. GStreamer + Rust Hybrid Pipeline (D6, D19)

| Layer | Technology | Phase |
|-------|------------|-------|
| **Protocol adapters** | GStreamer (RTMP/RTSP/SRT parsing) | Phase 1 |
| **Codec** | GStreamer + Rust NVENC/VAAPI bridges | Phase 1 |
| **HLS/DASH packaging** | GStreamer + Rust improvements | Phase 2+ |
| **WebRTC core** | str0m / libwebrtc / webrtc-rs | Phase 1 |
| **GPU encode bridge** | libloading calling NVENC/VAAPI/VT directly | Phase 1 |
| **Input injection pipeline** | Pure Rust | Phase 1 |

Phase 2+ goal: demote GStreamer to a pluggable protocol adapter; the core pipeline fully Rust.

---

## 6. Reference Project Mapping

| MediaServo Concept | LVQR | GStreamer | OBS | MediaMTX |
|--------------------|------|-----------|-----|----------|
| InternalPacket | Fragment | GstBuffer | — | unit.Unit |
| FragmentBroadcaster | FragmentBroadcaster (broadcast::Sender) | tee element | — | — |
| PipelineRegistry | FragmentBroadcasterRegistry | — | obs_load_all_modules2 | path struct |
| MediaSource | FragmentStream | BaseSrc | obs_source_info (activated) | Source interface |
| MediaProcessor | Transcoder, Agent | BaseTransform | obs_source_info (filter_video) | — |
| MediaSink | — (observer pattern) | BaseSink | obs_output_info | Reader interface |
| PluginCapability | — (header fields) | PadTemplate + Caps | output_flags | — |
