# AnyDesk Reference Analysis

> Generated: 2026-07-16 | Category: Remote Desktop

## 1. Product Profile

- **Name**: AnyDesk
- **Developer**: AnyDesk Software GmbH (Stuttgart, Germany; founded 2014 by ex-TeamViewer core members); acquired by a German industrial group in 2024, with the product's technology stack and management team retained
- **First release**: 2014
- **Positioning**: a remote-desktop solution renowned for extreme lightness and ultra-low latency. The core differentiator is the in-house DeskRT video codec — designed specifically for computer graphics (GUI) transmission, not a general-purpose video codec. Positioned as a "faster, lighter, cheaper" alternative to TeamViewer, emphasizing performance-first and deployment flexibility. The client is only 3.7MB and streams at 100kbps
- **Target user groups**:
  - Individual users (the free tier; ad-hoc remote support with no installation)
  - SMB IT support teams (remote support and maintenance; MSP/Help Desk scenarios)
  - Large-enterprise IT departments (On-Premises deployment for data sovereignty)
  - Healthcare (HIPAA-compliant telemedicine; low-bandwidth adaptability)
  - Manufacturing and engineering (remote access to CAD/PLM workstations without moving GB-scale files)
  - Education and training (remote teaching and virtual labs; a 3.7MB client suits student devices)
  - Government (data sovereignty + On-Premises compliance)
  - Extremely-low-bandwidth users (2G/3G mobile networks, satellite links, constrained international routes)
- **Licensing / business model**: Freemium subscription; free (personal non-commercial) → Essentials ($14.90/mo/license) → Performance ($29.90/mo) → Enterprise ($59.90/mo, incl. On-Premises option and SSO); 800M+ downloads worldwide, 180,

## 2. Technical Characteristics

### Overall Architecture

```
│                      The AnyDesk multiprocess separated architecture            │
│  │  Erlang/OTP backend (carrier-grade reliability):                            ││
│  │  │   AnyNet relay network  │  │   Signaling & auth services   │             ││
│  │  │  · Cowboy HTTP           │  │  · Mnesia distributed DB      │             ││
│  │  │    RESTful API           │  │  · ETS in-memory tables       │             ││
│  │  │  · PostgreSQL persistence│  │  · Device registration/discovery │          ││
│  │  │  · Hot upgrades, no downtime │ │ · License management      │             ││
│  │  │  · 10k+ concurrent devices│ │  · Presence maintenance      │             ││
│  │  │    GUI process (user identity) │   │  Service process (SYSTEM) │   │  │
│  │  │  · Qt graphical UI       │   │  · Network I/O handling     │   │  │
│  │  │  · Screen capture (DXGI/GDI) │ │  · NAT traversal probing  │   │  │
│  │  │  · DeskRT encoding       │   │  · TLS 1.3 encrypted channel │  │  │
│  │  │  · User interaction      │   │  · Relay connection management│  │  │
│  │  │  · Input capture/inject  │   │  · Port binding (6568)      │   │  │
│  │                  Named shared memory (qipc_shm_*)                       │  │
│  │                  · shm_svc_mtx (mutex)                                   │  │
│  │                  · ret_evt (return event)                                │  │
│  │                  · qipcshm_%u_%u_%u (inter-process data channel)         │  │
│  │  The AnyNet serialization engine (570+ bounds checks)                    │  │
│  │  │  All inbound protocol data goes through AnyNet parsing, per-field     │  │  │
│  │  │    bounds checks                                                      │  │  │
│  │  │  · Message types: Network Layer / Session / IPC / TCP Tunnel         │  │  │
│  │  │  · RPC methods: rt_register / rt_login / rt_session_create           │  │  │
│  │  │  · 30+ error codes, 8+ HRESULT mappings                              │  │  │
│  Connection strategy (relay-first, opposite of industry convention):          │
│  Layer 1: TLS relay (443)  ─── relay preferred (trust the Erlang network)    │
│  Layer 2: TLS relay (80)   ─── when 443 is blocked                           │
│  Layer 3: SOCKS proxy (443) ── corporate proxy environments                  │
│  Layer 4: P2P direct (443)  ─── P2P tried last                               │
│  Layer 5: P2P direct (80)   ─── backup port                                  │
│  Layer 6: P2P direct (6568) ── the AnyDesk dedicated port                     │
```

### Connection Establishment (AnyDesk's relay-first strategy)

```
Controller                          AnyNet relay network                 Controlled end
  │── 1. TLS connect + device auth ───────►│◄── 1. long-lived connection + heartbeat ──│
  │     · registers the device via the relay │     · device ID + password/token          │
  │     · exchanges RSA/ECC keys             │     · 10k+ concurrency managed by Erlang │
  │── 2. Query the target's presence ───────►│                                          │
  │◄── 3. Returns: online + routing info ────│                                          │
  │── 4. Request a relayed connection ──────►│◄── 4. Notify an inbound connection ──────│
  │     [AnyDesk defaults to the relay first]│                                          │
  │◄══════ TLS 1.3 relay channel ════════►│◄═════ TLS 1.3 relay ═════════│
  │     [Optional: attempt P2P direct]        │                                          │
  │── 5. P2P negotiation request ────────────►│◄── 5. P2P negotiation ─────────────│
  │── 6. UDP/TCP hole punching ───────────────────────────────────────────►│
  │     [Punch succeeds]                      │                                          │
  │◄══════ P2P AES-256 direct ════════════════════════════════════════►│
  │     [Punch fails: keep using the relay]    │                                         │
  │◄══ Relay AES-256 (TLS 1.3 wrapped) ══►│◄══ Relay AES-256 ═══════════│
  │  Video: DeskRT encoding ── 60fps ── <16ms latency ── as low as 100kbps       │
```

