//! 控制平面（Task F1）— 控制信封 + 执行器接口。
//!
//! 语义（D-H3 控制通道落地）：
//! - 通道边界 = DC label（chassis/gimbal/light），信封内不重复携带通道名；
//! - 请求 `{seq, cmd, payload}` → 回执 `{ack, result}`（同通道发回）；
//!   `seq` 由发送方单调递增，回执 `ack` 原样回传供对端配对；
//! - 执行器接口 trait 化：F1 为 `StubActuator`（日志 + 回执），CAN/GPIO
//!   实现在 Phase I 后接入（D-H3 本地兜底归 F2）。
//! - 通道可靠性（host-controller 创建）：chassis/light reliable-ordered，
//!   gimbal partial-reliable（D-H3：急停 reliable / 云台 partial-reliable）。

/// 信封类型 T1.3 已提 `mediaservo-common::protocol`（四方单一真源：host 两 bin /
/// TS 镜像 / sig_vector 夹具）——本模块原地 re-export，消费方 import 零改动。
pub use mediaservo_common::protocol::{parse_envelope, ControlAck, ControlEnvelope};

/// 执行器接口 — 按通道路由命令；返回回执 result（Err → `ControlAck::err`）。
/// 实现方必须打日志（C15）；错误信息返回给对端（ACK 语义，非静默）。
pub trait Actuator: Send + Sync {
    fn on_command(
        &self,
        channel: &str,
        env: &ControlEnvelope,
    ) -> Result<serde_json::Value, String>;
}

/// Stub 执行器（F1 阶段）：日志 + 回执 `{"ok": true, "channel": .., "seq": ..}`。
/// CAN/GPIO 真实实现在 Phase I 后替换（接口不变）。
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
            "actuator: stub 执行器收到命令"
        );
        Ok(serde_json::json!({ "ok": true, "channel": channel, "seq": env.seq }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 线形测试（roundtrip/parse/defaults/rejects/ack 形 ×6）已随类型迁
    // common::protocol::tests + sig_vector/sfu 夹具；此处驻执行器语义。

    #[test]
    fn stub_actuator_replies_with_channel_and_seq() {
        let actuator = StubActuator;
        let env = ControlEnvelope {
            seq: 5,
            cmd: "steer".into(),
            payload: serde_json::json!({ "value": -0.2 }),
        };
        let result = actuator.on_command("chassis", &env).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["channel"], "chassis", "回执应回显通道（label 路由证据）");
        assert_eq!(result["seq"], 5);
    }

    #[test]
    fn stub_actuator_distinguishes_channels() {
        let actuator = StubActuator;
        let env = ControlEnvelope { seq: 1, cmd: "pan".into(), payload: serde_json::json!({}) };
        let chassis = actuator.on_command("chassis", &env).unwrap();
        let gimbal = actuator.on_command("gimbal", &env).unwrap();
        assert_eq!(chassis["channel"], "chassis");
        assert_eq!(gimbal["channel"], "gimbal");
    }
}
