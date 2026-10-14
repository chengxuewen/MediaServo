# mediasoup-demo Architecture Deep Dive

> Source analysis: [versatica/mediasoup-demo](https://github.com/versatica/mediasoup-demo) v3 tag
> Analysis date: 2026-07-31

## 1. Overall Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                        Client Browser (app/)                      │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────────────┐  │
│  │ mediasoup-   │   │ protoo-      │   │  Redux Store         │  │
│  │ client       │   │ client       │   │  (state management)  │  │
│  │ (Device/     │   │ (WebSocket)  │   └──────────────────────┘  │
│  │  Transport)  │   └──────┬───────┘                             │
│  └──────┬───────┘          │                                      │
│         │ ICE/DTLS/SRTP    │ WebSocket (JSON)                     │
│         │                  │                                      │
└─────────┼──────────────────┼──────────────────────────────────────┘
          │                  │
          │   ┌──────────────┼───────────────────────────────┐
          │   │          Server (server/)                     │
          │   │  ┌──────────────────────────────┐            │
          │   │  │  WsServer (protoo-server)    │            │
          │   │  │  ┌────────────────────────┐  │            │
          │   │  │  │  protoo.Room           │  │            │
          │   │  │  │  ├─ Peer (browser 1)   │  │            │
          │   │  │  │  ├─ Peer (browser 2)   │  │            │
          │   │  │  │  └─ Peer (browser N)   │  │            │
          │   │  │  └────────────────────────┘  │            │
          │   │  └──────────┬───────────────────┘            │
          │   │             │ events                         │
          │   │  ┌──────────▼───────────────────┐            │
          │   │  │     Room                     │            │
          │   │  │  ┌───────────────────────┐   │            │
          │   │  │  │ producerRouter         │   │            │
          │   │  │  │ consumerRouter         │   │            │
          │   │  │  │ producerWebRtcServer   │   │            │
          │   │  │  │ consumerWebRtcServer   │   │            │
          │   │  │  │ AudioLevelObserver     │   │            │
          │   │  │  │ ActiveSpeakerObserver  │   │            │
          │   │  │  │ Bot                    │   │            │
          │   │  │  └───────────────────────┘   │            │
          │   │  └──────────┬───────────────────┘            │
          │   └──────────────┼───────────────────────────────┘
          │                  │
          │   ┌──────────────▼───────────────────────────────┐
          │   │         mediasoup Workers (N = CPU cores)     │
          │   │  ┌───────────┐  ┌───────────┐  ┌───────────┐ │
          │   │  │ Worker[0] │  │ Worker[1] │  │ Worker[N] │ │
          │   │  │┌─────────┐│  │┌─────────┐│  │┌─────────┐│ │
          │   │  ││Router   ││  ││Router   ││  ││Router   ││ │
          │   │  ││(producer││  ││(consumer││  ││Router   ││ │
          │   │  ││ side)   ││  ││ side)   ││  ││         ││ │
          │   │  │├─────────┤│  │├─────────┤│  │├─────────┤│ │
          │   │  ││WebRtc   ││  ││WebRtc   ││  ││WebRtc   ││ │
          │   │  ││Server   ││  ││Server   ││  ││Server   ││ │
          │   │  │└─────────┘│  │└─────────┘│  │└─────────┘│ │
          │   │  └───────────┘  └───────────┘  └───────────┘ │
          │   └───────────────────────────────────────────────┘
          └──────────────────────────────────────────────────────┘
```

### Core Layering

| Layer | Component | Responsibility |
|----|------|------|
| Signaling layer | protoo-server / WsServer | WebSocket connection management, JSON message routing |
| Room logic layer | Room | Producer/Consumer orchestration, peer lifecycle |
| Endpoint abstraction layer | Peer / BroadcasterPeer | Per-endpoint transport/producer/consumer management |
| Media routing layer | mediasoup Worker + Router | Actual media packet forwarding |
| Client SDK | mediasoup-client + protoo-client | Browser-side Device/Transport/Producer/Consumer |

## 2. Startup & Multi-Worker Configuration

### 2.1 Process Entry Point

```
index.ts → getConfig() → Server.create(config) → server listening
```

### 2.2 Worker Creation Strategy

```typescript
// Server.ts: createWorkersAndWebRtcServers()
const numWorkers = config.mediasoup.numWorkers; // default = os.cpus().length

for (let idx = 0; idx < numWorkers; ++idx) {
    const worker = await mediasoup.createWorker({ ...workerSettings });

    // Each Worker gets one WebRtcServer, with incrementing ports
    const webRtcServerOptions = { ...baseWebRtcServerOptions };
    for (const listenInfo of webRtcServerOptions.listenInfos) {
        listenInfo.port += idx;  // Worker 0: 44444, Worker 1: 44445, ...
    }

    const webRtcServer = await worker.createWebRtcServer(webRtcServerOptions);
    workersAndWebRtcServers.set(idx, { worker, webRtcServer });
}
```

Key points:
- **Worker count = CPU core count**
- **One WebRtcServer per Worker**, with incrementing ports (44444, 44445, ...)
- WebRtcServer listens on both UDP and TCP

### 2.3 Default Configuration

```javascript
// config.example.mjs
mediasoup: {
    numWorkers: Object.keys(os.cpus()).length,
    workerSettings: {
        logLevel: 'warn',
        // Rich logTags: info, ice, dtls, rtp, srtp, rtcp, rtx, bwe, score...
    },
    webRtcServerOptions: {
        listenInfos: [
            { protocol: 'udp', ip: '0.0.0.0', port: 44444 },
            { protocol: 'tcp', ip: '0.0.0.0', port: 44444 },
        ],
    },
    webRtcTransportOptions: {
        initialAvailableOutgoingBitrate: 1_000_000,
        minimumAvailableOutgoingBitrate: 600_000,
        maxSendMessageSize: 5_000_000,
        sctpSendBufferSize: 7_000_000,
        enableSctp: true,
    },
    routerOptions: {
        mediaCodecs: [
            { kind: 'audio', mimeType: 'audio/opus', clockRate: 48000, channels: 2 },
            { kind: 'video', mimeType: 'video/VP8', clockRate: 90000 },
            { kind: 'video', mimeType: 'video/H264', clockRate: 90000,
              parameters: { 'packetization-mode': 1, 'profile-level-id': '42e01f' } },
            // VP9, AV1, H265...
        ],
    },
}
```

### 2.4 listenIps vs webRtcServer

mediasoup-demo uses **webRtcServer mode** (not the old listenIps mode):

- `webRtcServer` is a standalone object that manages the UDP/TCP sockets
- A `WebRtcTransport` references the `webRtcServer` at creation time, with no direct listenIps configuration
- Benefit: multiple Transports share the same UDP/TCP ports, giving ICE reuse

## 3. Room Creation and Router Allocation

### 3.1 On-Demand Room Creation

```
Client WS connect → WsServer → Server.getOrCreateRoom(roomId)
```

```typescript
// Server.ts: getOrCreateRoom()
// Uses AwaitQueue to prevent race conditions (multiple users joining the same room at once)
await roomData.queue.push(async () => {
    if (roomData.room) return roomData.room; // Already exists, return it directly

    // Allocate a producer Worker (round-robin)
    const { worker: producerWorker, webRtcServer: producerWebRtcServer } =
        this.getNextWorkerAndWebRtcServer();

    // With usePipeTransports=true, allocate a second consumer Worker
    const { worker: consumerWorker, webRtcServer: consumerWebRtcServer } =
        usePipeTransports
            ? this.getNextWorkerAndWebRtcServer()
            : { worker: producerWorker, webRtcServer: producerWebRtcServer };

    // Create Router(s)
    const producerRouter = await producerWorker.createRouter({ mediaCodecs });
    const consumerRouter = usePipeTransports
        ? await consumerWorker.createRouter({ mediaCodecs })
        : producerRouter; // Single Worker: producer and consumer share one Router

    const room = await Room.create({ producerRouter, consumerRouter, ... });
});
```

### 3.2 Producer / Consumer Router Separation

```
Single Worker (usePipeTransports=false):
  ┌───────────────────┐
  │   Worker W0       │
  │  ┌─────────────┐  │
  │  │  Router R0  │  │ ← Shared by Producer + Consumer
  │  │  WebRtcSvr  │  │
  │  └─────────────┘  │
  └───────────────────┘

Multiple Workers (usePipeTransports=true):
  ┌──────────────┐     pipeToRouter    ┌──────────────┐
  │  Worker W0   │ ←────────────────── │  Worker W1   │
  │ ┌──────────┐ │                     │ ┌──────────┐ │
  │ │ Router R0│ │   (pipeTransport)   │ │ Router R1│ │
  │ │(producer)│ │───────────────────→ │ │(consumer)│ │
  │ │WebRtcSvr │ │                     │ │WebRtcSvr │ │
  │ └──────────┘ │                     │ └──────────┘ │
  └──────────────┘                     └──────────────┘
```

**Options received by Room**:
```typescript
type RoomCreateOptions = {
    roomId: RoomId;
    consumerReplicas: number;         // Number of Consumer replicas (0=none)
    usePipeTransports: boolean;       // Whether to use cross-Worker PipeTransport
    disableBwe: boolean;              // Whether to disable the BWE RTP extension
    config: ServerConfig;
    producerRouter: mediasoup.Router;
    consumerRouter: mediasoup.Router;
    producerWebRtcServer: mediasoup.WebRtcServer;
    consumerWebRtcServer: mediasoup.WebRtcServer;
};
```

## 4. Complete Client Join Signaling Sequence

### 4.1 Sequence Diagram

```
Browser (RoomClient)                  Server (WsServer → Room → Peer)
        │                                       │
        │─── WebSocket connect ────────────────→│ Query params: roomId, peerId
        │                                       │
        │←─── "mediasoupVersion" notification ──│ { version: "3.x.x" }
        │                                       │
   [Create mediasoupClient.Device]              │
        │                                       │
        │─── request: "getRtcStatsUrl" ────────→│
        │←─── response: { rtcstatsUrl } ────────│
        │                                       │
        │─── request: "getRouterRtpCapabilities"│
        │←─── response: { routerRtpCapabilities }│
        │                                       │
   [device.load({ routerRtpCapabilities })]     │
        │                                       │
   [getUserMedia hack for autoplay policy]      │
        │                                       │
   [if produce: create send Transport]          │
   [if consume: create recv Transport]          │
        │                                       │
        │─── request: "createWebRtcTransport" ─→│ { forceTcp, appData: { direction } }
        │                                       │  Server: router.createWebRtcTransport({
        │                                       │    webRtcServer, enableUdp, enableTcp,
        │                                       │    iceConsentTimeout: 20, enableSctp: true
        │                                       │  })
        │←─── response: {                       │
        │       id, iceParameters,              │
        │       iceCandidates, dtlsParameters,   │
        │       sctpParameters                   │
        │     }                                  │
        │                                       │
   [client.createSendTransport({...})]          │
        │                                       │
        │───  "connect" event ──────────────────│ DTLS handshake
        │─── request: "connectWebRtcTransport" ─→│ transport.connect({ dtlsParameters })
        │←─── accept ──────────────────────────│
        │                                       │
   [if consume: same flow for recvTransport]    │
        │                                       │
        │─── request: "join" ───────────────────→│ { displayName, device, rtpCapabilities }
        │←─── response: { peers: [...] } ────────│ List of other peers currently in the room
        │                                       │
   [State → 'connected']                        │
   [Notify each peer in peers via newPeer]      │
   [Call consume for existing peers' producers] │
        │                                       │
        │←─── notification: "newPeer" ────────────│ (broadcast to other peers)
        │                                       │
```

### 4.2 Key Code Paths

**Client `_joinRoom()`** (RoomClient.js:2327):

```javascript
async _joinRoom() {
    // 1. Create the device
    this._mediasoupDevice = await mediasoupClient.Device.factory({ handlerName });

    // 2. Fetch Router RTP capabilities
    const { routerRtpCapabilities } = await this._protoo.request('getRouterRtpCapabilities');
    await this._mediasoupDevice.load({ routerRtpCapabilities });

    // 3. Create send Transport (direction: 'producer')
    const transportInfo = await this._protoo.request('createWebRtcTransport', {
        forceTcp: this._forceTcp,
        appData: { direction: 'producer' },
    });
    this._sendTransport = this._mediasoupDevice.createSendTransport({...});

    this._sendTransport.on('connect', ({ dtlsParameters }, callback) => {
        this._protoo.request('connectWebRtcTransport', {
            transportId: this._sendTransport.id,
            dtlsParameters,
        }).then(callback);
    });

    // 4. Create recv Transport (direction: 'consumer') — if consume=true
    const recvInfo = await this._protoo.request('createWebRtcTransport', {
        forceTcp: this._forceTcp,
        appData: { direction: 'consumer' },
    });
    this._recvTransport = this._mediasoupDevice.createRecvTransport({...});

    // 5. Join room
    const { peers } = await this._protoo.request('join', {
        displayName, device,
        rtpCapabilities: this._consume ? this._mediasoupDevice.rtpCapabilities : undefined,
    });

    store.dispatch(stateActions.setRoomState('connected'));
}
```

**Server-side Peer join handling** (Peer.ts):

```typescript
case 'join': {
    const { displayName, device, rtpCapabilities } = data;
    this.#joined = true;
    this.#displayName = displayName;
    this.#device = device;
    this.#rtpCapabilities = rtpCapabilities;

    clearTimeout(this.#joinTimer); // Cancel the 30s timeout

    this.emit('joined', serializedPeers => {
        accept({ peers: serializedPeers }); // Return the other peers in the room
    });
}
```

**Peer timeout mechanism**: A 30s join timer starts when a Peer is created; it closes automatically on expiry.

## 5. WebRtcTransport Creation

### 5.1 Server-Side Handling

```typescript
// Room.ts: handlePeer() → 'create-web-rtc-transport' event
peer.on('create-web-rtc-transport', async ({ direction, forceTcp }, resolve, reject) => {
    // Pick the corresponding Router and WebRtcServer based on direction
    const mediasoupRouter = (direction === 'producer')
        ? this.#producerRouter
        : this.#consumerRouter;

    const mediasoupWebRtcServer = (direction === 'producer')
        ? this.#producerWebRtcServer
        : this.#consumerWebRtcServer;

    const transport = await mediasoupRouter.createWebRtcTransport({
        ...this.#config.mediasoup.webRtcTransportOptions,
        enableUdp: !forceTcp,
        enableTcp: true,
        webRtcServer: mediasoupWebRtcServer,  // Key: bind to the WebRtcServer
        iceConsentTimeout: 20,
        enableSctp: true,
        appData: { direction },
    });

    resolve(transport);
});
```

### 5.2 Transport Parameter Reference

| Parameter | Value | Description |
|------|-----|------|
| `webRtcServer` | The Worker's bound WebRtcServer | ICE reuse, many Transports on one port |
| `enableUdp` | `!forceTcp` | Default true |
| `enableTcp` | `true` | Always enable the TCP fallback |
| `iceConsentTimeout` | `20` | ICE consent refresh timeout (seconds) |
| `enableSctp` | `true` | Enable DataChannel |
| `initialAvailableOutgoingBitrate` | 1_000_000 | Initial outgoing bitrate |

### 5.3 Producer and Consumer Use Different Transports

- **sendTransport** (direction='producer'): bound to producerRouter + producerWebRtcServer
- **recvTransport** (direction='consumer'): bound to consumerRouter + consumerWebRtcServer

With a single Worker (usePipeTransports=false), both point at the same Router and WebRtcServer.

## 6. Produce / Consume Signaling Interaction

### 6.1 Producer Creation

```
Browser                           Server (Peer → Room)
  │                                       │
  │─── _sendTransport.on('produce')      │
  │    (triggered automatically by        │
  │     mediasoup-client)                 │
  │                                       │
  │─── request: "produce" ──────────────→│ { transportId, kind, rtpParameters, appData }
  │                                       │
  │                                       │ transport.produce({ kind, rtpParameters,
  │                                       │   appData: { peerId, source } })
  │                                       │
  │←─── response: { producerId } ─────────│
  │                                       │
  │                                       │ emit 'new-producer' → Room
  │                                       │
  │                                       │ Room: if usePipeTransports →
  │                                       │   producerRouter.pipeToRouter({
  │                                       │     producerId, router: consumerRouter
  │                                       │   })
  │                                       │
  │                                       │ Room: for each otherPeer →
  │                                       │   otherPeer.consume({ producer })
  │                                       │
  │                                       │ if audio:
  │                                       │   audioLevelObserver.addProducer()
  │                                       │   activeSpeakerObserver.addProducer()
  │                                       │
```

**Client-side produce trigger**:
```javascript
// RoomClient.js: sendTransport.on('produce')
this._sendTransport.on('produce', async ({ kind, rtpParameters, appData }, callback) => {
    const { producerId } = await this._protoo.request('produce', {
        transportId: this._sendTransport.id,
        kind, rtpParameters, appData,
    });
    callback({ id: producerId });
});

// Actual calls (when enableMic/enableWebcam)
this._micProducer = await this._sendTransport.produce({
    track: audioTrack, appData: { source: 'webcam' }
});
this._webcamProducer = await this._sendTransport.produce({
    track: videoTrack, appData: { source: 'webcam' }
});
```

### 6.2 Consumer Creation

```
Server (Peer.consume)                     Browser
  │                                       │
  │ 1. Check canConsume:                  │
  │    router.canConsume({ producerId,     │
  │      rtpCapabilities })                │
  │                                       │
  │ 2. Create Consumer (paused):          │
  │    consumer = await transport          │
  │      .consume({ producerId,            │
  │        rtpCapabilities,                │
  │        enableRtx: true,                │
  │        paused: true,                   │
  │        ignoreDtx: true })              │
  │                                       │
  │ 3. Notify the client:                 │
  │─── notification: "newConsumer" ───────→│ { peerId, transportId, consumerId,
  │                                       │   producerId, kind, rtpParameters,
  │                                       │   type, producerPaused, consumerScore,
  │                                       │   appData }
  │                                       │
  │                                       │ 4. Client creates the local Consumer:
  │                                       │    consumer = await
  │                                       │      recvTransport.consume({
  │                                       │        id: consumerId,
  │                                       │        producerId, kind,
  │                                       │        rtpParameters, appData })
  │                                       │
  │                                       │ 5. On success:
  │─── request: "resume" ←────────────────│
  │                                       │
  │ 6. await consumer.resume()            │
  │                                       │
  │                                       │ 7. consumer.on('track')
  │                                       │    → video.srcObject = stream
```

**Client-side newConsumer handling** (RoomClient.js:387):
```javascript
case 'newConsumer': {
    const { peerId, producerId, consumerId, kind, rtpParameters, type,
            producerPaused, consumerScore, appData } = request;

    const consumer = await this._recvTransport.consume({
        id: consumerId, producerId, kind, rtpParameters, appData,
    });

    // consumer.on('track') → render into the UI
    // Then resume: await this._protoo.request('resume', { consumerId });
}
```

### 6.3 Consumer Replicas

```typescript
// Peer.ts: consume()
const consumerCount = 1 + consumerReplicas; // Default 0 → 1 Consumer

for (let i = 0; i < consumerCount; ++i) {
    // Each consumer is created independently, possibly on a different transport
    const consumer = await transport.consume({ ... });
    await this.request('newConsumer', { ... });
    await consumer.resume();
}
```

### 6.4 Pipe Transport (usePipeTransports=true)

```
Producer (Worker 0)                    Consumer (Worker 1)
       │                                       │
       │  producerRouter.pipeToRouter({        │
       │    producerId,                        │
       │    router: consumerRouter             │
       │  })                                   │
       │                                       │
       │  ──── PipeTransport ────────────────→ │
       │         (internal RTP forwarding)     │
       │                                       │
       │                                       │ otherPeer.consume({ producer })
       │                                       │   → consumerRouter.canConsume()
       │                                       │   → transport.consume(...)
       │                                       │   → notify("newConsumer")
```

**Key point**: `pipeToRouter` is only called when `usePipeTransports=true`. With a single Worker, Producer and Consumer live inside the same Router, so no pipe is needed.

## 7. Error Handling and Reconnection

### 7.1 Peer Timeout

```typescript
// Peer.ts constructor
this.#joinTimer = setTimeout(() => {
    logger.debug(`Peer didn't join in ${JOIN_TIMEOUT_MS}ms, closing it`);
    this.close();
}, JOIN_TIMEOUT_MS); // 30 seconds
```

### 7.2 WebSocket Disconnection

```typescript
// Peer.ts: protooPeer.on('close')
this.#protooPeer.on('close', () => {
    // 1. Clean up all Transport/Producer/Consumer (mediasoup side)
    for (const transport of this.#transports.values()) {
        transport.close();       // mediasoup worker cleanup
    }
    this.#producers.clear();
    this.#consumers.clear();

    // 2. Emit the disconnected event
    this.emit('disconnected');

    // 3. Final close → Room removes the Peer
    this.close();
});
```

### 7.3 Room Notifies Other Peers

```typescript
// Room.ts: handlePeer()
peer.on('disconnected', () => {
    const otherPeers = this.getOtherPeers(peer);
    for (const otherPeer of otherPeers) {
        otherPeer.notify('peerClosed', { peerId: peer.id });
    }
});
```

### 7.4 Duplicate peerId Handling

```typescript
// Room.ts: mayCloseExistingPeer()
private mayCloseExistingPeer(peerId: PeerId): void {
    // If a Peer with the same peerId already exists, close the old one first
    if (existingPeer) { existingPeer.close(); }
    if (existingJoiningPeer) { existingJoiningPeer.close(); }
    if (existingBroadcasterPeer) { existingBroadcasterPeer.close(); }
}
```

### 7.5 Worker Crash Handling

```typescript
// Server.ts constructor
for (const { worker } of this.#workersAndWebRtcServers.values()) {
    if (worker.closed) {
        throw new InvalidStateError(
            `mediasoup Worker is closed [pid:${worker.pid}, died:${worker.died}]`
        );
    }
}
```

### 7.6 DTLS Connection Errors

```typescript
// Peer.ts: transport observer
transport.observer.on('close', () => {
    this.#transports.delete(transport.id);
});
// WebRtcTransport iceConsentTimeout: 20s — closes automatically on expiry
```

### 7.7 Client-Side Disconnection

The client detects WebSocket disconnection automatically via `protoo-client`. In `RoomClient.js`:

```javascript
this._protoo.on('close', () => {
    if (!this._closed) {
        this.close(); // Clean up all producer/consumer/transport
        store.dispatch(stateActions.setRoomState('closed'));
    }
});
```

## 8. Multi-Worker Load Balancing

### 8.1 Round-Robin Allocation

```typescript
// Server.ts
#nextWorkerIdx = 0;

