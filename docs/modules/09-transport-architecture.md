# Transport Architecture & Trait Design


> Phase 0 architecture definition — the MediaTransport trait, dual backends + test stub dispatch, sans-I/O run loop. References webrtc-kit's trait abstraction + str0m's sans-I/O design. Default backend is webrtc-sys (libwebrtc C++ FFI wrapper). str0m deferred to Phase 2+ (Embed scenario).

**Naming convention**: this document describes the sans-I/O internal transport layer, using Rust snake_case conventions for methods (e.g. create_offer). The external mediaservo-webrtc W3C API layer uses camelCase (e.g. createOffer) and RTC-prefixed type names (e.g. RTCPeerConnection). See docs/modules/17-webrtc-crate.md.


---

## 1. Three-Backend Architecture (D11)

```
┌─────────────────────────────────────────────────────────────┐
│                  MediaTransport trait (sans-I/O)             │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  backend-webrtc-sys (default)   backend-webrtc-rs   stub    │
│                                                             │
│  C++ libwebrtc FFI             tokio async         test stub│
│  ~1M+ LOC                      ~80K LOC            no deps  │
│  Full W3C                      Full W3C (D27)      no-op    │
│  cmake + Corrosion             pure Rust           build aid│
│                                                             │
│  str0m → Phase 2+ (Embed/LAN, sans-I/O native, ~30K LOC, zero external deps) │
└─────────────────────────────────────────────────────────────┘
```

| Backend | Compile Feature | Scenario | Phase |
|---------|-----------------|----------|-------|
| libwebrtc | `backend-webrtc-sys` (default) | Public-internet remote control, weak-network penetration | Phase 0 |
| webrtc-rs | `backend-webrtc-rs` | Future Embed upgrade | Phase 0 (struct skeleton) |
| stub | When no feature set | Dev/test/compile checks | Phase 0 |
| str0m | `backend-str0m` | Embedded integration, LAN P2P | Phase 2+ |

---

## 2. MediaTransport trait (D31)

### 2.1 Factory Trait

```rust
/// Transport factory (webrtc-kit's RTCPeerConnectionFactory pattern).

pub trait MediaTransportFactory: Send + Sync {
    fn backend_name(&self) -> &str;
    fn create_transport(
        &self,
        config: &TransportConfig,
        callbacks: Box<dyn TransportCallbacks + Send>,
    ) -> Result<Box<dyn MediaTransport>>;
}

pub trait TransportCallbacks {
    fn on_ice_candidate(&self, candidate: &RTCIceCandidate);
    fn on_connection_state_change(&self, state: ConnectionState);
    fn on_track_added(&self, track: &MediaTrackInfo);
    fn on_data_channel_open(&self, label: &str);
    fn on_data_channel_message(&self, label: &str, data: &[u8]);
}
```

### 2.2 Transport Trait (sans-I/O first)

```rust
/// Transport instance. References str0m's Rtc state machine + webrtc-rs's RTCPeerConnection.
/// sans-I/O first — the Embed scenario needs no tokio.

pub trait MediaTransport: Send {
    // ── Network I/O (sans-I/O model) ──

    /// Feeds in a network packet.
    fn handle_input(&mut self, data: &[u8]) -> Result<()>;

    /// Drains output. May return data to send, events, timeouts, or nothing.
    fn poll_output(&mut self) -> Result<TransportOutput>;

    // ── SDP negotiation ──

    fn create_offer(&self) -> Result<RTCSessionDescription>;
    fn create_answer(&self) -> Result<RTCSessionDescription>;
    fn set_local_description(&self, sd: &RTCSessionDescription) -> Result<()>;
    fn set_remote_description(&self, sd: &RTCSessionDescription) -> Result<()>;

    // ── ICE ──

    fn add_ice_candidate(&self, candidate: &RTCIceCandidate) -> Result<()>;

    // ── Media tracks ──

    fn create_video_track(&self, config: &VideoTrackConfig)
        -> Result<Box<dyn MediaTrack>>;
    fn create_data_channel(&self, label: &str)
        -> Result<Box<dyn DataChannelHandle>>;
    fn get_video_track(&self, id: &str)
        -> Option<Box<dyn MediaTrack>>;

    // ── Lifecycle ──

    fn close(&self);
    fn is_connected(&self) -> bool;
}
```

### 2.3 sans-I/O Output Enum

```rust
/// str0m's Output variants. All backends uniformly return this enum.

pub enum TransportOutput {
    Transmit(Vec<u8>),            // Packets to send
    Event(TransportEvent),        // State-change events
    Timeout(Duration),            // Wait time before the next poll_output call
    Nothing,                      // Nothing pending
}

pub enum TransportEvent {
    Connected,
    Disconnected,
    IceStateChanged(RTCIceConnectionState),
    TrackAdded(MediaTrackInfo),
    TrackData(MediaData),
    ChannelOpen(String),
    ChannelData(String, Vec<u8>),
    Stats(TransportStats),
}
```

### 2.4 Track and DataChannel traits

