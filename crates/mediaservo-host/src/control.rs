//! Control plane (Task F1) -- control envelope + actuator interface.
//!
//! Semantics (D-H3 control channel landing):
//! - Channel boundary = DC label (chassis/gimbal/light); the envelope does not repeat the channel name;
//! - Request `{seq, cmd, payload}` -> receipt `{ack, result}` (sent back on the same channel);
//!   `seq` increments monotonically at the sender; the receipt `ack` echoes it back for the peer to pair;
//! - Actuator interface is a trait: F1 is `StubActuator` (log + receipt); CAN/GPIO
//!   implementations land after Phase I (D-H3 local fallback goes to F2).
//! - Channel reliability (created by host-controller): chassis/light reliable-ordered,
//!   gimbal partial-reliable (D-H3: e-stop reliable / gimbal partial-reliable).

/// Envelope types were promoted to `mediaservo-common::protocol` in T1.3 (single source of truth across four parties: host's two bins /
/// TS mirror / sig_vector fixtures) -- this module re-exports them in place, so consumers need no import change.
pub use mediaservo_common::protocol::{ControlAck, ControlEnvelope, parse_envelope};

/// Actuator interface -- routes commands per channel; returns a receipt result (Err -> `ControlAck::err`).
/// Implementations must log (C15); error info goes back to the peer (ACK semantics, never silent).
pub trait Actuator: Send + Sync {
    fn on_command(&self, channel: &str, env: &ControlEnvelope)
    -> Result<serde_json::Value, String>;
}

/// Stub actuator (F1 stage): log + receipt `{"ok": true, "channel": .., "seq": ..}`.
/// Real CAN/GPIO implementations replace it after Phase I (interface unchanged).
pub struct StubActuator;

impl Actuator for StubActuator {
    fn on_command(
        &self,
        channel: &str,
        env: &ControlEnvelope,
    ) -> Result<serde_json::Value, String> {
        tracing::info!(
            channel,
            cmd = %env.cmd,
            seq = env.seq,
            payload = %env.payload,
            "actuator: stub actuator received command"
        );
        Ok(serde_json::json!({ "ok": true, "channel": channel, "seq": env.seq }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wire-shape tests (roundtrip/parse/defaults/rejects/ack form x6) moved along with the types
    // into common::protocol::tests + sig_vector/sfu fixtures; only actuator semantics stay here.

    #[test]
    fn stub_actuator_replies_with_channel_and_seq() {
        let actuator = StubActuator;
        let env = ControlEnvelope {
            seq: 5,
            cmd: "steer".into(),
            payload: serde_json::json!({ "value": -0.2 }),
            sig: None,
        };
        let result = actuator.on_command("chassis", &env).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(
            result["channel"], "chassis",
            "receipt should echo the channel (label routing evidence)"
        );
        assert_eq!(result["seq"], 5);
    }

    #[test]
    fn stub_actuator_distinguishes_channels() {
        let actuator = StubActuator;
        let env = ControlEnvelope {
            seq: 1,
            cmd: "pan".into(),
            payload: serde_json::json!({}),
            sig: None,
        };
        let chassis = actuator.on_command("chassis", &env).unwrap();
        let gimbal = actuator.on_command("gimbal", &env).unwrap();
        assert_eq!(chassis["channel"], "chassis");
        assert_eq!(gimbal["channel"], "gimbal");
    }
}