private getNextWorkerAndWebRtcServer() {
    const { worker, webRtcServer } =
        this.#workersAndWebRtcServers.get(this.#nextWorkerIdx)!;

    if (++this.#nextWorkerIdx === this.#workersAndWebRtcServers.size) {
        this.#nextWorkerIdx = 0; // Wrap around
    }

    return { worker, webRtcServer };
}
```

### 8.2 Load Balancing Strategy

**Allocation granularity: Room level**

- Each Room is assigned **a pair of Workers** (producer + consumer) at creation time
- All Peers inside a Room share the same producerRouter and consumerRouter
- New Room → round-robin onto the next Worker pair

**Design trade-offs**:

| Option | Pros | Cons |
|------|------|------|
| Room-level allocation (current) | Simple to implement; in-room media routes within one Worker, low latency | A popular room can overload a single Worker |
| Peer-level allocation | More even load distribution | Expensive cross-Worker communication within a room, needs more pipes |
| Tiered Workers | Separates Producer/Consumer | Requires at least 2 Workers; cross-Worker pipes add overhead |

### 8.3 Load Distribution with usePipeTransports

```
usePipeTransports=false (single Worker mode):
  All peers in a Room live on one Worker
  Pros: zero extra forwarding latency
  Cons: single-Worker bottleneck