### The DeskRT Codec in Detail

```
│            DeskRT — a codec designed specifically for computer-graphics transmission │
│  Design philosophy:                                                            │
│  │ General-purpose video codecs (H.264/H.265/VP9):                            ││
│  │ · Design target: natural video (camera-shot movies/TV)                     ││
│  │ · Optimization: motion estimation + DCT frequency transform + motion compensation ││
│  │ · Weakness on GUI: blurred text edges, color-block artifacts, wasted bitrate on large flat areas ││
│  │ DeskRT:                                                                     ││
│  │ · Design target: computer-generated graphics (GUI windows/text/icons/menus) ││
│  │ · Optimization: text sharpness, large-flat-area compression, precise cursor rendering ││
│  │ · Core strengths:                                                           ││
│  │   - Text and lines stay crisp (no DCT compression artifacts)                ││
│  │   - Compression of desktop GUI content far exceeds general encoders         ││
│  │   - Usable in extremely low bandwidth (100kbps)                             ││
│  │   - Encode latency <16ms at 60fps                                           ││
│  How it works (based on public information and reverse engineering):           │
│  │  1. Screen diff detection                                                   ││
│  │     · Compares changed screen regions between adjacent frames               ││
│  │     · Encodes only the changed rectangles (like RDP RemoteFX)               ││
│  │  2. Content classification & pattern recognition                            ││
│  │     · Text regions (high contrast, sharp edges) → run-length + differential ││
│  │     · Graphics regions (UI widgets, icons, borders) → lossy but edge-preserving ││
│  │     · Image regions (photos, thumbnails) → JPEG/WebP-like compression       ││
│  │     · Video regions (animation, playback) → inter prediction + motion estimation ││
│  │  3. Adaptive compression strategy                                           ││
│  │     · Ample bandwidth (>1Mbps): lossless or near-lossless text + high-quality images ││
│  │     · Constrained bandwidth (<500kbps): lossy text + reduced color depth    ││
│  │     · Extremely low bandwidth (100kbps): an 8-tone palette + a minimum 10fps ││
│  │  4. Desktop-GUI-specific optimizations                                      ││
│  │     · Window borders/shadows: cache reuse (position changes, style doesn't) ││
│  │     · Mouse cursor: a separate channel (not consuming encode bitrate)       ││
│  │     · Text input areas: partial updates (only changed text lines)           ││
│  Performance metrics:                                                           │
│  · Encode latency: <16ms (end to end)                                           │
│  · Frame rate: up to 60fps                                                      │
│  · Minimum bandwidth: 100kbps (still operable)                                   │
│  · Typical bandwidth: 1-5Mbps (30fps+ in everyday office scenarios)             │
│  · Static screens: 0 bandwidth (nothing sent when nothing changes)              │
│  · GUI compression: 50-70% bandwidth saved vs H.264 I-frames                    │
```

### The AnyNet Protocol Parser (revealed by reverse engineering)

```
│        The AnyNet serialization engine — a protocol-safe implementation with 570+ bounds checks │
│  Protocol layers (by message type):                                            │
│  Session control:                                                               │
│  │  · Device registration (rt_register)                                        ││
│  │  · User authentication (rt_login)                                           ││
│  │  · Session creation (rt_session_create)                                     ││
│  │  · Session join (rt_session_join)                                           ││
│  │  · Keep-alive (ping/pong)                                                   ││
│  Media channels:                                                                │
│  │  · Video frame transport (DeskRT-encoded data)                              ││
│  │  · Audio data transport                                                     ││
│  │  · Input event forwarding (keyboard/mouse/touch)                            ││
│  │  · Clipboard sync                                                           ││
│  │  · File transfer control                                                    ││
│  IPC messages:                                                                  │
│  │  · GUI → Service message forwarding                                         ││
│  │  · Service → GUI event notification                                         ││
│  Tunneling:                                                                     │
│  │  · Port forwarding (TCP tunnels)                                            ││
│  │  · VPN-mode tunnels                                                         ││
│  Check-type inventory:                                                          │
│  │  · Length-field validation: the declared length must be ≤ the packet's real size ││
│  │  · Range checks: offsets must fall within valid ranges                      ││
│  │  · Integrity checks: verifying data-block integrity markers                 ││
│  │  · Type checks: serialized field types must match expectations              ││
│  │  · Recursion depth limits: preventing stack overflow from nested structures ││
│  │  · Null checks: pointers validated non-null before dereference              ││
│  │  · Alignment: memory alignment of data accesses ensured                     ││
│  │  · 570+ checks in total cover the whole serialization pipeline              ││
│  Security assessment:                                                           │
│  ✅ Strength: defense in depth (per-message-type validation, not one generic parser) │
│  ✅ 30+ precisely classified error codes (network/auth/session/system)          │
│  ⚠️ the .itext code section is marked RWC (Read/Write/Execute)                  │
│  ⚠️ the whole 30MB code page stays writable at runtime (needed for binary unpacking) │
│  ⚠️ the IPC shared memory uses NULL security attributes (relies on Session isolation) │
```

### The Process-Privilege Separation Model

