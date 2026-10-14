# tether-rally Reference Analysis
> Generated: 2026-07-16 | Category: Teleoperation

## 1. Product Profile
- **Name**: tether-rally
- **Developer**: roman01la (personal open-source project)
- **First Release**: 2024 (GitHub repository created)
- **Product Positioning**: A WebRTC-based ARRMA RC remote-control car global remote-driving system, providing a low-latency FPV driving experience. Though a toy-grade project, its RTCDataChannel protocol design, tiered safety mechanism, and real-time control architecture have direct reference value for production-grade teleoperation systems
- **Target User Groups**: RC car enthusiasts, teleoperation-protocol researchers, WebRTC RTCDataChannel practitioners
- **License / Business Model**: Open source (GitHub: roman01la/tether-rally), MIT license

## 2. Technical Features
### Overall Architecture
```
┌──────────────────────────────────────────────────────────────────┐
│                        Cloudflare Workers                        │
│              signaling + TURN + static-page hosting              │
│              Cloudflare Tunnel NAT traversal                      │
└─────────────┬──────────────────────────────────────┬─────────────┘
              │ WebRTC (Video + RTCDataChannel)         │ Cloudflare Tunnel
              │ P2P direct (LAN RTT ~10-15ms)          │
              ▼                                      ▼
┌──────────────────────────┐          ┌─────────────────────────────┐
│    Browser (operator end) │          │   Raspberry Pi 4/5 (bridge) │
│  ┌────────────────────┐  │          │  ┌─────────────────────────┐ │
│  │ Video render        │  │          │  │ MediaMTX video service   │ │
│  │ 720p @ 60fps        │  │          │  │  · WebRTC/WHEP endpoint  │ │
│  │ Front main camera +  │  │          │  │  · H.264 forwarding      │ │
│  │ rear RTSP PiP        │  │          │  │    (no transcode)        │ │
│  ├────────────────────┤  │          │  │  · RTSP → WebRTC bridge  │ │
│  │ RTCDataChannel       │  │          │  ├─────────────────────────┤ │
│  │ ordered:false        │  │          │  │ HMAC-SHA256 Token verify │ │
│  │ maxRetransmits:0     │  │          │  │  · short-lived           │ │
│  │ custom binary        │  │          │  │  · verified once at      │ │
│  │  protocol            │  │          │  │    connect               │ │
│  │ multi-channel:       │  │          │  │  · blocks unauthorized   │ │
│  │  · control (50Hz)    │  │          │  │    control               │ │
│  │  · telemetry (10Hz)  │  │          │  ├─────────────────────────┤ │
│  │  · heartbeat (2Hz)   │  │          │  │ WiFi/UDP → ESP32        │ │
│  └────────────────────┘  │          │  │  port 54321              │ │
│  ┌────────────────────┐  │          │  │  wireless LAN (2.4/5GHz) │ │
│  │ Auto-reconnect      │  │          │  └──────────┬──────────────┘ │
│  │ · exponential       │  │          └──────────────┼──────────────┘
│  │   backoff           │  │                         │ WiFi/UDP
│  │ · reconnect signal- │  │    ┌─────────────────────────────────────┐
│  │   ing first, then   │  │    │   ESP32 (safe execution & control)  │
│  │   WebRTC            │  │    │   ┌──────────────────────────────┐ │
│  └────────────────────┘  │    │   │ FreeRTOS dual-core arch       │ │
└──────────────────────────┘    │   │ Core 0 (UDP receive thread)   │ │
                                │   │  · 50Hz control-frame recv    │ │
                                │   │  · command parse              │ │
                                │   │    (seq+cmd+payload)          │ │
                                │   │  · seq out-of-order/loss det. │ │
                                │   ├──────────────────────────────┤ │
                                │   │ Core 1 (control loop 200Hz)   │ │
                                │   │  · EMA exponentially-weighted │ │
                                │   │    moving average             │ │
                                │   │  · rate limit (max_rate)      │ │
                                │   │  · tiered-timeout safety      │ │
                                │   │  · stability control chain    │ │
                                │   ├──────────────────────────────┤ │
                                │   │ Sensor fusion                 │ │
                                │   │  · BNO055 IMU (9-DOF)         │ │
                                │   │  · GPS module (NEO-6M)        │ │
                                │   │  · wheel RPM sensor (IR)      │ │
                                │   ├──────────────────────────────┤ │
                                │   │ Actuator output               │ │
                                │   │  · MCP4728 12-bit DAC ×2      │ │
                                │   │  · steering channel (0-4095)  │ │
                                │   │  · throttle channel (0-4095)  │ │
                                │   │  · GPIO headlight control     │ │
                                │   └──────────────────────────────┘ │
                                └──────────────┬──────────────────────┘
                                               │ DAC analog signal
                                               ▼
                                ┌─────────────────────────────────────┐
                                │  ARRMA remote (Trainer Port)        │
                                │  → RC Car (ARRMA Granite/Fury)     │
                                └─────────────────────────────────────┘
```

