# LVQR Reference Analysis
> Generated: 2026-07-16 | Category: Streaming

## 1. Product Profile
- **Name**: LVQR (Live Video QUIC Relay)
- **Developer**: virgilvox (independent developer, GitHub @virgilvox)
- **First release**: 2026-04-10 (GitHub repo created); v1.0.0 on 2026-05-03; v1.1.0 on 2026-05-27
- **Product positioning**: A unified real-time media relay server. A single Rust binary implements RTMP/WHIP/SRT/RTSP/WebSocket fMP4 input and LL-HLS/MPEG-DASH/WHEP/MoQ/WebSocket fMP4 output. The core idea is to eliminate the N×M protocol conversion matrix through a unified fragment model (Unified Fragment Model).
- **Target audience**: live platform developers, backend engineers needing multi-protocol interop, architects pursuing low-latency large-scale distribution, and streaming engineers interested in frontier technologies such as QUIC/MoQ
- **License / business model**: AGPL-3.0, purely open source. No commercial license option. Most of the 29 crates are published to crates.io; the TypeScript SDK is published to npm

## 2. Technical Characteristics
### Overall architecture
```
┌──────────────────────────────────────────────────────────────────┐
│                     LVQR single binary                          │
│                                                                │
│  ┌──────────┐ ┌──────┐ ┌─────┐ ┌──────┐ ┌─────────┐           │
│  │  RTMP    │ │ WHIP │ │ SRT │ │ RTSP │ │ WS fMP4 │  ← input   │
│  │ (1935)   │ │(443) │ │(UDP)│ │(TCP) │ │ (WS)    │  protocols │
│  └────┬─────┘ └──┬───┘ └──┬──┘ └──┬───┘ └────┬────┘           │
│       │          │        │       │          │                 │
│       └──────────┴───┬────┴───────┴──────────┘                 │
│                      │ each input → Fragment stream             │
│                      ▼                                          │
│         ┌──────────────────────────┐                            │
│         │ FragmentBroadcasterRegistry│  ← broadcast registry    │
│         │  (broadcast, track) →     │                            │
│         │   FragmentBroadcaster     │                            │
│         └──────────┬───────────────┘                            │
│                    │ Observer taps                              │
│         ┌──────────┼──────────────────────────┐                 │
│         ▼          ▼          ▼               ▼                 │
│    ┌────────┐ ┌────────┐ ┌────────┐     ┌─────────┐            │
│    │LL-HLS  │ │ DASH   │ │  WHEP  │     │ MoQ/QUIC│ ← output   │
│    │partials│ │segments│ │RTP pkt │     │ relay   │ protocols  │
│    └────────┘ └────────┘ └────────┘     └─────────┘            │
│         │          │          │               │                 │
│         ▼          ▼          ▼               ▼                 │
│    ┌────────┐ ┌────────┐ ┌────────┐     ┌─────────┐            │
│    │Recording│ │WASM filt│ │AI Agent│     │Mesh P2P │            │
│    │(redb)  │ │(wasmtime)│(whisper)│    │offload94%│           │
│    └────────┘ └────────┘ └────────┘     └─────────┘            │
│                                                                │
│  Cluster plane (optional, feature-gated)                        │
│  ┌──────────────────────────────────────────────────┐          │
│  │ chitchat gossip → broadcast ownership(lease) → redirect │  │
│  │ → cross-cluster federation → eventual consistency (no Raft)││
│  └──────────────────────────────────────────────────┘          │
└──────────────────────────────────────────────────────────────────┘
```

### LVQR's three-plane architecture

LVQR's architecture is divided into three orthogonal planes:

**Data plane (core)**:
- The only internal media type is `Fragment { track_id, group_id, object_id, priority, dts, pts, duration, flags, payload }`
- `track_id` identifies the track (video/audio/subtitles), `group_id` maps to the MoQ subgroup, and `object_id` maps to the MoQ object sequence number
- `flags` carries flags such as `keyframe` (IDR/I-frame), `independent` (independently decodable), `discardable` (droppable)
- `payload` is `Bytes`, carrying the binary data of a CMAF chunk / fMP4 segment
- The `ingest_time_ms` field records the ingestion timestamp, used to compute glass-to-glass latency
- `FragmentBroadcasterRegistry` manages broadcaster handles keyed by `(broadcast, track)`
- `FragmentBroadcaster` is a single-producer multi-subscriber (SPMC) fan-out structure
- The `FragmentStream` trait is the async interface through which all Fragment producers and consumers interact

**Cluster plane (optional)**:
- chitchat gossip protocol (UDP), using a SWIM protocol variant for member discovery
- Broadcast ownership = lease, 10s lease period, 2.5s renewal interval
- Explicitly rejects Raft/leader election — linearizability is not a design goal
- When a client connects to a non-owner node, a 302 redirect to the owner node is returned
- Supports cross-cluster federation (feature-gated) by exchanging cluster routing information over chitchat

**Observability plane**:
- An independent SLO threshold system per transport
- Glass-to-glass latency histogram (from `ingest_time_ms` to `egress_emit_ms`)
- Client-push latency sampling endpoint `POST /api/v1/slo/client-sample`
- MoQ end-to-end latency is implemented via a sidecar `/0.timing` track
- OTLP tracing support, metrics can be fanned out to Prometheus

### Key technical capabilities
| Capability | Details |
|------|------|
| Protocol support | Input: RTMP (TCP 1935), WHIP/WebRTC (HTTPS 443), SRT (UDP), RTSP/1.0 (TCP), WebSocket fMP4 (WS 8080); Output: LL-HLS (HTTP 8888), MPEG-DASH (HTTP), WHEP/WebRTC (HTTPS), MoQ/QUIC/WebTransport (UDP 4443), WebSocket fMP4 (WS) |
| Coding | H.264, H.265(HEVC), AV1, Opus, AAC, FLAC (carried via CMAF/fMP4 containers). All codecs are handled in passthrough mode — no decode/re-encode |
| Transport | MoQ over QUIC/WebTransport — a first-class citizen transport (not bolted on afterwards); WebRTC RTCDataChannel (Mesh P2P relay); native TCP/UDP support per protocol |
| Recording | Archive storage indexed by redb (pure Rust embedded database), supporting DVR sliding-window playback and time-range retrieval. Recording is written directly from the Fragment stream without repackaging |
| Media processing | WASM filter runtime (wasmtime, per-fragment hot reload); in-process AI Agent framework (Whisper caption generation: AAC → PCM → WebVTT); GStreamer ABR transcoding (four hardware backends NVENC/VAAPI/QSV/AMF + runtime ladder add/remove) |
| SCTE-35 support | SCTE-35 markers are extracted from RTMP `onCuePoint` (AMF0) and MPEG-TS PMT 0x86 and passed through to the player as-is. Supports server-side ad insertion |
| Security/signing | C2PA content provenance signing (signed automatically at record time, verifies media provenance); Auth supports noop/static/api modes |
| Clustering | chitchat gossip protocol, broadcast ownership leases (10s lease, 2.5s renewal), redirect to the owner node, cross-cluster federation, Swarm Discovery |
| Mesh P2P | Browser-side WebRTC RTCDataChannel mesh relay. The first 30 viewers connect directly to the server (root peer), later viewers relay from other viewers. Self-balancing tree topology, dead-node detection by heartbeat |
| Observability | Glass-to-glass latency histogram, per-transport independent SLO thresholds (LL-HLS p50 500ms, WHEP p50 100ms, MoQ p50 80ms), OTLP tracing, Prometheus metrics |

