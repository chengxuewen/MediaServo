# Conference SDK API Reference

## 1. Overview

The Conference SDK provides a complete API for creating and managing video conferences. It is exposed to TypeScript/JavaScript via napi-rs bindings and also offers a native Rust crate interface.

### Installation

```bash
# npm (through the host platform)
npm install @mediaservo/conference-sdk

# Rust (through third-party platforms)
cargo add mediaservo-conference
```

### Initialization

```typescript
import { ConferenceClient } from "@mediaservo/conference-sdk";

const client = new ConferenceClient({
  endpoint: "wss://media.example.com",
  token: "jwt-token-here",
});
```

## 2. ConferenceClient

### 2.1 Room Management

```typescript
// Create a room
const room = await client.rooms.create({
  name: "Team Standup",
  maxParticipants: 16,
  mode: "conference",
  videoCodec: "vp9",
  simulcast: true,
  svc: true,
});
// → Room { id, name, status, createdAt, ... }

// Join a room
const joinResult = await client.rooms.join(room.id);
// → JoinResult { room, routerRtpCapabilities }

// Leave a room
await client.rooms.leave(room.id);

// End a room
await client.rooms.end(room.id);

// Get room info
const roomInfo = await client.rooms.get(room.id);

// List rooms
const rooms = await client.rooms.list({ status: "active" });
```

### 2.2 Media Publishing

```typescript
// Create a send transport
const sendTransport = await client.transports.create({
  roomId: room.id,
  direction: "send",
  iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
});

// Publish video
const videoProducer = await sendTransport.produce({
  track: localVideoTrack,     // MediaStreamTrack
  encodings: [
    { rid: "r0", maxBitrate: 150_000, scaleResolutionDownBy: 4 },
    { rid: "r1", maxBitrate: 400_000, scaleResolutionDownBy: 2 },
    { rid: "r2", maxBitrate: 1_200_000, scaleResolutionDownBy: 1 },
  ],
  codecOptions: { videoGoogleStartBitrate: 1000 },
});
// → Producer { id, kind, stats }

// Publish audio
const audioProducer = await sendTransport.produce({
  track: localAudioTrack,
});

// Pause/resume publishing
await videoProducer.pause();
await videoProducer.resume();

// Close the publication
await videoProducer.close();
```

### 2.3 Media Subscription

```typescript
// Create a recv transport
const recvTransport = await client.transports.create({
  roomId: room.id,
  direction: "recv",
});

// Subscribe to specific audio/video
const consumer = await recvTransport.consume({
  producerId: videoProducer.id,  // A remote participant's producer ID
  rtpCapabilities: routerRtpCapabilities,
});
// → Consumer { id, track, kind, type (simulcast/svc), producerPaused }

// Attach the remote track to a <video> element
videoElement.srcObject = new MediaStream([consumer.track]);

// Switch the encoding layer (Simulcast)
await consumer.setPreferredLayers({ rid: "r1" });

// Switch the encoding layer (SVC)
await consumer.setPreferredLayers({
  spatialLayer: 1,
  temporalLayer: 2,
});

// Pause/resume the subscription
await consumer.pause();
await consumer.resume();

// Close the subscription
await consumer.close();
```

### 2.4 Participant Management

```typescript
// Get the participant list
const participants = await client.participants.list(room.id);

// Mute a participant (Host only)
await client.participants.mute({ roomId: room.id, participantId: "id", kind: "audio" });

// Unmute
await client.participants.unmute({ roomId: room.id, participantId: "id", kind: "audio" });

// Remove a participant
await client.participants.kick({ roomId: room.id, participantId: "id" });

// Set a role
await client.participants.setRole({
  roomId: room.id,
  participantId: "id",
  role: "speaker",
});
```

### 2.5 Recording

```typescript
// Start recording
const recording = await client.recording.start({
  roomId: room.id,
  mode: "individual",  // "individual" | "composite"
  layout: "grid",      // composite layout
});
// → RecordingStatus { id, startedAt, format }

// Stop recording
await client.recording.stop(room.id);

// Query status
const status = await client.recording.status(room.id);
```

## 3. Event Listening

```typescript
// Participant joined
client.on("participant.joined", (event) => {
  console.log(`${event.participant.displayName} joined`);
});

// Participant left
client.on("participant.left", (event) => {
  console.log(`${event.participantId} left: ${event.reason}`);
});

// New track added (auto-subscribe)
client.on("track.added", async (event) => {
  const consumer = await recvTransport.consume({
    producerId: event.track.producerId,
    rtpCapabilities: routerRtpCapabilities,
  });
  // Attach to the UI
});

// Track removed
client.on("track.removed", (event) => {
  // Clean up the UI
});

// Dominant speaker changed
client.on("dominant.speaker.changed", (event) => {
  // Highlight the current speaker
});

// Connection state
client.on("connection.state", (event) => {
  console.log(`State: ${event.state}`);
});

// Errors
client.on("error", (event) => {
  console.error(`Error: ${event.code} - ${event.message}`);
});
```

## 4. Rust API

```rust
// Crate: mediaservo-conference
// Exposed to Node.js via napi-rs bindings, plus a native Rust API

use mediaservo_conference::{ConferenceClient, ConferenceConfig, Room};

#[tokio::main]
async fn main() -> Result<()> {
    let client = ConferenceClient::new(ConferenceConfig {
        endpoint: "wss://media.example.com".into(),
        token: std::env::var("MEDIASERVO_TOKEN")?,
    });

    let room = client.create_room(CreateRoomParams {
        name: "Team Standup".into(),
        max_participants: 16,
        mode: RoomMode::Conference,
        video_codec: VideoCodec::Vp9,
        simulcast: true,
        svc: true,
    }).await?;

    println!("Room created: {}", room.id);
    Ok(())
}
```

## 5. Configuration

```typescript
interface ConferenceConfig {
  // Required
  endpoint: string;           // Signaling endpoint
  token: string;              // Auth token

  // Optional
  iceServers?: RTCIceServer[];   // ICE servers
  videoCodec?: string;        // Default video codec
  audioCodec?: string;        // Default audio codec
  simulcast?: boolean;        // Simulcast
  svc?: boolean;              // SVC
  maxBitrate?: number;        // Max bitrate (kbps)
  maxResolution?: string;     // Max resolution

  // Debugging
  logLevel?: "error" | "warn" | "info" | "debug";
  statsInterval?: number;     // Stats reporting interval (ms)
}
```

## 6. Error Handling

```typescript
import { ConferenceError } from "@mediaservo/conference-sdk";

try {
  await client.rooms.join("non-existent-room");
} catch (error) {
  if (error instanceof ConferenceError) {
    switch (error.code) {
      case "ROOM_NOT_FOUND":
        // Room does not exist
        break;
      case "ROOM_FULL":
        // Room is full
        break;
      case "FORBIDDEN":
        // No permission
        break;
      default:
        // Other errors
    }
  }
}
```
