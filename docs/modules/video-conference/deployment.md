# Video Conference Deployment Reference

## 1. Deployment Forms

The video-conference module supports four deployment forms, consistent with MediaServo's overall architecture:

### 1.1 Embed (Rust crate)

**Scenario**: third-party platform embedded use
**Characteristics**: statically linked, ~5 plugins, lightweight

```
Third-party platform binary
  └── mediaservo-core
      └── mediaservo-conference (crate)
          └── mediasoup (statically linked)

Deployment: single binary, no external deps
Capacity: <50 sessions/SFU Worker
```

### 1.2 Sidecar (napi-rs bindings)

**Scenario**: host-platform enterprise apps
**Characteristics**: container + napi-rs bindings, ~12 plugins

```
Host-platform Docker
  └── mediaservo-service
      ├── napi-rs binding
      └── mediaservo-conference
          └── mediasoup Worker pool

Deployment: Docker container, sidecar mode
Capacity: <200 sessions/node
```

### 1.3 Standalone

**Scenario**: standalone deployment
**Characteristics**: complete backend + Web UI, standalone processes

```
mediaservo-server
  ├── Conference Controller (Rust)
  ├── Signaling (WebSocket)
  ├── mediasoup Worker pool
  ├── Recording pipeline
  └── Admin Web UI

Deployment: bare metal / VM / container
Capacity: <1000 sessions/node (depending on Worker count)
```

### 1.4 Platform Module

**Scenario**: host-platform integration
**Characteristics**: Docker container module, auth delegated to the platform

```
Host platform
  └── Container: mediaservo-conference
      ├── RBAC/LDAP delegated to the platform
      └── Independent SFU Worker pool

Deployment: host-platform container orchestration
Capacity: depends on the host platform's resource allocation
```

## 2. Configuration Reference

```yaml
# config.yaml — video-conference module configuration

conference:
  # Signaling
  signaling:
    port: 8000
    tls: true
    cert_path: "/etc/mediaservo/certs/server.pem"
    key_path: "/etc/mediaservo/certs/server.key"
    max_connections: 10000
    rate_limit: 1000  # Max requests per second

  # SFU
  sfu:
    worker_pool:
      min_workers: 2
      max_workers: 16
      auto_scaling: true
      cooldown_sec: 300
    worker:
      log_level: "warn"
      rtc_min_port: 40000
      rtc_max_port: 49999

  # Rooms
  room:
    default_max_participants: 16
    max_duration_sec: 86400   # 24h
    inactivity_timeout_sec: 300  # Auto-end after 5min with no participants
    max_rooms_per_user: 5

  # Media
  media:
    default_video_codec: "vp9"
    default_audio_codec: "opus"
    max_bitrate: 5000       # kbps
    max_resolution: "1080p"
    simulcast: true
    svc: true
    audio:
      fec: true
      dtx: true
      bitrate: 30000        # bps

  # Recording
  recording:
    enabled: true
    storage_path: "/var/mediaservo/recordings"
    format: "mp4"
    max_duration_sec: 43200  # 12h
    retention_days: 90

  # ICE / TURN
  ice:
    stun_servers:
      - "stun:stun.l.google.com:19302"
      - "stun:stun1.l.google.com:19302"
    turn_servers: []
    # turn_servers:
    #   - urls: "turn:turn.example.com:3478"
    #     username: "user"
    #     credential: "pass"

  # Distributed
  distributed:
    enabled: false
    redis_url: "redis://localhost:6379"
    region: "default"
    cluster:
      nodes: []
```

