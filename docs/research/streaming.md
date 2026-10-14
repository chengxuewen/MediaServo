# Push/Pull Streaming Product Research Report

> Research date: 2026-07-16 | Goal: provide technology selection references for the MediaServo push/pull streaming module

---

## Table of Contents

1. [Production Tools (encoders / stream push clients)](#1-production-tools)
   - OBS Studio
   - vMix
   - Wirecast
2. [Media Servers](#2-media-servers)
   - SRS (Simple Realtime Server)
   - MediaMTX
   - nginx-rtmp-module
   - Xiu
   - LiveGo
   - Node-Media-Server
3. [Protocol Converters / Stream Processing Engines](#3-protocol-converters)
   - FFmpeg
   - GStreamer
4. [Emerging Architecture References](#4-emerging-architecture-references)
   - LVQR — Unified Fragment Model
   - Muxshed (shed)
5. [GStreamer Pipeline Practice in Push/Pull Streaming](#5-gstreamer-pipeline)
6. [Synthesis](#6-synthesis)

---

## 1. Production Tools (encoders / stream push clients)

### 1.1 OBS Studio

**Overview**: The most popular open-source streaming encoder in the world, written in C/C++, GPLv2 license. Full Windows/macOS/Linux support, free. Positioned as a general-purpose live streaming and screen recording tool.

**Architecture patterns**:
- **Plugin-based pipeline**: The core engine `libobs` manages a scene graph (Scene Graph); plugins are inserted through the `obs_output_t` / `obs_source_t` interfaces, forming a source → filter → encoder → output pipeline
- **Multi-protocol output model**: Each output protocol is an independent plugin (`obs-outputs`), distinguished by the `OBS_OUTPUT_AV/VIDEO/AUDIO` flags
- **Two-thread RTMP architecture**: connection thread (`connect_thread`) + send thread (`send_thread`), coordinated with a semaphore; on Windows there is also a separate socket thread optimization
- **WebRTC output dropped the interleaver**: PR #13270 found that the audio/video packet interleaving ordering required by FLV/RTMP is redundant for WebRTC (audio and video go over independent RTP tracks), so skipping the interleaver eliminates pipeline delay

**Protocol support matrix**:

| Protocol | Direction | Codec | Notes |
|------|------|------|------|
| RTMP/RTMPS | Push | H.264 + AAC (FLV) | Via librtmp, supports Enhanced RTMP (HEVC/AV1/multi-track) |
| SRT | Push | H.264 + AAC (MPEG-TS) | Via libsrt |
| RIST | Push | H.264 + AAC | Via librist |
| WHIP (WebRTC) | Push | H.264/HEVC/AV1 + Opus | Added in v32.1, supports Simulcast (1-4 layers) |
| HLS | Push | H.264 + AAC | Custom muxer |

**Tech stack**:
- C/C++ core, Qt UI
- GPU encoding: NVENC, AMF, VAAPI, Apple VideoToolbox, QSV
- Network libraries: librtmp (self-maintained fork, supports mbedTLS/OpenSSL/GnuTLS), libdatachannel (WebRTC)
- Scripting extensions: Lua + Python

**Performance metrics**:
- CPU usage (1080p60 push): about 22% (OBS 30, 3-camera scenario)
- GPU encoding overhead: about 12%
- Startup time: about 4.2s
- Scene switch latency: about 120ms
- 6-hour stability test: zero dropped frames

**Pros**:
- Free and open source, largest community
- Rich plugin ecosystem (Lua/Python scripting)
- Full cross-platform support (Windows/macOS/Linux)
- Simulcast support for WebRTC push with encoding adaptation

**Cons**:
- Single-output-stream native design; simultaneous push to multiple platforms requires a third-party plugin (Multiple RTMP Outputs). OBS v32.1+ already supports WHIP Simulcast with 1-4 encoding layers (single endpoint, not multi-endpoint).
- Audio routing is complex, less intuitive than vMix's bus system
- No built-in instant replay (requires the Replay Buffer plugin)
- No built-in virtual studio

**Adoptable designs**:
1. **Plugin-based output model**: each protocol is an independent output plugin connected to encoder output through a unified interface. MediaServo's protocol adaptation layer can reference this "unified encoding output → per-protocol branching" pattern
2. **WHIP Simulcast layering strategy**: 1 layer = 100%, 2 layers = 50%, 3 layers = 33%, 4 layers = 25% — simple and practical, no complex negotiation needed
3. **Interleaver-skip optimization**: protocol-aware pipeline optimization — WebRTC does not need FLV's audio/video interleaving, and skipping this stage reduces latency

**Lessons**:
- Plugin dependency risk: the Teams integration requires the StreamFX plugin, but frequent Teams updates break compatibility
- `librtmp` needs a self-maintained fork: the community-standard librtmp does not support Enhanced RTMP, so patches are needed
- **Not suitable for multi-platform push scenarios** — the single-output architecture is the bottleneck for high-concurrency pushing

---

### 1.2 vMix

**Overview**: A Windows-only professional-grade live production software, positioned for medium-to-large event streaming. vMix 27 (released January 2026). Paid software, Basic $60 ~ Pro $1200.

**Architecture patterns**:
- Full-featured live production switcher: turns a PC into a complete broadcast studio
- Supports HDMI/SDI/NDI/IP multi-source mixed input
- Native multi-stream output: Basic HD supports 3 streams, Pro supports 5 simultaneous pushes
- Built-in instant replay engine (core feature for sports streaming)
- PTZ camera IP control (VISCA, ONVIF, NDI)

**Protocol support**:
- RTMP (with AES encryption), NDI
- No native direct support for SRT/WebRTC (through external tools)

**Tech stack**:
- Windows only (C# script extensions)
- GPU encoding: NVENC, QuickSync, AMF
- No cross-platform support (Mac users can only use it through Parallels)

**Performance metrics**:
- CPU usage (1080p60, 3 cameras): about 28%
- 4K CPU usage: about 20% (AMD Threadripper)
- GPU encoding overhead: about 15%
- Scene switch latency: about 80ms (best in industry)
- 8K stability 50% better than Wirecast

**Pros**:
- Best-in-class instant replay engine, one-click slow motion + marker + return to live
- Native multi-stream push to 5 targets, no plugins required
- Unlimited NDI inputs, suitable for large multi-camera productions
- Built-in virtual studio, PTZ control

**Cons**:
- **Windows only** — this is the biggest limitation
- High cost (Pro at $1200)
- No Mac/Linux support
- Scripting extensions are C# only, ecosystem less rich than OBS's Lua/Python
- Steeper learning curve than OBS

**Adoptable designs**:
1. **Bus audio system**: more intuitive multi-channel / multi-bus audio routing than OBS
2. **PTZ camera integration**: deep integration of VISCA/ONVIF protocols with live production, which is a useful reference for MediaServo's surveillance camera ingestion
3. **NDI zero-configuration network video**: configuration-free video sharing on a LAN, suitable for enterprise intranet scenarios

**Lessons**:
- Windows-only restricts the audience and deployment scenarios
- Closed-source dependency — MediaServo needs an open-source strategy

---

### 1.3 Wirecast

**Overview**: From Telestream, dual Windows/Mac platform. Professional-grade live production, positioned for enterprise streaming and broadcast. Wirecast 16 (released February 2026).

**Architecture patterns**:
- Professional switcher model: unlimited input sources, real-time switching, multi-picture monitoring
- **ISO recording**: the core differentiator — all input sources are recorded in parallel, so after the broadcast you can edit per independent camera source
- Remote production: the Rendezvous feature handles remote guests (latency about 120ms, 25% faster than the OBS WebRTC plugin)
- Multi-stream output: native support for pushing to multiple platforms simultaneously

**Protocol support**:
- RTMP/RTMPS, NDI, SRT
- Remote guests join via RTMP/NDI/browser
- AI real-time captions (97% accuracy, 4-speaker test)

**Tech stack**:
- C++ core, Windows/macOS
- GPU encoding: NVENC, QuickSync, Apple VideoToolbox
- JavaScript script extensions

**Performance metrics**:
- CPU usage (1080p60, 3 cameras): about 35% (heaviest of the three)
- RAM usage: about 3.1 GB
- Mac stability issues: crashed twice during a 3-hour stream on M3 Max
- Startup time: about 12.5s (slowest)
- GPU encoding overhead: about 18%

**Pros**:
- ISO recording is the differentiating weapon
- High AI caption accuracy
- Enterprise-grade SLA support
- Built-in professional graphics templates

**Cons**:
- High price ($299 Studio / $799 Pro)
- Unstable performance on Mac
- Highest CPU/RAM usage
- Basic audio mixer features
- No Linux support

**Adoptable designs**:
1. **ISO recording**: all input sources recorded independently; MediaServo's recording module can reference this "multi-track parallel recording" pattern
2. **Remote guest management**: the Rendezvous feature shows how to mix RTMP/NDI/browser multi-source ingestion

---

## 2. Media Servers

### 2.1 SRS (Simple Realtime Server)

**Overview**: Written in C/C++, MIT license, 28,646 GitHub Stars. Active Chinese-language community. Positioned as a simple, efficient realtime media server, serving RTMP/WebRTC/HLS/HTTP-FLV/SRT/MPEG-DASH/GB28181 simultaneously from a single binary.

**Architecture patterns**:
- **Coroutine architecture**: based on ST (State Threads) coroutines, no async callback hell; a single process handles concurrent connections with many coroutines
- **Single data path**: all input protocols are finally normalized into an internal RTMP stream, and all output protocols are converted from the internal RTMP stream
- **Multi-protocol output**: one RTMP input stream can be output simultaneously as HLS/HTTP-FLV/WebRTC/DASH with no transcoding (transmux only)
- **Hybrid model**: SRT is an independent process module communicating with the main process over internal RTMP
- **Origin-Edge clustering**: supports multi-tier edge nodes

**Protocol conversion matrix**:

| Input ↓ / Output → | RTMP | WebRTC | HLS | HTTP-FLV | SRT | DASH |
|-----------------|------|--------|-----|----------|-----|------|
| RTMP | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| WebRTC | ✅ | ✅ | - | - | - | - |
| SRT | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

**Tech stack**:
- C/C++, coroutines (State Threads)
- Codec support: H.264, H.265, AV1, VP9, AAC, Opus, G.711
- Config format: INI
- Monitoring: Prometheus + Grafana
- Deployment: native Docker + Kubernetes

**Performance metrics**:

| Test item | Data |
|--------|------|
| RTMP latency | 0.8s ~ 3s |
| WebRTC latency | 80ms ~ 400ms (80ms after optimization) |
| HLS latency | 3s ~ 10s standard, 2s ~ 3s LL-HLS |
| RTMP playback concurrency | Thousands of streams (single machine) |
| WebRTC playback concurrency | 500~1000+ (2 vCPU) |
| SRS 4.0.87 WebRTC latency | 80ms |
| SRS 2.0.72 RTMP latency | H.264: 0.4s, H.264+AAC: 0.6s |

**Pros**:
- Widest protocol coverage from a single binary server
- Protocol conversion without transcoding (transmux only), extremely low CPU overhead
- The coroutine architecture avoids async callback complexity
- Large community, thorough Chinese documentation
- MIT license, business-friendly

**Cons**:
- The internal data model is fixed to an RTMP stream, so adding a new protocol requires adapting to the RTMP intermediate representation
- No QUIC/Media-over-QUIC support (MediaMTX already supports it)
- TURN servers need separate deployment (WebRTC NAT traversal depends on external components)
- No cluster management GUI (Wowza/Ant Media have one)
- The C codebase dates from 2013; some modules have high maintenance cost

**Adoptable designs**:
1. **Single-internal-representation protocol conversion pattern**: all input protocols normalize into an internal stream, and all outputs are derived internally — one of the core inspirations for MediaServo's Unified Fragment Model
2. **Coroutine architecture**: lighter than a thread pool under high concurrency, simpler than async/await
3. **RTMP-to-* transmux**: protocol conversion with zero transcoding overhead

**Lessons**:
- An internal RTMP stream as the only intermediate representation limits data model extensibility — LVQR's Fragment Model solves this
- The SRT independent process module (hybrid model) illustrates the communication cost at the boundary of multi-language / multi-process architectures
- WebRTC's TURN is an independent operational burden

---

### 2.2 MediaMTX

**Overview**: Written in Go, MIT license. Formerly `rtsp-simple-server`. Positioned as a "media router", a single binary with zero external dependencies. Supports Media-over-QUIC (emerging protocol).

**Architecture patterns**:
- **Path-centered data model**: each path corresponds to one stream, provided by one publisher or an external source, and broadcast to all readers
- **Path Manager**: manages paths, authentication, and client binding
- **Automatic protocol conversion**: once a stream enters the server, it is automatically served on all enabled protocols — no extra configuration
- **On-demand pulling**: `sourceOnDemand` mode pulls the upstream stream only when there are viewers, saving bandwidth
- **Read replica mode**: read replicas pull from an origin instance and distribute, balanced by L4/L7 LB

**Protocol support**:

| Protocol | Push | Playback | Encryption |
|------|:----:|:----:|:----:|
| Media-over-QUIC | ✅ | ✅ | TLS 1.3 |
| SRT | ✅ | ✅ | AES-128/256 |
| WebRTC (WHIP/WHEP) | ✅ | ✅ | DTLS-SRTP |
| RTSP | ✅ | ✅ | RTSPS |
| RTMP | ✅ | ✅ | RTMPS |
| LL-HLS | - | ✅ | - |
| MPEG-TS | ✅ | ✅ | - |
| RTP | ✅ | - | - |

**Tech stack**:
- Single Go binary, zero external dependencies
- Codecs: H.264, H.265, AV1, VP9, Opus, MPEG-4 Audio, G.711, LPCM
- Recording format: fMP4 (fragmented MP4) or MPEG-TS
- REST API for runtime management
- Prometheus metrics, pprof

**Pros**:
- **Zero-dependency single binary** — extremely simple deployment (one Docker command)
- Media-over-QUIC support ahead of the industry
- On-demand pulling saves bandwidth (IoT/IP camera scenarios)
- Hot config reload without dropping existing connections
- RTSP tunneling (HTTP/WebSocket) solves firewall traversal

**Cons**:
- Go GC has stall risk under very high concurrency
- No built-in transcoding (requires FFmpeg)
- No built-in cluster management system (read replicas need manual LB configuration)
- Pure Go ecosystem, cannot use GPU hardware encoding

**Adoptable designs**:
1. **"Path" abstraction**: a path-centered decoupled model where publisher/source and readers are fully isolated, a good fit for MediaServo's channel/room model
2. **sourceOnDemand**: pull on demand, an energy-saving mode for sleeping cameras/IPC
3. **Automatic protocol conversion**: the publisher pushes one protocol and viewers watch on any protocol, zero configuration
4. **Read replica horizontal scaling pattern**: a simple origin-replica architecture, lighter than SRS clustering
5. **Media-over-QUIC**: MediaServo should consider QUIC as a unified transport layer

**Lessons**:
- No built-in transcoding limits standalone usage (FFmpeg is mandatory)
- LL-HLS caching problems when integrating with a CDN — some segments must bypass CDN cache

---

### 2.3 nginx-rtmp-module

**Overview**: Written in C, BSD-2-Clause license. The most widely used open-source RTMP module. Turns nginx into a full streaming server and is the standard choice for small self-hosted live streaming solutions.

**Architecture patterns**:
- **Embedded nginx module**: runs as an nginx module, sharing nginx's worker process model
- **RTMP → HLS/DASH conversion**: slices an RTMP input in real time into HLS `.ts` segments and `.m3u8` playlists, written to disk and then served by nginx's HTTP module
- **push/pull relay**: builds an origin-edge distributed network
- **exec hooks**: transcoding through external FFmpeg processes (one FFmpeg process forked per stream)
- **HTTP callback authentication**: event callbacks such as `on_publish`, `on_play`, `on_record_done`

**Protocol support**:
- Input: RTMP, RTMPS (via stunnel)
- Output: RTMP, HLS, MPEG-DASH, FLV
- Codecs: H.264, H.265/HEVC, VP6, VP9, Sorenson H.263 + AAC, MP3, Speex, Opus

**Performance metrics**:

| Scenario | Data |
|------|------|
| RTMP latency | 1.56s (average, academic test) |
| HLS latency (1s segment) | ~4s end-to-end |
| HLS latency (6s segment) | ~15-20s |
| Concurrency without transcoding | 30-50 streams (4-core VPS) |
| Concurrency with transcoding | 3-5 streams/core (FFmpeg per stream) |
| Optimal HLS fragment | 3s (the balance point between latency and stability) |
| LL-HLS floor | ~5-10s (no WebRTC support) |

**Pros**:
- Simplest deployment: nearly zero learning cost for nginx users
- Lowest resource usage: very small memory and CPU overhead without transcoding
- nginx ecosystem integration: can use nginx access control, rate limiting, and SSL termination
- Mature community, abundant material

**Cons**:
- **No WebRTC/SRT support** — this is the most fatal drawback
- No built-in ABR transcoding (requires manual FFmpeg exec setup + manual master playlist assembly)
- No native RTMPS support (requires a stunnel/HAProxy front end)
- Multi-worker requires `rtmp_auto_push` and is unavailable on Windows
- Slow updates to the upstream repo; bug fixes rely on community forks
- No DRM, no advanced analytics

**Adoptable designs**:
1. **HTTP callback event system**: `on_publish`/`on_play`/`on_done` — a simple stream lifecycle hook pattern. MediaServo also needs a similar event system for authentication and billing
2. **push/pull relay**: a simple origin-edge topology, suitable for MediaServo's early deployment
3. **Segment-based HLS generation**: write directly to the filesystem, HTTP static serving — the simplest pattern

**Lessons**:
- No WebRTC means it can only live on as a relic of the RTMP era
- per-stream FFmpeg fork transcoding is a performance black hole (a 4-core machine runs only 3-5 streams)
- **No internal unified data model** — each protocol is handled separately: HLS writes to disk, RTMP goes over TCP, DASH is yet another independent path — MediaServo must avoid this fragmentation

---

### 2.4 Xiu

**Overview**: Written in Rust, MIT license, 2,293 GitHub Stars. A lightweight pure-Rust live streaming server supporting RTMP/RTSP/WebRTC(WHIP/WHEP)/HTTP-FLV/HLS.

**Architecture patterns**:
- **Modular Cargo workspace**: each protocol is an independent crate (`rtmp`, `xrtsp`, `httpflv`, `hls`, `webrtc`)
- **StreamHub center**: an internal stream bus like SRS's; inter-protocol conversion happens through the StreamHub
- **TOML config file**: supports enabling protocol combinations (rtmp only / rtmp+hls / rtmp+httpflv+hls)
- **Command-line quick start**: `xiu -r 1935 -t 5544 -w 8900 -f 8080 -s 8081`

**Protocol support**:

| Protocol | Push | Playback | Notes |
|------|:----:|:----:|------|
| RTMP | ✅ | ✅ | H.264+AAC, GOP cache, cluster |
| RTSP | ✅ | ✅ | H.265/H.264+AAC, TCP/UDP |
| WebRTC (WHIP/WHEP) | ✅ | ✅ | Built-in browser player |
| HTTP-FLV | - | ✅ | Remuxed from RTMP/RTSP |
| HLS | - | ✅ | Remuxed from RTMP/RTSP, supports recording |

**Tech stack**:
- Rust, Tokio async
- Codecs: H.264, H.265, AAC
- HTTP framework: migrated from hyper to axum
- Dependencies: `audiopus` (cross-platform compilation), `reqwest`

**Pros**:
- Rust memory safety, suitable for long-running server services
- Clear modularity, decoupled protocols
- Supports RTSP over TCP/UDP (Interleaved)
- WHIP/WHEP includes a built-in web player (works out of the box)
- One-command CLI startup

**Cons**:
- Smaller community, few production deployment cases
- No SRT support (although Rust SRT libraries exist)
- HLS recording has a bug history (v0.13.0 panics on HLS URL paths)
- No built-in transcoding
- Opaque performance data (no published benchmarks)

**Adoptable designs**:
1. **Rust workspace modularity**: protocols as independent crates — highly consistent with MediaServo's crate architecture
2. **StreamHub pattern**: like SRS's internal RTMP stream but more modular
3. **CLI friendliness**: quickly switch protocol ports from the command line — high development/testing efficiency

**Lessons**:
- Array out-of-bounds panic in HLS URL path parsing (`index out of bounds: len is 3 but index is 3`) — a production-grade server must use defensive programming for URL parsing
- Community size affects bug fix speed

---

### 2.5 LiveGo

**Overview**: Written in Go, MIT license, developed by `gwuhaolin`. An extremely minimal Go streaming server supporting only RTMP push + HLS/HTTP-FLV/RTMP playback.

**Architecture characteristics**:
- Pure Go, single binary
- Supported protocols: RTMP (push + playback), AMF, HLS, HTTP-FLV
- GOP cache (controlled by the `gop_num` parameter)
- Static push (`static_push`)
- JWT authentication (fork version)
- YAML/JSON config files

**Pros**: extremely simple deployment, Go cross-platform, suitable for learning and small projects
**Cons**: limited features, no WebRTC/SRT/RTSP, inactive community

**Adoptable designs**: GOP cache implementation pattern — caching the most recent GOP so new viewers see a picture immediately

---

### 2.6 Node-Media-Server

**Overview**: Written in Node.js, MIT license, 6K GitHub Stars. Usable after a global npm install.

**Architecture patterns**:
- Event-driven, single-threaded async
- v4 supports Enhanced RTMP (HEVC/VP9/AV1)
- Full client SDK set (NodePlayer.js / NodeMediaClient iOS/Android)
- REST API + JWT authentication (v4.2)

**Protocol support**: RTMP/RTMPS, HTTP/HTTP2-FLV, WS/WSS-FLV, HLS, DASH

**Pros**: npm ecosystem, complete client SDKs, friendly to Node.js developers
**Cons**: single-thread concurrency limit, no WebRTC/SRT support, incompatible v2→v4 upgrade

---

## 3. Protocol Converters / Stream Processing Engines

### 3.1 FFmpeg

**Overview**: The most general-purpose audio/video processing command-line tool and library set. `libavcodec`/`libavformat`/`libavfilter`/`libswscale` form a complete codec + remux + filter stack.

**Roles in push/pull streaming scenarios**:
- **Encoder**: capture from camera/file/network stream → encode to H.264/H.265 → push to an RTMP/SRT server
- **Transcoder**: receive an RTMP stream → decode → multi-bitrate encode → output an HLS ABR ladder
- **Protocol converter**: RTMP → HLS, SRT → RTMP, RTSP → HLS, and so on
- **Recording**: `-f flv` saves the pushed stream, `-f segment` saves it in segments

**Key parameters (low-latency live streaming)**:
```bash
ffmpeg -re -i input \
  -c:v libx264 -preset veryfast -tune zerolatency \
  -g 60 -keyint_min 60 -sc_threshold 0 \
  -b:v 4000k -maxrate 4000k -bufsize 8000k \
  -f flv rtmp://server/live/key
```
- `-tune zerolatency`: disables frame reordering (critical!)
- `-g 60`: one keyframe every 2 seconds (HLS segment alignment)
- `-sc_threshold 0`: disables scene detection (guarantees IDR alignment)

**Inherent problems of ABR transcoding** (analysis by Twitch engineers):
- A single `1-in-N-out` FFmpeg instance creates N independent encoders, and the IDR frames of the N encoders are not aligned
- When a source stream is transmuxed and mixed with transcoded streams, misaligned IDR PTS causes Chromecast playback to pause
- Solution: 1 decoder → N scalers/encoders + frame-rate-adaptive down-sampling — this requires a custom transcoder

**Performance data**:
- NVENC T4: 4-6 concurrent 1080p60 encodes on a single GPU
- NVENC A10: equivalent workload at 4K
- CPU-only: 720p single-bitrate is feasible, multi-bitrate ABR requires a GPU
- 4-core VPS + FFmpeg transcoding: 3-5 concurrent streams

**Pros**: unmatched generality, supports virtually every format and protocol
**Cons**: a command-line tool, not a server, with no multi-viewer distribution capability

**Adoptable designs**:
1. **Pipeline parameter pattern**: the `preset`/`tune`/`GOP` configuration pattern can serve as a reference for MediaServo's encoding configuration
2. **Zero-latency tuning strategy**: `zerolatency` + `sc_threshold 0` + `bf=0`

---

### 3.2 GStreamer

> See [§5. GStreamer Pipeline Deep Dive](#5-gstreamer-pipeline)

**Overview**: A pipeline-based multimedia framework in which elements are linked into pipelines through pads. The GStreamer 1.x series, C core + Rust bindings, LGPL license.

**Roles in push/pull streaming**:
- Capture (`v4l2src`, `ximagesrc`, `videotestsrc`) → encode (`x264enc`, `nvh264enc`) → mux (`flvmux`, `mpegtsmux`) → transport (`rtmpsink`, `srtsink`)
- `webrtcbin` + `webrtcsink`/`webrtcsrc` for WebRTC

**Protocol support**: RTMP, SRT, RTSP, HLS/DASH, WebRTC (via `webrtcbin`)

**Pros**: extremely flexible pipeline composition, good hardware acceleration integration
**Cons**: extremely steep learning curve, complex caps negotiation, WebRTC is not natively supported

---

## 4. Emerging Architecture References

### 4.1 LVQR — Unified Fragment Model (core reference)

**Overview**: Written in Rust, a 29-crate workspace. A single binary implementing RTMP/WHIP/SRT/RTSP/WebSocket fMP4 input and LL-HLS/MPEG-DASH/WHEP/MoQ/WebSocket fMP4 output. MIT license. **This is the most important reference project for MediaServo's pipeline model.**

**Core architecture — Unified Fragment Model**:

```rust
// crates/lvqr-fragment/src/lib.rs
pub struct Fragment {
    pub track_id: TrackId,    // track identifier
    pub group_id: u64,        // group identifier (MoQ group)
    pub object_id: u64,       // object identifier (MoQ object)
    pub priority: u8,         // priority
    pub dts: i64,             // decode timestamp
    pub pts: i64,             // presentation timestamp
    pub duration: u32,        // duration
    pub flags: FragmentFlags, // keyframe / independent / discardable
    pub payload: Bytes,       // payload data (CMAF chunk / fMP4 segment)
}
```

**Design principles**:
1. **All input protocols produce Fragments**, and all output protocols consume Fragments
2. **One internal media type**: every wire format is a projection of a Fragment
3. **Adding a new protocol is ~50 lines of bridge code** (install an Observer or produce Fragments)
4. **The control plane uses `async-trait`** (one allocation per connection), **the data plane uses concrete types** (zero heap allocation per Fragment)

**Data plane architecture**:
```
RTMP (1935)  ─┐
WHIP  (HTTPS) ├┐
SRT   (UDP)   ├┼─► FragmentBroadcaster ─► FragmentObserver taps
RTSP  (TCP)   ├┘   per (broadcast, track)  ├─► MoQ relay
WS fMP4       ┘                            ├─► LL-HLS playlist + segments
                                           ├─► DASH MPD + segments
                                           ├─► WHEP RTP packetizer
                                           ├─► WebSocket fMP4 forwarder
                                           ├─► lvqr-record (disk)
                                           └─► lvqr-archive (redb index)
```

**Cluster plane** (optional, feature-gated):
- chitchat gossip protocol (UDP), eventual consistency
- Broadcast ownership = lease (not lock), 10s lease, 2.5s renewal
- Subscriber redirection: 302 jump to the owner node
- **Explicit rejection of Raft/leader election**: linear consistency is not a design goal

**Observability**:
- Server-side glass-to-glass latency histogram (ingest_time_ms → egress_emit_ms)
- Client-pushed latency samples (`POST /api/v1/slo/client-sample`)
- Independent SLO thresholds per transport
- MoQ glass-to-glass latency implemented through the sidecar ` /0.timing` track

**Mesh bandwidth offload**:
- The first 30 viewers connect directly to the server (root peer)
- Subsequent viewers relay from other viewers over WebRTC DataChannel
- At 500 viewers the server serves only 120 Mbps (94% offloaded)
- Tree topology, self-balancing, dead-node detection via heartbeat

**Performance SLO**:

| Transport | p50 | p95 warning | p99 critical |
|-----------|-----|-------------|--------------|
| LL-HLS | 500ms | 1500ms | 2000ms → 4000ms |
| DASH | 1000ms | 3000ms | 4000ms → 8000ms |
| WHEP (WebRTC) | 100ms | 250ms | 500ms → 1000ms |
| MoQ | 80ms | 200ms | 400ms → 800ms |
| WS (fMP4) | 300ms | 800ms | 1200ms → 2500ms |

**Implications for MediaServo**:
1. **A Fragment is exactly the Unified Fragment Model MediaServo should pursue** — it is a protocol-agnostic intermediate representation that avoids SRS's RTMP-centered limitation
2. **Zero virtual-function dispatch on the data plane** — Rust enum dispatch and concrete types are critical for performance
3. **chitchat gossip clustering** — simpler than Raft/Paxos yet sufficient; MediaServo can adopt it
4. **MoQ as a first-class citizen** — Media over QUIC is the future direction for low-latency transport
5. **A glass-to-glass latency SLO system** — different thresholds per protocol, complete observability
6. **Mesh offload** — a large-scale distribution solution reducing bandwidth cost by 94%+

---

### 4.2 Muxshed (shed)

**Overview**: Rust (Axum) + SvelteKit front end, AGPL license. A self-hosted multistream studio. RTMP + SRT input, fanned out to YouTube/Twitch/Kick/custom RTMP. Can be considered a self-hosted Restream/StreamYard alternative.

**Architecture characteristics**:
- Production switcher in the browser (SvelteKit UI)
- Custom Rust RTMP/FLV relay (`crates/api/src/rtmp/`)
- FFmpeg subprocess for transcoding and HLS
- Program failover: automatic switch to a fallback source when the broadcaster drops the stream
- Elgato Stream Deck plugin

**Implications**: demonstrates a practical pattern for the Rust + FFmpeg combination; the failover mechanism is worth adopting

---

## 5. GStreamer Pipeline Practice in Push/Pull Streaming

### 5.1 Pipeline pattern summary

GStreamer's pipeline model is the most flexible stream processing approach in push/pull streaming scenarios:

```
[Source] → [Decoder] → [Filter/Scale] → [Encoder] → [Muxer] → [Sink]
 v4l2src → decodebin  → videoscale    → x264enc  → flvmux  → rtmpsink
filesrc  → avidemux   → deinterlace   → nvh264enc→ mpegtsmux→ srtsink
```

### 5.2 Common protocol pipelines

**RTMP push**:
```bash
gst-launch-1.0 v4l2src ! videoconvert ! x264enc tune=zerolatency \
  speed-preset=veryfast bitrate=4000 ! h264parse ! flvmux \
  ! rtmpsink location='rtmp://server/live/stream live=1'
```

**SRT push** (screen sharing):
```bash
gst-launch-1.0 ximagesrc ! videoconvert \
  ! x264enc bitrate=32000 tune=zerolatency speed-preset=veryfast \
  byte-stream=true threads=1 key-int-max=15 intra-refresh=true \
  ! video/x-h264,profile=baseline ! mpegtsmux \
  ! srtserversink uri=srt://0.0.0.0:8888/ latency=100
```

**WebRTC (webrtcsink)**:
```bash
gst-launch-1.0 webrtcsink name=ws meta="meta,name=gst-stream" \
  videotestsrc ! ws. audiotestsrc ! ws.
```
- `webrtcsink` built-in: codec selection, TWCC congestion control (a re-implementation of the GCC algorithm), custom signaling
- `webrtcsrc` supports either decoded output or pass-through of encoded data

### 5.3 Production usage cases

**ShareChat live platform** (India, million-scale users):
- A GStreamer pipeline receives raw WebRTC frames (YUV/PCM) → the compositor synthesizes multi-participant video → x264enc encodes → RTMP output
- Multi-participant video synthesis uses GStreamer compositor + latency buffer synchronization
- Avatars/static pictures use uridecodebin + imagefreeze
- Simultaneous recording of the composed stream and of individual streams
- Temporal workflows guarantee high availability

### 5.4 Performance parameter tuning

| Parameter | Low-latency value | Standard value | Notes |
|------|---------|--------|------|
| `tune=zerolatency` | ✅ | - | Disables frame reordering |
| `speed-preset` | veryfast/ultrafast | medium | Encoding speed vs compression ratio |
| `key-int-max` | 15 (0.5s@30fps) | 60 (2s@30fps) | Keyframe interval |
| `intra-refresh` | true | false | Progressive refresh vs IDR |
| `threads` | 1 (avoid latency) | auto | Number of encoding threads |
| `latency` (SRT) | 80-200ms | 125ms | SRT latency |
| `bitrate` | 70% of link bandwidth | - | Avoid congestion |

### 5.5 GStreamer vs FFmpeg

| Dimension | GStreamer | FFmpeg |
|------|-----------|--------|
| Model | Linked pipeline elements | Command-line tool + libav* |
| Realtime control | Strong (runtime pipeline modification) | Weak (cannot be changed after start) |
| Dynamic pipelines | Supported (pads added/removed dynamically) | Not supported |
| Learning curve | Steep | Gentle |
| Applicable scenarios | Long-running applications, embedded | Batch processing, one-off transcoding |
| WebRTC | webrtcbin (semi-native) | Not supported (requires libdatachannel) |

### 5.6 Implications for MediaServo

GStreamer's pipeline model differs essentially from LVQR's Unified Fragment Model:
- GStreamer is an **element-level pipeline**: data flows between elements as buffers, and caps negotiation is complex
- LVQR is a **Fragment-level pipeline**: all inputs normalize into Fragments, and all outputs derive from Fragments

**MediaServo should take the LVQR route rather than the GStreamer route**:
1. LVQR's Fragment model is simpler: only one internal format
2. Adding a new protocol needs only bridge code (~50 lines)
3. GStreamer's caps negotiation is a fault source in production
4. But GStreamer's compositor + hardware codec integration is worth referencing on the GPU encoding path

---

## 6. Synthesis

### 6.1 Best practice for multi-protocol adaptation

| Pattern | Representative product | Pros | Cons | Suitable for MediaServo? |
|------|---------|------|------|:---:|
| Internal RTMP normalization | SRS | Simple, mature | Limited by RTMP semantics | ❌ |
| Path + automatic protocol conversion | MediaMTX | Zero configuration, flexible | No unified data model | ⚠️ |
| Pipeline-style linking | GStreamer | Extremely flexible | Extremely steep learning curve | ❌ |
| **Unified Fragment Model** | **LVQR** | Most elegant, protocol-agnostic | Relatively new, small community | ✅ **first choice** |
| No internal model | nginx-rtmp | Extremely minimal | Protocols fragmented from each other | ❌ |
| StreamHub message bus | Xiu | Rust-friendly | Fewer features | ⚠️ |

### 6.2 Protocol selection recommendation matrix

| Protocol | Push ingest | Playback distribution | Low latency | NAT traversal | Mobile | Recommendation |
|------|:---:|:---:|:---:|:---:|:---:|:---:|
| RTMP | ✅ | ⚠️ | 1-5s | ❌ | ❌ | Must support (ecosystem compatibility) |
| SRT | ✅ | - | 0.5-2s | ⚠️ | ❌ | High (reliable pushing) |
| WebRTC/WHIP | ✅ | ✅ | <500ms | Requires TURN | ✅ | High (low-latency interaction) |
| HLS/LL-HLS | - | ✅ | 2-10s | ✅ | ✅ | Must have (large-scale distribution) |
| MPEG-DASH | - | ✅ | 2-10s | ✅ | ✅ | Recommended |
| HTTP-FLV | - | ✅ | 1-3s | ✅ | ⚠️ | Optional (Chinese ecosystem) |
| RTSP | ✅ | ✅ | 0.5-2s | ❌ | ❌ | Recommended (IPC ingestion) |
| Media-over-QUIC | ✅ | ✅ | <100ms | ✅ | - | Future direction |
| MoQ (QUIC) | ✅ | ✅ | <100ms | ✅ | - | Future direction |

### 6.3 MediaServo Technology Selection Recommendations

**Core architecture**: adopt LVQR's **Unified Fragment Model** as the internal unified data representation

**Rationale**:
1. SRS's RTMP-centered model falls short when extending to new protocols such as MoQ/QUIC
2. MediaMTX's path model lacks a unified internal media type
3. LVQR proves that one Fragment type can simultaneously serve MoQ, HLS, DASH, WHEP — 10 protocols, one internal representation
4. The performance advantage of zero virtual-function dispatch on the data plane (each Fragment call goes through concrete types, zero heap allocation)

**Suggested crate split** (referring to LVQR's 29-crate structure):
- `mediaservo-fragment` — Fragment type + FragmentBroadcaster + FragmentObserver trait
- `mediaservo-ingest-rtmp` — RTMP input bridge
- `mediaservo-ingest-srt` — SRT input bridge
- `mediaservo-ingest-whip` — WHIP/WebRTC input bridge
- `mediaservo-ingest-rtsp` — RTSP input bridge
- `mediaservo-egress-hls` — LL-HLS output
- `mediaservo-egress-dash` — MPEG-DASH output
- `mediaservo-egress-whep` — WHEP/WebRTC output
- `mediaservo-egress-moq` — MoQ over QUIC output
- `mediaservo-segmenter` — CMAF segmenter (fMP4 packaging)
- `mediaservo-record` — recording engine
- `mediaservo-cluster` — cluster gossip (optional)

**Phased implementation plan**:

| Phase | Content | Reference |
|------|------|------|
| Phase 1 | Fragment Model + RTMP ingest + HLS egress | LVQR RTMP bridge + hls crate |
| Phase 2 | SRT ingest + WHIP/WHEP | MediaMTX SRT configuration pattern |
| Phase 3 | RTSP ingest + MPEG-DASH egress | Xiu RTSP protocol implementation |
| Phase 4 | MoQ egress + cluster + mesh | LVQR relay + cluster + mesh |
| Phase 5 | QUIC unified transport + GPU hardware encoding | LVQR moq + GStreamer NVENC pipeline |

**Key decisions**:
- [ ] Use CMAF/fMP4 as the Fragment payload for the internal representation (LVQR has verified it works)
- [ ] Control plane with async-trait, data plane with concrete type dispatch (LVQR principle)
- [ ] Cluster with chitchat gossip rather than Raft (LVQR has verified it works, and it is sufficient)
- [ ] SLO system with independent thresholds per transport (LVQR's SLO document provides a template)

### 6.4 Pitfall Guide

1. **Do not use RTMP as the internal unified representation** — SRS's lesson: adding a new protocol requires twisting the adaptation to RTMP semantics
2. **Do not fork FFmpeg per stream** — nginx-rtmp's lesson: the concurrency ceiling is extremely low
3. **Do not use dyn trait on the data plane** — LVQR principle: zero heap allocation per Fragment is a key performance requirement
4. **HLS segment IDR alignment** — Twitch's lesson: when mixing transmuxed source streams with transcoded streams, IDR PTS alignment must be guaranteed
5. **TURN is an independent operational burden** — verified by both SRS and MediaMTX: production WebRTC deployment cannot get around TURN
6. **Do not add a private per-frame header to the MoQ wire** — LVQR's lesson: keep wire formats interoperable and carry extra data on a sidecar track
7. **HLS URL path parsing must be defensive** — Xiu's panic lesson: `index out of bounds` is unacceptable in production

## Related Decisions

| Research finding | Related decision |
|---------|---------|
| RTMP/HLS/SRT protocol stack (GStreamer) | D6, D19 |
| Origin-Edge cluster topology (SRS/MediaMTX/nginx-rtmp) | D-STREAM-TOPOLOGY |
| sourceOnDemand on-demand pulling (MediaMTX) | MediaSource trait reserved |
| GOP Cache instant join (SRS/MediaMTX) | D-GOP-CACHE |
| RTMP recording hooks (nginx-rtmp on_publish/on_done) | D152 |
| RTP interceptor extension point (Pion) | D153 |
