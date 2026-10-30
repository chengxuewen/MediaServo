# SDK Layers & Architecture

> MediaServo's third-party integration SDK uses the **four-SDK master architecture**: link (connection) / field (composed sessions) / client (cockpit-side consumption orchestration) / deck (media data).
> Related decisions: D222-D234 (2026-08-13/14, superseding the D65-D69/D82 dual-facade model; D230-232 revised to static direct-link/composed SDK/consumer positioning; D233 single-layer session-style API; D234 calling conventions)
> **Public API contract**: [20-sdk-api-contract.md](20-sdk-api-contract.md) (link/field/client/deck interface signatures + C ABI binding forms)

---

## Architecture Overview

```
mediaservo-link (connection surface, pure Rust)      mediaservo-deck (media data surface, dual form)
  ▲ rlib                                               ├─ rlib: standalone app use
  │                                                    ├─ rlib full: field static dep (**default**)
mediaservo-webrtc (abstraction layer, C12)             └─ cdylib: deck-full.so (optional dlopen, OTA)
  ▲ backend-webrtc-sys / backend-webrtc-rs
  │
mediaservo-field (composed SDK = webrtc + link + deck)
  ├─ Media: push/pull (backend-webrtc-sys)
  ├─ Comms: signal/frame_bus/auth (re-export link)
  └─ Capture: MediaDevices/VideoSource (re-export deck[source])
      │ static dependency (merged at build time, self-contained at runtime)
      ▼
mediaservo-client (cockpit-side consumption orchestration) ── depends on field ── (+ deck playback integration)
```

Dependency direction is strictly one-way and acyclic: `field → webrtc + link + deck`; `client → field`; deck independent in dual form.

## 1. Responsibilities of the Four SDKs

| SDK | crate | Responsibility | C ABI Prefix |
|-----|-------|----------------|--------------|
| **link** | `mediaservo-link` | frame_bus (two-sided frame bus + Registry), signal (WS signaling client), auth integration (reuses common PSK/JWT), dc (Phase 2, webrtc backend) | `mediaservo_link_*` |
| **field** | `mediaservo-field` | **Composed SDK**: push/pull (webrtc via mediaservo-webrtc, C12) + re-export link (SignalClient/FrameBus/auth) + re-export deck (MediaDevices/VideoSource/CameraSource) — one-line dependency, complete closed loop | `mediaservo_field_*` |
| **client** | `mediaservo-client` | **Consumption orchestration**: VideoRenderer (GPU interop), multi-session orchestration, Input Forward/telemetry bindings, deck playback integration | `mediaservo_client_*` (batch-1b final form, 28 symbols, see note below) |
| **deck** | `mediaservo-deck` | source (camera/mic/desktop, GStreamer), codec (FFmpeg static), record (mux to disk), playback (replay/fast-forward) | `mediaservo_deck_*` |

> **client C ABI batch-1b final-form note (S6)**: the prefix was fully renamed `ms_client_*` → `mediaservo_client_*` (⚠ breaking,
> zero aliases). 28 symbols = free functions 5 (login as flattened-parameter form + strerror static text) · sessions 12 (new
> state/on_state state observation, wait_video pure wait, consume multi-stream form, error/last_error handle error slots
> — `mediaservo_client_error_t{struct_size,code,wire_code,retryable}` machine-readable bits) · consumers 3
> (consumer id/stats/close, each with an independent pump, no cross-impact) · control 8 (on_ack accumulating form + token, off_ack
> deregistration, ready_state/buffered_amount backpressure observation, producer_ids needed-form upgrade). ⊘ old forms kept one
> cycle (behavior preserved byte for byte): global last_error / first-stream consume_video / session union video_stats; the login
> config struct form is retired (struct kept one cycle). The list's source of truth =
> `bindings/c/mediaservo-client-c/include/mediaservo/client.h`
> (the ABI audit gate check-abi-drift unifies all four faces on the `mediaservo_<sdk>_` prefix).

## 2. deck Dual Form & Feature Slices

