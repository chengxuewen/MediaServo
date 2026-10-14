# WebRTC Weak-Network Emulation Tooling Landscape

> Type: research archive (Explanation). Verification point = 2026-09-08 (stars/archived/pushed all measured via the GitHub API).
> Draft basis = the tool list provided by the team; this document verifies and corrects each entry (corrections marked with **⚠**).
> This project's implementation form = main repo ms_rtc `scripts/weaknet.sh` (tc/netem + docker sidecar),
> usage manual = main repo `docs/modules/development/weaknet-harness.md`, discipline = skill `weaknet-testing`.

## 1. System-level / General-purpose Tools

| Tool | Platform | Capabilities | Pros | Cons | Status (measured 2026-09) |
|------|------|------|------|------|---------------------|
| **tc + netem** (iproute2) | Linux kernel | delay/jitter/loss (incl. gemodel bursts)/reorder/duplicate/corrupt/limit; bandwidth needs an htb/tbf combination | Kernel-native, per-port (u32) precise, the substrate of all advanced tools | Pure CLI; **GSO swallows loss granularity** (`ethtool -K gso off`); root or container NET_ADMIN | **Cornerstone**, evolves with the kernel |
| Clumsy | Windows | loss/latency/throttling/duplication/reordering/tampering | No install, GUI, system-level any UDP/TCP | Windows only (WinDivert) | Active, 6.2k★ (pushed 2025-11) |
| Network Link Conditioner | macOS/iOS | bandwidth/latency/loss/DNS | Official Apple tool, preset scenarios | Apple-ecosystem only | Officially maintained (no GitHub counts) |
| Comcast | Linux/macOS | thin wrapper over tc/ipfw | Simple | Basic features; is still tc under the hood | **10.5k★ but low frequency** (last push 2025-03) — the claim "continuously updated by the author" is optimistic **⚠** |
| Wondershaper | Linux | htb bandwidth shaping (+ optional netem) | One-line rate limiting | No fine-grained impairment surface | Stable/low-dimensional |

## 2. Dedicated / Advanced Tools

