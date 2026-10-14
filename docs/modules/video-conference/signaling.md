# Video Conference Signaling Protocol

## 1. Overview

The signaling protocol uses WebSocket as the real-time transport layer and gRPC for inter-service communication.

### Design Principles

- **Independent of the media layer**: signaling doesn't depend on a specific SFU implementation; the SFU engine is swappable
- **JSON payloads**: JSON first on the web side; Protobuf optional on the native side
- **Request-response + event push**: unified message format

## 2. Transport Layer

### 2.1 WebSocket Endpoints

```
ws://<host>/ws/conference/v1
wss://<host>/ws/conference/v1  (TLS)
```

Connections carry token auth:

```
GET /ws/conference/v1 HTTP/1.1
Upgrade: websocket
Sec-WebSocket-Protocol: conference-v1
Authorization: Bearer <token>
```

### 2.2 Message Format

```typescript
// Request
interface WsRequest {
  id: string;                    // Request ID, for correlating responses
  method: string;                // Method name, e.g. "room.create"
  params: Record<string, any>;   // Parameters
}

// Response
interface WsResponse {
  id: string;                    // Matching request ID
  result?: Record<string, any>;  // Success response
  error?: {                      // Error response
    code: string;
    message: string;
  };
}

// Event push
interface WsEvent {
  type: string;                  // Event type
  data: Record<string, any>;     // Event data
  timestamp: string;             // ISO 8601
}
```

## 3. Signaling Methods

### 3.1 Room Management

| Method | Direction | Description |
|--------|-----------|-------------|
| `room.create` | Client → Server | Create a room |
| `room.join` | Client → Server | Join a room |
| `room.leave` | Client → Server | Leave a room |
| `room.get` | Client → Server | Get room info |
| `room.list` | Client → Server | List rooms |
| `room.end` | Host → Server | End a room |
| `room.lock` | Host → Server | Lock a room |
| `room.unlock` | Host → Server | Unlock a room |

### 3.2 Media Negotiation

| Method | Direction | Description |
|--------|-----------|-------------|
| `transport.create` | Client → Server | Create a WebRTC transport |
| `transport.connect` | Client → Server | Connect a transport (ICE) |
| `produce` | Client → Server | Publish media |
| `consume` | Client → Server | Subscribe to media |
| `producer.pause` | Client → Server | Pause publishing |
| `producer.resume` | Client → Server | Resume publishing |
| `consumer.setLayer` | Client → Server | Switch the encoding layer |
| `consumer.setPreferredLayers` | Client → Server | Set preferred SVC layers |

### 3.3 Participant Management

| Method | Direction | Description |
|--------|-----------|-------------|
| `participant.mute` | Host → Server | Mute a participant |
| `participant.unmute` | Host → Server | Unmute a participant |
| `participant.kick` | Host → Server | Remove a participant |
| `participant.setRole` | Host → Server | Set a role |

### 3.4 Recording Management

| Method | Direction | Description |
|--------|-----------|-------------|
| `recording.start` | Host → Server | Start recording |
| `recording.stop` | Host → Server | Stop recording |
| `recording.status` | Client → Server | Query recording status |

## 4. Media Negotiation Flow

### 4.1 Joining a Room