```rust
pub trait MediaTrack: Send {
    fn id(&self) -> &str;
    fn kind(&self) -> TrackKind;
    fn write(&mut self, frame: &[u8]) -> Result<()>;  // Writes encoded frames → TrackLocal::write_frame → libwebrtc RTP/SRTP packing
    fn close(&self);
}

pub trait DataChannelHandle: Send {
    fn label(&self) -> &str;
    fn send_text(&self, text: &str) -> Result<()>;
    fn send_bytes(&self, data: &[u8]) -> Result<()>;
    fn close(&self);
}

pub enum TrackKind { Video, Audio }
```

### 2.5 RTP Track Pipeline (webrtc-sys backend)

When using the webrtc-sys (libwebrtc) backend, the video frame push path:

```
Encoder (GStreamer/HW encoder)
  │ H.264/H.265 encoded frame (Annex B / AVCC)
  ▼
TrackLocal (MediaTrack::write)
  │ Wraps the encoded frame as a webrtc::VideoFrame
  ▼
libwebrtc VideoTrackSource::write_frame()
  │ RTP packing (packetizer) + SRTP encryption
  ▼
Network (ICE/DTLS transport)
```

**Key boundary**: the byte boundary produced by GStreamer is consumed at `TrackLocal::write()`; from there RTP packaging and encryption are handled entirely inside libwebrtc. User code only needs to provide the encoded frame's `&[u8]`.

### 2.6 RTCDataChannel Degradation Note (D155)

In the webrtc-sys backend, RTCDataChannel is degraded to **control-signaling-only** use:
- Keyboard/mouse/gamepad input events
- Clipboard sync
- File transfer via HTTP multipart (not DC)

The reason: libwebrtc's SCTP RTCDataChannel retransmission policy is not flexible enough under weak networks, giving poor experience for bulk data. In the Phase 1 remote cockpit, the DC carries only `<1KB` control command frames.

### 2.7 GStreamer→WebRTC Byte Boundary (D155)

```
┌───────────────────────────────────────────────────┐
│              GStreamer Pipeline                   │
│  appsrc → videoscale → x264enc → appsink          │
│                               ↓ emit-signals=true │
└───────────────────────────────────────────────────┘
                                │
                H.264 Annex B byte stream (encoded frames)
                                │
                                ▼
┌───────────────────────────────────────────────────┐
│           MediaServo Transport Layer              │
│  MediaTrack::write(&[u8])                         │
│  → webrtc-sys: TrackLocal::write_frame()          │
│  → libwebrtc: RTP packetizer + SRTP + DTLS        │
└───────────────────────────────────────────────────┘
```

**Contract**: the GStreamer pipeline produces complete encoded frames (NAL unit boundaries correct). The Transport layer never re-fragments encoded frames — NAL unit boundaries are guaranteed by GStreamer; libwebrtc's RTP packetizer handles MTU fragmentation.

---

---

## 3. Backend Compile-Time Dispatch (D32)

### 3.1 cfg dispatch (webrtc-kit pattern)

```rust
// mediaservo-transport/src/engine.rs

pub fn create_factory(
    config: &TransportConfig,
) -> Option<Box<dyn MediaTransportFactory>> {
    // str0m: Phase 2+ — see section 1 for the backend table
    #[cfg(feature = "backend-str0m")]
    { return Some(Box::new(str0m_backend::Str0mFactory::new())); }

    #[cfg(feature = "backend-webrtc-sys")
    { return Some(Box::new(libwebrtc_backend::LibWebRtcFactory::new(config)?)); }

    #[cfg(feature = "backend-webrtc-rs")]
    { return Some(Box::new(webrtc_rs_backend::WebRtcRsFactory::new())); }

    None
}
```

### 3.2 Mutual-Exclusion Guards

```rust
// See webrtc-kit's compile_error! pattern

#[cfg(any(
    // str0m guards (Phase 2+):
    // all(feature = "backend-str0m", feature = "backend-webrtc-sys"),
    // all(feature = "backend-str0m", feature = "backend-webrtc-rs"),
    all(feature = "backend-webrtc-sys", feature = "backend-webrtc-rs"),
))]
compile_error!("Only one WebRTC backend can be enabled at a time");
```

### 3.3 Backend Comparison

| Dimension | str0m | libwebrtc (via webrtc-sys) | webrtc-rs |
|-----------|-------|----------------------------|-----------|
| Design philosophy | sans-I/O state machine | C++ FFI wrapper (webrtc-sys crate) | tokio async |
| W3C API compat | Incompatible (intentionally) | Fully compatible | Compatible (v0.20+) |
| Runtime requirements | None | libwebrtc.so + webrtc-sys FFI | tokio |
| Build complexity | cargo build | cmake + Corrosion cross-compile | cargo build |
| Binary size | ~100KB | ~30MB (.so) | ~2MB |
| GCC congestion control | ❌ homegrown | ✅ full Google GCC | ❌ homegrown |
| FEC + NetEQ | ❌ homegrown | ✅ full | ❌ homegrown |
| Suitable scenarios | LAN P2P, embedded integration | Public internet, weak network, remote control (default) | Future embedded integration |