### Performance SLO matrix
| Transport | p50 | p95 Warning | p99 Critical |
|-----------|-----|-------------|--------------|
| LL-HLS | 500ms | 1,500ms | 2,000ms → 4,000ms |
| DASH | 1,000ms | 3,000ms | 4,000ms → 8,000ms |
| WHEP (WebRTC) | 100ms | 250ms | 500ms → 1,000ms |
| MoQ | 80ms | 200ms | 400ms → 800ms |
| WS (fMP4) | 300ms | 800ms | 1,200ms → 2,500ms |

### Technology stack
- **Language**: Rust (77.4%), HTML (8.5%), TypeScript (7.8%), Vue (4.2%), Python (1.8%)
- **WebRTC stack**: str0m — a sans-I/O WebRTC implementation, used for WHIP ingest and WHEP egress
- **MoQ transport**: moq-lite — a Rust implementation of the MoQ (Media over QUIC) protocol
- **WASM runtime**: wasmtime — the per-fragment filter execution environment
- **Archive engine**: redb — a pure Rust embedded key-value database, used for the recording index
- **Cluster protocol**: chitchat — a Rust gossip protocol implementation
- **Transcoding engine**: GStreamer — used for ABR ladder generation, supporting NVENC/VAAPI/QSV/AMF
- **Client SDK**:
  - `lvqr-core` (Rust crate): shared types such as StreamId, TrackName, EventBus, RelayStats
  - `@lvqr/core` (TypeScript/npm v1.1.0): MoQ-Lite subscriber (WebTransport), WebSocket fMP4 fallback, MeshPeer relay, complete admin API client
  - `@lvqr/admin-ui` (Vue 3 SPA v1.1.1): the full admin console — Dashboard, Streams, DVR, Ingest Listeners (runtime STOP), Transcode Ladders (runtime add/remove), Agents (runtime start/stop), SLO, Cluster, Mesh, Federation, Auth, Provenance, SSE log tail
- **Build/deploy**: Cargo workspace (29 crates), a single binary `lvqr serve`, zero external dependencies

### Input protocol → Fragment normalization flow

LVQR's core innovation is that every input protocol converts wire data into the unified Fragment type through specific bridging logic. Below is the complete mapping from each input protocol to the Fragment's nine fields:

**RTMP input → Fragment mapping**:
```
RTMP stream (FLV tag sequence)
  |
  v
FLV demuxer parses tag type (audio/video/script) and timestamp
  |  video tag → H.264 NAL / HEVC NAL / AV1 OBU
  |  audio tag → AAC frame / Opus frame
  |  script tag → AMF0 onCuePoint (SCTE-35 bin64)
  |
  v
Fragment {
  track_id: TrackId::Video(0) / TrackId::Audio(0),  // inferred from tag type
  group_id: 0,                                          // RTMP has no subgroup concept, always 0
  object_id: sequence_number,                            // FLV tag sequence number
  priority: 0,                                           // RTMP has no priority, always 0
  dts: flv_tag.timestamp (ms),                           // 24/32-bit timestamp carried by FLV
  pts: flv_tag.timestamp + composition_offset,           // only video needs compensation
  duration: calculated_from_consecutive_tags,            // time difference between two tags on the same track
  flags: FragmentFlags::KEYFRAME if video keyframe,      // FLV frame type decision
  payload: Bytes::from(video/audio raw data),            // raw encoded frame
  ingest_time_ms: Instant::now(),                        // current time
}
```
Key points of the RTMP→Fragment bridge: FLV timestamps are in milliseconds, and both dts and pts are based on them. RTMP has no MoQ subgroup and object hierarchy, so group_id is always 0 and object_id degenerates to a sequence number.

**WHIP/WebRTC input → Fragment mapping**:
```
WHIP session (str0m RTP stream)
  |
  v
RTP depacketizer separates audio/video tracks by SSRC
  |  H.264: depacketize STAP-A/Single NAL Unit (RFC 6184)
  |  HEVC: depacketize AP/NAP (RFC 7798)
  |  Opus: depacketize (RFC 7587)
  |
  v
Fragment {
  track_id: TrackId::Video(ssrc) / TrackId::Audio(ssrc),  // identified by SSRC
  group_id: rtp_sequence_number >> 8,                     // MoQ subgroup maps to the high byte of the RTP sequence number
  object_id: rtp_sequence_number & 0xFF,                  // MoQ object maps to the low byte of the sequence number
  priority: 0,                                            // WHIP does not carry priority
  dts: rtp.timestamp,                                     // RTP timestamp (90kHz for video, 48kHz for audio)
  pts: rtp.timestamp,                                     // no PTS/DTS separation within an RTP frame
  duration: next_rtp_timestamp - current_timestamp,       // frame duration
  flags: KEYFRAME if H.264 IDR / HEVC IDR / AV1 Key,     // decided from the NAL unit type
  payload: Bytes::from(aggregated NAL units / frames),    // aggregated encoded frame
  ingest_time_ms: Instant::now(),
}
```
Key points of the WHIP→Fragment bridge: RTP timestamps are in 90kHz (video) or 48kHz (audio) and map directly to the Fragment dts field. The str0m library's RTP depacketizer already outputs complete encoded frames, so no reassembly is needed.

**SRT input → Fragment mapping**:
```
SRT stream (MPEG-TS over SRT)
  |
  v
MPEG-TS demuxer parses PAT/PMT to get audio/video PIDs
  |  PES packet unpacking → extract H.264 NAL / AAC frames
  |  PMT 0x86 → SCTE-35 splice_info_section()
  |
  v
Fragment {
  track_id: TrackId::Video(pmt_video_pid) / TrackId::Audio(pmt_audio_pid),  // mapped from PMT PID
  group_id: 0,                                          // MPEG-TS has no subgroup
  object_id: mpeg_ts_continuity_counter,                // continuity counter
  priority: transport_priority,                          // MPEG-TS transport_priority bit
  dts: pes.dts,                                          // DTS carried in the PES header
  pts: pes.pts,                                          // PTS carried in the PES header
  duration: calculated_from_pcr,                         // difference of adjacent PCRs
  flags: KEYFRAME if H.264 IDR,                          // decided from the NAL
  payload: Bytes::from(es_data),                         // raw PES payload
  ingest_time_ms: Instant::now(),
}
```
Key points of the SRT→Fragment bridge: MPEG-TS PCR (Program Clock Reference) provides a precise clock base, and the PES PTS/DTS fields map directly to the Fragment timing fields. SCTE-35 markers are extracted from the PMT 0x86 stream without affecting the main Fragment data stream.

**RTSP input → Fragment mapping**:
```
RTSP session (interleaved RTP over TCP)
  |
  v
RTSP SETUP determines the transport mode (TCP/UDP interleaved)
RTP unpacking logic is similar to WHIP, but RTSP adds SDP negotiation
  |
  v
Fragment {
  track_id: TrackId::Video(sdp_media_index) / TrackId::Audio(sdp_media_index),  // mapped from the SDP media index
  group_id: rtp_sequence_number >> 8,                     // same policy as WHIP
  object_id: rtp_sequence_number & 0xFF,
  priority: 0,
  dts: rtp.timestamp,
  pts: rtp.timestamp,
  duration: calculated,
  flags: KEYFRAME if IDR,
  payload: Bytes::from(nal_units),
  ingest_time_ms: Instant::now(),
}
```
Key points of the RTSP→Fragment bridge: RTSP's SDP negotiation provides codec parameters (fmtp lines), clock rates, SSRC and other information, and this metadata must be attached during the initialization phase of the FragmentStream. RTSP supports both TCP and UDP transport modes, but the bridging logic above the transport layer is unified.

