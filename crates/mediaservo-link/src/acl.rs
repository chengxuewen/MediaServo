//! 静态 ACL（D237：role 预置 + 节点覆盖；deny 审计日志）。

use crate::id::{FrameTopic, NodeId};

/// 节点角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Role {
    /// 出图节点：发布相机帧。
    Capture,
    /// 处理节点：订阅相机帧、发布派生 topic（如拼接）。
    Processor,
    /// 推流节点：订阅帧、推 WebRTC、发布自身 stats（stats/*）。
    Pusher,
    /// 舱端拉流（本地总线无权限）。
    Puller,
    /// 录制节点：订阅帧、录制。
    Recorder,
    /// 控制节点：发布控制指令、订阅遥测/状态。
    Control,
    /// 感知节点：发布感知结果、订阅相机帧。
    Perception,
    /// 监控节点（host-agent 数据面监控，E2）：订阅相机帧 + 推流状态，不发布。
    Monitor,
}

/// 节点 ACL（静态，D237）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeAcl {
    pub node_id: NodeId,
    pub role: Role,
    /// 允许发布的 topic 通配模式（如 `"camera/*"`）。
    pub publish_allow: Vec<String>,
    /// 允许订阅的 topic 通配模式。
    pub subscribe_allow: Vec<String>,
}

impl NodeAcl {
    /// D237 权限矩阵预置。
    pub fn for_role(node_id: NodeId, role: Role) -> Self {
        let (publish_allow, subscribe_allow): (Vec<String>, Vec<String>) = match role {
            Role::Capture => (vec!["camera/*".into()], vec![]),
            Role::Processor => (vec!["video/*".into()], vec!["camera/*".into()]),
            Role::Pusher => (vec!["stats/*".into()], vec!["camera/*".into(), "video/*".into(), "vision/*".into()]),
            Role::Puller => (vec![], vec![]),
            // E2 推流状态上报: Recorder 订阅帧 + 发布 stats/*
            // F3: 订阅视觉结果（vision/<camera-id>，D-H8 链路）
            Role::Recorder => (vec!["stats/*".into()], vec!["camera/*".into(), "video/*".into(), "vision/*".into()]),
            Role::Control => (
                // S1 总线镜像：controller 发布 cmd/ack 双 topic；消费侧（ROS 节点/回放）订阅 cmd。
                vec!["control/cmd".into(), "control/ack".into()],
                vec!["control/cmd".into(), "control/telemetry".into(), "status/*".into()],
            ),
            // F3: ROS 视觉节点发布 vision/<camera-id>（D-H7/D-H8，桥接配置单一来源）
            Role::Perception => (vec!["perception/*".into(), "vision/*".into()], vec!["camera/*".into()]),
            Role::Monitor => (vec![], vec!["camera/*".into(), "stats/*".into()]),
        };
        Self { node_id, role, publish_allow, subscribe_allow }
    }

    /// 是否允许发布该 topic；越权记审计日志（D237 + C15）。
    pub fn can_publish(&self, topic: &FrameTopic) -> bool {
        let ok = self.publish_allow.iter().any(|p| topic.matches(p));
        if !ok {
            tracing::warn!(
                node = %self.node_id.as_str(),
                topic = %topic.as_str(),
                "ACL deny publish"
            );
        }
        ok
    }

    /// 是否允许订阅该 topic；越权记审计日志（D237 + C15）。
    pub fn can_subscribe(&self, topic: &FrameTopic) -> bool {
        let ok = self.subscribe_allow.iter().any(|p| topic.matches(p));
        if !ok {
            tracing::warn!(
                node = %self.node_id.as_str(),
                topic = %topic.as_str(),
                "ACL deny subscribe"
            );
        }
        ok
    }
}
