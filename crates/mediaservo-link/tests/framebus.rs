//! Task 5: FrameBus 单进程测试（roundtrip / ACL 负例 / 单发布者）。

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
async fn pubsub_roundtrip() {
    // 发布方：Capture（pub camera/*）；订阅方：Processor（sub camera/*）——角色不对称（D237）
    let (tok_pub, vk_pub) = token(Role::Capture, "capture-fb0");
    let bus_pub = FrameBus::attach("", &tok_pub, &vk_pub).unwrap();
    let (tok_sub, vk_sub) = token(Role::Processor, "proc-fb0");
    let bus_sub = FrameBus::attach("", &tok_sub, &vk_sub).unwrap();
    let topic = FrameTopic::new(format!("camera/fb0/{}/raw", std::process::id())); // 唯一名, 避免跨 run 残留
    let stream = bus_sub.subscribe(&topic).unwrap();
    let meta = FrameMeta {
        seq: 1,
        width: 640,
        height: 480,
        format: 1, // I420
        version: FrameMeta::WIRE_VERSION,
        is_keyframe: true,
        ts_mono_ns: 100,
        ts_epoch_ns: 200,
    };
    bus_pub.publish(&topic, &[1u8, 2, 3], &meta).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await; // 调试: 看是否时序问题
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), stream.recv())
        .await
        .expect("recv timeout")
        .expect("frame");
    assert_eq!(frame.payload(), &[1u8, 2, 3]);
    assert_eq!(frame.meta().seq, 1);
    assert_eq!(frame.meta().width, 640);
    assert!(frame.meta().is_keyframe);
}

#[tokio::test]
async fn acl_deny_publish() {
    let (tok, vk) = token(Role::Capture, "capture-fb1");
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    let topic = FrameTopic::new("control/cmd");
    let err = bus.publish(&topic, &[1], &FrameMeta::default()).unwrap_err();
    assert!(matches!(err, LinkError::AclDenied { .. }), "capture 不应能 publish control/cmd，got: {err:?}");
}

#[tokio::test]
async fn acl_deny_subscribe() {
    let (tok, vk) = token(Role::Capture, "capture-fb2");
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    let topic = FrameTopic::new("camera/fb2/raw");
    let err = bus.subscribe(&topic).unwrap_err();
    assert!(matches!(err, LinkError::AclDenied { .. }), "capture 不应能 subscribe，got: {err:?}");
}

#[tokio::test]
async fn single_publisher_conflict() {
    // 节点 A 发布 camera/fb3/raw
    let (tok_a, vk_a) = token(Role::Capture, "capture-a");
    let bus_a = FrameBus::attach("", &tok_a, &vk_a).unwrap();
    let topic = FrameTopic::new("camera/fb3/raw");
    bus_a.publish(&topic, &[1], &FrameMeta::default()).unwrap();
    // 节点 B 发布同 topic → TopicConflict（D239）
    let (tok_b, vk_b) = token(Role::Capture, "capture-b");
    let bus_b = FrameBus::attach("", &tok_b, &vk_b).unwrap();
    let err = bus_b.publish(&topic, &[2], &FrameMeta::default()).unwrap_err();
    assert!(matches!(err, LinkError::TopicConflict { .. }), "第二发布者应冲突，got: {err:?}");
}

#[tokio::test]
async fn same_node_republish_allowed() {
    let (tok, vk) = token(Role::Capture, "capture-same");
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    let topic = FrameTopic::new("camera/fb4/raw");
    bus.publish(&topic, &[1], &FrameMeta::default()).unwrap();
    bus.publish(&topic, &[2], &FrameMeta::default()).unwrap(); // 同节点再次发布应允许
}
