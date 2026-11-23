//! Floor policy — the per-room switch and limits (R3/R16; SB-2 = J3=C two-tier config).
//!
//! The default is OFF: existing `audio-*` rooms stay plain conferences until a
//! room is explicitly enabled.
//!
//! Limit resolution (SB-2, ruling J3=C — global env defaults + per-room override):
//!   FloorLimits::default()            design baseline (hybrid 2 / queue 8 / hold 30 s)
//!   <- MEDIASERVO_FLOOR_{MAX_PEERS,QUEUE_MAX,HOLD_SECS}   global env overlay
//!   <- ManagedRoom.floor (rooms.yaml)                     per-room overlay
//! The resolved value is baked into the room actor (`FloorRoom::new`).

use mediaservo_common::floor::FloorLimits;

/// Per-room overrides carried in rooms.yaml (`ManagedRoom.floor`). Absent
/// fields keep the resolved global value; an absent `floor` keeps it all —
/// old rooms.yaml files load unchanged (backward compatible by construction).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FloorOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_peers: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queue: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_timeout_secs: Option<u64>,
}

impl FloorOverride {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn overlay(self, mut limits: FloorLimits) -> FloorLimits {
        if let Some(v) = self.max_peers {
            limits.max_peers = v;
        }
        if let Some(v) = self.max_queue {
            limits.max_queue = v;
        }
        if let Some(v) = self.hold_timeout_secs {
            limits.max_hold_secs = v;
        }
        limits
    }
}

/// Global floor limits: design baseline + env overlay. Parsed once at startup.
#[derive(Debug, Clone, Copy)]
pub struct GlobalFloorConfig {
    pub limits: FloorLimits,
}

impl Default for GlobalFloorConfig {
    fn default() -> Self {
        Self { limits: FloorLimits::default() }
    }
}

/// Pure env parser (testable without process-env races). `get` is `env::var`.
fn parse_env(get: impl Fn(&str) -> Result<String, std::env::VarError>) -> FloorLimits {
    let mut limits = FloorLimits::default();
    if let Ok(v) = get("MEDIASERVO_FLOOR_MAX_PEERS") {
        if let Ok(n) = v.parse::<usize>() {
            limits.max_peers = n;
        }
    }
    if let Ok(v) = get("MEDIASERVO_FLOOR_QUEUE_MAX") {
        if let Ok(n) = v.parse::<usize>() {
            limits.max_queue = n;
        }
    }
    if let Ok(v) = get("MEDIASERVO_FLOOR_HOLD_SECS") {
        if let Ok(n) = v.parse::<u64>() {
            limits.max_hold_secs = n;
        }
    }
    limits
}

impl GlobalFloorConfig {
    pub fn from_env() -> Self {
        Self { limits: parse_env(|k| std::env::var(k)) }
    }
}

/// Two-tier resolution: global (design+env) overlaid with the room's override.
pub fn resolve_limits(global: FloorLimits, room: Option<FloorOverride>) -> FloorLimits {
    match room {
        Some(o) => o.overlay(global),
        None => global,
    }
}

/// Per-room floor policy. The resolved value carried by the room actor.
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
        // SB-2 design defaults (J3=C): hybrid 2 / queue 8 / hold 30 s
        assert_eq!(p.limits.max_peers, 2);
        assert_eq!(p.limits.max_queue, 8);
        assert_eq!(p.limits.max_hold_secs, 30);
    }

    #[test]
    fn enabled_keeps_the_dispatch_defaults() {
        let p = FloorPolicy::enabled();
        assert!(p.is_enabled());
        assert_eq!(p.limits.max_peers, 2);
        assert_eq!(p.limits.max_queue, 8);
    }

    #[test]
    fn env_overlay_overrides_design_defaults() {
        let vars = [
            ("MEDIASERVO_FLOOR_MAX_PEERS", "3"),
            ("MEDIASERVO_FLOOR_QUEUE_MAX", "12"),
            ("MEDIASERVO_FLOOR_HOLD_SECS", "60"),
        ];
        let get = |k: &str| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| Ok(v.to_string()))
                .unwrap_or_else(|| Err(std::env::VarError::NotPresent))
        };
        let l = parse_env(get);
        assert_eq!((l.max_peers, l.max_queue, l.max_hold_secs), (3, 12, 60));
    }

    #[test]
    fn env_garbage_keeps_design_defaults() {
        let get = |_k: &str| Ok("not-a-number".to_string());
        let l = parse_env(get);
        assert_eq!((l.max_peers, l.max_queue, l.max_hold_secs), (2, 8, 30));
    }

    #[test]
    fn room_override_layers_only_present_fields() {
        let global = FloorLimits::default();
        let resolved = resolve_limits(
            global,
            Some(FloorOverride { max_queue: Some(4), ..Default::default() }),
        );
        assert_eq!(resolved.max_queue, 4); // overridden
        assert_eq!(resolved.max_peers, 2); // kept from global
        assert_eq!(resolved.max_hold_secs, 30); // kept from global
        assert_eq!(resolve_limits(global, None), global); // no override = global
    }
}
