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

> Related: D208 build optimization | Created per doc-audit M6

The production `docker-compose.yml` declares resource limits for the server service (`docker-compose.yml:10-17`):

| Item | limits | reservations |
|------|--------|--------------|
| CPU | 2 cores | 1 core |
| Memory | 4G | 2G |

**Rationale for the numbers** (aligned with this file's §Capacity Planning "small" tier):
- **limits 2c/4G**: the container runs the Rust server + the mediasoup C++ Worker (RTP forwarding, Router management) simultaneously; the Caddy reverse proxy is a separate service outside this quota. The small tier (10 rooms/20 peers) plans 2 cores/2GB; the 4G ceiling leaves headroom for the mediasoup Worker's bursty forwarding (large rooms + multiple transports).
- **reservations 1c/2G**: the signaling baseline is ~0.5 core/200MB; 1c/2G guarantees the worker's startup and regular forwarding aren't starved by scheduling pressure.

**dev environment intentionally unlimited**: `docker-compose.dev.yml` sets no `deploy.resources`. The dev container bears heavy loads like `cargo build` (mediasoup-sys meson/ninja C++ compilation, PIT-11/33); limits would OOM-kill the build; dev-machine resources are governed by Docker Desktop / the host scheduler.

**⚠️ Scope of effect**: `deploy.resources` only takes effect under Docker Swarm (`docker stack deploy`); plain `docker compose up` ignores it. If the current single-machine deployment needs enforced limits, use container-level fields instead:

```yaml
services:
  server:
    mem_limit: 4g
    cpus: 2
```

**How to adjust**: scale per the §Capacity Planning tiers — medium (50 rooms/100 peers) suggests limits 4c/8G, reservations 2c/4G; large 8c/16G. After changes run `docker compose up -d` to restart and `docker compose config` to validate.

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
