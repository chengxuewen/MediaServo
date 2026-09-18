//! Config builder unit tests.

use mediaservo_codec::codec::{CodecId, PixelFormat, VideoFormat};
use mediaservo_codec::config::{Bitrate, DecoderConfig, EncoderConfig, EncoderPreset};

#[test]
fn encoder_config_defaults_are_valid() {
    let fmt = VideoFormat { width: 640, height: 480, pixel_format: PixelFormat::Yuv420p };
    let cfg = EncoderConfig::builder(CodecId::H264, fmt).build();
    assert_eq!(cfg.codec, CodecId::H264);
    assert_eq!(cfg.format.width, 640);
    assert_eq!(cfg.format.pixel_format, PixelFormat::Yuv420p);
    assert_eq!(cfg.fps.fps(), 30.0);
    assert_eq!(cfg.gop, 60);
}

#[test]
fn encoder_config_builder_sets_all_fields() {
    let fmt = VideoFormat { width: 1920, height: 1080, pixel_format: PixelFormat::Yuv420p };
    let cfg = EncoderConfig::builder(CodecId::H264, fmt)
        .bitrate(Bitrate::Vbr { target: 4000, max: 8000 })
        .fps(60, 1)
        .preset(EncoderPreset::P3VeryFast)
        .gop(30)
        .build();
    assert_eq!(cfg.fps.fps(), 60.0);
    assert_eq!(cfg.gop, 30);
    assert_eq!(cfg.preset, EncoderPreset::P3VeryFast);
    if let Bitrate::Vbr { target, max } = cfg.bitrate {
        assert_eq!(target, 4000);
        assert_eq!(max, 8000);
    } else {
        panic!("expected Vbr");
    }
}

#[test]
fn encoder_config_builder_accepts_minimal() {
    let fmt = VideoFormat { width: 320, height: 240, pixel_format: PixelFormat::Nv12 };
    let cfg = EncoderConfig::builder(CodecId::H264, fmt).build();
    assert_eq!(cfg.format.width, 320);
    assert_eq!(cfg.format.height, 240);
}

#[test]
fn encoder_config_clone_preserves_fields() {
    let fmt = VideoFormat { width: 1280, height: 720, pixel_format: PixelFormat::Yuv420p };
    let cfg = EncoderConfig::builder(CodecId::H264, fmt)
        .bitrate(Bitrate::Cbr(3000))
        .preset(EncoderPreset::P1UltraFast)
        .gop(15)
        .build();
    let cfg2 = cfg.clone();
    assert_eq!(cfg2.codec, cfg.codec);
    assert_eq!(cfg2.gop, cfg.gop);
}

#[test]
fn decoder_config_default_is_valid() {
    let cfg = DecoderConfig { codec: CodecId::H264 };
    assert_eq!(cfg.codec, CodecId::H264);
}

#[test]
fn bitrate_cbr_and_vbr_are_distinct() {
    let cbr = Bitrate::Cbr(2000);
    let vbr = Bitrate::Vbr { target: 2000, max: 4000 };
    assert_ne!(cbr, vbr);
}

#[test]
fn encoder_preset_values_are_distinct() {
    assert!(EncoderPreset::P1UltraFast < EncoderPreset::P4Medium);
    assert!(EncoderPreset::P4Medium < EncoderPreset::P7Lossless);
}
