# Recording & Playback Architecture

> Status: Phase 0 architecture design
> Related decisions: D34–D40
> Depends on: 08-pipeline-model.md, 09-transport-architecture.md

## Overview

Recording and playback are MediaServo's cross-scenario foundational capabilities, covering five product areas: remote desktop, video conferencing, surveillance cameras, streaming push/pull, and teleoperation. Recording/playback reuse the PipelineEngine's `MediaSource → MediaProcessor → MediaSink` model — Recording is one implementation of a Sink; Playback is one implementation of a Source.

## 1. Three Recording Locations

```
                 Recording location
                 ══════════════════

  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
  │ In-pipeline      │  │ SFU egress       │  │ Client local     │
  │ (RecordingSink)  │  │ (SFU side hook)  │  │ (local encode +  │
  │                  │  │                  │  │  storage)        │
  ├──────────────────┤  ├──────────────────┤  ├──────────────────┤
  │ Reuses the       │  │ Direct bypass    │  │ Recording does   │
  │ PipelineEngine;  │  │ inside the SFU;  │  │ not depend on    │
  │ any stream that  │  │ same-process     │  │ the server;      │
  │ passes through   │  │ zero-overhead    │  │ local disk first;│
  │ the pipeline can │  │ copy; no WebRTC  │  │ optional async   │
  │ be recorded      │  │ handshake        │  │ upload to S3     │
  ├──────────────────┤  ├──────────────────┤  ├──────────────────┤
  │ Suitable for:    │  │ Suitable for:    │  │ Suitable for:    │
  │ Remote desktop   │  │ Conference       │  │ Local remote-    │
  │ screen recording │  │ recording        │  │ desktop capture  │
  │ Teleoperation    │  │ Streaming DVR    │  │ Local teleop     │
  │ logs             │  │                  │  │ logs             │
  │ Surveillance     │  │                  │  │                  │
  │ camera recording │  │                  │  │                  │
  └──────────────────┘  └──────────────────┘  └──────────────────┘
```

