//! MediaServo Codec — unified video encode/decode API.
//!
//! Two backends: GStreamer (dynamic .so, Host default) and FFmpeg (static .a, Remote default).
//! With no features enabled, a stub backend is used for compilation checks.

pub mod backend;
pub mod codec;
pub mod config;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod factory;
pub mod frame;
pub mod packet;

pub use codec::{BackendId, CodecId, FrameRate, PixelFormat, VideoFormat};
pub use config::{Bitrate, DecoderConfig, EncoderConfig, EncoderPreset};
pub use decoder::VideoDecoder;
pub use encoder::VideoEncoder;
pub use error::CodecError;
pub use factory::CodecFactory;
pub use frame::{Plane, VideoFrame};
pub use packet::EncodedPacket;