### Key Technical Capabilities
| Capability | Details |
|------|------|
| Video transport latency | LAN ~120ms G2G (capture→encode→network→decode→render), Internet ~150ms+; 720p@60fps H.264 hardware encoding |
| Control protocol | Custom minimal binary protocol: `seq(uint16 LE) + cmd(uint8) + payload`, control packet only 7 bytes (3-byte header + 4-byte payload) |
| RTCDataChannel | `ordered: false, maxRetransmits: 0` (pure UDP semantics), control frequency 50Hz (20ms interval), 4 independent RTCDataChannels |
| Safety redundancy | ESP32-side three-level timeout: L1(80ms hold) → L2(250ms center) → L3(long-term no-packet safe stop); WiFi signal monitoring |
| Encoding | Raspberry Pi Camera Module 3 hardware H.264 encoding, zero CPU overhead; GStreamer/MediaMTX muxing |
| Multi-channel separation | control(50Hz)·telemetry(10Hz)·heartbeat(2Hz)·reliable-command(on-demand) — four independent RTCDataChannels |
| Authentication | HMAC-SHA256 short-lived Token, verified on the Pi, blocks unauthorized control |
| Buffer management | bufferedAmount monitoring + onbufferedamountlow event-driven backpressure handling |

### WebRTC RTCDataChannel Configuration Details
```
Control channel (dc-ctrl):
  ordered: false          ← disable ordering, stale commands dropped on arrival
  maxRetransmits: 0       ← no retransmit, a stale command is more dangerous than a lost one
  protocol: "ctrl"        ← sub-protocol identifier
  Send frequency: 50Hz (every 20ms)
  Packet size: 7-11 bytes

Telemetry channel (dc-telem):
  ordered: false          ← newest telemetry wins
  maxRetransmits: 0
  protocol: "telem"
  Send frequency: 10Hz
  Packet size: 37-64 bytes

Heartbeat channel (dc-hb):
  ordered: false
  maxRetransmits: 0
  protocol: "heartbeat"
  Send frequency: 2Hz
  Packet size: 5 bytes (only seq + cmd)

Reliable channel (dc-reliable):
  ordered: true           ← ordering guarantee
  reliable: true          ← reliable transport
  protocol: "reliable"
  Send frequency: on-demand (RACE mode switch / config push)
```

### Tech Stack
- **Frontend**: HTML/CSS/JavaScript (browser-native WebRTC API, no framework)
- **Bridge OS**: Raspberry Pi OS (Bookworm), Node.js v20+
- **Bridge software**: `wrtc` / browser API (WebRTC), `node-fetch` (signaling)
- **Video service**: MediaMTX 1.x (open-source Go WebRTC/WHEP server)
- **Edge control**: ESP32-WROOM-32E, FreeRTOS dual-core, Arduino framework C++
- **Cloud service**: Cloudflare Workers (JS), Cloudflare TURN, Cloudflare Tunnel
- **DAC**: MCP4728 12-bit I2C DAC ×2 (steering + throttle, 4096-step resolution)
- **Sensors**: BNO055 9-DOF IMU (accelerometer, gyroscope, magnetometer), NEO-6M GPS, IR RPM sensor
- **Authentication**: HMAC-SHA256, Python script generates Token (configurable validity)
- **Video encoding**: H.264 hardware encoding (Pi Camera Module 3, Broadcom VideoCore VI ISP)
- **Audio**: no audio channel (design tradeoff: FPV driving needs no voice comms)

### Latency Data & Performance Metrics

| Metric | LAN Environment | Internet (4G hotspot) | Notes |
|------|---------|------------------|------|
| Video G2G latency | ~120ms | ~150ms+ | 720p@60fps H.264 hardware encoding, full pipeline |
| RTCDataChannel P2P RTT | ~10-15ms | ~30-100ms | Depends on TURN relay distance and NAT type |
| Control-command full-pipeline latency | ~15-20ms | ~50-120ms | Browser → Pi → WiFi UDP → ESP32 |
| ESP32 control loop | 5ms (200Hz) | 5ms (200Hz) | Core 1 fixed rate, unaffected by network |
| Camera encode latency | ~8-12ms | ~8-12ms | Pi Camera Module 3 hardware H.264 encoder |
| Browser decode latency | ~5-8ms | ~5-8ms | Hardware-accelerated decode (VideoToolbox/VAAPI) |
| Signaling connect setup time | ~1-2s | ~3-5s | Cloudflare Workers signaling + ICE hole-punching |
| HMAC auth latency | ~50ms | ~50-200ms | Token generation and verification |

### Binary Control Protocol Field Details

