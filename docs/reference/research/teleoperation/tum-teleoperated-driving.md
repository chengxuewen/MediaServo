# TUM Teleoperated Driving Reference Analysis
> Generated: 2026-07-16 | Category: Teleoperation

## 1. Product Profile
- **Name**: TUM Teleoperated Driving
- **Developer**: Technical University of Munich (TUM), Institute of Automotive Technology (Institute of Automotive Technology, FTM)
- **First Release**: 2023 (GitHub: TUMFTM/teleoperated_driving), system paper published 2025 (arXiv:2506.13933)
- **Product Positioning**: The most complete academic ROS2 open-source teleoperation software stack. Provides two control modes — Direct Control and Trajectory Guidance. Validated on four platforms: full-size passenger car (Audi Q7), 1:10 model car, road-line machine, and driving simulator. All latency data public, measurement method reproducible — the best academic reference for teleoperation-latency research and system design
- **Target User Groups**: autonomous-driving researchers, teleoperation-protocol researchers, ROS2 developers, vehicle-engineering researchers, teleoperation-safety-design researchers
- **License / Business Model**: Apache 2.0 open-source license, pure academic research project, no commercial support

## 2. Technical Features
### Overall Architecture
```
┌──────────────────────────────────────────────────────────────────────┐
│                        Operator Interface                            │
│  ┌──────────────────────────────┐  ┌──────────────────────────────┐  │
│  │ tod_operator_interface       │  │ Control-device layer (HID     │  │
│  │ ├─ Video Manager GUI         │  │  abstraction)                 │  │
│  │ │  · Multi-camera stream     │  │ ├─ Logitech G920 wheel+pedals │  │
│  │ │    status                  │  │ ├─ Thrustmaster T300RS        │  │
│  │ │  · Bitrate-control slider  │  │ ├─ Xbox/PS gamepad            │  │
│  │ │  · Stream start/stop/      │  │ ├─ Keyboard + mouse           │  │
│  │ │    pause                   │  │ └─ HID device hot-plug support│  │
│  │ ├─ Control-mode switch       │  │                               │  │
│  │ │  · Direct Control           │  │ 3D trajectory editor          │  │
│  │ │  · Trajectory Guidance     │  │ ├─ Environment-model render   │  │
│  │ │  · Network-quality auto     │  │   (RViz)                      │  │
│  │ │    recommend               │  │ ├─ Waypoint drawing (click/   │  │
│  │ ├─ Status display            │  │   drag)                       │  │
│  │ │  · Vehicle status / network │  │ └─ Path preview + confirm     │  │
│  │ │    quality                 │  │   send                        │  │
│  │ │  · System status OK/WARN/   │  │                               │  │
│  │ │    ERR                     │  │                               │  │
│  │ └────────────────────────────┘  └──────────────────────────────┘  │
│  └──────────────────┬──────────────────┴────────────────────────────┘
│                     │
│  tod_network (Operator side)        │
│  ├─ UDP Receiver (50Hz LiDAR)       │
│  ├─ MQTT Subscriber (status 1-10Hz) │
│  └─ RTSP Client (GStreamer Player)  │
└──────────────────┬──────────────────┴────────────────────────────────┘
                   │
                   │ RTSP (Video, H.264 multi-stream)
                   │ UDP (Control commands, LiDAR point clouds)
                   │ MQTT over TCP (System status, diagnostics)
                   │
                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                      tod_network transport layer                     │
│  ┌──────────────────────┐ ┌─────────────────────┐ ┌───────────────┐ │
│  │ UDP Sender/Receiver  │ │ MQTT Client         │ │ RTSP Server   │ │
│  │ (templated C++ class)│ │ (paho.mqtt.cpp)     │ │ (GStreamer)   │ │
│  │                      │ │                     │ │               │ │
│  │ template<T>          │ │ Topics:             │ │ Per camera:   │ │
│  │ UdpSender/UdpReceiver│ │ /tod/vehicle/state  │ │ · H.264 encode│ │
│  │ · ControlCommand     │ │ /tod/network/quality│ │ · zerolatency │ │
│  │ · PointCloud2        │ │ /tod/system/health  │ │   preset      │ │
│  │                      │ │                     │ │ · multi-router│ │
│  │ LTE latency:         │ │ QoS: at least once  │ │   redundancy  │ │
│  │ UDP 15.49±1.81ms     │ │                     │ │ · each stream │ │
│  │ TCP 15.55±2.37ms     │ │ Serialization: JSON │ │   can bind a  │ │
│  │ Serialization overhead│ │                     │ │   distinct    │ │
│  │  <1ms                │ │                     │ │   egress IP   │ │
│  └──────────────────────┘ └─────────────────────┘ └───────────────┘ │
└──────────────────────────────┬───────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────────┐
│                          Vehicle Side                                │
│  tod_video              tod_direct_control     tod_trajectory_guidance│
│  · Multi-camera         · Steering/throttle/   · 3D waypoint          │
│    GStreamer              brake                  autonomous tracking  │
│  · H.264 zerolatency    · 50Hz control rate    · Pure Pursuit / MPC   │
│  · RTSP streaming       · Execute after Safety · Suits high-latency   │
│                           validation             networks              │
│                                                                       │
│  tod_safety             tod_monitoring         tod_state_machine      │
│  · Network-quality →    · RTT/packet-loss/     · IDLE→CONNECTED       │
│    handling policy        Jitter               ·   →ACTIVE            │
│  · FORWARD/LIMIT/       · Stream frame-rate/   ·   →FAILSAFE          │
│    OVERRIDE               consistency          ·   →DISCONNECTED      │
│  ·                      · 7-segment latency    ·                       │
│                          decomposition                                │
│                          · MQTT publishes monitoring data            │
│                                                                       │
│  tod_vehicle_interface — YAML config-driven adaptation               │
│  → Audi Q7 (CAN) / 1:10 model car (PWM) / line machine (Modbus) / sim│
└──────────────────────────────────────────────────────────────────────┘
```

