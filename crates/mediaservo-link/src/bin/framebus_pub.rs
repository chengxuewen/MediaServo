//! 多进程测试子进程：attach 后发布一帧 1080p I420 到指定 topic。
//!
//! 用法：framebus_pub <topic>

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn main() {
    let topic = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "camera/mp/raw".to_string());
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new("capture-mp"), Role::Capture);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).expect("sign token");
    let bus = FrameBus::attach("", &tok, &vk).expect("attach");
    // 1080p I420 = 1920*1080*3/2 = 3_110_400 字节
    let frame = vec![0xABu8; 3_110_400];
    let topic_name = topic.clone();
    let meta = FrameMeta {
        seq: 7,
        width: 1920,
        height: 1080,
        format: 1, // I420
        version: FrameMeta::WIRE_VERSION,
        is_keyframe: true,
        ts_mono_ns: 0,
        ts_epoch_ns: 0,
    };
    bus.publish(&FrameTopic::new(topic), &frame, &meta)
        .expect("publish");
    println!("published 3110400 bytes to {topic_name}");
}
