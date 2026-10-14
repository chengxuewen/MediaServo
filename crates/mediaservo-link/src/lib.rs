//! MediaServo link IPC.
//!
//! Device-side multiprocess local IPC (D235-D243):
//! - [`bus::framebus::FrameBus`] - iceoryx2 SHM zero-copy frame bus (latest-frame overwrite)
//! - [`registry::Registry`] - register-on-attach + discovery + liveness (iceoryx2 built-in)
//! - [`acl::NodeAcl`] - static ACL (role presets + node overrides, D237)
//! - [`token::CapabilityToken`] - Ed25519 capability tokens (ACL signed into the JWT, D238)
//!
//! SignalClient (WS signaling toward the server) split out in Phase 1b.

pub mod acl;
pub mod bridge;
pub mod bus;
pub mod error;
pub mod frame;
pub mod id;
pub mod registry;
pub mod signal;
pub mod token;

pub use acl::{NodeAcl, Role};
pub use bus::framebus::FrameBus;
pub use error::LinkError;
pub use frame::{FrameMeta, FrameRef, FrameStream};
pub use id::{FrameTopic, NodeId};
pub use registry::{NodeInfo, Registry, TopicInfo};
pub use signal::{
    DeviceCredential, DeviceIdentity, LocalEnvelope, RetryConfig, SignalClient, SignalEvent,
    SignalSession,
};
pub use token::{CapabilityToken, Claims, Ed25519SigningKey, Ed25519VerifyingKey, TokenFile};
