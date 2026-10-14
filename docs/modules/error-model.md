# Error Model

> Status: pre-Phase 3 design | Related decisions: D-ERR-01, D-ERR-02 | Created per doc-audit M7

## Error Propagation

```
Host → [FlatBuffers ErrorPayload] → Server → [HTTP status + JSON body] → Client/Remote
```

Errors inside each crate must never leak directly across boundaries. The Component layer's `ComponentError` is the unified error type; when crossing process boundaries it is serialized into the FlatBuffers `ErrorPayload` (error code + context string).

## ComponentError (thiserror)

```rust
#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("initialization failed: {0}")]
    InitFailed(String),
    #[error("RPC timeout after {0}ms")]
    RpcTimeout(u64),
    #[error("channel closed")]
    ChannelClosed,
    #[error("component shutting down")]
    Shutdown,
    #[error("internal error: {0}")]
    Internal(String),
    #[error("not authorized: {0}")]
    Unauthorized(String),
    #[error("configuration invalid: {0}")]
    ConfigInvalid(String),
}
```

## Error Code Layering (D-ERR-02)

| Range | Layer | Examples |
|-------|-------|----------|
| 1xxx | Generic/Component | 1001 NotFound, 1002 InitFailed |
| 2xxx | Transport | 2001 ConnectionRefused, 2002 Timeout |
| 3xxx | Resources | 3001 StreamNotFound, 3002 RoomFull |
| 4xxx | Discovery | 4101 CameraNotFound, 4102 ONVIFAuthFailed, 4103 DiscoveryTimeout |
| 5xxx | Auth/authz | 5001 TokenExpired, 5002 PermissionDenied |
| 6xxx | Media | 6001 CodecUnsupported, 6002 EncoderFailed |

### SFU Media-Plane Errors (40xx, real-time protocol)

> Created per doc-audit L6 (merging three conflicting error-code semantics). The SFU code range is what the **wire signaling protocol actually uses** (signaling.rs); the discovery range has already shifted to 41xx.

| Code | Meaning | Code location |
|------|---------|---------------|
| 4000 | Transport connection failed / Produce requires a send transport | signaling.rs:675 |
| 4001 | Room join failed | signaling.rs:308 |
| 4002 | Room full | signaling.rs:295 |
| 4003 | Auth failure (PSK) — semantically auth-layer; [planned] migrate to 5003 | signaling.rs:215/232 |
| 4099 | SFU internal error — [planned] migrated in from 5000 | signaling.rs:629/636/661/703/726 |

> ⚠️ The 5xxx range note: **5000 is already occupied by SFU internal errors** (5 sites in signaling.rs), conflicting with 5xxx=auth/authz semantics; the migration plan is in the table above (5000→4099, 4003→5003) and needs a signaling-protocol compatibility assessment (browsers route by the type field, PIT-06, so code migration risk is low).

## HTTP Status Mapping

```
ComponentError → gRPC status / HTTP status

NotFound         → 404
InitFailed       → 500
RpcTimeout       → 504
ChannelClosed    → 503
Shutdown         → 503
Internal(_)      → 500
Unauthorized(_)  → 401
ConfigInvalid(_) → 400
```

## Logging Conventions

- **ERROR**: InitFailed, Internal — requires human intervention
- **WARN**: RpcTimeout, ChannelClosed — retryable recovery
- **INFO**: Shutdown, NotFound — normal operation flow
- **Must include**: `component_id`, `request_id`, context

## Runtime Error Scenarios

### Signal disconnect
- After a WebSocket disconnect, existing WebRTC connections are kept; ICE restart attempts to recover signaling.
- Timeout 30s → close the connection.
- Error code: 2004 SignalingDisconnected

### Transport disconnect (ICE)
- Graceful close (normal hang up): error code 2005 ConnectionClosed.
- Abnormal disconnect (network interruption): error code 2006 ConnectionLost.
- ICE restart attempts recovery automatically.

### Discovery timeout
- ONVIF/RTSP device discovery times out at 10s.
- Error code: 4103 DiscoveryTimeout

### Codec mismatch
- SDP negotiation finds no common codec → error code 3005 CodecNegotiationFailed.

### Config rollback barrier
- A new config crashes a component → roll back to the old config.
- A second failure → enter safe mode (minimal feature set).
- Error code: 9004 ConfigRollbackFailed

## Backpressure & Overflow Policy

### Encode queue full (D-ERR-03)
- Ring buffer, drop the oldest frame, keep keyframes.
- EncoderInputDropped counter.

### Push send buffer full (D-ERR-04)
- Bounded mpsc channel (cap 32); overflow → drop the oldest P frame.
- Signal lag counter.

### Backpressure propagation
- Sink → Processor → Source, via bounded channel capacities.
> The backpressure propagation design is implemented (D127-D134 Component architecture, consolidated-mvp plan)