**Fragment field characteristics after normalizing the five input protocols**:

| Field | RTMP | WHIP | SRT | RTSP | WS fMP4 |
|------|------|------|-----|------|---------|
| track_id | FLV tag type | SSRC | PMT PID | SDP index | fMP4 track ID |
| group_id | 0 (constant) | RTP seq >>8 | 0 (constant) | RTP seq >>8 | MoQ group |
| object_id | tag sequence | RTP seq &0xFF | continuity | RTP seq &0xFF | MoQ object |
| priority | 0 (constant) | 0 (constant) | transport_pri | 0 (constant) | MoQ pri |
| dts | FLV timestamp | RTP timestamp | PES DTS | RTP timestamp | fMP4 decode time |
| pts | FLV+offset | RTP timestamp | PES PTS | RTP timestamp | fMP4 pts |
| payload | NAL/AAC | NAL/Opus | PES ES | NAL/Opus | CMAF chunk |
| Codec overhead | none | none | none | none | none |

**Key conclusion**: bridging the five input protocols to Fragment involves no codec operations. Every field mapping is a mechanical format conversion — copying a metadata field from a wire format to the corresponding Fragment field. This is the foundation of the Unified Fragment Model's zero-overhead performance.

## 3. Feature Overview
### Core feature modules

| Module | Function | Dependencies |
|------|------|------|
| `lvqr-core` | StreamId, TrackName, EventBus, RelayStats — shared types with zero internal dependencies | none |
| `lvqr-fragment` | Fragment model, FragmentMeta, FragmentBroadcasterRegistry, FragmentStream trait, MoqTrackSink | lvqr-core, lvqr-moq |
| `lvqr-moq` | Facade crate over moq-lite, isolating moq-lite version changes. newtype wrapping of Track/Group/Object | lvqr-core |
| `lvqr-cmaf` | CMAF segmenter — generates the trinity of HLS partials, DASH segments, MoQ groups from the Fragment stream | lvqr-fragment |
| `lvqr-ingest` | RTMP + FLV parsing, AMF0 onCuePoint scte35-bin64 handling, RtmpMoqBridge. Converts input data to Fragment | lvqr-fragment, lvqr-cmaf |
| `lvqr-whip` | WebRTC ingest via str0m. Supports H.264 + HEVC + Opus coding. Generates Fragment streams | lvqr-fragment |
| `lvqr-srt` | SRT-over-UDP input + MPEG-TS demuxer + PMT 0x86 SCTE-35 reassembly | lvqr-fragment |
| `lvqr-rtsp` | RTSP/1.0 server + interleaved RTP depacketizing. Generates Fragment streams | lvqr-fragment |
| `lvqr-hls` | LL-HLS output + MultiHlsServer + master playlist + sliding DVR window + DATERANGE tags | lvqr-fragment, lvqr-cmaf |
| `lvqr-dash` | MPEG-DASH output + MultiDashServer + MPD lifecycle management + Period EventStream | lvqr-fragment, lvqr-cmaf |
| `lvqr-whep` | WebRTC egress via str0m. RTP packaging, AAC→Opus real-time transcoding | lvqr-fragment |
| `lvqr-mesh` | P2P mesh topology planner. Browser-side WebRTC RTCDataChannel data plane | lvqr-core, lvqr-signal |
| `lvqr-relay` | MoQ/QUIC relay over moq-lite, zero-copy fan-out | lvqr-fragment, lvqr-moq |
| `lvqr-cluster` | chitchat gossip member management, broadcast ownership, capacity claims, config sync, federation routing | lvqr-core |
| `lvqr-wasm` | wasmtime per-fragment filter runtime + hot reload. Implements the FragmentObserver trait | lvqr-fragment |
| `lvqr-agent` | AI Agent framework (trait + runner). Runs AI tasks inside the data plane | lvqr-fragment |
| `lvqr-agent-whisper` | WhisperCaptionsAgent: AAC audio → PCM decode → Whisper inference → WebVTT captions | lvqr-agent |
| `lvqr-transcode` | GStreamer ABR ladder. Software encoding + 4 hardware backends. Runtime ladder add/remove | lvqr-fragment |
| `lvqr-cli` | Single binary composition root. `lvqr serve` starts all protocols | all protocol crates |
| `lvqr-test-utils` | TestServer harness — full-stack test instances on ephemeral ports | lvqr-cli |
| `lvqr-conformance` | Reference test fixtures + external validator wrappers | lvqr-test-utils |
| `lvqr-soak` | Long-running stress test driver | lvqr-cli |

### Signature features
- **Unified Fragment Model**: every track is a sequence of Fragments, every protocol is a projection of a Fragment stream. Adding a protocol ≈ about 50 lines of bridging code. MoQ subgroups, LL-HLS partials, CMAF chunks, DASH segments, WHEP RTP packets, disk recording — all are the same representation of the same data
- **SCTE-35 ad marker passthrough**: SCTE-35 is extracted from RTMP `onCuePoint` and MPEG-TS PMT 0x86 and passed to the player as-is. This is broadcaster-grade ad insertion capability, extremely rare in the open-source streaming space
- **C2PA content provenance**: C2PA signatures are applied automatically when archiving recordings, verifying media provenance and integrity. Content authentication infrastructure for the age of AI-generated content
- **Mesh P2P bandwidth offload**: at 500 viewers the server serves only about 120 Mbps (94% offloaded). Tree topology, self-balancing, dead-node detection by heartbeat. Each peer manages child connections and frame forwarding through the `MeshPeer` API
- **In-process AI Agent**: Whisper caption generation runs inside the data plane, requiring no external HTTP service or GPU server. The Agent framework supports custom Agent implementations (implementing the Agent trait) and runtime Agent add/remove
- **Runtime hot configuration**: hot-reload WASM filters, runtime transcode ladder add/remove, runtime AI agent add/remove, runtime START/STOP ingest listeners — all without interrupting existing connections

### Extensibility / plugin mechanism
LVQR has no plugin system in the traditional sense (no dlopen, no WASM interface registry). Its extension model is achieved through Rust crate composition:
- **Adding an input protocol**: implement a crate that produces `Fragment` values and register it in `lvqr-cli::start` to the `FragmentBroadcasterRegistry`. The crate must implement the `FragmentStream` trait
- **Adding an output protocol**: implement a `FragmentObserver` / `RawSampleObserver` tap and subscribe to the `FragmentBroadcaster`. The crate installs the Observer onto the corresponding broadcast track in the registry
- **Data plane extensions**: WASM filters (implementing the `FragmentFilter` trait, compiled to .wasm files, hot-loaded through wasmtime) and AI Agent trait implementations (compiled on the Rust side, not WASM)
- **Rust compile-time composition**: all protocols are compiled into a single binary through Cargo feature flags. `lvqr-cli` is the only composition root, and every other crate is a library target usable independently in tests
- **TypeScript SDK extensions**: `@lvqr/core` provides a MoQ-Lite subscriber, WebSocket fMP4 fallback, MeshPeer relay, and a complete admin API client. `@lvqr/admin-ui` supports third-party plugin injection via `window.__LVQR_ADMIN_PLUGINS__`
- **Admin API coverage**: the `/api/v1/*` REST API covers broadcast management, transcode ladder management, agent management, ingest listener management, archive queries, config reload, auth management, SLO queries, cluster status, mesh status

### Fragment type hierarchy

