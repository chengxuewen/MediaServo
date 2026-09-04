//! MediaServo Field — 组合 SDK。
//!
//! 一行依赖完整闭环：信令 + 帧总线 + 采集/录制/回放 + WebRTC 抽象。
//! - 通信: `SignalClient`/`FrameBus`（re-export link）
//! - 媒体: `MediaDevices`/`CameraSource`（re-export deck）
//! - 会话: `PushSession`/`PullSession`（推流/拉流门面，Phase 2 接 webrtc 真传输）
//!
//! 依赖方向（C21 单向无环）：`field → webrtc + link + deck`。

pub mod config;
pub mod error;
pub mod session;
pub mod sfu;

pub use config::{PresetBundle, PublishOptions, PullConfig, PushConfig, StreamMode};
pub use error::FieldError;

pub use session::{PullSession, PushSession, SessionEvent, SessionEvents};

// ── 组合 re-export（契约 §4: 一行依赖闭环）────────────────
pub use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameTopic, LinkError,
    NodeAcl, NodeId, Role, SignalClient,
};
pub use mediaservo_deck::{
    CameraSource, CaptureOptions, Container, DeviceId, MediaDeviceKind, MediaDevices, Player,
    RecordOptions, Recorder, VideoCodec,
};