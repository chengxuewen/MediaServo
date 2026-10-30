# SDK Interface Contract (link / field / client / deck)

> **Status**: link/deck/field implemented (client design finalized) | **Date**: 2026-08-14 (updated 2026-08-17)
> **Related**: D233 (API shape = single-layer session style), D222-D232 (four-SDK architecture), 04-sdk-layers.md, the api-interface-design skill
> **Boundary**: this document is the four SDKs' **public Rust interface contract** + C ABI binding forms. Internal negotiation follows the standard mediasoup offer/answer (C18, against PIT-65).

---

## 0. Settled Decisions (contract basis)

| Decision | Content | Source |
|----------|---------|--------|
| **API shape** | Single-layer session style (`PushSession`/`PullSession`); no fine-grained mediasoup objects exposed; advanced needs go through rich Options; low-level control goes through the mediaservo-webrtc escape hatch | D233 |
| **Event model** | Rust = enum + channel/Stream; C/C++ = callback function pointers | D234 |
| **Branded IDs** | `RoomId/PeerId/TrackId/SessionId/StreamId/NodeId/DeviceId` wrapper types | D234 |
| **Error model** | One thiserror enum per SDK + `#[non_exhaustive]` | D234 |
| **Options style** | Plain struct + `Default` (`..Default::default()`), no builder | D234 |
| **Frame injection** | `write_raw_i420_with_ts` kept as the injection point; deck source objects produce frames to feed it | D234 |
| **async** | The Rust core is fully async (tokio), returning `Result<T, XxxError>` | D234 |

## 1. Unified Calling Conventions

| Convention | Rule |
|------------|------|
| async | All I/O/negotiation methods are `async fn`; local state flips (pause/resume) are sync |
| Errors | `Result<T, XxxError>`; libraries use thiserror; error enums are `#[non_exhaustive]` |
| Parameters | `&str` over `String`; `&[T]` over `Vec<T>`; IDs use branded types |
| Options | Plain struct + `Default`; optional fields are `Option<T>` |
| Events | Rust returns `UnboundedReceiver<Event>` (or a `Stream`); C/C++ register callbacks |
| Ownership | Rust shares internally via `Arc`; sessions/sources are owned objects; C++ uses RAII handles |
| Compatibility | **Additive-only**: new variants append to the enum tail, new fields are `Option<T>`, new methods get default impls; changing/deleting existing signatures is forbidden |

## 2. Shared Types (mediaservo-common / conventions)

> ✅ `SignalingMessage`/`PeerRole`/`RoomId` etc. are already implemented in mediaservo-common; the link SignalClient reuses them.

```rust
// Branded IDs (preventing argument mix-ups) — #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomId(String);
pub struct PeerId(String);
pub struct TrackId(String);
pub struct SessionId(String);
pub struct StreamId(String);
pub struct NodeId(String);
pub struct DeviceId(String);

// Frames (reusing mediaservo-media)
pub struct FrameRef;                 // I420/NV12/RGBA + width/height + timestamps (monotonic/epoch dual clocks)
pub type FrameStream = UnboundedReceiver<FrameRef>;

// Auth (reusing common auth)
#[non_exhaustive]
pub enum AuthCredential { Psk(PskCredential), Jwt(JwtCredential) }

// Signaling messages (reusing common protocol.rs's SignalingMessage, serde tag=type)
```

---

## 3. link — Connection Surface

> **The device-side IPC special topic** (FrameBus bus / Registry registration / ACL permissions / capability tokens) **is implemented** (iceoryx2), see [21-link-ipc.md](21-link-ipc.md). This section covers link's signaling toward the server (SignalClient) and shared types.

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    #[error("attach failed: {0}")]          Attach(String),
    #[error("acl denied: {topic}")]         AclDenied { topic: String },   // D237
    #[error("topic has publisher: {topic}")] TopicConflict { topic: String }, // D239
    #[error("token invalid: {0}")]          Token(String),                  // D238
    #[error("registry error: {0}")]         Registry(String),
    #[error("bus error: {0}")]              Bus(String),
    #[error("closed")]                      Closed,
}

