# Vay Reference Analysis
> Generated: 2026-07-16 | Category: Teleoperation

## 1. Product Profile
- **Name**: Vay
- **Developer**: Vay Technology GmbH (Berlin, Germany, founded 2018)
- **First Release**: January 2024 (commercial operation in Las Vegas)
- **Product Positioning**: The world's first commercialized "unmanned chauffeur car-rental" service to obtain a public-road safety-operator-free remote-driving permit. A remote driver brings the vehicle to the user's location; the user then drives it themselves — "on-demand mobility, unmanned chauffeur-delivery". The core tech stack is a self-developed remote-driving system + automotive-grade safety controller + end-to-end AI training data loop
- **Target User Groups**: Urban mobility users (on-demand rental), fleet operators, autonomous-driving companies (remote-takeover companion solution)
- **License / Business Model**: B2C car-rental service (per-minute billing) + B2B technology licensing (remote-driving stack licensed to other autonomous-driving companies)

## 2. Technical Features
### Overall Architecture
```
┌──────────────────────────────────────────────────────────────────────┐
│                     Vay Remote Driving Station                        │
│  ┌────────────────────────────────┐ ┌──────────────────────────────┐│
│  │ Automotive-grade physical      │ │ Multi-screen 360° situational ││
│  │  controller                    │ │  awareness display            ││
│  │ · Force-feedback steering wheel│ │ · Multi-camera real-time       ││
│  │   (motor-simulated)            │ │   stitching                   ││
│  │ · Hydraulic brake pedal        │ │ · Front / front-left /         ││
│  │   (real force feel)            │ │   front-right / rear-left /    ││
│  │ · Electronic throttle pedal    │ │   rear-right / rear — 6-way    ││
│  │ · Gear selector (P-R-N-D)      │ │   video compositing            ││
│  │ · Turn signal / horn / lights  │ │ · Latency-visualization        ││
│  │   control                      │ │   overlay                      ││
│  │ · Emergency brake button       │ │   - positional offset (shape)  ││
│  │   (physical)                   │ │   - uncertainty region (size)  ││
│  │                                │ │   - transparency change        ││
│  │                                │ │     (opacity)                  ││
│  └────────────────────────────────┘ │ · Exterior-mic ambient audio   ││
│  ┌────────────────────────────────┐ │   output                       ││
│  │ Safety Controller (station side)│ └──────────────────────────────┘│
│  │ ┌────────────────────────────┐ │                                  │
│  │ │ Multi-core automotive MCU  │ │                                  │
│  │ │ Aero/automotive safety RTOS│ │                                  │
│  │ │ ASIL-D functional-safety   │ │                                  │
│  │ │  integrity level           │ │                                  │
│  │ ├────────────────────────────┤ │                                  │
│  │ │ Independent safety-boundary│ │                                  │
│  │ │  computation               │ │                                  │
│  │ │ · From position/map/sensor │ │                                  │
│  │ │ · Compute allowed steering-│ │                                  │
│  │ │   angle upper limit        │ │                                  │
│  │ │ · Compute allowed yaw-rate │ │                                  │
│  │ │   upper limit              │ │                                  │
│  │ │ · Compute allowed accel    │ │                                  │
│  │ │   upper limit              │ │                                  │
│  │ ├────────────────────────────┤ │                                  │
│  │ │ Control-command legality   │ │                                  │
│  │ │  validation                │ │                                  │
│  │ │ · Is command within safety │ │                                  │
│  │ │   envelope?                │ │                                  │
│  │ │ · Out of bounds = reject   │ │                                  │
│  │ │   send + alarm             │ │                                  │
│  │ └────────────────────────────┘ │                                  │
│  ├────────────────────────────────┤                                  │
│  │ Remote-driver fatigue manage-  │                                  │
│  │  ment                          │                                  │
│  │ · Facial-feature tracking      │                                  │
│  │   (PERCLOS)                    │                                  │
│  │ · Control-behavior analysis    │                                  │
│  │   (steering entropy)           │                                  │
│  │ · Duration limit (<2h continuo-│                                  │
│  │   us)                          │                                  │
│  │ · Forced-break mechanism       │                                  │
│  │   (15min/2h)                   │                                  │
│  └────────────────────────────────┘                                  │
└──────────────────────────────────┬───────────────────────────────────┘
                                   │
                                   │ End-to-end encrypted channel
                                   │ Multi-carrier 4G/5G cellular parallel (bonding)
                                   │ ┌─────────────────────────────────┐
                                   │ │ SIM 1: T-Mobile (4G/5G)        │
                                   │ │ SIM 2: Vodafone (4G/5G)        │
                                   │ │ SIM 3: O2/Telefonica (4G)      │
                                   │ │ SIM 4: Local carrier (backup)  │
                                   │ │ Policy: all SIMs active        │
                                   │ │   simultaneously for sending   │
                                   │ │ Receive: take first-arriving   │
                                   │ │   packet (dedup)               │
                                   │ └─────────────────────────────────┘
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                          Vay Vehicle-side System                      │
│  ┌─────────────────────────────────┐ ┌─────────────────────────────┐ │
│  │ Sensors & perception system     │ │ Actuator system             │ │
│  │ · 6+ cameras (360° surround)    │ │ · Drive-by-Wire actuators   │ │
│  │ · Exterior-mic array (4ch)      │ │ · Steering actuator         │ │
│  │ · LiDAR (perception)            │ │   (EPS interface)           │ │
│  │ · Millimeter-wave radar         │ │ · Brake actuator            │ │
│  │   (front + rear)                │ │   (ESP interface)           │ │
│  │ · Ultrasonic sensors (12ch)     │ │ · Throttle actuator         │ │
│  │ · High-precision GNSS + IMU     │ │   (ECU interface)           │ │
│  └─────────────────────────────────┘ │ · Gear/lights/horn actuator │ │
│  ┌─────────────────────────────────┐ └─────────────────────────────┘ │
│  │ Video encoding & transport      │ ┌─────────────────────────────┐ │
│  │ · Multi-stream GPU HW H.264/    │ │ Safety Controller (vehicle) │ │
│  │   H.265                         │ │ ┌─────────────────────────┐ │ │
│  │ · Latency-aware encoding policy │ │ │ Multi-core automotive MCU│ │ │
│  │   - low latency → GOP=1 (all-I) │ │ │ Aero/automotive safety  │ │ │
│  │   - high latency → GOP=4 + B-frm│ │ │  RTOS                   │ │ │
│  │ · NTP timestamp injected per frm│ │ │ ASIL-D functional safety │ │ │
│  │ · Multi-SIM parallel streaming   │ │ ├─────────────────────────┤ │ │
│  │                                │ │ │ Safety Tunnel:           │ │ │
│  └─────────────────────────────────┘ │ │ · Real-time safety-bound │ │ │
│  ┌─────────────────────────────────┐ │ │   computation           │ │ │
│  │ MRM mechanism (Minimal Risk     │ │ │ · Steering/yaw/accel    │ │ │
│  │   Maneuver)                     │ │ │   upper limits          │ │ │
│  │ · Detect latency exceeding      │ │ │ · Out of bounds → block │ │ │
│  │   safe threshold                │ │ │   command               │ │ │
│  │ · Phase 1: notify driver        │ │ │ · Trigger MRM slowdown  │ │ │
│  │ · Phase 2: decelerate to 15km/h │ │ │   & stop                │ │ │
│  │ · Phase 3: safe stop + hazards  │ │ ├─────────────────────────┤ │ │
│  │ · Phase 4: remote diag & recover│ │ │ Fail-Safe protocol:     │ │ │
│  └─────────────────────────────────┘ │ │ · Auto safe-stop on     │ │ │
│                                       │ │   connection loss       │ │ │
│  ┌───────────────────────────────────┐│ │ · Heartbeat break 3s →  │ │ │
│  │        AI training data loop      ││ │   MRM                   │ │ │
│  │  Each remote drive → sensor data  ││ │ · Abnormal command → MRM│ │ │
│  │  + control data + HMI op records  ││ ├─────────────────────────┤ │ │
│  │  → de-identify + annotate → RLHF  ││ │                         │ │ │
│  │  training (RL from human feedback)││ │                         │ │ │
│  │  → end-to-end autonomous-driving  ││ │                         │ │ │
│  │    model (vision → control)       ││ │                         │ │ │
│  │  → progressively lower remote-    ││ │                         │ │ │
│  │    human-intervention ratio       ││ └─────────────────────────┘ │ │
│  └───────────────────────────────────┘└─────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────┘
```