```
│                  The AnyDesk multiprocess privilege-separation architecture     │
│  Session 0 (isolated session)             Session 1+ (user interaction session) │
│  │ AnyDesk Service (SYSTEM) │             │ AnyDesk GUI (user identity)         │
│  │  · All network I/O here  │  IPC        │  · The Qt graphical UI              │
│  │    in this process       │◄─────►│    │  · Screen capture (DXGI/GDI)        │
│  │  · TLS encrypt/decrypt   │ shared │   │  · DeskRT encoding                  │
│  │  · AnyNet protocol parsing│ memory│   │  · User authorization dialogs       │
│  │  · Relay connection mgmt │        │   │  · Input event capture/injection    │
│  │  · NAT traversal probing │        │   │  · Clipboard management             │
│  │  · File-transfer network layer │   │  │                                    │
│  │  · Port binding (6568/7070) │    │   │  Cannot access the network directly │
│  │  · Updater execution     │        │   │  (all network I/O forwarded via IPC)│
│  │  Cannot access the desktop/user UI │  │                                     │
│  Security boundaries:                                                           │
│  · Session 0 ↔ Session 1+ isolated naturally by Windows Session Isolation       │
│  · The Service, though SYSTEM, cannot read the user's desktop content directly  │
│  · The GUI, though it can capture the screen, cannot open network connections directly │
│  · Both process chains must cooperate to complete a remote-desktop session      │
│  · An attacker must break both processes to gain full control                    │
```

### Key Technical Capabilities

| Capability | Details |
|------------|---------|
| Video encoding | **The in-house DeskRT codec**: designed for computer graphics (GUI content), not general video. Streams smoothly at 100kbps, 60fps, <16ms encode latency, adaptive resolution. Zero bandwidth on static screens |
| Transport protocol | **The proprietary AnyNet protocol**: an in-house serialization engine (570+ bounds checks), TLS 1.3 transport encryption, a relay-first strategy (relay by default, optional P2P direct). Port strategy: 443 → 80 → SOCKS proxy → P2P 6568 |
| Security | TLS 1.3 + AES-256 transport encryption + RSA-4096 (On-Premises) / ECC-256 (Cloud) key exchange + PFS; two-factor auth (2FA); ISO 27001 + SOC 2 Type II + GDPR + NIS 2 compliance |
| Connection strategy | **Relay-first (not P2P-first!)**: TLS relay 443 → TLS relay 80 → SOCKS proxy 443 → P2P direct 443/80/6568. Trusting the Erlang relay network's availability and low latency rather than unreliable P2P hole punching |
| Platform support | Windows / macOS / Linux / iOS / Android / ChromeOS / Raspberry Pi / FreeBSD — 9 OS platforms |
| Client size | Only 3.7MB (Windows), about 1/10 of TeamViewer's |
| Deployment | Cloud (default) + On-Premises (enterprise); portable no-install runs, MSI silent installs, GPO policy management |
| Permission control | Device allowlists, per-session privilege separation (view/full-control/file-transfer each independently authorized), unattended passwords, one-time temporary passwords |

### Codec Comparison

| Property | DeskRT (AnyDesk) | H.264/H.265 (general) | RemoteFX (RDP) | VP8/VP9 (WebRTC) |
|----------|------------------|-----------------------|----------------|-------------------|
| Design target | Computer graphics (GUI) | Natural video (cameras) | Desktop content | General (video+desktop) |
| Text sharpness | ★★★★★ razor-sharp | ★★★☆☆ DCT blur | ★★★★☆ fairly sharp | ★★★☆☆ DCT blur |
| Large-flat-area compression | ★★★★★ excellent | ★★★☆☆ average | ★★★★☆ high | ★★★☆☆ average |
| Video content handling | ★★★☆☆ passable | ★★★★★ excellent | ★★★☆☆ passable | ★★★★★ excellent |
| Ultra-low bandwidth (100kbps) | ★★★★☆ usable | ★★☆☆☆ difficult | ★★★☆☆ passable | ★★★☆☆ passable |
| Encode latency | <16ms | 5-50ms | 10-30ms | 5-40ms |
| Royalty-free | ❌ proprietary closed | ❌ MPEG LA fees | ❌ Microsoft proprietary | ✅ fully free |
| Open implementations | ❌ unauditable | ✅ x264/openh264 | ❌ | ✅ libvpx |
| GPU hardware acceleration | ❌ pure software | ✅ all vendors' GPUs | ✅ Windows GPUs | ✅ partially supported |
| GUI compression efficiency | ★★★★★ optimal | ★★★☆☆ | ★★★★☆ | ★★★☆☆ |

### Technology Stack

