# OpenVidu Deployment Reference

> Generated: 2026-07-31 | Category: Video Conference Deployment

## 1. Overview

OpenVidu is an open-source WebRTC video-conferencing platform offering a complete solution from signaling to the media layer. This document focuses on its Docker Compose deployment architecture: 12 cooperating containers covering the reverse proxy, signaling, media engine (embedded LiveKit), recording/streaming, TURN (built into LiveKit), and object storage across the full chain.

All images come from Docker Hub (`docker.io/openvidu/...`).

### 1.1 Versions & Licensing

| Dimension | Notes |
|-----------|-------|
| Community edition | free deployment on LiveKit (the Pion Go SFU), Apache 2.0 license |
| Professional edition | adds mediasoup C++ engine switching, the v2 compatibility layer, priority support; commercial license |
| Recommended deployment | Docker Compose (single machine) or Kubernetes (cluster) |
| Version | reference v2.29+, stable as of July 2026 |

### 1.2 Container Overview

```
┌───────────────────────────────────────────────────────────────────┐
│                      OpenVidu Docker Compose                        │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐      │
│  │  Caddy   │  │ OpenVidu │  │  MongoDB │  │    Redis     │      │
│  │ reverse  │  │ signaling│  │persistence│  │cache/queue   │      │
│  │ proxy    │  │ + engine │  │          │  │              │      │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └──────┬───────┘      │
│       │             │             │                │               │
│  ┌────┴─────┐  ┌────┴─────┐  ┌────┴─────┐  ┌──────┴────────┐     │
│  │  MinIO   │  │Dashboard │  │ Ingress  │  │    Egress     │     │
│  │ recording│  │ admin    │  │ media    │  │ recording /   │     │
│  │ storage  │  │ panel    │  │ ingest   │  │ streaming out │     │
│  └──────────┘  └──────────┘  └──────────┘  └───────────────┘     │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐      │
│  │ Operator │  │   Meet   │  │RdyCheck  │  │    Setup     │      │
│  │ deployment│ │ web app  │  │ health   │  │ busybox init  │     │
│  │ manager  │  │          │  │ check    │  │              │      │
│  └──────────┘  └──────────┘  └──────────┘  └──────────────┘      │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────┐       │
│  │  LiveKit Server (Go binary, running inside openvidu)     │       │
│  │    · SFU media forwarding (Pion WebRTC v3)                │       │
│  │    · built-in TURN/STUN (port 3478) — no Coturn needed   │       │
│  └─────────────────────────────────────────────────────────┘       │
└───────────────────────────────────────────────────────────────────┘
```

**Key architectural facts:**
- LiveKit is not a standalone container — it is a Go binary (`livekit-server`) running inside the `openvidu` container
- TURN/STUN is provided by LiveKit built-in (port 3478); no separate Coturn container
- the database is MongoDB (replica set), not PostgreSQL
- there is no Kurento container — recording/streaming is handled by the Egress service

### 1.3 Port Mapping

| Port | Protocol | Purpose | Source |
|------|----------|---------|--------|
| 443 | TCP | HTTPS entry (Caddy TLS termination) | caddy |
| 5443 | TCP | OpenVidu signaling WebSocket API | openvidu |
| 7880 | TCP | LiveKit HTTP health check (inside the openvidu container) | openvidu |
| 7881 | TCP | LiveKit RTC-over-TCP fallback | openvidu |
| 7900-7999 | UDP | LiveKit media ports (ICE/DTLS/SRTP) | openvidu |
| 3478 | TCP/UDP | LiveKit built-in STUN/TURN | openvidu |
| 5349 | TCP/UDP | STUN/TURN over TLS (LiveKit built-in) | openvidu |
| 6379 | TCP | Redis internal | redis |
| 27017 | TCP | MongoDB internal | mongo |
| 9000 | TCP | MinIO S3 API | minio |
| 9001 | TCP | MinIO web console | minio |

## 2. Docker Compose Services in Detail

### 2.1 Caddy (Reverse Proxy)

OpenVidu uses Caddy as its TLS-termination proxy. Caddy requests certificates from Let's Encrypt automatically, with no manual certificate-file management.

**Configuration highlights:**