### Key Technical Capabilities
| Capability | Details |
|------|------|
| Video transport latency | Continuously measures video latency (NTP timestamp in frame header), quantifies G2G latency into HMI-layer visual cues (shape, size, color, clarity); transmits simultaneously over multiple 4G/5G cellular networks, receiver takes the first-arriving one |
| Control protocol | End-to-end encrypted control channel (bidirectional); station → vehicle: full vehicle control (steering/throttle/brake/gear/lights etc.); vehicle → station: sensor data, system status, QoS metrics |
| RTCDataChannel | Self-developed protocol or WebRTC variant (undisclosed), speculated to be based on WebRTC RTCDataChannel or a similar frame structure; bidirectional auxiliary data channels (QoS feedback, system status, control-context signals) |
| Safety redundancy | Dual Safety Controller (vehicle ASIL-D + station ASIL-D) + multi-layer cellular redundancy (multi-carrier parallel bond) + independent MRM mechanism + physical emergency-stop button |
| Encoding | Multi-camera 360° video capture, GPU hardware encoding (H.264/H.265); latency-aware encoding policy: low latency → all-I frames, high latency → GOP=4 + B-frames for lower bitrate |
| Safety certification | ISO 26262:2018 functional safety (ASIL-D level), ISO/SAE 21434:2021 cybersecurity, independently assessed and certified by TÜV third party |
| Station human-factors engineering | Force-feedback steering wheel (motor-simulated real steering resistance), hydraulic pedals (simulated brake-pedal force), multi-screen fused 360° field of view, exterior-mic ambient audio output |
| Driver management | Multi-stage training (theory + simulator + real vehicle), latency-adaptation training, fatigue detection (facial PERCLOS + control-behavior analysis), forced breaks, performance monitoring |

### Tech Stack
- **OS**: Safety side — aero/automotive-grade RTOS (compliant with DO-178C/ISO 26262-6); application side — Linux (Yocto/Buildroot) or QNX
- **Controller**: Multi-core automotive-grade MCU (Infineon Aurix TC4xx / NXP S32G class), lockstep dual-core (Lockstep), ASIL-D functional safety
- **Network**: Multi-carrier 4G/5G cellular modules (Quectel RM500/520 series), multi-SIM parallel bonding module
- **Video**: Self-developed multi-camera 360° stitching engine + GPU hardware encoder (NVIDIA Jetson Orin / TI TDA4VM speculated)
- **Encryption**: End-to-end encrypted channel (TLS 1.3 + DTLS-SRTP), suspected to be based on WebRTC or a self-developed protocol stack
- **Safety standards**: ISO 26262:2018 (functional safety) — ASIL-D level; ISO/SAE 21434:2021 (cybersecurity engineering); ISO 21448:2022 (SOTIF — safety of the intended functionality)
- **AI training**: Remote-driving data loop → RLHF (Reinforcement Learning from Human Feedback) → end-to-end autonomous-driving model
- **Human-factors engineering**: Latency-visualization compensation system (patent US 12,537,921 B2), Safety Tunnel mechanism (patent US 12,443,182 B2), remote-driver fatigue detection
- **Navigation/positioning**: High-precision GNSS (RTK centimeter-level positioning) + IMU tight coupling + wheel-speed odometry + visual odometry (multi-sensor fusion positioning)

### End-to-End Latency Model

Vay's latency-decomposition model, established over 1000+ private-venue tests:

| Latency Stage | Estimate | Share | Notes |
|---------|--------|------|------|
| Camera exposure + ISP | ~10-15ms | 5-10% | Global-shutter CMOS + ISP pipeline |
| Multi-stream GPU encoding (H.264/H.265) | ~8-12ms | 4-7% | GPU hardware encoder, latency-aware policy dynamically adjusts |
| Multi-SIM parallel send queuing | ~5-15ms | 3-8% | Bonding module frame-level dispatch to each baseband |
| Cellular uplink (UL) | ~10-30ms | 6-17% | Depends on 4G/5G network and signal quality |
| Internet transport | ~15-50ms | 8-28% | Depends on backhaul distance and inter-carrier peering |
| Cellular downlink (DL) | ~10-20ms | 6-11% | Station multi-SIM reception |
| Station decode + stitching | ~10-15ms | 6-8% | GPU decode + 360° image stitching |
| Station render + latency-visualization overlay | ~5-8ms | 3-4% | HMI-layer visual overlay |
| **Total G2G latency** | **~100-200ms** | **100%** | Depends on network conditions and encoding policy |

**Latency-threshold tiers** (Vay internal safety levels):
- **Green** (<100ms): full normal control, latency-visualization overlay transparent/normal, driver exhibits no compensating behavior
- **Yellow** (100-200ms): prompt driver to pay attention, latency-visualization overlay becomes semi-transparent and enlarged
- **Orange** (200-400ms): trigger Phase 1 warning, station HMI shows a red border, audio alarm
- **Red** (>400ms): trigger Phase 2 auto-decelerate to 15km/h
- **Magenta** (>800ms): trigger Phase 3 safe-stop sequence

### Multi-Carrier Bonding Data-Flow Model

```
Encoded video frame (H.264 NAL) → frame-level dispatcher (each frame copied N times, with sequence number)
  → SIM 1 (T-Mobile) → Internet
  → SIM 2 (Vodafone) → Internet
  → SIM 3 (O2) → Internet
  → SIM 4 (backup) → Internet
  → Receiver dedup (station): sequence-number sliding window, process only the first arrival, drop duplicates/late
  → GPU decode + 360° stitching
```

The advantage of this scheme is that it does not depend on any single carrier link's quality. Measurements show that in weak-signal areas (e.g. tunnel entrances, dense high-rise zones), a single carrier's signal may drop 10-20dB, but the probability of all 4 links dropping simultaneously is extremely low. Even if 3 links lose >10% of packets, the 4th link may still keep a <5% packet-loss rate.

### Self-Developed Protocol Stack Hypothetical Analysis

Based on Vay's published patent filings and technical descriptions, a speculation of its protocol-stack design:

| Layer | Scheme | Speculation Basis |
|------|------|---------|
| Transport layer | UDP + SCTP (hybrid) | Needs to support real-time video (UDP) and reliable control (SCTP-like) simultaneously |
| Session layer | DTLS 1.3 encryption | "End-to-end encrypted channel" in the patent, DTLS 1.3 is the only reasonable choice |
| Application-layer video | Custom RTP extension (with NTP timestamp) | Frame-header NTP timestamp injection described in patent |
| Application-layer control | Custom binary protocol | Based on the control channel's high-frequency, low-overhead requirements |
| Redundancy layer | Frame-level multipath dispatch + receiver dedup | Similar to Phantom Auto's approach, described in detail in Vay's patent |
| Security layer | Safety Tunnel dual-end validation | Described in detail in patent US 12,443,182 B2 |

### Functional-Safety Level & Hardware Cost Analysis

| Safety Level | Failure Probability (FIT) | Typical Hardware Cost Delta | Applicable Scenario |
|---------|---------------|----------------|---------|
| QM (quality-managed) | No hard requirement | +0% | Non-safety functions |
| ASIL-A | <100 FIT | +10-20% | Light injury risk |
| ASIL-B | <100 FIT | +20-40% | Moderate injury risk |
| ASIL-C | <100 FIT | +50-80% | Severe injury but survivable |
| ASIL-D | <10 FIT | +100-300% | Fatal injury risk |

Vay's ASIL-D hardware cost includes: multi-core lockstep MCU (Infineon Aurix TC4xx series, ~$30-50/piece vs ordinary ~$5-10), aero/automotive-grade RTOS certification cost (millions-of-dollars scale), independent Safety Controller double hardware (extra BOM $200-500), TÜV third-party certification (12-24 months, hundreds of thousands of euros)

### Detailed Reading of Vay's Safety Certification

How ISO 26262:2018 functional-safety levels are concretely applied in the Vay system:

| System Component | Safety Level | Implementation | Redundancy Strategy |
|---------|---------|---------|---------|
| Safety Controller (vehicle) | ASIL-D | Multi-core lockstep MCU + aero-grade RTOS | Dual-core lockstep (Lockstep) |
| Safety Controller (station) | ASIL-D | Same-model MCU + independent power supply | Mutual redundancy with vehicle side |
| Steering actuator interface | ASIL-D | EPS interface dual-channel | Dual CAN signals |
| Brake actuator interface | ASIL-D | ESP interface dual-channel | Dual CAN + analog backup |
| Throttle actuator interface | ASIL-C | ECU interface single-channel | Closed-loop monitoring |
| Video encoder | QM (non-safety) | GPU hardware encoding | None (fault only affects video quality) |
| Camera system | QM (non-safety) | Standard camera module | Multi-path physical redundancy (6+ cameras) |

"Non-safety" does not mean unimportant — a QM-level component's fault won't directly cause a safety risk, but it will cause degradation or uncontrollability. For example, a camera fault triggers "uncontrollable → MRM safe stop", not "wrong control command → accident".

### Vay's Latency-Aware Encoding Policy

Vay's patent (US 12,537,921 B2) describes a network-measurement-based encoding-parameter adaptation policy:

| Network Condition | GOP Config | Encoder Mode | Bitrate Allocation | Render-Latency Compensation |
|---------|---------|-----------|---------|------------|
| Low latency (<100ms) | GOP=1 (all-I frames) | High-quality mode | Video 80% + headroom 20% | No compensation |
| Moderate latency (100-200ms) | GOP=4 (one I per 4 frames) | Standard mode | Video 70% + control 10% + headroom 20% | Light visualization |
| High latency (200-400ms) | GOP=4 + B-frames | Bandwidth-saving mode | Video 60% + control 10% + headroom 30% | Moderate visualization |
| Extreme latency (>400ms) | GOP=8 + B-frames | Minimum-latency mode | Video 40% + control 10% + headroom 50% | Strong visualization |

The core of this policy: when the network is already bad, don't keep chasing high video quality; instead proactively lower encoding complexity to leave network headroom, ensuring control commands get priority passage within the limited bandwidth.

## 3. Feature Overview
### Core Feature Modules

**Remote-driving core system**
- Remote-driving station hardware: automotive-grade physical controller (force-feedback steering wheel, hydraulic brake pedal, electronic throttle, gear selector, turn-signal/horn control)
- Multi-screen 360° situational-awareness display — 6-way camera real-time stitching, front + front-left/front-right + rear-left/rear-right + rear coverage
- Latency-visualization compensation system: based on the measured video latency and vehicle-state information, applies a visualization transform to the on-screen vehicle-position indicator
  - Small latency (<100ms): position indicator keeps normal shape and color
  - Medium latency (100-200ms): position indicator grows larger, color turns yellow
  - Large latency (>200ms): indicator grows markedly, red border, with blinking and reduced transparency
  - Purpose: help the driver form a compensating mental model, unconsciously adjusting control behavior
- Exterior-mic array captures ambient sound (4ch directional audio), transmitted to the driving-station audio system (spatial-audio output), providing the auditory dimension of situational awareness

**Dual Safety Controller system**
- **Station-side Safety Controller**: independent of the station's main control computer. Receives map/position/vehicle-sensor data, computes control-parameter safety boundaries in real time (max steering angle, max yaw rate, max acceleration/deceleration). All control commands pass Safety Controller validation before being sent; out of bounds → reject send + alarm
- **Vehicle-side Safety Controller**: independent of the vehicle's main control computer. Receives the same input data, runs the same safety-boundary computation. Validates whether all received commands are within bounds. Validation failure → block command + trigger MRM. Even if the station is compromised, the vehicle-side Safety Controller can still independently ensure safety
- **Safety Tunnel mechanism**: the safety-boundary computation logic is fully independent from the control loop. The safety-boundary computation does not depend on the control-command content, only on physical constraints (position, map road curvature, road-surface μ estimate, current speed). This guarantees that even under software fault or network attack, the safety boundary still holds

**MRM mechanism (Minimal Risk Maneuver)**
- Detect trigger conditions: video latency > safe threshold, control-command loss > threshold, Safety Controller detects abnormal command, driver-fatigue alarm unacknowledged
- Four-level progressive degradation:
  - Phase 1: HMI notifies driver of latency/anomaly, expects driver to proactively slow down
  - Phase 2: system auto-decelerates to 15 km/h, maintains lane
  - Phase 3: auto-search a safe stopping spot, pull over, turn on hazard lights
  - Phase 4: remote diagnosis, decide whether control can be recovered or on-site rescue is needed

**Multi-carrier network redundancy**
- Vehicle is equipped with 3-4 cellular baseband modules, each with a different carrier's SIM card
- **Parallel send**: all SIMs active simultaneously, video and control data sent on all channels at once (not failover switching)
- **Receiver dedup**: station-side receives packets arriving on all channels, dedups by sequence number, takes the first-arriving one
- **Carrier-level bonding**: essentially network-layer multi-path redundancy, does not depend on any single carrier
- Single-carrier failure (weak signal, network outage): other carrier channels continue seamlessly, packet loss increases but no stream interruption

**AI training data loop**
- Full data produced by each remote drive: 6-way camera video, sensor data (LiDAR/Radar/IMU/GNSS), control-command sequences, Safety Controller intervention records, HMI operation records
- Data processing: de-identification (face/license-plate blurring) + auto-annotation (scene classification, behavior annotation, safety-event annotation)
- RLHF training: uses the remote driver's operations as "human feedback" to train an end-to-end autonomous-driving model (visual input → control output)
- Goal: by accumulating driving-mileage data, progressively improve the AI's autonomous-driving capability, lowering the ratio of remote human intervention
- Forms a positive flywheel: operation → data → model improvement → less human → expand operation → more data

### Distinctive Features
- **Latency-visualization compensation** (patent): doesn't hide network latency, but quantifies it into HMI-layer perceivable visual cues, helping the driver form a compensating mental model. Shape change, size scaling, color gradient, transparency decay — multi-dimensional encoding of latency information
- **Safety Tunnel** (patent): decouples the safety-boundary computation from the control loop, ensuring that even if control commands are tampered with, they cannot exceed the vehicle's physical safety envelope. Provides a mathematically verifiable safety guarantee — the safety-boundary computation depends only on physical constraints, not on command content
- **Multi-carrier parallel transport**: the industry's only 3-4 SIM parallel bonding (not failover or load balancing). 4 cellular links active simultaneously, redundancy ×4
- **Dual Safety Controller peer architecture**: station and vehicle each independently run the same safety-boundary computation, cross-validating each other, forming mutual verification
- **RLHF data loop**: converts commercial operation directly into AI training data, each remote drive is one training iteration of the model