- **Backend (the AnyNet infrastructure)**:
  - Language: Erlang/OTP (carrier-grade reliability, hot upgrades with no downtime, 10k+ concurrent device management)
  - Web framework: Cowboy (a high-performance Erlang HTTP server implementing the RESTful API)
  - Databases: PostgreSQL (persistent) + Mnesia (distributed) + ETS (in-memory tables, fast cache)
  - Architecture pattern: the Actor model for concurrency + supervision-tree fault isolation + horizontal scalability (Erlang's standard distributed model)
- **Client**:
  - Language: C++/Qt (cross-platform GUI + network I/O + screen capture)
  - Size: 3.7MB (extremely light; about 1/10 of TeamViewer's client)
  - Minimal dependencies: Qt core modules only (loading what's needed); no Electron, no WebView dependency
- **Protocol stack**:
  - Serialization: the in-house AnyNet protocol engine (a custom binary protocol with 570+ bounds checks)
  - Transport encryption: TLS 1.3
  - Session encryption: AES-256
  - Key exchange: RSA-4096 (On-Premises) / ECC-256 (Cloud)
  - Forward secrecy: full PFS support
  - Port strategy: 443/80/6568/7070 (multi-port + protocol multiplexing)
- **Video encoding**: the in-house DeskRT codec (designed for computer-graphics transmission)
- **Screen capture**: Windows DXGI Desktop Duplication (direct GPU frame copy)
- **Crypto library**: OpenSSL (about 1/4 of the binary is OpenSSL code, nearly 12,000 functions)
- **IPC**: named shared memory (CreateFileMappingW); about 1,500 lines of IPC management code
- **Input simulation**: Windows SendInput + Linux X11 XTest + macOS CGEvent

## 3. Feature Overview

### Core Feature Modules

- **Remote desktop control**: a complete remote-desktop experience with multi-monitor switching, 4K resolution, 60fps high frame rate, DeskRT adaptive quality/framerate, true color, a dedicated mouse-cursor channel (not consuming encode bitrate), and system-level hotkey passthrough (Alt+Tab etc.)
- **File transfer**: bidirectional file transfer with drag-and-drop, folder sync, transfer queues, resume, and bandwidth limits
- **Remote printing**: print documents from the remote host directly on a local printer without first transferring the file; color/BW selection and multi-printer configuration
- **TCP tunneling (port forwarding)**: maps any remote TCP port to local, proxying access to intranet services (HTTP/SSH/RDP/databases etc.). The AnyNet protocol has built-in TCP tunneling message types
- **Clipboard sync**: bidirectional clipboard sync for text and images (very low sync latency, <100ms)
- **Session recording**: full session recording to video (an enterprise feature) for compliance audits, with role-based access control (who may view/download recordings)
- **Voice calls (VoIP)**: built-in voice communication for in-session real-time talk (no third-party audio conferencing needed)
- **Whiteboard annotation**: draw annotations directly on the remote screen; suited to training and demos
- **Wake-on-LAN**: wake a remote computer over the LAN (requires target hardware support)

### Distinctive Features

- **DeskRT's ultra-low-bandwidth capability**: the core differentiator. Smooth remote desktop at 100kbps. An irreplaceable advantage for mobile networks (throttled 3G/4G), satellite links, and underdeveloped internet routes. H.264/H.265 cannot sustain usable frame rates and text sharpness at the same bandwidth
- **The 3.7MB ultra-light client**: one of the world's smallest full-featured remote-desktop clients. In IT support, download-to-session takes only 5-10 seconds, greatly improving Help Desk responsiveness. Portable mode needs no install and leaves no registry residue. Pure C++/Qt, not Electron-packaged (avoiding a 100MB+ Chromium bundle)
- **The Erlang/OTP carrier-grade backend**: the AnyNet infrastructure is built in Erlang, the large-scale concurrency approach proven by telecom (e.g. Ericsson mobile switches). The Actor model makes each connection an independent lightweight process (not an OS thread); supervision trees isolate crashes; hot code upgrading needs no downtime. The reliability target is 99.999%
- **The distinctive relay-first connection strategy**: unlike TeamViewer/RustDesk's "P2P-first" strategy, AnyDesk defaults to the Erlang relay network. The design rests on two judgments: 1) P2P hole punching has limited real-world success (~50-70%); 2) the Erlang infrastructure's latency is already low (the relayed path)
- **Multiprocess privilege separation + IPC shared memory**: the GUI process (user identity) and the Network Service process (SYSTEM identity) run as two independent processes. The network process cannot touch the desktop directly, but screen-capture data moves efficiently through named shared memory (zero-copy, faster than socket IPC). A canonical example of security isolation + high-performance communication
- **Complete On-Premises deployment** (enterprise): the whole AnyDesk infrastructure (signaling + relay + admin console) can run inside a corporate network; Windows/Linux server support; satisfies the strict data-sovereignty demands of government, finance, and defense; provides custom namespaces (e.g. `device@company.internal`)
- **The AnyNet serialization engine's extreme security investment**: protocol reverse engineering revealed 570+ bounds checks in the parser — one of the deepest protocol-layer security investments among remote-desktop products. Every field of every inbound message is independently validated — not one generic validation function covering all message types. This is the Log4Shell/Heartbleed-era lesson embodied
- **White labeling**: enterprise customers can ship the AnyDesk client under their own brand (custom logo, theme colors, installer name). For MSPs and large-enterprise IT, brand consistency of the support tool directly shapes end users' perception and trust

### Extensibility / Plugin Mechanisms

- **RESTful API**: device management, session control, and user management interfaces exposed via the Cowboy HTTP server; supports third-party PSA/RMM integrations (ConnectWise/Autotask etc.)
- **The AnyDesk Deployment Tool**: enterprise batch deployment/management with MSI silent installs and GPO policy distribution
- **my.anydesk.com**: the web admin console for device management, user permissions, session logs, and license management
- **Custom namespaces** (enterprise): enterprises can use custom address books (e.g. `desk01@company.internal`) instead of AnyDesk's default public ID system
- **Custom client branding** (enterprise): logo, theme colors, and installer name fully brandable
- **LDAP/AD integration** (enterprise): user/group sync + SSO, supporting Active Directory and OpenLDAP
- **SAML 2.0 SSO** (enterprise): integrates with enterprise identity providers
- **CI/CD automation scripts**: CLI flags like `--set-password`, `--with-password`, `--install`, `--remove`, `--get-id`, `--start-service` — 40+ flags supporting fully scripted deployment and configuration
- **URL handler registration**: the `anydesk://` protocol handler lets third-party apps initiate remote connections via URL scheme; embeddable in CRM/Help Desk ticket flows
- **Custom port configuration**: enterprises can configure client communication ports as needed (avoiding conflicts with internal firewall policy)

## 4. Status & Ecosystem

- **Current version**: the v8.x series (continuously updated 2025-2026), per-platform version numbers, monthly/quarterly cadence
- **Scale**:
  - 800M+ downloads worldwide (cumulative since 2014)
  - 180,000+ paying customers
  - Present in 190 countries and regions
  - One of Germany's 50 fastest-growing companies (2020-2025)
  - 500+ employees (2025)
  - Fortune 500 brand customers
  - 10k+ concurrent device connections per Erlang cluster
- **Key development timeline**:
  - **2014**: founded. The founding team came from TeamViewer's core, having lived through TeamViewer's 0-to-300M-MAU arc; released the first DeskRT codec prototype, marketing "faster than TeamViewer"
  - **2015-2016**: first public release; rapid user growth. The C++/Qt client kept optimizing; the 3.7MB size was finalized
  - **2017**: introduced the Erlang/OTP backend (the AnyNet infrastructure), upgrading the backend from a monolith to a distributed carrier-grade platform — AnyDesk's most important architectural upgrade; Erlang's Actor model and hot-upgrade capability became the core of its technical story
  - **2018-2019**: launched the On-Premises enterprise edition, penetrating from consumer into enterprise; released the REST API and admin console (my.anydesk.com)
  - **2020**: explosive download growth during COVID-19; remote work pushed the user base from 200M to 500M downloads; introduced 2FA
  - **2021**: passed 500M downloads; a German tech-unicorn candidate; expanded enterprise features (SSO/LDAP/Session Recording)
  - **2022-2023**: growth to 700M+ downloads; more security certifications (SOC 2 Type II / NIS 2); expanded to Raspberry Pi and more embedded platforms
  - **2024**: acquired by a German industrial group (acquirer and amount undisclosed); the product's technology and management team retained, operating independently
  - **2025**: passed the 800M-download milestone; published an AI info page (`anydesk.com/ai-info`), hinting AI-driven features in development
  - **2026**: a detailed protocol reverse-engineering report published publicly (Daniele Berardinelli, April 2026), revealing extensive internal architecture details and becoming a hot subject in the security-research community
- **Community**:
  - The official support community and knowledge base: support.anydesk.com (multilingual, active peer help)
  - The security transparency center: trust.anydesk.com (a dedicated subdomain listing all certifications and compliance docs)
  - Tech blog and product changelog: anydesk.com/blog
  - High security-research attention: DeskRT and AnyNet reverse engineering are popular research subjects (multiple Frida/Ida Pro/Ghidra reports)
  - Reddit r/AnyDesk: a user-help community
  - Stack Overflow: an active `anydesk` tag
- **Docs / SDK / API ecosystem**:
  - The official help center and knowledge base (multilingual, incl. Chinese)
  - Complete REST API docs (device management, user management, session control)
  - The API backend uses Cowboy (the Erlang HTTP framework) + PostgreSQL
  - The security compliance doc set: ISO 27001 / SOC 2 Type II / HIPAA / GDPR / NIS 2
  - The my.anydesk.com web admin portal guide
  - On-Premises deployment guides (Windows Server / Linux servers)
  - Detailed MSI deployment and Active Directory integration docs
  - No formal SDK, but the REST API is callable from any language
- **Known defects or limitations**:
  1. **Updates don't verify software signatures** (severe): reverse engineering revealed that AnyDesk downloads updates every 9 hours over the AnyNet relay protocol **without verifying signatures** — relying only on TLS transport security. If the relay network were compromised, a malicious update could be injected into millions of devices. Though TLS provides transport security and the Windows Installer verifies the package
  2. **The code section lacks DEP protection** (a security-architecture defect): reverse engineering revealed the `.itext` section marked RWC (Read/Write/Execute); the whole 30MB code page stays writable at runtime. This is a technical need of binary unpacking (LZMA decompression + keystream decryption), but it means the binary doesn't live under modern operating systems' W^X policies
  3. **Passwords stored in plaintext in memory** (runtime security): after DPAPI decryption, passwords and RSA private keys exist in plaintext in `service.conf` and related memory. Frida can hook `CryptUnprotectData()` to extract the decrypted secrets at runtime
  4. **IPC shared memory uses NULL security attributes**: `CreateFileMappingW` creates the shared-memory object with a `NULL` security descriptor, relying on Windows Session Isolation for cross-session protection, with no explicit ACL hardening — a contrast to the 570+-bounds-check protocol security investment
  5. **The relay-first strategy's privacy implication**: relaying by default means all media passes AnyDesk's servers. Though TLS 1.3-encrypted, the data is visible to the operator at the server side (transport encryption ≠ end-to-end encryption). An essential gap vs RustDesk's NaCl E2E model (the relay cannot decrypt)
  6. **DeskRT's closed-source unauditability**: the proprietary codec cannot be third-party audited or independently performance-verified; all performance claims rest on the vendor. In security-sensitive scenarios (government/finance/defense) this is a major compliance obstacle
  7. **The pure-TCP strategy's performance loss on weak networks**: relying on TCP transport (relay-first + P2P TCP), high loss (>5%) causes TCP congestion control to produce obvious stutter and jitter. UDP approaches (WebRTC / Parsec's BUD) outperform TCP on high-loss networks
  8. **Erlang talent-scarcity risk**: maintaining and extending the Erlang infrastructure requires rare Erlang/OTP engineers. Erlang has a stable market in telecom and finance but far fewer developers than Rust/Go/Java/TypeScript — a high long-term maintenance cost
  9. **Post-acquisition strategic uncertainty**: after the 2024 acquisition, though the team and product system were retained, the long-term roadmap is influenced by the parent's strategy. Similar events (LogMeIn's post-acquisition drift, Citrix's repositioning) have hurt user trust

## 5. Market Positioning

- **Primary industries**:
  - IT support and remote ops (the MSP/Help Desk core scenario: fast start + cross-platform + low-bandwidth unreliable networks)
  - Manufacturing and engineering (remote CAD/PLM/CAM workstation access without moving GB-scale model files)
  - Healthcare (telemedicine consults, HIPAA compliance, usable on low bandwidth — rural clinics on satellite internet)
  - Education (remote teaching and virtual labs; a 3.7MB client suits students' varied devices)
  - Government (data sovereignty + On-Premises deployment for national data-security regulations)
  - Financial services (strict compliance but needing low-latency access to trading terminals; the relay-first fast connection is key)
  - Call centers and BPO (fast-response remote IT support; relay-first reduces per-session setup waits)
  - Embedded/IoT (Raspberry Pi / ARM device remote management; the light client suits resource-constrained devices)
- **Competitor quick comparison**:

| Competitor | Strengths | Weaknesses |
|------------|-----------|------------|
| TeamViewer | the largest user base (300M+ devices/mo), the most complete enterprise features, an IoT/AR ecosystem, 1000+ global nodes | 10x the client size, ~40% pricier, trust damaged by the 2016 security incident, latency usually above AnyDesk's |
| RustDesk | fully open source, self-hostable, NaCl E2E encryption (the relay can't decrypt), zero cost | no DeskRT-grade low-bandwidth equivalent, lower TCP punch success, enterprise features (SSO/LDAP) need the Pro edition |
| Splashtop | refined product lines, On-Premises + multi-provider redundancy, AI-optimized codecs | weaker brand than AnyDesk, no DeskRT-grade proprietary encoding breakthrough, client >15MB |
| Parsec | the 7ms game-grade latency benchmark, GPU zero-copy, virtual-gamepad multiplayer innovation | hardware encoding only (excludes older machines), Windows Host only, its core scenario is gaming not remote desktop |

- **Pricing / licensing**:
  - **Free**: personal non-commercial use, core remote desktop, watermarked single session, limited to ~30 minutes
  - **Essentials**: $14.90/mo/license (1 user, up to 3 managed devices, 1 concurrent session)
  - **Performance**: $29.90/mo/license (unlimited devices, session recording, MSI deployment, mobile support)
  - **Enterprise**: $59.90/mo/license (unlimited devices, white labeling, self-hosted relays, SAML SSO, LDAP/AD integration, full API access, SLA)
  - The above are 2025 reference prices; annual subscriptions get roughly 15-20% off
  - AnyDesk claims to be ~40% cheaper than TeamViewer for equivalent features

## 6. Product Highlights

1. **DeskRT — a technical moat purpose-built for GUI transmission**: DeskRT is AnyDesk's core differentiator. It is not a general-purpose video codec but one designed specifically for computer graphics (windows, text, icons, buttons, menus). This brings two key advantages: 1) GUI compression far exceeding H.264/H.265 (text
2. **The relay-first connection strategy — challenging the "P2P-first dogma"**: nearly every remote-desktop product follows "P2P-first + relay fallback"; AnyDesk deliberately went the other way — TLS relayed connections by default, P2P direct as an optional fallback. Based on two judgments: 1) the Erlang global relay network's
3. **Erlang/OTP carrier-grade infrastructure — building a remote-desktop service with a telephone switch's reliability**: AnyDesk is the only remote-desktop product using Erlang/OTP at scale. Erlang was designed for telecom switches and natively supports: hot upgrades (no-downtime upgrades, the industry's highest continuous-availability standard), Actor-model concurrency (each connection as an
4. **The 3.7MB client's engineering aesthetics — subtraction taken to the extreme**: AnyDesk's 3.7MB client is an industry outlier (TeamViewer 30MB+, RustDesk 10MB+, ToDesk 50MB+). The extreme lightness comes from a minimal C++/Qt, on-demand module loading, and refusing unnecessary third-party dependencies. This means not only
5. **The AnyNet serialization engine — the security benchmark for protocol parsers**: reverse engineering revealed 570+ bounds checks invested in parser security — the deepest such investment among remote-desktop products. Every field of every inbound message is validated independently — not one generic validator covering all message types but per-type validation logic. This is
6. **Multiprocess privilege separation — a "compartmentalized defense" security architecture**: the GUI (user identity) and the Service (SYSTEM identity) run as two independent processes communicating via named shared memory. This achieves two security goals: 1) privilege isolation between the network layer and the user layer — the network process cannot touch the desktop and the GUI process cannot touch the network; 2) an attacker must break both

## 7. Reference Value for MediaServo

### [Adopt] Directly Adoptable

- **The process-privilege separation architecture**: a dedicated network-I/O process + a dedicated screen-capture process with privilege isolation. MediaServo's Plugin system naturally supports this — ScreenCapture/NetworkTransport/InputInjector can each live in their own Plugin Worker, communicating through
- **DeskRT's design philosophy — GUI-specific rather than general-video encoding**: MediaServo should, on top of general codecs (H.264/HEVC/AV1), explore a lightweight optimization layer for desktop GUI content. No need to reinvent the wheel — it can be a "preprocessor" that classifies GUI frames before the general encoder, handles text
- **The 570+-bounds-check protocol security philosophy**: MediaServo's ProtocolBroker should implement field-level bounds checks on all inbound messages. Protobuf itself provides basic serialization safety, but business-logic constraints need extra validation. Each message type should have its own validator, never sharing a generic parser. Treat "never trust inbound data" as an architectural
- **Erlang/OTP's hot upgrades and fault isolation**: MediaServo's core is Rust, but the signaling service's reliability bar is extremely high. No-downtime upgrades can be achieved via "dual-instance rolling upgrades + connection migration" (similar to Erlang OTP's release handling); fault isolation can ride Rust tokio task boundaries
- **Scenario-based thinking about relay-first**: in some MediaServo deployments (vehicle→cloud push, weak networks), relay-first may beat P2P-first. The connection strategy should be configurable: "P2P first" / "relay first" / "smart auto-select" (Phase 2+ deciding from historical data)
- **The 3.7MB minimal client's dependency-management philosophy**: MediaServo's native layer should follow the same principle — every non-stdlib dependency of the core SDK needs an explicit performance/security justification, reviewed and documented per dependency
- **The Cowboy HTTP + RESTful API management-interface pattern**: the design of management APIs for the signaling service (device CRUD, session control, permission management) aligns closely with MediaServo's admin API needs
- **The zero-bandwidth static-screen optimization idea**: stop sending when the screen hasn't changed — an optimization DeskRT shares with RDP. MediaServo's encoder plugin should check frame diffs before encoding — if the whole frame is unchanged, skip encode and transmit