usePipeTransports=true (dual Worker mode):
  Producer → Worker 0 (producerRouter)
           ↓ pipeToRouter
  Consumer ← Worker 1 (consumerRouter)
  Pros: separates producer and consumer load
  Cons: requires at least 2 Workers; pipes add internal latency
```

## 9. Key Technical Details

### 9.1 BWE Extension Control

```typescript
// Room.ts constructor
if (disableBwe) {
    // Remove the transport-wide CC extension from the consumerRouter RTP capabilities
    this.#consumerRouterRtpCapabilities = this.disableBweRtpExtensions();
}
```

Removed RTP header extensions:
- `http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01`
- `http://www.ietf.org/id/draft-holmer-rmcat-abs-send-time-13`

### 9.2 AudioLevelObserver + ActiveSpeakerObserver

```typescript
const audioLevelObserver = await producerRouter.createAudioLevelObserver({
    maxEntries: 10,
    threshold: -80,  // dB
    interval: 800,    // ms
});

const activeSpeakerObserver = await producerRouter.createActiveSpeakerObserver();
```

- After a Producer is created, audio producers are registered to the observer automatically
- `audioLevelObserver.on('volumes')` → broadcast to all peers
- `activeSpeakerObserver.on('dominantspeaker')` → broadcast to all peers

