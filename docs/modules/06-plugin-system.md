# Plugin System & Trait Design

> Phase 0 architecture definition — plugin trait design, PluginManager, capability declarations. References OBS's obs_source_info + GStreamer's element factory.

---

## 1. Microkernel Architecture

```
mediaservo-common (microkernel)
├── PluginManager     — plugin registration, discovery, lifecycle
├── LicenseManager    — permission validation, quota control
├── ProtocolBroker    — internal protocol routing (FlatBuffers)
├── PipelineEngine    — media pipeline scheduling
└── AuthProvider      — authentication interface (trait)
```

The microkernel contains only trait definitions and scheduling logic. All concrete functionality lives in plugins.

---

## 2. Plugin trait (D28)

### 2.1 Plugin Registration

```rust
/// A plugin = a registrable pipeline-node factory.
/// OBS's obs_source_info pattern: struct-of-callbacks registration.

pub trait Plugin: Send + Sync {
    /// Plugin name
    fn name(&self) -> &str;
    /// Semantic version
    fn version(&self) -> (u16, u16, u16);
    /// Plugin category: MediaSource | MediaProcessor | MediaSink (aligned with architecture.md §4.2)
    fn category(&self) -> PluginCategory;
    /// Capability declaration: declared at registration per D30, not runtime probed
    fn capabilities(&self) -> Vec<PluginCapability>;

    /// Initialization: Phase 0 compile-time, Phase 2+ dlopen
    fn init(&self, ctx: &PluginContext) -> Result<()>;
    /// Shutdown/cleanup
    fn shutdown(&self) -> Result<()>;
}
```

### 2.2 Capability Declaration (D30)

```rust
/// Declared at registration, not runtime probed. OBS's output_flags pattern.

pub struct PluginCapability {
    pub node_type: NodeType,
    pub media_type: MediaType,
    pub codecs: Vec<CodecId>,        // Supported codec list
                                      // Empty Vec = wildcard (passthrough nodes)
    pub pixel_formats: Vec<RawPixelFormat>,
    pub priority: u8,                // Selection priority (0-255, higher wins)
}

pub enum NodeType { Source, Processor, Sink }
pub enum MediaType { Encoded, Raw, Both }
```

### 2.3 Plugin Classification

#### Production (Host — media source output)

| Plugin | Type | Input | Output | Plugin Kind |
|--------|------|-------|--------|-------------|
| ScreenCapture | Source | — | RawFrame | Compile-time |
| CameraCapture | Source | — | RawFrame | Compile-time |
| AudioCapture | Source | — | RawFrame (audio) | Compile-time |
| RtmpSubscriber | Source | — | EncodedFragment | Compile-time |
| RTCRtpReceiver | Source | — | EncodedFragment | Compile-time |
| InputReceiver | Sink | InputEvent | — | Runtime |

#### Processing (Processor — media transforms)

| Plugin | Input | Output | Plugin Kind | Priority |
|--------|-------|--------|-------------|----------|
| NvencEncoder | RawFrame | EncodedFragment | Compile-time | 255 (GPU first) |
| VaapiEncoder | RawFrame | EncodedFragment | Compile-time | 200 |
| VideoToolboxEncoder | RawFrame | EncodedFragment | Compile-time | 200 |
| QsvEncoder | RawFrame | EncodedFragment | Compile-time | 200 |
| SoftwareH264 | RawFrame | EncodedFragment | Compile-time | 50 (fallback) |
| HwDecoder | EncodedFragment | RawFrame | Compile-time | 200 |
| SoftwareDecoder | EncodedFragment | RawFrame | Runtime | 50 |
| ColorConvert | RawFrame | RawFrame | Compile-time | — |
| Passthrough | EncodedFragment | EncodedFragment | Compile-time | — |

#### Consumption (Sink — media consumption)

| Plugin | Input | Output | Plugin Kind |
|--------|-------|--------|-------------|
| ScreenRender | RawFrame | — | Compile-time |
| AudioPlayback | RawFrame (audio) | — | Compile-time |
| RtmpPublisher | EncodedFragment | — | Compile-time |
| RTCRtpSender | EncodedFragment | — | Compile-time |
| Recorder | EncodedFragment | — | Runtime |
| InputForwarder | — | InputEvent | Compile-time |

#### Protocol