tether-rally's custom binary protocol defines 10 command types in total, covering control, heartbeat, telemetry, config, and management for all scenarios:

**CTRL (0x01) control command — 7 bytes:**
```
Byte 0-1: seq (uint16 LE)    — sequence number, receiver detects loss/out-of-order
Byte 2:   cmd (uint8)        — 0x01 = CTRL
Byte 3-4: steering (int16 LE) — steering value, range -4095 ~ +4095, 0 is centered
Byte 5-6: throttle (uint16 LE) — throttle value, range 0 ~ 4095, 0 is full brake
```

**TELEM (0x07) telemetry report — 37 bytes:**
```
Byte 0-1:  seq (uint16 LE)         — sequence number
Byte 2:    cmd = 0x07              — command identifier
Byte 3-4:  speed (uint16 LE)       — km/h × 10 (0-6553.5 km/h)
Byte 5-6:  rpm (uint16 LE)         — wheel RPM (0-65535 RPM)
Byte 7-10: lat (int32 LE)          — latitude × 10^7
Byte 11-14: lon (int32 LE)         — longitude × 10^7
Byte 15-16: heading (uint16 LE)    — heading angle 0.01° (0-359.99°)
Byte 17-18: accel_x (int16 LE)     — acceleration m/s² × 100
Byte 19-20: accel_y (int16 LE)
Byte 21-22: accel_z (int16 LE)
Byte 23-24: gyro_x (int16 LE)      — angular velocity °/s × 10
Byte 25-26: gyro_y (int16 LE)
Byte 27-28: gyro_z (int16 LE)
Byte 29-30: battery (uint16 LE)    — battery voltage mV
Byte 31-32: rssi (int16 LE)        — WiFi signal strength dBm
Byte 33-34: status_flags (uint16 LE) — bitmask
Byte 35-36: crc16 (uint16 LE)      — CRC-16/XMODEM checksum
```

**CONFIG (0x05) config command fields:**
| Parameter | Type | Range | Notes |
|------|------|------|------|
| max_throttle | uint16 | 0-4095 | Throttle upper limit (default 30%) |
| turbo_throttle | uint16 | 0-4095 | Turbo-mode throttle upper limit (default 65%) |
| smoothing_alpha | uint8 | 0-100 | EMA smoothing factor × 100 |
| rate_limit | uint16 | 0-4095 | Rate limit (max change per 5ms) |
| traction_enabled | uint8 | 0/1 | Traction-control switch |
| abs_enabled | uint8 | 0/1 | ABS switch |
| timeout_l1_ms | uint16 | 0-1000 | Level-1 timeout threshold (hold, default 80ms) |
| timeout_l2_ms | uint16 | 0-1000 | Level-2 timeout threshold (center, default 250ms) |
| timeout_l3_ms | uint16 | 0-5000 | Level-3 timeout threshold (safe stop, default 2000ms) |

### ESP32 Data-Flow Throughput Analysis

| Connection Type | Throughput | Control Frequency | Data Direction |
|---------|--------|---------|---------|
| RTCDataChannel | ~1-2 KB/s | 50Hz TX | Browser → Pi |
| RTCDataChannel | ~300-500 B/s | 10Hz TX | Pi → Browser |
| WiFi UDP | ~350 B/s | 50Hz TX | Pi → ESP32 |
| WiFi UDP | ~370 B/s | 10Hz TX | ESP32 → Pi |
| DAC output | — | 200Hz | ESP32 → Servo |

Total bandwidth consumption of the control channel is very low: 50Hz CTRL commands are only 350 B/s (7B × 50Hz), 10Hz TELEM reports 370 B/s (37B × 10Hz). If JSON replaced binary, CTRL commands would balloon to 80-120 bytes (~5 KB/s), TELEM to 150-200 bytes (~2 KB/s), total bandwidth demand rising roughly 8-10×.

### Video-Encoding Pipeline Details

```
Pi Camera Module 3 → Broadcom ISP → H.264 hardware encoder → NAL packing → MediaMTX WHEP
  → WebRTC SDP negotiation → ICE hole-punching → DTLS-SRTP encryption → RTP packetization → network → browser decode → render

Encoding params: 1280×720 (720p), 60fps, VideoCore VI H.264, VBR, Profile High, GOP ~30

Latency breakdown:
  Sensor exposure + ISP processing: ~3-4ms
  H.264 encoding: ~4-6ms
  NAL + RTP packing: ~1-2ms
  Network transport (LAN): ~1ms
  jitter buffer: ~33-50ms (@60fps)
  Decode: ~5-8ms
  Render compositing: ~1ms
  ─────────────────────────
  Total G2G latency: ~120-150ms
```

## 3. Feature Overview
### Core Feature Modules

