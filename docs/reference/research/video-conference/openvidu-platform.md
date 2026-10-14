# OpenVidu Platform Reference Analysis

> Generated: 2026-07-31 | Category: Video Conference
> Repository: https://github.com/OpenVidu/openvidu

## 1. Product Profile

- **Name**: OpenVidu Platform
- **Developer**: OpenVidu (a Spanish team, originating from the Kurento project at Universidad Rey Juan Carlos)
- **Key figure**: Luis López (CEO, community-driven)
- **First release**: 2016 (OpenVidu v1 based on Kurento)
- **Product positioning**: OpenVidu is a video conferencing platform, not a media engine. It wraps a complete REST API + client SDK + deployment suite + operations tooling on top of an underlying media engine (Kurento / mediasoup / LiveKit), with the goal of letting developers embed video conferencing into an app in 5 lines of code
- **Target users**: SMBs needing to integrate video conferencing quickly, education platforms, and telemedicine applications. Typical users: online education platforms (OpenVidu has a high market share in European online education), remote medical consultations, and enterprise collaboration SaaS integrations
- **Licensing / business model**: Apache 2.0 (CE community edition, free). OpenVidu Pro and Enterprise are paid editions providing elastic clusters, high availability, and commercial support. After the v3 migration, the CE edition gained many features previously exclusive to Pro

## 2. Architecture

### 2.1 Master-Worker Architecture

OpenVidu uses a classic two-tier Master-Worker architecture with the control plane and media plane completely separated:

```
┌──────────────────────────────────────────────────────────────────────┐
│  openvidu-server (Java Spring Boot) — Master                         │
│                                                                      │
│  ┌────────────────────┐  ┌────────────────────┐                     │
│  │  REST API          │  │  WebSocket Signaling│                    │
│  │  · POST /sessions  │  │  · SDP negotiation  │                     │
│  │  · POST /connections│  │  · ICE exchange     │                     │
│  │  · DELETE /sessions │  │  · connection state │                     │
│  └────────────────────┘  └────────────────────┘                     │
│  ┌────────────────────┐  ┌────────────────────┐                     │
│  │  Session Manager   │  │  Token Auth        │                     │
│  │  · room lifecycle  │  │  · PUBLISHER/       │                     │
│  │  · participant     │  │    SUBSCRIBER roles │                     │
│  │    management      │  │  · expiry/revocation│                     │
│  │  · media routing   │  │                     │                     │
│  └────────────────────┘  └────────────────────┘                     │
│  ┌────────────────────┐  ┌────────────────────┐                     │
│  │  Recording Manager │  │  CDR (Call Detail  │                     │
│  │  · recording start/│  │  Record)           │                     │
│  │    stop            │  │  · call statistics  │                     │
│  │  · file management │  │                     │                     │
│  └────────────────────┘  └────────────────────┘                     │
└──────────────────────────┬───────────────────────────────────────────┘
                           │ REST / WebSocket
┌──────────────────────────┴──────────────────────────────────────────┐
│  Media Node (Worker) — media plane                                  │
│                                                                      │
│  v2: Kurento Media Server (C++/GStreamer)                           │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Kurento Media Pipeline                                       │   │
│  │  WebRtcEndpoint → Composite (MCU) / Dispatcher (SFU)         │   │
│  │  RecorderEndpoint → WebM/MP4                                  │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│  v3 Enterprise: mediasoup (C++/Rust)                                │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  mediasoup Worker → Router → Transport → Producer/Consumer    │   │
│  │  Simulcast/SVC layer selection, 1000+ streams per node        │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│  v3 CE: openvidu-livekit (Go, LiveKit fork)                         │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  LiveKit Server (Pion WebRTC, pure Go)                        │   │
│  │  Signaling + SFU + recording (Egress) unified                 │   │
│  └──────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────┘
```

### 2.2 v2 vs v3 Architecture Comparison

| Dimension | OpenVidu v2 | OpenVidu v3 |
|------|------------|------------|
| Media engine | Kurento (GStreamer, C++) | LiveKit (CE) / mediasoup (Enterprise) |
| Architecture | MCU + SFU dual mode | Pure SFU (LiveKit/mediasoup) |
| Performance | ~100 streams per node | ~500-1000 streams per node (mediasoup) |
| Deployment | Docker Compose single node | Docker Compose / K8s |
| Monitoring | Elastic Stack | Prometheus + Loki + Grafana |
| E2EE | Not supported | Supported (Insertable Streams) |
| Migration path | — | The v2compatibility module preserves API compatibility |

### 2.3 Deployment Topology

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Browser     │     │  Browser     │     │  Browser     │
│  (Client)    │     │  (Client)    │     │  (Client)    │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │                    │                    │
       │  HTTPS + WSS       │  HTTPS + WSS       │  HTTPS + WSS
       │  (REST + Signal)   │  (REST + Signal)   │  (REST + Signal)
       ├────────────────────┼────────────────────┘
       │                    │
       ▼                    ▼
