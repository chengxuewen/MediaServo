# MediaServo Reference Docs Index

> Updated: 2026-09-23 | Organizing principle: the **Diátaxis framework** (living reference / research archive separation)
> Living references are organized to **mirror the product modules** (reference mirrors product structure); research archives sit separately under `research/` (historical research, inert)

## Living Reference (Reference — for lookup, by product module)

### 🌐 WebRTC module — `webrtc/`
| Document | Content |
|----------|---------|
| `webrtc/mediasoup-refs.md` | **Entry point** — official mediasoup source navigation + sync-script usage |
| `webrtc/mediasoup-client.md` | Official client architecture reference (profile/usage/architecture/cases) |
| `webrtc/webrtc-w3c-alignment.md` | Host SFU W3C API alignment rework analysis (incl. team-review corrections) |
| `webrtc/keyframe-black-screen-analysis.md` | PIT-65 keyframe black-screen root-cause analysis |
| `webrtc/gop-control-internal-encoder.md` | **GOP control root cause & fix path** — libwebrtc internal-encoder keyframe mechanics + option comparison + vendor decision |
| `webrtc/sdp-negotiation-bitrate-audit.md` | **SDP negotiation audit + bitrate chain** — Host build-form analysis + BWE feedback-break root cause + option A/C decision (2026-08-12) |

### 🧰 Codec / build strategy — `codec/`
| Document | Content |
|----------|---------|
| `codec/ffmpeg-static-build-strategy.md` | FFmpeg static build strategy (codec's three backends) |
| `codec/build-optimization-strategy.md` | Docker build optimization (layer caching, regional mirrors, lto) |

### 🧩 SDK C++ bindings — `sdk-cxx/`
| Document | Content |
|----------|---------|
| `sdk-cxx/link.md` | Device-side IPC: signaling + frame bus (cxx RAII over the 12-function C face; reconnect/resume/ACK pump semantics) |
| `auth-guide.md` | Client auth master guide (four-credential selection/timing/four-language snippets/ops notes; single source of truth — the four sdk-cxx volumes keep only pointers) | viewer-auth-matrix T6 |
| `sdk-cxx/deck.md` | Capture / record / playback (FFmpeg statically embedded; two warnings: FrameBus is not on the C/C++ face, capture MVP=stub) |
| `auth-guide.md` | Client auth master guide (four-credential selection/timing/four-language snippets/ops notes; single source of truth — the four sdk-cxx volumes keep only pointers) | viewer-auth-matrix T6 |
| `sdk-cxx/field.md` | Device-side push closed loop (PushSession/built-in frame source; two warnings: **no external frame-feed entry**, pull is not on this face) |
| `auth-guide.md` | Client auth master guide (four-credential selection/timing/four-language snippets/ops notes; single source of truth — the four sdk-cxx volumes keep only pointers) | viewer-auth-matrix T6 |
| `sdk-cxx/client.md` | Cockpit/consumer side (S6 batch-1b 28-symbol form; Consumer RAII, negotiation/control DC/e-stop signing surface) |
| `auth-guide.md` | Client auth master guide (four-credential selection/timing/four-language snippets/ops notes; single source of truth — the four sdk-cxx volumes keep only pointers) | viewer-auth-matrix T6 |

### Other living references (root)
| Document | Content |
|----------|---------|
| `janus-gateway.md` | Janus Gateway (WebRTC conferencing gateway) reference |

## Research Archive (Explanation — historical research, not part of active work)

> One-off competitor/tech-selection research notes, never referenced after writing. Archived in their original domain subdirectories, historical reference only.

### `research/`
| Subdirectory | Content |
|--------------|---------|
| `research/remote-desktop/` | anydesk / parsec / rustdesk / moonlight-sunshine / teamviewer |
| `research/streaming/` | mediamtx / srs / zlmediakit / nginx-rtmp-module / pion / obs-studio / lvqr / xiu |
| `research/video-conference/` | mediasoup* / openvidu-* / livekit / jitsi / kurento / zoom |
| `research/teleoperation/` | comma-ai-openpilot / tether-rally / tum-teleoperated-driving / vay |
| `research/network-emulation/` | Weak-network emulation tool landscape (tc/netem family / clumsy / ATC·toxiproxy verified corrections / webrtc test-network·Pion vnet / quick-selection table · pitfall list) |

## WebRTC Living-Reference Relationship Map

```
webrtc/mediasoup-refs.md (source navigation)
   ├── webrtc/mediasoup-client.md (official architecture reference)
   ├── webrtc/webrtc-w3c-alignment.md (alignment rework analysis)
   └── webrtc/keyframe-black-screen-analysis.md (PIT-65 root cause)
        ├── webrtc/gop-control-internal-encoder.md (GOP mechanics + fix path, 2026-08-10)
        └── .agents/memorys/ (C18 official-usage-first, PIT-46/54/56/65)
```

## Official Reference Sources (script-managed)

The official mediasoup client source is cloned into `.refinfo/` (git-ignored), managed by `scripts/sync-official-refs.sh`. See `webrtc/mediasoup-refs.md`.

## Organization Convention (Diátaxis)

| Need | Home | Convention |
|------|------|------------|
| **Reference** (facts/lookup) | `webrtc/` `codec/` or root | Mirrors product modules; restrained, authoritative, unambiguous |
| **Explanation** (research/understanding) | `research/` | Historical research archive; not part of active work |
| **New living reference** | The corresponding module directory | Register in this README |
| **New research** | `research/<domain>/` | Register in this README |

## Notes

- This directory holds **reference/research** material, not project design docs
- Project design docs live in `docs/modules/`; the architecture overview is `docs/architecture.md`
- Research archives (`research/`) are not referenced by code, historical reference only; may be archived/deleted if needed
