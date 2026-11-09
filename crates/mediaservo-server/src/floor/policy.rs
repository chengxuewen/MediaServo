//! Floor policy — the per-room switch and limits (R3/R16).
//!
//! The default is OFF: existing `audio-*` rooms stay plain conferences until a
//! room is explicitly enabled.

use mediaservo_common::floor::FloorLimits;

/// Per-room floor policy. Config resolution (env > server.yaml > runtime
/// override) lands with the server integration; this struct is the resolved
/// value carried by the room actor.
#[derive(Debug, Clone)]
pub struct FloorPolicy {
    pub enabled: bool,
    /// Whether legacy (non-account) sessions may take the floor. Default false.
    pub allow_legacy: bool,
    pub limits: FloorLimits,
}

impl Default for FloorPolicy {
    fn default() -> Self {
        Self { enabled: false, allow_legacy: false, limits: FloorLimits::default() }
    }
}

impl FloorPolicy {
    pub fn enabled() -> Self {
        Self { enabled: true, ..Self::default() }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off_and_legacy_is_closed() {
        let p = FloorPolicy::default();
        assert!(!p.is_enabled());
        assert!(!p.allow_legacy);
        assert_eq!(p.limits.max_peers, 1);
    }

    #[test]
    fn enabled_keeps_the_dispatch_defaults() {
        let p = FloorPolicy::enabled();
        assert!(p.is_enabled());
        assert_eq!(p.limits.max_peers, 1);
        assert_eq!(p.limits.max_queue, 16);
    }
}
