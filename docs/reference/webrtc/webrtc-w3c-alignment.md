# WebRTC W3C API Alignment Rework Analysis

> Status: analysis doc (not implemented) | Date: 2026-08-05 | Trigger: the PIT-65 keyframe problem + C18's explicit user requirement
>
> **Conclusion first**: the current Host SFU produce flow bypasses standard negotiation, substituting 5 "hand-built/minimal-interface" spots for the official mediasoup client architecture. This is the architectural root cause of the PIT-65 black screen (`x-google-max-keyframe-interval` landing in the wrong place is exactly the fruit of bypassing negotiation). The rework = aligning with the standard offer/answer negotiation flow of official mediasoup-client / libmediasoupclient.
> **Conclusion first**: the current Host SFU produce flow bypasses standard negotiation. **Key correction (team review): the mediasoup media plane is pure ORTC and the server never returns an SDP answer — the official client builds its own answer** (verified against Chrome74.ts + libmediasoupclient /tmp sources). The rework = aligning with the standard offer/answer negotiation flow of official mediasoup-client / libmediasoupclient.

## 0. Team-Review Corrections (2026-08-05, gap-auditor + official-source evidence)

The initial document had 3 errors, each verified line-by-line against official repo sources:

| # | Initial claim | Official-source evidence | Correction |
|---|---------|-------------|------|
| 1 | §1.1/§3.2 "the server returns an SDP answer" | the mediasoup media plane is pure ORTC and **never returns SDP**. The Chrome74.ts:542-560 answer is client-built (`this._remoteSdp.getSdp()`); libmediasoupclient Handler.cpp:365-369 the same (`remoteSdp->GetSdp()`); this project's server Produce handler replies only `Produced |
| 2 | §3.3 "put x-google-max-keyframe-interval in send_encodings" | webrtc-sys `RtpEncodingParameters` has no such field (rtp_transceiver.rs:21-24); the official injection path = `codecOptions` → the answer's fmtp (MediaSection.cpp:255-277 injects only x-google-{start,max,min}-bitrate | change to the codecOptions path |
| 3 | §3.1 "need to add create_offer/set_local/set_remote" | all three already exist (traits.rs:40,46,49); the real gaps are only `add_transceiver` + `get_sending_rtp_parameters`; webrtc-sys 0.3.41 already exposes `add_transceiver` (peer_connection.rs:171-179) | narrow P1's scope |

**Official-flow evidence (/tmp cloned sources)**: the client is the offerer — `addTransceiver(sendonly) → createOffer → getSendingRtpParameters (derived from the offer) → setLocalDescription(offer) → send rtpParameters and await the produce response → the client builds its own answer (RemoteSdp::Send using the local offerMediaObject + the server's rtpParameters) → setRemoteDescription(answer)`.


## 1. The Official Standard Flow (the comparison baseline)

### 1.1 mediasoup-client JS — the `Chrome74.ts` send() flow

The official `mediasoup-client/src/handlers/Chrome74.ts` (`/tmp/mediasoup-client`) standard produce negotiation:

```ts
// Chrome74.ts send()
const pc = this._pc;

// ① Standard W3C API: addTransceiver (direction + encoding parameters)
pc.addTransceiver(track, {
  direction: 'sendonly',
  sendEncodings: [{ ... }],          // optional simulcast/layers
});

// ② createOffer() lets the browser negotiate local capabilities
const offer = await pc.createOffer();

// ③ Extract the send RTP parameters from the offer's m= section (ortc.getSendingRtpParameters)
//    — RTP parameters are derived from the browser's negotiation result, not hand-built
const rtpParameters = this._extractRtpParameters(offer);

// ④ setLocalDescription(offer) — the standard sequence
await pc.setLocalDescription(offer);

// ⑤ Send the offer → the server produces the answer
// ⑥ await pc.setRemoteDescription(answer) — critical: must go through standard negotiation
```