| Form | Delivery | Usage |
|------|----------|-------|
| **rlib static** | `mediaservo-deck` (full) | Standalone app use (capture+record+playback closed loop, no WebRTC) |
| **rlib static full** | field static dependency (**default**) | Embedded in the field composed SDK (FFmpeg without OpenSSL → no BoringSSL conflict) |
| **cdylib plugin** | `deck-full.so` (built separately in CI) | Optional dlopen (only if standalone OTA upgrade / swappable implementation needs arise) |

field→deck **static direct-link is the default** (D231 revising D224): FFmpeg's default features exclude `build-lib-openssl` (verified) → zero overlap with libwebrtc(BoringSSL) → the primary dlopen motivation (static symbol isolation) disappears; dlopen remains an optional OTA evolution (D13 PluginManager carrier, same pattern as Janus; interface reserved, not implemented in MVP).

## 3. Consumption Matrix

| Consumer | Dependency | Capabilities |
|----------|------------|--------------|
| Vehicle push integrator | `field` | Push + capture (deck[source]) + signaling + frame bus, one-line dependency |
| ROS nodes / frame subscription / pure control | `link` | Frame subscription, signaling, DC (zero media dependency) |
| Capture + record (local surveillance) | `deck` | Direct capture and record, no WebRTC |
| Push + record (same process) | `field` + deck (static full) | Sessions + codec/record-playback (static direct link) |
| Cockpit app / tablet | `client` | Pull + render + playback (deck playback statically integrated) |

## 4. Process Topology

- **Vehicle side** (rigid multiprocess, host supervisor orchestrates): capture-worker (deck source → FrameBus) / push-worker (field subscribe → webrtc-sys push) / recorder-worker (Phase 2: link+deck encode-record) / control-worker (link only: signaling+DC+emergency). Process boundary = permission boundary = crash surface; control survives independently of media processes.
- **Cockpit side** (single process by default + FrameBus cross-process on demand): interactive terminal single process, multi-session; after pull+decode it publishes via FrameBus, third-party apps subscribe link-only.

## 5. FrameBus (link, two-sided)