LVQR's core innovation lies in the Fragment type system. Below is the mapping from Fragment to each transport protocol:

```
Fragment {
  track_id: TrackId       ────► MoQ track alias / HLS variant / DASH AdaptationSet
  group_id: u64           ────► MoQ subgroup sequence
  object_id: u64          ────► MoQ object sequence / HLS partial segment number
  priority: u8            ────► MoQ object priority (0=highest)
  dts: i64                ────► decode timestamp for all projections
  pts: i64                ────► presentation timestamp for all projections
  duration: u32           ────► frame/segment duration in timescale units
  flags: FragmentFlags    ────► keyframe→HLS #EXT-X-INDEPENDENT-SEGMENTS
                           │     independent→WHEP PLI response boundary
                           │     discardable→MoQ drop-on-overflow
  payload: Bytes          ────► CMAF chunk (common to HLS/DASH/MoQ/WHEP)
  ingest_time_ms: u64     ────► glass-to-glass latency measurement
}
```

The design goal of this struct is that a single Fragment value simultaneously satisfies the packaging requirements of all output protocols, with no re-parsing or re-packaging of the payload content at egress time.

## 4. Status & Ecosystem
- **Current version**: v1.1.0 (2026-05-27). Crate versions: lvqr-core 1.1.0, lvqr-fragment 1.1.0, lvqr-cli 1.1.0. TypeScript: @lvqr/core 1.1.0, @lvqr/admin-ui 1.1.1 (patch: fixes the Vue compiler configuration issue in the DVR view)
- **GitHub Stars / activity**: 5 stars (as of 2026-04), repo created 2026-04-10, still in an early stage. 3 releases. Medium commit frequency, latest commit 2026-05-27
- **Community size**: extremely small. The developer is the author himself (virgilvox). No external contributors. No community forum, Discord, or mailing list
- **Documentation / SDK / API ecosystem**:
  - Rust crates: lvqr-core, lvqr-fragment, lvqr-moq, lvqr-ingest, lvqr-whip, lvqr-srt, lvqr-rtsp, lvqr-hls, lvqr-dash, lvqr-whep, lvqr-relay, lvqr-mesh, lvqr-cluster, lvqr-wasm, lvqr-agent, lvqr-transcode, lvqr-cli and 29 crates in total published to crates.io
  - TypeScript SDK: `@lvqr/core` (npm), `@lvqr/admin-ui` (npm), a complete REST API client
  - Documentation: GitHub README (with the full architecture diagram), docs/architecture.md (29 crate map + 10 core design decisions), docs/quickstart.md (build from source + deployment guide), docs/mesh.md (deep dive into the mesh topology planner), tracking/ROADMAP.md (Tier 0-4 roadmap)
  - Integration tests: `lvqr-test-utils::TestServer` provides full-stack test instances on ephemeral ports. All E2E tests use the same composition root path. OBS can be used as an external publisher for integration testing
  - Benchmarks: `lvqr-soak` long-running stress test driver (publish = false, not published externally)
  - Conformance tests: `lvqr-conformance` reference fixtures + external validator wrappers (publish = false)
  - Administration: admin API (`/api/v1/*`), `/metrics` (Prometheus), `/healthz`, SSE real-time log tail
- **Known defects or limitations**:
  - **Extremely early project**: 5 stars, a single developer, no production deployment cases. Any update to a key dependency could cause breaking changes
  - **AGPL-3.0 license**: network viral. Any derivative work using LVQR code must be open-sourced under AGPL as well. Obvious limitations for closed-source commercial use
  - **No commercial support**: no enterprise edition, SLA, paid support channel, or commercial license option
  - **Immature ecosystem**: no third-party plugins, community forum, user conference, training material, or certification program
  - **Opaque performance data**: no publicly available independent benchmark. The performance SLOs (p50/p95/p99) are design targets, not measured data. No third-party stress test report
  - **MoQ protocol not yet standardized**: MoQ is an IETF draft (draft-ietf-moq-transport), and the wire format may change incompatibly during standardization. LVQR isolates part of the risk through the `lvqr-moq` facade crate
  - **Advanced features such as SCTE-35/C2PA**: described only in the README and roadmap, and their actual completion state has not been publicly verified
  - **Rust compile time**: the compile time of the 29 crates in CI is unknown. A large workspace may slow down development iteration

## 5. Market Positioning
- **Main application industries**: live streaming technology infrastructure, real-time video distribution, low-latency streaming, CDN edge nodes, backends for content creation tools
- **Competitive comparison summary**:
| Dimension | LVQR | MediaMTX | SRS | LiveKit | Ant Media | AWS KVS |
|------|------|----------|-----|---------|-----------|---------|
| Core language | Rust | Go | C/C++ | Go/TypeScript | Java | C++/Java |
| Number of protocols | 10 (5 in 5 out) | 10+ | 8 | WebRTC only | 6+ | 5+ |
| Internal model | **Unified Fragment** | Path-based | **RTMP-centric** | Room/Track | RTMP-centric | Fragment+timestamp |
| Clustering | gossip federation | read replica | origin-edge | built-in | origin-edge | built-in cloud service |
| MoQ/QUIC support | ✅ first-class citizen | ✅ v1.19.0 | ❌ | ❌ | ❌ | ❌ |
| License | AGPL-3.0 | MIT | MIT | Apache 2.0 | commercial | commercial |
| Stars | 5 | 20K | 29K | 30K+ | 3K+ | N/A (cloud service) |
| Maturity | early prototype | production ready | production ready | production ready | production ready | production ready |
| Admin UI | Vue 3 SPA (complete) | none (API only) | Oryx console | built-in Dashboard | built-in Dashboard | AWS Console |
| WASM filtering | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| AI Agent | ✅ (Whisper) | ❌ | ❌ | ❌ | ❌ | ❌ |
| C2PA signing | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
- **Pricing / license**: AGPL-3.0 (network viral), no commercial license option. All crates and npm packages are free

## 6. Product Highlights
1. **Unified Fragment Model — the essential innovation**: LVQR's most core differentiating competitiveness. It is not "supports more protocols" but "there is only one internal media type". `Fragment { track_id, group_id, object_id, priority, dts, pts, duration, flags, payload }` — one struct serving simultaneously MoQ subgroups, LL-HLS partial segments, DASH period segments, and WHEP RTP packets. This is the kundalini (single source of truth) for 10 protocols
2. **A one-shot kill of the N×M problem**: traditional media servers need to interface each newly added protocol with every existing protocol — N inputs × M outputs = N×M converters. LVQR reduces the problem to N+M: each input produces Fragments, each output consumes Fragments. Adding a new protocol does not require touching any existing protocol code
3. **Zero virtual dispatch in the data plane**: the control plane uses `async-trait` (one heap allocation per connection), the data plane uses concrete types (zero heap allocation per Fragment). This is the performance-critical decision in Rust implemented via enum dispatch and generics. No Fragment hand-off path (potentially thousands per second) involves a virtual function call from entry to exit
4. **Single binary zero-config startup**: `lvqr serve` is one command, automatically binding RTMP (1935)/WHIP (443)/SRT (UDP)/RTSP (TCP) inputs and LL-HLS (8888)/DASH/WHEP/MoQ (4443)/WS fMP4 (8080) outputs. No external Redis, Kafka, segmenter, or signaling server dependency. From cloning the repo to push-and-play in under 5 minutes
5. **Mesh P2P 94% bandwidth offload**: browser-side WebRTC RTCDataChannel mesh relay. The first 30 viewers connect directly to the server, later viewers relay from other viewers. Tree topology self-balancing, dead-node detection by heartbeat, automatic reconnection. At 500 viewers, server bandwidth drops from about 2000 Mbps to about 120 Mbps

