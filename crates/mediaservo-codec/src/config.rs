use crate::codec::{CodecId, FrameRate, VideoFormat};

/// Bitrate control mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bitrate {
    Cbr(u32),
    Vbr { target: u32, max: u32 },
}

/// Encoder quality/speed preset (x264-aligned scale).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EncoderPreset {
    P1UltraFast = 1,
    P2SuperFast = 2,
    P3VeryFast = 3,
    P4Medium = 4,
    P5Slow = 5,
    P6VerySlow = 6,
    P7Lossless = 7,
}

/// Encoder configuration.
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub codec: CodecId,
    pub format: VideoFormat,
    pub bitrate: Bitrate,
    pub fps: FrameRate,
    pub preset: EncoderPreset,
    pub gop: u32,
}

impl EncoderConfig {
    pub fn builder(codec: CodecId, format: VideoFormat) -> EncoderConfigBuilder {
        EncoderConfigBuilder {
            codec,
            format,
            bitrate: Bitrate::Cbr(2000),
            fps: FrameRate::new(30, 1),
            preset: EncoderPreset::P4Medium,
            gop: 60,
        }
    }
}

pub struct EncoderConfigBuilder {
    codec: CodecId,
    format: VideoFormat,
    bitrate: Bitrate,
    fps: FrameRate,
    preset: EncoderPreset,
    gop: u32,
}

impl EncoderConfigBuilder {
    pub fn bitrate(mut self, bitrate: Bitrate) -> Self {
        self.bitrate = bitrate;
        self
    }
    pub fn fps(mut self, num: u32, den: u32) -> Self {
        self.fps = FrameRate::new(num, den);
        self
    }
    pub fn preset(mut self, preset: EncoderPreset) -> Self {
        self.preset = preset;
        self
    }
    pub fn gop(mut self, gop: u32) -> Self {
        self.gop = gop;
        self
    }
    pub fn build(self) -> EncoderConfig {
        EncoderConfig {
            codec: self.codec,
            format: self.format,
            bitrate: self.bitrate,
            fps: self.fps,
            preset: self.preset,
            gop: self.gop,
        }
    }
}

/// Decoder configuration.
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    pub codec: CodecId,
}
