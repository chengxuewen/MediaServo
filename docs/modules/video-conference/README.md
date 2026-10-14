# MediaServo Video-Conference Module — Reference Docs

> Phase 0 architecture definition | 2026-07-16

> **Doc version**: written 2026-07-16 (before the D126-D155 decisions). Later D126 (three-layer model), D137-D139 (WebRTC backends), and D142 (integration plan) may affect this document's content; read against the latest decisions.

This document is the reference manual for MediaServo's video-conference (VC) module, covering architecture, API, signaling protocol, data model, deployment, and SFU configuration.

## Document Index

| Document | Content |
|----------|---------|
| [architecture.md](architecture.md) | Component architecture, SFU engine design, Worker model, cascading strategy |
| [api.md](api.md) | Conference SDK API reference (Rust/TS), pipeline and event models |
| [signaling.md](signaling.md) | WebSocket signaling protocol, room management, SDP/ICE negotiation |
| [data-model.md](data-model.md) | Room/Participant/Track models, encoding layers, permissions |
| [deployment.md](deployment.md) | Four deployment forms and configuration reference |
| [sfu-config.md](sfu-config.md) | SFU tuning, Worker allocation, bandwidth management, capacity planning |

## Design Principles

1. **Media layer separate from control layer** — the SFU only forwards RTP, no decode, no transcode. Signaling and business logic are independent.
2. **SFU first** — both audio and video go through SFU forwarding. MCU mixing is used only for PSTN bridging and recording bridging.
3. **Worker isolation** — each mediasoup Worker is an independent process; a crash does not affect other conferences.
4. **Cascade, not full mesh** — cross-region via pipeToRouter cascading, avoiding N×N connection explosion.
5. **Client-side mixing** — the client decodes multiple audio streams and mixes locally; the server does not mix.

## Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| SFU engine | mediasoup (C++) | Best performance, Worker isolation, signaling-agnostic |
| Signaling | In-house WebSocket + gRPC | Flexible and controllable; modeled on Colibri2 + PSRPC |
| Audio | SFU forwarding + client mixing | Avoids MCU scalability bottlenecks |
| Recording | RTP Forwarding | SFU stream duplication pushed to a recording service |
| Binding layer | napi-rs | Rust -> Node.js native module |

See the [research doc](../../research/video-conference.md) for the research process and comparative analysis.