Key point: **all RTP parameters (ssrc/PT/fmtp) are derived from the negotiation result**. **Note (review correction §0): the mediasoup media plane is pure ORTC and the server never returns an SDP answer — the client builds its own answer** (`RemoteSdp::Send` constructs it from the local offerMediaObject + the server's rtpParameters). The answer in the official `setRemoteDescription(answer)` is client-built, not server-returned.

### 1.2 libmediasoupclient C++ — `SendHandler::Send()`

`/tmp/libmediasoupclient/src/Handler.cpp` — the same W3C pattern:

```cpp
// SendHandler::Send()
// ① Standard W3C: AddTransceiver with direction
pc->AddTransceiver(track, { direction = sendonly });

// ② CreateOffer
auto offer = pc->CreateOffer();

// ③ Derive rtpParameters from the offer
auto rtpParameters = ... // from offer

// ④ SetLocalDescription(offer)
// ⑤ Send the offer and await the answer
// ⑥ SetRemoteDescription(answer)
```

### 1.3 mediasoup-demo — `RoomClient.js` produce

In `/tmp/mediasoup-demo/app/src/RoomClient.js` the produce flow is equally standard: `pc.addTransceiver` → `createOffer` → `setLocalDescription` → send the offer → `setRemoteDescription(answer)` → build rtpParameters from the negotiated send parameters.

## 2. Current Host Implementation vs Official — 5 Deviations

The current flow (`crates/mediaservo-host/src/main.rs:277-361` + `sfu_media.rs`):

```
① pc.setRemoteDescription(hand-built recvonly SDP)   ← direction reversal
② pc.add_track("video")                            ← a non-W3C minimal interface
③ pc.create_answer() + set_local_description       ← answering on the remote offer (roles reversed)
④ Hand-made build_produce_rtp_parameters(ssrc)     ← hardcoded PT=101
⑤ Hand-parsing the ssrc from the answer            ← bypasses standard rtpParameters derivation
```

| # | Current implementation | Official standard | Consequence |
|---|---------|---------|------|
| 1 | Hand-built remote SDP (`a=recvonly`, fprintf-assembled candidate/fmtp) | `addTransceiver(sendonly)` lets the browser generate the offer | Direction reversal; encoder parameters in the wrong place (fmtp on the remote is never read locally); hand-assembled SDP is error-prone |
| 2 | `add_track("video", TrackKind::Video)` with a non-W3C signature | `addTrack(track, stream)` / `addTransceiver(track, {...})` | No sendEncodings capability; direction decided implicitly by setRemoteDescription rather than explicitly |
| 3 | `setRemoteDescription(offer)` then `create_answer()` | `createOffer()` + `setLocalDescription(offer)` + `setRemoteDescription(answer)` | Role inversion: the Host acts as the callee when mediasoup is the offerer; negotiation semantics misplaced |
| 4 | Hand-made `build_produce_rtp_parameters(ssrc)` hardcoding H264 PT=101 + fixed parameters | `ortc.getSendingRtpParameters()` derived from the offer | PT/parameters may diverge from the negotiation result (the PIT-54 double-hardcode lesson); addTransceiver's sendEncodings unused |
| 5 | `negotiated_ssrc_from_sdp(&answer.sdp)` hand-parses the ssrc | Take it directly from the send parameters tracked by addTransceiver | Bypasses standard negotiation; ssrc hardcoded vs the negotiated value drifting |

### The Deviations' Chain of Damage (PIT-65 evidence)

- **`x-google-max-keyframe-interval=2000` has no effect**: the parameter sits on a `build_remote_sdp` fmtp line (the remote SDP), but libwebrtc reads the keyframe interval from the **local answer's negotiated codec parameters**. Since the Host does `setRemoteDescription(hand-built recvonly offer)` → the local answer lacks the parameter → it is dropped → the GOP stays ~99s
- **The PLI keyframe-request chain is broken**: the direction reversal (a recvonly offer) bypasses libwebrtc's send-direction negotiation; the PLI → RequestKeyFrame → IDR feedback chain is incomplete on the Host side (PIT-65 §root cause confirmed: the Host had no keyframe response for 28s)

## 3. Rework Plan (aligned with official)

### 3.0 Principles

- **mediaservo-webrtc exposes the complete W3C WebRTC API only** (C18): `addTransceiver`/`addTrack`/`createOffer`/`setLocalDescription`/`setRemoteDescription`/`onicecandidate`/`ontrack`/`getSendingRtpParameters` all present; trimming to a minimal interface is forbidden.
- **The Host SFU uses standard offer/answer negotiation** (aligned with libmediasoupclient / mediasoup-client / mediasoup-demo).
- **ssrc/PT/fmtp all derived from the negotiation result**; hand-made JSON rtp_parameters + hand-assembled SDP forbidden.

### 3.1 mediaservo-webrtc: complete the W3C API surface

`crates/mediaservo-webrtc/src/backend/traits.rs`'s `PeerConnectionApi` needs additions/alignment:

```rust
// W3C standard interfaces (natively supported by webrtc-rs; the webrtc-sys backend fills them at the FFI layer)
async fn add_transceiver(&self, kind: TrackKind, init: RTCRtpTransceiverInit) -> Result<RTCRtpTransceiver>;
// RTCRtpTransceiverInit { direction: Sendonly/Recvonly/Sendrecv, send_encodings: Vec<RTCRtpEncodingParameters> }
async fn create_offer(&self, options: RTCOfferOptions) -> Result<RTCSessionDescription>;
async fn set_remote_description(&self, desc: &RTCSessionDescription) -> Result<()>;
async fn set_local_description(&self, desc: &RTCSessionDescription) -> Result<()>;
fn on_ice_candidate(&mut self, cb: ...);   // complete trickle forwarding (the PIT-43 empty stub)
fn get_sending_rtp_parameters(&self, track_id: &str) -> Result<RtpParameters>;  // derived from the negotiation result
```

### 3.2 The Host's Standard produce Flow (aligned with Chrome74.ts)

Replaces `main.rs:277-361`'s `setRemoteDescription(hand-made) → add_track → create_answer → build_produce_rtp_parameters`:

```
① pc.add_transceiver(Video, { direction: Sendonly, send_encodings: [H264 encode] })
   // Review correction: the official side doesn't inject keyframe-interval (RtpEncodingParameters has no such field); bitrate uses x-google-max-bitrate or send_encodings.max_bitrate
② let offer = pc.create_offer()
③ pc.set_local_description(&offer)
④ let rtp_parameters = pc.get_sending_rtp_parameters(track_id)   // derived from the offer, not hand-made
⑤ Send rtp_parameters → server produce → returns Produced{producer_id} (no SDP answer)
⑥ The client builds its own answer (mirroring the local offer's codec parameters + the server's rtpParameters) → set_remote_description(answer)
   // once the standard sequence completes, the PLI → RequestKeyFrame → IDR feedback chain is complete
⑦ the write_raw_i420 frame loop (SquaresPattern, unchanged)
```

### 3.3 The Correct Home for Keyframe Parameters

- **The official side doesn't use `x-google-max-keyframe-interval`** (review correction §0): webrtc-sys `RtpEncodingParameters` has no such field, and the official MediaSection.cpp:255-277 injects only x-google-{start,max,min}-bitrate. Keyframes rely on the **standard PLI→RequestKeyFrame chain** (once aligned the Host is offerer+sendonly, and libwebrtc responds natively)
- Bitrate control: send_encodings.max_bitrate or codecOptions x-google-max-bitrate (the official path), replacing today's hand-injected b=AS.
- No more reliance on hand-made remote fmtp (the current dead path; parameters on the remote are dropped by the local answer).

### 3.4 Phased Implementation

| Phase | Content | Verification |
|------|------|------|
| P1 | mediaservo-webrtc adds `add_transceiver`/`create_offer`/`get_sending_rtp_parameters` (traits + the webrtc-sys backend) | `cargo test -p mediaservo-webrtc --features backend-webrtc-sys` |
| P2 | The Host SFU produce moves to the standard negotiation flow | E2E screenshot rendering + keyframe interval < 5s |
| P3 | Verify the multi-page black screen (PIT-65) is fixed | `/tmp/multi-page.cjs` 3 pages + first-frame latency |
| P4 | sfu-client.ts (browser) aligns with the official mediasoup-client consume flow | Browser consume works |

## 4. Risks & Trade-offs

- **webrtc-sys backend interface gaps**: `add_transceiver`/`create_offer`/`get_sending_rtp_parameters` may not be exposed at the webrtc-sys FFI layer; fill them in `webrtc_sys.rs` (the livekit sdk should support it; follow official usage).
- **P2P path regression**: the P2P relay also goes through `PeerConnectionApi`; after changing the trait, P2P E2E regression is required.
- **This is an architecture change**: per edit-safety.md's architecture decision gate, **implement only after user confirmation**. This document is analysis only.

## 7. Related Files

- `crates/mediaservo-host/src/main.rs:277-361` — the current SFU produce flow
- `crates/mediaservo-host/src/sfu_media.rs` — `build_remote_sdp` / `build_produce_rtp_parameters` / `negotiated_ssrc_from_sdp`
- `crates/mediaservo-webrtc/src/backend/traits.rs` — the W3C interfaces `PeerConnectionApi` needs
- `crates/mediaservo-webrtc/src/backend/webrtc_sys.rs` — the webrtc-sys backend implementation
- `/tmp/mediasoup-client/src/handlers/Chrome74.ts` — the official send() flow baseline
- `/tmp/libmediasoupclient/src/Handler.cpp` — the official C++ client baseline
- `/tmp/mediasoup-demo/app/src/RoomClient.js` — the official demo baseline

---

## 5. W3C APIs That Cannot Be Implemented (flagged for future)

> Plan: 2026-08-06 v2 (after the team review + the W3C benchmarking audit) | Basis: measured webrtc-sys 0.3.x FFI inventory (§6)
> The following APIs are **not implemented** for lack of FFI or browser exclusivity; flagged only, for future triggers:

| API | Reason | Future trigger |
|-----|------|---------|
| `RTCDTMFSender` | webrtc-sys has no DTMF FFI (new C++ needed) | Remote control / telephony gateway scenarios |
| `setIdentityProvider` / `RTCIdentityAssertion` | webrtc-sys has no identity FFI | DTLS certificate-level identity auth |
| `generateCertificate` (static) | libwebrtc manages certificates internally and automatically; unnecessary | Custom certificate fingerprints |
| `getDefaultIceServers` (static) | Browser-exclusive | none |
| the `sctp` attribute / `RTCSctpTransport` | Browser-exclusive | none |
| `RTCRtpScriptTransform` / `RTCEncodedFrame` | Browser-exclusive (insertable transforms) | Server-side E2E encryption (already covered by FrameCryptor) |
| `addStream` / `createDTMFSender` / `removeStream` | Deprecated (obsolete) | never |
| `RTCRtpReceiver.getContributingSources/getSynchronizationSources` | webrtc-sys has no CSRC/SSRC list FFI | Stats/monitoring scenarios |
| the `RTCRtpSender/Receiver.transport` attributes | webrtc-sys exposes no DTLS transport handle | Transport diagnostics |

**Note (v2 correction)**: the Sender's `set_parameters`/`replace_track`/`set_streams`/`get_stats` and the Receiver's `get_parameters` **are all implementable** (the webrtc-sys FFI has them), falling within host-sfu-w3c-alignment P1/P2 (implemented, D214) and are **not** flagged as future.

**mediaservo-webrtc covered/will cover**: all other W3C RTCPeerConnection / RTCRtpTransceiver / RTCRtpSender / RTCRtpReceiver / RTCDataChannel / RTCRtpCapabilities interfaces (host-sfu-w3c-alignment P0-P2, implemented, D214).

---

## 6. The webrtc-sys 0.3.x FFI Capability Inventory (complete)

> Measured 2026-08-06 | Sources: `/tmp/webrtc-investigation/rust-sdks/webrtc-sys/src/`

### 6.1 PeerConnection (peer_connection.rs, 18 methods)

```
set_configuration(config) -> Result
create_offer(options, ctx, on_success, on_error)   // async callbacks
create_answer(options, ctx, on_success, on_error)  // async callbacks
set_local_description(desc, ctx, on_complete)
set_remote_description(desc, ctx, on_complete)
add_track(track, stream_ids) -> Result<SharedPtr<RtpSender>>
remove_track(sender) -> Result
get_stats(ctx, on_stats)  // async callback, JSON
add_transceiver(track, init) -> Result<SharedPtr<RtpTransceiver>>
add_transceiver_for_media(media_type, init) -> Result<SharedPtr<RtpTransceiver>>
get_senders() -> Vec<RtpSenderPtr>
get_receivers() -> Vec<RtpReceiverPtr>
get_transceivers() -> Vec<RtpTransceiverPtr>
create_data_channel(label, init) -> Result<SharedPtr<DataChannel>>
add_ice_candidate(candidate, ctx, on_complete)
restart_ice()
current_local_description() -> UniquePtr<SessionDescription>
current_remote_description() -> UniquePtr<SessionDescription>
connection_state / signaling_state / ice_gathering_state / ice_connection_state
close()
// Enums: PeerConnectionState, SignalingState, IceConnectionState, IceGatheringState, ContinualGatheringPolicy, IceTransportsType
// Structs: RtcOfferAnswerOptions, IceServer, RtcConfiguration
```

### 6.2 RtpTransceiver (rtp_transceiver.rs, 16 methods)

```
media_type() -> MediaType
mid() -> Result<String>
sender() -> SharedPtr<RtpSender>
receiver() -> SharedPtr<RtpReceiver>
stopped() / stopping() -> bool
direction() -> RtpTransceiverDirection
set_direction(direction) -> Result
current_direction() -> Result
fired_direction() -> Result
stop_standard() -> Result
set_codec_preferences(codecs) / codec_preferences()
header_extensions_to_negotiate() / negotiated_header_extensions()
set_header_extensions_to_negotiate(headers)
// Structs: RtpTransceiverInit { direction, stream_ids, send_encodings }
```

### 6.3 RtpSender (rtp_sender.rs, 12 methods)

```
set_track(track) -> bool
track() -> SharedPtr<MediaStreamTrack>
get_stats(ctx, on_stats)
ssrc() -> u32
media_type() -> MediaType
id() -> String
stream_ids() -> Vec<String>
set_streams(stream_ids)
init_send_encodings() -> Vec<RtpEncodingParameters>
get_parameters() -> RtpParameters
set_parameters(parameters) -> Result
set_video_encoder_backend(backend)
```

### 6.4 RtpReceiver (rtp_receiver.rs, 8 methods)

```
track() -> SharedPtr<MediaStreamTrack>
get_stats(ctx, on_stats)
stream_ids() / streams()
media_type() -> MediaType
id() -> String
get_parameters() -> RtpParameters
set_jitter_buffer_minimum_delay(is_some, delay_seconds)
```

### 6.5 Parameter types (rtp_parameters.rs)

```
RtpParameters { transaction_id, mid, codecs, header_extensions, encodings, rtcp, degradation_preference }
RtpEncodingParameters { ssrc, bitrate_priority, network_priority, max_bitrate_bps, min_bitrate_bps, max_framerate, num_temporal_layers, scale_resolution_down_by, scalability_mode, active, rid, adaptive_ptime }
RtpCodecParameters { mime_type, name, kind, payload_type, clock_rate, num_channels, max_ptime, ptime, rtcp_feedback, parameters }
RtpCapabilities { codecs, header_extensions, fec }
RtpCodecCapability { mime_type, name, kind, clock_rate, preferred_payload_type, num_channels, rtcp_feedback, parameters }
RtpHeaderExtensionCapability { uri, preferred_id, preferred_encrypt, direction }
RtcpParameters { ssrc, cname, reduced_size, mux }
RtpExtension { uri, id, encrypt }
// Enums: FecMechanism, RtcpFeedbackType, RtcpFeedbackMessageType, DegradationPreference, RtpExtensionFilter
```

### 6.6 Factory（peer_connection_factory.rs）

```
create_peer_connection_factory()
create_peer_connection(config, observer)
create_video_track(label, source) / create_audio_track(label, source)
rtp_sender_capabilities(kind) -> RtpCapabilities
rtp_receiver_capabilities(kind) -> RtpCapabilities
// Observer callbacks: on_signaling_change / on_renegotiation_needed / on_negotiation_needed_event / on_ice_connection_change / on_connection_change / on_ice_gathering_change / on_ice_candidate / on_ice_candidate_error / on_ice_candidate_...
```

### 6.7 DataChannel（data_channel.rs）

```
send(data) / close() / id() / label() / state() / buffered_amount()
// Enums: Priority, DataState | Structs: DataChannelInit, DataBuffer
// Observer: on_state_change / on_message / on_buffered_amount_change
```

### 6.8 Opaque pointer wrappers (helper.rs)

```
MediaStreamPtr / CandidatePtr / AudioTrackPtr / VideoTrackPtr / RtpSenderPtr / RtpReceiverPtr / RtpTransceiverPtr
// Purpose: work around cxx limits (SharedPtr<T> can't go directly into a rust::Vec)
```
- `docs/reference/webrtc/keyframe-black-screen-analysis.md` — the PIT-65 keyframe analysis
