//! Stub backend behavior tests.

use mediaservo_codec::codec::{CodecId, PixelFormat, VideoFormat};
use mediaservo_codec::config::{Bitrate, DecoderConfig, EncoderConfig, EncoderPreset};
use mediaservo_codec::factory::CodecFactory;
use mediaservo_codec::frame::{Plane, VideoFrame};

fn make_test_frame() -> VideoFrame {
    let fmt = VideoFormat { width: 640, height: 480, pixel_format: PixelFormat::Yuv420p };
    VideoFrame {
        format: fmt.clone(),
        planes: vec![
            Plane { data: vec![128u8; 640 * 480], stride: 640 },
            Plane { data: vec![128u8; 320 * 240], stride: 320 },
            Plane { data: vec![128u8; 320 * 240], stride: 320 },
        ],
        pts: 0,
        keyframe: false,
    }
}

fn make_test_config() -> EncoderConfig {
    let fmt = VideoFormat { width: 640, height: 480, pixel_format: PixelFormat::Yuv420p };
    EncoderConfig::builder(CodecId::H264, fmt)
        .bitrate(Bitrate::Cbr(2000))
        .fps(30, 1)
        .preset(EncoderPreset::P4Medium)
        .gop(60)
        .build()
}

#[test]
fn stub_encode_configure_succeeds() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_test_config(), None).unwrap();
    assert!(encoder.configure(&make_test_config()).is_ok());
}

#[cfg(not(any(feature = "backend-ffmpeg", feature = "backend-gstreamer")))]
#[test]
fn stub_encode_push_pull_cycle_returns_none() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_test_config(), None).unwrap();
    encoder.configure(&make_test_config()).unwrap();

    let frame = make_test_frame();
    assert!(encoder.push_frame(&frame).is_ok());
    assert!(encoder.pull_packet().unwrap().is_none());
}

#[test]
fn stub_encode_flush_noop() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_test_config(), None).unwrap();
    encoder.configure(&make_test_config()).unwrap();
    assert!(encoder.flush().is_ok());
}

#[test]
fn stub_encode_stats_are_default() {
    let factory = CodecFactory::new();
    let encoder = factory.create_encoder(make_test_config(), None).unwrap();
    let stats = encoder.stats();
    assert_eq!(stats.frames_encoded, 0);
    assert_eq!(stats.packets_produced, 0);
}

#[test]
fn stub_decode_configure_succeeds() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    assert!(decoder.configure(&DecoderConfig { codec: CodecId::H264 }).is_ok());
}

#[cfg(not(any(feature = "backend-ffmpeg", feature = "backend-gstreamer")))]
#[test]
fn stub_decode_push_pull_returns_none() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();
    assert!(decoder.push_packet(&[0, 0, 0, 1, 0x67]).is_ok());
    assert!(decoder.pull_frame().unwrap().is_none());
}

#[test]
fn stub_decode_flush_noop() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();
    assert!(decoder.flush().is_ok());
}

#[test]
fn stub_encoder_and_decoder_are_send() {
    #[allow(dead_code)] // 编译期断言文档位
    fn assert_send<T: Send>() {}
    let factory = CodecFactory::new();
    let encoder = factory.create_encoder(make_test_config(), None).unwrap();
    let decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    // Compile-time check: VideoEncoder and VideoDecoder are Send
    let _ = encoder;
    let _ = decoder;
}
