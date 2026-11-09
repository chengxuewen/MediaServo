//! Floor control — the pure speaking-right model (P1).
//!
//! `state` is the deterministic reducer; `wire` holds the emitted signal types.
//! Protocol mapping (`SignalingMessage` variants) and server integration land in P2.

pub mod state;
pub mod wire;

/// MediaServo peer ids are strings (the room/conn registry owns uniqueness).
pub type PeerId = String;

pub use state::{FloorEvent, FloorLimits, FloorState, Pending};
pub use wire::{DenyReason, FloorMode, FloorSignal};
