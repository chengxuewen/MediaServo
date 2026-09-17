//! Task 6: e2e 出图→拼接→推流 + ACL 负例 + 单发布者（D236/D239/D235）。

use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    LinkError, NodeAcl, NodeId, Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

#[tokio::test]
async fn e2e_capture_stitch_push() {
    let suffix = std::process::id();
    let cam_topic = FrameTopic::new(format!("camera/e2e/{suffix}/front/raw"));
    let stitched_topic = FrameTopic::new(format!("video/e2e/{suffix}/stitched"));

    // 出图节点：Capture 发布相机帧
    let (tok_cap, vk_cap) = token(Role::Capture, "e2e-capture");
    let capture_bus = FrameBus::attach("", &tok_cap, &vk_cap).unwrap();

    // 拼接节点：Processor 订阅相机帧 → 拼接 → 发布拼接流
    let (tok_proc, vk_proc) = token(Role::Processor, "e2e-processor");
    let proc_bus = FrameBus::attach("", &tok_proc, &vk_proc).unwrap();
    let cam_stream = proc_bus.subscribe(&cam_topic).unwrap();

    // 推流节点：Pusher 订阅拼接流
    let (tok_push, vk_push) = token(Role::Pusher, "e2e-pusher");
    let push_bus = FrameBus::attach("", &tok_push, &vk_push).unwrap();
    let stitched_sub = push_bus.subscribe(&stitched_topic).unwrap();

    // 出图发布相机帧
    let cam_meta = FrameMeta {
        seq: 1,
        width: 1920,
        height: 1080,
        format: 1,
        version: FrameMeta::WIRE_VERSION,
        is_keyframe: true,
        ts_mono_ns: 0,
        ts_epoch_ns: 0,
    };
    capture_bus.publish(&cam_topic, &[0xCC; 100], &cam_meta).unwrap();

    // 拼接节点收相机帧 → "拼接"（此处直接转发，拼接逻辑占位）→ 发布拼接流
    let cam_frame = tokio::time::timeout(Duration::from_secs(3), cam_stream.recv())
        .await
        .expect("recv cam timeout")
        .expect("cam frame");
    assert_eq!(cam_frame.meta().width, 1920);
    let stitch_meta = FrameMeta {
        seq: 2,
        width: 3840, // 拼接宽度（占位：两路 1920 横拼）
        height: 1080,
        format: 1,
        version: FrameMeta::WIRE_VERSION,
        is_keyframe: true,
        ts_mono_ns: 0,
        ts_epoch_ns: 0,
    };
    proc_bus
        .publish(&stitched_topic, cam_frame.payload(), &stitch_meta)
        .unwrap();

    // 推流节点收拼接帧
    let stitched = tokio::time::timeout(Duration::from_secs(3), stitched_sub.recv())
        .await
        .expect("recv stitched timeout")
        .expect("stitched frame");
    assert_eq!(stitched.meta().width, 3840, "拼接帧应为拼接宽度");
    assert_eq!(stitched.meta().seq, 2);
    assert_eq!(stitched.payload().len(), 100);
}

#[tokio::test]
async fn e2e_acl_negative() {
    let (tok_proc, vk_proc) = token(Role::Processor, "e2e-proc-neg");
    let proc_bus = FrameBus::attach("", &tok_proc, &vk_proc).unwrap();
    let err = proc_bus
        .publish(&FrameTopic::new("control/cmd"), &[1], &FrameMeta::default())
        .unwrap_err();
    assert!(matches!(err, LinkError::AclDenied { .. }), "processor 不应能 publish control/cmd");
}

#[tokio::test]
async fn e2e_single_publisher_conflict() {
    let suffix = std::process::id();
    let topic = FrameTopic::new(format!("video/e2e/{suffix}/conflict"));
    let (tok_a, vk_a) = token(Role::Processor, "e2e-proc-a");
    let bus_a = FrameBus::attach("", &tok_a, &vk_a).unwrap();
    bus_a.publish(&topic, &[1], &FrameMeta::default()).unwrap();
    let (tok_b, vk_b) = token(Role::Processor, "e2e-proc-b");
    let bus_b = FrameBus::attach("", &tok_b, &vk_b).unwrap();
    let err = bus_b
        .publish(&topic, &[2], &FrameMeta::default())
        .unwrap_err();
    assert!(matches!(err, LinkError::TopicConflict { .. }), "第二发布者应冲突");
}