### [Adapt] Needs Modification Before Adoption

- **Named shared memory IPC → the mediaservo-core message bus + Plugin Workers**: AnyDesk's `CreateFileMappingW` + `NULL`-security-attribute IPC has security flaws. MediaServo should use safer IPC: Unix domain
- **DeskRT's proprietary closed codec → a layered codec strategy**: MediaServo cannot clone DeskRT (proprietary, closed), but can design a layered encode pipeline: Layer 1 (general encoders): H.264/HEVC/AV1 hardware encoding (performance-first); Layer 2 (software fallback): VP8/VP9/AV1 software encode
- **TLS 1.3 transport encryption (relay-visible) → NaCl/DTLS-SRTP end-to-end encryption**: MediaServo should guarantee by design that neither the signaling nor the relay server can decrypt media. WebRTC's DTLS-SRTP or RustDesk's NaCl (xsalsa20-poly1305) beats TLS transport encryption
- **Unsigned updates → dual-signature verification + the TUF framework**: MediaServo's update system must verify digital signatures independently of transport security. Use The Update Framework (TUF) or custom dual-signature verification (code signing + deployment signing) so that even a compromised CDN or relay network cannot inject a malicious update
- **RWC code sections (writable + executable) → a strict W^X policy**: MediaServo binaries must strictly separate code (RX) from data (RW); write+execute together is never acceptable. Enable `-C link-arg=-Wl,-z,relro -C link-arg=-Wl,-z,now` (Rust) at compile time to ensure
- **Relay-first's privacy problem → a configurable connection strategy + explicit privacy statements**: where relay-first is genuinely needed (e.g. vehicle push), the design doc should state the data-visibility boundary at the server explicitly. Offer E2E encryption as an enhancement. Make the boundary between "transport encryption" and "end-to-end encryption" transparent to users
- **The Erlang backend's skill barrier → Rust async + PostgreSQL + message queues**: MediaServo's signaling service uses Rust (consistent with the core), with PostgreSQL (persistence) or Redis (fast cache) replacing Mnesia/ETS roles. Rust's tokio async model can handle