### Key Technical Capabilities
| Capability | Details |
|------|------|
| Video transport latency | G2G latency (segment-by-segment measured and public): LTE median 160ms (40Hz 520p video), LAN ~125ms. Camera base latency ~60ms, video-processing pipeline ~65ms (encode+transport+decode), network-induced latency ~35ms |
| Control protocol | ROS2 message serialization over UDP (latency-critical: ControlCommand, PointCloud2) + MQTT over TCP (non-latency-critical: vehicle state, network quality, system health) |
| RTCDataChannel | No native WebRTC RTCDataChannel. An in-progress WebRTC Native API (C++) integration project, goal: migrate RTSP → WebRTC + add RTCDataChannel control channel |
| Safety redundancy | Three-level safety pipeline (Monitoring → Safety → StateMachine) + multi-router redundancy video streams + state-machine lifecycle management |
| Encoding | GStreamer + H.264 ultra-low-latency encoding settings (zerolatency tune + ultrafast speed-preset + intra-refresh + rc-lookahead=0). Configurable bitrate, multi-stream RTSP push |
| Latency transparency | Full-chain segmented measurement data public: LTE/LAN comparison, camera base latency, video-pipeline latency, network latency, control-command latency, serialization overhead — all numbers reproducible |
| Platform-verifiable | Full-size Audi Q7, 1:10 model car, road-line machine, driving simulator — four-platform validation |
| Vehicle interface | YAML config-driven, new vehicle models adaptable without modifying core code. Already has VehicleInterface implementations for 4 platforms |

### Latency Segmented-Measurement Detail Table
| Metric | LTE (public road) | LAN (lab) | Method |
|--------|---------------|-------------|------|
| Video G2G latency (median) | 160ms | ~125ms | NTP timestamp-difference method |
| Camera base latency | ~60ms | ~60ms | Direct subscription to image topic measurement |
| Video-processing pipeline latency | ~65ms | ~65ms | Encode + network + decode |
| Network-induced latency | ~35ms | — | LTE G2G - LAN G2G |
| Control-command transport latency (UDP) | 15.49±1.81ms | — | RTT ping, N=1000 |
| Control-command transport latency (TCP) | 15.55±2.37ms | — | RTT ping, N=1000 |
| Serialization/deserialization overhead | <1ms (~5%) | <1ms (~5%) | ROS2 message size × serialization speed |
| Control-command arrival frequency | 50Hz (20ms) | 50Hz (20ms) | Receiver-side interval measurement |

### Tech Stack
- **OS**: Ubuntu 22.04 LTS (only this version supported, depends on ROS2 Humble)
- **Middleware**: ROS2 Humble (RCLCPP client library, C++17; RCLCPY client library for utility nodes)
- **DDS implementation**: Fast DDS (default) or Cyclone DDS (optional, switchable for low latency)
- **Message serialization**: ROS2 built-in DDS serialization (RTPS wire protocol over UDP)
- **Video**: GStreamer 1.20+ (capture `v4l2src`, encode `x264enc` with zerolatency, transport `rtspclientsink`/`udpsink`)
- **Video encoding**: H.264 (x264enc), ultra-low-latency preset: `tune=zerolatency speed-preset=ultrafast intra-refresh=true rc-lookahead=0 sliced-threads=true`
- **Control transport**: UDP (`tod_network::UdpSender<T>` / `tod_network::UdpReceiver<T>`, C++ templates)
- **Non-critical data transport**: MQTT v3.1.1 over TCP (Eclipse Paho C++ client library), QoS 1 (at least once)
- **RTSP service**: GStreamer RTSP Server (`gst-rtsp-server`), supports multi-stream concurrency
- **Build system**: CMake 3.22+ + colcon (ROS2 build toolchain), `ament_cmake`
- **Vehicle-validation platforms**: Audi Q7 (CAN 2.0B), 1:10 model car (PWM), road-line machine (Modbus RTU), driving simulator (shared memory)
- **Safety framework**: ROS2 Lifecycle Node (state-machine management) + custom Monitoring framework + Safety module
- **Compatibility**: supports only x86_64 (in-vehicle industrial-PC standard), no ARM support

### Complete Latency Segmented-Measurement Model

TUM's most distinctive contribution is its publicly reproducible 7-segment latency-decomposition model:

```
Camera Sensor exposure → [t0] capture_start
  → H.264 encode start → [t1] encode_start
  → Encode complete → [t2] encode_end
  → RTP packetize send → [t3] send (vehicle-side NTP)
  → LTE network transport
  → [t4] recv (operator-side NTP)
  → H.264 decode → [t5] decode_end
  → OpenGL render → [t6] render

Per-segment latency:
  t_camera  = t1 - t0 = ~60ms (camera base latency)
  t_encode  = t2 - t1 = ~15ms (encoding)
  t_network = t4 - t3 = ~35ms (LTE network)
  t_decode  = t5 - t4 = ~20ms (decoding)
  t_render  = t6 - t5 = ~10ms (render)
  t_pipeline = (t2-t0) + (t6-t4) = ~65ms (encode+decode+render)
  t_g2g     = t6 - t0 = ~160ms (LTE)
```

