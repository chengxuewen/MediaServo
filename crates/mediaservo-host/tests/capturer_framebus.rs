//! Task C1: host-capturer 进程测试 — 采集 → FrameBus 发布（I420）。
//!
//! - `bad_args_exit_2_with_usage`: 缺参/坏参 → exit 2 + stderr 用法提示
//! - `capturer_publishes_i420_frames_to_framebus`: 真实二进制 + 生成令牌文件 →
//!   订阅 `camera/cam0` → 断言 3 帧（meta 正确、payload = 1280x720 I420）→
//!   SIGTERM → 退出码 0
//!
//! 前置（C25 修订版）: `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*`（iceoryx2 0.9.3 运行时
//! 根在 /tmp/iceoryx2；残留 service 使固定 topic camera/cam0 二次打开持久 SystemInFlux，
//! 重跑前必须全量清）。

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameTopic, NodeAcl,
    NodeId, Role, TokenFile,
};

// 测试用 Ed25519 密钥对（openssl 生成，仅测试用；与 link/deck 测试同源）。
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

#[test]
fn bad_args_exit_2_with_usage() {
    for args in [
        vec![],                                      // 全缺
        vec!["--camera"],                            // 缺值
        vec!["--camera", "cam0"],                    // 缺 config/token
        vec!["--bogus", "x"],                        // 未知参数
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
            .args(&args)
            .output()
            .expect("spawn host-capturer");
        assert_eq!(out.status.code(), Some(2), "args {args:?} 应 exit 2");
        assert!(
            !out.stderr.is_empty(),
            "args {args:?} stderr 应有用法提示, got: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}


/// mode 分派（契约）: camera/desktop/subscriber 采集未实现 → 明确报"未支持"退出；
/// generator 真实现（产帧证据见 capturer_publishes_i420_frames_to_framebus）。
#[test]
fn unsupported_modes_exit_1_with_clear_error() {
    for (mode, needle) in [
        ("camera", "camera"),
        ("desktop", "desktop"),
        ("subscriber", "subscriber"),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg_path = dir.path().join("host.yaml");
        std::fs::write(
            &cfg_path,
            format!(
                "sources:\n  - id: \"cam0\"\n    mode: \"{mode}\"\n    fps: 30\n"
            ),
        )
        .expect("write host.yaml");
        let tok_path = dir.path().join("cam0.token");
        std::fs::write(&tok_path, b"garbage").expect("write token");
        let out = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
            .args([
                "--camera",
                "cam0",
                "--config",
                cfg_path.to_str().expect("cfg utf8"),
                "--token",
                tok_path.to_str().expect("token utf8"),
            ])
            .output()
            .expect("spawn host-capturer");
        assert_eq!(out.status.code(), Some(1), "mode={mode} 应 exit 1");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("未支持"), "mode={mode} 应报未支持, got: {stderr}");
        assert!(stderr.contains(needle), "mode={mode} 错误应含类别 {needle}, got: {stderr}");
    }
}

#[tokio::test]
async fn capturer_publishes_i420_frames_to_framebus() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    width: 640\n    height: 480\n    fps: 30\n",
    )
    .expect("write host.yaml");

    // 生成 capturer 令牌文件（Role::Capture → 可发布 camera/*）
    let (tok, vk) = token(Role::Capture, "capture-cam0");
    let tok_path = dir.path().join("cam0.token");
    std::fs::write(&tok_path, TokenFile::encode(&tok, &vk)).expect("write token file");

    // 订阅侧（Recorder 角色 → 可订阅 camera/*）：先 attach + subscribe 再起 capturer
    let (sub_tok, sub_vk) = token(Role::Recorder, "test-recorder");
    let bus = FrameBus::attach("", &sub_tok, &sub_vk).expect("subscriber attach");
    let topic = FrameTopic::new("camera/cam0");
    let stream = bus.subscribe(&topic).expect("subscribe camera/cam0");

    let log = tempfile::NamedTempFile::new().expect("log file");
    let mut child = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
        .args([
            "--camera",
            "cam0",
            "--config",
            cfg_path.to_str().expect("cfg path utf8"),
            "--token",
            tok_path.to_str().expect("token path utf8"),
        ])
        .stdout(Stdio::from(log.reopen().expect("reopen log")))
        .stderr(Stdio::from(log.reopen().expect("reopen log")))
        .spawn()
        .expect("spawn host-capturer");

    // 收 3 帧，断言 meta + payload
    let mut last_seq: Option<u64> = None;
    let mut ts_positive = false;
    for _ in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(10), stream.recv())
            .await
            .expect("recv timeout — capturer 未发布帧")
            .expect("frame");
        let m = frame.meta();
        assert_eq!(m.width, 640, "宽度应从配置消费（width: 640）");
        assert_eq!(m.height, 480, "高度应从配置消费（height: 480）");
        assert_eq!(m.format, 1, "format 应为 I420(1)");
        assert_eq!(m.version, 1);
        assert_eq!(
            frame.payload().len(),
            640 * 480 * 3 / 2,
            "payload 应为紧凑 I420 大小"
        );
        if let Some(prev) = last_seq {
            assert!(m.seq > prev, "seq 应严格递增: {prev} -> {}", m.seq);
        }
        last_seq = Some(m.seq);
        ts_positive |= m.ts_mono_ns > 0;
    }
    assert!(ts_positive, "ts_mono_ns 应非零（C17 单调时钟）");

    // ready 行（启动完成的证据）
    let mut out = String::new();
    log.reopen()
        .expect("reopen log")
        .read_to_string(&mut out)
        .expect("read log");
    assert!(
        out.contains("capturer ready"),
        "stdout 缺 ready 行, got: {out:?}"
    );
    assert!(
        out.contains("source=generator"),
        "ready 行应标识 generator 源, got: {out:?}"
    );

    // SIGTERM → 优雅退出（退出码 0）
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().expect("wait host-capturer");
    assert_eq!(status.code(), Some(0), "期望优雅退出 0, got {status:?}");
}
