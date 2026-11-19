# Operations — Ops Design

> Status: Phase 3 design | Consolidates: D99 + D-OPS-01~10 + D111 | Created per doc-audit CR4

## Logging Strategy

| Component | Format | Output | Level |
|-----------|--------|--------|-------|
| Host | JSON (tracing-subscriber) | stdout + journald | info |
| Server | JSON (tracing-subscriber) | stdout + journald | info |
| Remote | JSON (tracing-subscriber) | stdout | info |

- tracing span: one span per request/WS connection, trace_id threading through
- Production logs: 30-day rotation, compressed archive

- Phase 1 bare-metal deployment: systemd-journald + the tracing-journald crate. Log format JSON. Phase 2 migrates to opentelemetry-otlp → collector.
## Metrics (Prometheus + /metrics endpoint)

| Metric | Type | Description |
|--------|------|-------------|
| `omsp_rooms_active` | gauge | Active room count |
| `omsp_peers_connected` | gauge | Connected peer count |
| `omsp_fps_current` | gauge | Current frame rate |
| `omsp_latency_ms` | histogram | End-to-end latency |
| `omsp_component_status` | gauge | Component status (0=stopped,1=running,2=degraded) |
| `omsp_cpu_pct` | gauge | CPU usage |
| `omsp_mem_mb` | gauge | Memory usage |

## Alerts (Alertmanager)

| Alert | Condition | Severity |
|-------|-----------|----------|
| ComponentCrashed | omsp_component_status=0 | critical |
| HighLatency | omsp_latency_ms > 200ms (5min) | warning |
| NoPeers | omsp_peers_connected=0 (1min) | warning |
| HighCPU | omsp_cpu_pct > 90 (5min) | warning |

## Alert Notification

Phase 1 alert notification channels:

| Channel | Method | Priority |
|---------|--------|----------|
| Slack webhook | Direct Incoming Webhook call | Primary |
| Email | smtplib (plain text/HTML) | Backup |

Phase 2: PagerDuty integration (on-call scheduling).

Alert routing:
- ComponentCrashed / HighLatency / NoPeers → on-call
- HighCPU / HighMemory → infra team
## TLS

- Phase 1: systemd socket activation + external TLS termination (nginx/Caddy)
- Phase 2: built-in rustls + Let's Encrypt ACME
- Certificate rotation: 30-day auto-renewal

## Backup

- SQLite: `sqlite3 .backup` daily + WAL checkpoint
- session_state.json: file rotation, keeping the most recent 100
- Backup targets: local + remote (Phase 2)

## Capacity Planning

| Deployment size | Rooms | Peers | CPU | RAM | Disk |
|-----------------|-------|-------|-----|-----|------|
| Small | 10 | 20 | 2 cores | 2GB | 10GB |
| Medium | 50 | 100 | 4 cores | 4GB | 50GB |
| Large | 200 | 500 | 8 cores | 8GB | 200GB |

## Resource Budget

Phase 1 resource estimates (to be refined with profiling data):

| Component | CPU | RAM | Disk | Network uplink |
|-----------|-----|-----|------|----------------|
| Host (720p H.264) | ~2 cores | ~500MB | ~50MB | 2-5 Mbps |
| Host (1080p H.265) | ~3 cores | ~800MB | ~50MB | 1-3 Mbps |
| Remote decode | ~1 core | ~300MB | ~20MB | — |
| Server (signaling) | ~0.5 core | ~200MB | ~100MB | — |
| GPU encoder (NVENC) | — | ~200MB VRAM | — | — |

> Note: the above are Phase 1 estimates, to be refined with profiling data.

## Network Planning

Port allocation:

| Service | Port | Notes |
|---------|------|-------|
| Host signaling WS | configurable, default 8080 | WebSocket signaling |
| Server relay | configurable | Media relay port |
| TURN/STUN | 3478-3480 UDP+TCP | coturn defaults |
| WebRTC media | 49152-65535 UDP | ephemeral port range |

Bandwidth reference:
- 720p@30 H.264: ~2-4 Mbps
- 1080p@30 H.265: ~1.5-3 Mbps
- Signaling channel: <50 Kbps
- RTCDataChannel control: <10 Kbps

## System Resource Limits

systemd unit template example:

```ini
[Service]
MemoryMax=1G
CPUQuota=200%
TasksMax=512
```

### Docker Resource Limits (compose)

> Related: D208 build optimization, container-build-rework B3.4/R13/R18 | Created per doc-audit M6

**B3.4 tree-packaged prod image** (`docker-compose.yml`): the single container
runs the whole deploy tree (msrtc-server + caddy + oxmgr PID1) — caddy is no
longer a separate proxy service (R12 retired). Resource policy per R13=C:

| Item | value | rationale |
|------|-------|-----------|
| `mem_limit` | `${MEM_LIMIT:-512m}` (R18 measured) | host protection; Small-tier peak ×1.3 round-up 512MB |
| CPU | **unset** | RTP bursts free (R13); no cpus/NanoCpus quota |