### 9.3 Bot Functionality

```typescript
const bot = await Bot.create({ usePipeTransports, producerRouter, consumerRouter });
```

The Bot joins automatically when a Room is created and can play audio from a file, for testing.

### 9.4 Network Throttle

Supports limiting Uplink/Downlink bandwidth via API:
```typescript
// Via the terminal client or the API
Server.applyNetworkThrottle({ secret, options: { uplink, downlink, rtt, loss } });
```

### 9.5 BroadcasterPeer (RTP Endpoint)

Supports non-browser endpoints joining via PlainTransport (RTP):
- `createPlainTransport` → `connectPlainTransport` → `produce` → `consume`
- Suitable for external encoders such as FFmpeg / GStreamer

## 10. Message Type Overview

### protoo requests (client → server)

| Method | Description |
|------|------|
| `getRtcStatsUrl` | Get the rtcstats monitoring URL |
| `getRouterRtpCapabilities` | Get the codecs supported by the Router |
| `createWebRtcTransport` | Create a WebRTC Transport (producer/consumer) |
| `connectWebRtcTransport` | DTLS handshake |
| `restartIce` | ICE restart |
| `join` | Join a room |
| `produce` | Create a Producer |
| `produceData` | Create a DataProducer |
| `resume` | Resume a Consumer |
| `pause` | Pause a Consumer |
| `setConsumerPreferredLayers` | Set the consumer layer (simulcast) |
| `consumerRequestKeyFrame` | Request a keyframe |
| `changeDisplayName` | Change the display name |
| `chatMessage` | Chat message (via DataChannel) |
| `muteMic` / `muteWebcam` | Mute the mic / turn off the camera |
| `restartIce` | ICE restart |

