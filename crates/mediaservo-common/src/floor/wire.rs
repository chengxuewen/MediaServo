//! Floor wire types — the pure model's emitted signals.
//!
//! P1 keeps the model independent of the server protocol: `FloorSignal` is the
//! reducer output; P2 maps these values onto the additive `SignalingMessage`
//! variants (dialect 4).

use serde::{Deserialize, Serialize};

use super::PeerId;

/// Floor arbitration mode (PTT exclusive / open conference / hybrid).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorMode {
    /// One speaker at a time.
    Exclusive,
    /// Everyone may speak; floor control degenerates to mute governance.
    Open,
    /// Up to the cap simultaneous speakers with queueing above it.
    Hybrid,
}

/// Why a floor request was denied (closed on purpose).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    /// Queue at ceiling.
    Busy,
    /// Would exceed the mode's simultaneous-speaker cap.
    ExceedsCeiling,
    /// Peer not a room member.
    NotMember,
    /// Request rate guard tripped.
    RateLimited,
    /// Preempt attempt without strictly-higher priority.
    PreemptPriority,
    /// Peer has no live media transport.
    NoMedia,
    /// Authorization refused (unauthorized command or a muted peer's request).
    Unauthorized,
}

/// Signals emitted by the pure reducer. P2 maps them onto `SignalingMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloorSignal {
    FloorGranted { grants: Vec<PeerId>, generation: u64 },
    FloorTaken { by: PeerId, generation: u64 },
    FloorDenied { reason: DenyReason, generation: u64 },
    FloorQueued { position: u32, generation: u64 },
    FloorIdle { generation: u64, reason: Option<String> },
}
