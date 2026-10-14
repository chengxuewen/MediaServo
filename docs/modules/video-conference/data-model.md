# Data Model

## 1. Room

```typescript
interface Room {
  id: string;                    // UUID
  name: string;                  // Room name
  createdAt: string;             // ISO 8601
  status: RoomStatus;
  maxParticipants: number;       // Max participants
  recording: boolean;            // Whether recording
  mode: "conference" | "webinar" | "p2p";
  region: string;                // Deployment region

  // Media config
  mediaConfig: {
    videoCodec: "vp8" | "vp9" | "h264" | "av1";
    audioCodec: "opus";
    simulcast: boolean;
    svc: boolean;
    maxBitrate: number;          // kbps
  };

  // Security
  security: {
    e2ee: boolean;
    requireAuth: boolean;
    allowedDomains: string[];
  };
}

enum RoomStatus {
  "idle",
  "active",
  "ended",
}
```

## 2. Participant

```typescript
interface Participant {
  id: string;                    // UUID
  roomId: string;
  userId: string;                // Linked to the user system
  displayName: string;
  role: "host" | "speaker" | "listener";
  joinedAt: string;              // ISO 8601

  // Connection info
  connection: {
    transportId: string;         // mediasoup Transport ID
    iceState: "connected" | "disconnected" | "failed";
    connectedAt: string;
    iceCandidateType: "host" | "srflx" | "relay";
    rtt: number;                 // ms
  };

  // Media state
  media: {
    video: MediaState;
    audio: MediaState;
    screenShare: MediaState;
  };

  // Client info
  client: {
    platform: "web" | "desktop" | "mobile";
    browser?: string;
    version: string;
  };
}

interface MediaState {
  enabled: boolean;
  muted: boolean;
  codec?: string;
  bitrate?: number;
  resolution?: { width: number; height: number };
  frameRate?: number;
}
```

## 3. Track

```typescript
type TrackKind = "audio" | "video" | "screen";

interface Track {
  id: string;
  kind: TrackKind;
  participantId: string;
  roomId: string;

  // mediasoup mapping
  producerId: string;            // mediasoup Producer ID
  routerId: string;              // mediasoup Router ID

  // Encoding info
  encoding: {
    codec: string;
    mimeType: string;            // e.g. "video/VP8"
    clockRate: number;
    channels?: number;           // audio only
  };

  // Simulcast / SVC layers
  layers?: SimulcastLayer[];
  svc?: {
    spatialLayers: number;
    temporalLayers: number;
  };

  // Stats
  stats: {
    bitrate: number;             // kbps
    packetLoss: number;          // %
    jitter: number;              // ms
    rtt: number;                 // ms
  };
}

interface SimulcastLayer {
  id: string;
  rid: "r0" | "r1" | "r2";      // Encoding layer ID
  resolution: { width: number; height: number };
  bitrate: number;               // kbps
  active: boolean;               // Whether it has subscribers
}
```

## 4. Encoding Layer Reference

### 4.1 Simulcast Layers

| Layer | RID | Resolution | Bitrate (VP8) | Bitrate (H.264) |
|-------|-----|------------|---------------|-----------------|
| Low | r0 | 320×180 | 150 kbps | 120 kbps |
| Mid | r1 | 640×360 | 400 kbps | 350 kbps |
| High | r2 | 1280×720 | 1200 kbps | 1000 kbps |

### 4.2 VP9 SVC Layers

| Spatial | Temporal | Resolution | Frame rate |
|---------|----------|------------|------------|
| S0 | T0 | 320×180 | 7.5 fps |
| S0 | T1 | 320×180 | 15 fps |
| S0 | T2 | 320×180 | 30 fps |
| S1 | T0 | 640×360 | 7.5 fps |
| S1 | T1 | 640×360 | 15 fps |
| S1 | T2 | 640×360 | 30 fps |
| S2 | T0 | 1280×720 | 7.5 fps |
| S2 | T1 | 1280×720 | 15 fps |
| S2 | T2 | 1280×720 | 30 fps |

### 4.3 Audio

| Parameter | Value |
|-----------|-------|
| Codec | Opus |
| Sample rate | 48 kHz |
| Channels | 2 (stereo) |
| Bitrate | 30 kbps |
| Frame length | 20 ms |
| FEC | Enabled |
| DTX | Enabled |

## 5. Permission Model

```typescript
interface ConferencePermission {
  // Features
  canPublishVideo: boolean;
  canPublishAudio: boolean;
  canShareScreen: boolean;
  canRecord: boolean;
  canMuteOthers: boolean;
  canKick: boolean;
  canLock: boolean;

  // Quotas
  maxPublishBitrate: number;     // kbps
  maxResolution: string;         // "720p" | "1080p" | "4k"
  maxParticipants: number;

  // Recording
  recording: "none" | "own" | "all";
}
```

## 6. Event Model

```typescript
type ConferenceEvent =
  | { type: "room.created"; room: Room }
  | { type: "room.ended"; roomId: string }
  | { type: "participant.joined"; participant: Participant }
  | { type: "participant.left"; participantId: string; reason: string }
  | { type: "participant.muted"; participantId: string; kind: TrackKind }
  | { type: "participant.unmuted"; participantId: string; kind: TrackKind }
  | { type: "track.added"; track: Track }
  | { type: "track.removed"; trackId: string }
  | { type: "track.layer.changed"; trackId: string; layer: string }
  | { type: "dominant.speaker.changed"; participantId: string }
  | { type: "recording.started"; roomId: string }
  | { type: "recording.stopped"; roomId: string }
  | { type: "connection.state"; participantId: string; state: string }
  | { type: "error"; code: string; message: string };
```

## 7. State Machines

### Room Lifecycle

```
Created ──→ Active ──→ Ended
                │
                ├──→ Recording (sub-state)
                │
                └──→ Locked (sub-state, no new joins)
```

### Participant Connection State

```
Invited ──→ Joining ──→ Connected ──→ Disconnected
                │            │
                └──→ Failed  └──→ Reconnecting ──→ Connected
                                       │
                                       └──→ Disconnected (timeout)
```

### Track Lifecycle

```
Added ──→ Active ──→ Removed
            │
            ├──→ Muted (forwarding paused)
            │
            └──→ LayerChanged (encoding layer switch)
```
