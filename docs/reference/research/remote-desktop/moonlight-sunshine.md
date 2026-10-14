# Moonlight + Sunshine Reference Analysis
> Generated: 2026-07-16 | Category: Remote Desktop

## 1. Product Profile
- **Name**: Moonlight (client) + Sunshine (server)
- **Developers**:
  - Moonlight: started by the open-source community (founded by CaseySJ, originally Limelight), implemented via reverse engineering of the NVIDIA GameStream protocol
  - Sunshine: developed by the LizardByte community — an open-source C++ GameStream-compatible server
- **First release**:
  - Moonlight (Limelight): 2013 (the first Android client)
  - Sunshine: 2020 (the community started its own server on the eve of NVIDIA ending GameStream)
- **Positioning**: the ultimate open-source game-streaming stack. Moonlight is a high-performance all-platform client; Sunshine is an all-GPU-vendor server. After NVIDIA formally ended GameStream in 2023, Sunshine+Moonlight became the de facto standard for open-source game streaming
- **Target users**:
  - Gamers (streaming PC games to phones/tablets/TVs/handhelds, on LAN or remotely)
  - Home streaming users (projecting a study PC to the living-room TV via Steam Deck/Apple TV/Nvidia Shield)
  - DIY cloud-gaming users (personal cloud setups over VPN or Tailscale)
  - Remote-desktop users (using the low-latency encoding for non-gaming control)
  - The open-source community (studying a complete game-streaming protocol implementation)
  - Extreme client reach (embedding on Switch/PS Vita/Steam Link and other non-mainstream platforms)
- **License / business model**:
  - Moonlight PC: GPLv3
  - Moonlight Android/iOS/tvOS/Switch: independent repos, GPLv3 each
  - Sunshine: GPLv3
  - Fully free; no commercial edition, no ads, no data collection

## 2. Technical Characteristics
### Overall Architecture
```
┌──────────────────────────────────────────────────────────────────┐
│          Moonlight + Sunshine — the six-protocol separated arch   │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │              Sunshine Host (server — C++/Qt)               │  │
│  │                                                             │  │
│  │  Screen-capture abstraction layer (multi-backend):          │  │
│  │  ┌──────────┬──────────┬──────────┬──────────────────┐     │  │
│  │  │  DXGI    │ KMS/DRM  │  NvFBC   │ X11 / Wayland    │     │  │
│  │  │ (Win)    │ (Linux)  │ (NVIDIA  │ wlroots / XDG /  │     │  │
│  │  │          │          │  propr.) │ KWin Screencast  │     │  │
│  │  └──────────┴──────────┴──────────┴──────────────────┘     │  │
│  │                         ↓                                    │  │
│  │  FFmpeg-style GPU-encoder abstraction layer                   │  │
│  │  ┌───────┬──────┬──────────┬─────────┬─────────┬──────┐     │  │
│  │  │NVENC  │ AMF  │QuickSync │  VAAPI  │   VT    │ SW   │     │  │
│  │  │NVIDIA │ AMD  │  Intel   │  Linux  │  macOS  │ CPU  │     │  │
│  │  ├───────┼──────┼──────────┼─────────┼─────────┼──────┤     │  │
│  │  │  MF   │Vulkan │          │         │         │      │     │  │
│  │  │ Win   │ Video │          │         │         │      │     │  │
│  │  └───────┴──────┴──────────┴─────────┴─────────┴──────┘     │  │
│  │                                                             │  │
│  │  Codecs: H.264 / HEVC (H.265) / AV1                          │  │
│  │  Chroma: YUV 4:2:0 / YUV 4:4:4 / HDR (HDR10/HLG)            │  │
│  └────────────────────────────────────────────────────────────┘  │
│                             │                                    │
│     ┌───────────────────────┼───────────────────────┐            │
│     │              ┌────────┼────────┐              │            │
│     ▼              ▼        │        ▼              ▼            │
│  HTTPS(pair)   RTSP(ctrl)   │   RTP video(47998) RTP audio(48000)│
│  TCP 47984     TCP 48010    │   UDP             UDP              │
│     │              │        │        │              │            │
│     │              │   ENet │(input)  │              │            │
│     │              │  UDP 47999    │              │            │
│     │              ▼        │       │              │            │
│     └──────────────┼────────┘       └──────────────┘            │
│                    │                                             │
│  Control plane (TCP)   │        Data plane (UDP)                │
│  Reliability first     │        Low latency first                │
│                        │                                         │
│  ┌─────────────────────┴─────────────────────────────────────┐  │
│  │              Moonlight client (Qt/C++ / native)           │  │
│  │                                                             │  │
│  │  · HW-accelerated decode: VAAPI / D3D11VA / VideoToolbox / │  │
│  │                  NVDEC / VDPAU / Vulkan Video              │  │
│  │  · GPU render: pixel-shader color conversion (YUV→RGB)     │  │
│  │  · Performance overlay: live encode/network/decode/render  │  │
│  │                  latency breakdown                         │  │
│  │  · Peripherals: 16 pads + force feedback + motion (gyro/   │  │
│  │                  accelerometer)                            │  │
│  │  · Audio: 7.1 surround Opus/AAC low-latency decode         │  │
│  └─────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### The Six Protocols in Detail
```
┌─────────────────────────────────────────────────────────────────────┐
│              GameStream's six protocols: duties and intent           │
├────────┬─────────┬─────────┬────────────────────────────────────────┤
│ Proto  │Transport│  Port   │ Duty                                   │
├────────┼─────────┼─────────┼────────────────────────────────────────┤
│ HTTP   │ TCP     │ 47989   │ Service discovery: fetch server lists, │
│        │         │         │ query supported codecs, resolution,    │
│        │         │         │ framerate, HDR capability              │
│        │         │         │ Stateless req/resp, REST-like          │
├────────┼─────────┼─────────┼────────────────────────────────────────┤
│ HTTPS  │ TCP     │ 47984   │ Pairing auth: on first connect the     │
│        │         │         │ client enters a PIN; the server issues │
│        │         │         │ a client cert for future PIN-free use  │
│        │         │         │ TLS + mutual certificate validation    │
├────────┼─────────┼─────────┼────────────────────────────────────────┤
│ RTSP   │ TCP     │ 48010   │ Stream-session control: SDP negotiates │
│        │         │         │ codec params (encoder type/resolution/ │
│        │         │         │ fps/chroma/bitrate); start/stop/pause  │
│        │         │         │ streams; dynamic resolution switching  │
│        │         │         │ The "control signaling" role, SIP-like │
├────────┼─────────┼─────────┼────────────────────────────────────────┤
│ RTP    │ UDP     │ 47998   │ Video transport: H.264/H.265/AV1 NAL   │
│ (video)│         │         │ units packaged into RTP with sequence   │
│        │         │         │ numbers and timestamps; unidirectional  │
│        │         │         │ (Server → Client); no retransmit (a     │
│        │         │         │ dropped frame is fine, next is coming)  │
├────────┼─────────┼─────────┼────────────────────────────────────────┤
│ RTP    │ UDP     │ 48000   │ Audio transport: Opus/AAC, 7.1         │
│ (audio)│         │         │ surround on its own port/stream, fully  │
│        │         │         │ decoupled from video                    │
├────────┼─────────┼─────────┼────────────────────────────────────────┤
│ ENet   │ UDP     │ 47999   │ Reliable control channel: ENet gives   │
│        │         │         │ UDP-based reliability (TCP-like, UDP-   │
│        │         │         │ latency) for pad/keyboard-mouse/touch   │
│        │         │         │ input, pad force-feedback commands,     │
│        │         │         │ motion (IMU) data                       │
└────────┴─────────┴─────────┴────────────────────────────────────────┘

