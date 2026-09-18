use crate::backend::{ActiveDecoder, ActiveEncoder};
use crate::codec::{BackendId, CodecId};
use crate::config::{DecoderConfig, EncoderConfig};
use crate::decoder::VideoDecoder;
use crate::encoder::VideoEncoder;
use crate::error::CodecError;

/// Encoder capability descriptor.
#[derive(Debug, Clone)]
pub struct EncoderCapability {
    pub backend: BackendId,
    pub codec: CodecId,
    pub max_width: u32,
    pub max_height: u32,
    pub hardware: bool,
}

/// Decoder capability descriptor.
#[derive(Debug, Clone)]
pub struct DecoderCapability {
    pub backend: BackendId,
    pub codec: CodecId,
    pub hardware: bool,
}

/// Central factory for creating video encoders and decoders.
pub struct CodecFactory;

impl Default for CodecFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl CodecFactory {
    pub fn new() -> Self {
        Self
    }

    pub fn create_encoder(
        &self,
        _config: EncoderConfig,
        _preferred_backend: Option<BackendId>,
    ) -> Result<Box<dyn VideoEncoder>, CodecError> {
        Ok(Box::new(ActiveEncoder::default()))
    }

    pub fn create_decoder(
        &self,
        _config: DecoderConfig,
        _preferred_backend: Option<BackendId>,
    ) -> Result<Box<dyn VideoDecoder>, CodecError> {
        Ok(Box::new(ActiveDecoder::default()))
    }

    pub fn encoder_capabilities(&self, _codec: CodecId) -> Vec<EncoderCapability> {
        vec![]
    }

    pub fn decoder_capabilities(&self, _codec: CodecId) -> Vec<DecoderCapability> {
        vec![]
    }
}