`mem_limit` is the container-level field (works under plain `docker compose up`,
unlike `deploy.resources` which only applies in Swarm). The R8 smoke asserts
`HostConfig.Memory > 0` AND `HostConfig.NanoCpus == 0` (R9).

**R18 load-test method (user-ruled A) — EXECUTED 2026-11-18**: Small tier
(10 rooms/20 peers, 720p H.264 ~2Mbps), steady 10min + burst 2min, cgroup
`memory.peak` / `docker stats` 5s sampling; `mem_limit` = measured peak ×1.3
round-up to 512MB. Pass = no OOM + /health green. Harness:
`scripts/r18-load-harness/` (mediasoup-client + playwright headless chromium,
fake 720p y4m video; each room = 1 host producer + N remote consumers; the
server runs host-networking for reliable WebRTC ICE). The pre-existing
`e2e-sfu-consume.cjs` / `e2e-bitrate-probe.cjs` are single-stream functional
E2E, not load generators — the harness fills that gap.

**R18 measured result (`msrtc-server:0.1.3`, 8-core host)**:

| Phase | Peers | Out (aggregate) | In (aggregate) | Notes |
|-------|-------|-----------------|----------------|-------|
| idle | 0 | — | — | steady 24.67 MiB, boot cgroup peak 28.35 MiB |
| steady 10min | 10 prod + 10 cons | 16.42 Mbps | 16.27 Mbps | 720p, all transports connected |
| burst 2min | 10 prod + 20 cons | 14.75 Mbps | 28.69 Mbps | forwarding doubled (30 peers) |

- `docker stats` peak **52.74 MiB**; cgroup `memory.peak` **56.81 MiB** (authoritative).
- `mem_limit` = ⌈56.81 × 1.3 / 512⌉ × 512 = **512 MiB** (the 512MB round-up floor;
  ~9× headroom over the measured peak). No OOM; /health green throughout.
- **Achieved bitrate caveat**: per-peer ~1.5–1.6 Mbps avg (target 2 Mbps) — the
  8-core host CPU-contends 10 software H.264 encoders + 10–20 decoders, so
  libwebrtc adapts bitrate down. The memory footprint is dominated by the
  per-peer transport/producer/consumer object graph (bitrate-independent), so
  the 512 MiB ceiling holds at full 2 Mbps; re-measure on the production host
  for larger tiers (medium 50 rooms/100 peers, large) per §Capacity Planning.

**dev environment intentionally unlimited**: the ssh-resident dev container
(`dev up`, builder image) sets no limits — it bears `cargo build` (mediasoup-sys
meson/ninja C++ compilation, PIT-11/33); limits would OOM-kill the build. (The
legacy build-based `docker-compose.dev.yml` retired with the embedded line.)

**How to adjust**: set `MEM_LIMIT` in `.env` (default 512m = Small-tier measured),
`docker compose --env-file .env up -d` to recreate, `docker compose config` to
validate. Scale per §Capacity Planning tiers; re-run `scripts/r18-load-harness/`
with larger ROOMS/PER_ROOM to measure medium/large peaks.

## Log Aggregation

### Format

```json
{"timestamp":"2026-07-29T12:00:00Z","level":"info","msg":"transport connected","peer_id":"test-consumer","room_id":"room-1","transport_id":"t-abc"}
```
- Structured JSON, fields: timestamp/level/msg + business fields
- Uses the `tracing-subscriber` JSON formatter (integrated in Phase 2)

### Export

| Phase | Approach | Notes |
|-------|----------|-------|
| Phase 1 | File + stdout | `tracing-subscriber` JSON to file, Docker stdout to `docker logs` |
| Phase 2 | OpenTelemetry OTLP | the `opentelemetry-otlp` crate, exporting to an OTel Collector |
| Phase 3 | Loki/Promtail | Lightweight; integrates with the existing Prometheus stack |

### Retention Policy

| Log type | Retention | Notes |
|----------|-----------|-------|
| Application logs | 30 days | SFU operations, signaling, errors |
| Access logs | 90 days | API request audit |
| Alert logs | 180 days | Incident tracing |
| System logs | 7 days | systemd/journald auto-rotation |

### Key Log Points

| Event | Level | Fields |
|-------|-------|--------|
| transport.connect() | info | peer_id, room_id, transport_id |
| transport.connect() failure | error | error_code, reason |
| producer/consumer creation | info | kind, producer_id/consumer_id |
| Worker crash | critical | worker_id, uptime |
| Auth failure | warn | client_addr, reason |

## Phase Dependencies

- Phase 3 Component framework: /metrics endpoint, tracing span integration
- Phase 4 Admin Dashboard: ops panel (Dashboard, Rooms)
- Phase 2 mediasoup SFU: media quality metrics (loss rate, jitter, RTT)