## 3. Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CONFERENCE_SIGNALING_PORT` | 8000 | Signaling port |
| `CONFERENCE_SFU_MIN_PORT` | 40000 | RTC min port |
| `CONFERENCE_SFU_MAX_PORT` | 49999 | RTC max port |
| `CONFERENCE_REDIS_URL` | - | Redis connection |
| `CONFERENCE_REGION` | default | Deployment region |
| `CONFERENCE_RECORDING_PATH` | /var/mediaservo/recordings | Recording storage path |
| `CONFERENCE_RECORDING_ENABLED` | true | Enable recording |
| `CONFERENCE_DEFAULT_CODEC` | vp9 | Default video codec |
| `CONFERENCE_MAX_BITRATE` | 5000 | Max bitrate (kbps) |
| `CONFERENCE_MAX_ROOMS` | 1000 | Max room count |
| `CONFERENCE_WORKER_MIN` | 2 | Min Worker count |
| `CONFERENCE_WORKER_MAX` | 16 | Max Worker count |

## 4. Docker Deployment

```dockerfile
# Dockerfile — conference module
FROM ubuntu:24.04 AS base
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

FROM base AS build
RUN apt-get install -y --no-install-recommends \
    build-essential cmake clang libclang-dev \
    && rm -rf /var/lib/apt/lists/*
# ... build mediaservo-conference ...

FROM base
COPY --from=build /opt/mediaservo/bin/mediaservo-conference /usr/local/bin/
COPY config.yaml /etc/mediaservo/config.yaml
EXPOSE 8000 40000-49999
CMD ["mediaservo-conference", "--config", "/etc/mediaservo/config.yaml"]
```

### docker-compose

```yaml
version: "3.8"
services:
  conference:
    image: mediaservo/conference:latest
    ports:
      - "8000:8000"
      - "40000-49999:40000-49999/udp"
    volumes:
      - ./config.yaml:/etc/mediaservo/config.yaml
      - ./recordings:/var/mediaservo/recordings
      - ./certs:/etc/mediaservo/certs
    environment:
      - CONFERENCE_REGION=beijing
      - RUST_LOG=info
    deploy:
      resources:
        limits:
          cpus: "8"
          memory: "8G"

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"
```

## 5. Firewall Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 8000 | TCP | Signaling WebSocket |
| 8001 | TCP | Inter-service gRPC |
| 40000-49999 | UDP | WebRTC media |
| 443 | TCP | Optional TLS signaling |
| 3478 | UDP | TURN service |

## 6. Monitoring

```yaml
# Prometheus metrics endpoint
metrics:
  port: 9090
  path: "/metrics"

# Key metrics
- conference_rooms_active      # Active room count
- conference_participants_total # Total participants
- conference_workers_count      # Worker count
- conference_sfu_cpu_usage     # SFU CPU usage
- conference_bitrate_total     # Total bitrate (bps)
- conference_packet_loss       # Packet loss rate
- conference_connection_errors # Connection error count
- conference_recording_active  # Active recordings
```

## 7. Capacity Planning

| Spec | Small | Medium | Large |
|------|-------|--------|-------|
| CPU | 4 cores | 8 cores | 32 cores |
| Memory | 8 GB | 16 GB | 64 GB |
| Worker count | 2 | 6 | 24 |
| Concurrent rooms | 50 | 200 | 1000 |
| Concurrent participants | 200 | 2000 | 10000 |
| Bandwidth | 100 Mbps | 1 Gbps | 10 Gbps |
| Deployment form | Embed | Sidecar/Standalone | Standalone cluster |

## 8. High Availability

```
                 ┌──────────┐
                 │  LB      │
                 │ (DNS/GLB)│
                 └────┬─────┘
                       │
         ┌─────────────┼─────────────┐
         │             │             │
         ▼             ▼             ▼
   ┌──────────┐  ┌──────────┐  ┌──────────┐
   │ Node 1   │  │ Node 2   │  │ Node N   │
   │ Beijing  │  │ Beijing  │  │ Shanghai │
   └──────────┘  └──────────┘  └──────────┘
         │             │             │
         └─────────────┼─────────────┘
                       │
                  ┌──────────┐
                  │  Redis   │
                  │  Cluster │
                  └──────────┘
```

- Multi-node horizontal scaling, Redis-shared state
- Region-nearest access, cross-region cascading
- Conferences auto-migrate on node failure
- No single point of failure