┌──────────────────────────────────────────────────────────────┐
│  openvidu-server (Java Spring Boot)                           │
│  · Signaling control plane (REST API + WebSocket)             │
│  · Session management · Token authentication · recording      │
│    orchestration                                              │
│  · Media node scheduling (CE: fixed / Pro: elastic)          │
└──────────────────────────┬───────────────────────────────────┘
                           │
                           │  Internal HTTP + WebSocket
                           │
┌──────────────────────────┴───────────────────────────────────┐
│  Media Node (Docker container)                                │
│  · Kurento (v2) / LiveKit (v3 CE) / mediasoup (v3 Ent)       │
│  · WebRTC media plane · recording execution                   │
│  · Lifecycle: launching → running → terminated                │
└──────────────────────────────────────────────────────────────┘
```

## 3. REST API

### 3.1 Session API

The core APIs of OpenVidu revolve around two abstractions, Session and Connection:

```
Session (room)
├── sessionId: unique identifier
├── createdAt: creation time
├── connections: Connection[]
├── recording: recording configuration
├── mediaMode: ROUTED / RELAYED
├── defaultOutputMode: COMPOSED / INDIVIDUAL
└── customSessionId: (optional) custom ID

Connection (participant connection)
├── connectionId: unique identifier
├── status: pending / connecting / connected / disconnected
├── role: PUBLISHER / SUBSCRIBER
├── token: authentication token
├── clientData: client custom data
├── serverData: server custom data
├── createdAt: creation time
├── location: geolocation (IP inferred)
├── platform: client platform (browser/OS version)
├── publishers: Publisher[] (PUBLISHER role)
└── subscribers: Subscriber[] (PUBLISHER role)
```

### 3.2 Key Endpoints

| Endpoint | Method | Description |
|------|------|------|
| `/api/sessions` | POST | Create a Session. Parameters: `customSessionId`, `mediaMode`, `recordingMode`, `defaultOutputMode` |
| `/api/sessions/<sessionId>` | GET | Get Session info |
| `/api/sessions/<sessionId>` | DELETE | Close the Session, disconnect all connections |
| `/api/sessions/<sessionId>/connection` | POST | Create a Connection. Parameters: `role`, `data`, `kurentoOptions` |
| `/api/sessions/<sessionId>/connection/<connId>` | DELETE | Disconnect a specific Connection |
| `/api/tokens` | POST | Generate a Token. Parameters: `session`, `role`, `data`, `kurentoOptions` |
| `/api/recordings/start` | POST | Start recording a Session |
| `/api/recordings/stop` | POST | Stop recording |
| `/api/recordings` | GET | List all recordings |
| `/api/recordings/<recordingId>` | DELETE | Delete a recording file |
| `/api/recordings/<recordingId>` | GET | Get recording details |
| `/api/health` | GET | Health check |
| `/api/call-detail-records` | GET | Call detail records (CDR) |

### 3.3 Token Authentication

OpenVidu uses a Token mechanism to control access permissions:

```json
// POST /api/sessions/<sessionId>/connection response
{
  "id": "con_xxxxx",
  "token": "wss://openvidu.example.com?sessionId=ses_xxx&token=tok_xxxxx",
  "status": "pending",
  "createdAt": 1720000000000,
  "role": "PUBLISHER",
  "clientData": "{\"displayName\":\"Alice\"}",
  "serverData": "{\"userId\":\"user_123\"}"
}
```

Token generation flow:
1. The server calls `POST /api/sessions` to create a Session
2. The server calls `POST /api/sessions/<id>/connection` to generate a Token
3. The Token contains the `role` (PUBLISHER/SUBSCRIBER) and custom data
4. The client uses the Token to connect to the WebSocket signaling
5. After connecting, media permissions are determined by role: PUBLISHER can publish and subscribe, SUBSCRIBER can only subscribe

### 3.4 Role Model

| Role | Publish permission | Subscribe permission | Typical scenario |
|------|---------|---------|---------|
| PUBLISHER | ✅ Publish audio/video | ✅ Subscribe to all | Regular participant |
| SUBSCRIBER | ❌ Cannot publish | ✅ Subscribe to all | Spectator/observer |
| (custom) | Composable via API | Composable via API | Custom roles |

## 4. Session Lifecycle

### 4.1 Standard Flow

```
┌─────────┐     ┌──────────┐     ┌──────────┐
│ Client  │     │ openvidu │     │ Media    │
│         │     │ -server  │     │ Node     │
└────┬────┘     └────┬─────┘     └────┬─────┘
     │                │                │
     │ 1. POST /sessions              │
     │────────────────>│              │
     │  {sessionId}    │              │
     │<────────────────│              │
     │                │                │
     │ 2. POST /connection            │
     │────────────────>│              │
     │  {token, role}  │              │
     │<────────────────│              │
     │                │                │
     │ 3. WebSocket connection        │
     │  (with token)   │              │
     │════════════════>│              │
     │                │                │
     │ 4. joinRoom     │  5. create media
     │────────────────>│  resources    │
     │                │──────────────>│
     │                │               │
     │ 6. SDP Offer   │ 7. processOffer│
     │<═══ SDP Offer ═══│════════════>│
     │                │              │
     │ 8. SDP Answer  │ 9. SDP Answer│
     │════════════════>│<══════════════│
     │                │              │
     │════════════════════════════════│
     │  ICE/DTLS/SRTP media channel   │
     │════════════════════════════════│
