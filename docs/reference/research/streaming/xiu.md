# Xiu Streaming Server Reference Document

> Last updated: 2026-07-19 | Source repo: [harlanc/xiu](https://github.com/harlanc/xiu) | Latest version: v0.13.0

## 1. Product Profile

Xiu is a simple, high-performance, secure live-streaming server written in pure Rust, initiated and maintained by HarlanC (@harlanc) since August 2020. The project positions itself as a "developer-friendly" lightweight streaming middleware, emphasizing concise code, a clear architecture, and ease of extension.

| Dimension | Details |
|------|------|
| **Language** | Rust 97.7% (rest is JS/HTML/Shell) |
| **License** | MIT |
| **Latest version** | v0.13.0 (2024-08-11) |
| **GitHub Stars** | ~2,300 |
| **Contributors** | 30 people, 570+ commits |
| **crates.io downloads** | ~43,000 |
| **Official website** | https://rustxiu.com |
| **Supported platforms** | Linux / macOS / Windows |
| **Runtime dependencies** | tokio (async runtime) |

**Core positioning:** a pure-Rust streaming server aimed at small-to-medium live-streaming scenarios, selling "out-of-the-box" support for RTMP/RTSP/WebRTC/HTTP-FLV/HLS plus concise code.

## 2. Technical Characteristics

### 2.1 Rust Async Architecture — tokio

Xiu builds its entire async I/O and concurrency model on tokio:

```rust
#[tokio::main]
async fn main() -> Result<()> { ... }
```

- **Default multi-threaded runtime**: `#[tokio::main]` (the `current_thread` option is commented out), one worker thread per CPU core
- **Fully async stream processing**: all protocol handling (RTMP connections, RTSP sessions, HTTP-FLV/HLS distribution) runs in tokio tasks
- **Signal handling**: graceful shutdown via `tokio::signal::ctrl_c()`
- **Pub-sub bus**: cross-protocol, cross-connection async data distribution implemented through the `streamhub` crate

### 2.2 Modular Workspace Architecture

The project uses a Cargo workspace managing 15 crates, split into 4 layers by responsibility:

```
harlanc/xiu (workspace)
├── application/          # Application layer
│   ├── xiu/              # Main service entrypoint (main.rs + config + CLI)
│   ├── http-server/      # HTTP API / notification service
│   └── pprtmp/           # RTMP play/push tool
├── protocol/             # Protocol layer (standalone crates)
│   ├── rtmp/             # RTMP protocol implementation (0.6.5)
│   ├── rtsp/ (xrtsp)     # RTSP protocol implementation (0.3.0)
│   ├── webrtc/ (xwebrtc) # WebRTC WHIP/WHEP (0.3.5)
│   ├── httpflv/          # HTTP-FLV distribution (0.4.5)
│   └── hls/              # HLS distribution and recording (0.5.5)
├── library/              # Base library layer
│   ├── streamhub/        # Pub-sub routing core
│   ├── bytesio/          # Byte-stream read/write abstraction
│   ├── common/           # Common types/utilities
│   ├── logger/           # Logging extensions
│   ├── container/
│   │   ├── flv/ (xflv)   # FLV container parsing (0.4.4)
│   │   └── mpegts/       # MPEG-TS container parsing
│   └── codec/
│       └── h264/         # H.264 codec parsing
└── confs/                # Example configuration files
```

**Design traits**:
- Every protocol is a standalone crate, publishable to crates.io independently
- Protocol crates do not depend on each other directly; they are decoupled through `streamhub`
- `bytesio` provides a unified byte-stream I/O abstraction (TCP/UDP)

### 2.3 StreamHub — Pub-Sub Data Bus

The `streamhub` crate (v0.2.4) is the architectural core of Xiu, implementing cross-protocol data routing:

```
Publisher (RTMP/RTSP/WHIP)
         │
         ▼
    StreamsHub
    ├── publish()      — register a publisher, receive media data
    ├── subscribe()    — register a subscriber, forward data
    ├── unsubscribe()  — cancel subscription
    │
    └── StreamHubEventSender → event notification (HTTP API callback)
         │
         ▼
Subscriber (RTMP/RTSP/HTTP-FLV/HLS/WHEP)
```

**Core capabilities**:
- **Multi-protocol routing**: RTMP push → automatically converted to HTTP-FLV / HLS / RTMP pull / RTSP pull / WHEP
- **GOP cache**: configurable GOP count; a new subscriber first receives the most recent keyframe GOP, enabling fast first frame
- **Protocol conversion (Remux)**: RTMP → HTTP-FLV (direct reuse, no re-encode), RTMP → HLS (m3u8 + ts segments)
- **Statistics**: per-stream publish/subscribe stats via `StatisticDataSender`
- **Event notification**: observe stream publish/stop events via `BroadcastEventReceiver` (HTTP API callback)

### 2.4 Protocol Handling Details

#### RTMP (protocol/rtmp crate v0.6.5)
- Full RTMP handshake, chunk fragmentation/assembly, AMF0/AMF3 codec
- Supports H.264/AAC push and pull
- GOP cache: configurable `gop_num` controls the number of cached GOPs
- RTMP chunk compression
- Cluster mode: static Push/Pull relay
- Known issue: [audio-stream GOP cache leak](#issue190) (audio-only stream → OOM)

#### RTSP (protocol/rtsp crate xrtsp v0.3.0)
- Supports H.265(HEVC)/H.264/AAC
- Supports TCP Interleaved and UDP transport modes
- Supports push and pull
- Protocol conversion: RTSP → RTMP / HLS / HTTP-FLV

#### WebRTC (protocol/webrtc crate xwebrtc v0.3.5)
- WHIP (push): OBS 30.0+ can push to Xiu directly via WHIP
- WHEP (pull): built-in HTML/JS web player for direct in-browser playback
- Protocol conversion: WHIP → RTMP / HLS / HTTP-FLV
- Requires static web client files (the `webrtc/src/clients/` directory)

#### HTTP-FLV (protocol/httpflv crate v0.4.5)
- Distributes FLV-format streams over HTTP long connections (low latency)
- Uses axum (v0.6.10) as the HTTP framework
- Port 8080

#### HLS (protocol/hls crate v0.5.5)
- Generates m3u8 + ts segments from RTMP/RTSP streams
- Supports live recording (record to HLS)
- Port 8081

### 2.5 Codec Support

| Protocol | Video codec | Audio codec | Notes |
|------|---------|---------|------|
| RTMP | H.264 | AAC | HEVC/H.265 and AV1 not supported |
| RTSP | H.264, H.265(HEVC) | AAC | Only RTSP supports H.265 |
| WebRTC | H.264 | AAC | WHIP/WHEP mode |
| HTTP-FLV/HLS | Inherits source stream codec | Inherits source stream codec | Protocol conversion, no re-encode |

**Not supported**: AV1, VP8/VP9, Opus, SRT, GB28181

### 2.6 Key Dependencies

| Library | Purpose |
|---|------|
| tokio ^1.26 | Async runtime |
| axum ^0.6.10 | HTTP service framework (HTTP API + HTTP-FLV + HLS) |
| clap ^4.1 | CLI argument parsing |
| serde + serde_json | Serialization / config parsing |
| toml ^0.5.8 | TOML config file parsing |
| log + env_logger_extend | Logging system (supports log rotation) |
| anyhow | Application-level error handling |
| reqwest ^0.11.24 | HTTP notification callback client |

### 2.7 Compile Optimizations

```toml
[profile.release]
codegen-units = 1
lto = true
```

Single codegen unit + LTO, optimizing binary size and runtime performance.

## 3. Key Capabilities

### 3.1 Protocol Conversion Matrix

| Source → Target | RTMP | RTSP | HTTP-FLV | HLS | WHIP/WHEP |
|-----------|------|------|----------|-----|-----------|
| RTMP push | ✅ | ✅ | ✅ | ✅ | ✅ (WHEP) |
| RTSP push | ✅ | ✅ | ✅ | ✅ | ❌ |
| WHIP push | ✅ | ❌ | ✅ | ✅ | ✅ (WHEP) |

All protocol conversions are **transcode-free remux/repackaging**, with no codec involvement and low CPU cost.

### 3.2 Cluster / Relay

Two static relay modes are supported:

**Static Push (push mode)**:
- The source node pushes and the stream is automatically forwarded to configured downstream nodes
- Config example: `[[rtmp.push]] enabled = true, address = "node2", port = 1935`

**Static Pull (pull mode)**:
- When a downstream node has a playback request, it pulls from the upstream node
- Config example: `[rtmp.pull] enabled = true, address = "node1", port = 1935`

**Limitation**: static configuration only; no dynamic discovery, load balancing, or automatic failover.

### 3.3 HTTP API / Notifications

- **Query endpoints**: query the list of currently active streams and stream info
- **Notification callbacks**: send event notifications to a configured HTTP URL on stream publish/stop
- **Token authentication**: token verification for push/pull

### 3.4 Deployment Methods

```
# 1. Cargo install
cargo install xiu
xiu -r 1935 -f 8080 -s 8081

# 2. Docker
docker run -d --net=host --name xiu harlancn/xiu:latest /app/start.sh /app/config.toml

# 3. Build from source
git clone https://github.com/harlanc/xiu.git && cd xiu
make local && make build
./application/xiu/target/release/xiu -c config.toml

# 4. Docker Compose cluster
docker compose up -d  # 3-node cluster
```

### 3.5 Configuration System

Two mutually exclusive configuration modes:

1. **Config file** (TOML): `xiu -c config.toml`
2. **Command-line arguments**: `xiu -r 1935 -t 554 -w 8900 -f 8080 -s 8081 -l info`

Four preset configuration templates are provided:
- `config_rtmp.toml` — RTMP only
- `config_rtmp_hls.toml` — RTMP + HLS
- `config_rtmp_httpflv.toml` — RTMP + HTTP-FLV
- `config_rtmp_httpflv_hls.toml` — all protocols

## 4. Deployment & Operations

### 4.1 Typical Deployment Topology

```
┌──────────┐     RTMP/WHIP      ┌──────────┐     HTTP-FLV/HLS     ┌──────────┐
│  OBS /   │ ──────────────────▶│          │ ────────────────────▶│  Web/App │
│  FFmpeg  │                    │   XIU    │                      │  Player  │
└──────────┘                    │  Server  │◀──── RTMP/RTSP ──────└──────────┘
                                │          │
                                └────┬─────┘
                                     │ RTMP push relay
                                ┌────▼─────┐
                                │   XIU    │
                                │  Node 2  │
                                └──────────┘
```

### 4.2 Operational Essentials

| Aspect | Description |
|------|------|
| **Resource footprint** | Single binary <20MB; memory depends on the number of active streams and the GOP cache configuration |
| **Logging** | Log levels (trace~error), file output, log rotation (by hour/day) |
| **Monitoring** | HTTP API to query stream state; HTTP callback notifications for push/stream-loss events; no built-in metrics export |
| **Graceful shutdown** | `Ctrl+C` triggers `tokio::signal::ctrl_c()`, stopping the logger |
| **Docker** | Official image `harlancn/xiu:latest`, using host network mode |

### 4.3 Known Operational Risks

- **Audio-stream GOP memory leak**: [issue #190](https://github.com/harlanc/xiu/issues/190) — with a pure-audio RTMP push the GOP cache grows without bound, leaking ~1.5GB/day within 24 hours. Temporary workaround: set `gop_num = 0` to disable the GOP cache.
- **No real-time monitoring**: no Prometheus/Grafana integration; you must hook up the HTTP API yourself
- **No dynamic configuration**: changing configuration requires a service restart

## 5. Ecosystem & Market

### 5.1 Community Activity

| Metric | Data |
|------|------|
| GitHub Stars | ~2,300 |
| Contributors | 30 |
| Latest release | 2024-08 (v0.13.0) |
| Last push | 2026-03 (still maintained) |
| crates.io downloads | 43,000+ |
| Docs | rustxiu.com (Docusaurus) |

The project is led by a single maintainer (HarlanC) with 30 community contributors participating. Update frequency is moderate; the most recent code push was in March 2026. A Discord community exists but is small.

### 5.2 Market Positioning

Xiu occupies a unique position in the Rust streaming ecosystem:

| Comparison | Xiu (Rust) | SRS (Go) | Monibuca v6 (Rust) | Node-Media-Server (Node.js) | nginx-rtmp (C) |
|--------|-----------|----------|-------------------|---------------------------|----------------|
| Protocol coverage | RTMP/RTSP/WHIP/FLV/HLS | RTMP/HLS/WebRTC/GB28181 | All 8 protocols | RTMP/HTTP-FLV/WS-FLV | RTMP |
| Cluster | Static relay | Mature origin/edge | Enterprise-grade cluster | None | relay |
| Ops panel | None | Simple HTTP | Full admin panel | REST API | None |
| Performance target | Lightweight | High-performance CDN edge | Enterprise-grade (10K+ streams) | Medium-low load | High concurrency |
| Positioning | Learning-friendly / quick integration | CDN origin | Enterprise full stack | Quick prototyping | Stable relay |

### 5.3 Typical Use Cases

- ✅ Personal / small-team live-streaming service
- ✅ Development and test environments (Rust tech stack integration)
- ✅ Reference implementation for learning Rust streaming programming
- ✅ RTMP → HLS/HTTP-FLV protocol conversion gateway
- ⚠️ Small-to-medium production environments (mind the audio-stream memory issue)
- ❌ Large-scale CDN edge nodes (recommend SRS)
- ❌ Enterprise full-protocol solution (recommend Monibuca v6)

## 6. Highlights & Limitations

### 6.1 Highlights

1. **Pure Rust implementation**: memory safe, no GC, compile-time concurrency safety guarantees. Directly valuable as a reference for MediaServo, which also uses the Rust tech stack.
2. **Clear code architecture**: the 15-crate workspace has explicit layering (application/protocol/library), with each protocol independently reusable.
3. **Elegant StreamHub design**: the pub-sub pattern decouples protocol implementations; adding a protocol only requires hooking into the StreamHub interface.
4. **Protocol conversion without re-encode**: remux/repackaging mode with extremely low CPU cost.
5. **Out of the box**: one-command `cargo install`, flexible CLI arguments, Docker support.
6. **Cross-platform**: Linux/macOS/Windows fully supported.
7. **Concise code**: aimed at education and friendly integration; even beginners can understand it.

### 6.2 Limitations

1. **Limited codec support**: the RTMP side does not support H.265/AV1; the WebRTC side does not support VP8/VP9/Opus; no SRT, no GB28181.
2. **Weak clustering**: static relay only; no dynamic discovery, load balancing, or failover.
3. **Missing ops tooling**: no Prometheus metrics, no built-in admin UI, no health-check endpoint.
4. **Single-maintainer risk**: core development depends on one person; long-term sustainability is uncertain.
5. **Audio-stream OOM bug**: a serious production defect, reported but not yet fixed (see issue #190).
6. **Incomplete WebRTC**: basic WHIP/WHEP only, no SFU, no simulcast, no built-in TURN/STUN support.
7. **Simple recording**: HLS recording only, no MP4/FLV recording.
8. **No plugin system**: extending protocols requires modifying source code; no dynamic loading mechanism.
9. **Error handling via anyhow**: application-level code uses anyhow, unsuitable for fine-grained error handling in a library.

## 7. Reference Value for MediaServo

### 7.1 Directly Adoptable Patterns

1. **Workspace layering architecture** (★★★★★)
   - Xiu's `application/protocol/library` three-way split is structurally similar to MediaServo's `host/remote/server` three layers
   - Worth adopting: the independence of protocol crates and the pub-sub decoupling pattern of `streamhub`

2. **StreamHub pub-sub pattern** (★★★★★)
   - The `publish()` / `subscribe()` / `unsubscribe()` interface design is concise and efficient
   - MediaServo's PipelineEngine can adopt this pattern, connecting capture → encode → push nodes with channels
   - The `StatisticDataSender` statistics interface in `streamhub` is worth reusing

3. **bytesio byte-stream abstraction** (★★★★)
   - A unified TCP/UDP byte-stream read/write abstraction that reduces duplication in protocol-layer code
   - MediaServo likewise needs a unified byte-stream abstraction from capture source (screen/camera) to encoder to network push

4. **Remux strategy for protocol conversion** (★★★★)
   - Non-re-encoding protocol conversion (RTMP→HTTP-FLV reuses FLV data directly, RTMP→HLS only re-wraps into TS)
   - MediaServo may need the RTMP output → HTTP-FLV distribution scenario; remux mode minimizes CPU cost

5. **Configuration system design** (★★★)
   - TOML config file + CLI arguments in dual mode, mutually exclusive handling
   - Configurable GOP cache, ports, logging and so on

6. **tokio multi-threaded runtime** (★★★★)
   - Default multi-threaded tokio runtime, exploiting multiple cores
   - Graceful shutdown pattern via `tokio::signal`
   - Log rotation (`env_logger_extend`)

### 7.2 Designs to Avoid

1. **Avoid anyhow in library code** (★★★)
   - Xiu makes heavy use of `anyhow` and the `failure` crate, which makes fine-grained error handling difficult for library consumers
   - MediaServo should use the layered strategy of `thiserror` (library crates) and `anyhow` (application crates)

2. **Avoid the GOP-cache memory-leak pattern** (★★★)
   - The unbounded-audio-GOP bug shows that a cache needs a hard time/size cap and cannot rely only on keyframe-triggered eviction
   - MediaServo's GOP cache design should include forced eviction and a memory budget

3. **Avoid static-configured relay** (★★)
   - Static relay configuration lacks flexibility
   - MediaServo's Server should have dynamic session management and routing capability

4. **Avoid absent built-in observability** (★★)
   - Missing metrics, health checks, admin panel
   - MediaServo should consider observability from the start of the design

### 7.3 As a Reference Baseline for Rust Streaming

Xiu is currently the **simplest and most readable reference implementation** in the Rust streaming ecosystem, particularly suitable for:

- Understanding how RTMP/RTSP are implemented in Rust
- Understanding tokio usage patterns in streaming scenarios
- An architectural cross-check for MediaServo's MVP stage — Xiu's codebase is a moderate size and can be read quickly to extract useful patterns
- Comparative evaluation: when MediaServo needs more advanced features (dynamic routing, codec pipelines, plugin system), it requires an architecture deeper than Xiu's

### 7.4 Specifically Reusable Crates

| Xiu crate | Purpose | MediaServo reuse value |
|-----------|------|-------------------|
| `streamhub` | Pub-sub routing | High — inter-pipeline-node communication pattern |
| `bytesio` | Byte-stream I/O abstraction | High — unified network read/write interface |
| `rtmp` | RTMP protocol | Medium — if RTMP output is needed |
| `xflv` | FLV container | Medium — FLV wrapping/unwrapping |
| `xmpegts` | MPEG-TS container | Medium — HLS segmentation |
| `h264` | H.264 parsing | Low — MediaServo will use system codecs |

> **Core suggestion**: MediaServo does not need to directly fork or depend on Xiu's crates, but should study its architectural ideas carefully, especially the StreamHub decoupling pattern and the workspace layering organization.

---
**Related decisions**: D5 (Fragment Model), D6, D19

## Reference Sources

- GitHub: https://github.com/harlanc/xiu
- Official docs: https://rustxiu.com
- crates.io: https://crates.io/crates/xiu
- streamhub docs: https://docs.rs/streamhub
- Known issues: [issue #190 - GOP cache OOM for audio-only streams](https://github.com/harlanc/xiu/issues/190)