- reverse-proxy the `/openvidu` path to the `openvidu` service, with WebSocket upgrade
- reverse-proxy LiveKit signaling (routed through the openvidu container)
- HTTP/2 enabled globally, but no protocol upgrade on media-stream paths
- TLS certificate paths persisted via Docker volumes
- the default domain is `openvidu.example.com`; replace with the real domain at deployment

**Core Caddyfile section (simplified):**

```
openvidu.example.com {
    reverse_proxy /openvidu* openvidu:5443 {
        header_up Host {host}
    }
}
```

### 2.2 OpenVidu Signaling Service (incl. the LiveKit Engine)

The OpenVidu main container provides the REST API and WebSocket signaling, owning room management, participant lifecycles, and recording-job scheduling. **Its key difference: the LiveKit Server runs as a Go binary (`livekit-server`) inside this container**, rather than deployed separately.

That means LiveKit's SFU media engine, signaling, and TURN/STUN all live inside the single `openvidu` container. Internal communication rides localhost with no extra network hop.

**Environment variables:**

```yaml
DOMAIN_OR_PUBLIC_IP=openvidu.example.com
OPENVIDU_SECRET=your-secret-key
OPENVIDU_RECORDING=true
OPENVIDU_RECORDING_PATH=/opt/openvidu/recordings
OPENVIDU_CDR=true  # Call Detail Records
LIVEKIT_KEYS: "openvidu: your-api-key"
LIVEKIT_RTC_PORT_RANGE: "7900-7999"
LIVEKIT_RTC_TCP_PORT: "7881"
LIVEKIT_RTC_UDP_PORT_RANGE: "7900-7999"
LIVEKIT_TURN_ENABLED: "true"
LIVEKIT_TURN_PORT: "3478"
LIVEKIT_TURN_TLS_PORT: "5349"
```

**Request body size limit:** OpenVidu caps request bodies at 10MB by default against large-upload attacks. In production, tune `OPENVIDU_WEBHOOK_MAXBODYSIZE` to real needs.

### 2.3 LiveKit (Community-Edition SFU Core — Embedded in the openvidu Container)

LiveKit is the default media engine of OpenVidu Community — **not a standalone container** but the `livekit-server` Go binary running inside `openvidu`. It builds its WebRTC stack on the Pion libraries. Single-binary deployment, stable performance, good protocol compatibility.

**LiveKit internal architecture:**

```
┌─────────────────────────────────────────────┐
│              LiveKit Server (Go)              │
│                                               │
│  ┌─────────────────────────────────────────┐ │
│  │         RTC layer (Pion/webrtc v3)       │ │
│  │  ┌──────────┐  ┌──────────┐             │ │
│  │  │ ICE      │  │ DTLS     │             │ │
│  │  │ transport│  │ transport│             │ │
│  │  └──────────┘  └──────────┘             │ │
│  │  ┌──────────┐  ┌──────────┐             │ │
│  │  │ SRTP/SRTCP│ │ DataChannel│            │ │
│  │  └──────────┘  └──────────┘             │ │
│  └─────────────────────────────────────────┘ │
│                                               │
│  ┌─────────────────────────────────────────┐ │
│  │     Room routing layer                    │ │
│  │  · room creation/teardown                   │ │
│  │  · participant Track publish/subscribe     │ │
│  │  · 3 Track kinds: audio/video/screenshare  │ │
│  │  · unicast → broadcast forwarding          │ │
│  │    (Selective Forwarding)                   │ │
│  └─────────────────────────────────────────┘ │
│                                               │
│  ┌─────────────────────────────────────────┐ │
│  │  Signaling layer (binary WS protocol)    │ │
│  │  · Protobuf encoding                     │ │
│  │  · SDP negotiation                       │ │
│  │  · ICE candidate exchange                 │ │
│  │  · Track subscription management          │ │
│  ├─────────────────────────────────────────┤ │
│  │  TURN/STUN layer (built-in)              │ │
│  │  · STUN binding requests (port 3478)     │ │
│  │  · TURN relay (ports 3478 / 5349 TLS)    │ │
│  └─────────────────────────────────────────┘ │
└─────────────────────────────────────────────┘
```

**LiveKit media port ranges:**