```

### 4.2 Session State Machine

```
         ┌──────────┐
         │  Created │    (POST /api/sessions)
         └────┬─────┘
              │
      ┌───────┴───────┐
      │  In Progress  │    (at least 1 connection)
      └───────┬───────┘
              │
       ┌──────┴──────┐
       │  Destroyed  │    (DELETE /api/sessions/<id>)
       └─────────────┘
```

A Session is destroyed automatically in the following cases:
- After all participants disconnect, once the `session.config.sessionGracefulShutdownTimeout` elapses (default 30 seconds)
- An explicit DELETE API call
- A media node failure

### 4.3 Media Modes

OpenVidu supports two media modes:

| Mode | Description | Applicable scenarios |
|------|------|---------|
| ROUTED | All media flows through the server. The server handles routing, recording, and transcoding. Media nodes are mandatory | Recording, transcoding, network quality optimization |
| RELAYED | P2P direct connection between clients. The server only acts as a signaling broker. Media nodes are not involved | Low latency, reduced server load |

## 5. Server Architecture

### 5.1 openvidu-server (Java Spring Boot)

openvidu-server is the control-plane hub of the entire platform:

```
openvidu-server/
├── src/main/java/io/openvidu/server/
│   ├── OpenViduServer.java           # main entry, Spring Boot bootstrap
│   ├── config/                       # configuration (CORS, Security, WebSocket)
│   ├── controllers/                  # REST API controllers
│   │   ├── SessionController.java    # /api/sessions
│   │   ├── ConnectionController.java # /api/connections
│   │   ├── RecordingController.java  # /api/recordings
│   │   ├── TokenController.java      # /api/tokens
│   │   └── HealthController.java     # /api/health
│   ├── services/                     # business logic
│   │   ├── SessionService.java       # session management
│   │   ├── MediaNodeService.java     # media node scheduling
│   │   ├── RecordingService.java     # recording management
│   │   └── CdrService.java           # call detail records
│   ├── websocket/                    # WebSocket signaling
│   │   ├── SignalingHandler.java     # signaling message handling
│   │   └── WebSocketConfig.java      # WS endpoint configuration
│   ├── models/                       # domain models
│   │   ├── Session.java
│   │   ├── Connection.java
│   │   ├── Token.java
│   │   ├── Recording.java
│   │   └── MediaNode.java
│   └── kurento/                      # Kurento integration (v2)
│       ├── KurentoSessionManager.java
│       └── KurentoMediaPipeline.java
```

### 5.2 Configuration Items

| Parameter | Default | Description |
|------|--------|------|
| `OPENVIDU_SECRET` | none | Server-side API secret |
| `OPENVIDU_PUBLIC_URL` | `https://localhost:4443` | Publicly accessible URL |
| `OPENVIDU_CDR` | false | Enable call detail records |
| `OPENVIDU_CDR_PATH` | `/opt/openvidu/cdr` | CDR log path |
| `OPENVIDU_RECORDING` | false | Enable the recording feature |
| `OPENVIDU_RECORDING_PATH` | `/opt/openvidu/recordings` | Recording file storage path |
| `OPENVIDU_RECORDING_COMPOSED_OVERLAY` | false | Composite recording watermark |
| `OPENVIDU_SESSIONS_GARBAGE_INTERVAL` | 900 | Session cleanup interval (seconds) |
| `OPENVIDU_SESSIONS_GARBAGE_THRESHOLD` | 3600 | Session expiry threshold (seconds) |
| `OPENVIDU_MEDIA_NODE_CPU_USE_THRESHOLD` | 80 | Media node CPU threshold (%) |
| `OPENVIDU_MEDIA_NODE_CPU_RESUME_THRESHOLD` | 60 | Threshold to resume scheduling (%) |

### 5.3 Health Check

openvidu-server exposes a `/api/health` endpoint returning JSON status:

```json
{
  "status": "UP",
  "mediaNodes": [
    {
      "id": "node_xxxxx",
      "status": "running",
      "cpuUsage": 45.2,
      "memoryUsage": 60.1,
      "sessionCount": 12,
      "connectionCount": 48
    }
  ],
  "version": "2.29.0"
}
```

