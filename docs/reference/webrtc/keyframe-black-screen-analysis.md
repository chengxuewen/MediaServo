# PIT-65 Keyframe Black Screen — Root-Cause Analysis & Architecture Comparison

**Status**: under analysis (2026-08-05) | **Related**: PIT-65 | **Topic**: uncontrolled keyframes on the Host raw-I420 path → multi-page consume black screens

## 0. Experiment Update (x-google-max-keyframe-interval verification result)

**2026-08-05 evidence**: after adding `x-google-max-keyframe-interval=2000` to the `build_remote_sdp` fmtp line and rebuilding the Host, the keyframe time sequence:

```
09:13:48 → 09:13:50 (2s)   # startup ramp burst
09:13:50 → 09:13:55 (5s)   # startup ramp burst
09:13:55 → 09:14:10 (15s)  # startup ramp burst
09:14:10 → 09:14:15 (5s)   # startup ramp burst
09:14:15 → 09:15:54 (99s)  # steady-state fall-back → ~99s GOP
```

**Conclusion: quick validation failed**. The steady-state keyframe interval is still ~99s; the parameter had no effect. The same "startup burst + steady-state fall-back to 99s" pattern as b=AS:500.
**Possible cause**: the fmtp sits on the **remote SDP** (the mediasoup-side description), while the Host's sending encoder reads its keyframe interval from the **negotiated codec parameters of the local answer** — our hand-built SDP bypassed libwebrtc's normal codec-parameter merging, so the parameter never reached OpenH264.

**Next step**: move to option A (diagnose the native PLI breakpoints H1/H2/H3; the architecturally optimal path).

## 1. Symptom

3 browser pages connect sequentially to the mediasoup SFU to pull the stream (the SquaresPattern frame-generator version), each waiting 60s:

- b=AS:2000 baseline: `page1: OK | page2: BLACK | page3: BLACK` (1 success / 2 black)
- b=AS:500 experiment: `page1: BLACK | page2: BLACK | page3: BLACK` (all black)

On the black pages the consumer is created and the transport connected (mediasoup keeps sending sync packets), but **the keyframe never arrives** → syncRequired never clears → video frames are not forwarded.

## 2. Root Cause (confirmed)

**The Host encoder produces a keyframe only every ~99s (GOP ≈ 3000 frames @30fps), and the consume-side PLI requests cannot force one earlier.** A late-joining consumer gets no keyframe within its 60s wait window → black screen.

### Evidence Chain

| Evidence | Location |
|------|------|
| Keyframe interval stable at ~99s | server log `key frame received [ssrc:...]` at 08:32:14→08:33:53→08:35:32... exactly 99s |
| b=AS:500 doesn't change the GOP | 08:46:36-43's 2-5s are the encoder's startup ramp bursts; it later settles back to ~99s (after 08:48:22) |
| mediasoup requests keyframes properly | with `ConsumerOptions.paused=false` mediasoup immediately requests a keyframe from the remote Producer (mediasoup-rs consumer.rs:112-115) |
| The Host doesn't respond to PLI | the consumer connected at 08:49:34; a producer keyframe appears only at 08:50:02 (the ~99s scheduled one); no PLI trigger for 28s |
| Keyframe-interval exclusions | both b=AS:2000 and 500 give ~99s → not bitrate-driven |

## 3. Exclusions (each individually rejected)

- **b=AS**: steady-state GOP ~99s for both 2000 and 500 → not the root cause
- **Encoder frame skipping / seq gaps**: fixed under b=AS:2000 (continuous seq) → not the root cause
- **PLI storm**: the frontend already removed request_key_frame → not the root cause
- **Resolution**: 320x240 is worse → not the root cause
- **peer_id overwrite**: isolated (independent sfuPeerId per page), still black → not the root cause
- **Consumer IsActive**: CONS-DUMP score=10 fully active → not the root cause
- **ICE/DTLS**: all ready → not the root cause

## 4. Architecture Comparison (benchmarked against the official clients)

### How Do the Official Clients Handle Keyframes?

| Client | Method | Keyframe mechanism |
|--------|------|-----------|
| **libmediasoupclient (C++)** | native `MediaStreamTrackInterface*` → `pc->AddTransceiver(track, kSendOnly)` | **Zero keyframe/GOP configuration**; relies entirely on the native libwebrtc encoder, whose native PLI response works |
| **mediasoup-client (JS)** | the browser's native RTCRtpSender/Transceiver | The browser's libwebrtc handles PLI + GOP natively; no such problem |
| **Our Host** | the livekit webrtc-sys custom raw I420 VideoTrackSource | `on_captured_frame` → `AdaptFrame` → `OnFrame`; **completely keyframe-unaware** |

### Key Finding: the raw path and the native track share one encoder pipeline