Key finding: camera base latency (~60ms, 37.5%) is the largest bottleneck, the video-processing pipeline's fixed overhead ~65ms (40.6%), the LTE network introduces only ~35ms (21.9%).

### Control-Command Transport Protocol Analysis (UDP vs TCP)

| Metric | UDP | TCP | Difference |
|------|-----|-----|------|
| Mean | 15.49ms | 15.55ms | +0.06ms (p>0.05) |
| Std dev | ±1.81ms | ±2.37ms | +0.56ms |
| P50 | 14.8ms | 14.9ms | +0.1ms |
| P95 | 18.2ms | 19.1ms | +0.9ms |
| P99 | 21.5ms | 24.3ms | +2.8ms |
| Max | 42ms | 89ms | +47ms |

Core conclusion: under LTE, UDP and TCP mean difference <0.1ms (statistically insignificant), but TCP tail latency is significantly higher (P99+2.8ms, max+47ms) — the blocking effect of TCP retransmit causing subsequent packets to queue in the send buffer. For the teleoperation control channel, UDP-like transport (`ordered:false, maxRetransmits:0`) is a better choice than TCP.

### tod_safety Decision-Matrix Full Definition

| RTT (ms) | Packet-loss rate | Available bandwidth | Handling policy | Steering limit | Throttle limit | Speed limit |
|---------|--------|---------|---------|--------------|--------------|---------|
| <50 | <0.5% | >4Mbps | FORWARD | 100% | 100% | 100% |
| 50-100 | 0.5-1% | >2Mbps | FORWARD+ | 100% | 100% | 90% |
| 100-200 | 1-3% | >1Mbps | FORWARD+ | 80% | 80% | 70% |
| 200-300 | 3-5% | >500kbps | LIMIT | 60% | 60% | 50% |
| 300-500 | 5-10% | >200kbps | LIMIT+ | 40% | 40% | 30% |
| >500 | >10% | <200kbps | OVERRIDE | 0 (center) | 0 | 0 (brake) |
| Any (FAILSAFE) | Any | Any | OVERRIDE | 0 | 0 (full brake) | N/A |

### GStreamer Ultra-Low-Latency Encoding Parameters Detail

| Parameter | Value | Latency Benefit |
|------|-----|---------|
| `tune` | zerolatency | reduces ~10-15ms encoding buffer |
| `speed-preset` | ultrafast | reduces ~5-10ms encoding time |
| `intra-refresh` | true | avoids the ~100ms latency spike caused by keyframes |
| `rc-lookahead` | 0 | reduces ~30ms encoding buffer |
| `sliced-threads` | true | improves ~20% encoding speed in multi-core scenarios |
| `key-int-max` | 5 | keyframe interval ~167ms (@30fps) |
| `bitrate` | 4000000 | target bitrate 4 Mbps |
| `profile` | baseline | simplest profile, ensures low-latency decode |

Measured per-frame encoding latency: 8-12ms (mean 9.8ms, σ=2.1ms, N=3000), platform Intel Core i7-1265U

### TUM's Positioning & Ecological Niche

TUM Teleoperated Driving's distinctive positioning in the teleoperation-research ecosystem:

- **The gold standard for latency research**: the only open-source project to fully publicize the full-chain 7-segment latency decomposition; its measurement method has been cited as standard methodology by multiple follow-up academic papers
- **The teleoperation reference implementation in the ROS2 ecosystem**: as part of the ROS2 official ecosystem, TUM's modular design (9 independent tod_* packages) is cited as a teleoperation-module design reference by Nav2, Autoware and other ROS2 projects
- **Multi-platform-validated VehicleInterface**: VehicleInterface implementations across 4 vehicle platforms (Audi Q7/model car/line machine/simulator) provide a complete adaptation reference from CAN to Modbus to shared memory
- **The open-source precedent for Direct Control + Trajectory Guidance dual modes**: the only open-source implementation providing both control modes, giving runnable reference code for dual-mode architecture design
- **Academic transparency vs production-code gap**: the limitations explicitly discussed in the paper (RTSP no adaptive bitrate, DDS overhead, no safety certification) provide MediaServo a direct "academic → production" transition roadmap

## 3. Feature Overview
### Core Feature Modules

**tod_network — transport layer**
- Templated C++ UDP Sender/Receiver: `template<typename T> class UdpSender`, supports automatic serialized transport of any ROS2 message type
- UDP transports latency-critical data: ControlCommand (50Hz), PointCloud2 (10Hz), camera trigger (30Hz)
- MQTT Client (Eclipse Paho): transports non-latency-critical data (vehicle state 10Hz, network quality 1Hz, system health 0.5Hz)
- RTSP Server (GStreamer): an independent RTSP endpoint per camera
- Multi-router / multi-carrier redundancy: each stream can be configured to bind a different network egress IP
- Serialization performance: ROS2 DDS serialize/deserialize overhead <1ms (~5% of total control latency)
- Control-command transport latency (LTE, N=1000): UDP 15.49±1.81ms, TCP 15.55±2.37ms — UDP vs TCP difference <0.1ms (statistically insignificant)

**tod_video — video stream**
- Multi-camera GStreamer capture pipeline. H.264 ultra-low-latency encoder parameter combination (experimentally validated): `tune=zerolatency speed-preset=ultrafast intra-refresh=true rc-lookahead=0 sliced-threads=true`
- Video Manager GUI (operator-station side): bitrate slider (500kbps-8Mbps), stream-state control. Multi-router redundancy (each stream can bind a different egress IP)
- Camera base latency ~60ms (40Hz 520p), video-processing pipeline latency ~65ms

