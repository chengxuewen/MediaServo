//! Static ACL (D237: role presets + node overrides; denial audit log).

use crate::id::{FrameTopic, NodeId};

/// Node role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Role {
    /// Capture node: publishes camera frames.
    Capture,
    /// Processor node: subscribes camera frames, publishes derived topics (e.g. stitched).
    Processor,
    /// Pusher node: subscribes frames, pushes WebRTC, publishes own stats (stats/*).
    Pusher,
    /// Cockpit puller (no permission on the local bus).
    Puller,
    /// Recorder node: subscribes frames, records.
    Recorder,
    /// Control node: publishes control commands, subscribes telemetry/status.
    Control,
    /// Perception node: publishes perception results, subscribes camera frames.
    Perception,
    /// Monitor node (host-agent data-plane monitoring, E2): subscribes camera frames + push stats, never publishes.
    Monitor,
}

/// Node ACL (static, D237).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeAcl {
    pub node_id: NodeId,
    pub role: Role,
    /// Wildcard patterns of publishable topics (e.g. `"camera/*"`).
    pub publish_allow: Vec<String>,
    /// Wildcard patterns of subscribable topics.
    pub subscribe_allow: Vec<String>,
}

impl NodeAcl {
    /// D237 permission-matrix presets.
    pub fn for_role(node_id: NodeId, role: Role) -> Self {
        let (publish_allow, subscribe_allow): (Vec<String>, Vec<String>) = match role {
            Role::Capture => (vec!["camera/*".into()], vec![]),
            Role::Processor => (vec!["video/*".into()], vec!["camera/*".into()]),
            Role::Pusher => (
                vec!["stats/*".into()],
                vec!["camera/*".into(), "video/*".into(), "vision/*".into()],
            ),
            Role::Puller => (vec![], vec![]),
            // E2 push-status reporting: Recorder subscribes frames + publishes stats/*
            // F3: subscribe vision results (vision/<camera-id>, D-H8 chain)
            Role::Recorder => (
                vec!["stats/*".into()],
                vec!["camera/*".into(), "video/*".into(), "vision/*".into()],
            ),
            Role::Control => (
                // S1 bus mirror: controller publishes cmd+ack topics; consumers (ROS nodes/replay) subscribe cmd.
                vec!["control/cmd".into(), "control/ack".into()],
                vec!["control/cmd".into(), "control/telemetry".into(), "status/*".into()],
            ),
            // F3: ROS vision node publishes vision/<camera-id> (D-H7/D-H8, single-source bridge config)
            Role::Perception => {
                (vec!["perception/*".into(), "vision/*".into()], vec!["camera/*".into()])
            }
            Role::Monitor => (vec![], vec!["camera/*".into(), "stats/*".into()]),
        };
        Self { node_id, role, publish_allow, subscribe_allow }
    }

    /// Whether publishing this topic is allowed; denials write an audit log (D237 + C15).
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

    /// Whether subscribing this topic is allowed; denials write an audit log (D237 + C15).
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
