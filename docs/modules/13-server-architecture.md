# 13. Server Architecture

> Phase 0 — architecture definition | 2026-07-17
> Related decisions: D86-D91, D98-D101, D118 + main doc reference: [architecture doc](../architecture.md) §2, §10

## 13.1 Overview

mediaservo-server is the Phase 1 MVP's central relay node, handling three responsibilities: signaling relay, media forwarding, and operational monitoring. The Server never decodes or re-encodes video frames — pure RTP packet forwarding with latency overhead capped at 30ms.

```
Host (macOS/Linux/Windows)
  │ WebRTC push (RTP/SRTP)
  │ Signaling (WS /ws, JSON SDP/ICE)
  ▼
mediaservo-server
  ┌──────────────────────────────────────┐
  │  Signaling Relay (axum WS)           │
  │  /ws endpoint, JSON SDP/ICE relay    │
  │  room management, PSK auth           │
  ├──────────────────────────────────────┤
  │  Relay Engine                        │
  │  Host PC ↔ Remote PC track bridging  │
  │  pure forwarding, no transcoding —   │
  │  no decode, no re-encode             │
  ├──────────────────────────────────────┤
  │  Monitoring                          │
  │  /health /ready /metrics             │
  │  session stats, rate limiting        │
  ├──────────────────────────────────────┤
  │  Web UI (React + Ant Design)         │
  │  Ops panel: device list, session     │
  │  monitoring                          │
  ├──────────────────────────────────────┤
  │  Auth (JWT + role RBAC)              │
  │  SQLite users table, PSK pre-shared  │
  │  key                                 │
  └──────────────────────────────────────┘
  │ WebRTC forward (RTP/SRTP)
  │ Signaling (WS /ws, JSON SDP/ICE)
  ▼
Remote (macOS/Linux/Windows)
  decode + render + RTCDataChannel control
```

## 13.2 Core Components

| Component | Responsibility | Decisions |
|-----------|----------------|-----------|
| Signaling Relay | WebSocket /ws; relays SDP/ICE between Host and Remote, never modifying content | D52, D118 |
| Relay Engine | WebRTC track bridging: Host push ingest → Remote pull forward | D86, D118 |
| Room Manager | Single-room create/join/leave, room state management (DashMap) | D86 design |
| Auth | PSK auth + JWT issuance + role RBAC (admin/operator/auditor) | D88, D100 |
| Monitoring | /health + /ready + /metrics + session stats | D86, D99 |
| Web UI | React 19 + Ant Design 5, ops management panel | D87 |

**Relay Engine core principle** (Phase 1): the Server never decodes or re-encodes video frames. The relay engine does track bridging at the RTP level, forwarding RTP packets directly between two RTCPeerConnections. Latency overhead ≤30ms.

### 13.2.1 mediasoup SFU Integration (Phase 2)

Phase 2 introduces the mediasoup SFU to replace Phase 1's same-process RTCPeerConnection bridge, improving media forwarding efficiency for multi-participant scenarios.

**Architecture**:

```
mediaservo-server (Rust)
  ├── Signaling Relay (axum + SignalHandler)
  │     SDP adapter → mediasoup parameter conversion
  │
  ├── Mediasoup Manager (mediasoup-rust FFI)
  │     ├── Worker management (1 worker per server process)
  │     │     └── crash recovery: worker restarted within 5s of SIGKILL
  │     ├── Router (1 per room)
  │     │     └── codec negotiation: H.264/VP9, simulcast branches
  │     ├── WebRtcTransport (per participant)
  │     │     └── ICE/DTLS management, inherits the Router's certificate
  │     ├── PlainTransport (per non-WebRTC stream)
  │     │     └── RTMP/SRT push ingest
  │     ├── Producer (created by Host)
  │     │     └── RTP tracks: video (H.264/VP9) + audio (Opus)
  │     └── Consumer (created by Remote)
  │           └── Subscribes to a specified Producer in the Router
  │
  └── Monitoring (Prometheus + Grafana)
```

**Key entity flow**:

```
Host push:
  Host → Signaling → SDP adapter → mediasoup::Transport::createProducer()
  → Router registers Producer{track_id, kind, rtp_parameters}

Remote pull:
  Remote → Signaling → SDP adapter → mediasoup::Transport::createConsumer(producer_id)
  → Router::pipeTransport() forwards RTP → Consumer outputs to Remote
```