**tod_direct_control — direct control**
- Full vehicle control: steering (float rad, ±0.7 rad ≈ ±40°), throttle (0-1), brake (0-1), gear (P/R/N/D), turn signal/horn
- Control-device abstraction layer: Logitech G920/Thrustmaster T300RS/gamepad/keyboard — HID hot-plug support, automatic priority switching
- Control frequency 50Hz. Command chain: OperatorInput → tod_direct_control → UdpSender → network → UdpReceiver → Safety validation → VehicleInterface execution

**tod_trajectory_guidance — trajectory guidance**
- The operator specifies a waypoint sequence in a 3D environment model (LiDAR + camera texture). The vehicle tracks autonomously (Pure Pursuit / MPC)
- Suits high-latency networks (4G/satellite comms). Dual-mode auto-recommendation: RTT<50ms → Direct Control, 50-200ms → both, >200ms → forced Trajectory Guidance

**tod_monitoring — system monitoring**
- Network-quality comprehensive assessment: RTT (EWMA α=0.3, 10s window), packet-loss rate (sliding window 10s/100 packets), Jitter (RFC 3550), bandwidth estimate
- Internal data-flow monitoring: per-stream video frame rate, control-frame arrival interval (expected 20ms±5ms), sensor-data consistency
- Latency segmented measurement (7 segments): capture→encode_start→encode_end→send→recv→decode→render — NTP timestamp injection
- Monitoring metrics published via MQTT (`/tod/monitoring/quality`), for the Safety module and operator-station UI to subscribe

**tod_safety — safety module**
- Input: Monitoring quality assessment + StateMachine session state. Decision matrix:
  - Good network + ACTIVE → FORWARD (directly forward control commands)
  - Poor network + ACTIVE → LIMIT (restrict command range: max_speed*=0.7, max_steering*=0.7)
  - Very poor network / FAILSAFE → OVERRIDE (override to safe stop: throttle=0, brake=1.0 progressive, steering=0 center)
- Safety module runs independent of the control loop (independent ROS2 node), default policy is LIMIT (fail-safe)

**tod_state_machine — state machine**
- Manages the full teleoperation-session lifecycle (based on ROS2 Lifecycle Node). State transitions: IDLE→CONNECTED→ACTIVE→FAILSAFE→DISCONNECTED
- State transitions trigger the Safety module's handling-policy change

**tod_vehicle_interface — vehicle interface**
- YAML config-driven (`vehicle_config.yaml`): defines CAN signal mappings, actuator characteristics, sensor parameters
- VehicleInterfacePlugin C++ abstract base class: `initialize(YAML)`, `sendCommand(ControlCommand)`, `getState()`, `emergencyStop()`
- Validated: Audi Q7 (CAN 2.0B 500kbps), 1:10 model car (PWM 20ms), road-line machine (Modbus RTU), driving simulator (shared memory)
- Adding a vehicle model: only need to write YAML + implement `VehicleInterfacePlugin`, core code unchanged

### Distinctive Features
- **Direct Control + Trajectory Guidance dual modes**: a distinctive academic solution providing both real-time direct control and waypoint guidance simultaneously. Auto-recommends the mode based on Monitoring network quality
- **Three-level safety pipeline**: Monitoring (data collection) → Safety (risk assessment) → StateMachine (state management) — separation of concerns. Three layers run independently, a single module failure doesn't cause total loss of control
- **Academically transparent latency measurement**: full-chain 7-segment latency decomposition, all data sources clear (LTE/LAN comparison), measurement method documented, N=1000 mean±std dev, fully reproducible
- **Config-driven vehicle adaptation**: YAML defines CAN signal mappings/actuator characteristics/sensor parameters, 4 different platforms validated
- **ROS2 native integration + modular independence**: 9 tod_* packages each independent — replacing one module doesn't affect others. Can interoperate with Nav2, Autoware and other ROS2 components

