//! Task E1: 拓扑监控 e2e — 真实数据源闭环。
//!
//! ① oxmgr 进程起/杀 → snapshot 实际态变化 + diff 出现 mismatch（进程级）；
//! ② FrameBus 发布 → collect() 的 actual_topics 可见（发现式实际，发布者级）。
//!
//! 依赖: oxmgr CLI（PATH 或 ~/.local/bin，daemon 自动拉起）+ iceoryx2 运行时。
//! 前置（C25）: `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*`。
//! oxmgr 不可用时跳过（CI 环境无 daemon 也能跑其余测试）。

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use mediaservo_host::monitor::topology::{Mismatch, OxmgrClient, TopologyMonitor, diff};
use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

/// PATH 查找 oxmgr，回退 ~/.local/bin（与 topology.rs find_oxmgr 同逻辑）。
fn oxmgr_bin() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let cand = dir.join("oxmgr");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let fallback =
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".local/bin/oxmgr");
    fallback.is_file().then_some(fallback)
}

/// oxmgr 托管进程清理守卫（断言失败也删除）。
struct OxCleanup(PathBuf, String);
impl Drop for OxCleanup {
    fn drop(&mut self) {
        let _ = Command::new(&self.0).args(["delete", &self.1]).output();
    }
}

#[test]
fn oxmgr_process_lifecycle_reflected_in_snapshot() {
    let Some(bin) = oxmgr_bin() else {
        eprintln!("oxmgr 不可用, 跳过");
        return;
    };
    let name = format!("e1-topo-{}", std::process::id());

    let out = Command::new(&bin)
        .args(["start", "--name", &name, "sleep 300"])
        .output()
        .expect("oxmgr start 执行");
    assert!(out.status.success(), "oxmgr start 失败: {}", String::from_utf8_lossy(&out.stderr));
    let _guard = OxCleanup(bin.clone(), name.clone());

    // 等待注册为 running（oxmgr 注册与进程拉起间有小窗口）
    let ox = OxmgrClient::with_bin(bin.clone());
    let mut procs = vec![];
    for _ in 0..50 {
        procs = ox.list().unwrap_or_default();
        if procs.iter().any(|p| p.name == name && p.status == "running") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        procs.iter().any(|p| p.name == name && p.status == "running"),
        "启动后应 running: {procs:#?}"
    );

    // 快照: 实际态含该进程 → 无 mismatch
    let monitor = TopologyMonitor::new_with_grace(String::new(), Duration::ZERO);
    let snap = monitor.collect();
    assert!(snap.actual_processes.iter().any(|p| p.name == name && p.status == "running"));
    assert!(diff(std::slice::from_ref(&name), &[], &snap.actual_processes, &[]).is_empty());

    // 杀 → 实际态缺失 → mismatch 出现（grace=0 即时生效）
    let out = Command::new(&bin).args(["stop", &name]).output().expect("oxmgr stop 执行");
    assert!(out.status.success(), "oxmgr stop 失败: {}", String::from_utf8_lossy(&out.stderr));
    for _ in 0..50 {
        procs = ox.list().unwrap_or_default();
        if !procs.iter().any(|p| p.name == name && p.status == "running") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let snap = monitor.collect();
    assert!(
        diff(std::slice::from_ref(&name), &[], &snap.actual_processes, &[])
            .contains(&Mismatch::ProcessMissing { name: name.clone() }),
        "停止后应报进程缺失: {snap:#?}"
    );
}

#[tokio::test]
async fn publisher_discovery_visible_in_snapshot() {
    // 真实发布者 → collect() 的 actual_topics 应可见（发现式实际闭环）
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new("capture-e1e2"), Role::Capture);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    let bus = FrameBus::attach("", &tok, &vk).unwrap();
    let topic = FrameTopic::new(format!("camera/e1e2/{}/raw", std::process::id()));
    bus.publish(&topic, &[1u8, 2, 3], &FrameMeta::default()).unwrap();

    let monitor = TopologyMonitor::new_with_grace(String::new(), Duration::ZERO);
    let snap = monitor.collect();
    assert!(
        snap.actual_topics.iter().any(|t| t == &topic),
        "发布者 topic 应出现在快照 actual_topics: {snap:#?}"
    );
}
