# ZLMediaKit Reference Analysis
> Generated: 2026-07-19 | Category: Streaming

## 1. Product Profile
- **Name**: ZLMediaKit
- **Developer**: Primarily developed by Xia Chu (xia-chu / ZeroLogic), with about 30+ community contributors. Core maintainer xia-chu has contributed over 3600+ commits, alongside active contributors such as xiongguangjie (200+ commits) and wasphin (63+ commits)
- **First release**: 2017-04-01 (GitHub repo created). Over 9 years of continuous development, with production usage cases already present by November 2017
- **Current version**: Continuous rolling updates (master branch), no fixed major-version policy. A closed-source professional edition zlmediakit-pro is also offered (WebRTC clustering, AI inference and other enhancements)
- **Stars**: ~17.3K (2026-07)
- **License**: MIT + supplementary agreement (the "ZLMediaKit" branding information must be retained; copyright statements must not be removed from fields such as Server/User-Agent)
- **Product positioning**: a high-performance operational-grade streaming service framework. Positioned as "the bridge between the surveillance-camera protocol stack and the live-streaming protocol stack", covering everything from the low-level C++11 network library (ZLToolKit) to the full upper-level MediaServer. It can be used "out of the box" as a standalone streaming server, or embedded as a C/C++ SDK for secondary development in other applications
- **Target audiences**: security-surveillance system integrators (GB28181/RTP camera access and protocol conversion), live-streaming platforms and CDN providers (high-concurrency RTMP/HLS/HTTP-FLV distribution), IoT video application developers (multi-platform embedded SDK), WebRTC real-time communication applications (SFU scenarios), and C++ streaming secondary-development teams
- **Website**: https://docs.zlmediakit.com | **Repo**: https://github.com/ZLMediaKit/ZLMediaKit

## 2. Technical Characteristics

### Overall Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                    ZLMediaKit (C++11 multithreaded architecture)      │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────┐      │
│  │              EventPollerPool (event loop pool)                │      │
│  │                                                              │      │
│  │  N EventPoller instances created at startup = CPU core count │      │
│  │  Each EventPoller = 1 epoll instance + 1 thread on epoll_wait│      │
│  │  Each TcpSession binds to one EventPoller (near-single-      │      │
│  │  threaded, lock-free operation)                              │      │
│  │  Linux: epoll | other platforms: select                      │      │
│  └─────────────────────────────────────────────────────────────┘      │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────┐      │
│  │            WorkThreadPool (background worker pool)            │      │
│  │                                                              │      │
│  │  CPU-intensive jobs dispatched here (encode/decode,          │      │
│  │  snapshots, recording, etc.)                                 │      │
│  │  Separated from EventPoller to avoid blocking the IO loop     │      │
│  └─────────────────────────────────────────────────────────────┘      │
│                                                                       │
│  ┌──────────┬──────────┬──────────┬──────────┬──────────────┐         │
│  │ RtspSess │ RtmpSess │ HttpSess │ RtcTrans │ GB28181Sess  │         │
│  │ ion      │ ion      │ ion      │ port     │              │         │
│  ├──────────┴──────────┴──────────┴──────────┴──────────────┤         │
│  │                Protocol Normalization                      │         │
│  │      internal RTP ⇄ internal Frame (H.264/H.265/AAC/OPUS) │         │
│  │      RTSP/WebRTC share the low-level RTP path, with no     │         │
│  │      protocol-conversion overhead                          │         │
│  └───────────────────────────────────────────────────────────────┘      │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────┐      │
│  │        TcpServer (multithreaded accept load balancing)       │      │
│  │                                                              │      │
│  │  listen fd registered in all EventPollers → the kernel       │      │
│  │  picks the least busy thread automatically                   │      │
│  │  After accept the TcpSession stays bound to that EventPoller │      │
│  │  until the connection closes                                 │      │
│  └─────────────────────────────────────────────────────────────┘      │
└──────────────────────────────────────────────────────────────────────┘
```

### Threading Model

```
TaskExecutor (ThreadPool base class)
    └─ EventPoller (event poller)
           ├─ _epoll_fd: epoll instance fd
           ├─ _pipe_fd[2]: pipe used to wake epoll_wait
           ├─ _event_map: fd → event callback mapping
           ├─ _list_task: async task queue (batch thread switching)
           ├─ _delay_task_map: delayed tasks (sorted by execution time)
           └─ runLoop(): epoll_wait + event dispatch + task execution loop
    └─ EventPollerPool (singleton, manages N EventPollers, N = CPU core count)
