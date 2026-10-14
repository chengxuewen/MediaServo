//! MediaServo Field — composition SDK.
//!
//! A one-line dependency for the full loop: signaling + frame bus + capture/record/playback + WebRTC abstraction.
//! - Comms: `SignalClient`/`FrameBus` (re-export of link)
//! - Media: `MediaDevices`/`CameraSource` (re-export of deck)
//! - Sessions: `PushSession`/`PullSession` (push/pull facades; Phase 2 wires the real webrtc transport)
//!
//! Dependency direction (C21 one-way acyclic): `field → webrtc + link + deck`.

pub mod config;
pub mod error;
pub mod session;
pub mod sfu;

pub use config::{PresetBundle, PublishOptions, PullConfig, PushConfig, StreamMode};
pub use error::FieldError;

pub use session::{PullSession, PushSession, SessionEvent, SessionEvents};

// ── Composition re-exports (contract §4: one-line dependency loop) ────────
pub use mediaservo_deck::{
    CameraSource, CaptureOptions, Container, DeviceId, MediaDeviceKind, MediaDevices, Player,
    RecordOptions, Recorder, VideoCodec,
};
pub use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameTopic, LinkError,
    NodeAcl, NodeId, Role, SignalClient,
};
