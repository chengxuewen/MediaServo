//! Task 1: 品牌化 ID + FrameMeta 测试。

use mediaservo_link::{FrameMeta, FrameTopic};

#[test]
fn topic_wildcard_match() {
    let t = FrameTopic::new("camera/front/raw");
    assert!(t.matches("camera/*"), "camera/* 应匹配 camera/front/raw");
    assert!(t.matches("camera/front/raw"), "精确匹配应成立");
    assert!(!t.matches("video/*"), "video/* 不应匹配 camera topic");
    assert!(!t.matches("camera/front"), "更短的精确串不应匹配更深的 topic");
}

#[test]
fn frame_meta_has_format_and_version_roundtrip() {
    let m = FrameMeta {
        seq: 7,
        width: 1920,
        height: 1080,
        format: 1, // I420
        version: FrameMeta::WIRE_VERSION,
        is_keyframe: true,
        ts_mono_ns: 123,
        ts_epoch_ns: 456,
    };
    let bytes = m.encode();
    assert_eq!(bytes.len(), FrameMeta::WIRE_LEN);
    let d = FrameMeta::decode(&bytes).unwrap();
    assert_eq!(d, m, "roundtrip 应无损");
    assert_eq!((d.format, d.version), (1, FrameMeta::WIRE_VERSION), "format/version 字段必须保留");
    assert!(d.is_keyframe);
}

#[test]
fn frame_meta_decode_rejects_short_buffer() {
    let short = [0u8; 4];
    assert!(FrameMeta::decode(&short).is_err(), "过短缓冲应报错");
}