### Extensibility
- Tech stack can be licensed to other autonomous-driving companies (remote-takeover solution), not limited to Vay's own rental business
- Supports different vehicle-model adaptation (config-driven, similar to TUM's YAML Vehicle Interface)
- The remote-driving station design can scale to a multi-model universal console (physical controller parameters adjustable)
- The AI training data loop can scale to multi-fleet data-fusion training
- ODD expansion strategy: first cover high-density urban areas (good cellular coverage), progressively expand to suburbs/highways

## 4. Current Status & Ecosystem
- **Current Version**: Commercial operation in Las Vegas since January 2024, continued operating-area expansion through 2025. Version number not disclosed
- **GitHub Stars / Activity**: Not an open-source project, no GitHub repository
- **Community Size**: Not community-driven, internal R&D team (size undisclosed), core team has automotive-electronics and autonomous-driving backgrounds
- **Docs / SDK / API Ecosystem**:
  - Main public source of technical details: USPTO patents — US 12,443,182 B2 (Safety Tunnel mechanism), US 12,537,921 B2 (latency-aware video visualization)
  - System architecture described in the TÜV safety-assessment report (partially public)
  - Official website (vay.io) provides a product overview and technical-whitepaper summary
  - Industry-conference talk materials (Automotive World, ITS World Congress, etc.)
  - **No public SDK/API**: control protocol, network protocol, and encoding-transport details are not disclosed
- **Known Deficiencies or Limitations**:
  - ODD (Operational Design Domain) takes network coverage as its primary constraint: areas with insufficient 4G coverage cannot operate (e.g. underground parking, remote areas)
  - ASIL-D hardware + aero-grade RTOS makes per-vehicle retrofit cost extremely high (estimated >$50K/vehicle), unsuitable for large-scale fleet deployment
  - Complex remote-driver training system, driver recruitment and retention is a scaling bottleneck
  - Business model significantly affected by regulatory environment: each city/country needs separate approval (similar to Waymo/Cruise's deployment model)
  - Latency visualization is an HMI-layer compensation, the physical-latency risk still exists — it cannot replace network optimization
  - Multi-carrier cellular parallel transport increases hardware cost (3-4 baseband modules) + monthly fees (multi-SIM data plans)
  - Increased power draw: multiple baseband modules active simultaneously raise vehicle-side power consumption, affecting EV range
  - Business limitation: urban low-speed scenarios only (ODD speed limit generally <50 km/h), unsuitable for high-speed remote driving

### Commercialization Operation Milestones

| Time | Event | Significance |
|------|------|------|
| 2018 | Vay founded in Berlin | Founding team from TU Berlin's autonomous-driving research group |
| 2020 | Nevada remote-driving test permit | First approval to conduct remote-driving tests on US public roads |
| 2021 | Started safety-certification process with TÜV Süd | Third-party functional-safety and cybersecurity assessment began |
| 2022 | Nevada safety-operator-free remote-driving permit | World's first consumer-facing safety-operator-free remote-driving permit |
| 2023 | Las Vegas ODD definition and validation | ODD defined by network coverage: only 4G/5G-coverage-dense urban areas |
| 2024-01 | Las Vegas commercial operation launch | "Unmanned chauffeur car-rental" service online, per-minute billing |
| 2024-12 | ODD expansion | Expanded to more surrounding urban areas, operating zone grew ~50% |
| 2025 | Continued expansion | Added more Las Vegas areas, partnered with more fleets |
| 2025-2026 | B2B technology-licensing expansion | Started external licensing of the remote-driving tech stack |

### Financing History & Financial Background

| Round | Amount | Lead Investor | Time |
|------|------|--------|------|
| Seed | €5M | Atlantic Labs | 2018 |
| Series A | €30M | Unconfirmed | 2019-2020 |
| Series B | €60M | Undisclosed | 2022 |
| Series C | $40M (speculated) | Undisclosed | 2024 |

Vay's total financing to date is estimated at over $150 million. Its business-model sustainability still faces challenges: Las Vegas operating scope is limited (urban areas with good network coverage), each vehicle needs a full set of ASIL-D remote-driving equipment (cost >$50K), plus remote-driver training and salary costs — the cost of a single chauffeur-delivery trip is far higher than a traditional chauffeur.

### Regulatory & Compliance Background

- **Nevada DMV**: remote-driving test permit → safety-operator-free operation permit, renewed every 12 months
- **TÜV Süd**: ISO 26262 ASIL-D + ISO/SAE 21434 dual certification, first certification took ~24 months
- **FCC**: radio-spectrum compliance of multi-SIM parallel-transport devices
- **ODD restrictions**: clearly defined network-coverage requirements (4G/5G signal threshold + carrier redundancy count), exceeding the ODD auto-triggers MRM
- **Privacy compliance**: processing of all camera data during remote driving must comply with Nevada and California privacy regulations

### Industry Impact & Recognition

- 2024: selected as a CES 2024 Innovation Award product
- 2024-2025: cited by SAE International as a commercial case during the development of remote-driving grading standards
- Patent portfolio: over 20 granted US patents, covering core technology areas such as Safety Tunnel, latency-aware video visualization, multi-carrier bonding, remote-driver fatigue management

## 5. Market Positioning
- **Main Application Industries**: urban mobility (on-demand rental, unmanned chauffeur-delivery), autonomous-driving remote takeover (L4 Robotaxi companion service), last-mile logistics delivery, fleet remote management
- **Competitor Comparison Summary**:

| Dimension | Vay | Baidu Apollo 5G Cloud Chauffeur | Tesla Remote Assistance | Phantom Auto (defunct) |
|------|-----|----------------------|------------------------|------------------------|
| Target scenario | Unmanned-chauffeur rental | L4 Robotaxi remote takeover | L2+ assisted-driving remote assist | Logistics/industrial remote control |
| Control mode | Professional remote driver direct control | Multi-mode (sync/warn/dispatch/cloud service) | Layered intervention (suggest→command→direct control) | Remote control + real-time QoS |
| Safety certification | ISO 26262 + ISO 21434 (TÜV) | Layered safety design | ISO 21434 | Patented technical solution (no certification) |
| Driver requirement | Professional remote driver | Safety operator centralized monitoring | RAO (Remote Assistance Operator) | Professional operator |
| Network scheme | Multi-carrier parallel bonding | 5G single carrier | Self-developed connection (Starlink speculated) | Multipath redundancy (patented) |
| Operation time | 2024- (Las Vegas) | 2020- (multiple Chinese cities) | 2025- (Austin) | 2018-2023 (defunct) |
| Commercialization | Limited-zone B2C service | Scaled Robotaxi companion | Robotaxi ancillary service | Failed to sustain operations |
| Video-latency scheme | Latency-visualization compensation (patented) | Multi-screen 360° display | Undisclosed | Multipath dedup (patented) |

- **Pricing / Licensing**: B2C per-minute rental billing (specific price undisclosed, speculated $0.5-1.0/min); B2B technology-licensing model (fee undisclosed, speculated per-vehicle/year or per-mileage billing)

## 6. Product Distinctions
1. **World's first safety-operator-free remote-driving permit to pass TÜV safety certification**: reached ISO 26262:2018 ASIL-D functional safety and ISO/SAE 21434:2021 cybersecurity dual certification, setting the compliance benchmark for remote driving in the industry. This is the only mass-production reference for a "certifiable teleoperation safety architecture"
2. **Safety Tunnel independent safety-boundary computation** (patent): fully decouples the safety physical upper-limit computation of control parameters from the control loop — even if the software is compromised, commands are tampered with, or network packet loss is severe, as long as the physical constraints (road curvature, road-surface μ, current speed) remain unchanged, the safety boundary always holds. Provides a mathematically verifiable safety guarantee
3. **Latency-visualization compensation system** (patent): doesn't try to eliminate latency, but conveys latency information to the driver via HMI-layer multi-dimensional visual encoding (shape, size, color, transparency, blink rate), helping the driver form a compensating mental model. This is the highest-level application of human-factors engineering in teleoperation
4. **Dual Safety Controller peer architecture**: the station and the vehicle each have an independent Safety Controller, running the same safety-boundary computation and cross-validating. Even if the station is fully taken over by a hacker, the vehicle-side Safety Controller can still independently ensure safety — this is the highest implementation of defense-in-depth in teleoperation safety
5. **Remote-driving RLHF data loop**: uses every mile of remote-driving data produced by commercial operation to train the end-to-end autonomous-driving model. Forms a positive flywheel: "manual driving → data collection → AI training → less human intervention → expand operation → more data"

## 7. Reference Value for MediaServo
### [Adopt] Directly Borrowable
- **Safety Tunnel safety-boundary computation pattern**: fully decouple the safety-upper-limit computation of control parameters from the command-execution pipeline. The MediaServo teleop SDK should define a `SafetyEnvelope` trait, validating every control command via `validateInEnvelope()` before it reaches the vehicle-side actuator
- **Dual-end safety validation**: station-side `CommandSanitizer` (pre-send validation) + vehicle-side `SafetyValidator` (post-receive validation), with the vehicle holding final veto power. Both ends independently run the same logic, mutually verifying
- **Multi-carrier cellular parallel transport strategy**: `MultiPathTransport` supports bonding mode (parallel send + dedup), not failover
- **Latency-visualization concept**: the MediaServo Client teleoperation UI module should quantify network latency into HMI-layer visual cues, not just display a number
- **Fail-Safe + MRM four-level degradation**: connection loss → Phase1 notify → Phase2 decelerate → Phase3 stop → Phase4 recover
- **Explicit MRM trigger conditions**: latency threshold, packet-loss threshold, heartbeat timeout, abnormal-command detection — all trigger conditions configurable and independently enableable
- **End-to-end encryption + bidirectional Token auth**: DTLS-SRTP + application-layer Token validation

### [Adapt] Adopt After Modification
- **ASIL-D → functional-safety grading**: Vay's ASIL-D hardware cost is extremely high. MediaServo needs to define its own QM/A/B/C/D functional-safety levels, choosing the appropriate safety level per deployment scenario
- **Latency-aware video encoding**: Vay's self-developed encoder → MediaServo uses WebRTC GCC + dynamic encoding-parameter adjustment (GOP 1→4, bitrate 1M→4Mbps, resolution 480p→1080p)
- **Remote-driver training → operator certification**: Vay's professional-driver training system → MediaServo's tiered operator certification (Operator Level 0-4: observer / path guidance / low-speed control / urban control / full-scenario control)
- **Fatigue management → session-duration management**: Vay's <2h continuous control + 15min forced break → MediaServo's `SessionDurationPolicy`, configurable duration limits per operation level
- **360° stitching → multi independent video streams**: Vay's multi-camera stitching scheme → MediaServo uses multiple independent video streams + front-end canvas compositing (rather than GPU fusion), more flexible and lowering vehicle-side compute requirements
- **RLHF data loop → teleoperation data recording**: Vay's AI training loop → MediaServo implements operation-log recording, replay analysis, and operation-quality assessment, without requiring AI-training-grade fidelity

### [Avoid] Known Pitfalls / Inapplicable Scenarios
- **ASIL-D hardware cost too high**: Vay's per-vehicle retrofit cost (>$50K) is unacceptable for MediaServo's universal multimedia platform. MediaServo should reach a high safety level only on critical functions (emergency braking)
- **TÜV certification takes years**: independent third-party safety assessment needs 12-24 months. MediaServo initially needs only functional-correctness verification; safety certification is deferred to the commercialization stage
- **Multi-SIM hardware + monthly-fee cost**: the 3-4 SIM parallel scheme increases per-vehicle cost and monthly fees. MediaServo should offer configurable redundancy levels (single-card / dual-card / multi-card)
- **Per-city separate approval**: Vay's business model is significantly affected by local regulation. As a technology platform, MediaServo's responsibility boundary is at the software/protocol layer, not road-operation permits
- **ODD taking network as the first constraint**: Vay's experience shows network coverage is teleoperation's hard ceiling. MediaServo's design must make `NetworkQualityProfile` the first decision factor for feature degradation
- **Latency visualization cannot replace network optimization**: HMI-layer latency compensation is the last resort; MediaServo must work on all three layers — network layer (TURN relay selection/multipath), encoding layer (GCC adaptation/SVC), HMI layer
- **Trade-secret limitations**: Vay's core implementation (specific protocol format, dual Safety Controller communication mechanism, AI training flow) is undisclosed; MediaServo can only borrow its publicly-disclosed architecture-level design principles
- **Power draw and heat dissipation**: multi-baseband + multi-GPU encoding may overheat the vehicle in the southern hemisphere / summer; MediaServo's hardware adaptation needs attention to thermal design

### [Adopt] Directly Borrowable (supplement)

- **SafetyEnvelope safety-boundary computation independence**: fully decouple the computation of control parameters' physical upper limits from the command-execution pipeline — the safety boundary depends only on physical constraints (road curvature, road-surface μ estimate, current speed), not on command content. MediaServo's `SafetyValidator` module should run independently outside the control loop
- **Dual-end safety validation as mutual redundancy**: the station-side `CommandSanitizer` handles pre-send validation, the vehicle-side `SafetyValidator` handles post-receive validation. Both ends independently run the same SafetyEnvelope computation logic, with the vehicle holding final veto power
- **Multi-carrier bonding, not failover**: MediaServo's `MultiPathTransport` component should support bonding mode (parallel send + receiver dedup), with latency determined by the fastest path rather than failover's switching overhead
- **Latency-visualization HMI encoding**: encode via color, shape, transparency and other dimensions onto the vehicle-position indicator. MediaServo Client teleoperation UI's `TeleopHmiOverlay` component should adopt a similar multi-dimensional latency-encoding strategy
- **MRM four-level degradation standardization**: Phase 1(warning) → Phase 2(decelerate) → Phase 3(stop) → Phase 4(diagnose) as the default `EmergencyHandler` implementation in the MediaServo SDK
- **DTLS 1.3 + bidirectional Token auth**: all media and control channels must pass DTLS 1.3 encryption + application-layer bidirectional Token validation

### [Adapt] Adopt After Modification (supplement)

- **ASIL-D → graded functional safety**: define multiple functional-safety levels QM/A/B/C/D, choosing the appropriate level per deployment scenario. Low-speed indoor robots use QM, public-road low-speed vehicles use ASIL-B, high-speed scenarios use ASIL-C/D
- **360° GPU stitching → front-end canvas compositing**: transmit multiple independent video streams, composite a 360° view on the operator side with WebGL/Canvas 2D. Lowers vehicle-side compute requirements; front-end can switch display layout on demand
- **Latency-aware encoding policy → WebRTC GCC dynamic bitrate**: leverage WebRTC's built-in GCC, combined with the Transport-CC RTP extension for bitrate adaptation. The encoding plugin should support an `EncoderDynamicConfig` trait
- **RLHF data loop → TeleopDataCollector + ReplayAnalyzer**: MediaServo initially implements a lightweight version of operation-log recording, replay analysis, and operation-quality assessment. `TeleopDataCollector` records video + control + telemetry data aligned by timestamp
- **Professional-driver training → OperatorLevel tiered certification**: Operator Level 0(observer) / 1(path guidance) / 2(low-speed control <30km/h) / 3(urban full-speed) / 4(full-scenario)
- **Fatigue management → SessionDurationPolicy**: a configurable policy — different OperatorLevels have different duration limits, adjustable per control scenario

### [Avoid] Known Pitfalls / Inapplicable Scenarios (supplement)

- **TÜV safety-certification time cost unbearable**: independent third-party safety assessment needs 12-24 months, costing hundreds of thousands to millions of euros. MediaServo initially needs only functional-correctness verification; safety certification is deferred to the commercialization stage
- **Multi-SIM hardware cost not generalizable**: 3-4 cellular baseband modules + data-plan monthly fees greatly increase per-vehicle cost. MediaServo should offer configurable redundancy levels: single-card / dual-card / multi-card
- **Per-city separate-approval model not scalable**: Vay's business model is significantly affected by local regulation. MediaServo is a technology platform, its responsibility boundary is at the software/protocol layer, not road-operation permits
- **Latency visualization cannot replace network optimization**: HMI-layer latency compensation is the last line of defense. MediaServo must work on all three layers — network layer (TURN relay selection), transport layer (WebRTC GCC), HMI layer
- **ODD taking network as the first constraint**: MediaServo's `NetworkQualityProfile` must be the first decision factor for feature degradation
- **Trade-secret limitations**: Vay's core implementation is undisclosed; MediaServo can only borrow architecture-design principles, the specific implementation must be self-designed
- **Power draw and heat dissipation cannot be ignored**: multi-baseband modules + GPU encoding may overheat the vehicle in hot summer environments

### [Adopt] Directly Borrowable (supplement two)

- **Control-command four-level timeout response**: Phase 1(50-100ms hold last command) → Phase 2(100-300ms steering-center + throttle-zero) → Phase 3(300-2000ms full-power braking) → Phase 4(>2000ms safe stop + hazards). MediaServo's `TimeoutHandler` component should implement this four-level timeout as the default safety policy
- **Vehicle-side watchdog timer**: a hardware watchdog independent of the main controller, triggering a vehicle safe-stop when the safety controller is unresponsive. The MediaServo Vehicle Agent should implement a two-level watchdog of `HardwareWatchdog` and `SoftwareWatchdog`
- **Control-command source authentication**: each control command carries a unique operator ID and sequence number, with the vehicle validating source legitimacy. MediaServo's `ControlCommand` struct should include an `operator_id` field

### [Adapt] Adopt After Modification (supplement two)

- **Vay's 6-camera 360° scheme → MediaServo's modular multi-camera architecture**: Vay uses a fixed 6-way camera setup (front + front-left/front-right + rear-left/rear-right + rear). MediaServo should design a configurable `CameraConfig` camera layout, supporting 2-8 cameras, layout determined by the deployment scenario
- **Vay's dedicated hardware driving station → MediaServo's universal Client control mode**: Vay uses a custom force-feedback steering wheel + hydraulic pedals + multi-screen. MediaServo Client should support multiple control modes — gamepad (entry), keyboard+mouse (standard), wheel+pedals (professional), touchscreen gestures (mobile)
- **Vay's Safety Controller dual-end heartbeat → MediaServo's RTCDataChannel heartbeat channel**: a dedicated heartbeat signal between the dual Safety Controllers (safety-critical). MediaServo uses an independent RTCDataChannel heartbeat channel (2Hz, 5-byte minimum packet), achieving equivalent functionality at the application layer

### [Avoid] Known Pitfalls / Inapplicable Scenarios (supplement two)

- **Vay's ASIL-D safety controller cannot be directly procured**: Infineon Aurix TC4xx series MCUs have long procurement lead times (12-20 weeks), high volume prices ($30-50/piece), and a complex dev toolchain. MediaServo should initially prioritize industrial-grade MCUs (STM32H7 / NXP i.MX RT series) to implement a functional-safety subset
- **Vay's remote-driver training system cannot be replicated**: multi-stage training (theory + simulator + real vehicle + latency-adaptation training) needs weeks and dedicated venues. MediaServo's operator certification should be based on simulator assessment and online training, lowering the training barrier
- **Vay's business model is bound by insurance and law**: remote-driving insurance pricing and accident-liability determination have no unified global standard. As a technology platform, MediaServo must clearly define the responsibility boundary in its service agreements

**Overall Rating**: ★★★★★ (5/5)
— As the industry benchmark for commercialized remote driving, Vay provides the most complete teleoperation safety-architecture reference. The Safety Tunnel's independent safety-boundary computation, dual Safety Controller peer validation, multi-carrier parallel transport, and latency-visualization compensation are the core reference sources for MediaServo's teleoperation safety design. The only regret is that the core technology is closed-source, so the specific implementation cannot be directly reused.
**Related Decisions**: D73, D77, D15, D96, D117, D62-D63


## Appendix
### A. Safety Tunnel Safety-Boundary Computation Model
```
Input: position (lat, lon), map data (road curvature), vehicle state (speed, yaw rate), road-surface estimate (μ)
Output: SafetyEnvelope { max_steering_angle, max_yaw_rate, max_acceleration, max_deceleration }

Computation logic (independent of command content):
  1. max_steering_angle = f(road_curvature) × μ_factor × speed_factor
     — The curvier the road → the smaller the allowed steering angle
     — The more slippery the surface → the larger the safety margin (μ factor)
     — The higher the speed → the smaller the allowed steering angle (∝ 1/v²)
  2. max_yaw_rate = sqrt(max_lateral_acc × road_curvature)
     — Based on lateral-acceleration upper limit (default 3.0 m/s², rainy 1.5 m/s²)
  3. max_acceleration = min(engine_torque_curve, μ × g × weight_transfer)
     — Not exceeding road-surface friction limit and engine torque curve
  4. max_deceleration = min(brake_capacity, μ × g × weight_transfer)
     — Not exceeding brake-system capacity and road-surface grip

Validation flow:
  for each command:
    if |command.steering_angle| > envelope.max_steering_angle → REJECT
    if |command.yaw_rate| > envelope.max_yaw_rate → REJECT
    if command.acceleration > envelope.max_acceleration → REJECT
    if command.deceleration > envelope.max_deceleration → REJECT
    else → FORWARD (with envelope-bounded values if LIMIT mode)
```

### B. MRM Four-Level Degradation Timing
| Phase | Trigger Condition | Action | Duration | Recovery Condition |
|-------|---------|------|---------|---------|
| Phase 0 | Normal control | Full-function control | Unlimited | N/A |
| Phase 1 | Latency >200ms or loss >2% | HMI yellow alarm + audio prompt | Until recovery or escalation | Latency recovers <100ms and loss <1% |
| Phase 2 | Latency >400ms or loss >5% (sustained 3s) | Auto-decelerate to 15km/h, maintain lane | 10-30s | Latency recovers <200ms |
| Phase 3 | Latency >800ms or loss >10% (sustained 1s) | Find safe spot to stop + hazards | 20-60s | On-site rescue or remote diagnosis |
| Phase 4 | Stop complete | Remote diagnosis + attempt recovery + call on-site | Until recovery | Remote diagnosis passed and network recovered |

### C. Multi-Carrier Parallel Transport Dedup Algorithm
```rust
struct MultiPathReceiver {
    last_seq: u16,
    dedup_window: BTreeSet<u16>,
}

fn on_packet(&mut self, seq: u16, data: &[u8]) -> Option<&[u8]> {
    // Take the first-arriving packet, dedup by sequence number
    if self.dedup_window.contains(&seq) {
        return None; // Already received (arrived earlier on another channel)
    }
    // Handle sequence-number wraparound (u16 0-65535)
    let is_newer = seq.wrapping_sub(self.last_seq) < 32768;
    if !is_newer && seq != self.last_seq {
        return None; // Old packet, drop
    }
    self.last_seq = seq;
    self.dedup_window.insert(seq);
    // Sliding-window cleanup (keep the most recent 1000 sequence numbers)
    if self.dedup_window.len() > 1000 {
        let oldest = seq.wrapping_sub(500);
        self.dedup_window.retain(|&s| s.wrapping_sub(oldest) < 500);
    }
    Some(data)
}
```

### D. Vay vs MediaServo Safety-Architecture Mapping
| Vay Component | MediaServo Correspondent | Implementation Priority |
|----------|---------------|-----------|
| Safety Tunnel | SafetyEnvelope trait | P0 (core safety) |
| Dual Safety Controller | CommandSanitizer (station) + SafetyValidator (vehicle) | P0 (dual-end validation) |
| MRM four-level degradation | SessionManager + EmergencyHandler | P0 (fault handling) |
| Latency visualization | TeleopHMI Overlay (latency → color/shape/transparency) | P1 (UX enhancement) |
| Multi-carrier parallel | MultiPathTransport (bonding mode) | P1 (reliability) |
| RLHF data loop | TeleopDataCollector + ReplayAnalyzer | P2 (quality analysis) |
| Driver fatigue management | OperatorSessionPolicy | P2 (operator management) |
| Multi-camera 360° stitching | MultiStreamCompositor (front-end canvas compositing) | P2 (visual experience) |