---

## 4. sans-I/O Run Loop (D33)

### 4.1 Contract (enforced by str0m)

```
After every mutation (handle_input, write, set_local_description, add_ice_candidate),
before the next mutation, poll_output must be fully drained until it returns Nothing or Timeout.
```

### 4.2 Standard Loop Pattern

```rust
impl App {
    fn run_loop(&mut self, transport: &mut dyn MediaTransport, socket: &UdpSocket) {
        loop {
            // Step 1: kick off from input
            if let Some(data) = socket.try_recv()? {
                transport.handle_input(&data)?;
                // mutation happened → must drain
            }

            // Step 2: drain output
            self.drain_output(transport, socket)?;

            // Step 3: write media (when available)
            if let Some(frame) = self.camera.next_frame() {
                if let Some(track) = transport.get_video_track("video-0") {
                    track.write(&frame)?;
                    // mutation happened → must drain
                    self.drain_output(transport, socket)?;
                }
            }

            // Step 4: wait for the next event
            // TransportOutput::Timeout → sleep for that duration
            // TransportOutput::Nothing → keep looping
        }
    }

    fn drain_output(
        &mut self, transport: &mut dyn MediaTransport, socket: &UdpSocket,
    ) -> Result<()> {
        loop {
            match transport.poll_output()? {
                TransportOutput::Transmit(data) => socket.send(&data)?,
                TransportOutput::Event(e) => self.handle_event(e),
                TransportOutput::Timeout(t) => {
                    self.next_wake = Instant::now() + t;
                    break;
                }
                TransportOutput::Nothing => break,
            }
        }
        Ok(())
    }
}
```

### 4.3 Why This Contract Matters

| Consequence of violation | Example |
|--------------------------|---------|
| ICE state events lost | Connection established but the app doesn't know → silent failure |
| Output packets accumulate in the buffer | Video's DTLS handshake completed, but the client never sent it |
| Reentrancy issues | handle_input produces events, poll_output not called, the next handle_input creates inconsistent state |
| Unified behavior across all backends | Even when libwebrtc internally uses callback threads, poll_output drains per the contract |

---

## 5. Configuration Structures

```rust
pub struct TransportConfig {
    pub ice_servers: Vec<RTCIceServer>,
    pub ice_transport_policy: IcePolicy,     // All, Relay
    pub bundle_policy: BundlePolicy,         // Balanced, MaxBundle
    pub rtcp_mux_policy: RtcpMuxPolicy,
    pub ice_candidate_pool_size: u8,
    pub cert_type: CertificateType,          // ECDSA, RSA

    // Backend-specific config
    pub backend_config: BackendConfig,
}

pub enum BackendConfig {
    Str0m {
        mtu: usize,
        ice_lite: bool,               // The signaling server can set this option
        rtp_mode: bool,               // RTP-level access instead of frame-level
    },
    LibWebRtc {
        field_trials: String,          // WebRTC field trials string
        threads: usize,                // libwebrtc worker thread count
        disable_encryption: bool,      // Testing only
    },
    WebRtcRs {
        ice_lite: bool,
        ice_udp_mux_port: Option<u16>,
    },
}

pub struct VideoTrackConfig {
    pub id: String,
    pub direction: RtpDirection,         // SendOnly, RecvOnly, SendRecv
    pub codecs: Vec<CodecConfig>,        // Priority-ordered
}

pub struct CodecConfig {
    pub mime_type: String,               // "video/H264", "video/VP9"
    pub clock_rate: u32,                 // 90000
    pub fmtp_params: HashMap<String, String>,  // "packetization-mode=1"
}
```

---

## 6. Scenario Matrix

| Scenario | Backend | Transport mode | TURN | Runtime |
|----------|---------|----------------|------|---------|
| Embedded integration (LAN) | str0m | P2P | None needed | None (sans-I/O) |
| Embedded integration (WAN) | libwebrtc (Phase 2) | P2P + relay | coturn | libwebrtc.so |
| Host-platform sidecar | libwebrtc (default) | P2P + relay | coturn | libwebrtc.so + gstreamer |
| Standalone server | libwebrtc + mediasoup SFU | SFU | coturn/srt | libwebrtc.so + tokio |
| Web viewer | Browser RTCPeerConnection | P2P/SFU (native) | coturn | Built into the browser |

---

## 7. Reference Project Mapping

| MediaServo Concept | webrtc-kit | str0m | mediasoup-rust |
|--------------------|------------|-------|----------------|
| MediaTransportFactory | RTCPeerConnectionFactory | — | Router |
| MediaTransport | RTCPeerConnection | Rtc | Transport trait |
| handle_input | — (callback-based) | handle_input | — (channel-based) |
| poll_output | — | poll_output | — (async) |
| TransportOutput | — (event callbacks) | Output enum | — (async) |
| MediaTrack | VideoTrack trait | Writer handle | Producer/Consumer |
| Backend dispatch | RTCEngine::create_factory (cfg) | None (single impl) | None (C++ worker) |
| sans-I/O contract | None (callback-driven) | Core design principle | None (channel IPC) |