## 6. openvidu-livekit (LiveKit Fork)

### 6.1 Positioning

openvidu-livekit is a core component of OpenVidu v3, but it is not a fork of upstream LiveKit — it is a LiveKit variant customized for OpenVidu, with the main changes concentrated in:

- **Analytics pipeline**: persists LiveKit's Prometheus metrics through MongoDB, providing historical query capability
- **Telemetry collection**: extends LiveKit's TelemetryService with OpenVidu-specific RTP/ICE quality metrics
- **v2compatibility module**: maintains backward compatibility with the OpenVidu v2 REST API
- **AI Agent code removed**: OpenVidu does not need the LiveKit Agents Framework

### 6.2 Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  openvidu-livekit (Go, based on LiveKit)                      │
│                                                              │
│  ┌────────────────────┐  ┌────────────────────┐             │
│  │  LiveKit Server    │  │  Analytics ext.     │            │
│  │  (Pion WebRTC)     │  │  ───────────────── │             │
│  │  · WebSocket       │  │  · RTP quality      │             │
│  │    signaling       │  │    statistics       │             │
│  │  · Room Service    │  │  · ICE connection   │             │
│  │  · SFU Pipeline    │  │    statistics       │             │
│  │  · Egress recording│  │  · client version   │             │
│  │                     │  │    statistics       │             │
│  └────────────────────┘  └────────────────────┘             │
│  ┌────────────────────┐  ┌────────────────────┐             │
│  │  v2compatibility   │  │  MongoDB Adapter    │            │
│  │  · REST API mapping│  │  · metric           │             │
│  │  · Token           │  │    persistence      │             │
│  │    conversion      │  │  · historical query │             │
│  │  · Session         │  │  · trend analysis   │             │
│  │    translation     │  │                     │             │
│  └────────────────────┘  └────────────────────┘             │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  Telemetry/Event Pipeline                             │  │
│  │  · participant join/leave → event record → MongoDB    │  │
│  │  · connection quality → 5s sampling → MongoDB time    │  │
│  │    series collection                                  │  │
│  │  · recording status → event record → MongoDB          │  │
│  └──────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

### 6.3 Core Differences from LiveKit

| Dimension | LiveKit (upstream) | openvidu-livekit (fork) |
|------|---------------|------------------------|
| AI Agent | First-class citizen, Agents Framework | Removed (OpenVidu does not need it) |
| Storage | Redis (distributed) / Local (standalone) | MongoDB (analytics only) |
| Routing | LocalRouter / RedisRouter | Simplified: single node, no distributed routing |
| Metrics | Native Prometheus | Prometheus + MongoDB persistence |
| Clients | 14 SDKs, private signaling protocol | Keeps the OpenVidu v2 client SDKs, adapted internally |
| API layer | LiveKit Twirp/REST API | OpenVidu REST API via the v2compatibility module |
| Recording | Egress as a separate service | Inherits Egress, keeps the OpenVidu recording API |

### 6.4 Key Conclusion

The LiveKit fork in openvidu-livekit is not a choice at the media routing level — it is a **borrowing of LiveKit's Telemetry and Analytics pipeline**. OpenVidu needs:

1. Session-level quality metrics (participant latency, packet loss rate, resolution)
2. Historical trend analysis (aggregated by time, by room, by participant)
3. CDR (Call Detail Record) for billing and audit

LiveKit's TelemetryService already collects this data; OpenVidu persists it through MongoDB to make it queryable. This is lighter and lower in operational cost than the Elastic Stack based scheme in OpenVidu v2.

## 7. Client SDK

### 7.1 openvidu-browser (v2)

The core client library of OpenVidu v2 is `openvidu-browser.js`, which wraps the browser WebRTC API:

```javascript
import { OpenVidu } from 'openvidu-browser';

// 1. Initialize
const OV = new OpenVidu();
const session = OV.initSession();

// 2. Connect to the Session
session.connect(token)
  .then(() => console.log('connected'))
  .catch(err => console.error(err));

// 3. Publish local media
const publisher = await OV.initPublisher('camera', {
  audioSource: undefined,  // default microphone
  videoSource: undefined,  // default camera
  publishAudio: true,
  publishVideo: true,
  resolution: '1280x720',
  frameRate: 30,
});
await session.publish(publisher);

// 4. Subscribe to remote media
session.on('streamCreated', event => {
  const subscriber = session.subscribe(event.stream, 'subscriber-div');
  subscriber.on('videoElementCreated', event => {
    document.getElementById('remote-video').appendChild(event.element);
  });
});
```

### 7.2 v3 Client Migration

OpenVidu v3 migrates the client SDK from its own `openvidu-browser.js` to the LiveKit client SDK:

| Capability | v2 (openvidu-browser) | v3 (LiveKit SDK) |
|------|----------------------|-------------------|
| Signaling | Proprietary WebSocket protocol | LiveKit binary WS protocol |
| Publish | `session.publish(publisher)` | `room.localParticipant.publishTrack()` |
| Subscribe | `session.subscribe(stream)` | `room.on('trackSubscribed', ...)` |
| Screen sharing | `OV.initPublisher('screen')` | `createLocalScreenTrack()` |
| Recording | Through the OpenVidu REST API | Through the OpenVidu REST API (compatible) |
| Roles | OpenVidu role model | Mapped onto LiveKit permissions |

### 7.3 Server-side SDKs

| Language | Package | Use |
|------|------|------|
| Java | `openvidu-java-client` | Backend application integration |
| Node.js | `openvidu-node-client` | Node.js backend integration |
| Python | `openvidu-python-client` | Python backend (community maintained) |
| Ruby | `openvidu-ruby-client` | Ruby backend (community maintained) |
| PHP | `openvidu-php-client` | PHP backend (community maintained) |
| Go | `openvidu-go-client` | Go backend (community maintained) |
| .NET | `openvidu-dotnet-client` | .NET backend (community maintained) |

The server-side SDKs only wrap REST API calls (creating Sessions, generating Tokens, managing recordings, etc.) and do not touch WebRTC media processing:

```java
// Java server-side SDK example
OpenVidu openvidu = new OpenVidu("https://openvidu.example.com", "MY_SECRET");

// Create a Session
Session session = openvidu.createSession();
String sessionId = session.getSessionId();

// Generate a Token
ConnectionProperties properties = new ConnectionProperties.Builder()
    .role(OpenViduRole.PUBLISHER)
    .data("{\"displayName\":\"Alice\"}")
    .build();
Connection connection = session.createConnection(properties);
String token = connection.getToken();

// Return the token to the client
```

## 8. Recording

### 8.1 Recording Modes

OpenVidu supports two recording output modes:

| Mode | Description | Output format | Applicable scenarios |
|------|------|---------|---------|
| COMPOSED | All participants are composited into a single video grid. GStreamer compositing pipeline | MP4 (H.264+AAC) | Meeting archival, playback |
| INDIVIDUAL | Each participant is recorded independently, a separate file | WEBM (VP8+Opus) | Post editing, single-stream analysis |

### 8.2 COMPOSED Recording Architecture

COMPOSED recording uses a GStreamer compositing pipeline:

```
┌────────────────────────────────────────────────────────────────┐
│  GStreamer compositing pipeline (COMPOSED recording)           │
│                                                                │
│  ┌────────┐  ┌────────┐  ┌────────┐                           │
│  │  Stream 1│  │  Stream 2│  │  Stream N│                       │
│  │  (VP8)  │  │  (VP8)  │  │  (VP8)  │                       │
│  └───┬────┘  └───┬────┘  └───┬────┘                           │
│      │           │           │                                │
│      ▼           ▼           ▼                                │
│  ┌──────────────────────────────────────────────────────┐    │
│  │  compositor (GStreamer compositor plugin)             │    │
│  │  · grid layout: 1x1, 2x2, 3x3, 4x4...                │    │
│  │  · automatically adjusts participant positions        │    │
│  └────────────────────────┬─────────────────────────────┘    │
│                           │                                   │
│                           ▼                                   │
│  ┌──────────────────────────────────────────────────────┐    │
│  │  encoder (H.264) + muxer (MP4)                       │    │
│  │  · video: H.264 encoded, configurable bitrate        │    │
│  │  · audio: AAC mixed, all participants' audio mixed    │    │
│  └──────────────────────────────────────────────────────┘    │
│                           │                                   │
│                           ▼                                   │
│                    ┌──────────────┐                           │
│                    │  output.mp4  │                           │
│                    └──────────────┘                           │
└────────────────────────────────────────────────────────────────┘
```

### 8.3 Recording API

```json
// POST /api/recordings/start
{
  "session": "ses_xxxxx",
  "name": "meeting-record-001",
  "outputMode": "COMPOSED",
  "hasAudio": true,
  "hasVideo": true,
  "resolution": "1920x1080",
  "frameRate": 25,
  "shmSize": 536870912
}

// response
{
  "id": "rec_xxxxx",
  "sessionId": "ses_xxxxx",
  "name": "meeting-record-001",
  "status": "started",
  "recordingMode": "COMPOSED",
  "resolution": "1920x1080",
  "frameRate": 25,
  "createdAt": 1720000000000
}
```

### 8.4 Recording Lifecycle

```
status: started → stopped → ready → (failed)
                        ↓
                    available
                        ↓
                (DELETE → deleted)
```