**Video-transport module**
- 720p @ 60fps low-latency FPV video stream (primary visual)
- Pi Camera Module 3 hardware H.264 encoding
- Supports an RTSP rear camera as PiP (Picture-in-Picture)
- MediaMTX provides WebRTC/WHEP endpoint
- Video path: Pi Camera → Broadcom ISP → H.264 hardware encoder → MediaMTX → WebRTC → Browser
- PiP path: rear RTSP camera → MediaMTX bridge → composited with the main stream (front-end canvas) → display

**Control-transport module**
- WebRTC RTCDataChannel bidirectional comm (4 independent DCs)
- Custom minimal binary control protocol (7-11 bytes)
- 10 command types:
  - PING(0x00): heartbeat request
  - CTRL(0x01): steering + throttle + mode bits (7 bytes)
  - PONG(0x02): heartbeat response (with RTT computation)
  - RACE(0x03): mode switch (practice/race)
  - STATUS(0x04): ESP32 status report
  - CONFIG(0x05): runtime parameter config
  - KICK(0x06): ESP32 soft restart
  - TELEM(0x07): telemetry data report
  - TURBO(0x08): Turbo-mode switch
  - TRACTION(0x09): traction-control switch
- Control frequency 50Hz (RTCDataChannel → UDP → ESP32 full chain)
- Sequence-number mechanism: uint16 LE (0-65535 wraparound), receiver detects loss/out-of-order

**ESP32 edge-execution layer**
- FreeRTOS dual-core architecture
- Core 0 (receive thread): 50Hz UDP receive + command parse + sequence-number detection
- Core 1 (control loop): 200Hz running frequency
  - EMA exponential smoothing (smoothing factor α=0.3)
  - Rate limit (max change rate rate_limit per 5ms tick)
  - Tiered timeout response
  - Stability control chain executed serially
- 12-bit MCP4728 DAC output (4096-step precision vs 8-bit PWM 256 steps)
- Sensor-data fusion (IMU + RPM + GPS used for stability control)

**Telemetry system**
- GPS position tracking (lat/lon + heading angle + velocity vector)
- Real-time speed and wheel-RPM monitoring
- IMU data (3-axis accel, 3-axis gyro, 3-axis euler/quaternion attitude)
- Compass direction (magnetometer fusion)
- Battery-voltage monitoring (via ADC voltage-divider circuit)
- WiFi signal RSSI

**Race-management module**
- Lap-time timing (GPS-triggered or manual-triggered)
- Split times
- Race-state management (ready / racing / finished / DNF)

### Distinctive Features

**Stability-control system (a distinctive engineering practice)**
1. **Traction Control**: IMU + wheel-RPM slip detection; when wheel speed > ground speed × 1.15 is detected, throttle output is automatically limited. The slip threshold and intervention strength are remotely adjustable via the CONFIG command
2. **Stability Control**: oversteer/understeer detection based on yaw rate. Computes expected yaw rate (steering angle × speed / wheelbase) vs actual yaw rate (IMU gyroscope); intervenes when the deviation exceeds the threshold — oversteer: reduce steering angle + add opposite throttle; understeer: increase steering angle + reduce throttle
3. **ABS (anti-lock braking)**: ESC state-machine control (IDLE → BRAKING → ABS_ACTIVE → RELEASED), pulsed braking when wheel-RPM detects lock-up
4. **Hill Hold**: auto-applies brake hold when IMU pitch angle > 5°, releases when throttle exceeds 10%
5. **Coast Control**: during coast (throttle=0), when reverse acceleration is detected (IMU), auto-injects a small throttle to prevent rollback
6. **Surface Adaptation**: continuously monitors wheel-slip ratio, dynamically estimates road-surface grip coefficient μ, adaptively adjusting the Traction-Control and ABS trigger thresholds
7. **Steering Shaper**: the higher the speed, the smaller the max allowed steering angle (∝ 1/v²). Reduces steering sensitivity at high speed to prevent rollover. Auto-applies counter-steer assist when oversteer is detected

**Driving-assist features**
- **Turbo mode**: throttle limit raised from 30% to 65%, suited for straight-acceleration sections
- **Headlight control**: GPIO → MOSFET drives high-power LED
- **Throttle limiting**: enforced on the vehicle side (ESP32), not trusting browser commands. ESP32's throttle output upper limit is hardcoded to config.max_throttle (default 30%, Turbo 65%)

### Extensibility
- Modular hardware architecture (Pi + ESP32 + sensors separated, each component connected via standard protocols)
- Binary protocol extensible: command codes 0x00-0x09 use only 10 types, 246 command-space entries available
- Video sources extensible: already supports main camera + rear RTSP camera PiP, more RTSP sources can be added
- Driving-assist system uses independent module design, each module can be enabled/disabled individually (via CONFIG command)
- Sensors replaceable: IMU/GPS/RPM sensors can be swapped for different models via I2C/UART standard interfaces
- Community-contribution friendly: clear hardware BOM and wiring diagram, README includes a full deployment guide
- Signaling layer decoupled: Cloudflare Workers signaling can be replaced with other implementations (e.g. a self-hosted WebSocket service)