// ── Signaling client (WS, reusing common SignalingMessage) ────────────────
// ✅ Implemented (D244, 2026-08-17): SignalClient/SignalSession + mock WS tests
pub struct SignalClient;
impl SignalClient {
    // Implementation: new(url, psk, room_id, role) + connect(&self) (PSK auth + built-in RoomJoin)
    pub fn new(url: &str, psk: &str, room_id: &str, role: PeerRole) -> SignalClient;
    pub async fn connect(&self) -> Result<SignalSession, LinkError>;
}

pub struct SignalSession;                       // Arc internally
impl SignalSession {
    pub fn events(&self) -> broadcast::Receiver<SignalEvent>; // Implemented as a tokio broadcast
    pub async fn send(&self, msg: SignalingMessage) -> Result<(), LinkError>;
    // request() req/resp not yet implemented (Phase 2; signaling has no request/response pairs)
    pub fn room_id(&self) -> &str;
    pub async fn close(&self);                                   // Implementation: close() forwards on the channel
}

#[non_exhaustive]
pub enum SignalEvent {
    Connected { room_id: RoomId },
    Message(SignalingMessage),
    Disconnected { reason: DisconnectReason },
    Error(LinkError),
}

// ── Frame bus (cross-process SHM, iceoryx2, D242/D239/D235) ────────────────
// buffer_size=1 + enable_safe_overflow → latest-frame; max_publishers(1) → single publisher
pub struct FrameBus;
impl FrameBus {
    pub fn attach(endpoint: &str, token: &CapabilityToken, vk: &Ed25519VerifyingKey)
        -> Result<FrameBus, LinkError>;                       // verify→load ACL→register
    pub fn publish(&self, topic: &FrameTopic, payload: &[u8], meta: &FrameMeta)
        -> Result<(), LinkError>;                              // ACL+single publisher→send
    pub fn subscribe(&self, topic: &FrameTopic) -> Result<FrameStream, LinkError>;
    pub fn close(self) -> Result<(), LinkError>;
}
pub struct FrameTopic(String);
pub struct FrameMeta { seq, width, height, format, version, is_keyframe, ts_mono_ns, ts_epoch_ns } // fixed-length LE, D243

// ── Node registration/discovery (iceoryx2 built-in liveness, no daemon) ───
pub struct Registry;
impl Registry {
    pub fn register(info: &NodeInfo) -> Result<(), LinkError>;
    pub fn discover_topics(prefix: &str) -> Result<Vec<TopicInfo>, LinkError>;
    pub fn discover_nodes(role: Role) -> Result<Vec<NodeInfo>, LinkError>;
    pub fn topic_publisher(topic: &FrameTopic) -> Result<Option<NodeId>, LinkError>;
}