## 7. Reference Value for MediaServo

### [Adopt] Directly Adoptable
1. **The Unified Fragment Model is the D5 blueprint for MediaServo's pipeline model**: the Fragment type definition (including the field semantics of track_id, group_id, object_id, pts/dts, flags, payload) can be mapped directly onto MediaServo's `MediaFragment` type. The `(broadcast, track)` key design of FragmentBroadcasterRegistry maps onto PipelineEngine multi-stream management. The FragmentObserver trait maps onto the observer pattern between plugins
2. **Data plane / control plane separation strategy**: LVQR's principle is that the control plane uses `async-trait` (allowing one dynamic allocation per connection) and the data plane uses concrete types and enum dispatch (zero heap allocation per Fragment). MediaServo's `Plugin` trait (control plane) and `MediaFrame` handling (data plane) should follow exactly the same separation
3. **Cargo workspace organization**: the dependency principles followed by the 29 crates (no circular dependencies, `lvqr-core` with zero internal dependencies, `lvqr-cli` as the only composition root) provide a validated template for MediaServo's workspace structure. The `lvqr-moq` facade crate pattern of isolating external dependency version changes should also be adopted
4. **chitchat gossip cluster model**: simpler than Raft/Paxos but sufficient. Broadcast ownership = lease rather than lock. For MediaServo's media distribution scenario (where linearizability is not required), the eventual consistency model is the right choice
5. **The glass-to-glass latency SLO system**: per-transport independent three-tier SLO thresholds (p50/p95/p99). Different latency tolerances are set per transport characteristics (MoQ 80ms vs HLS 500ms). MediaServo can reuse this SLO metrics design directly

### [Adapt] Needs Modification Before Adoption
1. **Fragment payload format selection**: LVQR uses CMAF/fMP4 uniformly as the Fragment payload. MediaServo needs to evaluate two paths: (a) CMAF/fMP4 (consistent with LVQR, better universality), (b) raw encoded frames (raw H.264 NAL units / raw AAC frames, the GPU zero-copy encoding path). A possible approach is for Fragment to carry a `payload_format` enum field supporting both payload formats
2. **WASM filters → Rust native plugins**: MediaServo's plugin system is based on Rust traits and does not need a WASM sandbox. But if multi-tenant scenarios require user-defined filters, the hot reload and isolation capabilities of a WASM sandbox are more valuable. Consider reserving a `WasmFilterPlugin` trait as a future extension point
3. **AI Agent framework adaptation**: LVQR's Agent trait + runner pattern provides a reference implementation for in-process AI computation. MediaServo's AI needs may include visual content recognition (safety monitoring), audio event detection, and real-time translation captions. The Agent injection point should be on the Fragment stream (consuming Fragments, producing metadata + a new Fragment track)
4. **MoQ facade crate pattern**: LVQR's `lvqr-moq` facade crate strategy of isolating moq-lite version changes must be adopted. Any external protocol dependency of MediaServo should be isolated through a facade crate, avoiding version changes propagating across the whole workspace
5. **C2PA signing → ProvenancePlugin**: signing at record time in LVQR is a good pattern. MediaServo should design a `ProvenancePlugin` trait supporting multiple signing backends (C2PA is the preferred one, but future possible standards should also be supported). Initially it can be an optional feature not affecting the core pipeline
6. **Mesh P2P → optional advanced feature**: the bandwidth cost savings are significant (94%), but client complexity increases (maintaining a WebRTC RTCDataChannel mesh topology). MediaServo should make it a feature-gated optional module. See LVQR's `lvqr-mesh` topology planner (topology assignments generated on the Rust side, data relaying executed in the browser)

