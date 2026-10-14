# 18. Codec Crate — mediaservo-codec

> Status: Phase 2 planned | Related decisions: D43, D46, D70, D71, D82, C5

## Positioning

`mediaservo-codec` is MediaServo's unified codec layer, providing the `VideoEncoder` / `VideoDecoder` traits with backends dispatched via compile-time feature gates across GStreamer and FFmpeg. It follows the C5 `&[u8]` byte boundary — no dependency on `mediaservo-media`'s type system.

```
┌─────────────────────────────────────────────────────┐
│                 mediaservo-codec                    │
│                                                     │
│  CodecFactory::create_encoder()                     │
│    └→ Box<dyn VideoEncoder>                         │
│                                                     │
│  CodecFactory::create_decoder()                     │
│    └→ Box<dyn VideoDecoder>                         │
│                                                     │
│  ┌─────────────────────────────────────────────┐    │
│  │            backend trait layer              │    │
│  │  GstEncoder    │    FfmpegEncoder           │    │
│  │  GstDecoder    │    FfmpegDecoder           │    │
│  └──────────────┬──────────────────────────────┘ │
│                 ▼                                  │
│  ┌─────────────────────────────────────────────┐    │
│  │    backend-gstreamer  │  backend-ffmpeg     │    │
│  │   (dynamic .so)       │  (static .a)        │    │
│  └───────────────────────┴─────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

## Naming Convention

| Type | Name | Notes |
|------|------|-------|
| Factory | `CodecFactory` | Encoder/decoder creation entry |
| Encode trait | `VideoEncoder` | Push-pull sync interface |
| Decode trait | `VideoDecoder` | Push-pull sync interface |
| Encode config | `EncoderConfig` | Builder pattern |
| Decode config | `DecoderConfig` | Builder pattern |
| Backend ID | `BackendId::GStreamer` / `BackendId::FFmpeg` | Runtime backend identifier |
| Codec ID | `CodecId::H264` / `CodecId::VP8` / ... | RFC 6381 aligned |
| Pixel format | `PixelFormat::Yuv420p` / `PixelFormat::Nv12` / ... | Internal format, decoupled from media |
| Error | `CodecError` | 10 variants |

## Trait Design

### VideoEncoder

```rust
pub trait VideoEncoder: Send {
    fn configure(&mut self, config: &EncoderConfig) -> Result<(), CodecError>;
    fn push_frame(&mut self, frame: &VideoFrame) -> Result<(), CodecError>;
    fn pull_packet(&mut self) -> Result<Option<EncodedPacket>, CodecError>;
    fn flush(&mut self) -> Result<(), CodecError>;
    fn reset(&mut self) -> Result<(), CodecError>;
}
```

Push-pull model (aligned with FFmpeg `avcodec_send_frame` / `avcodec_receive_packet`):
- `push_frame` feeds a raw frame → `pull_packet` pulls encoded packets
- B-frame reordering: one push may yield 0-N packets
- `flush()` drains the buffer, signaling EOF

### VideoDecoder

```rust
pub trait VideoDecoder: Send {
    fn push_packet(&mut self, packet: &EncodedPacket) -> Result<(), CodecError>;
    fn pull_frame(&mut self) -> Result<Option<VideoFrame>, CodecError>;
    fn flush(&mut self) -> Result<(), CodecError>;
    fn reset(&mut self) -> Result<(), CodecError>;
}
```

Mirror of encoding: `push_packet` → `pull_frame`.

## Backend Matrix

| Backend | Feature | Linking | Scenario |
|---------|---------|---------|----------|
| GStreamer | `backend-gstreamer` | Dynamic .so | Host encode default |
| FFmpeg | `backend-ffmpeg` | Static .a | Remote decode (str0m backend, D72); Host encode alternative (D43 revision) |
| Stub | No feature | None | Dev/test/compile checks |

The Host can switch between the two backends:
```toml
# Cargo.toml (mediaservo-host)
mediaservo-codec = { features = ["backend-ffmpeg"] }     # production/edge static distribution
# mediaservo-codec = { features = ["backend-gstreamer"] } # dev-machine default
```

## Encode/Decode Paths

### Host encoding

```
Capture → VideoFrame(I420) → CodecFactory::create_encoder()
  → VideoEncoder::push_frame() → pull_packet() → EncodedPacket
  → TrackSender::write_frame(packet.data) → WebRTC RTP → Server