// ── DataChannel (Phase 2, webrtc-rs) ─────────────────────
// pub struct ControlChannel;  // finalized in Phase 2
```

---

## 4. field — Composed SDK (session facade, the only public layer)

> 🟡 **MVP implemented** (2026-08-17, 4 tests): error + re-export closed loop + session type skeletons.
> Deltas: ① `PushSession`/`PullSession` `connect` explicitly reports "Phase 2 not implemented" (no silence —
> callers must not think it already works); ② media methods like `publish_video/audio`, `subscribe`,
> `control_channel`, `stats` are not implemented — they need the real webrtc-sys transport (mind Linux
> builds); ③ config types `PushConfig/PullConfig` have not landed (connect has a no-arg placeholder).

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FieldError {
    #[error("link: {0}")]      Link(#[from] LinkError),
    #[error("webrtc: {0}")]    WebRtc(String),
    #[error("codec: {0}")]     Codec(String),
    #[error("track {0}: {1}")] Track(TrackId, String),
    #[error("closed")]         Closed,
}

// ── Push session (capture→encode→push, signaling built in) ────────────────
pub struct PushSession;
impl PushSession {
    pub async fn connect(cfg: PushConfig) -> Result<(PushSession, SessionEvents), FieldError>;
    pub async fn publish_video(&self, src: VideoSourceRef, opts: PublishOptions) -> Result<TrackId, FieldError>;
    pub async fn publish_audio(&self, src: AudioSourceRef, opts: PublishOptions) -> Result<TrackId, FieldError>;
    pub async fn unpublish(&self, track: TrackId) -> Result<(), FieldError>;
    pub fn control_channel(&self) -> Result<ControlChannel, FieldError>;  // Control/telemetry (DC or relay)
    pub async fn stats(&self) -> Result<SessionStats, FieldError>;
    pub async fn close(self) -> Result<(), FieldError>;
}

// ── Pull session (subscribe→decode→frame output) ──────────────────────────
pub struct PullSession;
impl PullSession {
    pub async fn connect(cfg: PullConfig) -> Result<(PullSession, SessionEvents), FieldError>;
    pub async fn subscribe(&self, producer: ProducerRef, opts: SubscribeOptions) -> Result<FrameStream, FieldError>;
    pub async fn unsubscribe(&self, track: TrackId) -> Result<(), FieldError>;
    pub fn control_channel(&self) -> Result<ControlChannel, FieldError>;  // Send control
    pub async fn close(self) -> Result<(), FieldError>;
}

pub type SessionEvents = UnboundedReceiver<SessionEvent>;
#[non_exhaustive]
pub enum SessionEvent {
    Connected,
    StateChanged(SessionState),
    TrackPublished { track: TrackId },
    TrackSubscribed { track: TrackId },
    ProducerAvailable { producer: ProducerRef },     // For subscribe
    Disconnected { reason: DisconnectReason },
    Error(FieldError),
}

// ── Config / rich Options (absorbing advanced needs, D233) ────────────────
pub struct PushConfig {
    pub url: String,
    pub auth: AuthCredential,
    pub room: RoomId,
    pub role: PeerRole,                              // Host/Pusher
}
pub struct PullConfig {
    pub url: String,
    pub auth: AuthCredential,
    pub room: RoomId,
    pub role: PeerRole,                              // Client/Puller
    pub auto_subscribe: bool,
}
impl Default for PullConfig { /* auto_subscribe = true */ }

pub struct PublishOptions {
    pub codec: VideoCodec,                           // VP8/H264/VP9/AV1
    pub encoder_backend: EncoderBackend,             // Auto/Software/Hardware
    pub encoding: Option<VideoEncoding>,             // bitrate/max_framerate/resolution
    pub simulcast: bool,
    pub degradation: Option<DegradationPreference>,
}
impl Default for PublishOptions { /* codec=VP8, backend=Auto, simulcast=false */ }

pub struct SubscribeOptions { pub priority: u8, pub max_resolution: Option<Resolution> }
impl Default for SubscribeOptions { /* ... */ }

// ── Composed re-exports (one-line dependency closed loop) ──────────────────
pub use mediaservo_link::{SignalClient, FrameBus, AuthCredential};
pub use mediaservo_deck::{MediaDevices, CameraSource, AudioSource, ScreenSource, VideoSource};
```

> **Escape hatch**: those needing low-level control (SDP/transceivers/fine-grained RTP) should depend directly on `mediaservo-webrtc` (RTCPeerConnection/TrackSender/PeerConnectionApi); field does not re-expose it.

---

## 5. client — Cockpit-Side Consumption Orchestration

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    #[error("field: {0}")]   Field(#[from] FieldError),
    #[error("render: {0}")]  Render(String),
    #[error("playback: {0}")] Playback(String),
}

// ── Renderer (D47: CPU buffer → GPU interop) ────────────────
pub struct Renderer;
impl Renderer {
    pub fn new(surface: Surface, opts: RenderOptions) -> Result<Renderer, ClientError>;
    pub fn render(&self, frame: FrameRef) -> Result<(), ClientError>;
    pub fn set_surface(&self, surface: Surface) -> Result<(), ClientError>;
}
pub struct RenderOptions { pub backend: RenderBackend /* Auto/Cpu/GpuDirect */, pub zero_copy: bool }
impl Default for RenderOptions { /* backend=Auto */ }