### [Avoid] Known Pitfalls / Not Applicable
1. **AGPL-3.0 license risk**: LVQR is AGPL-3.0, MediaServo is Apache 2.0. No LVQR code can be copied directly. However, referencing its architecture design, interface definitions, and type structures does not raise copyright issues. The key is to write the implementation code independently
2. **Single-developer dependency risk**: LVQR is a personal project and its design decisions have not been reviewed by a community. Some design choices (such as chitchat's 10s lease interval) may just be the author's preference. MediaServo should keep independent engineering judgement for every design decision
3. **Unverified performance data**: LVQR's SLO numbers are design targets, not measured data. MediaServo must build its own performance benchmark system (refer to the structure of `lvqr-soak`, but write independent Rust benchmarks)
4. **MoQ protocol stability risk**: MoQ is currently an IETF draft (draft-ietf-moq-transport) and the wire format may change incompatibly. If MediaServo integrates MoQ support, it must isolate version changes through a facade crate (`mediaservo-moq-bridge`)
5. **GStreamer coupling risk**: LVQR binds GStreamer as the only implementation for ABR transcoding. MediaServo should design a `Transcoder` trait supporting multiple backends: GStreamer (general-purpose), FFmpeg (CLI-compatible), custom NVENC/VAAPI bridges (zero-copy). GStreamer is the preferred but not the only choice
6. **Compile overhead of 29 crates**: for MediaServo's Phase 0-1 stage, 29 crates may be too many. Every crate carries compile time, API documentation maintenance, and dependency version management costs. Start with 10-15 crates and split gradually as features grow

**Overall rating**: ★★★★★ (5/5)

LVQR's Unified Fragment Model is the **D5 top-priority reference** for MediaServo's pipeline model — highest priority at the architecture design level. Although the project is in an extremely early stage (5 stars, created April 2026) and its AGPL-3.0 license is incompatible with MediaServo's Apache 2.0, its core design ideas — a single internal media type (Fragment), protocols as projections, N+M complexity in place of N×M — constitute the only architecture in the current open-source streaming space that genuinely solves the multi-protocol interop problem.

---
**Related decisions**: D5 (Unified Fragment Model), D21 (timestamps)

## Appendix A: Fragment Model Deep Dive

### A.1 The N×M problem and its solution

Traditional media servers have an N×M protocol conversion matrix: every input protocol must interface with every output protocol. Assume 5 input protocols and 5 output protocols — 25 converters are required.

LVQR turns N×M into N+M:
- The N input protocols produce Fragment streams and write them into FragmentBroadcasterRegistry
- FragmentBroadcasterRegistry broadcasts Fragments to all Observers
- The M output protocols subscribe to Observers and consume Fragment streams

Effort to add a new protocol:
- Input protocol: implement the FragmentStream trait, about 50 lines
- Output protocol: implement the FragmentObserver trait, about 50 lines
- No modification to any existing protocol

### A.2 The relationship between Fragment and CMAF

LVQR's Fragment payload carries a CMAF chunk (ISO/IEC 23000-19).
CMAF defines fMP4-format media fragments and is compatible with both HLS and DASH distribution protocols.

Reuse paths of a single CMAF chunk:
```
Fragment.payload (CMAF chunk)
  ├── HLS: #EXT-X-PARTIAL-INF partial segment
  ├── DASH: SegmentTimeline Segment
  ├── MoQ: Object (subgroup delivery)
  ├── WHEP: RTP packet (split into MTU-size packets)
  └── Recording: fMP4 segment (written directly to disk)
```

### A.3 Complete semantics of the Fragment's nine fields

| Field | Type | Semantics | Mapping target |
|------|------|------|----------|
| track_id | TrackId | Track identifier | MoQ track, HLS variant, DASH AdaptationSet |
| group_id | u64 | MoQ subgroup sequence number | MoQ subgroup, HLS partial group |
| object_id | u64 | MoQ object sequence number | MoQ object ordering, HLS partial sequence |
| priority | u8 | Priority (0=highest) | MoQ subgroup priority |
| dts | i64 | Decode timestamp | Decode timing of all protocols |
| pts | i64 | Presentation timestamp | Presentation timing of all protocols |
| duration | u32 | Frame/segment duration | HLS EXTINF, DASH Segment duration |
| flags | FragmentFlags | Keyframe/independent decode/discardable | HLS #EXT-X-INDEPENDENT-SEGMENTS |
| payload | Bytes | CMAF chunk binary data | Media data of all protocols |

---

## Appendix B: Architecture Decision Record (ADR) Reference

LVQR's tracking/ROADMAP.md lists 10 architecture decisions. Below are the 5 most
valuable for MediaServo:

### B.1 The Unified Fragment Model is the single most important decision
**Decision**: all tracks are sequences of Fragments, all protocols are projections.
**For MediaServo**: this decision must be settled in Phase 0. The Fragment type
definition runs through all crates. Once settled, adding a protocol becomes mechanical bridging work.

### B.2 The CMAF segmenter is the data plane root node
**Decision**: the CMAF segmenter is the root of the data plane. HLS/DASH/MoQ/WHEP/Recording/DVR
are all different projections of the same Fragment stream.
**For MediaServo**: the mediaservo-segmenter crate should correspond to LVQR's lvqr-cmaf.
LVQR's 29 crates keep versions aligned at release — all crates are at version 1.1.0 in the v1.1.0 release (lvqr-core, lvqr-fragment, lvqr-moq, lvqr-ingest, lvqr-whip, lvqr-srt, lvqr-rtsp, lvqr-hls, lvqr-dash, lvqr-whep, lvqr-relay, lvqr-mesh, lvqr-cluster, lvqr-wasm, lvqr-agent, lvqr-transcode, lvqr-cli and 17 core crates in total). The advantage of a monolithic versioning strategy is that no compatibility worries are needed, at the cost that even a single-crate change requires republishing everything. MediaServo can adopt this strategy early on, and independently version high-iteration crates after Phase 3+.
The advantage of monolithic versioning: users do not have to worry about inter-crate compatibility, and all crates in the workspace are version-aligned. The downside: even when only one crate changes, all 29 crates must be republished. For MediaServo, the monolithic versioning strategy can be adopted early (Phase 0-1) and high-iteration crates (such as `mediaservo-ingest-rtmp` vs `mediaservo-cluster`) can be versioned independently later (Phase 3+).

### Dependency graph analysis

LVQR's crate dependency graph follows a strict DAG (directed acyclic graph) principle:

```
lvqr-core (zero-dependency layer)
  └── lvqr-moq (depends on core)
       └── lvqr-fragment (depends on core + moq)
            ├── lvqr-cmaf (depends on fragment)
            ├── lvqr-ingest (depends on fragment + cmaf)
            ├── lvqr-whip (depends on fragment)
            ├── lvqr-srt (depends on fragment)
            ├── lvqr-rtsp (depends on fragment)
            ├── lvqr-hls (depends on fragment + cmaf)
            ├── lvqr-dash (depends on fragment + cmaf)
            ├── lvqr-whep (depends on fragment)
            ├── lvqr-relay (depends on fragment + moq)
            ├── lvqr-wasm (depends on fragment)
            ├── lvqr-agent (depends on fragment)
            └── lvqr-transcode (depends on fragment)
                 └── lvqr-test-utils (depends on cli)
                      └── lvqr-conformance (depends on test-utils)
                           └── lvqr-soak (depends on test-utils)
                                └── lvqr-cli (composition root, depends on all)
```

**Key dependency principles**:
1. lvqr-core has no internal dependencies — it is the foundation of the entire workspace
2. All ingest/egress crates depend only on lvqr-fragment and not on each other
3. Adding a new input protocol does not affect existing output protocols, and vice versa
4. lvqr-cli is the only composition root; every other crate is a library target
5. Test crates (test-utils/conformance/soak) sit at the bottom and do not pollute the dependency graph

MediaServo's workspace structure should follow exactly the same DAG principle. mediaservo-core corresponds to lvqr-core, mediaservo-fragment to lvqr-fragment, and mediaservo-cli is the only composition root. This principle must be settled in Phase 0, because once circular dependencies appear between crates, the Rust compiler will refuse to compile outright.

As the unified fragmenting engine for all output protocols.

### B.3 No circular dependency graph
**Decision**: lvqr-core has zero internal dependencies. Other crates depend only on core or a protocol root crate.
**For MediaServo**: the workspace dependency graph must be a directed acyclic graph (DAG).
mediaservo-core does not depend on any other crate.

### B.4 lvqr-cli is the only composition root
**Decision**: only lvqr-cli assembles crates into an executable. All other crates are libraries.
**For MediaServo**: there should be a single mediaservo-server (Host) and mediaservo-client
(Client) composition root. Other crates keep library testability.

### B.5 The MoQ facade crate isolates version changes
**Decision**: lvqr-moq is a facade over moq-lite. All MoQ usage is exported via newtypes.
**For MediaServo**: any external protocol dependency should be isolated through a facade crate.
Avoid external library version changes propagating inside the workspace.

---

## Appendix C: Mapping to MediaServo Plugin Traits

```rust
// Correspondence in the MediaServo architecture

// LVQR                           -> MediaServo
// Fragment                       -> MediaFragment
// FragmentStream trait           -> MediaSource trait
// FragmentObserver trait         -> MediaProcessor/MediaSink trait
// FragmentBroadcasterRegistry    -> PipelineEngine StreamManager
// FragmentBroadcaster            -> PipelineEngine StreamBroadcaster
// ingest (rtmp/whip/srt)         -> Protocol plugins
// egress (hls/dash/whep)         -> Protocol plugins
// lvqr-moq facade                -> mediaservo-moq facade
// chitchat cluster               -> ClusterPlugin (Phase 3+)
// SLO observability              -> MetricsPlugin
```

MediaServo's MediaFragment definition should add an ingest_time_ms field for
latency computation, plus a FragmentFlags bitmask for flag control.

---

## Appendix D: LVQR Deployment Configuration Reference

LVQR's zero-config startup is a core feature:

```bash
lvqr serve
```

Ports bound by default:

| Service | Port | Protocol | Description |
|------|------|------|------|
| MoQ/QUIC/WebTransport | 4443/udp | MoQ over moq-lite | always enabled |
| RTMP ingest | 1935/tcp | RTMP | always enabled |
| LL-HLS | 8888/tcp | HTTP/1.1 | always enabled |
| Admin HTTP + WS | 8080/tcp | HTTP + WebSocket fMP4 | always enabled |

All four input protocols in, all output protocols out, through the same Fragment pipeline.

Implication for MediaServo: MediaServo's mediaservo-server should also pursue
a similar zero-config experience. mediaservo serve should bind all protocol default ports,
providing a push-and-play-any-protocol experience.

LVQR's architecture design document (docs/architecture.md) is the best entry point
to understanding the Fragment Model. The 29-crate workspace structure (tracking/ROADMAP.md) is an important reference for Rust project organization. Although the project is early, the foresight of its design ideas is sufficient to make it the
D5-priority reference for MediaServo's Phase 0 pipeline model definition stage.

Core conclusion: the Fragment Model is not better RTMP normalization, but a fundamentally different
architecture paradigm — a unified intermediate representation replaces the protocol matrix, and projections replace conversions.

*This document is based on LVQR v1.1.0 and publicly available GitHub documentation.*

## Appendix E: Reference for MediaServo Metrics Design from LVQR's SLO System

LVQR defines independent three-tier SLO thresholds for each transport protocol. This is the most direct reference for MediaServo's streaming quality monitoring
system:

Core metric: Glass-to-Glass Delay = egress_emit_time - ingest_time (Fragment.ingest_time_ms carries the ingestion timestamp; each egress observer records the emission timestamp)

Per-transport SLO tiers:

| Transport | p50 | p95 warn | p99 critical |
|-----------|-----|----------|--------------|
| WHIP→WHEP | 80ms | 150ms | 300ms → 500ms |
| RTMP→HTTP-FLV | 300ms | 800ms | 1200ms → 2000ms |
| RTMP→HLS | 500ms | 1500ms | 2000ms → 3000ms |
| RTMP→DASH | 800ms | 2500ms | 4000ms → 6000ms |
| SRT→WHEP | 100ms | 200ms | 400ms → 600ms |
| MoQ egress | 80ms | 200ms | 400ms → 800ms |

Secondary metrics: fragment_throughput, observer_lag, ingest/egress_bitrate, keyframe_interval, scte35_event_count
Alerting rules: p95 > warn for 60s → AlertManager; p99 > critical for 10s → urgent + automatic degradation; observer_lag > 2×interval → unhealthy

### E.1 Latency sampling endpoint design

LVQR provides two latency sampling endpoints:

1. **Server-side internal histogram**: the `lvqr_core::metrics::glass_to_glass_delay_seconds` Prometheus histogram, instrumented at ingest/egress points to compute the difference automatically
2. **Client sampling endpoint**: `POST /api/v1/slo/client-sample` — the client reports end-to-end latency (`performance.now()`), and the server merges client reports + internal latency to form a complete end-to-end view

MediaServo equivalent mechanism: PipelineEngine instruments inject/egress points on the Fragment path, StreamSubscriber reports client latency to `/metrics/client`, and the merge forms a complete glass-to-glass view.

### [Adopt] Supplementary — Fragment factory pattern and Pipeline composition

**7. Fragment factory pattern**: an input protocol crate is a "Fragment factory", mapping to the MediaServo `MediaSource` trait (`bind`/`start`/`stop`/`supported_codecs`/`protocol`). Each implementation registered with the `PipelineEngine` automatically becomes part of the pipeline.

**8. FragmentObserver as the MediaSink foundation**: output protocols consume Fragments through `FragmentObserver`, mapping to the `MediaSink` trait (`init`/`on_fragment`/`on_stream_end`/`protocol`). `on_fragment` must be zero heap allocation.

**9. PipelineEngine composition pattern**: manages both `MediaSource` and `MediaSink` collections, and Fragment flow is completed in the data plane by the `FragmentBroadcaster`. Adding a new protocol = registering a new source/sink.

### [Adapt] Supplementary — memory model and zero-copy strategy

**7. Zero-copy Fragment data path**: the payload uses `Bytes` (reference counted), and the entire path from network recv to output protocol involves no copy. `Bytes::slice()` and `Arc::clone()` are both O(1).

**8. Memory pooling**: LVQR does not explicitly use memory pools. MediaServo high-throughput scenarios (multiple 4K streams) should consider introducing `Bytes` memory pooling in Phase 3+.

**9. Fragment batching**: high-framerate scenarios (60fps+48kHz) may consider batching — merging multiple Fragments into one batch to reduce function calls and cache misses. Default batch size 1.

## Appendix F: Reference for MediaServo Cluster Design from LVQR's Gossip Protocol

LVQR uses chitchat as the cluster gossip protocol. Below is a technical comparison with mainstream clustering protocols:

| Protocol | Consistency model | Complexity | Applicable scenarios | LVQR's rationale |
|------|-----------|--------|---------|-------------|
| Raft | Linearizable (strong) | High (leader election + log replication) | Metadata storage, lock services | ❌ too complex |
| Paxos | Linearizable (strong) | Highest (multi-paxos is extremely complex) | Distributed consensus | ❌ implementation and comprehension cost too high |
| SWIM/gossip | Eventual (weak) | Low (UDP gossip + failure detection) | Member management, metadata sync | ✅ simple, comprehensible, sufficient |
| etcd Watch | Linearizable (strong) | Medium (depends on an etcd cluster) | Configuration and state storage | ❌ introduces an external dependency |
| Redis Pub/Sub | No persistence guarantee | Low (depends on Redis) | Real-time message broadcast | ❌ introduces an external single-point dependency |

Key characteristics of LVQR's gossip implementation:
- **Member discovery**: a SWIM protocol variant (UDP gossip, pinging 3 random nodes every 1s)
- **Failure detection**: indirect ping (pinging the suspect node via another node) + direct ping timeout → marked as dead
- **Broadcast ownership = lease**: 10s lease period, 2.5s renewal interval. Lease expiry → automatic release
- **Redirect rather than migration**: a non-owner node receiving a viewer request → HTTP 302 redirect to the owner node
- **Cross-cluster federation**: feature-gated, exchanging cluster routing information over chitchat, recursive DNS CNAME discovery

MediaServo's ClusterPlugin should design a `ClusterBackend` trait (claim_ownership/renew_lease/lookup_owner/discover_members) supporting three backends: ChitchatCluster (UDP gossip), RedisCluster (Pub/Sub), EtcdCluster (Watch+Lease).
struct ChitchatCluster { /* UDP gossip */ }
struct RedisCluster { /* Redis Pub/Sub */ }
struct EtcdCluster { /* etcd Watch + Lease */ }
```
*This document is based on LVQR v1.1.0 and publicly available GitHub documentation.*
## Appendix G: In-depth Mapping of Fragment and MediaServo Plugin Traits

### G.1 Mapping between LVQR's nine Observer types and MediaServo

LVQR defines nine Observer taps, each corresponding to one output path. Below is their direct mapping in the MediaServo architecture:

| LVQR Observer | Function | MediaServo equivalent | Implementation |
|---------------|------|----------------|----------|
| HlsObserver | Generates LL-HLS partials + playlist | `HlsSink` | CMAF chunk → HTTP partial segment |
| DashObserver | Generates DASH segments + MPD | `DashSink` | CMAF chunk → HTTP segment + MPD |
| WhepObserver | RTP packaging → WebRTC egress | `WhepSink` | Fragment → RTP packets via str0m |
| MoqObserver | MoQ/QUIC relay | `MoqSink` | Fragment → MoQ Object via moq-lite |
| WsObserver | WebSocket fMP4 forwarding | `WsSink` | Fragment → fMP4 → WS binary frame |
| RecordObserver | Recording to disk | `RecordSink` | Fragment → fMP4 → LocalFS/S3/OSS |
| WasmObserver | WASM per-fragment filtering | `WasmFilterPlugin` | Fragment → wasmtime filter → Fragment |
| AgentObserver | AI Agent processing | `AiAgentPlugin` | Fragment → AI inference → metadata |
| MeshObserver | P2P mesh relay | `MeshRelayPlugin` | Fragment → WebRTC RTCDataChannel → peer |

### G.2 MediaServo's PipelineEngine data flow

```
// Data flow in the MediaServo composition root
//                   ┌─────────────────┐
//                   │  PipelineEngine  │
//                   │ (control plane)  │
//                   └────────┬────────┘
//                            │
//            ┌───────────────┼───────────────┐
//            │               │               │
//            ▼               ▼               ▼
//    ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
//    │ MediaSource  │ │ MediaSource  │ │ MediaSource  │
//    │    (RTMP)    │ │    (WHIP)    │ │     (SRT)    │
//    └──────┬───────┘ └──────┬───────┘ └──────┬───────┘
//           │                │                │
//           └────────────────┼────────────────┘
//                            │  Fragment stream
//                            ▼
//              ┌─────────────────────────┐
//              │ FragmentBroadcasterRegistry │
//              │   (broadcast, track) →    │
//              │      FragmentBroadcaster  │
//              └──────────┬──────────────┘
//                         │ Observer taps
//              ┌──────────┼──────────┐
//              ▼          ▼          ▼
//        ┌──────────┐ ┌──────────┐ ┌──────────┐
//        │ HlsSink  │ │DashSink  │ │WhepSink  │
//        │(MediaSink)│ │(MediaSink)│ │(MediaSink)│
//        └──────────┘ └──────────┘ └──────────┘
//              │          │          │
//              ▼          ▼          ▼
//          HLS viewer  DASH viewer  WebRTC viewer
```

### G.3 MediaServo's complete MediaFragment definition

Combining the design experience of LVQR's Fragment, MediaServo's MediaFragment is defined as follows:

```rust
/// MediaServo's unified internal media data representation
/// All input protocols produce this type; all output protocols consume it
#[derive(Clone, Debug)]
pub struct MediaFragment {
    /// Track identifier — corresponds to MoQ track alias / HLS variant / DASH AdaptationSet
    pub track_id: TrackId,
    /// MoQ subgroup sequence number — the core grouping basis for output protocols
    pub group_id: u64,
    /// MoQ object sequence number — the ordering basis for frames/segments
    pub object_id: u64,
    /// Priority (0=highest) — used for MoQ priority scheduling and frame-dropping strategy
    pub priority: u8,
    /// Decode timestamp (microseconds) — the decode timing basis for all output protocols
    pub dts: i64,
    /// Presentation timestamp (microseconds) — the presentation timing basis for all output protocols
    pub pts: i64,
    /// Frame/segment duration (microseconds) — used for HLS EXTINF / DASH duration
    pub duration: u32,
    /// Flags — keyframe / independent / discardable / end_of_stream
    pub flags: FragmentFlags,
    /// Payload data — CMAF chunk / fMP4 segment / raw encoded frame
    pub payload: Bytes,
    /// Ingestion timestamp (milliseconds) — used for glass-to-glass latency computation
    pub ingest_time_ms: u64,
    /// Source protocol — keeps provenance information for debugging and routing
    pub source_protocol: ProtocolKind,
}

bitflags! {
    pub struct FragmentFlags: u8 {
        const KEYFRAME     = 0b0001;  // IDR/I frame, HLS INDEPENDENT-SEGMENTS
        const INDEPENDENT  = 0b0010;  // independently decodable, WHEP PLI response boundary
        const DISCARABLE   = 0b0100;  // droppable, MoQ congestion frame drop
        const END_OF_STREAM = 0b1000; // stream-end marker
    }
}
```

Main differences from LVQR's Fragment:
1. Adds the `source_protocol` field — keeps provenance information, easing debugging and protocol-based routing
2. Timestamp units unified to microseconds — more precise than LVQR's mixed units (ms/i64 undefined)
3. `FragmentFlags` uses the `bitflags` macro — unlike LVQR's enum approach, bitflags support combined flags
4. `payload` supports both CMAF chunks and raw encoded frames — distinguished by the `payload_format` field (to be introduced in a later version)

### G.4 Quantitative analysis of the N×M problem

LVQR simplifies the N×M protocol conversion problem to N+M. The following quantitative comparison shows the cost difference between the two approaches when adding a new protocol:

**Traditional approach (N×M converters)**:
- Adding the N+1th input protocol: requires implementing M converters (→HLS, →DASH, →WHEP, →MoQ, ...)
- Adding the M+1th output protocol: requires implementing N converters (RTMP→, WHIP→, SRT→, ...)
- Total converter count: N × M
- Complexity growth: O(N×M) — linear × linear = quadratic growth

**Fragment Model (N+M bridges)**:
- Adding the N+1th input protocol: implement 1 FragmentStream (→Fragment)
- Adding the M+1th output protocol: implement 1 FragmentObserver (Fragment→)
- Total bridge count: N + M
- Complexity growth: O(N+M) — linear growth

**Concrete numeric comparison** (5-in 5-out scenario):

| Metric | Traditional N×M | Fragment N+M | Saving ratio |
|------|---------|-------------|---------|
| Converter count | 5×5=25 | 5+5=10 | 60% |
| New code for the 6th input | 5 converters | 1 bridge | 80% |
| New code for the 6th output | 5 converters | 1 bridge | 80% |
| Impact on existing code when adding a protocol | 5 existing protocols need changes | 0 existing protocols need changes | 100% |
| Number of test cases | 25 conversion paths | 10 bridge paths | 60% |

**Conclusion**: with 10 protocols (5 in 5 out), the Fragment Model's bridge count is only 40% of the traditional approach. As the number of protocols grows, the advantage widens further (10 in 10 out: 20 bridges vs 100 converters, 80% saved).

### G.5 Recommended Phase 0 crate split for MediaServo

Based on LVQR's 29-crate workspace template, MediaServo's initial crate split (10-15 crates):

| Priority | Crate name | Corresponding LVQR | Function | Phase |
|--------|-----------|-----------|------|-------|
| P0 | mediaservo-core | lvqr-core | Shared types (StreamId, TrackId, ProtocolKind, error types) | 0 |
| P0 | mediaservo-fragment | lvqr-fragment | MediaFragment type + FragmentBroadcaster + Observer trait | 0 |
| P0 | mediaservo-ingest-rtmp | lvqr-ingest | RTMP input bridge (FLV→MediaFragment) | 1 |
| P0 | mediaservo-egress-hls | lvqr-hls | LL-HLS output (MediaFragment→CMAF→HTTP partial) | 1 |
| P1 | mediaservo-segmenter | lvqr-cmaf | CMAF segmenter (Fragment→CMAF chunk) | 1 |
| P1 | mediaservo-ingest-whip | lvqr-whip | WHIP/WebRTC input (str0m→MediaFragment) | 1 |
| P1 | mediaservo-egress-whep | lvqr-whep | WHEP/WebRTC output (MediaFragment→RTP→str0m) | 1 |
| P1 | mediaservo-ingest-srt | lvqr-srt | SRT input (MPEG-TS→MediaFragment) | 2 |
| P2 | mediaservo-ingest-rtsp | lvqr-rtsp | RTSP input (RTP→MediaFragment) | 2 |
| P2 | mediaservo-egress-dash | lvqr-dash | MPEG-DASH output (MediaFragment→CMAF→MPD) | 2 |
| P2 | mediaservo-record | lvqr-record | Recording engine (MediaFragment→fMP4→LocalFS/S3) | 2 |
| P2 | mediaservo-cli | lvqr-cli | Composition root (binary entry point) | 1 |
| P3 | mediaservo-egress-moq | lvqr-relay | MoQ/QUIC relay | 3 |
| P3 | mediaservo-cluster | lvqr-cluster | chitchat gossip cluster | 3 |
| P3 | mediaservo-mesh | lvqr-mesh | P2P mesh relay | 4 |

Crates omitted compared to LVQR: WASM filters (consider in Phase 4+), AI Agent (consider in Phase 4+), GStreamer transcoding (consider in Phase 3+), test crates (merged into the `tests/` directory rather than separate crates).
