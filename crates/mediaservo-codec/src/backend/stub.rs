//! Stub backend — no-op encoder/decoder for development and compile checks.

use crate::config::{DecoderConfig, EncoderConfig};
use crate::decoder::VideoDecoder;
use crate::encoder::EncoderStats;
use crate::encoder::VideoEncoder;
use crate::error::CodecError;
use crate::frame::VideoFrame;
use crate::packet::EncodedPacket;

pub(crate) struct StubEncoder;
impl Default for StubEncoder {
    fn default() -> Self {
        Self
    }
}

impl VideoEncoder for StubEncoder {
    fn configure(&mut self, _config: &EncoderConfig) -> Result<(), CodecError> {
        Ok(())
    }
    fn push_frame(&mut self, _frame: &VideoFrame) -> Result<(), CodecError> {
        Ok(())
    }
    fn pull_packet(&mut self) -> Result<Option<EncodedPacket>, CodecError> {
        Ok(None)
    }
    fn flush(&mut self) -> Result<(), CodecError> {
        Ok(())
    }
    fn stats(&self) -> EncoderStats {
        EncoderStats::default()
    }
}

pub(crate) struct StubDecoder;
impl Default for StubDecoder {
    fn default() -> Self {
        Self
    }
}

impl VideoDecoder for StubDecoder {
    fn configure(&mut self, _config: &DecoderConfig) -> Result<(), CodecError> {
        Ok(())
    }
    fn push_packet(&mut self, _data: &[u8]) -> Result<(), CodecError> {
        Ok(())
    }
    fn pull_frame(&mut self) -> Result<Option<VideoFrame>, CodecError> {
        Ok(None)
    }
    fn flush(&mut self) -> Result<(), CodecError> {
        Ok(())
    }
}