| Range | Purpose | Protocol |
|-------|---------|----------|
| 7900-7999 | ICE/DTLS/SRTP media streams | UDP |
| 7881 | RTC-over-TCP fallback | TCP |
| 3478 | LiveKit built-in STUN/TURN | TCP/UDP |
| 5349 | STUN/TURN over TLS | TCP/UDP |

100 UDP ports support roughly 300-500 concurrent participants at typical loads (depending on bitrate, resolution, frame rate).

**How the built-in TURN works:**

LiveKit ships an integrated TURN server — no Coturn container needed. When a client cannot establish P2P ICE, the LiveKit TURN server automatically relays. Configuration happens via `openvidu` container env vars:

```yaml
LIVEKIT_TURN_ENABLED: "true"
LIVEKIT_TURN_PORT: "3478"
LIVEKIT_TURN_TLS_PORT: "5349"
```

### 2.4 The mediasoup Engine (Professional Edition)

The Professional edition adds a mediasoup engine alongside LiveKit, switched via the `openvidu.rtc.engine` setting. mediasoup uses C++ Worker processes for media, delivering higher performance and finer-grained encode control.

**How to switch:**

```yaml
# in the openvidu container env
OPENVIDU_RTC_ENGINE=mediasoup  # or pion (default)

# mediasoup extras
OPENVIDU_MEDIASOUP_WORKER_BIN=/opt/mediasoup/worker/mediasoup-worker
OPENVIDU_MEDIASOUP_RTC_MIN_PORT=7900
OPENVIDU_MEDIASOUP_RTC_MAX_PORT=7999
OPENVIDU_MEDIASOUP_NUM_WORKERS=2
```

**mediasoup vs LiveKit differences:**

| Dimension | LiveKit engine | mediasoup engine |
|-----------|----------------|------------------|
| Language | Go (Pion) | C++ Worker + Rust/Node.js bindings |
| Process model | single process, many coroutines | multiple Worker child processes |
| Encode control | coarse (bitrate/resolution) | fine (per-Consumer encode parameters) |
| Scaling | horizontal (Redis cluster routing) | vertical (Workers pinned to CPU cores) |
| Latency | 3-5ms forwarding (Pion-tuned) | 1-2ms forwarding (native C++) |
| Resource use | ~5MB memory per participant | ~50MB baseline per Worker process |

### 2.5 The v2 Compatibility Layer (Professional Edition)

The Professional edition ships the `v2compatibility` service implementing OpenVidu v2 API compatibility — critical when migrating from v2 to v3.

**APIs covered by the compatibility layer:**

- Session create/close (mapped to LiveKit Rooms)
- Connection create (mapped to LiveKit Participants)
- Publisher/Subscriber model (mapped to LiveKit Track publish/subscribe)
- Token generation (v2 roles mapped to LiveKit canPublish/canSubscribe)

**Compatibility limits:**

- the `Filter` API (v2 media-processing filters) only works on the mediasoup engine
- v2 recording-callback formats differ from v3 and need adaptation
- some v2 events (e.g. `participantPublished`) may map fields differently

### 2.6 Redis (Cache / Queue / Pub-Sub)

Redis plays three roles in OpenVidu:

| Role | Purpose | Data structure |
|------|---------|----------------|
| cache | session Tokens, room metadata, short-lived config | String (with TTL) |
| queue | async recording jobs, webhook events | List (RPUSH/LPOP) |
| pub/sub | cross-service event notification (room-state changes) | Pub/Sub |

**Persistence configuration:**

```yaml
redis:
  image: redis:7-alpine
  command: redis-server --appendonly yes --save 60 1000
  volumes:
    - redis-data:/data
```

AOF (Append Only File) persistence fsyncs per second so cached data survives restarts; RDB snapshots act as a secondary backup every 60s or 1000 writes.

### 2.7 MongoDB (Persistent Database)

The MongoDB replica set stores OpenVidu's persistent data:

- user accounts and roles
- recording metadata (recording ID, timestamps, file paths, status)
- CDR (Call Detail Records)
- application config (webhook URLs, recording-storage policies)

**Replica-set configuration:**

