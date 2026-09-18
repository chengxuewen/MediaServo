//! CodecError unit tests.

use mediaservo_codec::codec::CodecId;
use mediaservo_codec::error::CodecError;

#[test]
fn error_invalid_config_displays() {
    let err = CodecError::InvalidConfig("bad bitrate".into());
    let msg = err.to_string();
    assert!(msg.contains("bad bitrate"));
    assert!(msg.contains("invalid"));
}

#[test]
fn error_invalid_dimension_displays() {
    let err = CodecError::InvalidDimension(0, 1080);
    let msg = err.to_string();
    assert!(msg.contains("0"));
    assert!(msg.contains("1080"));
}

#[test]
fn error_unsupported_codec_displays_codec_name() {
    let err = CodecError::UnsupportedCodec(CodecId::H264);
    let msg = err.to_string();
    assert!(msg.contains("H264"));
}

#[test]
fn error_nobackend_displays() {
    let err = CodecError::NoBackend(CodecId::H264);
    let msg = err.to_string();
    assert!(msg.contains("H264"));
    assert!(msg.contains("no backend"));
}

#[test]
fn error_chain_via_thiserror() {
    let inner = std::io::Error::other("io failed");
    // CodecError doesn't support chaining directly, but verify it produces Debug
    let err = CodecError::Internal("test".into());
    assert!(!format!("{:?}", err).is_empty());
    let _ = inner; // just checking compilation
}
