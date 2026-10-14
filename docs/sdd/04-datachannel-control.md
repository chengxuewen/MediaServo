# SDD 04: DataChannel Control

## 1. Overview

DataChannel control module. Transmits control commands (keyboard, mouse, touch) bidirectionally over the WebRTC DataChannel, with HMAC security signatures.

**Decision refs**: D65 (field DataChannel), D66 (remote DataChannel), D117 (control security)

## 2. Interface Definition

```rust
/// Control command type (binary protocol)
pub enum ControlCommand {
    // Keyboard
    KeyDown { key: KeyCode, modifiers: Modifiers },
    KeyUp { key: KeyCode },
    // Mouse
    MouseMove { x: f32, y: f32 },
    MouseDown { button: MouseButton },
    MouseUp { button: MouseButton },
    MouseWheel { delta: f32 },
    // Touch
    Touch { id: u32, x: f32, y: f32, phase: TouchPhase },
    // Teleoperation-specific
    Steering(f32),         // -1.0 ~ 1.0
    Throttle(f32),         // 0.0 ~ 1.0
    Brake(f32),            // 0.0 ~ 1.0
    // System
    Heartbeat(u64),
    EmergencyStop,
}
```

### Binary Frame Format

```
| length: u32 (4) | seq: u32 (4) | cmd: u8 (1) | payload (var) | hmac: [u8; 8] |
```

- `length`: Total frame length (including hmac)
- `seq`: Monotonically increasing sequence number, anti-replay
- `cmd`: ControlCommand enum tag
- `payload`: Command-specific parameters (bincode-serialized)
- `hmac`: 8-byte truncated HMAC-SHA256

### DataChannel Configuration

```
unordered: true
maxRetransmits: 0
label: "control"
```

## 3. HMAC Security (D117)

- Key: `DTLS-SRTP export_keying_material("mediaservo-control")`
- Algorithm: HMAC-SHA256, truncated to first 8 bytes
- Receiver: verifies HMAC, sequence number strictly increasing, seq > last_seq + window
- Sender: when queue > 3 frames, drops the oldest and sends only the newest

```rust
pub fn sign_command(key: &[u8], seq: u32, cmd: &ControlCommand) -> Vec<u8>;
pub fn verify_command(key: &[u8], data: &[u8]) -> Result<ControlCommand>;
```

## 4. Emergency Stop Channel (D117)

A UDP emergency stop path independent of the DataChannel:

```rust
pub trait EmergencyControl: Send {
    fn bind(&mut self, addr: SocketAddr) -> Result<()>;
    fn send_stop(&self) -> Result<()>;
    fn poll_stop(&mut self) -> Result<Option<()>>;
}
```

- Protocol: raw UDP, no WebRTC stack
- Data: single byte `0xFF` = emergency stop
- Target latency: <20ms
- Standalone listener on the vehicle controller, bypassing the Host process (Phase 2)

## 5. Error Handling

| Condition | Category | Error Code | Recovery |
|-----------|----------|------------|----------|
| HMAC verification failure | Recoverable | 4002 | Drop the command, log a warning |
| Out-of-order sequence number | Recoverable | 4002 | Drop commands with older seq |
| DataChannel disconnect | Recoverable | 1003 | Wait for WebRTC recovery |
| Emergency stop UDP packet loss | Transient | - | Resend 3 times at 50ms intervals |

## 6. Test Plan

| Test | Type | Description |
|------|------|-------------|
| hmac_sign_verify | Unit | Verification passes after signing, fails after tampering |
| seq_replay_protection | Unit | Commands with old seq are dropped |
| control_latency | Integration | End-to-end control latency <50ms |
| queue_overflow_drop | Unit | Oldest frame dropped when queue > 3 frames |
| emergency_stop_latency | Integration | UDP emergency stop <20ms |
| emergency_independence | Integration | UDP stop still works when WebRTC is down |
| keyboard_mouse_roundtrip | E2E | Key press → send → receive → event injection |
