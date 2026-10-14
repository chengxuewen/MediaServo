# SFU — mediasoup Integration

 Status: Phase 3 implemented | Related decisions: D138, D198 | Updated: 2026-07-31

 Design evolution: Phase 2 once designed the Host to use PlainRtp (RTP/UDP direct, see below); Phase 3 unified on WebRtcTransport.
## Overview

mediasoup-sys v0.22 serves as the MediaServo SFU server, forwarding media streams Host→Server→Remote within a Room. All operations are encapsulated in `SfuComponent`.

## Core Concept Mapping

| mediasoup | MediaServo Component |
|-----------|----------------------|
| Worker | SfuComponent::WorkerPool |
| Router | RoomRouter (per-room) |
| Transport (WebRtc) | HostWebRtcTransport (ICE/DTLS/SRTP push) |
| Transport (WebRtc) | RemoteWebRtcTransport (ICE/DTLS/SRTP distribution) |
| Producer | TrackProducer (uplink) |
| Consumer | TrackConsumer (downlink) |
| SDP ↔ RtpParameters | SdpAdapter (two-way codec) |

## Worker Lifecycle

```
SfuComponent::init()
  ├─ WorkerPool (one Worker per CPU core)
  ├─ Worker::create({ rtcMinPort: 40000, rtcMaxPort: 49999 })
  └─ Listen on "died" → auto restart
```
Crash recovery: mark the Router dead → RoomManager notifies Remotes to reconnect → WorkerPool::respawn → Remotes rebuild Transport + Consumer.

## Router Configuration

```rust
worker.create_router(RouterOptions {
    media_codecs: vec![
        RtpCodecCapability::Video { mime_type: "video/VP8", ... },
        RtpCodecCapability::Video { mime_type: "video/H264", ... },
        RtpCodecCapability::Audio { mime_type: "audio/opus", ... },
    ],
});
```

One Router per Room, indexed by `room_id`. Destroyed when the Room closes.

## Transport Creation

**Host→Server (WebRtcTransport, ICE/DTLS/SRTP)**:
```
Server: router.create_webrtc_transport(WebRtcTransportOptions::new_with_server(webRtcServer))
  → returns { id, ice_parameters, ice_candidates, dtls_parameters }

Host: builds the SDP remote offer from ice_candidates + ice_parameters + dtls_parameters
  → pc.set_remote_description(SDP)
  → pc.create_answer() → pc.set_local_description(answer)
  → extracts the DTLS fingerprint → ws.send(ConnectWebRtcTransport{dtls})
  → Server: transport.connect(remote_dtls)
  → ICE handshake (Host = controlling, mediasoup = ICE-Lite answering)
  → DTLS handshake → SRTP keys derived
  → Connection established
```

**Server→Browser (WebRtcTransport, Server-Offer)**:
```
Server: router.create_webrtc_transport({
    webRtcServer, enable_udp: true, enable_tcp: true,
    iceConsentTimeout: 20
}) → { id, iceParameters, iceCandidates, dtlsParameters }
→ Browser: device.createRecvTransport(params) → ICE/DTLS automatic
```

> Design evolution: Phase 2 once designed the Host to use PlainRtp (RTP/UDP direct); Phase 3 moved to the unified WebRtcTransport because the deployment scenario (vehicle → cloud) requires NAT traversal and SRTP encryption. All endpoints use the same ICE/DTLS protocols, simplifying symmetrically.

## Producer / Consumer

```
Host:  transport.produce({kind, rtpParameters}) → Producer {id, kind}
Remote: transport.consume({producer_id, rtp_capabilities}) → Consumer {id, paused: false}
```
Consumer pause/resume controls bandwidth.

## SdpAdapter (S8)

Bidirectional SDP ↔ mediasoup RtpParameters:

```rust
trait SdpAdapter {
    fn parse_offer(sdp: &str) -> Result<RtpCapabilities>;
    fn to_send_rtp_params(sdp: &str) -> Result<RtpParameters>;
    fn create_answer(caps: &RtpCapabilities) -> Result<String>;
}
```

Internally uses the `webrtc-sdp` crate for parsing and codec mapping.

## Observer Integration

- **AudioLevelObserver**: one per Router, volume tracking → auto mute
- **ActiveSpeakerObserver**: active-speaker detection → picture-in-picture switch → RoomManager

## Crash Recovery Flow

```
1. Worker "died" → marked dead
2. Router → RoomEvent::WorkerLost { room_id }
3. RoomManager → broadcast RejoinRequired
4. WorkerPool::respawn
5. Remote join → rebuild Transport + Consumer
```

> Implementation complete (Phase 3, S1-S18)

## Transport Failure Scenarios & Recovery

> Error-code definitions and layering: see `error-model.md` §SFU media-plane errors (40xx, real-time protocol) (unified per doc-audit L6).

### 1. transport.connect() timeout

| Parameter | Default | Notes |
|-----------|---------|-------|
| Timeout | 10s | mediasoup transport.connect() default timeout |
| Error code | 4000 | ConnectionFailed |
| Recovery | Recreate the transport | Unrecoverable connections require transport rebuild + re-offer |

```
connect_transport() → timeout → send error 4000 → browser creates new transport → re-offer
```

### 2. DTLS handshake failure

| Failure | Cause | Recovery |
|---------|-------|----------|
| DTLS fingerprint mismatch | Browser DTLS signature inconsistent with the transport | Recreate the transport (new DTLS parameters) |
| SRTP encryption failure | Browser/mediasoup negotiation failure | Check the crypto suite configuration (default AES_CM_128_HMAC_SHA1_80) |

### 3. ICE restart