| State | Description |
|------|------|
| started | Recording has started, writing to file |
| stopped | Recording has stopped, file is being processed (compositing/transcoding) |
| ready | Recording file is ready, can be downloaded |
| failed | Recording failed (disk full/encoding error) |
| available | Recording file is available (v2/v3 compatible state) |
| deleted | Recording file has been deleted |

### 8.5 Recording File Management

Recording files are stored by default under the `OPENVIDU_RECORDING_PATH` path, with S3/Azure Blob Storage sync supported:

```bash
# recording file structure
/opt/openvidu/recordings/
├── rec_xxxxx/
│   ├── video.mp4           # COMPOSED recording
│   └── metadata.json       # recording metadata
├── rec_yyyyy/
│   ├── participant_1.webm  # INDIVIDUAL recording
│   ├── participant_2.webm
│   └── metadata.json
```

## 9. Operator (Elastic Cluster)

### 9.1 Media Node Lifecycle

The elastic cluster of OpenVidu Pro manages the full lifecycle of media nodes:

```
         ┌───────────┐
         │ launching │  (Docker container starting up)
         └─────┬─────┘
               │
         ┌─────┴─────┐
         │  running  │  (operating normally, accepting media sessions)
         └─────┬─────┘
               │
         ┌─────┴─────┐
         │  draining │  (no new sessions, waiting for existing ones to end)
         └─────┬─────┘
               │
         ┌─────┴──────┐
         │ terminated │  (container destroyed, resources released)
         └────────────┘
```

### 9.2 CPU-Driven Auto Scaling

Media node scaling in OpenVidu Pro is based on CPU load:

```
                          CPU > 80%
                 ┌──────────────────────┐
                 │                      ▼
          ┌──────────┐            ┌──────────┐
          │  Node A  │            │  Node B  │  (newly started)
          │  65% CPU │            │  0% CPU  │
          └──────────┘            └──────────┘
                 │                      │
                 │                CPU < 60%
                 │                      │
                 │                      ▼
                 │              ┌──────────┐
                 │              │  Node B  │
                 │              │  drain   │  (starts draining)
                 │              └──────────┘
                 │                      │
                 │                      │ all sessions ended
                 │                      ▼
                 │              ┌──────────┐
                 │              │  Node B  │
                 │              │  term.   │  (destroyed)
                 │              └──────────┘
```

| Parameter | Default | Description |
|------|--------|------|
| `OPENVIDU_MEDIA_NODE_CPU_USE_THRESHOLD` | 80 | CPU above this value triggers scale-out (%) |
| `OPENVIDU_MEDIA_NODE_CPU_RESUME_THRESHOLD` | 60 | CPU below this value triggers scale-in (%) |
| Scale detection interval | 15 seconds | Periodically checks node load |
| Minimum node count | 1 | Guarantees at least one node is running |
| Maximum node count | 10 | Prevents unbounded scale-out |

### 9.3 Media Node Registration

Media nodes register with openvidu-server automatically on startup:

```json
{
  "id": "node_xxxxx",
  "ip": "10.0.1.100",
  "uri": "ws://10.0.1.100:8888/kurento",
  "status": "running",
  "cpuUsage": 45.2,
  "memoryUsage": 60.1,
  "sessionCount": 12,
  "connectionCount": 48,
  "createdAt": 1720000000000
}
```

## 10. Evolution History

### 10.1 Three Generations of Architecture

```
OpenVidu v1 (2016)       OpenVidu v2 (2018)      OpenVidu v3 (2024-2025)
─────────────────────    ─────────────────────    ────────────────────────
Kurento (MCU+SFU)        Kurento (SFU-focused)    LiveKit + mediasoup
├── Kurento CE            ├── Kurento CE           ├── LiveKit (CE)
├── no cluster            ├── Elastic Stack        ├── MongoDB Analytics
├── no recording          ├── Docker Compose       ├── Prometheus + Grafana
└── no Dashboard          ├── Pro/Enterprise       ├── E2EE
                          └── elastic cluster      └── v2compatibility
```

### 10.2 Evolution Drivers

**v1 → v2 (Kurento CE to Pro/Enterprise)**:
- Market demand: production deployments needed elastic scaling and monitoring
- Business model: CE as free acquisition, Pro/Enterprise delivering commercial value
- Technical improvement: moving from MCU to pure SFU to raise performance

**v2 → v3 (Kurento to LiveKit + mediasoup)**:
- **Performance bottleneck**: Kurento's GStreamer pipeline has a high CPU cost; in pure SFU scenarios mediasoup can reach 2x the capacity
- **Maintenance cost**: Kurento entered maintenance mode and the community focus shifted
- **The appeal of LiveKit**: single Go binary deployment + built-in signaling + a high-quality Telemetry pipeline
- **Enterprise demand**: mid-to-large customers needed a higher-performance SFU engine

### 10.3 Version Mapping

