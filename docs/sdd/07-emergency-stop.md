# SDD 07: Emergency Stop

## 1. Overview

Emergency stop module. A UDP emergency stop channel independent of WebRTC, ensuring the vehicle can still be stopped safely when the main communication path fails.

**Decision refs**: D117 (independent emergency stop channel), D-SAFETY-02 (SafetyEnvelope), D116 (STRIDE-Lite)

## 2. Interface Definition

```rust
/// Emergency stop channel (standalone UDP)
pub trait EmergencyStop: Send + Sync {
    fn bind(&mut self, addr: SocketAddr) -> Result<()>;
    fn send_stop(&self) -> Result<()>;
    fn poll_stop(&mut self, timeout: Duration) -> Result<Option<()>>;
    fn last_heartbeat(&self) -> Option<Instant>;
}

/// Heartbeat frame (sent periodically by the vehicle controller)
pub struct Heartbeat {
    pub seq: u32,
    pub timestamp: Instant,
    pub state: ControlState,
}
```

### SafetyEnvelope trait (D-SAFETY-02)

```rust
pub trait SafetyEnvelope: Send {
    fn check(&self, state: &ControlState) -> SafetyLevel;
    fn limits(&self) -> &ControlLimits;
}

pub enum SafetyLevel {
    Normal,
    Warning,
    Limit,
    SoftStop,   // Decelerate to zero within 2s
    HardStop,   // Cut power immediately
}

pub struct ControlLimits {
    pub max_steering_angle: f32,
    pub max_speed: f32,
    pub max_accel: f32,
    pub timeout_ms: u32,
    pub rtt_warning_ms: u32,
    pub rtt_emergency_ms: u32,
}
```

## 3. Architecture

```
Remote                   Vehicle Controller (ECU)
  │                         │
  ├── WebRTC DataChannel ──▶│ (primary control path)
  │                         │
  ├── UDP emergency stop ──▶│ (independent path)
  │   0xFF byte            │
  │   QoS DSCP EF         │
  │                         │
  │              heartbeat ◀─── (150ms period)
```

### Key Design

| Property | Value |
|----------|-------|
| Transport | Raw UDP, no WebRTC stack |
| Data | Single byte `0xFF` = emergency stop |
| Port | Dedicated UDP port (distinct from the DataChannel) |
| QoS | DSCP EF (46) priority forwarding |
| Retry | 3 sends, 50ms interval |
| Target latency | <20ms |

### Timeout Tiering (D-SAFETY-02)

```
L0 Normal    (RTT < 100ms)  → Normal control
L1 Warning   (RTT 100ms+)   → Execute allowed, alert raised
L2 Limit     (RTT 150ms+)   → Speed/steering limited
L3 SoftStop  (RTT 300ms+)   → Decelerate to zero within 2s
L4 HardStop  (RTT 500ms+)   → Cut power + UDP emergency stop
```

## 4. Independence Verification

| Failure Scenario | WebRTC DataChannel | UDP Emergency Stop | Result |
|------------------|--------------------|--------------------|--------|
| WebRTC down | ❌ | ✅ | Emergency stop still available |
| DTLS handshake failure | ❌ | ✅ | Emergency stop still available |
| 4G weak network, high latency | ❌ (loss) | ✅ | Emergency stop still available |
| WebRTC process crash | ❌ | ✅ (direct to ECU) | Emergency stop still available |
| UDP port unreachable | ✅ | ❌ | Timeout triggers SoftStop |

## 5. Error Handling

| Condition | Category | Recovery |
|-----------|----------|----------|
| UDP packet loss (single) | Transient | Retry 3 times, 50ms interval |
| Consecutive UDP loss | Recoverable | Vehicle-controller heartbeat timeout auto-triggers SoftStop |
| Heartbeat timeout 150ms | Recoverable | Automatic SoftStop |
| Heartbeat timeout 500ms | Fatal | Automatic HardStop |

## 6. Test Plan

| Test | Type | Description |
|------|------|-------------|
| stop_latency | Integration | UDP emergency stop end-to-end <20ms |
| webrtc_independence | Integration | UDP stop still works when WebRTC is down |
| heartbeat_timeout_soft | Integration | 150ms without heartbeat triggers SoftStop |
| heartbeat_timeout_hard | Integration | 500ms without heartbeat triggers HardStop |
| safety_level_escalation | Unit | SafetyLevel escalates correctly as RTT increases |
| control_limits_apply | Unit | Limit level clamps speed/angle correctly |
| retry_3_times | Integration | UDP loss auto-retries 3 times |
| dscp_ef_marking | Integration | Emergency stop packets carry DSCP field 46 |
| safety_envelope_vehicle | On-vehicle | Vehicle-controller SafetyEnvelope verdicts consistent |
