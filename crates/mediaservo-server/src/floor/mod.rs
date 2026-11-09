//! Floor-control server module (plan talkservo-port P2).
//!
//! - `registry`: per-connection delivery (R13)
//! - `policy`: per-room switch and limits (R3/R16)
//! - `room`: the per-room single-writer reducer wrapper
//!
//! Signaling/SFU wiring lands next: command dispatch, produce pause/resume,
//! audit, timers and resync.

pub mod policy;
pub mod registry;
pub mod room;

pub use policy::FloorPolicy;
pub use registry::{ConnEntry, ConnRegistry};
pub use room::FloorRoom;
