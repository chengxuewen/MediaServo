//! 多进程崩溃恢复测试子进程：持续向指定 topic 发布帧（默认 10fps 小帧）直到被杀死。
//!
//! 用法：framebus_pub_loop <topic> [frame_bytes] [fps]

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let topic = args
        .first()
        .cloned()
        .unwrap_or_else(|| "camera/crash/raw".to_string());
    let frame_bytes: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(64);
    let fps: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(10);
    let interval = std::time::Duration::from_millis(1000 / fps);

    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new("capture-crash"), Role::Capture);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).expect("sign token");
    let bus = FrameBus::attach("", &tok, &vk).expect("attach");

    let payload = vec![0xCDu8; frame_bytes];
    let mut seq = 0u64;
    loop {
        seq += 1;
        let meta = FrameMeta {
            seq,
            width: 16,
            height: 16,
            format: 1, // I420
            version: FrameMeta::WIRE_VERSION,
            is_keyframe: seq == 1,
            ts_mono_ns: seq,
            ts_epoch_ns: 0,
        };
        bus.publish(&FrameTopic::new(topic.as_str()), &payload, &meta)
            .expect("publish");
        std::thread::sleep(interval);
    }
}
