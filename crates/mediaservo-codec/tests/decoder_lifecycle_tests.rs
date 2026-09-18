//! Decoder lifecycle tests — push_packet/pull_frame flow with stub backend.

use mediaservo_codec::codec::CodecId;
use mediaservo_codec::config::DecoderConfig;
use mediaservo_codec::factory::CodecFactory;

#[test]
fn configure_before_push_succeeds() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    assert!(decoder.configure(&DecoderConfig { codec: CodecId::H264 }).is_ok());
}

#[test]
fn push_empty_data_is_ok_for_stub() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();
    // Stub accepts any data (no real decode)
    assert!(decoder.push_packet(&[]).is_ok());
}

#[test]
fn pull_before_push_returns_none() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();
    assert!(decoder.pull_frame().unwrap().is_none());
}

#[test]
#[cfg(not(any(feature = "backend-ffmpeg", feature = "backend-gstreamer")))]
fn push_pull_cycle_is_noop_for_stub() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();
    // Push a minimal NAL unit header
    decoder.push_packet(&[0, 0, 0, 1, 0x67]).unwrap();
    assert!(decoder.pull_frame().unwrap().is_none());
}

#[test]
fn flush_with_no_pending_is_ok() {
    let factory = CodecFactory::new();
    let mut decoder = factory.create_decoder(DecoderConfig { codec: CodecId::H264 }, None).unwrap();
    decoder.configure(&DecoderConfig { codec: CodecId::H264 }).unwrap();
    assert!(decoder.flush().is_ok());
}
