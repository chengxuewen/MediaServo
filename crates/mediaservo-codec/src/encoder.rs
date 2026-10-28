use crate::{CodecError, EncodedPacket, EncoderConfig, VideoFrame};

/// Live encoder statistics.
#[derive(Debug, Clone, Default)]
pub struct EncoderStats {
    pub frames_encoded: u64,
    pub packets_produced: u64,
    pub bytes_encoded: u64,
}

/// Video encoder trait — push raw frames, pull encoded packets.
pub trait VideoEncoder: Send {
    fn configure(&mut self, config: &EncoderConfig) -> Result<(), CodecError>;
    fn push_frame(&mut self, frame: &VideoFrame) -> Result<(), CodecError>;
    fn pull_packet(&mut self) -> Result<Option<EncodedPacket>, CodecError>;
    fn flush(&mut self) -> Result<(), CodecError>;
    fn stats(&self) -> EncoderStats;
}
