# Testing Strategy

> Status: pre-Phase 3 design | Related decisions: D113 (4-layer), D-TEST-01 | Created per doc-audit M6

## The 4-Layer Test Architecture (D113)

```
Layer 1: Unit         → per-crate #[cfg(test)] + #[test]
Layer 2: Component    → in-crate integration tests, dependencies mocked
Layer 3: Integration  → multi-crate composition, real transport
Layer 4: E2E          → Host→Server→Remote full chain
```

Current test distribution (2026-07-20): 147 workspace tests. Unit: core 58, webrtc 21 (w3c-api). Integration: webrtc-sys loopback 13, host E2E 21, server 37, remote 13, PipelineEngine 11.

### Layer 1: Unit

- Location: `#[cfg(test)] mod tests` inside `crates/*/src/**/*.rs`
- Coverage: pure functions, type conversions, serialization, config parsing
- Framework: `#[test]` + `assert_eq!`, no external dependencies

### Layer 2: Component

- Location: `crates/*/tests/component_*.rs`
- Coverage: integration of modules within a single crate (pipeline chains, signaling state machine, codec wrappers)
- Fixtures:
  - `MockTrackLocal` — mocks a local video track, injecting fixed frames
  - `LoopbackTransport` — in-memory channels simulating RTP send/receive
  - `TestConfig` — inline toml, no filesystem dependency

### Layer 3: Integration

- Location: `crates/mediaservo-server/tests/integration_*.rs` (the server is the coordination point)
- Scenarios:
  - Host registers → Server confirms → Remote discovers the Host list
  - RTP relay loop: Host → Server → Remote → Server → Host
  - Disconnect/reconnect: kill a transport → automatic reconnect + session resume
- Requires: tokio runtime + real WebSocket/QUIC transport (localhost)

### Layer 4: E2E

- Location: `tests/e2e/` (workspace root)
- Scenarios:
  - Remote desktop full chain: Host captures mock frames → Remote renders mock frames
  - Multi-party conference: 1 Server + 3 Hosts pushing + 3 Remotes pulling
  - mediasoup SFU multi-peer: 4+ concurrent producers/consumers
- Infrastructure: docker-compose orchestration, Playwright verifying the Remote GUI

## Coverage Targets (D-TEST-01)

| Crate | Unit | Component | Total |
|-------|------|-----------|-------|
| mediaservo-host | 60% | 80% | 80% |
| mediaservo-client | 50% | 75% | 75% |
| mediaservo-server | 60% | 80% | 80% |
| mediaservo-common (Phase 2) | 70% | 85% | 85% |

Tooling: `cargo tarpaulin --workspace --out xml`

## SDD Traceability

Every SDD acceptance criterion maps to a `#[test]`. Naming: `test_{sdd_id}_{scenario}`. AAA pattern (Arrange/Act/Assert) enforced.

> Implementation complete (Phase 3)