| Tool | Positioning | Pros | Cons | Status (measured) |
|------|------|------|------|-------------|
| **ATC (Augmented Traffic Control)** | Facebook gateway + Web UI | No install on the controlled endpoint, team collaboration | **Repository archived** (facebookarchive, unmaintained since 2018-04) — "continuously maintained by FB" does not hold **⚠** | Archived |
| **Toxiproxy** | Chaos proxy (REST API) | 12.3k★ active, the benchmark for automation integration | **TCP-only, UDP unavailable** (issue #54 long open) → not applicable to WebRTC media streams; can only hit the signaling WS | Active but a wrong-scenario fit |
| WANem | LiveCD/VM wide-area network simulation | High-fidelity WAN | Heavy deployment; stale tech stack from a 2026 vantage | Effectively stalled |
| QNET | Tencent mobile weak-network app | No root needed, 2G/3G/subway presets | Mobile only, closed source, requires login | Active on mobile (no public repo) |
| ToNetScale | Docker network topology simulation | A container-orchestration shell over tc | **No longer available in GitHub search (re-verified 2026-09: 404/deleted)** **⚠** (cited in an earlier research round) | Defunct |
| **udptoxy** | Rust UDP relay + **TUI live dialing** (l/j/p/r keys) | No root needed, per-port bidirectional, latency/jitter/loss/rate limiting | 7★ community project (loss fidelity needs self-calibration); the relay form only covers redirected flows | Active (2026-08); listed as roadmap F3 in this project |

## 3. Proxies / Development Aids (applicable to WebRTC media = no, to avoid the trap)

| Tool | Verdict |
|------|------|
| Charles / Fiddler | HTTP(S) layer proxies; UDP media never passes through them — **not applicable** |
| Chrome/Firefox DevTools network throttling | Acts on URLLoader (HTTP) and **does not affect WebRTC UDP media** — a common trap; don't use it to "simulate a weak network" and then wonder why the picture doesn't change at all |
| toxiproxy-dashboard and similar Web UIs | Bundled with Toxiproxy, subject to the same TCP-only limitation |

## 4. In-Code Emulation (the right answer for automated testing)

| Facility | Applicability | Key points |
|------|------|------|
| **webrtc.org `test/network/`** (NetworkEmulationManager / emulated_network / schedulable_network_behavior) | C++ integration tests / performance tests | In-process per-packet emulation (latency/loss/bandwidth/cross traffic/scheduler mutation); unusable on a real deployment path |
| **Pion vnet** (`pion/transport/vnet`) | Go unit tests | Virtual NIC/link topology, programmable loss/reordering/latency; standard issue in the pion ecosystem |
| mediasoup side | Node/Rust surface | `WorkerSettings.libwebrtcFieldTrials` (BWE behavior injection); transport `setMax{In,Out}goingBitrate/setMinOutgoingBitrate`; stats `rtpPacketLossSent/Received` (fractionLost f64); **mediasoup-demo's applyNetworkThrottle = a @sitespeed.io/throttle wrapper (a CLI skin over tc), and its localhost mode has only delay, no loss/rate**; upstream's own CI uses zero netem (RTCP logic is unit-tested with synthetic inputs) |
| aiortc / browser getStats | Observation surface (not emulation) | packetsLost/jitter/availableIncomingBitrate; watch out for **OOO deduction** (a NACK-retransmission artifact) and SVC layer downgrade polluting fractionLost |

## 5. tc Orchestration Layer (options when writing scripts/CI, measured activity)

| Tool | Stars | Activity | One-liner |
|------|----|--------|--------|
| pumba | 3.1k | ✅ 2026-09 | docker chaos CLI (runs netem/htb inside containers); a textbook for fail-closed guards and replace semantics |
| tcconfig | 853 | ✅ 2026-04 | pip-installable file-level tc orchestration (auto-composes htb+netem+ifb) |
| docker-tc | — | ❌ ~2020 | precedent for sidecar bidirectional (ifb); the mechanism is copyable, the project itself is unmaintained |
| quic-network-simulator | — | ✅ in use by IETF interop | per-link containers + self-describing config echo — a mature template for the scenario form |
| sitespeed.io throttle | — | ✅ active | half-RTT input convention + an external-qdisc "attach-and-leave" discipline + **change does not clear rate properties, so a sentinel replay is needed** (evidenced in the netem_impair comments) |
| Mahimahi | 280 | ✅ 2025 | record/replay link emulation (aimed at academic congestion research); real-time UDP is not its strength |
| This project's weaknet.sh | — | ✅ | direct tc use + sudo-free sidecar + room-level targeting + baseline-paired verdicts + three-tier fuses (the full dev-surface set; see the main repo docs) |

## 6. Hardware Impairment Instruments

HoloWAN / Spirent / Apposite Netropy and similar = the ceiling for real-link emulation (RF modeling, geographic latency), but procurement is heavy; at the dev stage, the netem family plus real hardware (Jetson / phone + adb network shaping) already covers the requirements, leaving hardware for acceptance/certification scenarios.

## 7. Quick Selection Guide (with this project's mapping)

| Scenario | First choice | This project's landing point |
|------|------|-----------|
| Linux dev single host (this project's primary form) | direct tc/netem (root or a docker NET_ADMIN sidecar) | ✅ delivered: the agent dual-surface `mediaservo-weaknet` (`msrtc.sh weaknet apply --profile remote-burst --stream <room name>` / the `serve` panel; D277) |
| Vehicle-side Jetson (with root) | tc applied on the device NIC (wlan/eth; egress=uplink, ingress→ifb=downlink) | agent M4′ made permanent (T12/T13: msrtc-weaknet scp+yaml port enumeration; the ifb base landed with T9) |
| Mobile/Web demos wanting a "dialing feel" | clumsy (Win) / NLC (mac) ready-made; Linux/no root → udptoxy TUI | roadmap F3 |
| Server-side API/signaling chaos | Toxiproxy (TCP domain) | usable for the signaling WS; media domain excluded |
| Code-level automated integration tests | webrtc test/network, Pion vnet, the mediasoup synthetic-RTCP unit-test route | upstream precedent (the worker TestTransportCongestionControlServer form) |
| CI regression on every PR | full-hit lo netem + true-value assertions on the injector side (netem dropped>0) | ✅ delivered: the submodule `test-weaknet` job |
| Shared team network environment | the ATC approach (tool now dead) → a self-built gateway script, or each endpoint applying it itself | not needed (dev single-host form) |

## 8. Deep Pitfalls List (measured in this project, tool-agnostic)

1. GSO large-segment aggregation swallows loss; single-port lo delay is bidirectional = a 2× convention; `netem limit` defaults to 100 packets.
2. Always read back after applying and prove the hit (tc -s fingerprint + leaf counter delta) — a silent no-op is the number-one incident in this domain (PIT-184).
3. `tc qdisc change` has full-replacement semantics, and independent rate properties are not cleared → replay all parameters plus a sentinel (evidenced by sitespeed; measured on this kernel: counters are not reset).
4. The server side does not mark a UDP transport as dead (mediasoup/IceState semantics): use the ICE tuple as the liveness predicate; don't trust closed()/stats reachability (PIT-185 family).
5. Use baseline-paired retention ratios as the verdict (ClickHouse no-fault twin / J2 >0.95); absolute thresholds yield false green / false red from compute differences.
6. The netem seed needs iproute2≥6.6 and **only reproduces the loss sequence for the GE model** (uniform/latency/corrupt have no seeded entropy).
7. gaiadocker/iproute2 is the 2015 tc release: no seed, a non-empty root tree cannot be replaced, `qdisc show parent` reads empty — choose the image version or adapt the script.
8. Split the verdict tracks for control flows vs targeted flows: retention>0.95 holds only for non-targeted flows; keep ≥25s of dwell for the recovery half-verdict (BWE ramp-up).

## Reference Links

- iproute2/tc-netem: https://man7.org/linux/man-pages/man8/tc-netem.8.html
- clumsy: https://github.com/jagt/clumsy | Comcast: https://github.com/tylertreat/comcast
- ATC (archived): https://github.com/facebookarchive/augmented-traffic-control | toxiproxy: https://github.com/Shopify/toxiproxy
- udptoxy: https://github.com/9mothers/udptoxy | pumba: https://github.com/alexei-led/pumba | tcconfig: https://github.com/thombashi/tcconfig
- quic interop: https://github.com/quic-network-simulator/quic-network-simulator
- sitespeed throttle: https://github.com/sitespeedio/throttle (origin of the demo engine)
- webrtc in-code emulation: https://webrtc.googlesource.com/src/+/main/test/network/ | Pion vnet: https://github.com/pion/transport (vnet/)
- mediasoup surface: WorkerSettings.libwebrtcFieldTrials, Transport.setMax*Bitrate, demo applyNetworkThrottle (versatica/mediasoup-demo); reproduction recipes issues #1115/#1536 (versatica/mediasoup)
- In-project: main repo `docs/plans/weaknet-simulation-research.md` (landscape + citations) / `weaknet-harness/` (plan + evidence) / `weaknet-followup-roadmap.md` (F1-F4)