// ── Multi-stream orchestration: reuse field::PullSession (one each) + Renderer ──
// A cockpit app = N × PullSession.subscribe() → FrameStream → Renderer.render()

// ── Playback / fast-forward (deck playback integration) ────
pub struct Playback;
impl Playback {
    pub fn open(path: &str) -> Result<Playback, ClientError>;
    pub fn frames(&self) -> Result<FrameStream, ClientError>;
    pub fn play(&self) -> Result<(), ClientError>;
    pub fn pause(&self) -> Result<(), ClientError>;
    pub fn seek(&self, ts_us: i64) -> Result<(), ClientError>;
    pub fn set_rate(&self, rate: f32) -> Result<(), ClientError>;   // Fast/slow playback
}
```

---

## 6. deck — Media Data Surface

> ✅ **Implemented** (source/record/playback domains, 2026-08-17, 10 tests + ffprobe evidence).
> Deltas: ① methods are async (`recv()`/`record()` etc.); ② `FrameStream` is deck's own type
> (bounded channel with latest-frame dropping, not link's FrameStream); ③ `AudioSource`/`ScreenSource`
> not implemented (MVP has only the CameraSource stub); ④ `Recorder::record` consumes `impl Frames`
> rather than a concrete FrameStream type parameter; ⑤ `Player::next_frame()` is synchronous, frame by
> frame; ⑥ `SeekOptions`/`set_rate` not implemented (Phase 3).

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeckError {
    #[error("device: {0}")]   Device(String),
    #[error("codec: {0}")]    Codec(String),
    #[error("io: {0}")]       Io(#[from] std::io::Error),
    #[error("not_found: {0}")] NotFound(String),
}

// ── Capture (source domain, GStreamer backend) ───────────────
pub struct MediaDevices;
impl MediaDevices {
    pub fn enumerate(kind: DeviceKind) -> Vec<DeviceId>;   // Camera/Audio/Screen
}

pub struct CameraSource;
impl CameraSource {
    pub fn open(dev: DeviceId, opts: CaptureOptions) -> Result<CameraSource, DeckError>;
    pub fn frames(&self) -> Result<FrameStream, DeckError>;
    pub fn start(&self) -> Result<(), DeckError>;
    pub fn stop(&self) -> Result<(), DeckError>;
}
pub struct AudioSource;   // Same shape: open/frames/start/stop
pub struct ScreenSource;  // Same shape

pub struct CaptureOptions { pub resolution: Option<Resolution>, pub framerate: Option<u32>, pub format: Option<PixelFormat> }
impl Default for CaptureOptions { /* device defaults */ }

// VideoSource: the frame-injection abstraction (input type of field publish_video)
pub struct VideoSource;   // A unified handle over CameraSource/files/custom frame sources
// Frames produced internally are injected via mediaservo-webrtc TrackSender::write_raw_i420_with_ts

// ── Recording (record domain, FFmpeg mux) ────────────────────
pub struct Recorder;
impl Recorder {
    pub fn new(path: &str, opts: RecordOptions) -> Result<Recorder, DeckError>;
    pub fn record(&self, stream: FrameStream) -> Result<(), DeckError>;   // or record_topic(FrameTopic)
    pub fn stop(self) -> Result<(), DeckError>;
}
pub struct RecordOptions { pub codec: VideoCodec, pub container: Container /* Mp4/Mkv */, pub keyframe_align: bool }

// ── Playback (playback domain, demux + decode) ───────────────
pub struct Player;
impl Player {
    pub fn open(path: &str) -> Result<Player, DeckError>;
    pub fn frames(&self) -> Result<FrameStream, DeckError>;
    pub fn seek(&self, ts_us: i64) -> Result<(), DeckError>;
    pub fn set_rate(&self, rate: f32) -> Result<(), DeckError>;
}

// ── codec (reusing the mediaservo-codec engine; facade, no merger) ──
pub use mediaservo_codec::{Encoder, Decoder};   // D229: deck depends on codec, no merger
```

---

