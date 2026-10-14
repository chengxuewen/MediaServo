# OpenVidu Meet Client Architecture Reference

> Generated: 2026-07-31 | Category: Video Conferencing
> Repo: https://github.com/OpenVidu/openvidu (openvidu-call)

## 1. Overview

OpenVidu Meet (officially OpenVidu Call) is the flagship official web-client application of the OpenVidu platform. A React-based WebRTC video-conferencing app, it demonstrates the full capability of the openvidu-browser SDK and defines the best-practice client architecture in the OpenVidu ecosystem: connection management, media pipelines, UI component orchestration, and network fault tolerance.

### 1.1 The Three-Layer Architecture

```
UI layer: React components (video grid / toolbar / chat panel / settings)
State management: React Context + useReducer
        │
Business-logic layer: openvidu-browser SDK
Session / Publisher / Subscriber / StreamManager
Event-driven: streamCreated, connectionCreated, signal
        │ REST (HTTPS) + WebSocket (WSS)
openvidu-server (Java Spring Boot)
REST API: session/connection/token CRUD
WebSocket: SDP negotiation, ICE exchange, signaling messages
```

OpenVidu Meet never touches the WebRTC APIs directly. All WebRTC interaction goes through the openvidu-browser SDK wrapper; internally the SDK manages RTCPeerConnection, ICE candidate gathering, and SDP negotiation. Client developers only call high-level methods such as Session.connect(), Session.publish(), and Session.subscribe().

### 1.2 The Interface Boundary with openvidu-server

| Interface | Protocol | Purpose | Client-side user |
|-----------|----------|---------|------------------|
| REST API | HTTPS | Create a Session, obtain a Token | The application server (never called directly by the client) |
| WebSocket | WSS | SDP negotiation, ICE exchange, signaling messages | The openvidu-browser SDK |
| TURN/STUN | UDP/TCP | ICE candidate gathering | The browser's native WebRTC |

Key design: the client never calls the REST API directly. The application server fetches the token from openvidu-server and passes it to the client. The client communicates with openvidu-server only over the WebSocket.

## 2. Technology Stack

### 2.1 Core Dependencies

| Layer | Technology | Version | Purpose |
|-------|------------|---------|---------|
| Framework | React | 18.x | UI component architecture |
| Language | TypeScript | 5.x | Type safety |
| Build | Vite or CRA | 5.x | Dev/prod builds |
| WebRTC SDK | openvidu-browser | 2.31.0 | The WebRTC wrapper |
| Styling | CSS Modules + SCSS | - | Component style isolation |
| State management | React Context + useReducer | - | Global state |
| Routing | React Router | 6.x | Page routing |

### 2.2 The openvidu-browser SDK's Core Classes

- **OpenVidu**: the entry-point factory; creates Session/Publisher
- **Session**: the conference session; core methods connect/publish/subscribe/disconnect/signal; event-driven (streamCreated, connectionCreated, sessionDisconnected)
- **Publisher** (extends StreamManager): local media publishing; methods publishAudio/publishVideo/replaceTrack; events accessAllowed/accessDenied
- **Subscriber** (extends StreamManager): remote media reception; events videoElementCreated/streamPlaying
- **StreamManager** base class: the stream property, the videos array, addVideoElement/createVideoElement video-element management

### 2.3 State Management

OpenVidu Meet manages global state with React Context + useReducer, without Redux or MobX:

```typescript
interface CallState {
  session: Session | null;
  connectionStatus: 'disconnected' | 'connecting' | 'connected';
  participants: Map<string, Participant>;
  publisher: Publisher | null;
  subscribers: Map<string, Subscriber>;
  audioEnabled: boolean;
  videoEnabled: boolean;
  isScreenSharing: boolean;
  chatMessages: ChatMessage[];
  activeSpeaker: string | null;
}
```

The Context is injected into the component tree via a Provider; children read state through useContext and update through dispatch.

## 3. The Connection Flow

### 3.1 The Full Connection Lifecycle

```
Client                        Application server              openvidu-server
 1. Request to join a room ─────────────►
                              2. POST /api/sessions ────────►
                              3. sessionId returned ◄──────────
                              4. POST /api/sessions/{id}/connection ──►
                              5. token returned ◄───────────────
 6. token returned ◄──────────────
 7. new OpenVidu() + OV.initSession()
 8. session.on('streamCreated', ...)
 9. session.connect(token) ─────────────────────────────────►
                   ◄── WebSocket connection established ──►
10. session.on('connectionCreated')
11. OV.initPublisher() + session.publish(publisher) ────────►
                   ◄── SDP offer/answer ──────►
                   ◄── ICE candidate exchange ──────────►
12. streamCreated event (remote) → session.subscribe(stream)
13. Video elements added to the DOM and playing
```

### 3.2 Token Acquisition (Application-Server Side)