## 4. Current Status & Ecosystem
- **Current Version**: actively developed (last updated 2026-06-30), no formal version number, follows continuous main-branch releases
- **GitHub Stars / Activity**: 37 Stars, 6 Forks, 0 Open Issues
- **Community Size**: small personal open-source project, not community-driven. Issues and PRs managed directly by the author
- **Docs / SDK / API Ecosystem**:
  - README contains a full hardware BOM (bill of materials), wiring diagram (Fritzing format)
  - System-architecture description (Markdown + block diagram)
  - Environment-setup guide (Pi side + ESP32 side + cloud deployment)
  - No SDK or API; protocol format defined in source code (TypeScript types + C header)
  - Control protocol and command encoding explained in source comments
- **Known Deficiencies or Limitations**:
  - Supports only a single ARRMA RC car model (Granite/Fury), high hardware coupling
  - ESP32 WiFi rather than cellular — control range limited by local network coverage (<100m WiFi direct, <50m through walls)
  - Heavy Cloudflare-infrastructure dependency: no TURN means no cross-NAT, no Tunnel means no public-internet access to the Pi
  - No production-grade safety certification (ISO 26262 / ISO 21434), cannot be used directly on manned vehicles
  - No multi-vehicle concurrent support (single Pi to single ESP32 to single car)
  - RTCDataChannel JSON telemetry serialization is less efficient than all-binary (10Hz telemetry JSON packet ~150 bytes vs binary 37 bytes)
  - No audio channel (video + control only, no voice comms)
  - HMAC Token has no auto-expiry refresh — needs a manually generated new Token redeployed

### Project Evolution History

tether-rally's architecture went through three major phases:

| Phase | Time | Architecture | Control Latency | Notes |
|------|------|------|---------|------|
| Phase 1 | 2024 Q3 | WebSocket relay (Node.js middleware) | 100-200ms | Early prototype, browser→Node.js→Pi→ESP32 |
| Phase 2 | 2025 Q1 | Cloudflare Workers signaling + direct | 30-60ms | Removed Node.js relay, switched to P2P direct |
| Phase 3 (current) | 2025 Q3+ | WebRTC RTCDataChannel + custom binary protocol | 10-15ms (LAN) | Unified video and control onto WebRTC |

### Community Ecosystem & Usage

- **GitHub stats**: 37 Stars, 6 Forks, 0 Open Issues, 1 Watch. Repo created 2024, ~50+ commits total
- **Contributors**: only the author roman01la (100% of commits), no externally-merged PRs
- **Discussion channels**: GitHub Issues, Reddit r/rccars, r/webRTC community discussions
- **Known deployment cases**: author validated in a personal backyard and local park, farthest successful control distance ~800m (via TURN relay)
- **Technical debt**: frontend is single-file JavaScript (~400 lines, no framework, no TypeScript), ESP32 firmware has no unit-test framework, stability-control parameters (EMA α=0.3, rate_limit=200) are hardcoded and need recompile to modify
- **Competitor positioning (RC FPV teleoperation domain)**:
  - vs traditional RC transmitters (Spektrum DX/Futaba T): 1-3km range, <10ms latency, but no FPV video feedback
  - vs DJI FPV digital video transmission (O3 Air Unit): ~28ms latency, 1080p@60fps, 15km range, but closed control protocol
  - vs Rosserial/ROS2 Serial approaches: suited for research, but lack real-time-control features (tiered timeouts, stability control chain)

### Known Issues & Improvement Directions

1. **WiFi range limit**: ESP32 WiFi coverage <100m (outdoor LOS), <50m (indoor through walls), cannot adapt to cellular modules
2. **Strong Cloudflare dependency**: Signaling + TURN + Tunnel all bound to Cloudflare services, no self-hosted alternative
3. **Missing HMAC Token management**: Token generated by a manual script, no auto-expiry refresh, cannot hot-swap after leak
4. **Multi-camera limited**: supports only single main camera + single rear RTSP PiP, no multi-stream sync
5. **No adaptive bitrate**: video encoding parameters fixed, WebRTC's GCC not utilized
6. **No audio channel**: FPV driving needs no voice comms (design tradeoff), but the MediaServo scenario needs bidirectional Opus audio

## 5. Market Positioning
- **Main Application Industries**: RC car enthusiast community, WebRTC teleoperation-protocol reference, education/research, robotics intro
- **Competitor Comparison Summary**:

