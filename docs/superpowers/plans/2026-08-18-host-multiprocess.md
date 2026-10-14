# Host Multiprocess Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the single-process mediaservo-host into an OxMgr-managed multiprocess architecture (host-capturer/streamer/recorder/controller/emergency/audio/agent + a thin host wrapper CLI), achieving crash isolation, a single WS signaling bus, link authorization, and order-independent robustness.

**Architecture:** One crate with multiple bins (shared lib) + FrameBus(I420) media bus + host-agent signaling gateway (single aggregated WS) + OxMgr process management + host.toml business config translated into oxfile.toml. External ROS nodes join via the link SDK (single-file self-describing tokens). Server side: zero changes (before Phase D); P2P mode validates the control DC first.

**Tech Stack:** Rust (edition 2024), iceoryx2 (FrameBus), tokio, mediaservo-webrtc (webrtc-sys backend), OxMgr (process manager), ed25519-dalek (link tokens)

**Spec:** `docs/superpowers/specs/2026-08-18-host-multiprocess-design.md` (14 decisions D-H1~H14 — this plan argues from the spec; executors must read both spec and plan)

## Global Constraints

- All cross-process interactions are order-independent (D-H14): local signature verification / open_or_create bidirectional compatibility / WS reconnect / stall detection
- Process naming: host- prefix family (D-H5): host-capturer/host-streamer/host-recorder/host-controller/host-emergency/host-audio/host-agent/host
- Instance parameterization: `host-capturer --camera cam0` (one binary, N instances, D-H13)
- Single config source: host.toml (business) → translator → oxfile.toml (process semantics) — oxfile is an artifact, not source (D-H9)
- C21 dependency direction: host-family processes must not depend on mediaservo-server / mediasoup; SFU integration goes through the WS signaling protocol
- C22: host runs natively (macOS/Linux host); running host tests inside Docker is forbidden
- Rust discipline: no unwrap in production, thiserror for library errors, SAFETY comment on every unsafe, clippy -D warnings
- link tokens: single-file self-describing (verifying key + claims + signature), file mode 0600 (D-H10/D-H13)
- Each phase independently verifiable + commit (TDD: test RED first → implement GREEN → refactor → commit)

---

## Phase Overview

| Phase | Name | Deliverables | Verification |
|-------|------|--------------|--------------|
| **A** | Foundation skeleton | lib + 8 thin bin shells + early host CLI + OxMgr integration | host init/start → all 8 processes start and exit |
| **B** | link enhancements | SignalClient reconnect + single-file self-describing token + ros_bridge export | Unit tests + disconnect-reconnect e2e |
| **C** | Media split | capturer (capture→FrameBus) + streamer (subscribe→push) + recorder (subscribe→disk) | Multiprocess closed-loop e2e (replaces single process) |
| **D** | Signaling bus | host-agent WS gateway (local aggregation + single remote WS) | One-vehicle-one-session e2e (zero server changes) |
| **E** | monitor | Topology/data-flow/signaling-state monitoring + desired-state comparison + reporting | monitor e2e (kill processes and verify alerts) |
| **F** | Control/e-stop | host-controller (DC multi-label) + host-emergency (local fallback) | P2P DC control e2e |
| **G** | Security | Device credentials + session token (server) + token issuance flow | Auth/authz e2e |
| **H** | Server extensions | Audio rooms + data domain + dispatcher permissions | Audio conference e2e + dispatcher panel |
| **I** | Deployment packages | host/sdk dual packages + install host + provisioning | Clean-machine install verification |

