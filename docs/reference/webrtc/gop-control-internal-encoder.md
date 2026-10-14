# libwebrtc Internal-Encoder GOP Control Analysis — First-Frame Latency Root Cause & Fix Paths

> Doc: 2026-08-10 | Status: **research complete, plan pending implementation** | Related: [keyframe-black-screen-analysis.md](keyframe-black-screen-analysis.md) (PIT-65)
> Background: Host push first-frame latency of 68s → traced to the libwebrtc internal encoder's steady-state 99s GOP + a broken PLI response chain

## 1. Symptom

| Metric | Measured |
|------|--------|
| Negotiation + ICE + consume elapsed | 73ms (normal) |
| consumed → first frame rendered | **68.4s** (abnormal) |
| Host steady-state keyframe interval | **99s** (confirmed by the mediasoup worker's `key frame received` logs) |
| request_key_frame (PLI) after consume | arrives at the Host's UDP port (confirmed by the 72B tcpdump) but the **take_keyframe_request() flag is never set** |
| `x-google-max-keyframe-interval` SDP injection (local answer / remote offer) | **both ineffective** (only transient 2s/20s at startup, falling back to 99s steady state) |

## 2. Root-Cause Chain (complete)

```
The libwebrtc internal-encoder path (raw I420 → VideoStreamEncoder → libvpx VP8)
  ├─ GOP scheduling: VideoCodecVP8.keyFrameInterval = 0 (default, never set)
  │    → libvpx kf_max_dist = 0 → no proactive keyframes
  │    → keyframes appear only by accident of scene changes/bitrate fluctuations → steady state 99s
  │
  ├─ PLI response: mediasoup request_key_frame → the RTCP PLI arrives at the Host's UDP port
  │    → but the livekit fork's RtpVideoSender→VideoStreamEncoder RTCP wiring is broken
  │    → the take_keyframe_request() flag is never set (confirmed by diagnostic logs)
  │
  └─ SDP injection: x-google-max-keyframe-interval is a Chrome integration-layer-only parameter
       → the standard libwebrtc library contains no parsing code (no such string in webrtc_video_engine.cc)
       → the livekit fork kept the GOP chain but has no SDP-parsing entry point
```

**Essence**: in the livekit fork's libwebrtc internal-encoder path, **both the GOP configuration entry and the PLI response chain are unusable** — this is an architectural cut by the fork, not a configuration problem.

## 3. libwebrtc's Original GOP Control Mechanism (the standard chain)

Standard libwebrtc's periodic keyframes are an **encoder-layer mechanism** (`kf_max_dist`), not a scheduling-layer one:

```
The application sets VideoEncoderConfig.encoder_specific_settings
  = Vp8EncoderSpecificSettings{ VideoCodecVP8{ keyFrameInterval: N } }
  │
  ▼
VideoSendStreamImpl::ReconfigureVideoEncoder(config)   // video_send_stream_impl.cc:611
  ▼
VideoStreamEncoder::ConfigureEncoder → InitEncode(&send_codec_)  // video_stream_encoder.cc:1386
  ▼
libvpx_vp8_encoder.cc:651-653:
  if (inst->VP8().keyFrameInterval > 0) {
    vpx_configs_[0].kf_max_dist = inst->VP8().keyFrameInterval;
  }
  ▼
libvpx emits a keyframe every kf_max_dist frames (deterministic, PLI-independent)
```

**Key facts**:
1. `VideoCodecVP8.keyFrameInterval` (in frames) → `kf_max_dist` → libvpx periodic keyframes
2. The only entry is `VideoEncoderConfig.encoder_specific_settings` (a programmatic API)
3. **Standard libwebrtc does not parse the SDP fmtp's `x-google-max-keyframe-interval`** (that's a Chromium integration-layer extension, not the libwebrtc library)
4. The livekit fork **kept the entire chain** (the `Vp8EncoderSpecificSettings`/`FillVideoCodecVp8`/`LibvpxVp8`/`kf_max_dist` symbols are all in libwebrtc.a) — webrtc-sys just never exposed a setting entry