**SFU bypass recording architecture** (distinct from LiveKit's "hidden peer" mode):

```
SfuRelayPlugin (PipelineNode)
    │
    ├──▶ Forwards to each participant's WebRTC peer
    │
    └──▶ RecordingEgress (same pipeline)
         │
         ├── Compositor (mixed scenarios)   → GStreamer compositor
         ├── Encoder                        → NVENC/VAAPI H.264
         └── splitmuxsink                   → fMP4 segments → S3/MinIO
```

## 2. Recording Granularity

| Scenario | Default granularity | Optional |
|----------|---------------------|----------|
| Video conference | Mixed MP4 (gallery/speaker layout) | Per-stream raw |
| Remote desktop | Single-stream screen recording | — |
| Surveillance camera | Single stream per camera | — |
| Stream receiving | Single-stream DVR | — |
| Teleoperation | MP4 video + JSONL control log | SEI embedding (evidence-grade) |

## 3. Container Format: fMP4 + splitmuxsink

fMP4 (Fragmented MP4) is used across the board, natively supported by GStreamer's `splitmuxsink`.

```
Pipeline output → splitmuxsink
                    │
                    ├── max-size-time: 3600s (1h segment)
                    ├── muxer-factory: mp4mux
                    ├── sink-factory: filesink
                    ├── async-finalize: true (gapless segmentation)
                    └── location: session_%d.mp4
```

### Part-Segment Two-Layer Model (modeled on MediaMTX)

```
session_1730000000/
├── 001.mp4    ← Segment 1 (0:00–1:00)
├── 002.mp4    ← Segment 2 (1:00–2:00)
└── 003.mp4    ← Segment 3 (2:00–3:00)
```

- **Part**: the smallest recording unit (~1s); a crash loses at most 1 second
- **Segment**: the file-organization unit (~1h), easing cleanup, upload, and playback
- **splitmuxsink behavior**: at each segment end it async-finalizes the current muxer and starts a new one, gapless

## 4. Compositing Architecture

```
Participant A ──▶ SFU ─┐
Participant B ──▶ SFU ─┤
Participant C ──▶ SFU ─┼──▶ GStreamer compositor
                       │     ├── Decode N H.264 streams → YUV
                       │     ├── Compose frames (gallery/speaker layout)
                       │     ├── Overlay watermarks (timestamp, participant name)
                       │     └── NVENC/VAAPI → mixed H.264
                       │
                       └──▶ splitmuxsink → fMP4 segments
```

- The `compositor` element supports GPU acceleration (CUDA zero-copy path: decode→compose→encode entirely in GPU memory)
- Composition latency: ~10ms (hardware path)
- Layout config: gallery (N×N grid) / speaker (speaker + thumbnails)

## 5. WebRTC Recording Tap Point

Recording happens **after the MediaTransport trait output**, i.e. at the media layer, not the transport layer.

```
MediaTransport::poll_output()
    │
    ▼ TransportOutput::Media(frame)
    │
    ▼ PipelineEngine
        ├── MediaProcessor: scale/color convert (if needed)
        ├── MediaProcessor: SEI injection (teleoperation scenario)
        └── RecordingSink: a MediaSink implementation
```

**Why the media layer**:
- Decoupled from the transport backend (invisible to str0m / libwebrtc / webrtc-rs)
- Handles Simulcast automatically (records only the highest layer) and FEC/RTX (retransmitted packets not recorded)
- SVC streams keep the full bitstream (layers can be dropped at playback)
- Recording starts aligned to a keyframe (first seconds dropped, standard behavior)

## 6. Teleoperation Multi-Track Recording

### Default: separated files

```
session_001/
├── video_001.mp4          ← H.264 video stream
└── control_001.jsonl      ← Control command log
```

JSONL format:
```jsonl
{"ts": 1730000000001, "type": "steering", "angle": 15.2, "speed": 30.0}
{"ts": 1730000000050, "type": "brake", "pressure": 0.7}
{"ts": 1730000000100, "type": "throttle", "value": 45.0}
```

### Optional: SEI Embedding (evidence-grade)

```
H.264 bitstream:
  [SPS][PPS][SEI: control_state][IDR][SEI: control_state][P][SEI: control_state]...
      ↑                      ↑
  control-state snapshot    frame-exact binding, impossible to separate or tamper
  attached to every frame
```

- SEI NAL unit type 6, ignored by standard players
- Dedicated replay tooling can extract the SEI for frame-accurate sync
- SEI injection is a `SeiInjector` MediaProcessor; no architecture change

## 7. Playback Architecture

```
Recording artifacts          Playback method               Playing end
──────────────              ───────────────               ──────────
fMP4 segments ──────▶ HLS playlist ────────────▶ Browser <video>
                      (nginx/S3 static serving)  Admin web portal

Local fMP4 files ───▶ FrameServer ────────────▶ Client App
                      (precise seek)            Remote-desktop screen replay
                                                Teleoperation incident review
```

- **HLS streaming**: segments are HLS fragments directly; the `m3u8` playlist is assembled by nginx/S3 and played natively in the browser's `<video>`
- **Local playback**: FrameServer provides timeline seek, variable-speed playback, and frame stepping
- **No on-demand transcoding needed**: no Phase 1 requirement; Cloudflare Stream or similar can serve later

## 8. Storage & Lifecycle

```
Storage: S3 / MinIO / local disk
Path template: recordings/{room_id}/{timestamp}_{seq}.mp4
Retention policy: configurable per scenario
  - Video conference: 30 days default
  - Surveillance: 7-day rolling overwrite
  - Teleoperation: incident sessions retained permanently
  - Remote desktop: user-managed
```

## 9. Project References

| Reference Product | What We Borrow |
|-------------------|----------------|
| LiveKit Egress | GStreamer compositor compositing, template filenames, multi-output formats |
| mediasoup | PlainTransport recording mode (SFU bypass validated) |
| GStreamer splitmuxsink | Gapless segmentation, async-finalize, mp4mux factory |
| MediaMTX | Part-Segment model, path templates, auto cleanup |
| OBS Studio | Subprocess-isolated muxer, replay buffer (future reference) |

**Phase 2+ addition**: fMP4 + splitmuxsink recording as a first-class pipeline path (D34-D40). Phase 1 recording is a simple MediaSink implementation; Phase 2+ promotes recording into an independent, composable first-class pipeline node supporting compositing, SEI embedding, multi-track parallel recording, and other advanced capabilities.