### protoo notifications (server → client)

| Notification | Description |
|------|------|
| `mediasoupVersion` | mediasoup version |
| `newPeer` | A new peer joined |
| `peerClosed` | A peer left |
| `newConsumer` | A new stream is available to subscribe |
| `consumerClosed` | A stream ended |
| `consumerPaused` / `consumerResumed` | Stream paused/resumed |
| `consumerLayersChanged` | Simulcast layer change |
| `consumerScoreChanged` | Consumer quality score changed |
| `activeSpeaker` | Active speaker |
| `speakingPeers` | All speakers' volumes |
| `downlinkBwe` | Downlink bandwidth estimate |
| `chatMessage` | Chat message (via DataChannel) |
| `newDataConsumer` | New DataConsumer |

## 11. Dependency Summary

| Layer | Dependency | Version |
|----|------|------|
| Server | `mediasoup` | ^3.x |
| Server | `protoo-server` | (WebSocket JSON signaling) |
| Client | `mediasoup-client` | ^3.x |
| Client | `protoo-client` | (WebSocket JSON signaling) |
| Client | `awaitqueue` | (request serialization) |
| Client | `@rtcstats/rtcstats-js` | (WebRTC statistics) |

## References

- [mediasoup-demo v3](https://github.com/versatica/mediasoup-demo/tree/v3)
- [mediasoup official docs v3](https://mediasoup.org/documentation/v3/)