Summary of intent:
┌─────────────────────────────────────────────────────────────────────┐
│  Control plane: HTTP/HTTPS/RTSP → all TCP                           │
│  · Needs reliability (no PIN or stream-control command may be lost) │
│  · Tiny volume (KB-scale), TCP overhead acceptable                  │
│                                                                     │
│  Data plane: RTP video / RTP audio / ENet input → all UDP           │
│  · Needs low latency (TCP HOL blocking and retransmit are disaster  │
│    for real-time streams)                                            │
│  · Large volume (video 10-100 Mbps); UDP has no congestion burden   │
│  · ENet as the exception: selective reliability over UDP (only       │
│    input data needs it)                                              │
│                                                                     │
│  Benefits of six separate ports:                                     │
│  · Independent QoS per flow (video > audio > input priority)         │
│  · No interference (video stutter doesn't drop input or cut audio)   │
│  · Each manageable by firewall rules independently (though many      │
│    ports is also a downside)                                          │
└─────────────────────────────────────────────────────────────────────┘
```

### The Seven Encoding APIs — Compatibility Matrix
```
┌─────────────────────────────────────────────────────────────────────┐
│       Sunshine encode API × GPU vendor × platform, 3-D matrix       │
├────────────┬────────┬────────┬────────┬────────┬────────┬───────────┤
│ Encode API │ NVIDIA │  AMD   │ Intel  │ Qualc. │ Apple  │ Platform  │
├────────────┼────────┼────────┼────────┼────────┼────────┼───────────┤
│ NVENC      │   ✅   │   ➖    │   ➖    │   ➖   │   ➖   │ Win/Linux │
│ AMF        │   ➖    │   ✅   │   ➖    │   ➖   │   ➖   │ Windows   │
│ QuickSync  │   ➖    │   ➖    │   ✅   │   ➖   │   ➖   │ Windows   │
│ VAAPI      │   ✅   │   ✅   │   ✅   │   ➖   │   ➖   │ Linux/BSD │
│ VideoToolbx│   ➖    │   ➖    │   ➖    │   ➖   │   ✅   │ macOS     │
│ MediaFound.│   ✅   │   ✅   │   ✅   │   ✅   │   ➖   │ Windows   │
│ Vulkan Video│  ✅   │   ✅   │   ✅   │   ➖   │   ➖   │ Linux     │
│ Software   │   ✅   │   ✅   │   ✅   │   ✅   │   ✅   │ all       │
├────────────┼────────┼────────┼────────┼────────┼────────┼───────────┤
│ Codecs     │H.264/  │H.264/  │H.264/  │H.264/  │H.264/  │           │
│            │HEVC    │HEVC    │HEVC    │HEVC    │HEVC    │           │
│ + AV1      │✅NVENC │➖       │✅QSV   │➖       │➖       │ GPU 4000+ │
└────────────┴────────┴────────┴────────┴────────┴────────┴───────────┘
```

### Screen Capture × Encoding Compatibility Matrix (Linux/FreeBSD)
```
┌─────────────────────┬───────┬──────────┬──────────────┬──────────────┐
│ Capture method      │ VAAPI │ VulkanVid│ NVENC (CUDA) │ Software     │
├─────────────────────┼───────┼──────────┼──────────────┼──────────────┤
│ KMS/DRM             │  ✅   │    ✅    │      ✅      │     ✅       │
│ NvFBC (NVIDIA propr)│  ❌   │    ❌    │      ✅      │     ❌       │
│ Wayland (wlroots)   │  ✅   │    ❌    │      ✅      │     ✅       │
│ X11                 │  ✅   │    ❌    │      ✅      │     ✅       │
│ XDG Desktop Portal  │  ✅   │    ✅    │      ✅      │     ✅       │
│ KWin Screencast     │  ✅   │    ✅    │      ✅      │     ✅       │
│ Windows.Graphics    │  ➖   │    ➖    │      ➖      │     ➖        │
│ Capture (Win)       │       │          │              │              │
└─────────────────────┴───────┴──────────┴──────────────┴──────────────┘
```

### Key Technical Capabilities
| Capability | Details |
|------------|---------|
| Video encoding | H.264 / HEVC (H.265) / AV1; NVENC / AMF / QSV / VAAPI / VideoToolbox / Media Foundation / Vulkan Video / Software |
| Chroma subsampling | YUV 4:2:0 (standard game quality), YUV 4:4:4 (crisp text and accurate color — Sunshine+Moonlight only; among commercial products only Parsec Warp supports it) |
| HDR | HDR10 and HLG, 10-bit depth |
| Audio | 7.1 surround; Opus (modern, low latency, broad support), AAC (legacy-device compatibility) |
| Transport protocol | Six-protocol separation: HTTPS (pairing auth) / RTSP (stream control SDP) / RTP (video 47998) / RTP (audio 48000) / ENet (input control 47999) |
| Security | HTTPS PIN pairing + client certificates (PIN-free after first pairing); optional VLAN isolation; on LAN no extra encryption by default (latency first) |
| Input control | The ENet reliable-UDP channel: keyboard/mouse/touch + up to 16 simultaneous pads + force feedback (dual-motor rumble) + motion (6-axis IMU: gyroscope + accelerometer) + 10-point multitouch |
| Platform support (Host) | Windows 11+ / Linux (Debian/Ubuntu/Fedora) / macOS 14.2+ / FreeBSD 14.4+ |
| Platform support (Client) | Windows / macOS / Linux / iOS / Android / tvOS / Nintendo Switch (homebrew) / PS Vita (homebrew) / Steam Link / Web / Raspberry Pi |
| Hardware requirements | Minimum: AMD VCE 1.0+ / Intel Skylake QSV+ / NVENC-capable NVIDIA GPU; 4K: RTX 2000+ or GTX 1080+ / Ryzen 5 / Core i5+; 4GB+ RAM |

### Technology Stack
- **Sunshine server**:
  - Language: C++ (core codebase) + Qt (the Web UI configuration interface)
  - Screen-capture backends: DXGI (Windows) / KMS-DRM (Linux kernel framebuffer) / NvFBC (NVIDIA proprietary framebuffer capture) / PipeWire (Linux unified A/V graph) / wlroots / XDG Desktop Portal / KWin Screencast (the three Wayland capture protocols) / X11 / ScreenCaptureKit (macOS) / Windows.Graphics.Capture (Win10+ API)
  - Encoding abstraction: FFmpeg-style APIs (not a direct FFmpeg dependency but borrowing its codec-factory pattern; each encoder backend wrapped as an independent module)
  - Web UI config: an embedded HTTP server (built into Qt) + a responsive web interface
  - Service management: systemd (Linux) / launchd (macOS) / Windows Service
  - Logging: spdlog (a high-performance C++ log library)
- **Moonlight client**:
  - Desktop: Qt 6.7 + C++ (one codebase for Windows/macOS/Linux)
  - Mobile: native per platform
    - Android: Kotlin + Java; the NDK for hardware decode (MediaCodec)
    - iOS/tvOS: Swift + ObjC; VideoToolbox hardware decode
  - Third-party ports: Nintendo Switch / PS Vita (community-maintained on each platform's native SDK)
  - Hardware-decode backend matrix:
    - Windows: D3D11VA (Direct3D 11 Video Acceleration) / NVDEC / DXVA2
    - Linux: VAAPI (Video Acceleration API) / VDPAU (NVIDIA) / Vulkan Video
    - macOS/iOS/tvOS: VideoToolbox (Apple's unified HW codec framework)
    - Raspberry Pi: V4L2 M2M (Video4Linux Memory-to-Mem, kernel-interface hardware decode)
  - GPU rendering: OpenGL / DirectX pixel shaders (YUV→RGB conversion + scaling + HDR tone mapping)
  - Audio: SDL2 / PulseAudio / ALSA / CoreAudio (unified behind SDL2)

## 3. Feature Overview
### Core Feature Modules
- **Game streaming**: streaming PC games to any Moonlight client device at up to 4K@120fps HDR. LAN (<1ms network latency) and remote (IPv6/Tailscale/VPN) both supported. Adaptive bitrate and dynamic resolution scaling
- **Remote desktop**: non-gaming control via "direct mouse" mode; system hotkey passthrough (Alt+Tab, Win+D, etc.); multitouch gesture mapping
- **Audio streaming**: 7.1 surround low-latency transport; Opus achieves <20ms end-to-end audio; AAC for legacy clients
- **Multitouch**: up to 10 synchronized touch points, enabling touch-driven Windows apps from phones/tablets
- **The full pad ecosystem**: up to 16 pads at once, supporting Xbox 360/Xbox One/Xbox Series / DualShock 4 / DualSense / Switch Pro Controller; independent dual-motor force feedback; DS/DSE motion passthrough (6-axis IMU: 3-axis gyro + 3-axis accelerometer)

### Distinctive Features
- **Extreme decoupling of the six-protocol separation**: GameStream implements pairing (HTTPS), stream control (RTSP-SDP), video transport (RTP), audio transport (RTP), and input control (ENet) over five independent standard protocols. Control plane all TCP (reliability), data plane all UDP (latency). This Separation of Concerns is a textbook protocol design
- **The unified abstraction over seven encoding APIs**: Sunshine's core innovation bridges NVIDIA/AMD/Intel/Qualcomm/Apple encode APIs (NVENC/AMF/QSV/VAAPI/VideoToolbox/Media Foundation/Vulkan Video) under an FFmpeg-style factory. A unique practice among open-source projects — other projects (like RustDesk) integrate GPU encoders one by one; Sunshine provides one unified adapter layer
- **Unmatched client matrix**: Moonlight covers Windows, macOS, Linux, iOS, Android, tvOS, Nintendo Switch (homebrew), PS Vita (homebrew), Steam Link, web browsers, Raspberry Pi — nearly every screen-bearing device. No commercial product can match this reach (each platform needs native development); only distributed open-source effort achieves it
- **HDR and YUV 4:4:4**: Sunshine+Moonlight are among the very few remote-desktop/game-streaming stacks supporting both HDR (HDR10 + HLG) and full 4:4:4 chroma. 4:4:4 is critical for text sharpness and color accuracy; among commercial options only Parsec Warp supports it — behind a paywall
- **Latency-breakdown visualization — the performance overlay**: the client shows the four components of end-to-end latency live: Host encode / network transit / client decode / client render. The transparent breakdown lets users pinpoint the bottleneck — is the GPU encoder slow? WiFi interference? An underpowered decoder?
- **Resilience born from crisis**: when NVIDIA killed GameStream support in 2023, the Moonlight community faced an existential crisis. Within three years Sunshine grew from zero into a replacement stronger than NVIDIA GameStream itself (more GPU vendors, more codecs, more platforms). A canonical case study for "never bind to a single vendor"

### Extensibility / Plugin Mechanisms
- **The community-fork ecosystem**: multiple maintained forks — Moonlight iOS (on the App Store), Moonlight tvOS, Moonlight Switch (homebrew), Moonlight PS Vita (homebrew), Moonlight Steam Link — proof that protocol openness yields ecosystem growth
- **Custom encoder extension**: Sunshine's FFmpeg-style factory allows adding encode backends as modules; a new GPU encoder just implements the unified interface
- **Web-UI remote configuration**: an embedded web server lets any LAN device's browser manage the server settings
- **Full CLI configuration**: every setting is reachable via CLI flags or config files — headless servers and automation supported
- **Docker containers**: the Linux edition supports Docker deployment, though GPU devices must be mapped (`--device /dev/dri`) along with network ports
- **Pre/post-command hooks**: run custom scripts before a stream starts / after it ends (e.g. auto-switching monitor resolution/refresh to match stream parameters)

- **Auto-resolution switch script hooks**: run a "do" script before streaming (e.g. switch to the best resolution/refresh for the stream) and an "undo" script after (restore the original desktop mode). PowerShell (Windows) and Bash (Linux)
- **QoS packet marking**: Sunshine supports DSCP (Differentiated Services Code Point) marking, setting IP precedence per protocol flow. Video marked EF (low latency), audio AF41 (high reliability), input AF21 (low loss). Enterprise switches can enforce QoS off the DSCP marks
- **NVIDIA NVML integration**: Sunshine reads GPU encoder utilization, temperature, and power via NVML. Live encoder load in the Web UI aids bottleneck diagnosis

### Key Sunshine Version Milestones
| Version | Date | Key updates |
|---------|------|-------------|
| v0.1 | 2020-01 | the first PoC: NVIDIA NVENC H.264 only + DXGI capture; no Linux |
| v0.10 | 2021 | AMD AMF and Intel QSV encoding; the first experimental Linux build (VAAPI+KMS/DRM) |
| v0.15 | 2022 | HEVC (H.265) encoding, early HDR10. PipeWire audio capture introduced |
| v0.20 | 2023-03 | NVIDIA ends GameStream → Sunshine becomes Moonlight's primary server. Experimental AV1. The Web UI rewritten |
| v0.21 | 2023-07 | YUV 4:4:4 chroma support. macOS 14.2 ScreenCaptureKit. The Vulkan Video encode framework |
| v0.22-0.23 | 2024 | FreeBSD support. XDG Desktop Portal + KWin Screencast (all three Wayland capture protocols covered) |
| v2025.x | 2025 | version numbers switched to year format. AV1 stable. DSCP QoS marking. 10-bit HDR |
| v2026.516 | 2026-05 | the 46th release. The Wayland DMA-BUF zero-copy path optimized. PipeWire low-latency audio mode (<10ms) |

### Moonlight Client Matrix
| Client | Latest | Stars | Platform | Notes |
|--------|--------|-------|----------|-------|
| Moonlight PC (Qt) | v6.1.0 | 17,900+ | Win/Mac/Linux | the unified desktop client, 48 releases |
| Moonlight Android | v11.0 | 8,000+ | Android | on Google Play; MediaCodec hardware decode |
| Moonlight iOS | v2.x | separate repo | iOS/tvOS | on the App Store; VideoToolbox decode |
| Moonlight Switch | community | - | Nintendo Switch | homebrew (Atmosphere) |
| Moonlight PS Vita | community | - | PS Vita | homebrew (HENkaku) |
| Moonlight Embedded | community | - | Raspberry Pi | V4L2 M2M hardware decode |
| Moonlight Web | experimental | - | browser | WebAssembly + WebCodecs API |

### The NVIDIA GameStream Shutdown Timeline (2023 crisis retrospective)
| Date | Event |
|------|-------|
| 2022-12 | NVIDIA announces removing GameStream from the NVIDIA Games App |
| 2023-02 | GameStream removed from GeForce Experience |
| 2023-03 | the Moonlight community formally announces: all users should migrate to Sunshine |
| 2023-03-12 | Sunshine v0.20 released, marking it the official recommended server |
| 2023-04 | Sunshine users up 500%; Discord membership jumps from 2k to 15k |
| 2023-07 | Sunshine v0.21: YUV 4:4:4 support — features surpass the original NVIDIA GameStream |
| 2024-2026 | Sunshine keeps evolving independently, exceeding original GameStream on every feature axis |

### Performance Data (community testing, not official)
| Scenario | Resolution | FPS | Encoder | Encode latency | LAN latency | Decode latency | End-to-end |
|----------|------------|-----|---------|----------------|-------------|----------------|------------|
| Office desktop | 1920x1080 | 60 | NVENC H.264 | 3ms | <1ms | 2ms | ~6ms |
| AAA game (4K) | 3840x2160 | 60 | NVENC HEVC | 8ms | <1ms | 5ms | ~14ms |
| FPS game (high rate) | 1920x1080 | 120 | NVENC H.264 | 2ms | <1ms | 2ms | ~5ms |
| 3D rendering | 2560x1440 | 60 | AMF HEVC | 12ms | <1ms | 4ms | ~17ms |
| Video editing (HDR) | 3840x2160 | 60 | QSV HEVC HDR | 10ms | <1ms | 6ms | ~17ms |
| AV1 gaming (new GPUs) | 2560x1440 | 120 | NVENC AV1 | 5ms | <1ms | 4ms | ~10ms |

## 4. Status & Ecosystem
- **Current versions**:
  - Sunshine: v2026.516.143833 (2026-05-16; 46 releases)
  - Moonlight PC (Qt): v6.1.0 (2024-09-17; 48 releases)
  - Moonlight Android: v11.0 (2025, on Google Play)
  - Moonlight iOS/tvOS: v2.x (on the App Store)
- **GitHub stars / activity**:
  - Sunshine: 39,300+ stars, continuously active commits, prompt community responses
  - Moonlight PC: 17,900+ stars, active major releases
  - Moonlight Android: 8,000+ stars
- **Community size**:
  - Moonlight and Sunshine each run independent Discords (thousands online)
  - Reddit r/MoonlightStreaming (active) + r/cloudygamer (general streaming)
  - multiple LizardByte sub-project communities cross-link
- **Docs / SDK / API ecosystem**:
  - official docs covering install/config, troubleshooting, codec comparisons, hardware recommendations
  - no formal SDK, but the GameStream protocol rests on published reverse-engineering documentation
  - the community contributes many multilingual tutorials, YouTube setup guides, Docker templates
- **Known defects or limitations**:
  1. **The lesson of depending on a proprietary protocol**: Moonlight originally relied on NVIDIA's proprietary GameStream; one announcement (2023's end-of-support) nearly destroyed the community. Sunshine was the rescue, but it took three years to reach a level better than the original GameStream
  2. **RTSP's fit for remote desktop**: RTSP/SDP was designed for streaming media (like RTMP push); its session management is far less flexible than WebRTC SDP negotiation. Fixed message formats constrain dynamic negotiation — seamless mid-stream encoder switching, for instance, is impossible
  3. **The many-ports networking problem**: six ports (47984, 47989, 47998-48010) are blocked almost everywhere in corporate firewalls and public WiFi. Remote access only works over VPN or Tailscale
  4. **Connection difficulty on WAN**: designed for LAN streaming (low-latency wired / 5GHz WiFi); remote access needs an added VPN layer (Tailscale/WireGuard/ZeroTier), raising configuration complexity
  5. **Non-gaming input latency**: in 4K desktop scenarios the direct-mouse mode has input-queue latency (a different input path from the game-mode cursor capture)
  6. **The legal gray zone of reverse engineering**: the original GameStream protocol came from reverse-engineering NVIDIA's proprietary protocol, always carrying legal risk. Sunshine resolved it via full reimplementation, but the historical baggage remains

  7. **The macOS Host ScreenCaptureKit limit**: ScreenCaptureKit on macOS 14.2+ is a user-space API that cannot capture the GPU framebuffer directly the way DXGI does on Windows. All frames stage through the CPU, adding 8-12ms — a fundamental platform limitation
  8. **HDR tone mapping on non-HDR clients**: when a Host outputs HDR and the client's display is SDR, Moonlight tone-maps HDR→SDR client-side via GPU pixel shaders, adding 2-5ms of render latency. Mapping algorithms vary across HDR standards, so consistency of quality is hard to guarantee
  9. **Cross-platform audio latency variance**: Windows WASAPI low-latency mode (<10ms) differs greatly from Linux PulseAudio (>30ms) and PipeWire (<15ms)
  10. **Multi-pad Bluetooth interference**: with 4+ pads on Bluetooth simultaneously, TDMA on the radio causes input-jitter (random 5-30ms swings). Wired USB pads are the recommended path for 16-player use

## 5. Market Positioning
- **Primary use cases**:
  - individual gamers (streaming the desktop rig to living room / bedroom / handhelds)
  - home media streaming (the Steam Deck + Steam Link + Apple TV ecosystem)
  - hobbyists (DIY cloud gaming, home servers, Raspberry Pi experiments)
  - some remote-work scenarios (developers/designers with extreme latency demands)
  - education and research (studying game-streaming protocol implementations; computer graphics and networking course material)
- **Competitor quick comparison**:

| Competitor | Strengths | Weaknesses |
|------------|-----------|------------|
| Parsec | 7ms lower latency (GPU zero-copy), better WAN (97% NAT traversal), tuned BUD protocol | Windows-only Host, hardware encoding only, closed source, 4:4:4 and dual screens behind Warp |
| Steam Remote Play | seamless in Steam, best pad support (Valve native), auto LAN discovery | Steam DRM lock-in, fiddly for non-Steam games, only the client is open |
| RustDesk | full remote desktop (file/terminal/port forwarding), self-hosted, open source | latency not streaming-grade, TCP-leaning strategy |
| RDP | OS-level integration, GPU-accelerated render, enterprise security (NLA/certs) | high latency (50-100ms on LAN), not built for games, console/WDDM session conflicts |

- **Pricing / licensing**:
  - Moonlight: GPLv3 — fully free, no commercial edition, no in-app purchases
  - Sunshine: GPLv3 — fully free, no commercial edition, no ads
  - neither has subscriptions, paid features, or data collection

## 6. Product Highlights
1. **The six-protocol separation paradigm**: GameStream's pairing (HTTPS), stream control (RTSP-SDP), video (RTP), audio (RTP), and input (ENet) are five independent protocols achieving thorough control-plane (TCP) vs data-plane (UDP) separation. This avoids single-protocol complexity bloat; each sub-protocol is an industry standard (RTSP/RTP/ENet are all published RFCs or specs), evolving independently. Direct reference value for MediaServo's protocol modularization (separating signaling / media / control channels)
2. **All-GPU-vendor freedom from lock-in**: Sunshine is the only open-source stack bridging NVENC/AMF/QSV/VAAPI/VideoToolbox/Media Foundation/Vulkan Video — seven encode backends — behind one FFmpeg-style abstraction. It binds to no GPU vendor and depends on no proprietary driver interface (NvFBC is an optional enhancement, not a requirement). This "no lock-in" philosophy was purchased with Moonlight's painful GameStream history
3. **Resilience born from crisis**: in 2023 NVIDIA ended GameStream, facing Moonlight with a protocol black hole. In three years Sunshine grew from a patchwork project into a server stronger than the original (more GPU vendors, codecs, and OSes). The best case study for "don't bind to a single vendor" — and the direct reason MediaServo insists on all-GPU compatibility and full protocol openness (RFC standards) from the start
4. **An unmatchable client coverage matrix**: from desktop to mobile, from consoles (Switch/Vita) to set-tops (Apple TV/Steam Link), from web to embedded (Raspberry Pi) — Moonlight's reach proves a principle: **when the protocol is an open standard, the community can achieve cross-platform coverage no commercial product can match.** Strong evidence supporting MediaServo's choice of WebRTC standard protocols
5. **Forward-looking AV1 investment**: Sunshine's AV1 support (the new-generation royalty-free codec, ~30% more efficient than HEVC) shows technical foresight. AV1 hardware encode on RTX 4000-series+ and Intel Arc offers a clear future path through the H.264/H.265 patent thicket

## 7. Reference Value for MediaServo

### [Adopt] Directly Borrowable
- **Protocol separation of control and data planes**: signaling over TCP (reliability), media and input over UDP (latency), each flow on its own channel. MediaServo's ProtocolBroker should inherit this principle
- **The unified abstraction layer over multi-vendor GPU encode APIs**: Sunshine's FFmpeg-style factory (each backend implementing one interface) maps directly onto MediaServo's HardwareEncoder plugin trait design
- **Codec capability negotiation**: the client declares its decode matrix (codecs/resolution/chroma/HDR), the server picks the best encode combination. This is the core idea of WebRTC SDP negotiation
- **Latency-breakdown visualization**: a development-time diagnostic showing encode/network/decode/render contributions separately, enabling precise bottleneck identification
- **Industry-standard protocols (RTP/RTSP/ENet) over in-house ones**: every sub-protocol is a published RFC or spec, independently implementable by third parties

### [Adapt] Needs Modification Before Adoption
- **GameStream's fixed six ports → MediaServo multiplexing onto 1-2 ports**:
  - signaling + pairing + stream control merged into one WebSocket (SDP/ICE negotiation multiplexed)
  - video RTP + audio RTP become WebRTC media tracks (multiplexed on one port)
  - input ENet becomes a WebRTC RTCDataChannel (unordered mode + maxRetransmits=0 = a low-latency quasi-reliable channel)
- **RTSP + fixed SDP message format → WebRTC SDP negotiation**: more flexible, extensible, and able to carry dynamic parameters (simulcast/SVC, etc.)
- **LAN-only (no NAT traversal) → the ICE/STUN/TURN standard family**:
  - MediaServo must support WAN (remote work, vehicle→cloud)
  - the WebRTC ICE framework natively supports STUN/TURN/relay fallback
- **HTTPS pairing + certificate auth → SRP/PAKE + OIDC**: broader auth options, supporting enterprise SSO and self-hosted setups
- **Hardcoded port numbers → IANA registration or dynamic ports**: avoid firewall conflicts

### [Avoid] Known Pitfalls / Not Applicable
- **Depending on a single vendor's proprietary protocol**: Moonlight's lesson is written in blood. MediaServo's core protocol must be standardized, documented, and third-party-implementable from day one — so a vendor's withdrawal doesn't kill the community
- **RTSP's limits for remote desktop**: RTSP is a push model; remote desktop needs frequent dynamic renegotiation (resolution changes, encoder switches, joins/leaves). WebRTC SDP fits better
- **The legal and sustainability risk of reverse engineering**: MediaServo's protocol must be publicly documented from Phase 0 — no "copy first, document later" gray zone
- **The LAN-only connectivity constraint**: MediaServo needs the WAN story (ICE punching + TURN relay + signaling) designed in from Phase 0; "ship LAN first, VPN for WAN" like Sunshine is not acceptable
- **Six ports die in corporate firewalls**: MediaServo should multiplex all traffic on a single port (or 1-2)
- **The GPLv3 boundary**: Sunshine/Moonlight code cannot merge into MediaServo's Apache 2.0 project. Architecture ideas can be borrowed; implementation must be independent

**Overall rating**: ★★★★☆ (4/5)
**Comment**: the best reference for six-protocol separation and all-GPU-vendor unified abstraction — important inspiration for MediaServo's protocol modularization and encoder-plugin system. But the proprietary-protocol history (NVIDIA ending GameStream) and the LAN-only design are traps MediaServo must avoid. Protocol openness, full-platform compatibility, and WAN support are principles to hold from Phase 0.

### GameStream-to-WebRTC Migration Map
If MediaServo uses the WebRTC standard stack in place of GameStream's six protocols:
| GameStream protocol | WebRTC equivalent | Difference |
|---------------------|-------------------|------------|
| HTTP (service discovery) | ICE candidate exchange | WebRTC exchanges candidates via signaling — no separate HTTP discovery |
| HTTPS (pairing auth) | DTLS handshake (embedded in WebRTC) | WebRTC DTLS gives E2E encryption + certificate auth |
| RTSP (stream control) | SDP offer/answer | WebRTC SDP is more flexible and supports simulcast/SVC |
| RTP video | RTP video (same port) | WebRTC ships RTP/RTCP + bandwidth estimation + congestion control |
| RTP audio | RTP audio (same port) | WebRTC multiplexes audio+video on one port |
| ENet (input) | RTCDataChannel (unordered) | over SCTP/DTLS; maxRetransmits=0 matches ENet low-latency |

WebRTC's advantages:
- port collapse: six ports → 1-2, surviving enterprise firewalls
- standardization: every component is an RFC; no reverse-engineered protocol dependency
- rich ecosystem: mature implementations in libwebrtc / webrtc-rs / pion (Go)
- NAT traversal: ICE/STUN/TURN are native WebRTC capabilities

The cost:
- SDP complexity: harder than RTSP's fixed formats; requires full SDP parsing/generation
- uncertain bandwidth estimation: WebRTC GCC targets conferencing and may be suboptimal for high-bandwidth low-latency desktop streams
- dependency tree: libwebrtc carries 200+ deps (the reason Parsec/AnyDesk refuse it) though webrtc-rs is lighter (~30 deps)
**Related decisions**: D47 (rendering), D-SIMULCAST, C5 (&[u8] boundary)

## Appendix: Sunshine's Encoder Factory Pattern
Sunshine's FFmpeg-style encoder factory is the direct reference for MediaServo's HardwareEncoder plugin:
```cpp
// Sunshine encoder factory pattern (conceptual abstraction, not exact source)
class EncoderFactory {
public:
  virtual std::unique_ptr<VideoEncoder> create(
    const EncoderConfig& config) = 0;
  virtual bool supports_codec(Codec codec) = 0;
  virtual std::vector<Codec> supported_codecs() = 0;
};

// Concrete implementation example
class NVENCEncoderFactory : public EncoderFactory {
public:
  std::unique_ptr<VideoEncoder> create(const EncoderConfig& c) override {
    return std::make_unique<NVENCEncoder>(c.width, c.height, c.fps);
  }
  bool supports_codec(Codec c) override {
    return c == Codec::H264 || c == Codec::HEVC || c == Codec::AV1;
  }
};
```

The Rust equivalent:
```rust
#[async_trait]
pub trait HardwareEncoder: Send + Sync {
    async fn encode(&mut self, frame: &VideoFrame) -> Result<EncodedPacket>;
    fn supported_codecs(&self) -> Vec<Codec>;
    fn capabilities(&self) -> EncoderCapabilities;
    async fn reconfigure(&mut self, config: EncoderConfig) -> Result<()>;
}

struct EncoderCapabilities {
    max_resolution: (u32, u32),
    max_fps: u32,
    supports_hdr: bool,
    supports_444: bool,  // YUV 4:4:4
    codecs: Vec<Codec>,
}
```
Key design points:
- traits, not abstract classes: Rust's trait system supports polymorphism and dynamic dispatch natively
- an async encode interface: encoding may involve GPU synchronization and should be async
- reconfigure(): switching parameters mid-stream (the analogue of WebRTC renegotiation)
- capabilities(): lets signaling know each encoder's capability matrix

## Appendix: Protocol Openness from the GameStream Crisis
NVIDIA ending GameStream is the strongest case for MediaServo's protocol philosophy:
| If Moonlight had... | Outcome | MediaServo's lesson |
|---------------------|---------|---------------------|
| kept depending on NVIDIA GameStream | the community would have died in 2023 | never depend on a single vendor's proprietary protocol |
| only open-sourced the Android client | iOS/tvOS/Switch/Vita users abandoned | protocol openness matters more than any client implementation |
| protocol left as reverse engineering (hard to reimplement) | Sunshine needed three years to catch up | document the core protocol publicly from Phase 0 |
| had no Sunshine community | the whole GameStream ecosystem dies | community is the ultimate guarantor of a protocol's vitality |

The single most important lesson: **protocol openness (RFC standards + public docs + third-party implementability) is not nice-to-have — it is a survival precondition.** If MediaServo depends on any proprietary technology, it risks being the next GameStream — abandoned by its vendor, community dead, code legacy. Choosing the WebRTC standard stack is not a technical preference; it is a strategic decision for architectural survival.

## Appendix: Implications of Moonlight's Client Coverage
Moonlight's 11-platform reach comes from:
1. GameStream built on standardized sub-protocols — each has open implementations
2. distributed community contribution — each platform maintained by different developers

For MediaServo: WebRTC is even more standardized (every component an RFC), so theoretically client coverage can exceed Moonlight's. The real challenge is per-platform adaptation of screen capture and input injection.

## Appendix: Sunshine's Architectural Resilience
Sunshine grew within three years into a server surpassing NVIDIA GameStream; its resilience comes from:

1. the encoder factory pattern: new GPU encoders just implement one interface
2. multi-backend screen capture: new platforms supported by adding capture modules
3. GitHub-community driven: the multi-subproject structure lets contributors own their modules independently
4. flexible versioning evolution: from v0.1 to v2026.516, adapting to release-cadence changes

These map directly onto MediaServo:
- the HardwareEncoder plugin trait = the encoder factory pattern
- the ScreenCapture plugin trait = the multi-backend capture design
- the crates workspace = modular multi-subproject structure

## Appendix: The Outlook for AV1 in Remote Desktop
Sunshine's AV1 support is a codec-evolution foresight move:

AV1's strengths: royalty-free, ~30% above HEVC compression, 4:4:4 and HDR support, hardware encode on new GPUs
AV1's limits: hardware encode only on new GPUs; software encoding 2-3x slower than VP9; higher client decode requirements than H.264/HEVC

MediaServo's AV1 policy: VP8/VP9 + H.264/H.265 in Phase 1; enable AV1 hardware encode in Phase 2+ once RTX 4000+/Arc saturate; let codec-capability negotiation pick the best encoder automatically.

### Moonlight+Sunshine Technical Debt
1. Dependence on a single vendor's proprietary protocol (fixed by Sunshine, but the lesson is permanent)
2. RTSP's limited session management for remote desktop
3. six ports blocked almost everywhere in enterprise firewalls
4. LAN-only design: WAN needs VPN/Tailscale
5. the reverse-engineering legal gray zone (resolved by Sunshine's reimplementation)
6. the macOS Host ScreenCaptureKit limit (CPU staging, +8-12ms)
7. HDR-to-SDR tone-mapping complexity
8. multi-pad Bluetooth interference (USB wired recommended)

### Summary: Moonlight+Sunshine's Core Value to MediaServo
They bequeath three design legacies crucial to MediaServo:
1. the six-protocol separation architecture — proof that strict control/data-plane separation is the right paradigm for high-performance remote transport
2. all-GPU-vendor compatibility — proof that not binding to a single vendor is not just technically right but an ecosystem-survival necessity
3. rebirth from crisis — proof that protocol openness (RFC standards + public docs) is the sole guarantor of long-term ecosystem vitality

The most important lesson MediaServo takes from Moonlight+Sunshine: NVIDIA ending GameStream was no accident but the inevitable outcome of vendor-controlled protocols. MediaServo must build on RFC standards from Phase 0, with public documentation as the spec and community ecosystem as the goal — not a technical preference, but a strategic decision for architectural survival.

## Appendix: Moonlight+Sunshine's Influence on MediaServo's Feature Priorities
From their protocol separation and encoder abstraction experience, MediaServo's architecture plan:

Phase 0: WebRTC stack selection (replacing GameStream's six protocols), the encoder plugin trait definition (from the factory pattern)
Phase 1: control/data-plane separation implemented, VP8/VP9 + H.264/H.265 multi-encoder support, codec-capability negotiation
Phase 2: AV1 hardware encoding, full HDR/4:4:4 chroma, the performance overlay latency-breakdown
Phase 3: the unified abstraction across all GPU vendors' encode APIs, runtime hot-switching of encoders, DSCP QoS marking
Phase 4: completing the web client, more non-mainstream platform coverage (embedded/consoles)