| Dimension | tether-rally | TUM Teleoperated Driving | comma.ai openpilot (body) |
|------|-------------|--------------------------|---------------------------|
| Target platform | ARRMA RC car | Full-size passenger car + model car | comma body robot |
| Network | WiFi/LAN | LTE/5G + MQTT | WiFi/LTE (athenad) |
| Control protocol | 7-byte binary (DC) | ROS2 DDS (UDP) | Cereal/Cap'n Proto → JSON |
| RTCDataChannel | 4 independent DCs | none (RTSP+UDP scheme) | bidirectional DC (JSON) |
| Safety level | tiered timeouts (ESP32 side) | three-level safety pipeline | athenad auth + error grading |
| Open source | MIT | Apache 2.0 | MIT |
| Maturity | personal project | academic research | production-grade (L2 car) |
| Stars | 37 | ~200 | 63,127 |
| Latency data | full-chain segmented public | academic paper fully public | undisclosed |

- **Pricing / Licensing**: MIT open-source license, fully free; hardware BOM cost ~$200-400 (Pi 4/5 $35-60 + ESP32 $8-15 + Camera Module $25 + DAC + sensors + RC car)

## 6. Product Distinctions
1. **Extreme-lean binary protocol (7-byte control packet)**: demonstrates the theoretical minimum overhead of a teleoperation control channel — seq(2B) + cmd(1B) + steering(2B) + throttle(2B) = 7B vs a standard MTU of 1500B. This proves high-speed teleoperation doesn't need large packets; 20Hz-200Hz control only needs a minimized packet header
2. **Three-level timeout safety model**: the ESP32 side executes tiered safety responses (L1 80ms hold → L2 250ms center → L3 long-term safe stop), trusting none of the network-incoming data. The safety logic closes entirely at the edge; even if an attacker controls the browser, it cannot be bypassed
3. **Stability control-chain architecture**: 7 independent modules serially modify the final throttle/steering output, each module independently switchable. Demonstrates modular design of an in-vehicle control system — control-chain output = S(C(T(A(H(Surf(Steer(input))))))), each module only modifying the dimension it cares about
4. **Dual-core real-time control separation**: specifically solved the control jitter (stuttering) problem of the WebSocket-relay phase. Core 0 dedicated to network I/O, Core 1 runs a 200Hz hard-real-time control loop, proving real-time control needs a dedicated processing thread
5. **WebSocket-relay → WebRTC RTCDataChannel architecture evolution**: control latency dropped from 100-200ms to 10-15ms (LAN), demonstrating the decisive improvement direct P2P brings to teleoperation latency — the extra latency a relay introduces can be avoided directly via P2P

## 7. Reference Value for MediaServo
### [Adopt] Directly Borrowable
- **RTCDataChannel binary-protocol design**: the compact packet-header format `seq + cmd + flags + payload`. MediaServo recommended format: `timestamp(4B) + seq(2B) + cmd(1B) + flags(1B) + payload(N×2B)`, total 8+N×2 bytes
- **Channel-separation strategy**: four independent RTCDataChannels — control (unordered), telemetry (unordered), heartbeat (unordered), reliable-command (ordered+reliable)
- **Tiered-timeout safety model**: L1(80ms hold) → L2(250ms center) → L3(500ms emergency brake) → L4(2000ms safe stop) — as the teleop SDK safety baseline
- **Edge-side safe execution**: the vehicle validates all commands, not trusting network data. MediaServo's Vehicle Agent should implement a `validateCommand()` method, independent of the station-side `sanitizeCommand()` method
- **HMAC Token short-lived authorization**: simple effective on-demand access control, suited for a single remote-control session
- **Hardware encoder direct to WebRTC**: H.264 hardware encoder → MediaMTX/WHEP → WebRTC zero-copy pipeline

### [Adapt] Adopt After Modification
- **Pi bridge-layer abstraction**: currently the Pi only does UDP forwarding. MediaServo needs a `BridgeController` trait to support multiple edge hardware (Pi/Jetson/ESP32/STM32)
- **ESP32 dual-core real-time separation**: the MediaServo Vehicle Agent needs dedicated threads: receive thread + control-loop thread (Rust `std::thread` + `mpsc::channel`)
- **Stability-system pluginization**: a `DrivingAssist` trait, each module implementing `process(command: &mut ControlCommand, state: &VehicleState) -> Result<()>`, the control chain serialized via PluginManager
- **EMA smoothing + rate limit**: as the default policy of `ControlSmoother`, supporting α and rate_limit configuration
- **7-byte minimal packet → 16-byte standard packet**: add timestamp(u32), flags(u8), emergency bit — reference the extended-protocol design