### [Avoid] Known Pitfalls / Non-Applicable

- **The unsigned-update design flaw**: AnyDesk's most serious security liability — downloading updates every 9 hours over the relay while relying only on TLS. MediaServo must implement independent signature verification from Phase 0. Any "ship first, add signature checks later" is unacceptable
- **Missing code-section DEP**: the binary must never, under any circumstance, allow a writable code page. This is OS-level security hygiene, not something to trade for performance or deployment convenience
- **IPC shared memory with NULL security attributes**: every IPC channel needs an explicit ACL or permission validation. On Unix, Unix sockets use file permissions (0600); on Windows, named pipes get a security descriptor
- **DeskRT's proprietary closedness**: MediaServo's protocols and codecs must be RFC standards or publicly documented. Proprietary codecs are unauditable (a compliance obstacle for government/finance/defense) and create vendor lock-in (if AnyDesk changes its license or drops support, DeskRT becomes a technical dead end)
- **Relay-first exposing media to a third party**: for self-hosted scenarios provide E2E encryption + a relay-opaque option. For delegated scenarios (managed by AUDEBase), audit requirements may need server-side visibility — make that a configuration option, not a hardcode
- **The pure-TCP strategy's weak-network losses**: TCP's head-of-line blocking and slow start are fatal at high loss. MediaServo's media transport must ride UDP (RTP/WebRTC RTCDataChannel); TCP is for signaling only. This is the Parsec/ToDesk/RustDesk consensus
- **Erlang talent scarcity's long-term maintenance risk**: MediaServo's infrastructure components should stay within the core stack (Rust/C++/TypeScript), avoiding technology islands that need scarce talent
- **Passively accepting a relayed encryption scheme**: don't equate "encrypted" with "secure" — TLS 1.3 protects the link, but data is visible to the operator at the server. The architecture doc must explicitly distinguish "transport encryption" (TLS, protecting against man-in-the-middle) from "end-to-end encryption" (WebRTC DTLS-SRTP / NaCl, protecting against the server operator), and the design must support

