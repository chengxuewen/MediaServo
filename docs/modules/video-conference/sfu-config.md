# SFU Configuration & Tuning

## 1. mediasoup Worker Configuration

```javascript
// worker.js — mediasoup Worker configuration
const worker = await mediasoup.createWorker({
  logLevel: "warn",
  logTags: [
    "info",
    "ice",
    "dtls",
    "rtp",
    "srtp",
    "rtcp",
  ],
  rtcMinPort: 40000,
  rtcMaxPort: 49999,
  dtlsCertificateFile: "/etc/mediaservo/certs/dtls.pem",
  dtlsPrivateKeyFile: "/etc/mediaservo/certs/dtls-key.pem",
});
```

### Worker Allocation Policy

```
auto policy (default):
  workers = ceil((nproc * 0.8) + (max(0, nproc - 32) / 2))
  e.g. 8 cores →  ceil(6.4 + 0) = 7 Workers
  e.g. 64 cores → ceil(51.2 + 16) = 68 Workers

Allocation by media type (large deployments):
  ┌──────────────┬─────────┬──────────┐
  │ Worker Pool  │ Workers │ Purpose  │
  ├──────────────┼─────────┼──────────┤
  │ audio        │ 2       │ Audio only │
  │ video        │ nproc-4 │ Video     │
  │ screen       │ 2       │ Screen share │
  └──────────────┴─────────┴──────────┘
```

## 2. Router Configuration

```javascript
const router = await worker.createRouter({
  mediaCodecs: [
    {
      kind: "audio",
      mimeType: "audio/opus",
      clockRate: 48000,
      channels: 2,
      parameters: {
        useinbandfec: 1,
        usedtx: 1,
      },
    },
    {
      kind: "video",
      mimeType: "video/VP8",
      clockRate: 90000,
      parameters: {
        "x-google-start-bitrate": 1000,
      },
    },
    {
      kind: "video",
      mimeType: "video/VP9",
      clockRate: 90000,
      parameters: {
        "profile-id": 2,
      },
    },
    {
      kind: "video",
      mimeType: "video/H264",
      clockRate: 90000,
      parameters: {
        "packetization-mode": 1,
        "profile-level-id": "42e01f",
        "level-asymmetry-allowed": 1,
      },
    },
  ],
});
```

## 3. Transport Configuration

```javascript
const transport = await router.createWebRtcTransport({
  listenIps: [
    { ip: "0.0.0.0", announcedIp: "public-ip" },  // Public IP
  ],
  enableUdp: true,
  enableTcp: true,
  preferUdp: true,
  initialAvailableOutgoingBitrate: 1_000_000,       // 1 Mbps
  minimumAvailableOutgoingBitrate: 100_000,          // 100 Kbps
  maxSctpMessageSize: 262_144,                       // 256 KB
  maxIncomingBitrate: 5_000_000,                     // 5 Mbps limit
});
```

## 4. Producer/Consumer Configuration

### Producer (publisher)

```javascript
const producer = await transport.produce({
  kind: "video",
  rtpParameters: {
    mid: "0",
    codecs: [...],
    headerExtensions: [...],
    encodings: [
      { rid: "r0", maxBitrate: 150_000, scaleResolutionDownBy: 4 },
      { rid: "r1", maxBitrate: 400_000, scaleResolutionDownBy: 2 },
      { rid: "r2", maxBitrate: 1_200_000, scaleResolutionDownBy: 1 },
    ],
    rtcp: { reducedSize: true },
  },
  appData: { participantId: "xxx" },
});
```

### Consumer (subscriber)

```javascript
const consumer = await transport.consume({
  producerId: "producer-id",
  rtpCapabilities: clientRtpCapabilities,
  paused: false,
  preferredLayers: {
    spatialLayer: 2,    // SVC spatial layer
    temporalLayer: 2,   // SVC temporal layer
  },
  appData: { participantId: "yyy" },
});
```

## 5. Bandwidth Management

### 5.1 Dynacast (encode on demand)

Dynacast monitors each Producer's subscriptions. When an encoding layer has no subscribers, the publisher is told to stop sending that layer, saving uplink bandwidth.

```javascript
// Enable on the transport
transport.enableDynacast();

// Automatically triggered when subscription layers switch
await consumer.setPreferredLayers({ rid: "r1" });
// Dynacast sees r0 and r2 have no subscribers → tells the publisher to stop
```

### 5.2 PLI/FIR Aggregation

Multiple Consumers requesting keyframes simultaneously causes encoder bursts. PLI/FIR aggregation merges multiple requests into one.