```
Client                      Server                      mediasoup
  │                           │                           │
  │  room.join(roomId)        │                           │
  │──────────────────────────►│                           │
  │                           │                           │
  │  ◄────────────────────────│ room.joined               │
  │                           │   { room,                 │
  │                           │     routerRtpCapabilities }│
  │                           │                           │
  │  transport.create(type)   │                           │
  │──────────────────────────►│                           │
  │                           │─── router.createTransport │
  │                           │──────────────────────────►│
  │                           │◄──────────────────────────│
  │  ◄────────────────────────│ transportCreated          │
  │      { transportId, iceParameters, iceCandidates,     │
  │        dtlsParameters }                               │
  │                           │                           │
  │  transport.connect()      │                           │
  │──────────────────────────►│                           │
  │                           │─── transport.connect()    │
  │                           │──────────────────────────►│
  │                           │◄──────────────────────────│
  │  ◄────────────────────────│ transportConnected        │
  │                           │                           │
  │  produce(kind, rtpParameters)                         │
  │──────────────────────────►│                           │
  │                           │─── producer.create()      │
  │                           │──────────────────────────►│
  │                           │◄──────────────────────────│
  │  ◄────────────────────────│ produced { producerId }   │
  │                           │                           │
  │  consume(producerId)      │  (creates Consumers for    │
  │──────────────────────────►│   other participants)      │
  │                           │─── consumer.create()      │
  │                           │──────────────────────────►│
  │                           │◄──────────────────────────│
  │  ◄────────────────────────│ consumed { consumerId,    │
  │      producerId, id,      │
  │      kind, rtpParameters, │
  │      type, producerPaused }                           │
  │                           │                           │
```

### 4.2 Subscribing to New Participants

When participant B joins, existing participant A automatically receives events:

```
Server                      Client A
  │                           │
  │  participant.joined       │
  │──────────────────────────►│
  │  { participant: B }       │
  │                           │
  │  track.added              │
  │──────────────────────────►│
  │  { track: B's video }     │
  │                           │
  │  track.added              │
  │──────────────────────────►│
  │  { track: B's audio }     │
  │                           │
  │  (Client A may consume proactively) │
  │                           │
```

## 5. Inter-Service Signaling (gRPC)

### 5.1 Conference Service

```protobuf
service ConferenceService {
  // Room management
  rpc CreateRoom(CreateRoomRequest) returns (Room);
  rpc GetRoom(GetRoomRequest) returns (Room);
  rpc EndRoom(EndRoomRequest) returns (Empty);
  rpc ListRooms(ListRoomsRequest) returns (ListRoomsResponse);

  // Participant management
  rpc JoinRoom(JoinRoomRequest) returns (JoinRoomResponse);
  rpc LeaveRoom(LeaveRoomRequest) returns (Empty);
  rpc KickParticipant(KickRequest) returns (Empty);
  rpc MuteParticipant(MuteRequest) returns (Empty);

  // Recording
  rpc StartRecording(RecordingRequest) returns (RecordingStatus);
  rpc StopRecording(RecordingRequest) returns (RecordingStatus);
  rpc GetRecordingStatus(RecordingRequest) returns (RecordingStatus);
}
```

### 5.2 SFU Management Service

```protobuf
service SfuService {
  // Worker management
  rpc GetWorkerStatus(Empty) returns (WorkerStatus);
  rpc ListWorkers(Empty) returns (ListWorkersResponse);
  rpc AllocateWorker(AllocateRequest) returns (Worker);

  // Region routing
  rpc GetRegionStatus(Empty) returns (RegionStatus);
  rpc RouteToRegion(RouteRequest) returns (RouteResponse);
}
```

## 6. Error Codes

| Error code | HTTP analogue | Description |
|------------|---------------|-------------|
| `ROOM_NOT_FOUND` | 404 | Room does not exist |
| `ROOM_FULL` | 403 | Room is full |
| `ROOM_LOCKED` | 403 | Room is locked |
| `ROOM_ENDED` | 410 | Room has ended |
| `UNAUTHORIZED` | 401 | Not authenticated |
| `FORBIDDEN` | 403 | No permission |
| `INVALID_PARAMS` | 400 | Invalid parameters |
| `TRANSPORT_ERROR` | 500 | Transport layer error |
| `PRODUCE_FAILED` | 500 | Publish failed |
| `CONSUME_FAILED` | 500 | Subscribe failed |
| `RATE_LIMITED` | 429 | Rate limited |
| `INTERNAL_ERROR` | 500 | Internal error |

## 7. References

- Message format modeled on Colibri2's RESTful API design (Jitsi)
- Inter-service RPC modeled on LiveKit's PSRPC pattern
- Media negotiation flow follows the standard mediasoup API
