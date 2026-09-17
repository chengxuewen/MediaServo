//! Task 5: FrameBus 多进程零拷贝测试（父进程订阅，子进程发布 1080p）。

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameTopic, NodeAcl, NodeId,
    Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

#[tokio::test]
async fn multiproc_zero_copy_1080p() {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new("proc-mp"), Role::Processor);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    // 唯一 topic（避免跨 run 的 iceoryx2 全局服务污染）
    let topic = FrameTopic::new(format!("camera/mp/{}/raw", std::process::id()));
    let stream = bus.subscribe(&topic).unwrap();
    // spawn 子进程发布 3_110_400 字节
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_framebus_pub"))
        .arg(topic.as_str())
        .spawn()
        .expect("spawn framebus_pub");
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.recv())
        .await
        .expect("recv timeout")
        .expect("frame");
    assert_eq!(frame.payload().len(), 3_110_400, "payload 应为 1080p I420 大小");
    assert_eq!(frame.meta().seq, 7);
    assert_eq!(frame.meta().width, 1920);
    assert_eq!(frame.meta().height, 1080);
    let status = child.wait().expect("wait child");
    assert!(status.success(), "child 应成功");
}