**Crash recovery**: the mediasoup Worker is supervised by the Manager and auto-restarts within 5s of a crash. On restart:
1. Rebuild the Worker → rebuild the Router → notify all participants to renegotiate SDP
2. Participants re-createOffer/answer → rebuild Producer/Consumer
3. The signaling layer stays transparent — participants only perceive a brief ICE restart

## 13.3 Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| HTTP/WS framework | axum 0.7 | WS in-process; JWT middleware shared |
| Database | sqlx + SQLite (compile-time SQL validation) | Lightweight, users table only, no separate DB service |
| Config | serde_yaml + env-var override | Sensitive values (JWT_SECRET) injected via env |
| Logging | tracing + tracing-subscriber (JSON stdout) | Collected via Docker logs |
| Metrics | prometheus-client (GET /metrics) | Prometheus format, Grafana panels |
| SFU metrics | prometheus-client + mediasoup observer | SFU Worker/Router/Transport/Producer/Consumer metrics |
| Middleware | tower-http (CORS, rate-limit, trace) | Production-grade HTTP middleware |
| Auth | jsonwebtoken (JWT) + argon2 (password hashing) | Self-contained tokens, no external auth service |
| Web UI | React 19 + Ant Design 5 + recharts | Shares the component ecosystem with mediaservo-client |

## 13.4 Rooms & Relay

The Server uses a simple room model. A room contains one Host (pusher) and one or more Remotes (pullers).

```
Room {
  id: String (UUID v4),
  host: Option<RTCPeerConnection>,
  remotes: Vec<RTCPeerConnection>,
  created_at: Instant,
  status: Waiting | Relaying | Closed,
}
```

Host joins → Waiting. A Remote joins → Relaying. The Remote leaves → Waiting. All participants leave → Closed.

## 13.5 Authentication

Two layers of auth:

1. **PSK (Pre-Shared Key)**: validated at WebSocket upgrade; all clients (Host/Remote) share one PSK. Stored in server.conf, configured at deployment.

2. **JWT (optional in Phase 1)**: SQLite users table stores users; argon2-hashed passwords. The JWT carries a role field (admin/operator/auditor); axum middleware extracts it into Extensions\<CurrentUser\>.

## 13.6 Observability

| Endpoint | Purpose | Notes |
|----------|---------|-------|
| GET /health | Liveness probe | Lightweight, returns 200 OK only |
| GET /ready | Readiness probe | Checks signaling and relay state |
| GET /metrics | Prometheus endpoint | HTTP request counts/latency, WS connection count, forwarding bandwidth, session duration |

All logs output JSON to stdout/stderr, collected by the Docker logs driver. tower-http TraceLayer injects a traceId into every request.

## 13.7 State Machine

```
INIT → WAITING (awaiting Host+Remote join) → RELAYING (forwarding) → WAITING (Remote left)
  │                                                                      │
  └── shutdown ──────────────────────────────────────────────────────────┘
```

## 13.8 Deployment

Phase 1: single binary + Docker Compose (server + monitoring stack).

```
docker-compose.yml:
  mediaservo-server   # main service
  prometheus          # metrics collection
  grafana             # monitoring dashboards
```

server.conf configures ports, PSK, and the database path. Sensitive values are injected via env vars in Docker deployment.

## 13.9 Phase Evolution

| Phase | Architecture | Notes |
|-------|--------------|-------|
| Phase 1 | Single binary (axum + relay in one process) | Signaling, forwarding, monitoring unified, ~800 lines |
| Phase 2 | mediasoup SFU integration | Multi-participant SFU relay, worker management, crash recovery <5s, SDP adapter |
| Phase 2+ | Multi-node scaling | The relay scales horizontally independently; signaling splits into its own service |

## 13.10 Cross References

- Signaling protocol and message formats: [signaling architecture](10-signaling-architecture.md)
- Overall three-layer architecture: [architecture doc](../architecture.md) §2
- Deployment forms: [deployment modes](02-deployment-modes.md)
- Host-side capture and push: [client & host](03-client-host.md)
