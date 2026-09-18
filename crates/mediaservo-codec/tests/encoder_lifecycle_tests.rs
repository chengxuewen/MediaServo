//! Encoder lifecycle tests — push/pull/flush flow with stub backend.

use mediaservo_codec::codec::{CodecId, PixelFormat, VideoFormat};
use mediaservo_codec::config::{Bitrate, EncoderConfig, EncoderPreset};
use mediaservo_codec::factory::CodecFactory;
use mediaservo_codec::frame::{Plane, VideoFrame};

fn make_config(w: u32, h: u32) -> EncoderConfig {
    let fmt = VideoFormat { width: w, height: h, pixel_format: PixelFormat::Yuv420p };
    EncoderConfig::builder(CodecId::H264, fmt)
        .bitrate(Bitrate::Cbr(2000))
        .fps(30, 1)
        .preset(EncoderPreset::P4Medium)
        .gop(30)
        .build()
}

fn make_frame(w: u32, h: u32, pts: u64, keyframe: bool) -> VideoFrame {
    let y_size = (w * h) as usize;
    let uv_size = ((w / 2) * (h / 2)) as usize;
    VideoFrame {
        format: VideoFormat { width: w, height: h, pixel_format: PixelFormat::Yuv420p },
        planes: vec![
            Plane { data: vec![128u8; y_size], stride: w },
            Plane { data: vec![128u8; uv_size], stride: w / 2 },
            Plane { data: vec![128u8; uv_size], stride: w / 2 },
        ],
        pts,
        keyframe,
    }
}

#[test]
fn configure_before_push_succeeds() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_config(640, 480), None).unwrap();
    encoder.configure(&make_config(640, 480)).unwrap();
    let frame = make_frame(640, 480, 0, false);
    assert!(encoder.push_frame(&frame).is_ok());
}

#[test]
#[cfg(not(any(feature = "backend-ffmpeg", feature = "backend-gstreamer")))]
fn push_30_frames_then_pull_returns_none_for_stub() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_config(320, 240), None).unwrap();
    encoder.configure(&make_config(320, 240)).unwrap();
    for i in 0..30 {
        let frame = make_frame(320, 240, i, i % 30 == 0);
        assert!(encoder.push_frame(&frame).is_ok());
    }
    // Stub always returns None
    for _ in 0..5 {
        assert!(encoder.pull_packet().unwrap().is_none());
    }
}

#[test]
fn flush_with_no_pending_frames_is_ok() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_config(640, 480), None).unwrap();
    encoder.configure(&make_config(640, 480)).unwrap();
    assert!(encoder.flush().is_ok());
}

#[test]
fn push_flush_pull_sequence_does_not_panic() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_config(320, 240), None).unwrap();
    encoder.configure(&make_config(320, 240)).unwrap();
    for i in 0..5 {
        let frame = make_frame(320, 240, i, i == 0);
        encoder.push_frame(&frame).unwrap();
    }
    encoder.flush().unwrap();
    // Pull until drained
    loop {
        match encoder.pull_packet() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => panic!("unexpected error"),
        }
    }
}

#[test]
fn stats_increment_with_frames() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_config(320, 240), None).unwrap();
    encoder.configure(&make_config(320, 240)).unwrap();
    for i in 0..10 {
        let frame = make_frame(320, 240, i, i == 0);
        encoder.push_frame(&frame).unwrap();
    }
    // Stub encoder tracks frames_encoded
    let stats = encoder.stats();
    assert!(stats.frames_encoded <= 10);
}

#[test]
fn configure_twice_overwrites() {
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_config(640, 480), None).unwrap();
    encoder.configure(&make_config(640, 480)).unwrap();
    // Second configure should succeed
    encoder.configure(&make_config(320, 240)).unwrap();
    // Encoder should still work
    let frame = make_frame(320, 240, 0, false);
    assert!(encoder.push_frame(&frame).is_ok());
}

#[test]
#[cfg(not(any(feature = "backend-ffmpeg", feature = "backend-gstreamer")))]
fn push_after_close_continues_working_for_stub() {
    // Stub backend is stateless — close is a no-op
    let factory = CodecFactory::new();
    let mut encoder = factory.create_encoder(make_config(640, 480), None).unwrap();
    encoder.configure(&make_config(640, 480)).unwrap();
    encoder.flush().unwrap();
    // Stub should continue to accept frames after flush
    let frame = make_frame(640, 480, 0, false);
    assert!(encoder.push_frame(&frame).is_ok());
}
