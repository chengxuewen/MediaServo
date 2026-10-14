# SDD 02: WebRTC Push

## 1. Overview

WebRTC push module. Encodes I420 frames to H.264/H.265 and pushes them via libwebrtc to the Server relay.

**Decision refs**: D11 (triple-backend architecture), D32 (compile-time dispatch), D-ERR-01 (circuit breaker)

## 2. Interface Definition

```rust
#[async_trait]
pub trait StreamPublisher: Send + Sync {
    fn name(&self) -> &str;
    fn supported_codecs(&self) -> Vec<Codec>;
    async fn connect(&mut self, signaling: &SignalingConfig) -> Result<()>;
    async fn push_frame(&mut self, frame: EncodedFrame) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;
    fn stats(&self) -> StreamStats;
}

pub struct StreamStats {
    pub fps: f64,
    pub bitrate_kbps: u32,
    pub rtt_ms: u32,
    pub packets_lost: u64,
    pub ice_state: IceState,
}

pub struct SignalingConfig {
    pub ws_url: String,
    pub room_id: String,
    pub peer_id: String,
    pub psk: Option<String>,
}
```

### Encoding Configuration

```rust
pub struct EncodeConfig {
    pub codec: Codec,           // H264 | H265
    pub width: u32,             // 1280
    pub height: u32,            // 720
    pub fps: u32,               // 30
    pub bitrate_kbps: u32,      // 2000
    pub gop: u32,               // 30
    pub preset: EncoderPreset,  // P1-P7
}
```

## 3. Backend Strategy (D11)

| Backend | Scenario | feature flag |
|---------|----------|--------------|
| str0m | Embed/LAN, sans-I/O lightweight | backend-str0m (default) |
| libwebrtc | Public internet / weak network, GCC+FEC | backend-libwebrtc |
| webrtc-rs | Future upgrade target | backend-webrtc-rs |

Mutually exclusive via compile-time `#[cfg(feature)]`; one backend per build.

## 4. Signaling Flow (WHIP/WHEP)

```
Remote              Server              Host
  │                   │                  │
  │─── SDP Offer ────▶│─── relay ───────▶│
  │                   │                  │
  │◀── SDP Answer ────│◀── relay ────────│
  │                   │                  │
  │◀══════ ICE ═══════▶◀══════ ICE ══════▶
  │                   │                  │
  │═══════════ RTP/SRTP ═════════════════▶
```

## 5. Error Handling (D-ERR-01 Circuit Breaker)

| Condition | Category | Error Code | Recovery |
|-----------|----------|------------|----------|
| ICE connect timeout | Recoverable | 1003 | ICE restart + exponential backoff |
| WebSocket disconnect | Recoverable | 1001 | Exponential backoff reconnect (1s, 2s, 4s...) |
| Encoder init failure | Fatal | 2001 | Fall back to software VP8 |
| 5 failures / 60s | Circuit break | 1001 | Stop pushing, wait 30s to recover |

Circuit breaker state machine: `Closed → Open (5 failures / 60s) → HalfOpen (wait 30s) → Closed`

## 6. Test Plan

| Test | Type | Description |
|------|------|-------------|
| stream_startup | Integration | Full connect → push_frame → disconnect lifecycle |
| simulcast_enabled | Integration | Multi-resolution push, each layer encoded independently |
| ice_restart | Integration | Simulate ICE disconnect → ICE restart → recovery |
| bitrate_adaptation | Integration | GCC adjusts bitrate per simulated bandwidth changes |
| circuit_breaker_open | Unit | Circuit opens after 5 failures; Open state blocks pushing |
| circuit_breaker_recover | Unit | Open state transitions to HalfOpen after 30s |
| codec_negotiation | Unit | Codec negotiation correct in SDP Offer/Answer |
| stats_reporting | Unit | stats() returns live fps/bitrate/rtt |