```yaml
mongo:
  image: mongo:7
  command: mongod --replSet rs0 --bind_ip_all
  volumes:
    - mongo-data:/data/db
```

In production MongoDB runs as a replica set for high availability and automatic failover. OpenVidu connects through the MongoDB driver; read/write splitting boosts performance.

### 2.8 MinIO (Recording Storage)

MinIO provides S3-compatible recording storage. OpenVidu uploads generated `.webm` files via the MinIO client SDK.

**Storage path layout:**

```
recordings/
  └── {session_id}/
      ├── participant_{participant_id}/
      │   ├── video_{timestamp}.webm
      │   └── audio_{timestamp}.webm
      └── metadata.json
```

**Data lifecycle:**

MinIO keeps every file by default. Production should configure lifecycle policies:

```yaml
# via the MinIO client
mc ilm rule add --expire-days 90 myminio/recordings
```

### 2.9 Dashboard (Admin Panel)

The Dashboard container serves the web admin panel:

- live view of active sessions and participants
- recording-file management and playback
- system configuration and engine switching
- monitoring-metric visualization

Image: `docker.io/openvidu/dashboard`

### 2.10 Ingress (Media Ingest)

The Ingress service handles external media ingest:

- RTMP push ingest
- WHIP (WebRTC-HTTP Ingestion Protocol)
- pulling external RTSP sources

Ingress converts external streams into LiveKit Tracks consumable by conference participants.

### 2.11 Egress (Recording / Streaming Out)

The Egress service handles recording composition and stream output, replacing Kurento from OpenVidu v2:

- **Composited recording**: multiple feeds mixed into one (Composite mode) or recorded separately (Individual mode)
- **RTMP stream out**: push conference video to streaming platforms (YouTube, Twitch, etc.)
- **File output**: MP4 / WebM recordings written straight to MinIO

Egress talks to LiveKit directly over gRPC to pull media — bypassing the OpenVidu signaling layer.

### 2.12 Operator (Deployment Management)

The Operator container manages the OpenVidu deployment lifecycle:

- automatic certificate renewal
- service health monitoring and auto-restart
- hot config updates
- version-upgrade orchestration

### 2.13 The Setup Container (Initialization)

The Setup container runs the `busybox` image, initializing state before the OpenVidu main service starts:

```yaml
setup:
  image: busybox:1.36
  entrypoint:
    - /bin/sh
    - -c
    - |
      mkdir -p /opt/openvidu/recordings
      chown -R 1000:1000 /opt/openvidu/recordings
      mkdir -p /opt/openvidu/custom-layout
      echo "Init complete"
  volumes:
    - openvidu-data:/opt/openvidu
```

**Work performed by the Setup container:**

1. create the recording directory `/opt/openvidu/recordings` (OpenVidu user UID 1000)
2. set directory permissions ensuring the OpenVidu container can write
3. create the custom-layout directory `/opt/openvidu/custom-layout`
4. initialize the log directory `/opt/openvidu/logs`
5. exit when done (`restart: "no"`)

### 2.14 Ready Check (Health Gate)

The Ready Check container verifies every dependency (MongoDB, Redis, MinIO, LiveKit) is up at boot; once green it signals readiness to Caddy, which only then starts forwarding traffic.

### 2.15 Meet (Web Conference App)

The OpenVidu Meet container ships a ready-to-use React-based web conference front end. Users join meetings from a browser with no client development needed.

Image: `docker.io/openvidu/openvidu-meet`

## 3. Network Topology

### 3.1 Internal Network

OpenVidu uses the Docker custom network `openvidu-net`, default subnet `172.28.0.0/16`. All containers address each other by service name — no manual IPs.

```
openvidu-net (172.28.0.0/16)
 ├── caddy (172.28.0.2)
 ├── openvidu (172.28.0.3)  ← the LiveKit Server binary runs inside
 ├── mongo (172.28.0.4)
 ├── redis (172.28.0.5)
 ├── minio (172.28.0.6)
 ├── dashboard (172.28.0.7)
 ├── ingress (172.28.0.8)
 ├── egress (172.28.0.9)
 ├── operator (172.28.0.10)
 ├── meet (172.28.0.11)
 ├── ready-check (172.28.0.12)
 └── setup (172.28.0.13)
```

