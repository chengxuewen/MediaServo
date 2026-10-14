//! MediaServo Deck — media data plane SDK.
//!
//! Standalone deployment scenarios (local monitoring/NVR) capture and record directly, with no WebRTC dependency:
//! - [`source`] — capture (CameraSource/AudioSource/ScreenSource + MediaDevices enumeration)
//! - [`record`] — recording (Recorder, FFmpeg mux to disk)
//! - [`playback`] — playback (Player, demux + decode) — Phase 3

pub mod error;
pub mod playback;
pub mod record;
pub mod source;

pub use error::DeckError;
pub use playback::Player;
pub use record::{Container, RecordOptions, Recorder, VideoCodec};
pub use source::{
    CameraSource, CaptureOptions, DeviceId, FrameStream, MediaDeviceKind, MediaDevices,
};
