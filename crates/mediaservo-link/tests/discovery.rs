//! Task E1: FrameBus::list_topics 跨进程发现（Momus MEDIUM-2 选项 ①，D-H4 发现式实际）。
//!
//! iceoryx2 0.9.3 数据源: `Service::list(Config::global_config(), ...)` 服务注册表枚举
//! （官方示例见 iceoryx2 源码 `src/service/mod.rs` `Service::list` doc example）。
//!
//! 前置（C25）: `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*` — 跨 run 残留 → SystemInFlux。

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role,
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
async fn list_topics_finds_published_topic_with_alive_node() {
    let (tok_pub, vk_pub) = token(Role::Capture, "capture-disc0");
    let bus = FrameBus::attach("", &tok_pub, &vk_pub).unwrap();
    let topic = FrameTopic::new(format!("camera/disc0/{}/raw", std::process::id())); // 唯一名
    bus.publish(&topic, &[1u8, 2, 3], &FrameMeta::default()).unwrap();

    let topics = FrameBus::list_topics().unwrap();
    let found = topics
        .iter()
        .find(|t| t.topic == topic)
        .expect("已发布 topic 应被服务注册表枚举到");
    assert!(found.alive_nodes >= 1, "发布端进程存活, alive_nodes 应 >= 1, got {}", found.alive_nodes);
}

#[tokio::test]
async fn list_topics_sees_subscriber_only_service() {
    // 订阅端（如 recorder）也会创建 topic 服务 — 发现语义: 服务存在即可见
    let (tok_sub, vk_sub) = token(Role::Processor, "proc-disc0");
    let bus = FrameBus::attach("", &tok_sub, &vk_sub).unwrap();
    let topic = FrameTopic::new(format!("camera/disc0/{}/subonly", std::process::id()));
    let stream = bus.subscribe(&topic).unwrap();

    let topics = FrameBus::list_topics().unwrap();
    assert!(
        topics.iter().any(|t| t.topic == topic),
        "仅有订阅者的 topic 服务也应被枚举到: {topics:#?}"
    );
    drop(stream);
}