### 3.2 External Traffic Paths

```
External clients
    │
    ├── HTTPS (443) → Caddy → TLS termination
    │   ├── /openvidu/* → WebSocket/REST → openvidu:5443
    │   └── /livekit/*  → WebSocket → openvidu (LiveKit embedded)
    │
    ├── STUN (3478) → openvidu (LiveKit built-in STUN)
    │
    ├── TURN (5349/TLS) → openvidu (LiveKit built-in TURN/TLS)
    │
    └── UDP media (7900-7999) → openvidu (LiveKit ICE/DTLS/SRTP)
```

### 3.3 Firewall Rules

Production needs at least these inbound rules:

| Rule | Port | Protocol | Source | Notes |
|------|------|----------|--------|-------|
| HTTPS | 443 | TCP | 0.0.0.0/0 | web access and signaling |
| STUN | 3478 | TCP/UDP | 0.0.0.0/0 | candidate discovery (LiveKit built-in) |
| TURN/TLS | 5349 | TCP/UDP | 0.0.0.0/0 | encrypted relay (LiveKit built-in) |
| media ports | 7900-7999 | UDP | 0.0.0.0/0 | RTP/RTCP media streams |
| SSH | 22 | TCP | admin IPs only | server administration |

## 4. Startup Flow

### 4.1 First-Boot Order

```
time  container       action
 │    ├── setup       create directory structure, set permissions, exit
 │    ├── mongo       start the database replica set, wait for readiness
 │    ├── redis       start the cache, wait for readiness
 │    ├── minio       start S3 storage, wait for readiness
 │    ├── openvidu    start the main service + the LiveKit Server binary
 │    │   ├── connect to MongoDB, Redis, MinIO
 │    │   ├── start LiveKit Server (ports 7880/7881/7900-7999/3478/5349)
 │    │   └── start the signaling WebSocket service (port 5443)
 │    ├── ingress     start the media-ingest service
 │    ├── egress      start the recording/streaming service
 │    ├── dashboard   start the admin panel
 │    ├── meet        start the web conference front end
 │    ├── operator    start deployment management
 │    ├── ready-check verify every dependency is ready
 │    └── caddy       start last, configure TLS certs, begin accepting traffic
 ▼
ready                 ready-check returns 200; Caddy starts forwarding
```

### 4.2 Health Checks

OpenVidu exposes several health endpoints:

| Endpoint | Purpose | Returns |
|----------|---------|---------|
| `/openvidu/health` | the OpenVidu main service | `200 OK` |
| `/livekit/health` | the LiveKit engine (inside openvidu) | `serving` |
| `/openvidu/api/rooms` | API availability | the room list |

### 4.3 Log Diagnostics

**Common startup problems:**

| Symptom | Root cause | Fix |
|---------|------------|-----|
| Caddy TLS cert request fails | the domain isn't pointed at the public IP | check the DNS A record |
| LiveKit fails to start | ports 7900-7999 occupied | check `netstat -tulpn \| grep 7900` |
| MongoDB replica set uninitialized | first deployment needs a manual `rs.initiate()` | run it in the mongo shell |
| recordings not writable | MinIO not ready or permission denied | check MinIO logs and bucket policy |

## 5. Persistent Storage

### 5.1 Volume Mapping

```yaml
volumes:
  caddy-data:
    external: false
  openvidu-data:
    external: false
  redis-data:
    external: false
  mongo-data:
    external: false
  minio-data:
    external: false
```

Production should use NFS or a cloud provider's persistent volumes so data survives container restarts.

### 5.2 Backup Strategy

| Data | Frequency | Method | Retention |
|------|-----------|--------|-----------|
| MongoDB | daily | mongodump + upload to MinIO | 30 days |
| Redis AOF | hourly | copy the AOF file | 7 days |
| MinIO recordings | on demand | cross-region replication | 90 days |
| Caddy certs | on each renewal | auto-backup to MinIO | 365 days |

## 6. Engine Switching in Detail

### 6.1 Runtime Switching

OpenVidu Professional supports switching the RTC engine at runtime via environment variable. Restart the `openvidu` container afterwards — no full-stack redeploy.