## 7. C ABI Binding Forms (c / cxx / py)

Continuing D109: **opaque handles + int error codes + callbacks**. One `mediaservo_<sdk>_*` prefix per SDK.

```c
/* Example: link signaling */
typedef struct mediaservo_link_signal_t mediaservo_link_signal_t;   /* opaque */
typedef int mediaservo_err_t;                                /* 0 = ok, <0 = error code */

mediaservo_err_t mediaservo_link_signal_connect(const char* url, const mediaservo_auth_t* auth, mediaservo_link_signal_t** out);
mediaservo_err_t mediaservo_link_signal_send(mediaservo_link_signal_t* s, const uint8_t* msg, size_t len);
void     mediaservo_link_signal_on_event(mediaservo_link_signal_t* s, mediaservo_signal_event_cb cb, void* user);  /* event callback */
mediaservo_err_t mediaservo_link_signal_close(mediaservo_link_signal_t* s);
mediaservo_err_t mediaservo_last_error(char* buf, size_t len);       /* details of the most recent error */

/* field push (async → handle + completion callback, or blocking; the binding layer picks one) */
typedef struct mediaservo_field_push_t mediaservo_field_push_t;
mediaservo_err_t mediaservo_field_push_connect(const mediaservo_push_config_t* cfg, mediaservo_field_push_t** out);
mediaservo_err_t mediaservo_field_push_publish_video(mediaservo_field_push_t* s, mediaservo_video_source_t* src,
                                     const mediaservo_publish_options_t* opts, mediaservo_track_id_t* out_track);
```

- **C++ (`-cxx`)**: header-only RAII wrapping the C ABI (`FfiHandle`-style destructor + `Result<T,E>`)
- **Python (`-py`)**: first version loads the cdylib via ctypes (LiveKit pattern); a pyo3 acceleration backend when bottlenecked (D227 two-step plan)
- **Events**: C-side callback function pointers + user data; async operations use a handle + completion callback
- **Frames**: frames cross FFI as pointers + a metadata struct (zero-copy preferred), avoiding serialization

### 7.1 Delivery Form (D240)
**Single dynamic library (.so/cdylib) as the main form**; no pre-built static .a (added later, additively, when embedded self-contained needs appear).
- Deliverables: `link.so` / `field.so` (packaging link+deck) / `client.so` / `deck.so` (+ the `deck-full.so` OTA plugin)
- C++ (-cxx): header-only RAII over the `.so`; Python (-py): ctypes loads the `.so`; Rust internal use goes through the rlib

### 7.2 Versioning & C ABI Stability (D241)
- **soname**: `libmediaservo_<sdk>.so.<MAJOR>` (e.g. `libmediaservo_field.so.1`); the real file `.so.<MAJOR>.<MINOR>.<PATCH>`; dev symlinks unversioned
- **MAJOR = the C ABI version**: bump MAJOR only for breaking ABI changes; MINOR/PATCH = additive/fix
- **Within-MAJOR stability promise**: additive-only (existing signatures/semantics unchanged) + opaque handles hiding internal layout + C-compatible types only; structures needing evolution use version/size fields or `_v2` functions
- **Compatibility rules**: within a MAJOR, binary-compatible (swap the .so without relinking); across MAJORs, relink + a migration guide
- **cbindgen discipline**: export the stable C ABI only; the Rust internal API may evolve via semver while the C ABI surface stays stable within a MAJOR

## 8. Backward-Compatibility Discipline (Hyrum's Law)

| Rule | Enforcement |
|------|-------------|
| Public enums all `#[non_exhaustive]` | `SignalEvent/SessionEvent/*Error` |
| New variants append at the tail; new fields `Option<T>` | Changing/deleting existing variants/fields forbidden |
| New methods prefer default implementations | Trait evolution doesn't break implementors |
| Branded IDs | Prevents `RoomId`↔`TrackId` argument mix-ups |
| E2E pins behavior | Observable behaviors like message order/retry intervals go into E2E scripts |
| Checks | `cargo doc --no-deps`, serde roundtrip tests, `cargo tree -d` empty |