| OpenVidu version | Underlying engine | Deployment | Support status |
|--------------|---------|---------|---------|
| v2.29.x (CE) | Kurento | Docker Compose | Maintenance mode, security updates |
| v2.29.x (Pro) | Kurento | Docker Compose + elastic cluster | Maintenance mode |
| v2.29.x (Enterprise) | Kurento | Docker Compose + HA | Maintenance mode |
| v3.x (CE) | LiveKit | Docker Compose | Active development |
| v3.x (Enterprise) | mediasoup | Docker Compose + K8s | Active development |

### 10.4 Performance Comparison

| Metric | v2 (Kurento) | v3 CE (LiveKit) | v3 Enterprise (mediasoup) |
|------|-------------|----------------|--------------------------|
| Streams per node | ~100 | ~500 | ~1000+ |
| Connection latency | ~1s | ~0.5s | ~0.25s |
| CPU efficiency | Low (GStreamer decode+encode) | Medium (Go SFU) | High (C++ SFU) |
| Deployment image | ~1.5GB (Kurento + GStreamer) | ~50MB (Go binary) | ~200MB (mediasoup + deps) |
| Startup time | ~30s | ~3s | ~5s |

## 11. E2E Testing

### 11.1 openvidu-loadtest

OpenVidu provides the `openvidu-loadtest` tool for stress testing and performance benchmarking:

```bash
# install
git clone https://github.com/OpenVidu/openvidu-loadtest.git
cd openvidu-loadtest

# run a stress test
BROWSER_USER=1 \           # number of simulated browser users
BROWSER_SESSIONS=1 \       # number of concurrent Sessions
DURATION=60000 \            # test duration (ms)
OPENVIDU_URL=https://openvidu.example.com \
OPENVIDU_SECRET=MY_SECRET \
npm start
```

### 11.2 Test Metrics

| Metric | Collection method | Use |
|------|---------|------|
| Connection success rate | Each participant's connection state | Network/service availability |
| Connection latency | From Token to ICE connection completion | User experience |
| Publish latency | From publish() to the first RTP packet | Media pipeline latency |
| Subscribe latency | From subscribe() to the first video frame | Subscribe pipeline latency |
| Packet loss rate | RTCP RR statistics | Network quality |
| CPU usage | Node monitoring | Scaling decisions |
| Memory usage | Node monitoring | Capacity planning |

### 11.3 Test Topology

```
┌────────────────────────────────────────────────────────────────┐
│  openvidu-loadtest architecture                                 │
│                                                                │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│  │  Browser 1   │  │  Browser 2   │  │  Browser N   │         │
│  │  (Puppeteer) │  │  (Puppeteer) │  │  (Puppeteer) │         │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘         │
│         │                 │                 │                  │
│         └─────────────────┼─────────────────┘                  │
│                           │                                    │
│                           ▼                                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  openvidu-loadtest (Node.js coordinator)                  │  │
│  │  · create/destroy Sessions                                │  │
│  │  · generate Tokens                                        │  │
│  │  · orchestrate the participant lifecycle                  │  │
│  │  · collect metrics                                        │  │
│  └──────────────────────────────────────────────────────────┘  │
│                           │                                    │
│         ┌─────────────────┼─────────────────┐                  │
│         │                 │                 │                  │
│         ▼                 ▼                 ▼                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│  │ openvidu     │  │ Media Node 1│  │ Media Node 2│         │
│  │ -server      │  │ (Kurento/   │  │ (Kurento/   │         │
│  │              │  │  LiveKit)   │  │  LiveKit)   │         │
│  └──────────────┘  └──────────────┘  └──────────────┘         │
└────────────────────────────────────────────────────────────────┘
```

## 12. Reference Value for MediaServo

### [Adopt] Directly Adoptable

1. **Master-Worker architecture**: the Java control plane + Kurento/LiveKit media plane separation of openvidu-server. MediaServo's mediaservo-server (control plane) + media node (media plane) architecture design can directly reference this pattern

2. **Session/Connection/Token three-layer abstraction**: the Session (room) → Connection (participant) → Token (authentication) model is simple and practical. MediaServo's signaling layer can directly reuse this layering

3. **PUBLISHER/SUBSCRIBER role model**: a concise two-role permission model — publishers have full permissions, subscribers can only watch. MediaServo's remote desktop and surveillance scenarios fit this model naturally (Host=PUBLISHER, Client=SUBSCRIBER)

4. **COMPOSED/INDIVIDUAL recording modes**: the two recording modes cover the vast majority of scenarios — COMPOSED for meeting playback, INDIVIDUAL for post editing. MediaServo's recording module should support both modes directly

5. **CDR (Call Detail Record)**: call detail records are foundational infrastructure for operations and billing. MediaServo's Telemetry module should have CDR collection built in

