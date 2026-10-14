//! Node registry/discovery (D235: register-on-attach, decentralized, no daemon).
//!
//! Phase 1: process-local registry + active-publisher tracking.
//! Cross-process node metadata sharing in Phase 2; single publisher enforced by iceoryx2 `max_publishers(1)`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::acl::Role;
use crate::error::LinkError;
use crate::id::{FrameTopic, NodeId};
use crate::token::Claims;

/// Node self-describing info.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: NodeId,
    pub role: Role,
    /// Publishable topic patterns (from the ACL, e.g. `"camera/*"`).
    pub publishes: Vec<String>,
    /// Subscribed topic patterns.
    pub subscribes: Vec<String>,
}

impl NodeInfo {
    /// Build from token claims (register-on-attach).
    pub fn from_claims(claims: &Claims) -> Self {
        Self {
            id: NodeId::new(claims.node_id.clone()),
            role: claims.role,
            publishes: claims.acl.publish_allow.clone(),
            subscribes: claims.acl.subscribe_allow.clone(),
        }
    }
}

/// Topic discovery info.
#[derive(Debug, Clone)]
pub struct TopicInfo {
    pub topic: FrameTopic,
    pub publisher: NodeId,
}

/// Registry (decentralized, no daemon, D235).
pub struct Registry;

static NODES: OnceLock<Mutex<HashMap<NodeId, NodeInfo>>> = OnceLock::new();
/// Active publishers: topic -> node currently publishing it (single-publisher check).
static PUBLISHERS: OnceLock<Mutex<HashMap<FrameTopic, NodeId>>> = OnceLock::new();

fn nodes() -> &'static Mutex<HashMap<NodeId, NodeInfo>> {
    NODES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn publishers() -> &'static Mutex<HashMap<FrameTopic, NodeId>> {
    PUBLISHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_err() -> LinkError {
    LinkError::Registry("registry lock poisoned".into())
}

impl Registry {
    /// Register a node (register-on-attach; called by `FrameBus::attach`).
    pub fn register(info: &NodeInfo) -> Result<(), LinkError> {
        nodes().lock().map_err(|_| lock_err())?.insert(info.id.clone(), info.clone());
        Ok(())
    }

    /// Unregister a node (called by `FrameBus::close`, best effort).
    pub fn unregister(id: &NodeId) -> Result<(), LinkError> {
        nodes().lock().map_err(|_| lock_err())?.remove(id);
        publishers().lock().map_err(|_| lock_err())?.retain(|_, n| n != id);
        Ok(())
    }

    /// Record an active publisher (called after `FrameBus::publish` succeeds).
    pub fn mark_publisher(topic: &FrameTopic, id: &NodeId) -> Result<(), LinkError> {
        publishers().lock().map_err(|_| lock_err())?.insert(topic.clone(), id.clone());
        Ok(())
    }

    /// Discover `(topic, publisher)` for topics published under the given prefix.
    ///
    /// Phase 1: process-local registry; topics come from node-declared publish patterns.
    pub fn discover_topics(prefix: &str) -> Result<Vec<TopicInfo>, LinkError> {
        let reg = nodes().lock().map_err(|_| lock_err())?;
        let mut out = Vec::new();
        for info in reg.values() {
            for p in &info.publishes {
                if p.starts_with(prefix) {
                    out.push(TopicInfo {
                        topic: FrameTopic::new(p.clone()),
                        publisher: info.id.clone(),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Discover nodes with a given role (Phase 1: process-local registry).
    pub fn discover_nodes(role: Role) -> Result<Vec<NodeInfo>, LinkError> {
        let reg = nodes().lock().map_err(|_| lock_err())?;
        Ok(reg.values().filter(|i| i.role == role).cloned().collect())
    }

    /// **Active** publisher of a topic (single-publisher check; cross-process enforced by iceoryx2 `max_publishers(1)`).
    pub fn topic_publisher(topic: &FrameTopic) -> Result<Option<NodeId>, LinkError> {
        Ok(publishers().lock().map_err(|_| lock_err())?.get(topic).cloned())
    }
}
