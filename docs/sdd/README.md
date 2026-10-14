# SDD Spec Index — MediaServo Phase 1

> Spec-Driven Development spec documents. Each SDD covers one core module, including interface definition, error handling, and test plan.

## Index

| # | Module | File | Status | Key Decision Refs | Dependencies |
|---|--------|------|--------|-------------------|--------------|
| 1 | CameraCapture | `01-camera-capture.md` | ✅ Defined | D64, D75, D-ERR-01 | mediaservo-core |
| 2 | WebRTC Push | `02-webrtc-push.md` | ✅ Defined | D11, D32, D-ERR-01 | Transport trait |
| 3 | Decode + Render | `03-decode-render.md` | ✅ Defined | D46, D47, D48, D75 | Codec crate |
| 4 | DataChannel Control | `04-datachannel-control.md` | ✅ Defined | D65, D66, D117 | WebRTC plugin |
| 5 | Host Web Config | `05-host-web-config.md` | ✅ Defined | D81, D84, D85, D114 | axum |
| 6 | Server Monitoring | `06-server-monitoring.md` | ✅ Defined | D86-D91, D99, D-OPS-09, D111 | prometheus-client |
| 7 | Emergency Stop | `07-emergency-stop.md` | ✅ Defined | D117, D-SAFETY-02 | UDP |

## Cross-Module References

```
CameraCapture ──I420──▶ WebRTC Push ──RTP──▶ Server Relay
                                                  │
                                                  ▼
EmergencyStop ◀──UDP── DataChannel ◀──WebRTC── DecodeRender
                              │
                              ▼
                    Host Web Config (SSE)
```

## Conventions

- Interface definitions use Rust pseudocode (trait + struct signatures)
- Error codes follow the D-ERR-02 coding scheme (1xxx-9xxx)
- Test plans follow the 4-layer system: Unit → Integration → E2E → On-vehicle (D113)
- Latency budget: see D115 MVP pipeline latency breakdown
- Security: see D116 STRIDE-Lite threat model