The client never calls the openvidu-server REST API directly. The application server creates the Session and Connection via the openvidu-java-client or openvidu-node-client SDK and returns a token to the client. The token is a JWT string carrying sessionId, connectionId, role, and permission information. The client authenticates by carrying the token when opening the WebSocket connection:

```
wss://openvidu.example.com?sessionId=ses_xxx&token=wss://....
```

## 4. Video/Audio Pipelines

### 4.1 The Publish Pipeline (Publisher)

```
getUserMedia → MediaStream
    → OV.initPublisher(targetElement, properties)
        → create the local <video> preview, bind the MediaStream
        → await the accessAllowed event
    → session.publish(publisher)
        → RTCPeerConnection created, addTrack
        → createOffer → setLocalDescription
        → SDP sent to the server over the WebSocket
        → answer received → setRemoteDescription
        → ICE candidate gathering and exchange
    → publish succeeded; streamCreated event fires
```

Subsequent control: publishAudio(false) to mute, publishVideo(false) to turn off the camera, replaceTrack(newTrack) to switch camera/screen share.

### 4.2 The Subscribe Pipeline (Subscriber)

When a remote participant publishes a stream, the streamCreated event fires:

```typescript
session.on('streamCreated', (event) => {
    const subscriber = session.subscribe(event.stream, 'video-container', { insertMode: 'APPEND' });
    subscriber.on('videoElementCreated', (event) => { /* video element added to the DOM */ });
    subscriber.on('streamPlaying', () => { /* remote video starts playing; update layout */ });
});
```

### 4.3 Video-Element Management

Two modes: SDK-managed (pass a targetElement; the SDK creates and appends the `<video>` automatically) and manual (pass null, then call subscriber.addVideoElement() or createVideoElement()). The manual mode suits custom video-grid layout libraries.

### 4.4 Screen Sharing

Uses replaceTrack to swap the video track — no renegotiation of the WebRTC connection required:

```typescript
const screenStream = await navigator.mediaDevices.getDisplayMedia({ video: true });
const videoTrack = screenStream.getVideoTracks()[0];
await publisher.replaceTrack(videoTrack);
```

## 5. Room Management

### 5.1 The Session Lifecycle

```
Create OpenVidu → initSession() → register event listeners
    → connect(token) → connected (publish/subscribe/signal)
        → disconnect() → disconnected
```

### 5.2 Participant Tracking

The participants Map is maintained live via Session events:

```typescript
// connectionCreated → add a participant
// connectionDestroyed → remove a participant
// streamCreated → associate a stream with a participant
// streamDestroyed → disassociate
```

The participant data structure holds connectionId, nickname, isLocal, audioEnabled, videoEnabled, stream, subscriber, and joinTime.

### 5.3 Leaving the Room

```typescript
function leaveRoom() {
  session.disconnect();
  // the publisher unpublishes and subscribers unsubscribe automatically (handled inside the SDK)
  // the sessionDisconnected event fires
  dispatch({ type: 'RESET' }); // clear the participant list
  navigate('/'); // back to the home page
}
```

## 6. UI Component Architecture

### 6.1 Component Tree

```
<App>
  └── <CallProvider> (Context Provider)
        <RoomPage>
          ├── <PreJoinScreen> (device preview / name entry)
          ├── <CallContainer>
          │   ├── <VideoGrid> → <VideoTile> × n
          │   │   <video> + <ParticipantInfo> + <AudioIndicator>
          │   ├── <Toolbar>
          │   │   <MicButton> <CamButton> <ScreenShareButton>
          │   │   <ChatButton> <SettingsButton> <HangupButton>
          │   ├── <ChatPanel> (<MessageList> + <MessageInput>)
          │   └── <ParticipantsPanel>
          └── <SettingsModal> (<AudioSettings> <VideoSettings> <GeneralSettings>)
```

### 6.2 VideoGrid Layout Strategy

| Participant count | Layout mode |
|-------------------|-------------|
| 1 | Fullscreen |
| 2 | Side-by-side 50%/50% |
| 3-4 | 2x2 grid |
| 5-6 | 3x2 uneven; the active speaker slightly larger |
| 7+ | Dynamic grid, scrolling + active-speaker emphasis |

Layout computation uses pure CSS Grid or absolute positioning, with no third-party layout library:

```typescript
function calculateGridLayout(count: number) {
  if (count <= 1) return { columns: 1, rows: 1 };
  if (count <= 2) return { columns: 2, rows: 1 };
  if (count <= 4) return { columns: 2, rows: 2 };
  const columns = Math.ceil(Math.sqrt(count));
  const rows = Math.ceil(count / columns);
  return { columns, rows };
}
```

### 6.3 Toolbar ↔ Media-State Synchronization

Toolbar button states are synced into the SDK via dispatch; useCallback wrappers avoid re-renders:

