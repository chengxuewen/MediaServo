# SDD 03: Decode + Render

## 1. Overview

Video decode + render module. Receives encoded frames from WebRTC, decodes to I420, converts to GPU textures, and outputs to screen.

**Decision refs**: D46 (VideoDecode trait), D47 (VideoRender trait), D75 (I420 standard)

## 2. Interface Definition

```rust
pub trait VideoDecoder: Send {
    fn supported_codecs(&self) -> Vec<Codec>;
    fn decode(&mut self, encoded: EncodedFragment) -> Result<RawFrame>;
    fn request_keyframe(&mut self) -> Result<()>;
    fn reset(&mut self) -> Result<()>;
    fn caps(&self) -> DecoderCaps;
}

pub trait VideoRenderer: Send {
    fn set_surface(&mut self, target: &RenderTarget) -> Result<()>;
    fn render(&mut self, frame: RawFrame) -> Result<()>;
    fn caps(&self) -> RenderCaps;
}

bitflags! {
    pub struct DecoderCaps: u32 {
        const H264      = 1 << 0;
        const HEVC      = 1 << 1;
        const VP8       = 1 << 2;
        const VP9       = 1 << 3;
        const GPU_TEXTURE = 1 << 4;
        const ADAPTIVE  = 1 << 5;  // Packet-loss recovery
    }
}

bitflags! {
    pub struct RenderCaps: u32 {
        const CPU_FALLBACK = 1 << 0;
        const GPU_DIRECT   = 1 << 1;
    }
}
```

### Frame Structure

```rust
pub struct RawFrame {
    pub data: FrameData,         // I420 data or GPU texture handle
    pub width: u32,
    pub height: u32,
    pub pts: u64,
}

pub enum FrameData {
    I420(Vec<u8>),               // Phase 1 CPU path
    Texture(TextureHandle),      // Phase 2 GPU path
}
```

## 3. Pipeline Flow

```
WebRTC on_message
    │ EncodedFragment (H.264 Annex-B)
    ▼
VideoDecoder::decode (GStreamer decodebin)
    │ RawFrame (I420)
    ▼
I420 → YUV→RGB conversion (CPU Phase 1 / shader Phase 2)
    │ RGBA buffer
    ▼
VideoRenderer::render → output to window/Canvas
```

Phase 1 CPU readback path: `appsink → CPU buffer → Canvas 2D`

## 4. Backend Strategy

| Backend | Scenario | Notes |
|---------|----------|-------|
| GStreamer decodebin | Phase 1 general | appsrc → h264parse → decodebin → videoconvert → appsink |
| FFmpeg libavcodec | Remote SDK static link | Only in the backend-str0m scenario |
| WebCodecs | Browser | Skips the codec crate |

## 5. Error Handling

| Condition | Category | Error Code | Recovery |
|-----------|----------|------------|----------|
| Decoder init failure | Fatal | 5001 | Exit and prompt config change |
| Invalid decoded frame format | Recoverable | 5002 | Drop the frame, request_keyframe |
| Invalid render surface | Recoverable | 5002 | Skip render, wait for new surface |

## 6. Test Plan

| Test | Type | Description |
|------|------|-------------|
| decode_h264_baseline | Integration | H.264 Annex-B input, complete I420 frames out |
| decode_latency_budget | Integration | Per-frame decode latency <50ms (720p) |
| frame_integrity | Integration | Post-decode YUV plane sizes correct, PSNR > 40dB |
| keyframe_request | Unit | IDR frame received after request_keyframe |
| decoder_reset | Unit | Decoder state cleared after reset, can decode again |
| render_rgba_output | Integration | RGBA buffer dimensions correct after render |
| phase1_cpu_path | Integration | CPU fallback path > 20fps |
| format_conversion | Unit | I420 → RGBA color values correct |
