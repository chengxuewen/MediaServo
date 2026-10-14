//! Branded IDs (prevent cross-argument mix-ups).

/// Node ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Frame topic (e.g. `"camera/front/raw"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FrameTopic(String);

impl FrameTopic {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Wildcard match: pattern `"camera/*"` matches `"camera/front/raw"`; exact match also holds.
    pub fn matches(&self, pattern: &str) -> bool {
        if let Some(pfx) = pattern.strip_suffix("/*") {
            self.0.starts_with(&format!("{pfx}/"))
        } else {
            self.0 == pattern
        }
    }
}