```
native camera track ──┐
                ├─→ VideoTrack → RtpVideoSender → VideoStreamEncoder → the real encoder (OpenH264)
raw I420 source ──┘              └─ PLI → RequestKeyFrame → IDR (native mechanism)
```

**Both sources go through the same VideoStreamEncoder**. PLI→IDR is an encoder-layer native mechanism, theoretically independent of the source type. This raises the core question:

> **If native PLI works fine in libmediasoupclient, why doesn't it work in our Host?**

My initial judgment (pushing the encoded path) was an **overreaction** — mistaking "the raw path doesn't check `keyframe_request_flag`" for "the raw path doesn't respond to PLI". These are two different mechanisms:
- `keyframe_request_flag` — used only by the encoded path's PassthroughVideoEncoder (application-layer polling)
- **Native PLI → VideoStreamEncoder** — the raw path goes through it the same way and should work normally

### PLI Breakpoints to Diagnose (PIT-50 attribution order)

| Hypothesis | Verification method |
|------|---------|
| H1: the mediasoup Consumer's PLI was never sent (no RTCP PLI producer-side) | check the mediasoup worker's `RTC::Producer` RTCP logs |
| H2: the PLI reached the Host but the webrtc-sys PC didn't process inbound RTCP (RTCP receive not enabled on the sendonly direction) | tcpdump the Host's RTP port + the libwebrtc verbose LogSink (`webrtc_sys::new_log_sink`) |
| H3: the PLI reached the encoder but OpenH264's long GOP ignores PLI | switch to backend-webrtc-rs as a comparison, or read the libwebrtc encoder logs |

## 5. Fix Direction Fork

### Option A: Make the native PLI work (architecturally optimal, mirrors the official side)

Diagnose H1/H2/H3 to locate the PLI breakpoint; fix it so native PLI triggers an IDR. **This is fixing the root against the original design**.

- Pros: keyframes return to normal, no architecture change
- Cons: needs diagnosis first (tcpdump + LogSink), may touch the webrtc-sys PC's RTCP receive configuration

### Option B: Add `x-google-max-keyframe-interval=2000` to the SDP fmtp

A libwebrtc-ecosystem standard mechanism (mediasoup-demo uses it too): configure the software encoder's keyframe interval in the SDP fmtp line.

- Pros: a one-line change, inside webrtc-sys, quick to validate
- Cons: depends on whether OpenH264 parses this Google extension parameter (not RFC standard, risky); still a tactical fix, constrained by the libwebrtc black box

### Option C: encoded path + mediaservo-codec (architecture rework, long-term)

Switch to `capture_encoded_frame` + `take_keyframe_request()`; keyframes become fully application-layer controlled.

- Pros: keyframes 100% explicitly controllable (GOP config + PLI closed loop), extensible (H.265/VP9/AV1/simulcast)
- Cons: a large change (the Host depends on mediaservo-codec + a new path + frame-loop rework)

### Decision Recommendation

Do **option A (diagnose the native PLI breakpoint) first** — the architecturally optimal, root-cause path. C is an overreaction (self-corrected). B is fine for quick validation but should not be the architectural decision.

## 6. TODO

- [ ] Diagnose H1/H2/H3 to locate the PLI breakpoint (tcpdump the Host RTP port + the libwebrtc verbose LogSink)
- [ ] Confirm whether the mediasoup producer actually sends RTCP PLIs (H1)
- [ ] Confirm whether the webrtc-sys PC's sendonly direction enables RTCP reception (H2)
- [ ] After the fix, rebuild Host + server; measure the keyframe interval + multi-page E2E
- [ ] Commit the peer_id isolation fix (uncommitted; architecture correctness, not the black-screen root cause)

## 7. Related Files

- `crates/mediaservo-host/src/sfu_media.rs`: `build_remote_sdp` (b=AS:2000, the fmtp line)
- `crates/mediaservo-host/src/main.rs`: the SFU frame loop (SquaresPattern + absolute timeline, 640x480); the dead `keyframe_interval` config (line 592)
- `crates/mediaservo-webrtc/src/backend/webrtc_sys.rs`: the `WebrtcSysTrack` raw-I420 path (line 587 on_captured_frame)
- `crates/mediaservo-server/src/sfu.rs`: `create_consumer` (line 353)
- `~/.cargo/registry/src/*/webrtc-sys-0.3.41/src/video_track.cpp`: the raw path (167-247) never checks the keyframe flag; the encoded path (309) uses it
- `~/.cargo/registry/src/*/mediasoup-0.24.1/src/router/consumer.rs`: the ConsumerOptions paused comment (112-115); `request_key_frame()` (1234)
- versionsatica/libmediasoupclient `src/Handler.cpp`: `SendHandler::Send()` native track + zero keyframe configuration
- livekit/rust-sdks webrtc-sys `src/passthrough_video_encoder.cpp`: 240-253, the keyframe-request forwarding mechanism