6. **API key authentication (OPENVIDU_SECRET)**: a simple but effective server-side authentication scheme. MediaServo's Server API authentication can directly reference this pattern

### [Adapt] Needs Modification Before Adoption

1. **Token generation flow**: OpenVidu's Token contains the role (PUBLISHER/SUBSCRIBER) and custom data. MediaServo should extend this to richer access control — track-level permissions (a given user can subscribe to only one particular video stream), time constraints, IP whitelists

2. **Elastic cluster scaling**: CPU-driven scaling is a baseline strategy. MediaServo should add more dimensions — memory usage, connection count, network bandwidth, custom metrics. It should also support prediction-based proactive scale-out

3. **Recording manager**: OpenVidu's recording manager is part of the server. MediaServo should design it as an independent recording service (similar to LiveKit Egress) so recording CPU load does not affect signaling responsiveness

4. **Media node lifecycle**: the launching → running → draining → terminated four-phase model is good. MediaServo should add health checks and an auto-recovery mechanism — automatically migrating sessions when a node fails

5. **v2compatibility strategy**: the v2compatibility module in OpenVidu v3 is a reference template for vendor migration. MediaServo should also maintain backward compatibility for at least one major version when evolving its API

### [Avoid] Known Pitfalls / Non-Applicable

1. **Kurento's CPU-intensive MCU mode**: the Kurento scheme in OpenVidu v2 has low CPU efficiency and a deployment image of ~1.5GB. MediaServo should avoid the GStreamer-tightly-bound MCU mode — pure SFU is the more efficient choice

2. **High resource consumption of the Java control plane**: openvidu-server (Java Spring Boot) needs ~512MB-1GB of memory and takes ~30s to start. MediaServo's control plane should be implemented in Rust (the mediaservo-server skeleton already exists) — zero JVM overhead, millisecond-level startup

3. **LiveKit's private signaling protocol**: OpenVidu v3 uses LiveKit's private binary WebSocket signaling, which means standard WebRTC clients cannot connect directly. MediaServo should use the standard SDP offer/answer model to preserve interoperability

4. **Single-point openvidu-server**: openvidu-server is a single point — when it fails, every session is affected. MediaServo's Server design should natively support stateless horizontal scaling — multiple Server instances sharing session state

5. **Recording coupled to media nodes**: OpenVidu performs recording on media nodes, and recording CPU load degrades media node performance. MediaServo should design an independent recording service that receives streams via RTP Forwarding

6. **Over-reliance on Docker Compose**: OpenVidu's deployment is almost entirely bound to Docker Compose. MediaServo should support multiple deployment methods — native binary, Docker, K8s, systemd service

### Core Summary

| Dimension | OpenVidu experience | MediaServo application |
|------|-------------|-------------|
| Architecture | Master-Worker control plane / media plane separation | mediaservo-server + media node |
| Abstraction | Session/Connection/Token three layers | Reuse directly in the signaling layer |
| Roles | PUBLISHER/SUBSCRIBER | Host=publisher, Client=subscriber |
| Recording | COMPOSED/INDIVIDUAL dual mode | Support directly in the recording module |
| Cluster | CPU-driven scaling | Add multi-dimensional + predictive scheduling |
| Migration | v2compatibility module | API backward compatibility strategy |
| Monitoring | CDR + Prometheus + MongoDB | Telemetry with built-in CDR collection |

**Overall rating**: ★★★★☆ (4/5)

> **Comment**: The value of OpenVidu does not lie in its media engine capabilities (Kurento is aging, LiveKit is third-party, and mediasoup is third-party too) — it lies in its **Master-Worker platform architecture** and its **Session/Connection/Token abstraction model**. OpenVidu proved the commercial and technical feasibility of "wrapping a complete platform API on top of a media engine". MediaServo's mediaservo-server should borrow OpenVidu's platform architecture design, but use mediasoup directly as the media engine (rather than Kurento or a LiveKit fork), implement the control plane in Rust (rather than Java Spring Boot), and use standard SDP/JSEP for signaling (rather than a private protocol). OpenVidu's platformization experience + mediasoup's performance + Rust's zero-cost abstractions = the best path for MediaServo Server.

---

> **References**
> GitHub: OpenVidu/openvidu (Apache 2.0)
> GitHub: OpenVidu/openvidu-loadtest (stress testing framework)
> Official documentation: docs.openvidu.io
> OpenVidu v3 migration guide: docs.openvidu.io/en/stable/openvidu-v3/
> OpenVidu Pro architecture: docs.openvidu.io/en/stable/openvidu-pro/
> Kurento/openvidu/kurento-media-server
> LiveKit: livekit/livekit (forked as openvidu-livekit)
> MediaServo: docs/research/video-conference.md

---

**Related decisions**: D97 (SFU/MCU hybrid), D138 (mediasoup selection), D-KURENTO-EVOLUTION