**Overall rating**: ★★★★☆ (4/5)
**Comment**: DeskRT's proprietary codec and the Erlang backend's carrier-grade reliability are AnyDesk's technical pillars. The process-privilege separation architecture and the parser's deep security investment (570+ bounds checks) are direct references for MediaServo's design. But the relay-first privacy implication, the unsigned updates, and the missing code-section DEP are security lessons to absorb

**Related decisions**: D2, D77

## Appendix: Key Findings of the AnyDesk Reverse Engineering (published April 2026)

### Binary Structure Analysis

```
Structure analysis of anydesk.exe:
├── the .data section
│   └── an encrypted payload → a linear congruential generator (LCG) produces the keystream for decryption
│       └── LZMA decompression → 30MB of x86 machine code (the real AnyDesk binary)
│           └── written into the .itext section (marked Read/Write/Execute)

├── the .itext section (RWC — readable/writable/executable simultaneously)
│   ├── OpenSSL code (~12,000 functions, about 1/4 of the binary)*
│   │   └── identified via assertion strings (crypto/*.c, ssl/*.c filename references)
│   ├── Qt framework code (~1/5 of the binary)
│   │   └── GUI event handling, widget rendering, SendMessageW (516 calls)
│   ├── CRT/STL boilerplate
│   ├── the AnyNet protocol implementation (0x102FB550)
│   │   └── 570+ bounds checks, message serialization/deserialization, error-code mapping
│   ├── screen capture/rendering code
│   │   └── capture_pool_%u_shm object creation and destruction
│   └── the Service/IPC layer
│       └── wrappers over CreateFileMappingW, CreateProcessAsUserW etc.

└── key function addresses and roles
    ├── 0x102FB550: the AnyNet serialization engine (570+ bounds checks)
    ├── 0x108F9970: the CLI argument parser (40+ flags: --set-password etc.)
    ├── 0x101F68D0: the HRESULT error-code mapping (30+ codes, 8+ HRESULTs)
    ├── 0x10ACF350: the shared-memory manager (CreateFileMappingW + NULL attributes)
    ├── 0x10812770: CreateProcessAsUserW (Service→GUI process creation)
    ├── 0x10A383E0: a SetSecurityInfo wrapper (a silent failure point)
    └── 0x108038C0: the anydesk:// URL protocol registrar

*Identified via hardcoded OpenSSL source paths in the binary (crypto/evp/e_aes.c etc.) and
certificate/X.509/TLS-related functions. AnyDesk statically links OpenSSL.
```