After the ICE connection drops, the browser side should trigger an ICE restart:
```
pc.restartIce()  →  new ICE candidates  →  server re-gathers  →  new candidate pair
```
- ICE restart keeps the transport and producer/consumer unchanged
- No new candidate pair within 30s → treat the transport as failed

### 4. produce() / consume() failure

| Failure | Cause | Recovery |
|---------|-------|----------|
| Producer creation failure | RTP parameter mismatch | Return error 4001; browser checks RTP codecs |
| Consumer creation failure | RTP capabilities mismatch | Return error 4002; browser checks receiver capabilities |
| Producer disconnected | Host-side capture stopped | Broadcast ProducerClosed; browser shows black screen |
| Consumer disconnected | SFU paused proactively | Trigger pause/resume; browser shows buffering state |

### 5. Worker crash (existing)

See "Crash Recovery Flow" above. A worker crash is the ultimate transport-level failure — all transports + producers + consumers are destroyed.

## Transport Security

### DTLS

| Parameter | Default | Notes |
|-----------|---------|-------|
| Fingerprint algorithm | SHA-256 | mediasoup uses SHA-256 certificate fingerprints by default |
| Negotiation mode | ICE-Lite | Simplified SFU-side ICE (server-side only) |
| DTLS role | auto | Auto-negotiated per ICE controlling/controlled role |

### SRTP

| Parameter | Description |
|-----------|-------------|
| Crypto suite | AES_CM_128_HMAC_SHA1_80 (default) |
| Key agreement | SRTP keys derived from the DTLS handshake |
| Replay protection | Enabled (SSRC + ROC counters) |

### Security Hardening Recommendations

1. **ICE credentials**: each transport uses independent ice-ufrag/ice-pwd (mediasoup default behavior)
2. **RTCP feedback**: enable NACK/FIR/PLI for packet-loss recovery, not relying on upper-layer retransmission
3. **Transport auth**: every SFU message carries room_id + peer_id for route-level authentication (PIT-08)
4. **Port range restriction**: narrow mediasoup `rtcPortsRange` to the minimal UDP range (default 40000-40100)
5. **Consumer isolation**: different consumers' RTP streams are mutually invisible; forwarded only inside the mediasoup Router


## Encoding Degradation Strategy

### Degradation Trigger Conditions

| Metric | Threshold | Action |
|--------|-----------|--------|
| Packet loss > 5% | Sustained 3s | Switch to a lower bitrate layer |
| RTT > 300ms | Sustained 5s | Reduce resolution |
| Packet loss > 15% | Sustained 10s | Switch to the lowest layer or pause |
| Bandwidth < 500kbps | Immediate | Stop video, keep audio |

### Bitrate Degradation Steps (720p → 360p → stop)

```
Normal:    720p@30  H.264   2-4 Mbps
 ↓ loss >5%
Degrade 1: 480p@30  H.264   0.8-1.5 Mbps
 ↓ loss >15%
Degrade 2: 360p@15  H.264   0.3-0.5 Mbps
 ↓ bandwidth <500kbps
Degrade 3: stop video, keep audio (<50kbps)
```

### Quality Layer Switching (Simulcast)

| Layer | Resolution | Frame rate | Bitrate | Use |
|-------|------------|------------|---------|-----|
| High | 720p | 30 | 2-4 Mbps | Full-screen playback |
| Medium | 480p | 30 | 0.8-1.5 Mbps | Thumbnail/mid-size |
| Low | 360p | 15 | 0.3-0.5 Mbps | Picture-in-picture/preview |

Switching logic:
1. The SFU detects consumer bandwidth → proactively switches the spatial layer
2. The browser receiver detects packet loss → triggers PLI → the SFU switches
3. Manual switch: the user clicks "low-bandwidth mode" → sends consume specifying the spatial layer

---

## DTLS-SRTP Media-Plane Security

### DTLS Handshake

| Parameter | Default | Notes |
|-----------|---------|-------|
| Fingerprint algorithm | SHA-256 | mediasoup uses SHA-256 certificate fingerprints by default |
| DTLS role | auto | Auto-negotiated per ICE controlling/controlled |
| Negotiation mode | ICE-Lite | Simplified SFU-side ICE (server-side only) |

Handshake flow:
```
Client                          mediasoup (ICE-Lite)
  │                                    │
  │── STUN Binding Request ───────────→│ ICE connectivity check
  │←── STUN Binding Response ─────────│
  │                                    │
  │── DTLS ClientHello ───────────────→│ DTLS handshake begins
  │←── DTLS ServerHello + Certificate │
  │── DTLS ClientKeyExchange ─────────→│
  │←── DTLS Finished ─────────────────│
  │                                    │
  │═══════ SRTP key derived ═══════════│ SRTP keys derived from the DTLS handshake
```

### SRTP Encryption

| Parameter | Value |
|-----------|-------|
| Crypto suite | AES_CM_128_HMAC_SHA1_80 (default) |
| Key agreement | SRTP keys derived from the DTLS handshake |
| Replay protection | Enabled (SSRC + ROC counters) |
| Key rotation | DTLS re-handshake triggers SRTP key update |

### SCTP Encryption (DataChannel)

When the DataChannel is enabled, SCTP over DTLS receives the same DTLS encryption protection. The SCTP association is established after the DTLS handshake completes.

### Host-Side Security

The Host connects through mediaservo-webrtc (webrtc-sys/libwebrtc); libwebrtc handles DTLS-SRTP encryption automatically. No extra Host-side configuration needed.

### Security Hardening Recommendations

1. **ICE credentials**: each transport uses independent ice-ufrag/ice-pwd (mediasoup default)
2. **DTLS fingerprint validation**: the Server validates the client DTLS fingerprint at `transport.connect()`
3. **SRTP replay protection**: enabled by default, preventing RTP packet replay attacks
4. **Port range restriction**: narrow the UDP port range to reduce the attack surface
