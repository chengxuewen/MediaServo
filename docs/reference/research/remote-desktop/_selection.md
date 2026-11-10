# Remote Desktop Product Research Report

> Research date: 2026-07-16 | Perspective: architect | Goal: provide references for the MediaServo architecture design

---

## Table of Contents

- [I. Commercial Giants](#i-commercial-giants)
  - [1.1 TeamViewer](#11-teamviewer)
  - [1.2 AnyDesk](#12-anydesk)
  - [1.3 Splashtop](#13-splashtop)
- [II. Open-Source Rising Stars](#ii-open-source-rising-stars)
  - [2.1 RustDesk](#21-rustdesk)
  - [2.2 Moonlight + Sunshine](#22-moonlight--sunshine)
- [III. Performance Pioneers](#iii-performance-pioneers)
  - [3.1 Parsec](#31-parsec)
  - [3.2 NoMachine](#32-nomachine)
- [IV. China-Market Solutions](#iv-china-market-solutions)
  - [4.1 Sunlogin](#41-sunlogin)
  - [4.2 ToDesk](#42-todesk)
- [V. Platform Built-In Solutions](#v-platform-built-in-solutions)
  - [5.1 Microsoft RDP](#51-microsoft-rdp)
  - [5.2 Chrome Remote Desktop](#52-chrome-remote-desktop)
- [VI. Synthesis](#vi-synthesis)

---

## I. Commercial Giants

### 1.1 TeamViewer

**Overview**: Founded in Germany in 2005, the definer of the global remote desktop market. More than 300 million monthly active devices; the business model is subscription + enterprise licensing. In 2023 the 2016 Russian APT organization attack was revealed — the attackers accessed TeamViewer's internal systems through leaked credentials, but user data was not affected. This incident exposed the security weakness of centralized architectures: even when the infrastructure is secure, credential management remains the weak link.

**Architecture pattern**: the classic centralized signaling + P2P hybrid architecture. The system consists of three classes of servers: Master server (identity authentication, session management), KeepAlive server (online status maintenance, router probing), Router server (data relay). About 70% of connections achieve P2P direct links through UDP hole punching, with the rest falling back to TCP relay or HTTP tunneling. Port strategy: primarily TCP/UDP 5938, falling back to 443 and 80. Connection process: client → authentication against the Master server → learn the target's online status → negotiate connection parameters through the KeepAlive server → attempt P2P hole punching → on success connect directly, on failure go through the Router relay.

**Tech stack**: proprietary in-house protocol (not publicly documented), clients built with C++/Qt, RSA-4096 public key exchange + AES-256 session encryption, TLS 1.3 (v15.73+). Authentication uses the SRP (Secure Remote Password) protocol — the password is never transmitted, only a password-verification key negotiation is performed. The full cloud infrastructure is deployed in ISO 27001 data centers (Germany, Austria, Netherlands). Key learning: **the use of the SRP protocol is a pillar of TeamViewer's security architecture**, ensuring that even if the server is breached, an attacker cannot obtain user passwords or decrypt sessions.

**Known lessons**:
1. **2016 security incident**: a Russian APT organization used leaked account credentials to access a large number of TeamViewer user devices. Lesson: once a centralized identity system suffers a credential leak, the blast radius is enormous. MediaServo should support multi-factor authentication and hardware key binding.
2. **Single point of failure risk of the centralized architecture**: TeamViewer depends heavily on its cloud infrastructure; once a server goes down or is attacked, users worldwide cannot establish new connections. MediaServo should consider supporting self-hosted or federated signaling services.
3. **The cost of protocol closedness**: an undisclosed protocol means security audits depend on the vendor's self-disclosure, and third parties cannot verify its security claims. The encryption model described in TeamViewer's 2017 security white paper was updated to TLS 1.3 in 2024, showing that protocols require continuous modernization, though this is invisible to users.

**Adoptable Designs for MediaServo**:
- SRP or a similar PAKE protocol for password authentication, avoiding password transmission
- Layered connection strategy of P2P direct connection first + relay fallback
- TLS 1.3 + PFS session encryption model
- Multi-port fallback (UDP → TCP → HTTP tunnel) firewall traversal strategy

---

### 1.2 AnyDesk

**Overview**: Founded in Germany in 2014 by a team of former TeamViewer members, known for extreme lightness and low latency. The client is only 3.7MB; the core differentiator comes from the proprietary DeskRT video codec — a codec designed specifically for computer graphics transmission, still able to stream smoothly at 100kbps bandwidth. Acquired by a German company in 2024, but the product's technology stack was preserved.

**Architecture pattern**: multi-process separation. The GUI process runs in the user session, while network I/O runs in an independent SYSTEM-level service process (a different PID); the two communicate through named shared memory objects (`CaptureFileMappingW`, `svc_mtx`, `ret_evt`). This separation design provides **permission isolation** — the network layer running with SYSTEM privileges can handle NAT traversal, service registration and other operations, while the GUI and screen capture run with user privileges, preventing the network layer from directly accessing desktop content. Multi-layer connection fallback: relay on 443 → relay on 80 → SOCKS proxy on 443 → P2P on 443/80/6568.

**Tech stack**: Erlang backend (telecom-grade reliability, supporting 10k+ concurrent devices), C++/Qt clients, a proprietary AnyNet protocol serialization engine (570+ boundary checks), the DeskRT video codec. TLS 1.3 + 256-bit AES + RSA 4096/ECC 256 key exchange. Protocol reverse analysis shows that AnyDesk's serialization engine is rigorously designed — it performs boundary checks on every incoming protocol datum, making it one of the few remote desktop products that invests this deeply in protocol parser security.

**Key features**:
- DeskRT codec: optimized specifically for GUI content rather than a general video codec, latency <16ms at 60fps
- Extremely light client: 3.7MB in size, usable straight from install
- Erlang server side: hot updates, high concurrency, fault tolerance
- IPC uses shared memory rather than network sockets, giving zero-overhead inter-process communication

**Known lessons**:
1. **Security weakness in the update mechanism**: reverse analysis found that AnyDesk downloads updates through the relay protocol every 9 hours but **does not verify software signatures** — it relies only on TLS transport security. This means that if the relay is breached, malicious updates can be injected. MediaServo must perform independent signature verification on all update packages.
2. **Security of shared memory IPC**: using `CreateFileMappingW` with `NULL` security attributes means that any process in the same session can theoretically access the shared memory. The isolation between Session 0 (services) and Session 1+ (users) is the last security boundary.
3. **Multi-process architecture increases port complexity**: AnyDesk maintains multiple ports (6568, 7070) and multiple connection strategies, which easily triggers security audit alerts in enterprise firewall environments.

**Adoptable Designs for MediaServo**:
- Process permission separation: split the network layer / service layer / UI layer into different processes, each responsible for its own duties
- The design thinking behind a proprietary codec: optimized for GUI content rather than general video
- Erlang/OTP for signaling service reliability and hot update capability
- Shared memory IPC as a high-frequency data transfer channel

---

### 1.3 Splashtop

**Overview**: Founded in Silicon Valley in 2006, originally famous for Splashtop OS (an instant-boot operating system preinstalled on more than 100 million devices), later transforming into a remote desktop leader. More than 30 million users, featured in Gartner and Forrester reports. In February 2026 it launched an AI-optimized codec, achieving stable 4K@60fps transmission and up to 240fps ultra-high frame rate support. Business model: subscription, split across three product lines — Remote Access (personal/team), Remote Support (MSP/IT), and Enterprise.

**Architecture pattern**: classic three-layer cloud architecture. Core components: Streamer (agent on the controlled side), Business App (the controlling side), API Server, Relay Server, Web Server, Database. Infrastructure is distributed across three regions (US, EU, Oceania), each deployed independently. Relay Servers are deployed at many nodes worldwide for nearest-edge access. Connection establishment: the Streamer and the Business App each connect to the API Server over HTTPS to complete authentication, then establish an end-to-end TLS tunnel through the Relay Server, with session encryption negotiating the AES-256 key directly between endpoints. On-Premises deployment (Splashtop Center) is also supported, allowing self-hosting of relay servers inside an enterprise DMZ.

**Tech stack**: a 2013 patent reveals its hybrid encoding strategy — adaptively switching between full-frame compression (H.264) and differential residual encoding: when screen change exceeds a threshold it uses standard H.264 keyframes, and when the change is small it uses 8×8 block-based XOR residual encoding. In 2026 an AI-optimized codec was introduced, providing dynamic bitrate adaptation, content-aware ROI encoding, and smart capture technology. TLS 1.2 + AES-256 end-to-end encryption, with Active Directory integration and SSO support. Infrastructure uses AWS/GCP/OCI multi-provider redundancy.

**Key features**:
- AI-optimized codec: 4K@60fps in GPU-accelerated scenarios, supporting up to 240fps
- Multi-provider infrastructure: deployment across AWS/GCP/OCI avoids single-vendor lock-in
- On-Premises option: satisfies data sovereignty and compliance requirements
- Enterprise-grade security: MFA, SIEM export, operation logging, screen recording
- Multi-scenario product lines: remote access, remote support, enterprise management

**Known lessons**:
1. **The rise and fall of Splashtop OS**: the instant-boot OS preinstalled on 100M+ devices was displaced by Windows fast startup technology and SSD proliferation. Splashtop's timely pivot to remote desktop was key to its survival. Implication for MediaServo: **technology choices must keep pace with hardware evolution trends**.
2. **Barriers to the enterprise market**: after succeeding in the consumer market, Splashtop spent many years building enterprise-grade trust. Rigorous SOC 2, HIPAA, and ISO certifications are the stepping stone for entering the enterprise market.

**Adoptable Designs for MediaServo**:
- Hybrid encoding strategy (full frame + differential residual) balancing image quality and bandwidth
- Multi-provider infrastructure achieving geographic redundancy and vendor diversity
- On-Premises deployment option satisfying data sovereignty requirements
- AI-optimized adaptive codec as a future evolution direction

---

## II. Open-Source Rising Stars

### 2.1 RustDesk

**Overview**: An open-source remote desktop project started in 2020 (AGPLv3), with 118k+ Stars on GitHub and 400+ contributors. Positioned as an open-source alternative to TeamViewer, emphasizing data sovereignty and self-hosting. The core is developed primarily by a Chinese team. The commercial edition is RustDesk Server Pro, the free OSS edition is RustDesk Server OSS. Full platform support — Windows/macOS/Linux/iOS/Android/Web.

**Architecture pattern**: single-binary multi-process architecture — this is RustDesk's most ingenious design decision. The same binary distinguishes process roles through command-line arguments (`--server`, `--cm`, `--cm-no-ui`, or no arguments): Server (background service, handling remote connections), CM (Connection Manager, handling session UI and authorization), Main (UI process). Inter-process communication uses `parity-tokio-ipc` (named pipes on Windows, Unix Domain Sockets on Unix), with the message protocol being the Rust enum `ipc::Data`. Connection level: the client first registers with and looks up the peer through HBBS (Rendezvous Server), attempts TCP hole punching to establish a P2P connection, and falls back to HBBR (Relay Server) relay on failure. Users can self-deploy HBBS and HBBR, achieving full data autonomy.

**Tech stack**: Rust (67.6% core logic) + Dart/Flutter (24.1% cross-platform UI) + C++/C (platform adaptation). Encoding support: VP8/VP9/AV1 software codecs + H264/H265 hardware codecs (NVENC/VAAPI/MediaCodec/VideoToolbox). Encryption: NaCl (libsodium) for end-to-end encryption, with keys exchanged through the `secure_connection` module. Protobuf is used for network protocol definitions (`libs/hbb_common`). `libs/scrap` implements cross-platform screen capture (Windows DXGI, Linux X11/KMS/DRM, macOS ScreenCaptureKit).

**Key features**:
- Fully self-hostable: HBBS (signaling) + HBBR (relay) can be deployed with Docker
- Multiple connection types: remote desktop, file transfer, port forwarding, terminal access, camera viewing
- P2P encrypted direct connection: NaCl provides end-to-end encryption, the relay server cannot decrypt
- Permission separation: independent permission control across four dimensions — keyboard/clipboard/audio/file
- Single-binary deployment: no installation needed on Windows, runs portably

**Known lessons**:
1. **Limited TCP hole punching success rate**: RustDesk documentation and user feedback show that in symmetric NAT environments the TCP hole punching success rate is low, with heavy reliance on relay. This is especially pronounced in complex domestic carrier network environments. This contrasts with ToDesk's proprietary SD-WAN.
2. **Flutter UI performance trade-offs**: although Flutter achieves cross-platform consistency, it incurs extra overhead in rendering performance and GPU texture integration at 4K high resolutions (supplemented via the `vram` feature flag and `flutter_gpu_texture_renderer`).
3. **The adoption barrier of the AGPL license**: AGPLv3 requires derivative code to be open-sourced as well, which is a significant license consideration for commercial integration. The commercial Pro Server is a separate product not subject to AGPL.

**Adoptable Designs for MediaServo**:
- Single-binary multi-process architecture: an elegant implementation of process role separation
- Defining all protocols in Protobuf: type-safe, cross-language, version-evolution friendly
- The cross-platform screen capture abstraction layer design of `libs/scrap`
- Self-hosted signaling + relay infrastructure model
- A demonstration of Rust's network security and memory safety

---

### 2.2 Moonlight + Sunshine

**Overview**: Moonlight (formerly Limelight) is an open-source GameStream client (GPLv3), implemented by reverse engineering NVIDIA Shield's proprietary protocol. Sunshine is a community-developed open-source GameStream-compatible server (C++) supporting AMD/Intel/NVIDIA GPUs. After NVIDIA announced the end of the GameStream service in 2023, the community turned fully to Sunshine. It is now the de facto standard for open-source game streaming.

**Architecture pattern**: six-protocol separation architecture. The GameStream protocol consists of six independent sub-protocols:
1. HTTP (TCP 47989): service discovery and status queries
2. HTTPS (TCP 47984): pairing and authentication
3. RTSP (TCP 48010): stream session control (SDP negotiation)
4. ENet (UDP 47999): reliable control channel (gamepads, keyboard and mouse input)
5. RTP video (UDP 47998): H.264/HEVC/AV1 video stream
6. RTP audio (UDP 48000): 7.1 surround audio stream

The protocol design reflects **strict separation of the control plane and the data plane**: RTSP is used for stream control negotiation, RTP for media data transfer, and ENet for low-latency input control. The three use different transport layer protocols (TCP control vs UDP data) and do not interfere with each other.

**Tech stack**: the Sunshine server is C++, using Qt (Web UI configuration) + FFmpeg-like APIs to abstract multi-vendor GPU encoding. The Moonlight client supports multiple platforms (Qt/C++ desktop, native Android/iOS). The encoding API matrix shows the depth of its cross-GPU compatibility: NVENC (NVIDIA), AMF (AMD), QuickSync (Intel), VAAPI (Linux), VideoToolbox (macOS), Vulkan Video, Media Foundation (Windows), and software encoding. Codec support: H.264, HEVC, AV1, YUV 4:4:4 chroma subsampling, and HDR. Screen capture method matrix: DXGI Desktop Duplication (Windows), KMS/DRM (Linux), ScreenCaptureKit (macOS), NvFBC (X11 NVIDIA), Wayland wlroots/XDG Desktop Portal/KWin Screencast (Linux Wayland).

**Key features**:
- Six-protocol separation design: independent channels for control/video/audio/input
- Full GPU support: not locked into any vendor, covering all mainstream encoding APIs
- AV1 encoding support: higher compression efficiency than H.265
- Game peripheral emulation: full support for DualShock 4/DualSense/Switch Pro/Xbox
- Performance overlay: real-time breakdown of encoding latency and network latency

**Known lessons**:
1. **Fragility of depending on a proprietary protocol**: Moonlight's history proves that depending on a single vendor's proprietary protocol is fatal — NVIDIA can terminate an entire ecosystem with a single announcement. This is also why the community invested heavily in Sunshine. MediaServo should ensure its core protocols can be implemented independently and should not depend on any vendor-exclusive technology.
2. **Sustainability of reverse engineering as a technology source**: Moonlight is based on reverse engineering the NVIDIA protocol and always faces legal risk and compatibility uncertainty. Sunshine's choice to reimplement the server is the more sustainable path.
3. **Applicability limits of RTSP in remote desktop**: RTSP is a protocol designed for streaming, with limited session management capability in remote desktop scenarios. Compared with WebRTC's flexible SDP negotiation, RTSP's fixed message format limits dynamic negotiation.

**Adoptable Designs for MediaServo**:
- Protocol separation of control plane and data plane
- A unified abstraction layer design for multi-GPU-vendor encoding APIs
- Codec capability negotiation mechanism (the client declares decoding capability, the server selects the optimal encoding)
- Endpoint latency breakdown in performance monitoring (visualizing the encoding/network/decoding stages)

---

## III. Performance Pioneers

### 3.1 Parsec

**Overview**: Founded in 2016, focused on ultra-low-latency game streaming and remote creation. Patented in 2018 (US10951890). Acquired by Unity in 2021 and later spun off. Positioned as "giving your friend a second controller on your computer" — with P2P game streaming as the core scenario. Core technical metrics: in a LAN environment it adds **only 7ms of latency** (end to end), with a 97% NAT traversal success rate.

**Architecture pattern**: an extreme zero-copy GPU pipeline. The core idea: **never let video frames leave GPU memory**. The full chain is designed as follows:
1. The Windows Desktop Duplication API captures the GPU frame buffer directly
2. Raw frames are passed directly to the hardware encoder (NVENC/AMF/QSV), bypassing the CPU
3. H.264 encoded packets are transported via the proprietary BUD protocol (UDP-based, DTLS 1.2 encrypted)
4. After client-side hardware decoding, a pixel shader (OpenGL/DirectX) performs color space conversion
5. Direct rendering to the back buffer, using Flip-Sequential present effects (DirectX) to synchronize with the display refresh rate

**Tech stack**: the core SDK is written in cross-platform C, rejecting any unnecessary dependency (the project explicitly opposes Google WebRTC's huge dependency tree). The proprietary BUD (Better User Datagrams) protocol — UDP-based, DTLS 1.2 encrypted (AES128/256), with a custom congestion control algorithm tightly coupled to the encoding layer (the encoder adjusts QP parameters in real time in response to network conditions). Only hardware encoding/decoding is supported (mainly H.264, with H.265 being added) — this "stubborn" decision makes performance hard-guaranteed but limits hardware compatibility.

**Key features**:
- 7ms latency: full-chain GPU zero-copy
- BUD protocol: a UDP protocol tailored for video streaming, 97% NAT traversal
- GPU-direct color space conversion: avoids CPU involvement, reducing latency
- Performance data driven: real encoding latency data collected from 250k+ sessions guides optimization

**Known lessons**:
1. **The cost of supporting hardware encoding only**: Parsec explicitly rejects software encoding, arguing performance and latency are unacceptable. But this also means older devices cannot use it, limiting the user base. MediaServo should provide a software encoding fallback as a compatibility guarantee. Their data shows NVENC encoding median latency of 5.8ms vs AMD VCE 15.06ms — the hardware gap is huge, and software encoding may go above 100ms.
2. **H.264 licensing risk**: although Parsec uses hardware encoders (licensing covered by the GPU vendor), the MPEG LA licensing pool charging for software H.264 encoding has always been a burden for open-source projects. MediaServo's software encoding choices should prioritize royalty-free codecs such as VP8/VP9/AV1.
3. **The cost of network priority ordering**: the BUD protocol's priority order is latency > frame rate > image quality. This is reasonable for gaming scenarios, but in productivity and design scenarios image quality matters just as much. MediaServo should provide a configurable priority strategy according to the use scenario.

**Adoptable Designs for MediaServo**:
- Full-chain design thinking of the GPU zero-copy pipeline
- Tight coupling of a proprietary transport protocol with the encoder — the encoder can respond in real time to network congestion signals
- Use of the Desktop Duplication API — the lowest-latency screen capture method on Windows
- UDP-based custom congestion control replacing TCP — TCP's head-of-line blocking and slow start are performance killers in remote desktop scenarios
- The methodology of optimizing performance with real user data

---

### 3.2 NoMachine

**Overview**: NX technology was introduced by the Italian company NoMachine in 2003, based on deep optimization of the X11 protocol. Its core innovation is a technology milestone often overlooked: the NX proxy system can transparently bridge RDP and RFB (VNC) protocols into the NX protocol. This means **a single NX client can uniformly access remote X11 desktops, Windows RDP sessions, and VNC services**. This is highly valuable as a reference for MediaServo's goal of unified multi-protocol access.

**Architecture pattern**: dual-proxy tunnel model. This is NX's most distinctive design:
1. Client NX Proxy: receives X11 requests from the local X Server and transports them to the remote end over the NX protocol
2. Server NX Proxy + nxagent (shadow X Server): nxagent acts as a "fake X Server" for remote X applications, forwarding X11 drawing commands through the NX protocol after receiving them, while exploiting the low-latency advantage of local Unix Domain Sockets to complete internal round trips

The two Proxies communicate using the NX protocol, which is optimized at three levels:
- **Message caching**: maintains a MessageStore cache, classifying and storing the "identity" and "data" parts of recent messages by X protocol opcode. A new message first computes an MD5 fingerprint and looks it up in the cache; on a cache hit only the reference ID is transferred
- **Differential encoding**: on a cache miss, the message is encoded field by field, transferring only the differing parts relative to the previous message of the same type
- **Value caching**: frequently occurring integer values such as Window IDs are encoded with a move-to-front algorithm, so a 32-bit ID can be compressed to 1 bit

The final data flow then passes through ZLIB compression, selecting different compression levels according to link type (MODEM → WAN → LAN).

**Tech stack**: implemented in C++, with the protocol based on an extension of the X11 wire protocol. Core components: nxproxy (compression proxy), nxagent (shadow X Server based on Xnest), nxdesktop (RDP → NX bridge, based on rdesktop), nxviewer (RFB/VNC → NX bridge, based on vncviewer). Image compression supports multiple methods (JPEG, PNG, native RDP, TIGHT, etc.), automatically selected by the proxy according to link speed.

**Key performance data**:
- Typical X message cache hit rate 60-80%, reaching 100% for graphics requests/images/fonts
- Cache persistence: at session end the MessageStore is written to disk and reloaded on the next connection — this is key to startup acceleration
- Combined compression ratios can reach 2000:1 (JPEG 20:1 × differential cache 100:1)
- Second-run startup data transfer reduced from hundreds of MB to 35KB
- Mozilla remote startup reduced from 7 minutes to 20 seconds

**Known lessons**:
1. **Deep binding to X11**: NX's performance advantage depends heavily on X11 protocol characteristics and faces challenges in the Wayland migration. The latest NX versions already support Wayland, but the optimization level is far below the X11 path. MediaServo should design a protocol-agnostic transport abstraction layer.
2. **The cost of complexity**: stacking NX's MessageStore, differential encoding, and value caching — three layers of compression — makes the code highly complex and hard to maintain. The FreeNX open-source branch demonstrated how difficult it is for an open-source community to sustain such a complex codebase.
3. **Single-threaded SSH bottleneck**: one important reason NoMachine chose a proprietary NX protocol instead of SSH is that sshd is single-threaded, while NX supports multi-threaded transfer. This illustrates the concurrent processing demands of remote desktop.

**Adoptable Designs for MediaServo**:
- Multi-protocol bridging model: one core protocol uniformly carrying X11/RDP/VNC and other remote protocols
- The extreme compression idea of message caching + differential encoding, tailored to GUI-specific content
- Cache persistence to accelerate repeat connections
- Bandwidth arbitration: interactive traffic takes priority over bulk image transfer, preventing GUI freezing
- Adaptive compression levels selected according to link type

---

## IV. China-Market Solutions

### 4.1 Sunlogin

**Overview**: A product of Oray, a pioneer of the domestic remote control market. It shares its technical infrastructure with Oray Peanut Shell (NAT traversal). Positioning extends from personal remote control to enterprise IT operations. In 2025 Sunlogin 16 introduced a proprietary SADDC codec algorithm and a GPU + Zero-copy architecture. It supports domestic operating systems (UOS, Kylin, Founder, Deepin) and is a common choice for domestic government and state-owned enterprise scenarios.

**Architecture pattern**: P2P + cloud relay hybrid architecture, equipped with 200+ global acceleration nodes. At its core is the self-developed "Sunlogin Remote Communication Protocol". Connection flow:
1. The controlled side actively establishes a long connection to the ID verification server (unidirectional from intranet to extranet, bypassing NAT limitations)
2. The verification server records the device's public IP and port mapping information, and assigns a unique device code
3. The controlling side obtains the controlled side's network coordinates through the verification server
4. The "predictive UDP hole punching" algorithm is used: both NAT types are analyzed, cone NAT is punched directly, symmetric NAT goes through port prediction + STUN assistance
5. Hole punching success rate >90%; on failure it automatically degrades to TCP relay mode

**Tech stack**: proprietary SADDC codec algorithm (patented) + GPU hardware acceleration. RSA-2048 asymmetric key exchange + AES encryption (supporting both AES-128/256 schemes). TLS encrypted transport. The Zero-copy architecture claims to compress latency down to 7ms. Dual-mode screen capture support on Windows GDI/DirectX. BGP high-speed forwarding server network.

**Key features**:
- Remote SDK embedding: can be embedded into third-party Android device applications without system signing
- Screen wall monitoring: batch viewing of real-time feeds from multiple devices
- Remote control of offline devices: access intranet-only devices through special channels
- Domestic OS adaptation: full support for domestic OSes such as UOS and Kylin
- Privacy screen: the controlled side goes black, preventing local peeping

**Adoptable Designs for MediaServo**:
- The "predictive UDP hole punching" adaptive classification strategy for NAT types
- The 200+ node BGP relay network deployment model
- The remote SDK embedding design: remote control capability can be integrated by third parties as an SDK
- Scenario support for controlling offline devices

---

### 4.2 ToDesk

**Overview**: An emerging domestic remote control brand whose core team has 10+ years of network optimization experience, having supported million-scale concurrent live streaming networks. Technical investment is centered on optimizing for local network conditions — under the "southern Telecom, northern Unicom" multi-carrier landscape, P2P direct connection success rates are far lower than overseas, so ToDesk treats network transfer optimization as its top technical investment. In 2024 it disclosed an SD-WAN network covering 200+ domestic nodes, with device-to-edge-node latency <10ms.

**Architecture pattern**: RTC (Real-Time Communication) + SD-WAN dual-layer optimization. ToDesk is the only domestic product that has deeply integrated WebRTC concepts into remote desktop, but its RTC implementation is proprietary (not Google WebRTC):

1. **Transport layer**: the RTP protocol replaces direct use of TCP/UDP. RTP supplements UDP with sequence information, payload description, and quality monitoring. The receiving end eliminates out-of-order packets based on sequence numbers and periodically feeds back transfer quality
2. **Congestion control**: adopts both Delay-based (RTT sampling + Kalman Filter to monitor delay) and Loss-based (intelligently distinguishing random loss vs congestion loss vs burst loss) dual strategies, raising bandwidth utilization by 30-50% and reducing the congestion rate by 90%
3. **Weak network countermeasures**: Jitter Buffer (Kalman Filter-based adaptive buffering, 9-20ms) + HARQ (ARQ retransmission + Reed-Solomon FEC forward redundancy), keeping the actual decoded packet loss below 3‰ even at a 30% packet loss rate
4. **SD-WAN**: three-tier networking (core-mid-edge), backbone nodes directly linked by dedicated lines, user-space DPDK forwarding based on SRv6 (5-6x faster than Linux kernel forwarding, single pass <1ms), and global path selection via the QoE algorithm

**Tech stack**: proprietary ZeroSync transport engine, RTC protocol family (RTP + congestion control + weak network countermeasures), OTT SD-WAN (Over-The-Top Software-Defined WAN), VP9-first video encoding (low-bandwidth optimized), also supporting H.265. TLS 1.2 + AES-256 end-to-end encryption, Protobuf for encapsulating input events. Dynamic port strategy (automatically switching to HTTPS encapsulation on port 443 in enterprise environments). End-to-end latency as low as 40ms.

**Known lessons**:
1. **Trust issues from non-transparent algorithms**: the core congestion control algorithm is not open source, so enterprise users cannot perform kernel-level tuning for specific intranet environments. This is an inherent limitation of closed-source commercial software.
2. **QoS limits of the free edition**: it does not limit connection duration, but throttles bandwidth through QoS policies in high-concurrency and large-file transfer scenarios — a routine cost-balancing technique in commercial software, though it creates a user perception of "hidden limits".
3. **Special challenges of domestic network conditions**: a high proportion of symmetric NAT and an immature IPv6 transition make a guaranteed pure P2P direct connection hard to achieve, and relay bandwidth costs rise exponentially with user growth. ToDesk's SD-WAN is a proactive response to this problem, but whether its cost structure is sustainable remains to be verified.

**Adoptable Designs for MediaServo**:
- The RTC protocol family application paradigm in remote desktop (RTP transport + congestion control + Jitter Buffer + FEC)
- The precision of using Kalman Filters for network state estimation
- SD-WAN three-tier networking + global path selection + DPDK forwarding
- Weak network countermeasures in complex network environments (≤30% packet loss)
- Special considerations for domestic carrier network conditions

---

## V. Platform Built-In Solutions

### 5.1 Microsoft RDP

**Overview**: Windows' native remote desktop protocol, first released in 1998 with Windows NT 4.0 Terminal Server Edition. The IP is published in the Microsoft Open Specifications ([MS-RDPBCGR], etc.), with dozens of protocol specification documents. It is the underlying transport protocol for Azure Virtual Desktop, Windows 365 Cloud PCs, and Microsoft Dev Box. It is deeply integrated with the operating system and has the broadest client support and the most mature enterprise management system.

**Architecture pattern**: a rich graphics channel model — RDP uses a "virtual channel" architecture, multiplexing multiple data types over a single TCP connection: graphics, audio, clipboard, printer redirection, USB device redirection, file system redirection, and so on. Each virtual channel is a logically independent data stream. Graphics transfer uses a layered processing pipeline:

1. Frame capture → image processor (differential detection, motion detection, cache lookup)
2. Content classifier → separates text/image/video content
3. Mixed-mode encoding:
   - Text: a custom text-optimized codec (about 80% of graphics data)
   - Images: the H.264/AVC or RemoteFX graphics codec
   - Video: full-screen H.264/AVC video encoding

**Tech stack**: the RemoteFX codec is a tile-based DWT (Discrete Wavelet Transform) + RLGR (Run-Length Golomb-Rice) entropy coding — a transform domain encoder optimized specifically for text clarity. The graphics pipeline extension ([MS-RDPEGFX]) adds H.264/AVC and H.265/HEVC hardware accelerated encoding support. GPU acceleration path: application GPU rendering → RDP GPU encoding → client GPU decoding, with YUV 4:4:4 chroma subsampling support. Both software encoding and GPU hardware encoding modes are supported.

**Key features**:
- GPU-accelerated frame encoding: the RemoteFX vGPU technology can virtualize a physical GPU to remote sessions
- AVC/H.264 full-screen video encoding: suitable for 3D modeling and video editing
- HEVC/H.265: saves 25-50% bandwidth compared with H.264
- Mixed mode: text/images/video each use the optimal codec
- Rich device redirection: printers, USB, smart cards, cameras
- Group Policy management, AD integration, NLA authentication

**Known lessons**:
1. **The historical burden of port 3389**: the RDP default port 3389 is one of the most common targets of global scanners. Although NLA (Network Level Authentication) provides security hardening, port exposure is itself a security risk.
2. **The cost of protocol complexity**: RDP's dozens of specification documents and hundreds of virtual channel extensions make it the most complex of all remote desktop protocols. Some performance optimizations in Azure Virtual Desktop require deep customization of the RDP protocol stack.
3. **Cross-platform performance differences**: although the RDP client supports macOS/Linux/iOS/Android, non-Windows clients cannot enjoy Windows-exclusive features such as RemoteFX GPU acceleration and certain device redirections.

**Adoptable Designs for MediaServo**:
- Virtual channel architecture: multiplexing multiple data types over a single transport connection
- Content classification + mixed codec strategy: text/images/video each use the most suitable encoding
- Standardized open protocol specifications (usable by third-party implementations)
- GPU-accelerated end-to-end pipeline
- Real data on bandwidth savings from HEVC/H.265 (25-50%)

---

### 5.2 Chrome Remote Desktop

**Overview**: A remote desktop solution developed by Google, built entirely on WebRTC and Chromium infrastructure. Open source (the Chromium `//remoting` directory), free to use. Two modes: Me2Me (Remote Access, a persistent system-level daemon) and It2Me (Remote Support, an ephemeral session started on demand in the browser). It is deeply bound to the Google account system, and its security model relies on Google's identity authentication infrastructure.

**Architecture pattern**: multi-process + Mojo IPC + WebRTC. Me2Me mode adopts a complex multi-process architecture:
- **Daemon process** (SYSTEM/root privileges): process lifecycle management, Mojo Broker role
- **Network process**: handles network I/O and WebRTC connections
- **Desktop process**: screen capture and input injection
- **User process**: user-session-level services

The processes communicate through Mojo interfaces, defined in `remoting/host/mojom/`:
- `remoting_host.mojom`: Network ↔ Daemon (host control and status)
- `desktop_session.mojom`: Network ↔ Desktop (screen capture, input injection, session lifecycle)
- `chromoting_host_services.mojom`: Network ↔ UserProcess

The security design is strict: the Daemon process runs as SYSTEM/root, but the Network process and Desktop process are each isolated. Connections use the ICE/STUN/TURN protocol family and interact with Google's signaling service (74.125.247.128). Access is authorized through Google account OAuth.

**Tech stack**: C++ (the Chromium codebase), WebRTC (PeerConnection, DataChannel, video/audio tracks), Mojo IPC, Protobuf. Video encoding uses WebRTC's built-in codec factory (VP8/VP9 by default, H.264 optional). The network layer supports three connection modes — Direct/STUN/TURN — through the ICE framework. ChromeOS special implementation: It2Me hosts run inside the Chrome browser process, so a crash affects the entire browser session.

**Key features**:
- Completely free, no third-party software required
- Security and reliability of Google infrastructure
- The browser itself can act as the client (Chrome extension/Web App)
- Deep ChromeOS integration
- Enterprise policy management: Chrome policies control firewall traversal, UDP port ranges, relay enablement, and so on

**Known lessons**:
1. **Browser dependency**: Chrome Remote Desktop depends entirely on the Chrome/Chromium ecosystem. It is unavailable in non-Chrome browsers, limiting platform independence.
2. **Applicability of WebRTC in remote desktop**: WebRTC was designed for video conferencing, and its single-stream bandwidth allocation strategy in remote desktop scenarios is less flexible than dedicated solutions. Comments in the Chromium code reveal several hacks: min_bitrate is forcibly set to 1Mbps to keep the bandwidth estimator responsive to peak traffic ("padding needs to be enabled to workaround b/w estimator not handling spiky traffic patterns well").
3. **Legacy issues of Plan B SDP**: Chromium code still uses the deprecated Plan B SDP semantics rather than Unified Plan, because the migration is a large amount of work. This reflects the technical debt of using WebRTC outside its designed scenario.

**Adoptable Designs for MediaServo**:
- The Mojo IPC multi-process architecture (similar to the front-end/back-end separation MediaServo may need)
- The standard ICE/STUN/TURN connection establishment flow
- WebRTC DataChannels for input event transport (low latency, on a channel independent from the video stream)
- The viable approach of using a browser as the client
- Multi-process permission separation and security isolation design

---

## VI. Synthesis

### 6.1 Cross-Product Common Patterns

By analyzing all 11 products, the following architecture patterns are industry consensus:

| Pattern | Adopting products | Notes |
|------|---------|------|
| **P2P first + relay fallback** | All | Without exception, every product follows this connection strategy |
| **End-to-end encryption** | All | AES-256 is the de facto standard, with differences in key negotiation methods |
| **Content-aware encoding** | Parsec, RDP, Splashtop, ToDesk | Text/images/video use different encoding strategies |
| **Process permission separation** | AnyDesk, RustDesk, CRD | Network layer and UI layer isolated into different processes |
| **Global relay node deployment** | TeamViewer, AnyDesk, Splashtop, Sunlogin, ToDesk | A hard requirement for reducing end-to-end latency |
| **Hardware acceleration + software fallback** | RDP, RustDesk, Sunshine | Most products retain a software encoding path |
| **Custom protocol** | All products except CRD and RDP | General protocols (RDP/VNC/WebRTC) have scenario limits |

### 6.2 Key Design Decision Comparison

| Decision dimension | Option A | Option B | Who chose A | Who chose B |
|---------|-------|-------|---------|---------|
| Transport protocol | TCP-dominant | UDP-dominant | RDP, TeamViewer, AnyDesk | Parsec, ToDesk, RustDesk |
| Codec | General codec | Proprietary GUI codec | Parsec(H.264), Moonlight | AnyDesk(DeskRT), RDP(RemoteFX) |
| Process model | Single process | Multi-process separation | Parsec, TeamViewer | AnyDesk, RustDesk, CRD |
| Signaling service | Centralized proprietary | Self-hostable | TeamViewer, AnyDesk | RustDesk, Splashtop(OnPrem) |
| GPU pipeline | CPU transfer | GPU zero-copy | Early products | Parsec, Sunlogin 16 |

### 6.3 MediaServo Reusable Checklist

#### Mandatory Architectural Decisions

1. **A P2P direct connection + relay fallback connection model** (industry consensus, no exceptions)
2. **UDP as the first-priority transport protocol** (TCP's head-of-line blocking is fatal in remote desktop scenarios; the experience of Parsec and ToDesk shows UDP + a custom reliability layer is the optimal solution)
3. **Hardware codec first + software codec fallback** (balancing performance and compatibility)

#### Strongly Recommended Design Patterns

4. **Multi-process permission separation** (AnyDesk/RustDesk/CRD practice: network I/O in an independent process, screen capture in an independent process, GUI in an independent process)
5. **Define all protocols in Protobuf** (RustDesk experience: type-safe, multi-language support, version evolution)
6. **Content-aware mixed codec** (RDP practice: text uses a dedicated codec, images/video use H.264/HEVC)
7. **RFC standard protocols before proprietary protocols** (WebRTC's ICE/STUN for connection establishment, DTLS-SRTP for encryption, RTP for media transfer — CRD has verified feasibility)
8. **Caching + differential transfer** (NoMachine experience: within the same session GUI has many repeated elements, cache hit rate 60-80%)

#### Differentiating Directions Worth Exploring

9. **Unified multi-protocol bridging** (NoMachine experience: one core protocol carrying X11/RDP/VNC and so on, but with high implementation complexity)
10. **AI-driven adaptive codec** (Splashtop's 2026 direction: dynamic adjustment according to content and network conditions)
11. **Self-hosted signaling + relay infrastructure** (RustDesk experience: driven by data sovereignty and compliance requirements)
12. **GPU zero-copy pipeline** (Parsec experience: but extreme latency is only reachable under the Windows Desktop Duplication API)

#### Pitfalls to Avoid

- **Do not support TCP only**: Parsec's migration from TCP to its proprietary BUD (UDP) was a performance turning point
- **Do not bind to a single GPU vendor**: Moonlight's lesson — after NVIDIA ended GameStream the community was forced to invest in Sunshine
- **Do not use a proprietary unauditable encryption scheme**: the protocol opacity of TeamViewer/AnyDesk is an obstacle to security audit
- **Do not ignore weak network environments**: ToDesk's experience — 30% packet loss is normal in domestic carrier environments
- **Do not make the signaling service a single point of failure**: TeamViewer's centralized architecture lesson

### 6.4 Technology Selection Recommendations

```
Recommended tech stack (based on research conclusions):
├── Transport layer: WebRTC (ICE/STUN/TURN + DTLS-SRTP + RTP/RTCP)
│   └── Alternative: proprietary UDP protocol (if finer-grained congestion control and codec coupling are needed)
├── Codecs:
│   ├── Software: VP8/VP9/AV1 (royalty-free, full platform support)
│   ├── Hardware: NVENC/AMF/QSV/VAAPI/VideoToolbox (through a unified abstraction layer)
│   └── Text optimization: design a dedicated codec following the RDP mixed-mode idea
├── Control channel: WebRTC DataChannel or a separate TCP channel
├── Signaling service: a lightweight self-hostable signaling service (the RustDesk HBBS model)
├── Relay service: TURN servers + optional dedicated relay nodes
├── Core language: Rust (performance + safety, feasibility verified by RustDesk)
├── UI framework: a cross-platform solution (Flutter/Qt)
└── Protocol definitions: Protobuf
```

> **Note**: this document is a research artifact of the Phase 0 architecture definition stage. As the project advances, some decisions may be adjusted according to actual requirements and constraints. All product data is as of July 2026.

## Related Decisions

| Research finding | Related decision |
|---------|---------|
| Unified Fragment Model (LVQR) | D5 |
| TextureHandle ownership (Parsec/OBS/GStreamer) | D20 |
| Multi-backend trait abstraction (webrtc-kit) | D144-D145 |
| Zero-copy GPU encoding bridge (Parsec 7ms) | D41 |
| Capture-encoding coupled multi-channel output | D45 |
| Rendering Moonlight YUV→RGB shader | D47 |
| GPU Direct interop (Moonlight/Sunshine) | D47 |
| Remote desktop scenario Simulcast 2-layer | D-SIMULCAST |
