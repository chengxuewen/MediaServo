//! Task E1: FrameBus::list_topics cross-process discovery (Momus MEDIUM-2 option 1, D-H4 hands-on discovery).
//!
//! iceoryx2 0.9.3 data source: `Service::list(Config::global_config(), ...)` service-registry enumeration
//! (official example: iceoryx2 source `src/service/mod.rs` `Service::list` doc example).
//!
//! Prerequisite (C25): `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*` - cross-run residue causes SystemInFlux.

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
    let topic = FrameTopic::new(format!("camera/disc0/{}/raw", std::process::id())); // unique name
    bus.publish(&topic, &[1u8, 2, 3], &FrameMeta::default()).unwrap();

    let topics = FrameBus::list_topics().unwrap();
    let found = topics
        .iter()
        .find(|t| t.topic == topic)
        .expect("a published topic must be enumerated by the service registry");
    assert!(
        found.alive_nodes >= 1,
        "publisher process alive, alive_nodes should be >= 1, got {}",
        found.alive_nodes
    );
}

#[tokio::test]
async fn list_topics_sees_subscriber_only_service() {
    // subscribers (e.g. recorder) also create the topic service - discovery semantics: a live service is visible
    let (tok_sub, vk_sub) = token(Role::Processor, "proc-disc0");
    let bus = FrameBus::attach("", &tok_sub, &vk_sub).unwrap();
    let topic = FrameTopic::new(format!("camera/disc0/{}/subonly", std::process::id()));
    let stream = bus.subscribe(&topic).unwrap();

    let topics = FrameBus::list_topics().unwrap();
    assert!(
        topics.iter().any(|t| t.topic == topic),
        "a subscriber-only topic service must also be enumerated: {topics:#?}"
    );
    drop(stream);
}