## 4. The Standard Mechanism for Triggering a One-Off Keyframe (key to the option choice)

The standard libwebrtc API: `RtpEncodingParameters.request_key_frame = true`:

```
RtpSender::SetParameters({ encodings: [{ request_key_frame: true }] })
  ▼
RtpSenderBase::SetParameters (rtp_sender.cc:650)
  ▼
SetRtpParametersOnWorkerThread(media_channel_) → WebRtcVideoChannel::SetParameters
  ▼
webrtc_video_engine.cc:2227: if (encoding.request_key_frame) → GenerateKeyFrame()
  ▼
The next frame is an IDR (deterministic; a local call, no RTCP involved)
```

**webrtc-sys today**:
- The C++ `RtpEncodingParameters` has a `request_key_frame` field (rtp_parameters.h:707)
- The Rust binding's `RtpEncodingParameters` **lacks the field** (rtp_parameters.rs:126-146)
- `to_native_rtp_encoding_paramters` (rtp_parameters.cpp:98) **never sets it**

## 5. Option Comparison

### 5.1 Candidates

| Option | GOP control | Simulcast | SVC | First frame | Change size |
|------|:---:|:---:|:---:|:---:|------|
| **A. Periodically rebuild the VideoTrackSource** | ✅ needs testing | ❌ | ❌ | <2s? | medium (main.rs) |
| **B. capture_encoded_frame + passthrough** | ✅ fully controllable | ❌ (supports_simulcast=false) | ❌ (L1T1) | <0.5s | large (frame-loop rework) |
| **C. Accept the status quo** | ❌ 99s | — | — | ~50s | zero |
| **D. Periodic request_key_frame triggers (recommended)** | ✅ 1 IDR per N seconds | ✅ kept | ✅ kept | <2s | **small (2 FFI lines + the frame loop)** |

### 5.2 Recommendation: Option D (periodic request_key_frame)

**Why it's optimal**:
- Keeps the internal-encoder path (the Auto backend; simulcast/SVC/auto-degradation all preserved) — fits the "simple internal encoder" leaning
- Uses the libwebrtc **standard API** (`RtpEncodingParameters.request_key_frame`), not a hack
- Minimal change: 2 lines in webrtc-sys + 1 method in mediaservo-webrtc + a timer in the Host frame loop
- Unrelated to the broken PLI (a local SetParameters; no RTCP involved)
- The official livekit `peer_transport.rs` also does SDP munging (x-google-start-bitrate), proving SDP injection is an officially sanctioned path; but max-keyframe-interval has no parser → a programmatic trigger is needed

**Cost**: forcing an IDR every N seconds adds bandwidth (a keyframe is ~10x a delta frame). A 2s interval at 2Mbps adds roughly 5-8% bandwidth — acceptable.

## 6. webrtc-sys Dependency Decision: vendored [patch] (not a fork)

### 6.1 Why Not fork + git submodule