### [Avoid] Known Pitfalls / Inapplicable Scenarios
- **JSON telemetry inefficient**: 10Hz JSON telemetry wastes ~4× bandwidth vs binary — MediaServo adopts an all-binary telemetry format
- **WiFi LAN-only**: MediaServo must support 4G/5G cellular and satellite comms
- **Single video stream**: MediaServo needs multi-stream sync video (front + side + rear)
- **No audio**: MediaServo teleoperation scenarios need bidirectional Opus audio
- **No adaptive bitrate**: needs to integrate WebRTC GCC and SVC scalable coding
- **Signaling bound to Cloudflare**: needs a self-developed universal signaling service (WebSocket + message queue)
- **DAC precision lesson**: 8-bit PWM (256 steps) is not fine enough — MediaServo needs 12-bit+ precision actuators
- **HMAC Token no auto-refresh**: MediaServo needs a Token-expiry auto-renewal mechanism
- **Hardware-selection coupling**: MediaServo needs to build a hardware abstraction layer (HAL) — `SensorHal` / `ActuatorHal` traits
- **No multi-vehicle concurrency**: MediaServo's signaling service needs to support `room_id` multi-conference-room / multi-vehicle isolation

### [Adopt] Directly Borrowable (supplement)

- **WebSocket → RTCDataChannel architecture-evolution path**: provides MediaServo a clear evolution route from rapid prototype to production deployment. Initially use a lightweight signaling relay to quickly validate protocol design, introduce P2P direct optimization once stable
- **bufferedAmount backpressure management**: the `bufferedAmountLowThreshold` + `onbufferedamountlow` event-driven pattern is the standard practice for the WebRTC RTCDataChannel sender. MediaServo's `DataChannelManager` component should embed this backpressure-management logic
- **ESP32 power & signal monitoring**: WiFi RSSI monitoring + battery ADC voltage-divider monitoring provide a basic vehicle-side health view. The MediaServo Vehicle Agent embeds a `HealthMonitor` component that periodically reports supply voltage and wireless signal strength
- **MCP4728 12-bit DAC selection**: proves 12-bit (4096-step) precision is the minimum requirement for actuator control, 8-bit PWM (256 steps) has severely insufficient resolution for steering and throttle control
- **Sequence-number wraparound handling**: the pattern `seq.wrapping_sub(last_seq) < 32768` is the correct implementation of uint16 sequence-number detection, all MediaServo RTCDataChannel data-packet headers should adopt it

### [Adapt] Adopt After Modification (supplement)

- **MCP4728 DAC → ActuatorHal trait**: currently uses an I2C DAC to output analog signals directly driving the RC transmitter Trainer Port. MediaServo needs to abstract this into an `ActuatorHal` trait, supporting DAC/UART PWM/RC PWM/CAN/CAN-FD multiple physical-layer interfaces
- **Core 0/1 fixed dual-core split → dynamic thread pool**: the FreeRTOS dual-core fixed split is efficient on ESP32. MediaServo's Rust Vehicle Agent needs a `tokio` async runtime to handle network I/O + a dedicated priority thread to handle the real-time control loop
- **7 stability-control modules → PluginRegistry configurable control chain**: currently each module's call order is hardcoded. MediaServo should implement a `PluginRegistry` mechanism, allowing dynamic control-chain assembly via YAML/JSON config files
- **WiFi UDP direct → NetworkHal trait**: the MediaServo Vehicle Agent needs to abstract this into a `NetworkHal` trait, supporting WiFi/UART serial/CAN bus/Ethernet multiple connection types
- **7-byte minimal packet → 16-byte extended packet**: add timestamp(u32), flags(u8, including emergency bit), checksum(u8) fields, forming a 16-byte standard packet format
- **Single video stream → MultiStreamManager**: MediaServo needs to manage 3-6 sync video streams, supporting camera dynamic switching (triggered via RTCDataChannel signaling)

### [Avoid] Known Pitfalls / Inapplicable Scenarios (supplement)

- **Cloudflare Workers ecosystem lock-in**: MediaServo must self-host the signaling service, not bound to any specific cloud vendor, supporting multi-cloud and private deployment
- **Single-player architecture**: MediaServo's multi-user scenarios (operator + supervisor + observer) need different permission-tier design — operator full control, supervisor can override commands, observer view-only
- **No active RTCDataChannel QoS monitoring**: no active one-way delay measurement and available-bandwidth estimation on the RTCDataChannel. MediaServo needs a built-in RTCDataChannel quality probe
- **ESP32 firmware no OTA**: firmware updates need USB serial flashing. The MediaServo Vehicle Agent needs a built-in OTA update mechanism (supporting chunked download + verify + rollback)
- **Single-camera limitation**: only a single front camera + rear RTSP PiP. MediaServo's vehicle teleoperation scenario needs 360° coverage — at least front + rear + both sides, 4-6 video streams
- **Hardware BOM too bespoke**: the ARRMA transmitter Trainer Port interface is an unusual RC-transmitter interface. MediaServo should drive actuators directly via CAN/CAN-FD or industrial PWM
- **Tail-latency jitter unhandled**: only control-latency mean was considered, no targeted P95/P99 tail-latency optimization. MediaServo's jitter buffer needs teleoperation-scenario-specific tuning