```

ZLMediaKit uses a **single-process multithreaded + IO multiplexing + non-blocking** model:

- **Near-single-threaded, lock-free**: each TcpSession has its lifecycle owned by exactly one EventPoller and must not be manipulated directly by other threads. Therefore network IO inside TcpSession needs no mutex protection — lock granularity is reduced to the extreme
- **Batch thread switching**: when distributing media data, events are first aggregated by target thread in _dispatcherMap, then a single batch task is posted to each target thread. N target threads need only N thread switches (instead of one per client), greatly cutting context-switch overhead
- **Zero-copy forwarding**: leveraging C++11 shared_ptr reference counting, multiple target connections share the same data buffer, eliminating the memory copies typical of multithreaded programming
- **Object circular pool**: high-frequency allocations such as RTP packets are reused from a circular pool, reducing new/delete calls (globally-locked memory allocation is a performance bottleneck)
- **sendmsg batch send**: the sendmsg/writev syscalls merge multiple data buffers into one send, reducing syscall count
- **Socket tuning**: TCP_NODELAY is enabled during the handshake to improve latency, then TCP_NODELAY is disabled and MSG_MORE enabled afterwards to reduce ACK packets and raise bandwidth utilization
- **Comparison with SRS**: SRS uses a single-threaded multi-coroutine model (State Threads) running on one CPU core, unable to fully exploit multicore. ZLMediaKit is multithreaded with one thread per CPU core, squeezing out multicore performance. Under low load ZLMediaKit's single-threaded throughput is about 50% of SRS's (SRS has merged-write caching that sends in batches after 300ms), but in real networks under heavy load the gap narrows and aggregate multicore performance far exceeds SRS

### Protocol Normalization

Both RTSP and WebRTC are RTP-based at the bottom layer, so ZLMediaKit can interconnect them internally with no complex demux/remux logic. Other protocols (RTMP, HLS, HTTP-FLV, etc.) are converted through an internal Frame data structure, supporting cross-protocol conversion for H.264/H.265/AAC/G711/OPUS/MP3/VP8/VP9/AV1/MP2; unsupported codecs are only passed through transparently.

### Module Organization

| Module | Description |
|------|------|
| ZLToolKit (base library) | Four major parts — Thread/Poller/Network/Util — providing the EventPoller event engine, TcpServer/TcpSession network abstractions, and an async task framework |
| server/ | MediaServer main program, an out-of-the-box streaming server |
| src/ | Core APIs: Rtsp/Rtmp/Hls/Http/WebRTC/GB28181/Record and other modules |
| api/ | C API SDK for use from other languages |
| tests/ | Test tools: test_benchmark (performance stress test), test_bench_pull (pull stress test), test_bench_push (push stress test), test_server (test server) |
| webrtc/ | WebRTC implementation (standalone submodule) |
| srt/ | SRT protocol support (standalone submodule) |

### Key Design Decisions

- **Smart-pointer memory management**: C++11 shared_ptr/weak_ptr used throughout, avoiding raw pointers, managing memory sharing and lifetime perfectly across thread switches
- **On-demand demux**: protocol conversion/demux is enabled only when viewers exist, and shuts down automatically when nobody is watching, lowering CPU usage. Intelligent stream shutdown is implemented by combining the hook event (on_stream_none_reader) with business logic
- **Play before publish**: when a player requests a non-existent stream the server does not return an error immediately, but suspends and waits for a push. Once the push arrives the request is answered successfully — this lowers video open latency and improves user experience
- **Interrupted push continues**: after the pusher disconnects abnormally, the server delays reclaiming media source resources (configurable timeout); if the pusher reconnects within the timeout the resources are reused and players are unaware
- **HLS long-connection simulation**: using HTTP Cookie tracking, stateless HLS segment requests are simulated as a "long connection", achieving on-demand HLS pulling, playback authorization (no repeated auth), and playback traffic accounting
- **Codec format symmetry**: WebRTC's supported codec formats match the RTSP protocol (H.264/H.265/AAC/G711/OPUS/MJPEG/MP3/VP8/VP9/AV1), simplifying cross-protocol conversion
- **enhanced-rtmp support**: beyond traditional RTMP-H265/OPUS, the veovera enhanced-rtmp standard is supported (H.265/VP8/VP9/AV1/OPUS)

## 3. Key Capabilities

### Protocol Support Overview

| Protocol | Role | H264 | H265 | AAC | G711 | OPUS | VP8 | VP9 | AV1 | Notes |
|------|------|------|------|-----|------|------|-----|-----|-----|------|
| RTSP[S] | Server/Pusher/Player | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 4 RTP transport modes; Basic/Digest auth |
| RTMP[S] | Server/Pusher/Player | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | enhanced-rtmp support |
| HLS (mpegts/fmp4) | Server/Player | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Multi-track mode; Cookie long-connection tracking |
| HTTP-FLV | Server/Player | ✅ | ✅ | ✅ | ✅ | ✅ | — | — | — | WebSocket-FLV also supported |
| HTTP-TS | Server | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | WebSocket-TS also supported |
| HTTP-fMP4 | Server | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | WebSocket-fMP4 also supported |
| MP4 | VOD/recording | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Seek supported; multi-track recording |
| WebRTC | SFU/Client | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | WHIP/WHEP; Simulcast; RTCDataChannel |
| GB28181/RTP | Server/Client | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Two-way voice intercom; PS/TS/ES/EHOME |
| SRT | Server/Client | ✅ | ✅ | ✅ | ✅ | ✅ | — | — | — | TS pass-through mode |
| STUN/TURN | Server | — | — | — | — | — | — | — | — | Built-in STUN/TURN service |

### WebRTC SFU Features

ZLMediaKit's WebRTC implementation is one of its most competitive modules, with several unique features in the open-source world:

- **Single port + multithreading + connection migration**: through repeated bind/connect operations each client is assigned a unique fd and evenly distributed across threads. A single UDP port carries all WebRTC clients, breaking the 60k-port limit of traditional multi-port designs (theoretically supporting millions of clients). When a user's network changes (WiFi→4G) the connection is re-anchored via STUN packets, supporting seamless migration — the only open-source solution with all three capabilities at once
- **ICE-Full support**: can act as a WebRTC client to actively pull/push, supporting P2P mode
- **Simulcast + RTX/NACK**: bidirectional loss retransmission combined with an excellent jitter buffer algorithm delivers outstanding loss resistance
- **TWCC dynamic bitrate control**: Transport-Wide CC based adaptive bitrate adjustment
- **GOP-buffer instant start**: recent GOPs are cached to achieve WebRTC instant playback
- **WHIP/WHEP protocols**: standardized WebRTC push/pull signaling protocols supported

### Recording Capabilities

- Recording to three formats: FLV, HLS (mpegts), MP4
- Recording can be triggered through the HTTP API; real-time screenshots are supported and returned
- Multi-track recording (multiple audio/video streams)

### Cluster Deployment

- **Origin-tracing mode**: Origin + Edge architecture, enabled via configuration file
- **Origin-tracing protocols**: RTSP, RTMP, HLS, HTTP-TS, HTTP-FLV tracing methods
- **Load balancing**: origins support multiple addresses, selected via Round Robin
- **Edge HLS**: ZLMediaKit's unique HLS edge support — SRS edges do not support HLS
- **Professional edition enhancements**: WebRTC cluster support with RTC traffic proxying, solving the problem that signaling and media streams cannot hit the same Pod in K8s deployments

### API and Monitoring

- **RESTful API**: comprehensive HTTP API covering stream management, recording control, screenshots, thread load queries, configuration hot reload, etc.
- **Web Hook**: event-driven callbacks supporting push auth (on_publish), playback auth (on_play), stream register/unregister (on_stream_changed), no viewer (on_stream_none_reader), stream not found (on_stream_not_found), traffic reporting (on_flow_report) and other events
- **Telnet debugging**: a simple Telnet command-line debugging interface is supported
- **Hot reload**: configuration files and SSL certificates support hot reload without a service restart

## 4. Deployment & Operations

### Deployment Modes

- **MediaServer binary**: compile directly or download a prebuilt executable; one command to start
- **C/C++ SDK embedding**: link ZLMediaKit as a library into your own C++ application with custom business logic
- **C API wrapper**: called from Go, C#, Python, Java and other languages via the C API SDK
- **Docker**: an official `zlmediakit/zlmediakit` Docker image, pullable directly from Docker Hub

```bash
# Docker deployment example
docker run -d \
  -p 1935:1935 -p 8080:80 -p 8554:554 \
  -p 10000:10000/udp -p 8000:8000/udp \
  zlmediakit/zlmediakit