```javascript
// Default behavior (built into mediasoup)
// Aggregation window: 500ms
// Multiple requests within the window → 1 upstream PLI/FIR

// Configuration
const consumer = await transport.consume({
  producerId: "...",
  rtpCapabilities: "...",
  // PLI/FIR aggregation enabled (default on)
});
```

### 5.3 Bandwidth Estimation

```
Sender-side BWE:
  transport-cc feedback → per-packet RTT → bandwidth estimate

Receiver-side BWE:
  TWCC (Transport Wide Congestion Control) feedback → REMB

MediaServo policy:
  TWCC preferred (more accurate)
  REMB as fallback
  delay-based algorithm for latency-sensitive networks
  loss-based algorithm for loss-sensitive networks (cross-carrier)
```

### 5.4 Bitrate Table

| Scenario | Resolution | Frame rate | Reference bitrate | Encoding layer |
|----------|------------|------------|-------------------|----------------|
| Multi-party thumbnail | 320×180 | 15fps | 150 Kbps | Simulcast r0 |
| Single mid-size | 640×360 | 24fps | 400 Kbps | Simulcast r1 |
| Full-screen video | 1280×720 | 30fps | 1200 Kbps | Simulcast r2 |
| Screen share | 1920×1080 | 15fps | 2000 Kbps | Simulcast r2 |
| Audio | - | - | 30 Kbps | Opus |

## 6. NAT Traversal

### ICE Configuration

```javascript
const transport = await router.createWebRtcTransport({
  listenIps: [
    { ip: "0.0.0.0", announcedIp: "203.0.113.1" },  // Public IP
  ],
  enableUdp: true,
  enableTcp: true,
});
```

### TURN Integration

When P2P hole punching fails, use a TURN relay:

```typescript
const client = new ConferenceClient({
  iceServers: [
    { urls: "stun:stun.l.google.com:19302" },
    {
      urls: "turn:turn.mediaservo.io:3478",
      username: "user",
      credential: "password",
    },
  ],
});
```

## 7. Performance Tuning

### 7.1 System Parameters

```bash
# /etc/sysctl.conf — network tuning
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.ipv4.udp_mem = 4096 87380 16777216
net.ipv4.tcp_congestion_control = bbr
net.core.netdev_max_backlog = 5000

# UDP buffers
net.ipv4.udp_rmem_min = 65536
net.ipv4.udp_wmem_min = 65536

# Connection tracking
net.netfilter.nf_conntrack_max = 1048576
```

### 7.2 ulimit

```bash
# /etc/security/limits.conf
mediaservo  hard  nofile  1048576
mediaservo  soft  nofile  1048576
```

### 7.3 Worker Tuning

| Parameter | Recommended | Notes |
|-----------|-------------|-------|
| Worker per CPU | 1 | Each Worker binds one CPU core |
| Room per Worker | <500 | Max rooms per Worker |
| Participant per Worker | <2000 | Max participants per Worker |
| Consumer per Participant | <50 | Max streams one participant subscribes |
| Transport per Worker | <2000 | Max transports per Worker |
| Dedicated Worker for large rooms | - | Rooms >16 people get a dedicated Worker |

### 7.4 Memory Estimation

```
per Participant:
  Transport: ~100 KB
  Audio Producer: ~200 KB
  Video Producer (Simulcast): ~500 KB
  Consumer (per stream): ~300 KB

Example: a 16-person room (each subscribing to 15 videos + 15 audios):
  per participant: 100KB + 200KB + 500KB + 30 × 300KB = 9.8 MB
  16 people: ~157 MB + Worker base overhead (~50 MB) = ~207 MB
  100 rooms of 16 people: ~20.7 GB
```

## 8. Troubleshooting

### 8.1 Common Issues

| Symptom | Possible cause | Fix |
|---------|----------------|-----|
| Connection fails | ICE/firewall ports | Check whether UDP 40000-49999 is open |
| Video stuttering | Insufficient bandwidth | Lower bitrate / switch codec |
| High latency | NAT/TURN relay | Check the relay path, optimize ICE candidates |
| Audio drops | Insufficient FEC | Enable Opus inband FEC |
| Memory leak | Workers not reclaimed | Check Worker lifecycle management |
| SFU crash | Worker overload | Add Workers, cap per-Worker load |

### 8.2 Diagnostic Commands

```bash
# Check Worker status
curl http://localhost:9090/metrics | grep conference_

# Check WebRTC ports
lsof -i :40000-49999

# Check UDP buffer usage
ss -uap | wc -l

# Live logs
journalctl -u mediaservo-conference -f

# Memory usage
ps aux | grep mediasoup-worker | awk '{sum+=$6} END {print sum/1024 " MB"}'
```