**Overall Rating**: ★★★☆☆ (3/5)
— As a reference for RTCDataChannel protocol design and edge-safety architecture, extremely valuable. But project scale and maturity are limited. Its 7-byte binary protocol, three-level timeout safety model, and stability control-chain architecture are the most core borrowable assets. For MediaServo's production-grade teleop SDK, tether-rally provides the best minimal-viable-implementation reference.
**Related Decisions**: D117 (emergency stop), D4, D149


## Appendix
### A. RTCDataChannel Control-Packet Binary Format Example
```
CTRL command (0x01) — 7 bytes:
  Byte 0-1: seq (uint16 LE)  — sequence number 0-65535
  Byte 2:   cmd (uint8)      — 0x01 = CTRL
  Byte 3-4: steering (int16 LE) — steering value (-4095 ~ +4095)
  Byte 5-6: throttle (uint16 LE) — throttle value (0 ~ 4095)

TELEM command (0x07) — 37 bytes:
  Byte 0-1:  seq (uint16 LE)
  Byte 2:    cmd = 0x07
  Byte 3-4:  speed (uint16 LE)         — km/h × 10
  Byte 5-6:  rpm (uint16 LE)
  Byte 7-10: lat (int32 LE)             — latitude × 10^7
  Byte 11-14: lon (int32 LE)            — longitude × 10^7
  Byte 15-16: heading (uint16 LE)       — 0.01°
  Byte 17-18: accel_x (int16 LE)        — m/s² × 100
  Byte 19-20: accel_y (int16 LE)
  Byte 21-22: accel_z (int16 LE)
  Byte 23-24: gyro_x (int16 LE)         — °/s × 10
  Byte 25-26: gyro_y (int16 LE)
  Byte 27-28: gyro_z (int16 LE)
  Byte 29-30: battery (uint16 LE)       — mV
  Byte 31-32: rssi (int16 LE)           — dBm
  Byte 33-34: status_flags (uint16 LE)  — bitmask
  Byte 35-36: crc16 (uint16 LE)         — CRC-16/XMODEM
```

### B. Mapping Suggestions to the MediaServo Control Protocol
| tether-rally command | MediaServo Mapping | Notes |
|-------------------|---------------|------|
| CTRL (0x01) | ControlCommand (50Hz) | steering + throttle + mode bits |
| PING (0x00) | Heartbeat (2Hz) | heartbeat request + RTT measurement |
| PONG (0x02) | HeartbeatResponse | heartbeat response |
| TELEM (0x07) | VehicleState (10Hz) | GPS + IMU + vehicle speed + battery |
| STATUS (0x04) | SystemState (1Hz) | WiFi signal + CPU temp + device health |
| CONFIG (0x05) | ConfigUpdate (on-demand) | encoding params + safety thresholds + control limits |
| RACE (0x03) | ModeSwitch | operation-mode switch |
| KICK (0x06) | RestartDevice | remotely restart device controller |
| TURBO (0x08) | DrivingAssistToggle | Turbo-mode switch |
| TRACTION (0x09) | DrivingAssistToggle | traction-control switch |

### C. ESP32 Tiered-Timeout Parameter Suggestions
| Level | Timeout | Action | Applicable Scenario |
|------|------|------|---------|
| L0 | 0-50ms | Normal receive, execute directly | All normal operations |
| L1 | 50-100ms | EMA smoothing interpolation (hold last command direction) | Brief network fluctuation |
| L2 | 100-300ms | Steering center + Throttle zero (progressive) | Intermittent network outage |
| L3 | 300-2000ms | Full-power braking + emergency stop | Severe network failure |
| L4 | >2000ms | Safe stop + hazard lights + remote-diagnosis request | Connection fully lost |

### D. Stability Control-Chain Code Structure (pseudocode)
```rust
fn compute_final_output(input: ControlCommand, state: VehicleState) -> ActuatorOutput {
    // Control chain executed serially — each module only modifies the dimension it cares about
    let mut cmd = input;
    cmd = steering_shaper(cmd, state.speed);        // 1. steering shaper
    cmd = traction_control(cmd, state);              // 2. traction control
    cmd = stability_control(cmd, state);             // 3. stability control
    cmd = abs_control(cmd, state);                   // 4. ABS
    cmd = hill_hold(cmd, state);                     // 5. hill hold
    cmd = coast_control(cmd, state);                 // 6. coast control
    cmd = surface_adaptation(cmd, state);            // 7. surface adaptation
    cmd = ema_smooth(cmd, prev_cmd, alpha=0.3);     // 8. smoothing filter
    cmd = slope_limit(cmd, prev_cmd, max_rate);     // 9. rate limit
    ActuatorOutput::from(cmd)
}
```