### Security Issues Revealed by Frida Runtime Tracing

```
The DPAPI data flow:
1. service.conf contains DPAPI-encrypted configuration data
2. At startup AnyDesk calls CryptUnprotectData() to decrypt
3. The decrypted data (plaintext passwords and the RSA private key) lives in process memory
4. Re-encrypted with DPAPI (fresh random padding parameters) and written back to disk

Key findings:
· Passwords and the RSA private key are stored in plaintext in service.conf (post-DPAPI state)
· DPAPI re-encryption introduces random padding → service.conf's content changes every start
· But the underlying data is unchanged (the same password/the same private key) — re-encryption only changes the ciphertext's appearance
· Problem: the in-memory plaintext lacks an extra protection layer (no memory-region encryption, no prompt zeroization)
· Hooking CryptUnprotectData() with Frida → the complete decrypted secrets are obtainable

Impact:
· An attacker who can read the running AnyDesk process's memory → extracts passwords and the private key
· But that requires local physical access or admin rights → the practical threat depends on environment security
· Application-memory encryption (Windows enclave/VBS) could mitigate, at high complexity and performance cost
```

### A Security Analysis of the Update Mechanism

```
The update-check flow:
│ every 9 hours                                                 │
│  └────────┬─────────┘    (over the AnyNet relay protocol, TLS-encrypted) │
│  │ the update package download  │                              │
│  │ · transported via the relay   │                              │
│  │ · protected by TLS 1.3        │                              │
│  │ install & execute             │                              │
│  Security assessment:                                       │
│  ✅ TLS transport encryption protects the download path     │
│  ✅ the Windows Installer verifies the MSI's integrity      │
│  ❌ no independent digital-signature verification — the key gap │
│  ⚠️ full trust in the content served by the relay           │
│  ⚠️ a compromised relay could inject malicious updates to all clients │
│  💡 best practice: independently verify the update package's code-signing certificate (Authenticode) │
```

## Appendix: What AnyDesk Teaches MediaServo's Feature Planning

### Scenario-Driven Connection Strategy Design

AnyDesk's relay-first strategy, though contrary to industry convention, reveals an important truth: **the optimal connection strategy differs per scenario**. MediaServo should consider three connection modes:

| Mode | Suited scenarios | Strategy |
|------|------------------|----------|
| P2P first | LAN offices, home remote, same-carrier networks | ICE/STUN hole punching first, falling back to the relay after a timeout |
| Relay first | vehicle push, weak networks, ad-hoc tech support, strict corporate firewalls | go straight through TURN/the relay, avoiding punch waits (connection speed first) |
| Smart select | general scenarios | dynamic decisions from historical connection data + current network probes (Phase 2+ ML optimization) |

### DeskRT's Lesson — a GUI-Optimized Preprocessing Layer

DeskRT's commercial success proves that a GUI-content-specific codec has clear market value and technical validity. MediaServo doesn't need to clone DeskRT (proprietary, closed; legal/compliance risk) but can design a "smart preprocessing layer":
- classify frame content before the hardware/software encoders
- Text regions: keep lossless or near-lossless sharpness (unsuited to DCT quantization)
- Window borders/icons: cache reuse (extremely low change frequency; position changes, style doesn't)
- Flat-color regions: compress to a color value + rectangle coordinates (not a pixel matrix; extremely efficient)
- Image/video regions: hand to the standard encoders (H.264/HEVC/AV1)
- Fully compatible with standard encoders — the preprocessing layer emits standard-format frames to the encoder

This layer can land in Phase 2+ as an optional optimization plugin. Its complexity is far below building a codec from scratch, with significant performance upside.

### A Licensing Boundary Warning

AnyDesk's DeskRT codec and AnyNet protocol are fully closed-source (protected by patents and trade secrets). MediaServo must not build an AnyDesk compatibility layer via reverse engineering or protocol mimicry (extreme legal risk). AnyDesk's architectural ideas (process separation, protocol security design, GUI-optimized encoding, scenario-based relay-first thinking) are legitimate learning material, but the implementation must be independent. Especially the unsigned-update lesson — from Phase 0 the architecture doc must mandate update-signature verification; this is a security baseline not to be compromised.

> The analysis above is based on public materials, security-community reverse-engineering reports (published by Daniele Berardinelli, April 2026), official product documentation, and technical information. All technical details are as of July 2026.