```typescript
const toggleMic = useCallback(async () => {
  await publisher.publishAudio(!audioEnabled);
  dispatch({ type: 'SET_AUDIO', payload: !audioEnabled });
}, [publisher, audioEnabled]);
```

## 7. Network Fault Tolerance

### 7.1 WebSocket Reconnection

| Scenario | Behavior | User experience |
|----------|----------|-----------------|
| Transient network loss | Auto-reconnect, up to 5 attempts, exponential backoff | Brief freeze, then recovers |
| Reconnect timeout | The sessionDisconnected event fires | Prompt the user to rejoin |
| ICE connect failure | ICE restart triggered | Brief black screen, then recovers |

Exponential backoff: base 1 second, max 30 seconds, factor 2x, jitter ±25%, up to 5 retries.

### 7.2 ICE Restart & Connection-Quality Monitoring

When the server detects degrading connection quality it triggers an ICE restart; the client perceives the state change through the reconnecting/reconnected events.

openvidu-browser samples connection quality via RTCPeerConnection.getStats() every 2 seconds (RTT, packet loss, jitter, bitrate, resolution) and adapts automatically based on quality.

## 8. Consuming the openvidu-server API

### 8.1 REST API Endpoints (used by the application server)

| Method | Path | Purpose |
|--------|------|---------|
| POST | /openvidu/api/sessions | Create a session |
| POST | /openvidu/api/sessions/{id}/connection | Create a connection (generate a Token) |
| DELETE | /openvidu/api/sessions/{id} | Close a session |
| GET | /openvidu/api/sessions/{id} | Get session info |
| POST | /openvidu/api/recordings/start | Start recording |
| POST | /openvidu/api/recordings/stop | Stop recording |
| PATCH | /openvidu/api/sessions/{id}/connection/{connId} | Update connection properties |

### 8.2 WebSocket Message Types

| C→S | S→C |
|-----|-----|
| joinRoom | participantJoined |
| publishVideo | participantLeft |
| receiveVideoFrom | participantPublished |
| unpublishVideo | participantUnpublished |
| unsubscribeFrom | mediaError |
| sendMessage | iceCandidate |
| iceCandidate | connectionCreated |
| leaveRoom | connectionDestroyed |

### 8.3 Signal Messages (Custom Signaling)

OpenVidu supports custom messages via session.signal(), powering features like chat:

```typescript
// send
await session.signal({ type: 'chat', data: JSON.stringify({ message, timestamp, sender }), to: [] });
// receive
session.on('signal:chat', (event) => { /* parse event.data */ });
```

No extra WebSocket connection is needed; message kinds are distinguished by the type field, and both targeted sends and broadcasts are supported.

## 9. Designs MediaServo Can Borrow

### 9.1 The Session Abstraction Layer

The client never touches RTCPeerConnection directly; every WebRTC detail is wrapped by the SDK. The MediaServo Admin Dashboard can define a `MeetingSession` interface wrapping join/leave/publish/subscribe, with a swappable implementation underneath (P2P, SFU-mediasoup, SFU-LiveKit), fully transparent to the UI layer.

### 9.2 The Token Authorization Flow

The application server generates tokens; the client never touches REST API credentials. Each token is bound to a specific Session and Connection, supports role permissions (PUBLISHER/SUBSCRIBER), and is single-use to prevent replay. MediaServo's SFU authorization can borrow this design to replace the current plain PSK model.

### 9.3 Subscriber Auto-Management + the Signal Channel

streamCreated automatically triggers subscribe and streamDestroyed triggers unsubscribe — event-driven rather than polled. MediaServo's SFU consume flow can be wrapped into a similar reactive pattern.

The signal() mechanism provides a lightweight custom message channel with no extra connection. MediaServo's WebSocket signaling protocol could add a similar generic signal channel.

### 9.4 Separating Video-Element Management

The media layer owns obtaining/publishing/subscribing to streams; the rendering layer owns binding streams to DOM elements. The rendering layer can use any layout library, unconstrained by the media layer.

### 9.5 Gap Analysis

| Dimension | OpenVidu Meet | MediaServo Admin Dashboard |
|-----------|---------------|----------------------------|
| SDK wrapping | Complete (openvidu-browser) | None (directly drives mediasoup-client) |
| Session management | Session object + event-driven | Manually managed WebSocket messages |
| Participant tracking | Built-in (connectionCreated/Destroyed) | Maintained manually |
| Video-element management | Auto/manual dual modes | Must be implemented |
| Signal messages | Built-in signal() channel | Requires custom WS messages |
| Reconnect policy | Built-in exponential backoff | None |
| Connection quality | Built-in getStats monitoring | None |
| Device management | getDevices() + auto-switching | Must be implemented |
| Screen sharing | One-line switch via replaceTrack | Must be implemented |
| Recording | One-line REST API trigger | Not implemented |

OpenVidu Meet is a mature reference implementation; the MediaServo Admin Dashboard can extract many design patterns from it instead of reinventing the wheel.