- Frame metadata FlatBuffers (width/height/format/timestamps/**is_keyframe** + monotonic/epoch **dual clocks**) + raw pixel memory
- Semantics: latest-frame overwrite (immune to backlog/startup order); multiple subscribers consume independently
- Three scenarios share it: vehicle capture chain / cockpit decode distribution / ROS perception subscription

## 6. Conflict Discipline (continuing PIT-71, updated 2026-08-13)

1. FFmpeg static symbol isolation: deck's FFmpeg must not enable `build-lib-openssl`/`build-zlib` (verified: openssl absent by default → zero overlap with BoringSSL); field→deck is a **static default dependency** (not a plugin)
2. zlib/third-party: FFmpeg uses the system dynamic zlib; x264/openh264/opus intersections with the libwebrtc dependency set verified item by item
3. Capture goes GStreamer dynamic (D64 v4l2src) + Rust doesn't export symbols by default → zero overlap with BoringSSL
4. Plugin loading RTLD_LOCAL (GLOBAL forbidden); the main process exports no symbols (no -rdynamic); version handshake (dlopen/OTA mode only)
5. Link-conflict matrix in CI: field standalone / deck standalone / field+deck[source] static / **field+deck-full static (new default)** / (future) field+deck-full plugin

## 7. Application-Layer Consumers

| Application | Dependencies | Notes |
|-------------|--------------|-------|
| `mediaservo-host` | field + link (+optional deck plugin/workers) | Vehicle device supervisor (orchestrates workers), dogfooding the four SDKs |
| `mediaservo-client` | client SDK (lib + bin dual target) | Cockpit GUI (Tauri v2), the SDK's first consumer |
| `mediaservo-server` | common/webrtc (P2P relay) | Signaling + relay + SFU (C21: no coupling with the SDK) |

## 8. Binding System & Directory Layout

**Binding naming family** (D227): C=`-c`, C++=`-cxx`, Python=`-py` (dual backend: plain ctypes + pyo3 bottleneck), future Android=`-jni`. Bindings are created as each SDK lands (YAGNI).

```
crates/      → core+SDK+apps (link/field/deck/client SDK cores + the existing 7)
bindings/c/  → mediaservo-{link,field,deck,client}-c    (workspace member)
bindings/cxx/→ mediaservo-{link,field,deck,client}-cxx  (workspace member)
bindings/python/ → mediaservo_{link,field,deck,client}/ (not a cargo member, pure py)
```

For the link-conflict discipline and cross-language comparison tests (c/cxx/py/pyo3 asserting the same operation sequence), see spec §7, §11. For dependency-era semantics (build time vs runtime), see spec §15.

## 9. Future Extensions (Phase 3+)

- Video conference / live streaming / surveillance sub-scenario SDKs: scenario APIs layered over the field session surface (ConferenceSession etc.)
- Android JNI: unified C ABI (the mediaservo_* family) feeding thin Java/Kotlin wrappers (livekit JNI_OnLoad pattern)
- Python/C++ bindings: thin wrapper packages for link/field/deck/client each (release layer split as needed)

## 10. Key Technology Choices (argued)

| Item | Conclusion | Basis |
|------|------------|-------|
| DataChannel (Phase 2) | webrtc-rs (backend already exists, SFU e2e 4/4) | libdatachannel = fourth backend + C++ crash surface, excluded |
| Control channel | MVP goes server relay WS (implemented); DC is the Phase 2 low-latency enhancement | 07-protocols four-channel semantics unchanged |
| Capture implementation | GStreamer (D64 proven on all platforms, incl. Jetson CSI/RTSP) | webrtc-sys has no camera-capture bridge |
| Desktop capture | deck source backend: FFmpeg/GStreamer first; webrtc-sys desktop_capturer optional | The latter already exists (added by LiveKit) |
| Local recording (Phase 2) | Dual encoding (main/sub streams, industry NVR standard): recorder fixed high quality + push BWE-adaptive | A shared encode would let BWE pollute recording quality |
| FFmpeg backend | No openssl/network features (local processing needs no HTTPS) → zero symbol overlap with BoringSSL | PIT-71 conflict root resolved (D231) |

## 11. Evolution Roadmap

| Phase | Scope |
|-------|-------|
| **MVP** | link + field + deck[source] (push chain: capture→FrameBus→webrtc-sys→SFU); no deck codec/record-playback; host single-process composed chain proven |
| **Phase 2** | deck lands (source/codec/record/playback) static direct-link; recorder-worker (dual encode); link matrix in CI |
| **Phase 3** | client SDK consumer delivery (rendering/multi-session); cockpit FrameBus publishing; binding system lands (c/cxx crates + py dual backend); Android JNI |

## 12. Dependency-Era Semantics (build time vs runtime)

| Dependency | Build time | Runtime |
|------------|-----------|---------|
| link (rlib) | Statically merged into the field binary | Self-contained (zero extra .so, pure Rust) |
| deck (rlib) | Statically merged into the field binary | Rust code self-contained; **GStreamer system libraries are a runtime environment dependency** (not a crate) |
| webrtc (.a + BoringSSL) | Statically merged | Self-contained |
| deck-full.so (optional plugin/OTA) | Not linked | Loaded at runtime in OTA mode only |

**Deployment form**: vehicle-side field = a single executable (containing all link+deck+libwebrtc code) + the GStreamer system environment. **The only dynamic dependency entry point = GStreamer**; everything else is static.

## 13. deck vs codec (not a replacement, D229)

deck depends on codec (facade); the codec engine stays independent — no merger. If deck absorbed codec, the lower abstraction would depend back on the upper SDK (layer inversion). Precedent: FFmpeg's own split libraries (libavcodec engine / libavformat+libavdevice wrappers — deck's record/playback exactly plays the latter pair's role). deck = the media-processing layer above codec, absorbing the layer's semantics but not its implementation.
