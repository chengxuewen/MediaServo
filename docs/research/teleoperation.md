# Teleoperation / Remote-Driving Product Research

> Reference material for the MediaServo WebRTC teleoperation module design
> Research date: 2026-07-16

---

## Table of Contents

1. [1. Commercially Operated Products](#1-commercially-operated-products)
   - [1.1 Vay](#11-vay)
   - [1.2 Baidu Apollo 5G Cloud Chauffeur](#12-baidu-apollo-5g-cloud-chauffeur)
   - [1.3 Phantom Auto](#13-phantom-auto)
   - [1.4 Tesla Remote Assistance](#14-tesla-remote-assistance)
2. [2. Open-Source / Research Projects](#2-open-source--research-projects)
   - [2.1 TUM Teleoperated Driving](#21-tum-teleoperated-driving)
   - [2.2 comma.ai openpilot](#22-commaai-openpilot)
   - [2.3 tether-rally](#23-tether-rally)
3. [3. Telecom / Infrastructure Grade](#3-telecom--infrastructure-grade)
   - [3.1 Huawei 5G Remote Driving](#31-huawei-5g-remote-driving)
4. [4. DataChannel Protocol Design Best Practices](#4-datachannel-protocol-design-best-practices)
5. [5. Synthesis: Common Teleoperation Architecture Patterns](#5-synthesis-common-teleoperation-architecture-patterns)
6. [6. The MediaServo Teleoperation Module Design Checklist](#6-the-mediaservo-teleoperation-module-design-checklist)

---

## 1. Commercially Operated Products

### 1.1 Vay

**Overview**: Vay is a remote-driving company in Berlin, Germany, which launched a commercialized "unmanned chauffeur car rental" service in Las Vegas in January 2024 — a remote driver drives the vehicle to the user's location, and the user drives it themselves after getting in. Vay is the first company in Europe to be approved for remote driving without a safety operator on public roads, and has passed third-party TÜV evaluation (ISO 26262 functional safety + ISO/SAE 21434 cybersecurity).

**Video transport solution**: Vay uses a multi-camera 360° surround perception system, transmitting real-time video and audio (ambient sound captured by microphones outside the vehicle) simultaneously over multiple 4G/5G cellular networks. Video transport details are not fully public, but its patent (US 12,537,921 B2) reveals latency-aware video visualization technology — based on measured video latency and vehicle state information, it overlays a vehicle position visualization (including an uncertainty indicator) on the remote driving station screen to help the driver compensate for latency. Technical documentation mentions "high-speed, low-latency data transmission", suggesting the use of a proprietary protocol or a WebRTC variant.

**Control channel**: The remote driving station is equipped with automotive-grade physical controllers such as a steering wheel, pedals, and gear selector, with control signals transmitted over an end-to-end encrypted channel. The vehicle carries a Vay proprietary safety controller (ASIL-D grade) based on a multi-core controller and an aviation/automotive-grade safety operating system. The control channel uses a bidirectional redundant design — there is one Safety Controller each at the Remote Driving Station and the vehicle side, monitoring each other. Patent (US 12,443,182 B2) reveals a "safety tunnel" mechanism: the system computes allowed parameter thresholds such as steering angle, yaw rate, and acceleration in real time from position, map, vehicle and sensor data, and intervenes automatically when a threshold is exceeded.

**Latency metrics**: Vay determined the latency threshold range for safe control through 1000+ private track tests. Specific numbers are not public, but it is mentioned that the system automatically triggers a Minimal Risk Maneuver (MRM) when latency is detected to exceed the safe range, gradually decelerating until a safe stop and activating the hazard flashers. Patents show the system continuously measures video latency and quantifies it into visual cues (changes in shape, size, color, sharpness, etc.).

**Safety mechanisms**:
- **Multi-carrier cellular redundancy**: connected to multiple mobile network operators simultaneously, a single network failure does not affect control
- **Fail-Safe protocol**: automatic safe stop (MRM) when the connection is lost or abnormal
- **Safety Tunnel**: computes and enforces safety bounds on control parameters in real time
- **Dual Safety Controllers** (vehicle side + operator station side): aviation-grade OS, ASIL-D grade
- **ISO 26262 / ISO 21434 certification**: independently evaluated by TÜV
- **Remote driver training system**: multi-stage training, including latency-adaptation training
- **Fatigue management**: dedicated mechanisms for detecting and responding to driver fatigue

**Reusable design patterns**:
- The Safety Tunnel concept — decoupling safety-bound computation from the control loop
- Latency visualization compensation — not hiding latency, but visualizing its impact
- Dual-end Safety Controller peer architecture
- Multi-carrier cellular redundancy (bonding rather than failover)
- Remote-driving data closed loop: data from each drive is used to train the end-to-end AI driving model (RLHF)

**Lessons**:
- Latency compensation cannot rely on network optimization alone; HMI-level visualization is equally critical
- ASIL-D hardware is costly; it must be clear which functions truly require this grade
- The ODD (Operational Design Domain) must take network coverage as its primary constraint

---

### 1.2 Baidu Apollo 5G Cloud Chauffeur

**Overview**: Baidu Apollo 5G Cloud Chauffeur is a key companion service in the Apollo autonomous driving ecosystem, deployed at scale in cities including Beijing, Guangzhou, Changsha, and Cangzhou, providing remote take-over for the "Luobo Kuai Pao" robotaxi. First released in 2020, with an enterprise edition released in 2021. The product is positioned to provide remote redundancy for L4 unmanned driving, and has already landed in mining, port, and logistics scenarios. 5G Cloud Chauffeur has been written into the SAE global remote-driving grade standard.

**Technical architecture**: The system has four working modes — **Synchronous mode** (parallel driving, real-time control), **Warning mode** (single-vehicle binding, targeted monitoring), **Dispatch mode** (fleet-level concurrent request allocation), and **Cloud service authorization mode** (network optimization service). Supported functions include the full set of vehicle control: steering wheel, power pedal, brake pedal, gear, horn, turn signals, etc. The cloud cockpit is equipped with a force-feedback steering wheel and multi-display 360° sensor views. One-to-many supervision is supported (one safety operator monitors multiple vehicles).

**Video transport solution**: Multiple displays show the single vehicle's 360° sensor views (including perception result overlays). Parallel driving mode uses a surround display for the environment model plus the main view and a top-down view. Different modes have different network requirements: parallel driving needs 5G or an equivalent Wi-Fi network; guidance control (trajectory drawing) supports 4G; customer-service mode needs a wired network of at least 100 Mbps. Video coding and transport protocols are not publicly documented in detail.

**Control channel**: The cloud cockpit and the vehicle are connected over 5G, supporting millisecond-level response. Baidu claims to achieve "millisecond-level access" for autonomous vehicles requesting remote assistance. The layered safety design monitors cockpit, network, and vehicle state in real time and applies graded safety handling according to fault or risk level.

**Latency metrics**: "Millisecond-level" response and access. Specific numbers are not public, but it emphasizes that 5G makes parallel driving (real-time control) possible. Without network, the vehicle pulls over to a safe stop.

**Safety mechanisms**:
- **Comprehensive layered safety design**: real-time monitoring of cockpit, network, and vehicle state
- **Graded safety handling**: different responses according to fault or risk level
- **Pull-over when offline**: automatic safe stop on communication loss
- **Remote driver screening and training**: more than 1000 hours of cloud driving training, zero-accident record
- **Fleet-level dispatch**: ensures every vehicle is allocated an available cockpit resource

**Reusable design patterns**:
- Multi-mode graded architecture — switching between synchronous control / trajectory guidance / asynchronous dispatch according to scenario needs
- One-to-many supervision mode — improved operational efficiency
- Layered safety design — decoupling state monitoring from graded response
- Vehicle-cloud linked architecture — three-layer coordination of vehicle, cloud cockpit, and cloud service

**Lessons**:
- Functions are limited in the 3G/4G degraded mode (guidance only, no real-time driving); the function-degradation strategy under each network condition must be explicit
- Fleet-level dispatch is the bottleneck for scaled operation — efficient matching between vehicle requests and cockpit resources is required
- A unified multi-city, multi-scenario deployment standard is needed

---

### 1.3 Phantom Auto

**Overview**: Phantom Auto is a Silicon Valley remote-control solution provider, focused on remote control for logistics and forklift industrial scenarios, later expanding to remote take-over for passenger vehicle autonomous driving. It holds multiple remote-control related patents. It ceased operations in 2023 due to funding problems, but its patents and technical solution remain valuable references for teleoperation architecture design.

**Video transport solution**: Phantom Auto's core patent is a multi-path redundant video transport system. The vehicle side encodes the raw video into multiple copies (each adapted to a different wireless network connection) and sends them simultaneously to the remote control station over multiple parallel wireless channels (e.g. LTE+5G+Wi-Fi). The receiving end compares the same video segment arriving over each channel via a Video Performance Indicator (VPI) unit, taking the earliest arrival and discarding the redundant segments. This is essentially network-layer multi-path bonding, not depending on the reliability of any single network. Another patent (US 2019/0279020) describes "landscape video stream compression" technology — identifying elements in the video such as sky and distant foliage that can be replaced by generic objects, removing them and transmitting only metadata, with the receiving end rebuilding from pre-placed assets, greatly reducing bandwidth demand.

**Control channel**: In the patent, the remote control station can send QoS feedback and control commands to the vehicle via a messaging interface. For example, when the operator executes a small-radius U-turn, the station can send a message asking the vehicle to raise video coding quality. The specific control signal protocol is not public, but the architecture supports a bidirectional auxiliary data channel.

**Latency metrics**: The patent describes strict end-to-end latency constraints — across the entire pipeline from capture to display, processing per frame must not exceed the frame time to ensure real-time transmission. The encoder has a built-in timeout threshold computation and aborts compression when encoding takes too long, preferring lower compression ratio over higher latency. The dual-channel redundant design makes video latency determined by the fastest arriving packet rather than waiting for all packets.

**Safety mechanisms**:
- **Multi-path redundant transmission**: not dependent on a single network, a failure in any channel does not affect control
- **Real-time QoS monitoring**: continuous measurement of round-trip time, transport delay, jitter, and packet loss rate
- **Feedback loop**: the station sends QoS metrics to the vehicle for adaptive coding
- **Safety driver on board** (in actual deployment): remote control always has an in-vehicle safety operator as the last line of defense

**Reusable design patterns**:
- Multi-path redundant transmission with segment-level dedup — using networks in parallel rather than switching between them
- Semantic video compression (object recognition + replacement) — an innovative approach to lowering bandwidth requirements
- Encoder time budget mechanism — ensuring encoding does not become a latency bottleneck
- Control-situation adaptive QoS — dynamically adjusting coding quality according to control difficulty

**Lessons**:
- The company was technically advanced but could not sustain commercially — a pure remote-control solution must be deeply integrated with autonomous driving for market positioning
- Multi-path increases hardware and monthly costs (multiple SIM cards); ROI must be clear
- Semantic compression (object replacement) may lose safety-critical information — strict verification is required

---

### 1.4 Tesla Remote Assistance

**Overview**: Tesla launched its Robotaxi service in Austin in June 2025. According to a February 2026 letter to Senator Markey, Tesla acknowledges the use of Remote Assistance Operators (RAOs). RAOs normally provide route suggestions and directly remote-control the vehicle only as a "last resort" — takeover is only possible when vehicle speed is ≤2 mph, and the top speed after takeover is limited to 10 mph. RAOs may also execute discrete commands such as forward/backward creep, door locks, system restart, and destination modification. Tesla has remote assistance centers in Austin and Palo Alto, redundant with each other.

**Technical architecture**: All communications (video, audio, telemetry, remote commands) are transmitted over authenticated encrypted channels. An RAO must complete hardware-level multi-factor authentication before initiating a remote connection. Remote operations execute within a "carefully validated safety envelope" — including initiation from a stationary state, limits on speed and acceleration, validation of connection quality, and acceptance of commands only after confirming vehicle system health. Some vehicle functions are restricted for RAO use according to the vehicle's specific situation. Tesla emphasizes that its autonomous driving system is designed to perform the dynamic driving task safely without remote assistance in both common and rare scenarios.

**Video transport solution**: Tesla vehicles use a pure vision approach (8 cameras), and RAOs can view the vehicle's sensor feeds to assess the situation. The specific technical solution for video transport is not public, but "authenticated and encrypted channels" are mentioned, possibly based on Tesla's own connectivity infrastructure.

**Control channel**: RAOs do not proactively connect to vehicles; a connection is established only by an internal assignment system when a help request is received. Remote command types fall into two categories: advanced commands (route suggestions, destination modification) and discrete vehicle commands (creep, door locks, system restart). Direct control is the last level of graded escalation.

**Latency metrics**: Tesla did not provide latency data to the Senator. Industry estimates, based on Tesla's own connectivity infrastructure (possibly including Starlink), put latency in the 100-300ms range. For comparison, Waymo publishes 150ms (domestic) / 250ms (overseas) one-way latency.

**Safety mechanisms**:
- **Hardware-level MFA**: RAOs require multi-factor authentication to initiate a remote connection
- **Safety envelope**: speed limits (stationary → takeover, max 10 mph), acceleration limits, connection quality validation
- **Dual-city redundancy**: RAO centers in Austin + Palo Alto
- **Graded intervention**: route suggestions → discrete commands → direct control (escalating step by step)
- **ISO 21434 cybersecurity certification**
- **Strict RAO management**: US driving license ≥3 years, background check, drug testing, zero-tolerance alcohol policy
- **Fatigue management**: at most 5 consecutive working days, at most 7.5 hours per shift, mandatory rest

**Reusable design patterns**:
- Graded intervention system — escalating step by step from suggestions to direct control, each level with an explicit permission boundary
- The safety envelope concept — hard limits on control parameters (speed, acceleration, preconditions)
- Multi-location redundant control centers — not dependent on a single geographic location
- On-demand rather than persistent connection — reducing the attack surface

**Lessons**:
- Direct control as a "last resort" means 99%+ of interventions are in suggestion mode — the technical requirements of these two modes are completely different
- The low speed limit (10 mph) greatly reduces control difficulty and safety risk — MediaServo could consider a control strategy graded by speed level
- Tesla's Robotaxi in practice still relies heavily on humans (in-vehicle safety operator + RAOs); the boundary between "autonomous driving" and "remote control" is very blurry in practice

---

## 2. Open-Source / Research Projects

### 2.1 TUM Teleoperated Driving

**Overview**: A ROS2 open-source teleoperation software stack developed by the Institute for Automotive Engineering at Technical University of Munich (TUM), the most complete open-source teleoperation solution in academia. It supports two control modes: Direct Control (direct control, real-time via steering wheel + pedals) and Trajectory Guidance (trajectory guidance, specifying waypoints in a 3D environment for the vehicle to track autonomously). Validated on a full-size passenger car (Audi Q7), a 1:10 model car, a road line marking machine, and a driving simulator. GitHub repository: `TUMFTM/teleoperated_driving`.

**Technical architecture**: Based on ROS2 Humble + Ubuntu 22.04, highly modular. Packages are grouped as: `tod_network` (network transport), `tod_video` (video streaming), `tod_direct_control` (direct control), `tod_trajectory_guidance` (trajectory guidance), `tod_safety` (safety module), `tod_monitoring` (system monitoring), `tod_vehicle_interface` (vehicle interface), `tod_operator_interface` (operator interface), `tod_state_machine` (state machine). The vehicle interface uses a configuration-driven design — deploying to a new platform requires only a config file.

**Video transport solution**: Uses GStreamer + ROS2 integration, H.264 coding + ultra-low latency settings, transported over RTSP channels. Multi-camera is supported, and the Video Manager GUI can configure bitrate and stream state. Multi-router / multi-carrier redundant streams are supported. Intranet G2G latency is about 104ms (40Hz 520p video, 144Hz gaming display). Camera base latency is about 60ms, and the video processing pipeline adds about 65ms. TUM is running a WebRTC Native API (C++) integration project to leverage WebRTC's Google Congestion Control (GCC) and adaptive bitrate capabilities.

**Control channel**: The `tod_network` package provides templated send-receive pairs for transporting serialized ROS messages. Latency-critical data (control commands, LiDAR) uses UDP transport. Non-critical data (system state) uses MQTT over TCP. Control command transport latency: average 15.55±2.37ms (TCP) / 15.49±1.81ms (UDP) over LTE, with serialization/deserialization overhead <1ms (about 5%). Intranet transport latency is about 125ms (covering camera+encoding+network+rendering end to end).

**Latency metrics**:
| Metric | LTE | LAN | Notes |
|------|-----|-----|------|
| Video G2G latency | 150-200ms (median 160ms) | ~125ms | 40Hz camera |
| Network-introduced latency | ~35ms | — | LTE minus intranet baseline |
| Camera + rendering base latency | ~60ms | ~60ms | Subscribing the image topic directly |
| Video processing pipeline latency | ~65ms | ~65ms | Encode + transport + decode |
| Control command transport latency (UDP) | 15.49±1.81ms | — | ROS2 message |
| Serialization overhead | <1ms | <1ms | ~5% |

**Safety mechanisms**:
- **Monitoring framework**: network quality, latency, and bandwidth assessment + internal data flow availability and consistency monitoring
- **Safety module**: decides whether control commands are forwarded, restricted, or overridden to trigger a safe stop, based on Monitoring and State Machine output
- **State machine**: manages the teleoperation session lifecycle
- **Multi-router redundancy**: copes with bursty packet loss

**Reusable design patterns**:
- Modular decomposition: Network / Video / Control / Safety / Monitoring / StateMachine each independent
- Configuration-driven vehicle interface — adapting to new platforms via YAML configuration rather than code changes
- Dual-mode architecture of Direct Control and Trajectory Guidance
- Monitoring → Safety pipeline — monitoring output directly drives safety decisions

**Lessons**:
- The RTSP solution lacks adaptive bitrate; switching to WebRTC is needed to obtain GCC capabilities
- The latency difference between UDP and TCP for control commands is small (<0.1ms), but UDP tolerates loss better
- LTE median latency is 160ms, but tail latency is the real safety threat — jitter buffer tuning is needed
- The open-source stack still has integration cost — a Vehicle Interface implementation is required

---

### 2.2 comma.ai openpilot

**Overview**: comma.ai's openpilot is an open-source implementation of a consumer-grade L2 driving assistance system, running on comma three/comma body hardware. Its teleoperation capability is mainly used for the **comma body** robot platform (not cars), achieving remote control and live streaming over WebRTC. In 2026, PR #37732 made a major upgrade to the body teleop experience and integrated it into the comma connect app.

**Technical architecture**: Three key daemons — `athenad` (WebSocket connection to the comma cloud server, JSON-RPC remote management), `webrtcd` (WebRTC gateway, handling video streaming and DataChannel bridging), and `uploader.py` (log upload). `webrtcd` uses Python asyncio, bridging openpilot's internal `msgq` (Cereal / Cap'n Proto message system) with the WebRTC PeerConnection. It uses the `aiortc` library or the self-developed `teleoprtc` library.

**Video transport solution**: `webrtcd` reads the hardware-encoded H.264 video stream via `LiveStreamVideoStreamTrack` and packs it directly into a WebRTC video track. After the upgrade: single-encoder architecture, camera switchable via DataChannel parameters; bitrate raised from 1 Mbps to 4 Mbps; GOP reduced from 15 to 5 (shorter keyframe interval to lower latency). Timestamp information is injected into video frame headers for frontend debug statistics. HTTPS + self-signed certificate + CORS support.

**Control channel**: `CerealOutgoingMessageProxy` listens to specific cereal services, converts Cap'n Proto structs to JSON, and broadcasts over DataChannel. `CerealIncomingMessageProxy` receives JSON strings from the browser, parses them, and publishes to the local `msgq`. The frontend sends a `testJoystick` control message over DataChannel every 50ms (WASD keyboard control). Bidirectional audio channel is supported.

**Latency metrics**: No specific end-to-end latency is published in the documentation. Control frames are sent over DataChannel at 50ms intervals (20Hz). Video latency depends on the H.264 hardware encoder + WebRTC pipeline.

**Safety mechanisms**:
- **athenad authentication**: maintains an authenticated WebSocket connection to the comma cloud server via an API Token
- **HTTPS + self-signed certificate**: SSL required for local connections
- **On-demand connection**: initiated through the comma connect app, one-time QR code pairing
- **Graded error handling**: tells the user the reason for connection failure (already occupied by someone else, ignition not on, etc.)

**Reusable design patterns**:
- Cereal (Cap'n Proto) zero-copy serialization — suitable for high-performance telemetry data
- Bidirectional WebRTC DataChannel bridging — transparently extending an internal message bus to the remote
- `webrtcd` + `athenad` separated architecture — decoupling real-time streaming from control management
- Hardware encoder passthrough to WebRTC — avoiding software encoding latency
- Dynamic camera switching (via DataChannel signaling)

**Lessons**:
- GOP=15 is too long for teleoperation (~500ms @ 30fps); lowering to 5 is required to meet control needs
- The single-encoder architecture simplifies resource management, but requires extra camera switching logic
- WebRTC DataChannel JSON serialization is not efficient for high-frequency control (>20Hz) — a binary protocol is better
- Compatibility between `aiortc`'s WebRTC implementation and browsers requires continuous testing

---

### 2.3 tether-rally

**Overview**: tether-rally is an open-source project achieving global remote control of an ARRMA RC car over WebRTC. It uses Raspberry Pi + ESP32 + camera module to deliver a low-latency FPV driving experience. Although it is a toy-grade project, its DataChannel protocol design, stability system, and ESP32 safety mechanisms are directly relevant references for MediaServo's design. GitHub: `roman01la/tether-rally`.

**Technical architecture**:
```
Browser ←WebRTC (Video + DataChannel)→ Raspberry Pi ←WiFi/UDP→ ESP32 (MCP4728 DAC)→ Transmitter → RC Car
```
Cloudflare Workers provide signaling + TURN + static pages; Cloudflare Tunnel punches through the camera WHEP and control relay ports; MediaMTX provides the WebRTC/WHEP video stream service.

**Video transport solution**: Raspberry Pi + Camera Module 3 hardware H.264 encoding, 720p @ 60fps. MediaMTX (open source) provides the WebRTC/WHEP endpoint. Video path Pi → Browser (LAN ~120ms, Internet ~150ms+ G2G). An RTSP rear camera is supported as a PiP picture-in-picture.

**Control channel**:
- WebRTC DataChannel (`ordered: false, maxRetransmits: 0`, UDP-like)
- Custom binary protocol: `seq(uint16 LE) + cmd(uint8) + payload`
- Command types: PING(0x00), CTRL(0x01), PONG(0x02), RACE(0x03), STATUS(0x04), CONFIG(0x05), KICK(0x06), TELEM(0x07), TURBO(0x08), TRACTION(0x09)
- Control frequency: 50Hz (DataChannel → UDP → ESP32)
- Control latency: LAN ~10-15ms, Internet ~30-100ms, control command packets only 7 bytes
- ESP32: FreeRTOS dual core, Core 0 UDP receive, Core 1 200Hz control loop (EMA smoothing + slope limiting)

**Latency metrics**:
| Path | LAN | Internet |
|------|-----|----------|
| Browser → Pi → ESP32 control | ~10-15ms | ~30-100ms |
| Video Pi → Browser G2G | ~120ms | ~150ms+ |
| DataChannel P2P RTT | ~10-15ms | — |

**Safety mechanisms**:
- **ESP32 staged timeouts**: 80ms keep-alive (hold the last command) → 250ms return to center (throttle=0, steering=center)
- **Safety limits enforced on the ESP32** (not in the browser) — unaffected by network attacks
- **HMAC-SHA256 Token authentication**: verified on the Pi side, preventing unauthorized control
- **Auto-reconnect**: exponential backoff reconnect after connection loss
- **ESP32 safety features**: emergency stop, staged timeout, WiFi signal monitoring

**Stability system** (a distinctive engineering practice):
- **Traction Control**: slip detection via IMU + wheel RPM, automatic throttle limiting
- **Stability Control**: oversteer/understeer intervention based on yaw rate
- **ABS**: anti-lock braking, ESC state machine control
- **Hill Hold**: automatic brake hold via pitch angle detection
- **Coast Control**: throttle injection while coasting to prevent rolling backward
- **Surface Adaptation**: dynamic grip estimation, adaptive threshold adjustment
- **Steering Shaper**: speed-based steering limiting + counter-steer assistance

**Reusable design patterns**:
- Minimalist binary protocol (7-byte control packet) — ultimate bandwidth efficiency
- Three-stage timeout safety (hold → neutral → safe stop)
- Safety limits enforced at the edge — never trusting network data
- HMAC token short-lived authorization — simple and effective access control
- Stability system control chain — multiple independent modules chained in series to modify the final throttle/steering output
- The Pi as a bridge rather than a direct controller — layered isolation

**Lessons**:
- ~~WebSocket relay→WebRTC DataChannel~~ the switch reduced control latency from 100-200ms to 10-15ms — direct P2P improves latency significantly
- ~~Control stuttering~~ solved by dual-core separation (receive/control) + EMA smoothing — real-time control needs a dedicated thread
- ESP32 DAC output needs 12-bit resolution — 8-bit PWM is not precise enough
- The 7-byte minimal packet demonstrates that control teleoperation does not need a 1500-byte MTU

---

## 3. Telecom / Infrastructure Grade

### 3.1 Huawei 5G Remote Driving

**Overview**: At MWC Shanghai 2017, Huawei, China Mobile, and SAIC jointly demonstrated the world's first 5G-based consumer vehicle remote driving. The driver was located 30 km away, controlling an SAIC iGS intelligent concept car in real time over a 5G C-band network. This was a milestone demonstration for 5G remote driving, showing the potential of 5G eMBB + URLLC in teleoperation scenarios.

**Technical architecture**: Huawei provided the 5G wireless solution (C-band), China Mobile provided connectivity. Multiple high-definition cameras are installed inside the vehicle, providing a 240° field of view (beyond the 180-190° field of human eyes). Control signals (steering wheel, throttle, brake) are transmitted over the 5G network.

**Video transport solution**: Multiple real-time HD video streams are transported over 5G high-bandwidth channels. Huawei emphasizes that 5G's "ultra-high bandwidth" capability guarantees the HD video connection is "always perfect". Specific coding and transport protocols are not publicly documented in detail.

**Control channel**: Control signals are transported over 5G URLLC. Key data: end-to-end control latency <10ms (5G new radio air interface latency <1ms). This means that when the vehicle is traveling at 30 km/h, the distance from the brake command being issued to actual deceleration is only 8 cm. Latency is introduced mainly by the core network and transport network; 5G NR itself contributes almost no latency.

**Latency metrics**:
| Metric | Value |
|------|------|
| 5G NR air interface latency | <1ms |
| End-to-end control latency | <10ms |
| Brake response distance (@30km/h) | ~8cm |

**Application scenarios**: Huawei positions remote driving for: dangerous environments such as mines and scrap yards, repetitive work such as remote road rollers, centralized remote take-over for autonomous driving fleets, and emergency rescue in disaster areas.

**Follow-up research progress** (2024-2025 academic papers):
- 5G MEC (edge computing) architecture is key to supporting multi-vehicle teleoperation — centralized architecture is limited by Internet backhaul latency
- TDD frame structure selection directly affects uplink video capacity — DDDSU is better suited to teleoperation than DDDDDDDSUU
- Handover in commercial 5G networks is the main source of tail latency for teleoperation
- WebRTC's built-in GCC does not respond quickly enough to rapid 5G channel changes — 5G-aware application design is needed (combining PHY layer metrics)
- Concurrent connections across multiple carriers can reduce tail latency

**Reusable design patterns**:
- Native low latency of 5G NR (<1ms) means the network is no longer the bottleneck; the bottleneck shifts to the encoding/rendering/processing pipeline
- MEC deployment lets the teleoperation control center be placed at the network edge, avoiding Internet backhaul latency
- 240° ultra-wide field of view — beyond human physiological limits
- Network Slicing provides dedicated QoS guarantees for teleoperation

**Lessons**:
- The <10ms latency of a lab/demo environment (static vehicle, fixed route, excellent signal) is hard to reproduce in real urban environments
- The 200-500ms tail latency caused by commercial 5G Handover is a safety-critical problem
- WebRTC's adaptive algorithms need to be tuned for cellular network characteristics
- Uplink bandwidth (video upload) is the biggest bottleneck for scaled teleoperation — 5G networks mainly optimize downlink

---

## 4. DataChannel Protocol Design Best Practices

### 4.1 The Channel-Separation Strategy

Based on RFC 8831 (WebRTC Data Channels) and practice summaries:

| Channel type | ordered | Reliability mode | Typical payload | Frequency |
|----------|---------|-----------|----------|------|
| **Control command channel** | false | maxRetransmits: 0 | Steering/throttle/brake (uint16 × N) | 50-200Hz |
| **Telemetry channel** | false | maxRetransmits: 0 | Speed/GPS/IMU/status bits | 20-50Hz |
| **Reliable command channel** | true | reliable | System restart/mode switch/config push | On demand |
| **File/log channel** | true | reliable | Log upload/firmware update | On demand |
| **Heartbeat channel** | false | maxRetransmits: 0 | Timestamp + sequence number | 1-10Hz |

**Core principles**:
- Ordered reliable channels share a single SCTP stream — one channel blocking does not affect the others
- Real-time control must use `ordered: false` — a stale command arriving is more dangerous
- Heartbeat on an independent channel — not mixed with other data, measuring true network RTT

### 4.2 The Binary Protocol Format

Referring to tether-rally and RFC 8831 best practices:

**Minimal control packet design** (tether-rally style):
```
[seq: uint16 LE] [cmd: uint8] [payload: N bytes]
Total overhead: 3-byte header
```

**Extended control packet design** (recommended for teleoperation):
```
[timestamp: uint32 LE] [seq: uint16 LE] [cmd: uint8] [flags: uint8] [payload: N bytes]
Total overhead: 8-byte header

flags:
  bit 0: emergency (emergency stop)
  bit 1: heartbeat (heartbeat)
  bit 2: ack_requested (acknowledgment required)
  bit 3-7: reserved
```

**Telemetry packet design**:
```
[timestamp: uint32 LE] [seq: uint16 LE]
[steering_angle: int16] [throttle: int16] [brake: uint16]
[speed: uint16] [yaw_rate: int16] [lat_accel: int16]
[gps_lat: int32] [gps_lon: int32] [heading: uint16]
[status_flags: uint16] [battery: uint8] [signal_rssi: int8]
Total overhead: 37 bytes
```

### 4.3 Backpressure Handling

The most critical engineering issue with WebRTC DataChannel:

1. **`dataChannel.send()` is non-blocking** — data piles up locally, with no built-in backpressure
2. **Set `bufferedAmountLowThreshold`** (64KB recommended)
3. **Listen for the `onbufferedamountlow` event** — resume sending when the buffer drops below the threshold
4. **Send-loop pattern**:
   ```
   while (dataChannel.bufferedAmount > MAX_BUFFERED) {
       await onBufferedAmountLow;
   }
   dataChannel.send(data);
   ```
5. **Timeout protection**: the await needs a 30s timeout to prevent a deadlock caused by the remote side stopping consumption
6. **Message size limit**: a single message ≤ 16KB (browser compatibility ceiling); chunk large data yourself

### 4.4 Key SCTP Internals

- SCTP has a heartbeat mechanism (HEARTBEAT chunk); an application-layer heartbeat is still necessary (to measure RTT)
- Each SCTP message has a 16-bit Stream Sequence Number, which wraps around after 65535
- The FORWARD TSN chunk allows skipping stale data — `maxRetransmits: 0` exploits this mechanism
- SCTP's Nagle algorithm should be disabled (low-latency scenarios)
- Message fragmentation: handled automatically by SCTP, but large data blocks subsequent messages on the same channel

---

## 5. Synthesis: Common Teleoperation Architecture Patterns

### 5.1 Architecture Layering

All surveyed systems can be summarized into the following layered architecture:

```
┌─────────────────────────────────────────────┐
│                Operator Station              │
│  HMI | Input devices | Video render | Audio out    │
├─────────────────────────────────────────────┤
│                 Network transport layer       │
│  WebRTC | SRT | RTSP | Proprietary protocol  │
│  Multi-channel: video/control/telemetry/audio    │
├─────────────────────────────────────────────┤
│                  Safety layer                  │
│  Safety Controller | MRM | Safety Tunnel     │
│  Heartbeat/watchdog | Staged timeouts | Redundant verification │
├─────────────────────────────────────────────┤
│                    Vehicle                    │
│  Sensor capture | Video encode | Command execution | Status reporting │
└─────────────────────────────────────────────┘
```

### 5.2 Common Security Design Patterns

| Pattern | Description | Adopters |
|------|------|--------|
| **Staged timeout response** | Short timeout holds → medium timeout returns to center → long timeout safe stop | tether-rally, Vay |
| **Safety Tunnel** | Hard bounds on control parameters, intercepting when exceeded | Vay, Tesla |
| **Safety limits enforced at the edge** | Safety logic on the vehicle side, the network is not trusted | tether-rally, Vay |
| **Multi-channel redundancy** | Multi-carrier / multi-path parallel transmission | Phantom Auto, Vay |
| **Safety envelope** | Preconditions validated before control | Tesla |
| **Dual-end Safety Controller** | Operator station + vehicle side peer safety monitoring | Vay |
| **MRM (Minimal Risk Maneuver)** | The system comes to a safe stop autonomously | Vay, TUM |

### 5.3 Latency Benchmarks

| Scenario | Control latency | Video G2G latency | Total end-to-end |
|------|---------|-------------|---------|
| LAN/WiFi (ideal) | <15ms | ~100ms | ~120ms |
| 4G LTE | 15-40ms | 150-250ms | 200-300ms |
| 5G NSA (current commercial) | 10-30ms | 100-200ms | 150-250ms |
| 5G SA + MEC | <10ms | <100ms | <100ms |
| 5G NR theoretical | <1ms | <50ms | <50ms |

**Safe control thresholds** (industry consensus):
- Low-speed scenarios (<20 km/h): total latency <300ms is acceptable
- Urban roads (20-50 km/h): total latency <200ms is ideal
- High-speed scenarios (>50 km/h): total latency <100ms is required

---

## 6. The MediaServo Teleoperation Module Design Checklist

Based on the research above, the MediaServo teleoperation module should consider the following design elements:

### 6.1 Video Transport

- [x] WebRTC as the preferred transport protocol (built-in GCC, adaptive bitrate, P2P direct)
- [ ] Synchronized multi-camera capture and timestamp alignment (NTP/PTP clock sync)
- [ ] Hardware encoder passthrough (H.264/H.265), avoiding software encoding latency
- [ ] GOP configuration ≤5 (keyframe interval ≤200ms @ 30fps)
- [ ] Adaptive multi-stream (dynamically switching resolution/bitrate according to network quality)
- [ ] Optional Multi-path transport (parallel multi-SIM, Phantom Auto pattern)
- [ ] Configurable jitter buffer depth (latency vs smoothness trade-off)

### 6.2 The Control Channel

- [ ] Multi-DataChannel channel separation (control/telemetry/heartbeat/reliable commands each independent)
- [ ] Real-time control channel: `ordered: false, maxRetransmits: 0`
- [ ] Reliable command channel: `ordered: true, reliable`
- [ ] Compact binary protocol (≤16-byte header + payload)
- [ ] Timestamp + sequence number (NTP synced, loss/out-of-order detection)
- [ ] Backpressure management (bufferedAmount monitoring + threshold pause)
- [ ] Adaptive control frequency (dynamically adjusting send frequency according to RTT)
- [ ] Dedicated heartbeat channel — precisely measuring one-way latency and RTT

### 6.3 Security Mechanisms

- [ ] **Staged timeout response**:
  - L1 (50ms without packets): hold the last command
  - L2 (200ms without packets): steering wheel returns to center, throttle to zero
  - L3 (500ms without packets): trigger emergency braking
  - L4 (2000ms without connection): safe stop + hazard flashers
- [ ] **Safety Tunnel**: define the safety bounds of control parameters (max_steering_angle, max_acceleration, max_speed)
- [ ] **Safety limits enforced at the edge**: the vehicle validates all commands, never trusting network data
- [ ] **Watchdog timer**: hardware-level watchdog, independent of the main controller
- [ ] **Heartbeat monitoring**: independent DataChannel channel, ≤2Hz
- [ ] **Identity authentication**: Token/certificate mutual authentication
- [ ] **End-to-end encryption**: DTLS-SRTP + data channel encryption
- [ ] **Operation log**: recording of all control commands + state changes

### 6.4 Monitoring & Ops

- [ ] Real-time network quality monitoring (RTT, packet loss rate, jitter, bandwidth)
- [ ] Video stream quality monitoring (frame rate, bitrate, G2G latency)
- [ ] System resource monitoring (CPU/GPU/memory)
- [ ] Prometheus + Grafana dashboards
- [ ] Segment-by-segment latency measurement (encoding/network/decoding/rendering)

### 6.5 Reusable Open-Source Assets

| Project | Reusable content | Reuse method |
|------|----------|---------|
| TUM Teleoperated Driving | ROS2 modular architecture, safety module design | Architecture reference |
| comma.ai openpilot | WebRTC + Cereal bridging pattern, athenad management channel | Design patterns |
| tether-rally | DataChannel binary protocol, staged timeouts, ESP32 safety limits | Direct protocol-format reference |
| MediaMTX | WebRTC/WHEP video streaming server | Can be integrated directly |
| aiortc / teleoprtc | Python WebRTC implementation | Python prototype validation |

---

## References

1. Vay Technology — official website technical documentation and patents US 12,443,182 B2 / US 12,537,921 B2
2. Baidu Apollo 5G Cloud Chauffeur — apollo.auto product page and China Daily coverage
3. Phantom Auto — patents US 2020/0351322 A1 / US 2019/0279020 A1
4. Tesla Remote Assistance — 2026 Senator Markey letter (markey.senate.gov)
5. TUM Teleoperated Driving — arXiv:2506.13933 / GitHub: TUMFTM/teleoperated_driving
6. comma.ai openpilot — GitHub: commaai/openpilot / DeepWiki docs
7. tether-rally — GitHub: roman01la/tether-rally
8. Huawei 5G Remote Driving — Huawei news center 2017
9. RFC 8831 — WebRTC Data Channels (IETF)
10. WebRTC for the Curious — webrtcforthecurious.com
11. AWS Teleoperations Blog — aws.amazon.com/blogs/industries
12. Frontiers in Future Transportation — 4G/5G remote driving evaluation
13. arXiv:2606.17654 — 5G network architecture and teleoperation scaling
14. arXiv:2507.20438 — teleoperation feasibility study on commercial 5G networks

## Related Decisions

| Research finding | Related decision |
|---------|---------|
| Host cross-platform (D77) | D73, D77 |
| P2P fallback to relay (ICE) | D15, D96 |
| DataChannel control commands | D4, D149 |
| Independent emergency-stop channel | D117 |
| Multi-SIM bonding (Vay) | Phase 3 deferred |
| Management plane / data plane separation (comma.ai) | D62-D63 |