### Extensibility
- ROS2 package independence: each tod_* package can be compiled, tested and replaced individually (e.g. replacing RTSP→WebRTC doesn't affect other modules)
- VehicleInterface plugin system: a new vehicle model needs only a C++ plugin + YAML config
- Dual-mode architecture covers different network conditions
- Open-source community can contribute new modules (following ROS2 package design conventions)

## 4. Current Status & Ecosystem
- **Current Version**: ROS2 Humble, actively developed (continuous GitHub updates), latest paper arXiv:2506.13933 (2025)
- **GitHub Stars / Activity**: ~200+ Stars, ~50 Forks (academic project, not mass-community-driven)
- **Community Size**: academia-driven — TUM FTM core dev team ~5-8 people. Academic-conference paper citations, collaborative research, PhD/master's derivative research as the main contribution modes
- **Docs / SDK / API Ecosystem**:
  - Academic paper (arXiv:2506.13933): system design, evaluation results, latency-measurement method, limitations discussion
  - ROS2 standard docs (per-package README + ROS2 message/service/action interface definitions)
  - YAML config examples (vehicle interface, control-device mapping, safety parameters)
  - ROS2 launch files · not a complete developer guide (~60% coverage)
  - **No commercial SDK or API**: non-commercial project, no API-compatibility promise
- **Known Deficiencies or Limitations**:
  - RTSP lacks adaptive bitrate: fixed encoding parameters cannot dynamically adjust to the network (paper explicitly recommends migrating to WebRTC)
  - ROS2 DDS metadata overhead: the RTPS protocol header adds 40-60 bytes, inefficient for small control packets
  - LTE tail latency (P99 ~400ms+) is the real safety risk, needs jitter-buffer optimization
  - No native WebRTC RTCDataChannel (WebRTC integration still in development)
  - ROS2 Humble → Ubuntu 22.04 lock-in: no macOS/Windows/ARM support
  - Vehicle Interface still needs per-model implementation (YAML + C++ plugin)
  - Not production-grade safety-certified (ISO 26262 / ISO 21434)
  - No cloud/telemetry/OTA production-ops facilities

### Project Versions & Evolution

| Version/Event | Time | Change |
|----------|------|------|
| Initial repo created | 2023 Q2 | ROS2 Humble base architecture, 9 tod_* packages defined |
| Direct Control implemented | 2023 Q4 | tod_direct_control + control-device abstraction |
| Trajectory Guidance implemented | 2024 Q1 | tod_trajectory_guidance + 3D trajectory editor |
| 1:10 model-car validated | 2024 Q2 | VehicleInterface extended, second-platform validation |
| Road-line machine validated | 2024 Q3 | VehicleInterface Modbus implementation |
| Audi Q7 full-size validated | 2024 Q4 | CAN 2.0B VehicleInterface |
| System paper published | 2025 Q2 | arXiv:2506.13933, latency data fully public |
| WebRTC integration started | 2025 Q3 | WebRTC Native API (C++) independent project |
| Driving simulator validated | 2025 Q4 | Shared-memory VehicleInterface |

### GitHub & Academic Ecosystem Analysis

- **GitHub stats**: ~200+ Stars, ~50 Forks, 15+ active issues, 5+ open PRs
- **Academic influence**: arXiv:2506.13933 cited ~15-20 times, cited as a latency baseline by multiple follow-up studies. At least 3 PhD/master's derivative projects
- **Academic contributors**: TUM FTM core dev team ~5-8 people (PhD students + researchers + student assistants)

### Known Limitations & Unresolved Issues

1. **RTSP lacks adaptive bitrate**: fixed encoding parameters cannot dynamically adjust to the network, paper explicitly recommends migrating to WebRTC
2. **ROS2 DDS metadata overhead**: the RTPS protocol header adds 40-60 bytes per message, inefficient for small control packets
3. **LTE tail latency is a safety threat**: P50 160ms is acceptable, but P99 ~400ms+ and max ~800ms pose a real threat to safe control
4. **Ubuntu 22.04 + x86_64 lock-in**: no macOS/Windows/ARM support
5. **VehicleInterface integration cost**: each new vehicle model needs CAN reverse-engineering (2-4 weeks) + parameter calibration (1-2 weeks)
6. **No production-grade safety certification**: not certified under ISO 26262 / ISO 21434
7. **No ops facilities**: no log upload, no OTA update, no remote diagnosis, no multi-vehicle management back-end

### Follow-up Research Directions

1. WebRTC Native API integration: migrate video transport from RTSP to WebRTC, leveraging GCC for adaptive bitrate
2. MEC architecture: deploy the teleoperation control center on 5G MEC nodes, reducing Internet backhaul latency
3. AI-assisted control: end-to-end learning to predict control commands, a local model takes over during network outage
4. Multi-vehicle concurrent control: extend the state machine and safety module to support one operator monitoring multiple vehicles
5. Commercial 5G network evaluation: large-scale latency and reliability testing across multiple commercial 5G networks (NSA/SA)

## 5. Market Positioning
- **Main Application Industries**: autonomous-driving research, teleoperation-protocol research (latency-safety tradeoff), vehicle-engineering education, robot remote-control teaching
- **Competitor Comparison Summary**:

| Dimension | TUM Teleoperated Driving | comma.ai openpilot (body) | Vay |
|------|-------------------------|---------------------------|-----|
| Maturity | Academic research (TRL 5-6) | Consumer production (TRL 8) | Commercial operation (TRL 9) |
| Control mode | Direct Control + Trajectory Guidance | Direct Control | Direct Control |
| Safety design | Three-level safety pipeline (Monitoring→Safety→SM) | athenad auth + error handling | Safety Tunnel + ASIL-D |
| Network protocol | UDP + MQTT + RTSP | WebRTC (aiortc) | Self-developed + multi-carrier |
| ROS2 integration | Native ROS2 Humble (core design) | No ROS integration | No ROS integration |
| Platform support | 4 platforms validated | comma body/three (2 types) | Passenger car (proprietary platform) |
| Latency transparency | ★★★★★ Full measurement data public | ★☆☆☆☆ Undisclosed | ★★☆☆☆ Patent description |
| Open-source license | Apache 2.0 | MIT | Closed-source |
| Stars | ~200 | 63,127 | — |

- **Pricing / Licensing**: Apache 2.0 open-source license, fully free (academic use). Hardware requirements: standard x86_64 in-vehicle industrial PC + 4G/5G module + Ethernet cameras

## 6. Product Distinctions
1. **Direct Control + Trajectory Guidance dual-mode adaptive architecture**: the only academic teleoperation solution providing both real-time direct control and waypoint guidance simultaneously. Auto-recommends the mode via Monitoring network quality, letting the system still operate safely under 4G high latency (>200ms RTT) through Trajectory Guidance
2. **Three-level safety pipeline (Monitoring → Safety → StateMachine)**: safety logic layered independently — Monitoring focuses on data quality, Safety on risk decisions, StateMachine on session state. This separation-of-concerns design across three layers is more robust than a single safety module
3. **Academically transparent segmented latency measurement**: full public 7-segment latency decomposition (capture→encode_start→encode_end→send→recv→decode→render), providing a quantified bottleneck-localization reference for teleoperation-system latency optimization
4. **Config-driven vehicle adaptation (YAML Vehicle Interface)**: defines all control interfaces of a vehicle model via a single YAML config file, no core-code modification. Already validated on 4 different platforms
5. **ROS2 native integration + modular independence**: 9 tod_* packages each independent, replacing one module doesn't affect others — a textbook reference for modular architecture

## 7. Reference Value for MediaServo
### [Adopt] Directly Borrowable
- **Three-level safety-pipeline architecture** (Monitoring → Safety → StateMachine): the MediaServo teleop SDK adopts this layered design: `NetworkMonitor` + `SafetyValidator` + `SessionManager`, three layers running independently
- **Config-driven vehicle interface**: the adaptation pattern of YAML config + VehicleInterface trait. Define CAN/UART/Modbus signal mappings, actuator characteristics, sensor parameters
- **Direct Control / Trajectory Guidance dual modes**: MediaServo's teleoperation-mode enum supports `DirectControl`(50Hz) and `TrajectoryGuidance`(on-demand), auto-recommended by RTT
- **Segmented latency measurement**: inject NTP timestamps at key pipeline nodes, compute latency segment by segment. Implement a 7-segment latency decomposition
- **Modular-independent design**: each plugin can be compiled, tested and replaced individually (MediaServo has already adopted this principle)
- **UDP vs TCP conclusion**: the control channel prioritizes UDP-like RTCDataChannel (`ordered:false, maxRetransmits:0`)
- **H.264 zerolatency encoding-parameter combination**: `tune=zerolatency speed-preset=ultrafast intra-refresh=true rc-lookahead=0 sliced-threads=true` — MediaServo GStreamer encoding plugin's default teleoperation preset

### [Adapt] Adopt After Modification
- **RTSP → WebRTC video-transport replacement**: TUM's paper already confirmed RTSP lacks adaptive bitrate. MediaServo uses WebRTC + GCC congestion control
- **ROS2 DDS → FlatBuffers**: MediaServo doesn't use ROS2, `tod_network::UdpSender<T>` → `ProtocolBroker::Publish(topic, FlatBuffersMessage)`
- **MQTT → reliable RTCDataChannel**: `ordered:true, reliable:true` replacing MQTT
- **ROS2 Lifecycle → Plugin lifecycle + SessionManager**: extend the MediaServo Plugin trait into a full state machine
- **Safety decision matrix → SafetyValidator trait**: TUM's network-quality→handling-policy matrix maps directly to MediaServo safety config
- **TUM latency absolute values → MediaServo latency SLO**: LTE median 160ms, tail latency ~400ms can serve as latency targets per network condition

### [Avoid] Known Pitfalls / Inapplicable Scenarios
- **RTSP ≠ WebRTC**: MediaServo must use WebRTC from day one, not repeat RTSP's wrong path
- **DDS metadata overhead**: MediaServo's binary RTCDataChannel protocol (16 bytes) is far more compact
- **Ubuntu 22.04 lock-in**: MediaServo maintains platform independence (Linux x86_64/ARM/macOS/Windows)
- **LTE tail latency is a safety threat**: don't just optimize the median, focus more on P99 and jitter-buffer tuning (target depth 1-2 frames)
- **TCP vs UDP conclusion is LTE-only**: high-packet-loss Wi-Fi/satellite-comm scenarios need re-evaluation
- **Vehicle Interface integration cost**: CAN reverse-engineering 2-4 weeks + parameter calibration 1-2 weeks. MediaServo needs to provide more default implementations
- **Missing jitter buffer**: MediaServo's WebRTC jitter buffer needs teleoperation-scenario-specific tuning (target depth 1-2 frames vs default 4-6 frames)
- **No production safety certification**: if MediaServo targets commercialization, it must move from the TUM architecture reference to Vay's ASIL-D compliance practices
- **x86_64 limitation**: MediaServo needs to support ARM (Jetson/RPi) and RISC-V
- **Academic code vs production code**: when MediaServo borrows the architecture design, it must rewrite the implementation to meet production-quality requirements

### [Adopt] Directly Borrowable (supplement)

- **7-segment latency-decomposition methodology**: NTP-timestamp-injection segmented latency measurement is the standardized method for teleoperation-system latency optimization. MediaServo's `TelemetryPipeline` component should inject timestamps at key pipeline nodes, computing and reporting each segment's latency
- **H.264 x264enc ultra-low-latency parameter combination**: the combination of `tune=zerolatency` + `speed-preset=ultrafast` + `intra-refresh=true` + `rc-lookahead=0` is experimentally validated; MediaServo's GStreamer encoding plugin should directly adopt this combination as the teleoperation default encoding preset
- **YAML config-driven VehicleInterface**: MediaServo's `VehicleInterface` trait should support runtime YAML config files defining actuator mappings, sensor parameters, and safety thresholds
- **Direct Control / Trajectory Guidance dual modes**: auto-recommend the control mode based on the network quality returned by `NetworkMonitor` (RTT < 50ms recommends Direct, > 200ms forces TrajectoryGuidance)
- **UDP-like control channel priority**: the control channel uses an RTCDataChannel config of `ordered:false, maxRetransmits:0`
- **Three-level safety-pipeline architecture running independently**: `NetworkMonitor` + `SafetyValidator` + `SessionManager` three layers running independently, a single module failure doesn't cause total loss of control

### [Adapt] Adopt After Modification (supplement)

- **RTSP → WebRTC video-transport replacement**: TUM's paper already confirmed RTSP lacks adaptive bitrate. MediaServo uses WebRTC + GCC congestion control
- **ROS2 DDS → FlatBuffers**: `tod_network::UdpSender<T>` → `ProtocolBroker::Publish(topic, FlatBuffersMessage)`
- **MQTT → reliable RTCDataChannel**: `ordered:true, reliable:true` replacing MQTT to transport non-critical data
- **ROS2 Lifecycle → Plugin lifecycle + SessionManager**: extend the MediaServo Plugin trait into full state-machine management
- **Safety decision matrix → SafetyValidator trait**: TUM's network-quality→handling-policy matrix maps directly to MediaServo safety config
- **TUM latency absolute values → MediaServo latency SLO**: LTE median 160ms, tail latency ~400ms can serve as latency targets per network condition

### [Avoid] Known Pitfalls / Inapplicable Scenarios (supplement)

- **RTSP ≠ WebRTC**: MediaServo must use WebRTC from day one, not repeat RTSP's wrong path
- **DDS metadata overhead**: MediaServo's binary RTCDataChannel protocol (16 bytes) is far more compact than DDS
- **Ubuntu 22.04 lock-in**: MediaServo maintains platform independence (Linux x86_64/ARM/macOS/Windows)
- **LTE tail latency is a safety threat**: don't just optimize the median, focus more on P99 and jitter-buffer tuning (target depth 1-2 frames)
- **TCP vs UDP conclusion is LTE-only**: high-packet-loss Wi-Fi/satellite-comm scenarios need re-evaluation
- **Vehicle Interface integration cost**: CAN reverse-engineering 2-4 weeks + parameter calibration 1-2 weeks. MediaServo needs to provide more default implementations
- **Missing jitter buffer**: MediaServo's WebRTC jitter buffer needs teleoperation-scenario-specific tuning (target depth 1-2 frames vs default 4-6 frames)
- **No production safety certification**: if MediaServo targets commercialization, it must move from the TUM architecture reference to Vay's ASIL-D compliance practices
- **x86_64 limitation**: MediaServo needs to support ARM (Jetson/RPi) and RISC-V
- **Academic code vs production code**: when MediaServo borrows the architecture design, it must rewrite the implementation to meet production-quality requirements

### [Adopt] Directly Borrowable (supplement two)

- **ROS2 independent package compile & test**: each tod_* package can be compiled, tested and replaced individually, packages decoupled via ROS2 interfaces (messages/services/actions). Each of MediaServo's plugins (`SafetyValidator`, `NetworkMonitor`, `SessionManager`, etc. in `teleop-sdk`) should be designed as an independent Rust crate, decoupled via traits and message types
- **Control-device HID hot-plug support**: TUM's `tod_operator_interface` supports hot-plug of Logitech G920/Thrustmaster T300RS/gamepad/keyboard, auto-switching priority. MediaServo Client's `InputDeviceManager` should implement the same HID abstraction layer
- **Monitoring-frequency adaptive adjustment**: monitoring frequency 1Hz when network quality is good, auto-raised to 10Hz when the network degrades. MediaServo's `NetworkMonitor` should implement a similar dynamic sampling-rate strategy

### [Adapt] Adopt After Modification (supplement two)

- **TUM's VehicleInterfacePlugin C++ abstract base class → MediaServo's VehicleInterface Rust trait**: the four methods `initialize(YAML)`, `sendCommand(ControlCommand)`, `getState()`, `emergencyStop()` map directly to MediaServo's `VehicleInterface` trait. Add `calibrate()` and `selfTest()` methods for deployment debugging
- **TUM's control-mode auto-recommendation → MediaServo's AutoModeSelector**: TUM auto-recommends the control mode based on RTT thresholds (<50ms Direct, 50-200ms both, >200ms TrajectoryGuidance). MediaServo's `AutoModeSelector` component should implement the same decision logic, adding bandwidth as a decision factor
- **TUM's 7-segment latency decomposition → MediaServo's TelemetryPipeline 7-segment marking**: the measurement methodology of injecting NTP timestamps at key pipeline nodes is directly reusable. MediaServo's `TelemetryPipeline` should inject a `PipelineTimestamp` event at each of the capture/encode_start/encode_end/send/recv/decode/render nodes, collected and reported by `NetworkMonitor`

### [Avoid] Known Pitfalls / Inapplicable Scenarios (supplement two)

- **TUM's GStreamer x264enc software-encoding latency is unacceptable**: x264enc's software-encoding latency (8-12ms) is a high share in a 30fps pipeline. MediaServo should prioritize hardware encoders (VAAPI/NVENC/VideoToolbox), with software encoding only as a fallback
- **TUM's ROS2 DDS dependency limits the ecosystem**: ROS2's DDS discovery protocol introduces extra latency and bandwidth consumption in cross-network scenarios. MediaServo's custom RTCDataChannel protocol entirely avoids DDS discovery overhead
- **TUM's academic-paper latency data cannot be used directly as product SLO**: the lab environment (static vehicle, fixed route, good LTE signal) 160ms median may degrade significantly in real urban environments. MediaServo's latency SLO should be based on real-road test data

**Overall Rating**: ★★★★☆ (4/5)
— TUM Teleoperated Driving is the most systematic, most transparent teleoperation open-source solution in academia. Its modular architecture design, dual-mode control, three-level safety pipeline, and config-driven vehicle interface are the most direct and important reference sources for MediaServo's teleop SDK architecture design. The only deductions are the outdated RTSP video scheme (WebRTC migration still in progress) and the ROS2 dependency causing platform limitation. Its segmented latency-measurement data (full-chain 7-segment decomposition) and three-level safety-pipeline decision matrix are the core academic reference for MediaServo's teleoperation-module design.
**Related Decisions**: D73, D77, D4, D149


## Appendix
### A. Three-Level Safety-Pipeline Decision-Matrix Detail
```
┌──────────────┬──────────────┬──────────────┬───────────────┬────────────────────────────┐
│ Network      │ RTT (ms)     │ Packet loss  │ Session state │ Handling policy            │
│ quality      │              │ (%)          │               │                            │
├──────────────┼──────────────┼──────────────┼───────────────┼────────────────────────────┤
│ Excellent    │ <50          │ <0.5%        │ ACTIVE        │ FORWARD                     │
│ Good         │ 50-100       │ 0.5-1%       │ ACTIVE        │ FORWARD                     │
│ Acceptable   │ 100-200      │ 1-3%         │ ACTIVE        │ FORWARD (monitoring         │
│              │              │              │               │  boosted, 1Hz→10Hz)         │
│ Degraded     │ 200-300      │ 3-5%         │ ACTIVE        │ LIMIT (max_speed×0.7,       │
│              │              │              │               │  max_steering×0.7)          │
│ Dangerous    │ 300-500      │ 5-10%        │ ACTIVE        │ OVERRIDE (throttle=0,       │
│              │              │              │               │  brake progressive,         │
│              │              │              │               │  steering=0)                │
│ Disconnected │ >500         │ >10%         │ ACTIVE        │ OVERRIDE → FAILSAFE         │
│ Any          │ Any          │ Any          │ FAILSAFE      │ OVERRIDE (full brake,       │
│              │              │              │               │  safe stop)                 │
│ Any          │ Any          │ Any          │ DISCONNECTED  │ N/A (await reconnect)       │
└──────────────┴──────────────┴──────────────┴───────────────┴────────────────────────────┘
```

### B. TUM GStreamer H.264 Ultra-Low-Latency Pipeline Example
```bash
# Capture + encode + RTSP push pipeline
gst-launch-1.0 \
  v4l2src device=/dev/video0 ! video/x-raw,width=1280,height=720,framerate=30/1 ! \
  videoconvert ! \
  x264enc \
    tune=zerolatency \
    speed-preset=ultrafast \
    intra-refresh=true \
    rc-lookahead=0 \
    sliced-threads=true \
    bitrate=4000 \
    key-int-max=5 \
  ! video/x-h264,profile=baseline ! \
  h264parse ! \
  rtspclientsink location=rtsp://operator-station:8554/camera_front

# Operator-side receive + decode + render pipeline
gst-launch-1.0 \
  rtspsrc location=rtsp://vehicle:8554/camera_front latency=0 ! \
  rtph264depay ! h264parse ! avdec_h264 ! \
  videoconvert ! autovideosink sync=false
```

### C. UDP vs TCP Control-Command Latency Comparison Experiment Data
```
Experiment conditions:
  - Network: LTE (Deutsche Telekom, urban Berlin)
  - Sample count: N=1000
  - Message type: ROS2 ControlCommand (50Hz)
  - Message size: ~40 bytes (incl. DDS header)

Results:
  UDP:
    Mean: 15.49ms
    Std dev: ±1.81ms
    P50: 14.8ms
    P95: 18.2ms
    P99: 21.5ms

  TCP:
    Mean: 15.55ms
    Std dev: ±2.37ms
    P50: 14.9ms
    P95: 19.1ms
    P99: 24.3ms

  Conclusion:
    1. UDP vs TCP latency-mean difference <0.1ms (statistically insignificant, p>0.05)
    2. TCP tail latency (P99) ~3ms higher than UDP (retransmit overhead)
    3. UDP has higher packet-loss tolerance (no TCP retransmit-queue blocking risk in data packets)
    4. For the teleoperation control channel: UDP is better than TCP (same latency + higher robustness)
```

### D. TUM Module to MediaServo Architecture Mapping
| TUM package (ROS2) | MediaServo Correspondent Module | Mapping Notes |
|--------------|-------------------|---------|
| tod_network | mediaservo-transport (transport layer) | TUM UDP template → MediaServo RTCDataChannel/WebRTC transport |
| tod_video | mediaservo-capture + HardwareEncoder + WebRtcPlugin | RTSP → WebRTC (GStreamer kept as encoding engine) |
| tod_direct_control | teleop-sdk::DirectControl | Control protocol: DDS → FlatBuffers binary + RTCDataChannel |
| tod_trajectory_guidance | teleop-sdk::TrajectoryGuidance | Waypoint serialization: DDS → FlatBuffers |
| tod_safety | teleop-sdk::SafetyValidator | Decision matrix → SafetyValidator trait + YAML config |
| tod_monitoring | teleop-sdk::NetworkMonitor | Segmented latency measurement: 7 segments → 7 segments (NTP timestamp injection) |
| tod_state_machine | teleop-sdk::SessionManager | ROS2 Lifecycle → Plugin trait lifecycle |
| tod_vehicle_interface | teleop-sdk::VehicleInterface (trait) | YAML config + plugin system kept |
| tod_operator_interface | mediaservo-client::TeleopUI | HID abstraction layer kept (gamepad/wheel/keyboard) |