| Plugin | Description |
|--------|-------------|
| RtmpPlugin | RTMP ingest/distribution (GStreamer) |
| HlsPlugin | HLS packaging (GStreamer + Rust CMAF) |
| SrtPlugin | SRT transport (GStreamer srtp) |
| RtspPlugin | RTSP ingest (GStreamer) |
| WebRtcPlugin | WebRTC P2P/SFU (str0m / libwebrtc / webrtc-rs) |
| DataChannelPlugin | WebRTC RTCDataChannel |

#### Relay

| Plugin | Description |
|--------|-------------|
| StunTurnPlugin | NAT traversal (coturn integration) |
| SfuRelayPlugin | SFU media forwarding (LiveKit / mediasoup plugins) |

---

## 3. PluginManager (D29)

### 3.1 Dual-Mode Loading

```rust
pub struct PluginManager {
    compile_time: Vec<Arc<dyn Plugin>>,    // compile-time registrations
    run_time: Vec<Arc<DynamicPlugin>>,      // runtime loads
}

impl PluginManager {
    /// Registers compile-time plugins (generated in build.rs via inventory).
    pub fn register(&mut self, plugin: Arc<dyn Plugin>);

    /// Loads runtime plugins from a plugin directory (dlopen).
    pub fn load_runtime(&mut self, path: &Path) -> Result<()>;

    /// Queries plugins by capability. PipelineEngine uses this to find matching nodes.
    pub fn find_nodes(
        &self, node_type: NodeType, media_type: MediaType, query: &FormatQuery,
    ) -> Vec<&dyn Plugin>;

    /// Creates pipeline node instances from matching plugins.
    pub fn create_node(
        &self, capability: &PluginCapability, config: &NodeConfig,
    ) -> Result<Box<dyn AnyPipelineNode>>;
}
```

### 3.2 Compile-Time Registration (inventory pattern)

```rust
// Auto-registered at compile time via a Rust macro

inventory::submit! {
    PluginRegistry::new(
        name: "nv-encoder",
        version: (0, 1, 0),
        kind: PluginKind::CompileTime,
        capability: PluginCapability {
            node_type: NodeType::Processor,
            media_type: MediaType::Raw,
            codecs: vec!["h264".into(), "h265".into()],
            pixel_formats: vec![RawPixelFormat::Nv12],
            priority: 255,
        },
        factory: || Box::new(NvencEncoder::new()),
    )
}
```

### 3.3 Runtime Loading (dlopen pattern)

```rust
// Dynamic plugins export C ABI functions

#[no_mangle]
pub extern "C" fn mediaservo_plugin_register(registry: &mut PluginManager) {
    registry.register(Arc::new(MyCustomDecoder));
}

// ABI stability guaranteed via sizeof checks (see OBS's obs_register_source_s)
```

### 3.4 Pipeline Engine Selection Flow

```
PipelineEngine::build_pipeline(sources, sinks)
  │
  ├── PluginManager::find_nodes(NodeType::Processor, MediaType::Raw, "h264")
  │     → [NvencEncoder(255), VaapiEncoder(200), SoftwareH264(50)]
  │
  ├── Sort by priority → pick NvencEncoder
  │
  └── PluginManager::create_node(nvenc, config)
        → Box<dyn MediaProcessor<RawFrame, EncodedFragment>>
        → insert into the pipeline
```

---

## 4. Plugin Lifecycle

```
CompileTime:  inventory::submit! → compile-time static registration → PluginManager::register()
  │
  ├── trait Plugin::on_load()   → init global resources (GPU contexts, decoder pools)
  ├── [pipeline runs]           → create_node() instantiates on demand
  ├── trait Plugin::on_unload() → clean up global resources
  └── Plugin dropped

RunTime:      dlopen → extern "C" register() → PluginManager::register()
  │
  ├── [same as above]
  └── dlclose
```

---

## 5. Capability Declaration vs Runtime Negotiation

| Dimension | MediaServo (D30) | GStreamer | OBS |
|-----------|------------------|-----------|-----|
| Declaration timing | Compile-time registration | Plugin init + PadTemplate | Module load |
| Format matching | Direct comparison (codec strings + pixel formats) | Caps intersection + fixate | output_flags bitmask |
| Dynamic negotiation | Phase 2+ optional (like GST_QUERY_CAPS) | Native (GST_QUERY_CAPS → caps filter → fixate) | Limited (video_get_color_space) |
| ABI safety | sizeof checks (OBS pattern) | GObject type system | sizeof checks |

**Phase 1 rationale**: declarative capabilities suffice for every planned plugin. GStreamer's complex caps negotiation is over-engineering for Phase 1's fixed-format chains.