**Dependency chain**: A → B → C → D → E → F → G → H → I (F's DC control validates P2P first; the SFU data domain lands in H)

Execution strategy: **each phase is refined by the executing session into bite-size tasks of the same format as this plan before starting** (this plan provides phase-level task lists and interface contracts; Phase A is already refined to steps). Run the full regression at the end of each phase (existing e2e_sfu 4/4 + codec_prefs 6/6 + host 9/9 must not regress — after Phase C starts, host 9/9 adapts to the new form).

---

## Phase A: Foundation Skeleton (refined to steps)

**Goal**: Refactor crates/mediaservo-host into lib + 8 thin bin shells (each bin only "parse config → init logging → run placeholder → graceful exit"); the host CLI generates the oxfile and drives OxMgr to start/stop all processes. No media functionality is migrated (the existing single-process functionality stays untouched until Phase C — **lib.rs initially holds only shared infrastructure; main.rs's current functionality is parked as the host-legacy bin**).

**File structure**:
- `crates/mediaservo-host/src/lib.rs` (new): config parsing (initial host.toml schema), logging init, common process utilities
- `crates/mediaservo-host/src/bin/host.rs` (new): thin wrapper CLI (init/status/start/stop/apply/doctor/version — Phase A implements init/status/start/stop; apply/doctor land in Phase C/E)
- `crates/mediaservo-host/src/bin/host-agent.rs` and 6 more thin shells (Phase A placeholders: start → print role → wait for SIGTERM → exit 0)
- `crates/mediaservo-host/src/main.rs` (renamed `src/bin/host-legacy.rs`): the existing 770-line single-process functionality preserved verbatim (deleted after Phase C migration)
- `crates/mediaservo-host/Cargo.toml`: `[[bin]]` sections × 9 (host-legacy kept) + lib section

### Task A1: lib + bin skeleton

**Files:**
- Create: `crates/mediaservo-host/src/lib.rs`
- Create: `crates/mediaservo-host/src/bin/host.rs`, `host-agent.rs`, `host-capturer.rs`, `host-streamer.rs`, `host-recorder.rs`, `host-controller.rs`, `host-emergency.rs`, `host-audio.rs`
- Modify: `crates/mediaservo-host/Cargo.toml`, rename `src/main.rs` → `src/bin/host-legacy.rs`

**Interfaces:**
- Consumes: none (new skeleton)
- Produces:
  - `lib.rs`: `pub fn init_logging(role: &str)`, `pub fn run_placeholder(role: &str) -> Result<(), Box<dyn std::error::Error>>` (prints role + blocks awaiting SIGTERM/SIGINT → Ok)
  - `bin/host.rs`: `host init` (writes the `etc/host.toml` template), `host status` (calls oxmgr list and parses output), `host start` (oxmgr apply oxfile.toml), `host stop` (oxmgr stop namespace host) — CLI subcommands parsed manually via `std::env::args` (no clap in Phase A)
  - Initial `host.toml` schema: `[host] device_id = "..."` `[[cameras]] id = "cam0" source = "stub" fps = 30` `[[streams]] id = "cam0-stream" camera = "cam0" codec = "h264"` `[record] enabled = false` `[control] enabled = false`

- [ ] **Step 1: write the failing tests (bin existence + lib functions)**

`crates/mediaservo-host/tests/multiproc_skeleton.rs`:
```rust
#[test]
fn all_bins_declared() {
    // Read the Cargo.toml [[bin]] sections: must contain host, host-agent, host-capturer,
    // host-streamer, host-recorder, host-controller, host-emergency, host-audio, host-legacy
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    for bin in ["host", "host-agent", "host-capturer", "host-streamer",
                "host-recorder", "host-controller", "host-emergency", "host-audio", "host-legacy"] {
        assert!(manifest.contains(&format!("name = \"{bin}\"")), "missing bin {bin}");
    }
}

#[test]
fn placeholder_blocks_and_exits_on_signal() {
    // Spawn the host-capturer binary (env CARGO_BIN_EXE_host-capturer) → wait 200ms
    // → assert process alive (output contains "capturer placeholder ready") → SIGTERM → exit code 0
}
```

- [ ] **Step 2: run and verify failure**

Run: `cargo test -p mediaservo-host --test multiproc_skeleton`
Expected: FAIL (bins don't exist / no [[bin]] sections in Cargo.toml)

- [ ] **Step 3: implement the skeleton**

`Cargo.toml` additions:
```toml
[lib]
name = "mediaservo_host"
path = "src/lib.rs"

[[bin]]
name = "host"
path = "src/bin/host.rs"
# ... host-agent/host-capturer/host-streamer/host-recorder/host-controller/host-emergency/host-audio isomorphic

[[bin]]
name = "host-legacy"
path = "src/bin/host-legacy.rs"
```
`git mv src/main.rs src/bin/host-legacy.rs` (existing functionality untouched).
Implement lib.rs and each bin shell per Interfaces (run_placeholder waits via `tokio::signal::ctrl_c()` + `signal(SIGTERM)`).

- [ ] **Step 4: run and verify pass**

Run: `cargo test -p mediaservo-host --test multiproc_skeleton`
Expected: PASS (2 tests)
Run: `cargo build -p mediaservo-host && cargo clippy -p mediaservo-host -- -D warnings`
Expected: build succeeds + clippy zero warnings (existing host-legacy code stays clippy-clean if it already is)

- [ ] **Step 5: Commit**

```bash
git add crates/mediaservo-host/
git commit -m "refactor(host): lib + 9 bin skeleton (host CLI + 7 process placeholders + host-legacy kept)"
```

### Task A4: external script adaptation + interface contract record (A1 review I1/I2)
- **Files**: `scripts/e2e-test.sh:47` (`--bin mediaservo-host` → host-legacy), `scripts/mediaservo_cli.py:358-373` (run-host/stop-host binary paths + pkill), `scripts/install.sh:11` (BIN_NAME), `scripts/run-e2e-sfu.sh:24` (pgrep) — all 4 files updated to host-legacy or the new CLI
- **Interface record**: `run_placeholder` is an **async** signature (the brief's original "sync" was drift — the implementation defers to `pub async fn run_placeholder(role: &str)`, required by tokio signals); `init_logging(role)` is sync
- **Verification**: `./mediaservo.sh e2e` (9/9) + run-host/stop-host smoke + install.sh install smoke

### Task A2: host CLI init/status/start/stop + oxfile translator prototype

**Files:**
- Modify: `crates/mediaservo-host/src/bin/host.rs`
- Create: `crates/mediaservo-host/src/translate.rs` (lib module: host.toml → oxfile.toml text)
- Test: `crates/mediaservo-host/tests/translate.rs`

**Interfaces:**
- Consumes: A1's lib.rs (init_logging)
- Produces:
  - `translate.rs`: `pub fn to_oxfile(cfg: &str) -> Result<String, String>` — takes host.toml content, outputs oxfile.toml text (apps include the 8 host processes + a capturer per camera + a streamer per stream; the Phase A oxfile contains only placeholder processes and a parameterized instance skeleton)
  - `host init <dir>`: writes the `etc/host.toml` template + generates `etc/link/signing.pem` (Ed25519 keypair, 0600) + an empty `etc/link/` directory
  - `host start --dir <dir>`: reads etc/host.toml → translate → writes run/oxfile.toml → executes `oxmgr apply run/oxfile.toml` (clear error + install hint when oxmgr is not on PATH)
  - `host stop --dir <dir>`: executes `oxmgr stop --namespace host` + `oxmgr delete --namespace host` (idempotent)
  - `host status --dir <dir>`: executes `oxmgr list` filtered to the host namespace, printing a per-process status table
  - Process count: 7 types + parameterized instances (agent + capturer×N + streamer×N + recorder + controller + emergency + audio) — the host CLI itself is not a process

- [ ] **Step 0: confirm OxMgr CLI verbs and oxfile format (C11/C18)**

Per C11, read the OxMgr official docs (README confirmed `oxmgr apply`/`oxmgr list` and oxfile `version=1 + [[apps]]` ✓; **unconfirmed**: `oxmgr stop --namespace host` / `oxmgr delete --namespace host` syntax — confirm against official docs/CLI.md or the AI Skill Reference docs/SKILL.md, then implement the smoke test with the confirmed verbs)

- [ ] **Step 1: write the failing test**

`tests/translate.rs`:
```rust
#[test]
fn to_oxfile_emits_all_placeholder_apps() {
    let cfg = "[host]\ndevice_id = \"car-01\"\n[[cameras]]\nid = \"cam0\"\nsource = \"stub\"\nfps = 30\n[[streams]]\nid = \"cam0-stream\"\ncamera = \"cam0\"\ncodec = \"h264\"\n[record]\nenabled = false\n[control]\nenabled = false\n";
    let ox = translate::to_oxfile(cfg).unwrap();
    for app in ["host-agent", "host-capturer", "host-streamer", "host-recorder",
                "host-controller", "host-emergency", "host-audio"] {
        assert!(ox.contains(&format!("name = \"{app}\"")), "missing {app}");
    }
    assert!(ox.contains("host-capturer --camera cam0")); // parameterized instance
    assert!(ox.contains("host-streamer --stream cam0-stream"));
}
```

- [ ] **Step 2: run and verify failure**

Run: `cargo test -p mediaservo-host --test translate`
Expected: FAIL (translate module doesn't exist)

- [ ] **Step 3: implement translator + CLI subcommands**

translate.rs: hand-written oxfile.toml text generation (aligned to the OxMgr oxfile format: `version = 1` + `[[apps]]` name/command/restart_policy="always").
host.rs: match args "init"/"start"/"stop"/"status"/"version" (unknown subcommands print usage and exit 2).

- [ ] **Step 4: run and verify**

Run: `cargo test -p mediaservo-host --test translate`
Expected: PASS
Run (manual smoke, oxmgr must be installed):
```bash
cargo build -p mediaservo-host
rm -rf /tmp/host-smoke && ./target/debug/host init /tmp/host-smoke
./target/debug/host start --dir /tmp/host-smoke   # Expected: oxmgr apply succeeds
./target/debug/host status --dir /tmp/host-smoke  # Expected: 7 processes (template config: 5 fixed + capturer + streamer — Momus/A2 review arithmetic correction)
./target/debug/host stop --dir /tmp/host-smoke    # Expected: all stopped
```
Expected: full flow exits 0; oxmgr list shows processes starting/stopping

- [ ] **Step 5: Commit**

```bash
git add crates/mediaservo-host/
git commit -m "feat(host): host CLI init/start/stop/status + host.toml→oxfile translator (A2)"
```

### Task A3: OxMgr detection and graceful degradation

**Files:**
- Modify: `crates/mediaservo-host/src/bin/host.rs`

**Interfaces:**
- Consumes: A2 CLI
- Produces: `host doctor` (Phase A minimal version: checks oxmgr presence/version, host.toml parseability, oxfile generatability)

- [ ] **Step 1: write the failing test**

`tests/doctor.rs`: in an environment without oxmgr, `host doctor` prints "oxmgr: not found" and exits non-zero (the test skips `which oxmgr` internally — CI may lack oxmgr)

- [ ] **Step 2-4: implement + verify** (doctor subcommand: which oxmgr → host.toml parse → translate → print checklist; exit code = number of failed items)

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(host): host doctor environment diagnostics (oxmgr/config/translate three checks)"
```

### Phase A Acceptance

- [ ] `cargo test -p mediaservo-host` fully green (including existing host-legacy tests, no regression)
- [ ] Smoke: host init → start → status (8 processes) → stop, full flow
- [ ] Existing host 9/9 E2E no regression (host-legacy bin name adapted: binary paths in e2e scripts updated to host-legacy)
- [ ] `host doctor` prints clear guidance on machines without oxmgr

---

## Phase B: link Enhancements (phase-level task list)

**Goal**: Three link SDK enhancements, independently testable, no dependency on the host refactor.

### Task B1: SignalClient reconnect (exponential backoff + jitter)
- **Files**: `crates/mediaservo-link/src/signal.rs` (connect gains a reconnect parameter `RetryConfig { max_retries, base_delay, max_delay }`, reusing the coding-style retry_with_backoff pattern), `tests/` mock WS server disconnect tests
- **Interface**: `SignalClient::connect_with_retry(cfg: RetryConfig) -> Result<SignalSession, LinkError>`; `SignalSession` gains `on_disconnect(cb)` (disconnect notification for the upper layer to trigger reconnect); **re-authentication after reconnect** (device credentials/session token → session resume — spec D-H14)
- **Tests**: mock server rejects then accepts → reconnect succeeds; consecutive rejects → error after the backoff cap
- **Verification**: `cargo test -p mediaservo-link` + clippy

### Task B2: single-file self-describing token
- **Files**: `crates/mediaservo-link/src/token.rs` (`TokenFile` format: verifying key + claims + signature merged; `CapabilityToken::to_file()/from_file()`), `tests/token_file.rs`
- **Interface**: `TokenFile::encode(&CapabilityToken, &Ed25519VerifyingKey) -> Vec<u8>`; `TokenFile::decode(&[u8]) -> (Ed25519VerifyingKey, CapabilityToken)` (decode verifies the signature internally)
- **Tests**: roundtrip; tampering any byte fails verification; wrong length errors
- **Verification**: `cargo test -p mediaservo-link`

### Task B3: host init exports ros_bridge.yaml
- **Files**: `crates/mediaservo-link/src/bridge.rs` (new: generates ros_bridge.yaml from the host.toml camera/stream list) + host CLI `host init` wiring
- **Interface**: `pub fn ros_bridge(cameras: &[String], streams: &[String], token_path: &str) -> String` (yaml text: topics + token_path)
- **Tests**: output contains all camera/vision topics + token_path
- **Verification**: unit test + `host init` smoke

---

## Phase C: Media Split (phase-level task list)

**Goal**: Migrate the existing single-process media functionality into process pairs (capturer → FrameBus → streamer/recorder); deck closed_loop mode goes multiprocess.

### Task C1: capturer process (capture → FrameBus publish)
- **Files**: `crates/mediaservo-host/src/bin/host-capturer.rs` (real implementation replaces the placeholder): `--camera cam0` argument → read the host.toml camera config → reuse mediaservo-media VideoFrameGenerator (stub color bars first, USB/MIPI sources later) → link FrameBus publish (token loaded from `--token <path>`)
- **Interface**: depends on mediaservo-link (allowed by C12/C21 — link is the SDK base); publishes topic `camera/cam0` (naming convention fixed)
- **Tests**: single-process test (publish N frames → FrameBus subscriber receives + meta correct); CLI argument parsing test

### Task C2: streamer process (subscribe → encode → push)
- **Files**: `crates/mediaservo-host/src/bin/host-streamer.rs`: `--stream cam0-stream` → subscribes `camera/cam0` → encode (reuse existing host encoding logic + webrtc TrackSender) → push (P2P mode first: signaling still goes over the original direct WS — no bus before Phase D)
- **Interface**: migrate the push sections of the existing host sfu_media.rs/webrtc_transport.rs; P2P signaling reuses the field PushSession pattern (C21 external server interaction)
- **Tests**: e2e (capturer + streamer dual processes → external server receives, bytes_sent>0 — aligned with the field push_e2e 6/6 pattern)

### Task C3: recorder process (subscribe → disk)
- **Files**: `crates/mediaservo-host/src/bin/host-recorder.rs`: subscribes camera/* → deck Recorder (reuse — deck has already proven the FrameBus→Recorder closed loop)
- **Tests**: closed-loop e2e (capturer → FrameBus → recorder writes to disk → ffprobe verifies h264/moov)

### Task C4: e2e adaptation
- **Files**: `scripts/` (host 9/9 E2E scripts: `host start` starts multiprocess pairs instead of the single-process host)
- **Verification**: full regression (e2e_sfu 4/4 + codec_prefs 6/6 + new multiprocess closed loop)

---

### Task C5: crash-restart fault-injection e2e (Momus MEDIUM-3 — validates the architecture's core value)
- **Files**: `crates/mediaservo-host/tests/crash_recovery.rs`
- **Tests**: kill the capturer → OxMgr restarts it (restart_policy=always) → **republish on the same topic succeeds** (max_publishers(1) + iceoryx2 residual services must not block — if this fails, a production-grade SHM residue cleanup mechanism is needed, not manual rm, C25) → subscribers (streamer/recorder) resume frame receipt + monitor shows no persistent alerts
- **Verification**: `cargo test -p mediaservo-host --test crash_recovery` + manual smoke

## Phase D: Signaling Bus (phase-level task list)

**Goal**: host-agent WS gateway — each process WS-connects to a local 127.0.0.1:PORT; the agent aggregates into a single WS toward the Server (D-H6).

### Task D1: agent gateway core (Momus HIGH-1 correction: protocol semantics must land first)
- **Files**: `crates/mediaservo-host/src/bin/host-agent.rs` (real implementation): local WS accept (tokio-tungstenite) → single remote WS (SignalClient reconnect — B1 deliverable) → bidirectional forwarding + response routing
- **Protocol semantics (empirical basis — Momus verified against signaling.rs)**:
  - (a) **RoomJoin interception**: each local process's RoomJoin is intercepted by the agent (no longer sent per-process — signaling.rs evidence: RoomJoin is only handled during connection setup; a RoomJoin received inside the relay loop is silently dropped; the relay whitelist covers only Sdp/RTCIceCandidate/Frame/EncoderStatus); the agent joins once as the whole-vehicle identity
  - (b) **Response routing**: SFU messages map back to local connections by msg_peer_id/transport_id (signaling.rs implements this semantics, PIT-65); the P2P relay's Sdp/RTCIceCandidate have no transport marker → track by negotiation ownership (the agent maintains a transport↔local-connection mapping table) or explicit serialization
  - (c) **Infeasible options rejected**: "add a from prefix remotely" (changes the wire format, breaking zero server changes) and "reuse in order" (concurrent negotiation would scramble) are both rejected
- **Interface**: local protocol = `{src: "host-streamer-cam0", msg: SignalingMessage}`; remote = pure SignalingMessage (zero wire change); the agent maintains a `transport_id/peer_id → local connection` mapping
- **Tests**: mock Server verifies: ① multiple local clients forward correctly ② RoomJoin interception (child-process RoomJoins don't go upstream; the agent joins once) ③ **concurrent negotiation** (two simultaneous create offers → response routing doesn't cross) ④ forwarding resumes after disconnect-reconnect

### Task D2: per-process signaling address becomes configurable
- **Files**: host.toml gains `[signaling] local_gateway = "127.0.0.1:PORT"` (default on) + each process's WS target changes from the Server address to the local gateway; the translator passes the parameter
- **Tests**: D1 mock full-chain e2e (capturer/streamer processes connect to the mock Server through the gateway)

### Task D3: one-vehicle-one-session e2e
- **Verification**: real Server (Docker) + multiprocess host → exactly 1 peer session server-side + multi-transport produce all pass; existing e2e_sfu 4/4 regression

---

## Phase E: host-agent Monitoring (phase-level task list)

**Goal**: Topology/data-flow/signaling-state monitoring + desired-state comparison + reporting (D-H4).

### Task E1: topology monitoring
- **Files**: `crates/mediaservo-host/src/monitor/topology.rs`: desired state (declared in host.toml: N capturers + N streamers + recorder + controller...) vs actual state (oxmgr list process liveness + publisher enumeration) → diff list
- **Publisher-enumeration data source decision (Momus MEDIUM-2)**: link cross-process discovery is not implemented (status.md "cross-process discovery deferred to Phase 2") — E1 picks one: ① add a discovery implementation task in Phase E (iceoryx2 ServiceRegistry enumeration — fully delivers D-H4) ② MVP degradation: only oxmgr list process-level topology + FrameBus subscription stats (data-plane topology deferred). **Default is ①** (D-H4's declarative-desired + discovery-based-actual is the core of monitoring), task folded into E1
- **Tests**: 3 capturers desired, 2 actual → report missing; grace period suppresses the startup window

### Task E2: data-flow monitoring
- **Files**: `crates/mediaservo-host/src/monitor/flow.rs`: FrameBus subscription stats (per-topic frame rate/byte rate/stall detection — ts_mono timestamp deltas) + streamer push state (bytes_sent/frames_encoded — webrtc stats)
- **Tests**: simulated publisher speed changes → frame-rate curve + stall alerts

### Task E3: signaling-state monitoring + reporting
- **Files**: `crates/mediaservo-host/src/monitor/signal.rs`: per-process WS connection state (held by the gateway) + remote Server connection state → panel data + report to Server (signaling extension message StatusReport)
- **Tests**: disconnect → state updates + reported message content asserted

---

### Task E4: cloud config loop (spec §5 — Momus HIGH-3 addition)
- **Files**: `crates/mediaservo-host/src/bin/host-agent.rs` (extension: ConfigPush receive) + `crates/mediaservo-host/src/bin/host.rs` (adds `host apply`/`host restart` subcommands)
- **Interface**: signaling extension message `ConfigPush { config: String, version: u64 }` (PSK/JWT auth + audit log — C15/C16 discipline) → validate → write host.toml (backup the old version) → call `host apply` → translator → oxfile.toml → OxMgr file-watch hot apply (add/remove paths = incremental apply)
- **Tests**: mock Server pushes ConfigPush → host.toml updated + oxfile regenerated + processes restart to apply; invalid config rejected + audit log asserted

## Phase F: Control/E-Stop (phase-level task list)

**Goal**: host-controller (DC multi-label) + host-emergency (standalone PC + local fallback) — P2P mode first (D-H3/H8).

### Task F1: controller process
- **Files**: `crates/mediaservo-host/src/bin/host-controller.rs`: pure DC PC (create_data_channel × 3 labels: chassis/gimbal/light) → cockpit client direct connect (P2P) → route DC messages to the actuator interface (Phase A uses a stub actuator first: logging + receipt)
- **Tests**: dual-process e2e (controller ↔ mock cockpit PC: DC message roundtrip + label distinction + reliable ordering assertion; the actuator stub is implemented in this Phase F stage — "Phase A first stub" was a typo)

### Task F2: emergency process
- **Files**: `crates/mediaservo-host/src/bin/host-emergency.rs`: standalone PC + DC (label emergency) + local fallback interface (trait `EmergencyActuator` — stub impl: logging; CAN/GPIO implementations after Phase I)
- **Tests**: e-stop message → actuator fires + receipt; a controller crash doesn't affect emergency (verify by killing the controller process)

---

### Task F3: streamer vision DC (D-H8 chain — Momus HIGH-2 addition)
- **Files**: `crates/mediaservo-host/src/bin/host-streamer.rs` (extension): standalone transport B (pure DC, no track — mediasoup's official send/recv separation) + label "vision"; subscribes the vision topic (e.g. `vision/cam0`, source = ROS vision node) → forwards DC JSON messages (object arrays: class/confidence/bbox/text/color + frame association ts_mono/seq) to the cockpit HMI
- **Tests**: ① external publisher (ROS simulation process, link SDK attach to the vision topic) → streamer subscribes → DC receives JSON (also closes the D-H7 external-node verification gap) ② assert the vision DC and video track use separate transports (two m-lines in SDP) ③ ts/seq frame-association fields exist

## Phase G: Security (phase-level task list)

**Goal**: Device credentials + session token (server side) + token issuance flow (D-H10/H11 landing).

### Task G1: link token issuance flow
- **Files**: host CLI `host token issue --role <R> --topic <T> --out <path>` (signing key read from etc/link/signing.pem) + ros_bridge.yaml wiring
- **Tests**: issue → ROS side from_file attach succeeds; wrong role/topic rejected

### Task G2: Server device credentials
- **Files**: `crates/mediaservo-server/src/` (auth extension): device table (config/DB: device_id + secret hash) + Join device authentication → short-lived session token (JWT already exists) → connection-level identity
- **Tests**: auth success/failure/reconnect token refresh; multi-device isolation (vehicle A's token cannot act as vehicle B)

### Task G3: cockpit tiered authorization
- **Files**: Server session/permission layer: viewer/operator/admin/dispatcher roles + authorization matrix (consume/control/emergency/config/audio checked per role — matrix in spec D-H11)
- **Tests**: role×capability matrix full-combination test (table-driven)

---

### Task G4: host device identity provisioning (Momus MEDIUM-4 — prerequisite for G2 tests)
- **Files**: `crates/mediaservo-host/src/bin/host.rs` (generated by `host init`) + `crates/mediaservo-host/src/bin/host-agent.rs` (carried on Join)
- **Interface**: host init/install generates `identity.json` (device_id + device_secret, 0600 — D-H13 layout); the host-agent Join auth flow carries device credentials → session token
- **Tests**: init generates → agent Join succeeds; wrong credentials → clear auth failure

## Phase H: Server Extensions (phase-level task list)

**Goal**: Audio conference rooms + data domain (SFU-mode DC) + dispatcher pull permissions.

### Task H1: SFU data domain
- **Files**: `crates/mediaservo-server/src/sfu.rs`: DataProducer/DataConsumer (native mediasoup — official API usage, C18) + signaling message extensions (produce_data/consume_data)
- **Tests**: SFU DC end-to-end (controller/vision DCs relayed through the Server — complements the P2P validation in Phase F)

### Task H2: audio conference rooms
- **Files**: Server audio room management (join/leave/member list/permissions — dispatcher any vehicle / cockpit only authorized vehicles) + a real host-audio process implementation (mic/speaker — ALSA/MMAPI + opus codec via the FFmpeg backend)
- **Tests**: 3-party audio conference e2e (vehicle + cockpit + dispatcher browser — play silent frames to verify routing)

### Task H3: admin/dispatcher frontend
- **Files**: `www/apps/admin/` (audio panel + multi-vehicle monitoring view + dispatcher login role)
- **Verification**: Playwright e2e (browser-testing skill)

---

## Phase I: Deployment Packages (phase-level task list)

**Goal**: host/sdk dual packages + install host + provisioning scripts (D-H13).

### Task I1: install host command
- **Files**: `scripts/mediaservo_cli.py` (`install host --prefix`: produces an /opt/mediaservo-host layout — 8 bins + oxmgr copy/detect + etc templates + run initialization) + host init wiring
- **Tests**: install into a clean directory → host doctor all green → start/stop smoke

### Task I2: dual-package release script
- **Windows verification (spec §7)**: minimal closed loop on a CARLA machine (x86-Windows) with capturer capture + push — packaging verification folded into this task
- **Files**: `scripts/` (package mediaservo-host-<ver>.tar.gz + mediaservo-sdk-<ver>.tar.gz — the SDK reuses install bindings artifacts)
- **Verification**: unpack both packages on a clean machine, both independently usable + protocol-version declaration file

---

## Load-Bearing Invariants (F3 review — prevents silent breakage by future refactors)

- **transport A (video push) sends no Sdp wire messages**: field PushSession negotiates as answerer (build_remote_sdp + create_answer, locally constructed offer); the whole chain never sends SignalingMessage::Sdp to the gateway → never touches the p2p_owner single slot. **If PushSession is ever refactored to offerer negotiation, the P2P negotiations of vision/controller/emergency would be silently hijacked by transport A's ownership** — D1's p2p_owner mechanism must be re-reviewed before any such change.
- **At most 4 P2P sessions per vehicle share the gateway's single slot** (controller/emergency/vision 3 offerers + 1 answerer): sequential negotiation (OxMgr starts processes in order) holds; concurrent negotiation is a known gap (evaluate in Phase G).

## Risk Register

| Risk | Mitigation |
|------|------------|
| Media split breaks existing e2e (9/9) | host-legacy kept until Phase C completes; full regression each phase |
| OxMgr niche / API changes | oxfile.toml generation centralized in translate.rs; oxmgr CLI calls behind a thin wrapper (A2's _run_or_exit pattern) |
| iceoryx2 multiprocess service parameter conflicts | Config unified and fixed in the link API (D-H14); pre-clean before cross-process tests (C25) |
| Single-WS aggregation incompatible with the Server protocol | Phase D validates the protocol wrapping against a mock Server first; real-Server regression as backstop |
| Audio/control depend on the Server data domain | Phase F validates P2P first; H1 independently testable |
| C22 (host bans Docker) | All host tests run native on the host machine; Server only in Docker |
| host-agent single point (Momus MEDIUM-5) | Signaling interrupted during the crash window (remote e-stop channel affected) — mitigated by OxMgr second-level restart + local fallback; F2 acceptance includes "controller crash doesn't affect emergency" |
| P2P control NAT failure (Momus MEDIUM-5) | Without TURN, P2P DC may be unavailable — F acceptance adds an ICE-failure degradation note; the SFU data domain (H1) is the final backstop, risk window = F→H phases |
