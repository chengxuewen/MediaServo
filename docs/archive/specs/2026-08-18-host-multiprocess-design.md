# Host Multiprocess Architecture Design — capturer/stitch/streamer/recorder/controller/emergency/monitor + OxMgr

**Date**: 2026-08-18
**Status**: Design discussion confirmed (completed interactively via brainstorming)
**Prerequisites**: D102 (host IPC tokio::mpsc Phase 1 → iceoryx2 SHM Phase 2), D222-D243 (link's four SDKs), D235 (decentralized Registry, cross-process discovery deferred to Phase 2), deck closed_loop e2e (capture → FrameBus → recorder proven), host control.rs (P2P DataChannel control prototype)

## 1. Background & Motivation

**Scenario**: a vehicle (Jetson) pushing **all 8-9 cameras live**, the cockpit pulling live for remote control; vehicle surround-view stitching (AVM) streamed; an x86-Windows machine running CARLA simulation sources; cockpit HMI frames streamed. Today the host is single-process (mediaservo-host main.rs, 770 lines, merging capture+encode+push+signaling+config).

**Core motivation**: **crash isolation + independent restart** — a capture driver crash must not take down the push; one media process crashing must not affect other streams or control; a single module can hot-restart. The thread model cannot provide real isolation (panic contagion/memory stomping); the process model is the only choice.

**Deployment matrix** (the process manager must be cross-platform):
| Form | Platform | Push load |
|------|----------|-----------|
| Vehicle Jetson | linux-aarch64 | 8-9 cameras + 1 surround stitch ≈ 10 live streams |
| Edge box | linux-amd64 | 1-4 streams |
| CARLA simulation machine | x86-Windows | 1-4 simulated cameras |
| Cockpit HMI | TBD | 1 HMI stream |

## 2. Process Inventory & Links

```
OxMgr (process supervisor: restart policies/health checks/log rotation/CPU-RAM metrics/file-watch config hot apply)
├── host-capturer × N      one per camera → FrameBus publishes I420 (ts_mono/ts_epoch aligned)
├── host-streamer × N      one per stream: subscribes any link topic → encode → WebRTC push
│                          (sources are not limited to host-capturer — third-party publishers push the same way; negotiation WS → local bus)
├── host-recorder × 1      aggregation: subscribes any stream (camera streams + stitch streams) → encode each to disk
├── host-controller × 1    control channel: one PC (opens DC, no track) + multiple DataChannel labels
├── host-emergency × 1     e-stop: standalone process + standalone PC + local fallback (highest-reliability channel)
└── host-agent × 1         signaling gateway (single aggregated WS) + topology/data-flow/signaling-state monitoring
                           + cloud config mirror + remote reporting to Server

External producers (not managed by OxMgr, own lifecycle, roslaunch-managed):
├── ROS stitching node (e.g. surround-view AVM) attaches via the link SDK (C++ bindings) → subscribes N cameras
│   → GPU stitch → publishes 1 panorama topic → host-streamer pushes + host-recorder records
└── ROS vision node attaches via the link SDK → subscribes each camera stream → detection (obstacles/persons/lane lines etc.)
    → publishes a vision-results topic (object bboxes + hint text + suggested display color, frame-associated ts/seq)
    → host-streamer subscribes and forwards over the DataChannel → cockpit HMI overlay renders
```

**link role = shared library (not a process)**: FrameBus (iceoryx2 SHM) + decentralized Registry (D235: attach-as-register, enumerated via iceoryx2 service discovery) + static ACL + capability tokens, embedded in each process.

**Media links**: `host-capturer →(FrameBus I420)→ host-streamer / host-recorder / external nodes (ROS stitch)`; `external node →(FrameBus 1 panorama)→ host-streamer(stitch) / host-recorder`
**Control link**: `cockpit/Server →(WebRTC DataChannel)→ host-controller` (P2P direct; SFU goes through the Server data domain)
**Vision-results link**: `ROS vision node →(FrameBus vision topic)→ host-streamer →(standalone transport B: DataChannel label "vision")→ cockpit HMI overlay`
— video goes RTP (transport A), vision results go **the DC of the standalone transport B** (mediasoup official: client-side send/recv must use separate transports + libwebrtc SCTP and RTP scheduling interfere with each other → media/data separation); "video + vision" association relies on ts/seq frame association (not transport association)
**Signaling link**: `each process →(WS→127.0.0.1:PORT)→ host-agent (signaling gateway) →(single WS)→ Server` — one host on the Server side = one peer session

## 3. Key Decision Records

### D-H1: Process manager = OxMgr (lightweight Rust, a cross-platform PM2 alternative)
- **Source**: github.com/Vladimir-Urik/OxMgr (Rust, 226 stars, active as of 2026-08-17); PM2's current state; Jetson constraints C22/C23
- **Pros/cons** (vs pm2/systemd/homegrown):
  - pm2: cross-platform but needs the Node runtime (adds ~50MB+ on the vehicle), not a vehicle-side ecosystem
  - systemd: strongest on Linux but **the Windows CARLA machine has no systemd** (split ops), and trimmed Jetson images may lack it
  - Homegrown Rust supervisor: ecosystem-consistent but reinvents the wheel (restart/logging/daemonization 3-6 months), violating C18/ponytail
  - **OxMgr**: hits all three platforms (Rust cross-compile) + lightweight single binary + restart policies/health checks/log rotation/CPU-RAM metrics out of the box + oxfile.toml config-as-code + **file-watch config hot apply (the key mechanism for cloud config)** + PM2 ecosystem compatibility
- **Risk hedge**: OxMgr handles lifecycle only; process topology/data-flow/signaling monitoring is handled by the homegrown host-monitor → monitor interfaces survive a future pm2/systemd switch
- **Impact**: oxfile.toml declares the process group (capturer/stitch/streamer/recorder/controller/emergency/monitor), restart_policy=always/on-failure

### D-H2: Frame bus = RAW I420 (not encoded frames)
- **Rationale**: deck closed_loop already proved the same chain (capture → FrameBus I420 → recorder to disk); streamer/recorder encode independently (dual-encoder overhead acceptable under Jetson hardware NVENC/MMAPI); recording quality decoupled from push bitrate; 1080p30 ≈ 93MB/s/stream, no pressure for SHM zero-copy (iceoryx2 measured 684MB/s per service, 10 independent topic services)
- **Rejected**: H264 bus (recording quality = push quality, degraded local recordings at low bitrates); dual bus (YAGNI — add when dual-bitrate demand appears)

### D-H3: Control channel = WebRTC DataChannel (multi-label)
- **Rationale**: latency (SCTP over UDP vs WS/TCP) + reuse WebRTC infrastructure + decoupled from push negotiation (controller owns a separate PC) + per-channel reliable/ordered configurability (e-stop reliable / gimbal partial-reliable)
- **Current state**: mediaservo-webrtc already has create_data_channel/on_data_channel/RTCDataChannelRx (webrtc-sys backend); host control.rs has a DC usage prototype (P2P E2E validated); **the Server SFU data domain (DataProducer/DataConsumer) is not implemented — SFU-mode control needs the Server to add it later**
- **Process boundary**: DC labels = channel boundaries (chassis/gimbal/light); **emergency is a standalone process + standalone PC** (a PC crash doesn't affect e-stop + local fallback connects straight to actuators)

### D-H4: Monitoring topology = declarative desired + discovery-based actual
- **Desired state**: the local mirror of cloud-pushed config (config push → monitor storage) or a local oxfile/topology declaration
- **Actual state**: oxmgr list (process liveness) + link Registry/iceoryx2 service discovery (inter-process connections/publisher enumeration) + FrameBus stats (data flow) + streamer signaling state
- **Comparison alerts**: N capturers desired, N-1 actual → alert "capturer-cam3 missing"

## 4. host-monitor Monitoring Dimensions

| Dimension | Data source | Output |
|-----------|-------------|--------|
| Node relationships (topology) | oxmgr list + link Registry enumeration | desired-vs-actual comparison, missing-node alerts |
| Data flow | FrameBus stats (pub/sub, frame rate, bandwidth/stream) | Stream health: frame-rate targets, stall detection, bandwidth curves |
| State | per-process health (oxmgr CPU/RAM/restart count) + app-level heartbeat | Status panel, abnormal-restart counters, OOM detection |
| Signaling state | streamer/controller/monitor WS/PC connection state (connected/disconnected/ICE state) | Signaling connection matrix, disconnect alerts |

**Output forms**: local Web/CLI (vehicle-side debugging) + remote reporting to Server (cloud dashboard reuses the admin 9800 extension).

## 5. Cloud Remote Config Loop

```
Cloud Server ──(signaling WS extension: ConfigPush message + PSK/JWT auth + audit)──▶ host-monitor
host-monitor ──(write oxfile/process config)──▶ OxMgr file-watch detects → restarts the affected processes
host-monitor ──(desired-state mirror)──▶ topology verification/alert loop
```

- **Channel**: signaling WS extension (reuses the connection + existing auth; config push/e-stop commands share the extension surface)
- **Security**: remote config is security-sensitive (remotely changing capture/push parameters) → existing PSK/JWT auth + audit logs (C15/C16 discipline)
- **Effect**: process parameters hot-apply (OxMgr file-watch debounce restart); link changes (adding/removing streams) start/stop processes dynamically

### D-H5: Process naming convention — the host- prefix family
- All host unit processes are named with the `host-` prefix: host-capturer/host-stitch/host-streamer/host-recorder/host-controller/host-emergency/host-agent
- Multi-instance disambiguation: instance suffixes (host-capturer-cam0, host-streamer-cam0); organized under an OxMgr namespace (namespace: host)
- **Rationale**: a unified family identifier for "the vehicle-side host unit"; distinct from Server-side processes (mediaservo-server); the agent name is unclaimed (gateway clashes with GatewayComponent, signaling with the module name)

### D-H6: Single WS signaling bus (WS proxy mode) — zero Server changes
- **Form**: each process WS-connects to local 127.0.0.1:PORT → host-agent acts as the WS gateway (local accept + single remote WS + bidirectional forwarding + session disambiguation) → the Server sees one peer = one vehicle
- **Rationale**: the multi-peer semantics lack a "vehicle" aggregation layer (kick/credentials/pull routing/admin views are all per-device); zero Server changes (multi-stream produce = multiple transports under one peer, natively supported by mediasoup; P2P relay exchanges only SDP/ICE)
- **Impact**: per-process signaling addresses become one config item (connect to the local gateway); **RoomJoin is intercepted by the agent** (signaling.rs evidence: a RoomJoin received inside the relay loop is silently dropped — child processes must not join per-process; the agent joins once as the whole-vehicle identity); response routing maps back to local connections by msg_peer_id/transport_id (SFU) or negotiation-ownership tracking (P2P relay); host-agent doubles as the signaling gateway (mixed responsibilities acceptable — signaling state monitoring is naturally at hand); the controller's PC negotiation rides the bus (holds no independent signaling)
- **Evolution**: with real multi-vehicle, upgrade to Server device aggregation (direction 2); the agent gateway transitions smoothly

### D-H7: Surround-view stitching external by default — third-party nodes join via the link SDK
- **Decision**: the host does not embed a stitch process; stitching is done by an external node (e.g. ROS) via the link SDK (attach → subscribe camera streams → publish the panorama stream), and host-streamer/host-recorder subscribe to its result for push/record
- **Rationale**: stitching algorithms (perspective transforms/blending/camera calibration) belong to the ROS/algorithms team, not the media servo platform; the link SDK (the C++ binding matrix — the ROS ecosystem) is precisely the third-party integration contract surface (D222-D243); host-process and algorithm-process lifecycles stay decoupled (roslaunch manages ROS, OxMgr manages the host family)
- **Security**: external node attach goes through link ACL (D237 static ACL) + capability tokens (D238) — ROS nodes get configured subscribe/publish permissions
- **Impact**: host-streamer/host-recorder inputs generalize to "any link topic" (sources not limited to host-capturer); FrameMeta.format supports I420 and variants

### D-H8: Vision processing results = metadata streams, forwarded over the streamer's DC (cockpit HMI overlay)
- **Decision**: the ROS vision node publishes a detection-results topic via the link SDK (object bboxes + hint text + suggested display color + frame-associated ts/seq); host-streamer subscribes and forwards over **the DataChannel of the same PC (label "vision")**; the cockpit HMI renders the local overlay (video RTP + detection DC on the same connection)
- **Rationale**: detection results are frame-level metadata, not video frames — DC transport (low latency, frame-associated); the overlay renders cockpit-side (the vehicle does not burn in annotations; no loss of bandwidth or quality); "video + vision on one connection" keeps the HMI naturally aligned (no extra subscription relations)
- **Channel semantics**: the vision DC is an information-display stream (not a control stream) — not merged into the controller's PC (control/info separation); it hangs off the streamer process's **standalone transport B** (mediasoup official: mediasoup-client/libmediasoupclient by design require send/recv on separate transports + libwebrtc SCTP and RTP scheduling on one PC interfere with each other → media/data transport separation; controller/emergency pure-DC transports are naturally compliant)
- **Message format**: JSON to start (object arrays: class/confidence/bbox/text/color); small scale (10-30Hz detections per stream); frame association via ts_mono/seq (FrameMeta-aligned semantics)
- **Security**: vision node attach goes through link ACL + capability tokens (same as D-H7)

### D-H9: host binary as thin wrapper — business config model + ops entry (not resident)
- **Decision**: host (a Rust CLI, executed on demand, not resident) carries the business config model (host.toml: cameras/streams/record/control business semantics) + the translator (business → process topology + oxfile.toml) + an oxmgr CLI proxy (business-view ops); **it does not re-implement process management** (OxMgr directly manages the host-* processes)
- **Startup chain**: systemd/TaskScheduler → oxmgr daemon (OxMgr's own service) → host-* processes; local ops `host start/stop/status/ps` (camera/stream views) → proxy to oxmgr
- **Config chain**: cloud ConfigPush → host-agent → host.toml → `host apply` (translator) → oxfile.toml → OxMgr file-watch hot apply (add/remove streams = incremental apply)
- **Rationale**: the cloud pushes business config, not process-management config (restart_policy etc. are ops details); the translation layer on the host side keeps OxMgr swappable (the translator's output target can become systemd/pm2); the same entry on the Windows CARLA machine
- **Command surface**: host init/start/stop/restart/status/apply/doctor/version

## 5.5 Security Design (authn/authz/encryption panorama)

### D-H10: inter-node link authorization — authorization files + long-lived tokens (fixed)
- **Form**: host init issues long-lived Ed25519 token files (e.g. /etc/mediaservo/link/ros-vision.token); ROS nodes' link SDK attach loads them via from_file — **configure once, use forever, zero dynamic changes**
- **Trust root**: host-agent holds the signing key (the local trust root); each host node + ROS node holds a verifying key + its own token (issued via `host token issue --role --topic`)
- **Fixed-token hedge**: token claims carry topic whitelists (least privilege — a leaked ROS token can only publish what it declares) + file mode 0600 + revocation = re-issue and redeploy; Ed25519 is unforgeable
- **Not doing** (YAGNI): short TTL + auto renewal (the agent holds the signing key; an issuance endpoint can be added later)

### D-H11: host↔server dual identity classes + cockpit tiered authorization
- **Vehicle side (host, unmanned device)**: device credentials (device_id + device_secret) → Join auth → short-lived session token; a global PSK retained for dev/intranet (gradual); single-WS aggregation = single-point auth (one device session, one auth)
- **Cockpit side (client, operator)**: operator accounts (humans) — the dual identity-class model; **three-plus-one roles**:
  | Capability \ Role | viewer | operator | admin | dispatcher |
  |---|---|---|---|---|
  | Pull streams (video+vision) | ✅ | ✅ | ✅ | ✅ (any vehicle, incl. vision overlay) |
  | Audio conversation (conference) | ✅ | ✅ | ✅ | ✅ (any vehicle room) |
  | Control (chassis/gimbal) | ❌ | ✅ | ✅ | ❌ |
  | E-stop | ❌ | ✅ (strong audit: who/when/from which cockpit) | ✅ | ❌ |
  | Config push | ❌ | ❌ | ✅ | ❌ |
  | Status/alert viewing | ❌ | ✅ | ✅ | ✅ |
- **Authorization matrix**: the vehicle produces its own streams / receives config / receives control relays; the cockpit consumes authorized vehicles' streams / sends control / sends e-stop per role; the dispatcher may pull/monitor any vehicle (incl. vision overlay) + join any vehicle's audio room; vehicle×cockpit authorization relationship table (tenant isolation: vehicle A cannot see vehicle B)
- **Encryption**: signaling TLS (wss) mandatory in production; WebRTC DTLS/SRTP built in; SHM unencrypted (declared boundary of the trusted same-machine domain)

### D-H12: audio conference rooms — one room per vehicle, N-way full mesh
- **Model**: one audio room per vehicle (a per-vehicle conference); participants = the vehicle (host-audio process, always on) + the cockpit (viewer+) + the dispatch backend (dispatcher, any vehicle room, browser WebRTC); audio goes WebRTC opus tracks straight to the Server SFU (not via FrameBus)
- **Topology**: full mesh (each participant publishes 1 + subscribes to all others); opus ~50kbps/stream, ≤10 people/room — forwarding mode suffices (only larger scale needs server-side mixing — YAGNI)
- **Vehicle side**: the host-audio process (mic capture + speaker playback + opus codec — via the codec FFmpeg backend)
- **Server extension**: audio room management (join/leave + room member list + permission checks — dispatcher any vehicle / cockpit only authorized vehicles) + an admin dashboard audio panel (dispatch side)

### D-H13: application package layout — host package + SDK package released separately
- **Source**: crates/mediaservo-host becomes lib + multiple bins (host.rs/host-agent/host-capturer/host-streamer/host-recorder/host-controller/host-emergency/host-audio, 8 [[bin]]s sharing the lib); instance parameterization (host-capturer --camera cam0)
- **Deployment package**: /opt/mediaservo-host/{bin (8 processes + version-pinned oxmgr), etc/{host.toml,link/{signing.pem,*.token}}, run/{oxfile.toml,logs,oxmgr.db}, recordings/, identity.json (0600)}; the release tar contains a versioned top-level directory `{brand}-{target}-{ver}/`; to land directly at a prefix use `tar xzf <pkg>.tar.gz -C <prefix> --strip-components=1`
- **Release form**: split into two packages — mediaservo-host (vehicle side) + mediaservo-sdk (the existing install bindings layout); consumers are asymmetric (ROS/algorithms only need the SDK); version compatibility via explicit protocol-contract pairing (FrameMeta version/token claims schema), not implicit same-package
- **link token provisioning**: tokens are vehicle-deployment artifacts (issued by host init), not shipped in the SDK package; **single-file self-describing tokens** (verifying key + claims + signature merged) → deployment orchestration (scripts/Ansible) provisions them to ROS nodes; ROS-side startup args/env point to the path, used as fixed

### D-H14: order-independent robustness (startup-timing tolerance)
- **Principle**: all cross-process/cross-service interactions must be startup-order-independent — purely local (signature verification) / fault-tolerant retries (open_or_create/WS reconnect) / stall detection backstop (monitor frame rates); OxMgr start order is a performance hint, not a correctness dependency
- **Already built in**: attach signature verification is purely local (zero RPC timing dependency); iceoryx2 open_or_create is bidirectional (a subscriber starting first creates the service, the publisher joins later) + SystemInFlux transient 5 retries; service config unified (fixed in the API)
- **Enhancement 1**: SignalClient reconnect (exponential backoff + jitter — the coding-style retry_with_backoff pattern; reconnect → re-auth → session resume)
- **Enhancement 2**: host init exports ros_bridge.yaml (topic list + token paths) — the vehicle-side single source for ROS config, zero manual drift
- **Startup window vs failure**: monitor desired state + grace period (alert suppression during startup) distinguishes "starting up" from "broken"

## 6. Known Gaps & Follow-up Work

1. **Server SFU data domain**: mediasoup DataProducer/DataConsumer (prerequisite for SFU-mode DC control)
2. **External nodes (ROS: stitching + vision processing)**: join via the link SDK (D-H7/D-H8); frame sync/ts alignment owned by the external nodes (FrameMeta ts_mono/ts_epoch provides the alignment inputs)
2b. **Vision DC message format**: JSON to start; evaluate compact binary encoding when volume grows (high-frequency multi-object)
2c. **Audio conference (D-H12)**: one room per vehicle, N-way full mesh; remaining: Server audio room management (join/leave/members/permissions) + browser dispatch panel + the host-audio process
3. **Frame sync strategy**: stitch buffering alignment window (arrival-time differences across streams)
4. **Capture zero-copy**: zero-copy optimization for MIPI/CSI capture into SHM (immediate transfer)
5. **emergency local fallback**: actuator-direct forms (CAN/GPIO/serial) and controller redundancy
6. **Server side**: session scale for 10 streams/vehicle × N vehicles + admin views (cloud multi-vehicle monitoring)

## 7. Testing Strategy

- **Multiprocess e2e**: reuse the deck closed_loop pattern (capture publishes → FrameBus → recorder to disk), extended to real processes (spawned child-process pairs)
- **Fault injection**: kill any process → verify OxMgr restarts it + other processes unaffected + monitor alerts
- **Link regression**: host's existing 9/9 E2E + e2e_sfu 4/4 + codec_prefs 6/6 must not regress
- **Windows verification**: minimal closed loop on the CARLA machine (x86-Windows) with capturer capture + push

## 8. Scope Boundary

- **In this scope**: host multiprocess form design + process inventory + the monitor subsystem + OxMgr integration + cloud config channel design
- **Not in this scope**: client (cockpit pull, skeleton stage); Server SFU data-domain implementation; the stitch node (external team, link SDK contract surface); the client consumption side