| Dimension | fork + submodule | vendored [patch] |
|------|:---:|:---:|
| Change size | 2 FFI lines; fork-management overhead > benefit | same change, zero management overhead |
| Team workflow | nested git; submodule update/sync is complex | a normal directory, tracked by git normally |
| libwebrtc prebuilt | still downloaded from livekit releases (download_url unchanged) | same |
| Future change growth | can upgrade to a fork | can upgrade at any time |
| C20 compliance | ✅ | ✅ (vendored with the user's consent) |

### 6.2 Implementation Form (confirmed)

```bash
mkdir -p vendor
cp -r ~/.cargo/registry/src/index.crates.io-*/webrtc-sys-0.3.39 vendor/webrtc-sys
rm vendor/webrtc-sys/Cargo.toml.orig
```

Root Cargo.toml:
```toml
[patch.crates-io]
webrtc-sys = { path = "vendor/webrtc-sys" }
```

**Key preconditions** (verified):
- The registry's Cargo.toml is the normalized form — **no workspace inheritance, no path dependencies; compiles standalone**
- `webrtc-sys-build` (0.3.18) still pulls from crates.io — the libwebrtc download logic is unchanged
- Total size 6.7M, committable to git

### 6.3 Files Needing Changes (the minimal set)

| File | Change |
|------|------|
| `vendor/webrtc-sys/src/rtp_parameters.rs` | add `pub request_key_frame: bool` to `RtpEncodingParameters` |
| `vendor/webrtc-sys/src/rtp_parameters.cpp` | add `native.request_key_frame = parameters.request_key_frame;` to `to_native_rtp_encoding_paramters` |
| `crates/mediaservo-webrtc/src/...` | expose a `request_key_frame()` method on `RtpSender` |
| `crates/mediaservo-host/src/main.rs` | the frame loop calls `request_key_frame()` every 2s |

## 7. Related Research Conclusions (the livekit rust-sdks architecture)

Reference sources: `.refinfo/rust-sdks` (a complete local clone, v0.3.41, incl. the f1d8e5a passthrough path)

### 7.1 livekit's Two Publish Paths

| Path | API | GOP control | Simulcast | SVC | Use |
|------|-----|:---:|:---:|:---:|------|
| raw (internal encoder) | `capture_frame()` + `VideoEncoderBackend::Auto` | ❌ none | ✅ | ✅ | simple camera/screen push (default) |
| encoded (passthrough) | `capture_encoded_frame()` + `VideoEncoderBackend::PreEncoded` | ✅ `take_keyframe_request()` | ❌ | ❌ L1T1 | pre-encoded/professional push |

### 7.2 livekit's Handling of GCC/SVC/simulcast

- **GCC**: SFU side (livekit-server Go/Pion) `pkg/sfu/bwe` (TWCC default/REMB); the client rust-sdk responds via `take_rate_control_request()`
- **Simulcast**: the client supplies multiple `RtpEncodingParameters` (RID layers) → libwebrtc's SimulcastEncoderAdapter; passthrough `supports_simulcast=false`
- **SVC**: the client sets `scalability_mode` (e.g. "L3T3_KEY") → real SVC in the internal encoder; passthrough is L1T1 only

### 7.3 Key Source Evidence (.refinfo/rust-sdks)

- `libwebrtc/src/native/video_source.rs`: the `take_keyframe_request` comment states explicitly "raised by the **pass-through encoder**...forward the request to the **upstream encoder**" — designed for the encoded path only
- `livekit/src/rtc_engine/rtc_session.rs:1920`: `transceiver.sender().set_video_encoder_backend(options.video_encoder)` — the backend is chosen explicitly at publish
- `livekit/src/room/options.rs:170`: defaults to `VideoEncoderBackend::Auto`
- `examples/local_video/src/publisher.rs`: the official example defaults to the raw path + an optional backend

## 8. Next Steps (pending user confirmation to implement)

1. Create `vendor/webrtc-sys` (a complete copy of registry 0.3.39)
2. Add the 2-line FFI change (the `request_key_frame` field + the conversion)
3. mediaservo-webrtc exposes `request_key_frame()`
4. The Host frame loop triggers every 2s
5. Measure: keyframe interval ~2s, first frame <2s, e2e regression 4/4

## 9. Related Files

- `docs/reference/webrtc/keyframe-black-screen-analysis.md` — the PIT-65 precursor analysis
- `.agents/memorys/pitfalls.md` — PIT-76 (the complete first-frame-68s root-cause chain)
- `.agents/memorys/conventions.md` — C20 (vendored compliance), C22 (Host bans Docker)
- `.refinfo/rust-sdks/` — the local livekit rust-sdks source reference
- Standard libwebrtc sources (downloaded during research): video_stream_encoder.cc, video_codec.h, video_encoder_config.h, webrtc_video_engine.cc, rtp_sender.cc, rtp_parameters.h, libvpx_vp8_encoder.cc