```

### Configuration Format

INI configuration file format (based on the mINI parser). The config file path defaults to `config.ini` under the working directory. Main configuration sections:

| Config section | Description |
|---------|------|
| [http] | HTTP server port, cross-origin, virtual hosts, etc. |
| [rtmp] | RTMP server port |
| [rtsp] | RTSP server port, timeout settings |
| [rtc] | WebRTC configuration, STUN/TURN addresses, port range |
| [rtp] | RTP proxy, GB28181-related configuration |
| [rtp_proxy] | RTP push proxy configuration |
| [hls] | HLS segment duration, file retention policy |
| [record] | Recording path, segment size, file type |
| [general] | General settings: stream timeout, merged-write toggle, virtual hosts, origin-tracing cluster |
| [srt] | SRT protocol configuration |
| [api] | API key, debugging toggles |
| [hook] | Web Hook callback URL, timeout, retries |
| [ffmpeg] | FFmpeg executable path (used for pull proxying) |
| [cluster] | Cluster mode configuration, origin-tracing addresses, timeout |

### Monitoring and Observability

- **Thread load API**: `/index/api/getThreadsLoad` retrieves EventPoller thread load; `/index/api/getWorkThreadsLoad` retrieves background worker thread load
- **Performance metrics**: each thread returns load (load percentage), delay (latency in ms), fd_count (number of file descriptors being watched)
- **Traffic reporting hook**: per-stream traffic data is obtained through the `on_flow_report` event
- **Playback statistics**: playback volume monitoring is implemented via the `on_stream_none_reader` event and the viewer statistics feature

### Platform Support

- **Operating systems**: Linux, macOS, Windows, iOS, Android (all platforms)
- **CPU architectures**: x86, ARM, RISC-V, MIPS, Loongson (LoongArch), Shengwei (SW)
- **Build requirements**: CMake 3.1+, a C++11-capable compiler (GCC 4.8+ / Clang 3.3+ / MSVC 2015+)

## 5. Ecosystem & Market

### Community Activity

- GitHub Stars: ~17.3K, Forks: ~4.1K
- Open Issues: ~130+ (actively maintained)
- Contributors: 30+ core, primarily driven by the Chinese developer community
- Documentation: docs.zlmediakit.com online docs (Chinese/English); the Wiki covers performance tests, API reference, and FAQs

### Third-Party Ecosystem Projects

| Project | Description |
|------|------|
| wvp-GB28181-pro | GB28181 signaling server implemented in Go, deeply integrated with ZLMediaKit |
| ZLMRTCClient | WebRTC client SDK (iOS/Android) |
| ZLMediaKit-CSharp-API | Complete C# wrapper library for the C SDK |
| node-zlmediakit | Node.js HTTP API client |
| metaRTC | A domestic WebRTC SDK used together with ZLMediaKit |
| ZLMDataView | ZLMediaKit management console (Vue.js) |

### Adoption Cases

- **Security surveillance**: large-scale GB28181 camera access scenarios using the wvp-GB28181-pro + ZLMediaKit combination
- **DJI drone video streaming**: measured data at 4K/30fps, with WebRTC latency of 112±18ms (LAN) far better than SRS's 238±32ms
- **Smart vehicles**: in-vehicle camera RTMP push plus the play-before-publish mechanism, solving vehicle-device bring-up latency
- **IoT video**: deployment on ARM embedded platforms, with support for domestic platforms such as RISC-V/Loongson/Shengwei
- **Enterprise live streaming**: high-concurrency RTMP/HLS/HTTP-FLV distribution, supporting tens of thousands of concurrent connections

### Competitive Comparison

| Dimension | ZLMediaKit | SRS | Janus | mediasoup | Live555 |
|------|-----------|-----|-------|-----------|---------|
| Protocol richness | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐ | ⭐⭐ | ⭐⭐ |
| Single-node concurrency | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐ |
| WebRTC SFU | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ❌ |
| Cross-platform | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ |
| Surveillance protocols | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ | ❌ | ❌ | ⭐⭐⭐ |
| Secondary development | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐ |
| Community size | ⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐ |
| Learning curve | Medium | Low | Medium-high | High | High |

## 6. Highlights & Limitations

### Highlights (Strengths)

| Highlight | Description |
|------|------|
| **Multicore concurrency performance** | The multithreaded model fully exploits multicore CPUs; a single node supports 100k-level players and 100Gb/s bandwidth, with RTMP pulling reaching 30k streams at 7Gb/s |
| **WebRTC single-port multithreading** | The only open-source solution supporting a single UDP port + multithreading + client connection migration, breaking the 60k-port limit |
| **Zero-copy + lock-free design** | Smart-pointer reference-counted zero-copy forwarding + near-single-threaded lock-free TcpSession operations for peak performance |
| **Full protocol coverage** | Simultaneously covers the live-streaming protocol stack (RTMP/HLS/HTTP-FLV) and the surveillance protocol stack (RTSP/GB28181/RTP), connecting surveillance with live streaming |
| **Cross-platform, all architectures** | Linux/macOS/Windows/iOS/Android + x86/ARM/RISC-V/MIPS/Loongson/Shengwei, meeting domestic-platform requirements |
| **Production-grade reliability** | Years of commercial validation, long-term valgrind memory-safety testing, mature hot reload, interrupted-push continuity, and cluster features |
| **Play before Publish** | A unique playback suspension-and-wait mechanism that significantly lowers video open latency, improving the IoT/vehicle experience |
| **HLS long-connection simulation** | Cookie tracking simulates stateless HLS segment requests as a long connection, enabling playback authorization, traffic accounting, and on-demand pulling |
| **RTSP + WebRTC internal RTP interoperability** | Sharing the low-level RTP path avoids protocol-conversion overhead, suitable for large-scale low-latency WebRTC live streaming |

### Limitations (Limitations)

| Limitation | Description |
|------|------|
| **No transcoding capability** | Server-side video transcoding is not supported; the device side must provide the target codec format. Bitrate adaptation or format conversion requires external FFmpeg (though an FFmpeg pull proxy is supported) |
| **Slightly lower single-threaded low-load performance** | Under low load + ideal network conditions, single-threaded performance is about 50% of SRS's (no merged-write optimization), but the advantage is pronounced in heavy-load multicore scenarios |
| **Primarily a Chinese community** | Documentation and community discussion are mainly in Chinese; internationalization lags behind SRS/Janus |
| **Closed-source professional edition** | Advanced features (WebRTC clustering, AI inference, etc.) require paid use of the closed-source professional edition |
| **Configuration complexity** | Many feature modules, with 100+ INI options; full mastery has a steep learning curve |
| **No official admin UI** | No web management backend is provided; it relies on third-party projects (ZLMDataView) or self-development |
| **Limited codec support** | AV1 pass-through is not supported under RTMP (enhanced-rtmp only); some protocol combinations do not support certain codec conversions |
| **License supplementary clause** | The MIT supplementary agreement requires retaining ZLMediaKit branding, limiting white-label/OEM scenarios |

## 7. Reference Value for MediaServo

### What to Adopt

- **EventPoller multithreaded model**: MediaServo's Host/Server sides could consider a similar multithreaded event-loop architecture. One EventPoller per CPU core with the lock-free pattern of TcpSession bound to a fixed thread is a proven high-concurrency solution. In Rust, a similar effect can be simulated with tokio's `tokio::spawn` + `Runtime::new()`
- **Shared RTP path for RTSP + WebRTC**: MediaServo needs both surveillance camera access (RTSP/ONVIF) and WebRTC teleoperation; ZLMediaKit's RTP normalization design can be adopted to avoid repeated unwrapping/wrapping during RTSP→WebRTC conversion
- **Play-before-publish mechanism**: MediaServo's vehicle-side push scenario (vehicle cameras pushing to the cloud) is exactly the case where "the playback request arrives first and the push later". ZLMediaKit's play-before-publish suspension-and-wait mechanism can be referenced directly
- **Interrupted push continues**: unstable network on edge devices is the norm. The ability to delay resource reclamation after a pusher disconnects and to reconnect transparently is critical for MediaServo's vehicle/mobile scenarios
- **Batch thread switching + zero copy**: when distributing media data to multiple Peers, aggregating by thread and batching task posts + shared_ptr shared buffers can be reproduced in Rust with `Arc<[u8]>` + `tokio::spawn`

### What to Adapt

- **Protocol normalization layer**: ZLMediaKit uses RTP/Frame as its core normalization format internally. MediaServo can choose its own normalization format as needed (RTP-based is recommended, since MediaServo's WebRTC teleoperation and RTSP camera access are both RTP-based)
- **Recording and playback**: MediaServo's remote desktop recording can borrow ZLMediaKit's multi-format recording (FLV/MP4/HLS) design, combined with MediaServo's own GPU encoding capability
- **WebRTC single-port multithreading**: if MediaServo's WebRTC service must carry a large number of clients, ZLMediaKit's single-port multithreaded approach is the best reference implementation. On the Rust side, `SO_REUSEPORT` + `connect` binding can achieve a similar effect
- **Cluster origin-tracing mode**: with MediaServo Server as the signaling + relay center, ZLMediaKit's origin-tracing cluster mode can be referenced when designing an edge-origin architecture. The tracing protocol can be extended into MediaServo's internal protocol

### What to Avoid

- **Bare C++ development**: choosing Rust for MediaServo is a sound decision. ZLMediaKit is a very high code-quality C++ project (smart pointers, no raw pointers), but C++ memory safety still demands high developer discipline. Rust's compile-time memory safety guarantees fit MediaServo's safety-sensitive scenarios (remote desktop, vehicle control)
- **Single INI configuration file**: ZLMediaKit's 100+ option INI file delivers a poor experience for novice deployment and automated management. MediaServo should consider structured configuration (YAML/TOML) with environment variable overrides, unified through the OMO management surface
- **No official admin UI**: as an AUDE ecosystem component, MediaServo should provide a management interface (gRPC API + Web UI) from Phase 1, avoiding dependence on third-party management tools
- **No built-in transcoding**: MediaServo should evaluate whether server-side transcoding is needed. For remote desktop scenarios, terminal capability varies widely (different resolutions, different decoder support), so transcoding may be a hard requirement. A GPU hardware-encoding transcode path could be considered
- **C++ dependency chain**: ZLMediaKit depends deeply on the C++ toolchain (CMake, GCC/Clang); cross-compiling to ARM/MIPS requires a full toolchain. MediaServo's Rust + cargo toolchain is more modern for cross-compilation and dependency management
**Related decisions**: D-STREAM-TOPOLOGY, D-GOP-CACHE