```

### Remote decoding

```
TrackReceiver → EncodedPacket(H.264 bytes)
  → CodecFactory::create_decoder()
  → VideoDecoder::push_packet() → pull_frame() → VideoFrame(I420)
  → Renderer
```

## Data Boundary (C5)

The codec crate **does not depend** on `mediaservo-media`. All data exchange goes through `&[u8]`:

- `VideoFrame` (codec's own type, distinct from media's `VideoFrame<T>`)
- `EncodedPacket` (pure `Vec<u8>` + metadata)
- Callers handle media ↔ codec type conversion (flattening I420Buffer planes or wrapping EncodedFragments)

## Factory Entry

```rust
pub struct CodecFactory;

impl CodecFactory {
    pub fn new() -> Self;

    pub fn create_encoder(
        &self, config: EncoderConfig,
        preferred_backend: Option<BackendId>,
    ) -> Result<Box<dyn VideoEncoder>, CodecError>;

    pub fn create_decoder(
        &self, config: DecoderConfig,
        preferred_backend: Option<BackendId>,
    ) -> Result<Box<dyn VideoDecoder>, CodecError>;

    // Capability discovery
    pub fn encoder_capabilities(&self, codec: CodecId) -> Vec<EncoderCapability>;
    pub fn decoder_capabilities(&self, codec: CodecId) -> Vec<DecoderCapability>;
}
```

Usage example:

```rust
use mediaservo_codec::{CodecFactory, EncoderConfig, CodecId, BackendId};

let factory = CodecFactory::new();
let mut encoder = factory.create_encoder(
    EncoderConfig::builder(CodecId::H264, format)
        .bitrate(Bitrate::Vbr { target: 4000, max: 8000 })
        .preset(EncoderPreset::VeryFast)
        .build(),
    Some(BackendId::FFmpeg),  // Force the static FFmpeg backend
)?;

for raw_frame in capture_source {
    encoder.push_frame(&raw_frame)?;
    while let Some(pkt) = encoder.pull_packet()? {
        webrtc_track.write_frame(&pkt.data).await?;
    }
}
encoder.flush()?;
```

## File Structure

```
crates/mediaservo-codec/
├── Cargo.toml
├── src/
│   ├── lib.rs              re-exports + feature gate guards
│   ├── codec.rs            CodecId, PixelFormat, VideoFormat, FrameRate
│   ├── config.rs           EncoderConfig, DecoderConfig + builders
│   ├── encoder.rs          VideoEncoder trait
│   ├── decoder.rs          VideoDecoder trait
│   ├── factory.rs          CodecFactory + EncoderCapability/DecoderCapability
│   ├── error.rs            CodecError (thiserror)
│   ├── frame.rs            VideoFrame, Plane
│   ├── packet.rs           EncodedPacket
│   └── backend/
│       ├── mod.rs          cfg dispatch + compile_error! guards
│       ├── gstreamer.rs    GstEncoder, GstDecoder (dynamic)
│       ├── ffmpeg.rs       FfmpegEncoder, FfmpegDecoder (static)
│       └── stub.rs         StubEncoder, StubDecoder (dev)
├── tests/
│   ├── integration_roundtrip.rs
│   ├── cross_backend.rs
│   ├── perf_bench.rs
│   └── ...
└── benches/
    └── encode_bench.rs