```
openvidu.rtc.engine = pion
    └── OpenVidu drives the embedded LiveKit Server through the LiveKit client SDK
    └── all media routing goes through LiveKit's Pion engine

openvidu.rtc.engine = mediasoup
    └── OpenVidu drives mediasoup Workers through the mediasoup client SDK
    └── all media routing goes through mediasoup's C++ Workers
    └── mediasoup Workers are embedded in the OpenVidu container — no extra container
```

### 6.2 Switch Impact Surface

| Dimension | Pion engine | mediasoup engine |
|-----------|-------------|------------------|
| container count | 12 (standard) | 12 (unchanged) |
| memory | ~200MB baseline | ~400MB baseline |
| CPU | ~10% forwarding 100 streams | ~5% forwarding 100 streams |
| signaling protocol | LiveKit Protobuf | mediasoup JSON |
| recording path | Egress service | Egress service (unchanged) |
| v2 compatibility | mostly compatible | fully compatible (incl. Filter) |

### 6.3 Switching Advice

- **choose Pion**: lightweight deployment, fast startup, the community-edition default. Good for small/medium conferences (<50 participants), IoT devices, rapid prototyping
- **choose mediasoup**: high performance, large conferences (100+ participants), fine-grained encode control. Good for professional conferencing, live streaming, education platforms

## 7. Borrowings for MediaServo

### 7.1 Architectural Borrowings

| OpenVidu practice | MediaServo counterpart | Value |
|-------------------|------------------------|-------|
| 12-container microservices | the 7-crate workspace | clear containerization boundaries, crisp service responsibilities |
| Caddy reverse proxy for TLS termination | mediaservo-server currently exposed directly | add a proxy layer to separate certificate management |
| the Setup container does directory init | mediaservo-host initializes at startup | separate initialization logic so container startup order stays correct |
| swappable engines (pion/mediasoup) | mediaservo-webrtc's three-backend abstraction | same switching pattern; reuse LiveKit's config-driven approach |
| the LiveKit embedded mode (no standalone SFU container) | mediaservo-server's SFU integration | SFU + service in one container cuts network hops and deployment complexity |
| TURN provided built into the SFU | mediaservo-server ICE candidates | cuts the ops overhead of a standalone TURN container |

### 7.2 Port-Planning Borrowings

| Range | Takeaway |
|-------|----------|
| 7900-7999 | a 100-port range suits medium deployments; MediaServo's current 40000-40100 is too narrow |
| 3478/5349 | LiveKit's built-in TURN ports; MediaServo can consider folding STUN/TURN into the SFU rather than standalone deployment |

### 7.3 Introducing a Reverse-Proxy Layer

MediaServo currently exposes port 9800 directly on `mediaservo-server`. Borrowing OpenVidu's Caddy pattern:

```
Today:
  Client → mediaservo-server:9800 (direct, no TLS)

Borrowing:
  Client → Caddy (443 TLS) → mediaservo-server:9800 (private network)
```

Advantages:
- automatic TLS certificate management (Let's Encrypt)
- path routing (/signaling, /admin, /livekit, etc.)
- unified WebSocket upgrade handling
- centralized load balancing and health checks

### 7.4 Engine-Switch Strategy

OpenVidu's config-driven `openvidu.rtc.engine` switching matches MediaServo's `mediaservo-webrtc` three-backend abstraction. MediaServo can borrow its configuration shape:

```toml
# MediaServo future config
[webrtc]
backend = "webrtc-sys"  # webrtc-rs | webrtc-sys | str0m
```

The community edition defaults to `webrtc-sys` (stable libwebrtc performance); the professional edition allows switching. The switch logic lives in `mediaservo-webrtc/src/backend/`, decided jointly by feature flags and runtime config.

### 7.5 Recording Architecture

OpenVidu's recording depends on MinIO + Egress (no more Kurento). MediaServo's recording path can be simplified likewise:

```
MediaServo recording (today):
  Host → RTP → mediaservo-server → file write

OpenVidu recording (borrowed):
  LiveKit → Egress compose/encode → MinIO S3 storage → lifecycle management
```

MediaServo's future recording can introduce a MinIO-compatible layer, reusing OpenVidu's S3 recording paths and lifecycle policies.