```
## API Boundary & Type Mapping

The codec crate does not depend on mediaservo-media; data is exchanged via &[u8] (C5).

| codec type | media/webrtc type | Conversion direction |
|------------|-------------------|----------------------|
| codec::VideoFrame | media::VideoFrame\<I420Buffer\> | media→codec: flatten I420Buffer planes |
| codec::EncodedPacket | webrtc::TrackSender::write_frame(&[u8]) | codec→webrtc: encoded packet bytes → transport |
| codec::EncodedPacket | webrtc::TrackReceiver (raw bytes) | webrtc→codec: received encoded bytes → decoder |

**Note**: the webrtc crate currently exposes no raw encoded-packet receive API (reserved for Phase 2).

## Error Handling

CodecError (thiserror), key variants:

| Variant | Description |
|---------|-------------|
| InvalidConfig(String) | Invalid encoder configuration |
| UnsupportedCodec | The backend does not support this codec |
| NoBackend(CodecId) | No backend compiled (missing feature flag) |
| Encoder(String) | Encoding runtime error |
| Decoder(String) | Decoding runtime error |
| InvalidState(String) | Illegal state transition |
| ResourceExhausted(String) | Internal queue full |
| InvalidInput(String) | Input data format mismatch |

## Cross-Platform

| Backend | macOS x64 | macOS ARM | Linux x64 | Linux ARM | Windows |
|---------|:---------:|:---------:|:---------:|:---------:|:-------:|
| GStreamer | brew | brew | apt | apt | MSYS2 |
| FFmpeg static | prebuilt | prebuilt | prebuilt | cross-compile | vcpkg |
| FFmpeg dynamic | brew | brew | apt | apt | vcpkg |

See docs/reference/codec/ffmpeg-static-build-strategy.md.


## Test Coverage (69 tests, planned)

| Class | Count | Notes |
|-------|-------|-------|
| Unit: encoder config/lifecycle | 15 | configure, push/pull, flush, error |
| Unit: push-pull loop | 10 | 30-frame encode, PTS monotonic |
| Unit: decoder | 11 | decode lifecycle, reset, close |
| Unit: codec selection | 3 | probe, auto-select, fallback |
| Integration: roundtrip | 5 | I420→H.264→I420, PSNR >40dB |
| Integration: pipeline | 2 | as a MediaProcessor in PipelineEngine |
| Cross-backend | 4 | GStreamer vs FFmpeg output comparison |
| Performance | 5 | encode latency p99 <33ms, throughput ≥30fps |
| Static build | 4 | nm verifies no undefined symbols, size <14MB |
| Fuzz | 6 | random I420 input, random bitstream, concurrent |
| Property | 5 | keyframe interval, PTS monotonic |

## Current Status

| Capability | Status |
|------------|--------|
| VideoEncoder trait definition | 🔲 Planned |
| VideoDecoder trait definition | 🔲 Planned |
| CodecFactory | 🔲 Planned |
| FFmpeg backend | 🔲 Planned |
| GStreamer backend | 🔲 Planned |
| Stub backend | 🔲 Planned |
| TDD tests | 🔲 69 tests planned |
| E2E acceptance | 🔲 8 scenarios planned |
| Static FFmpeg build | 🔲 Strategy designed |

## Related Documents

- [17. WebRTC Crate](17-webrtc-crate.md) — the WebRTC transport layer, target of encoder output
- [08. Pipeline Model Reference](08-pipeline-model.md) — PipelineEngine integration
- [Decision records D43/D46/D70/D71/D82](../.agents/memorys/decisions.md) — the codec architecture decision chain
- [FFmpeg static build strategy](../reference/codec/ffmpeg-static-build-strategy.md) — build plan
- [SDD acceptance criteria](../../docs/plans/mediaservo-codec/acceptance-criteria.md) — concrete thresholds
- [Acceptance matrix §10](../../docs/plans/mediaservo-codec/acceptance-criteria.md#10-test-matrix) — 20-scenario AC matrix

## Cross References

The following documents link back to this one:
- [17. WebRTC Crate](17-webrtc-crate.md) — Phase 2 reserved write_frame / on_encoded_packet API
- [SDD acceptance criteria](../../docs/plans/mediaservo-codec/acceptance-criteria.md) — aligned with the push-pull trait API
- [FFmpeg static build strategy](../reference/codec/ffmpeg-static-build-strategy.md) — prebuilt + CI integration
- [Acceptance matrix §10](../../docs/plans/mediaservo-codec/acceptance-criteria.md#10-test-matrix) — 8+ scenario test matrix
