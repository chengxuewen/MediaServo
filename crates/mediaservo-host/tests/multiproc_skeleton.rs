//! Multiprocess 骨架测试（Task A1）：9 个 bin 声明 + 占位进程生命周期。
//!
//! - all_bins_declared: Cargo.toml 必须声明 mediaservo-host / host-agent / host-capturer /
//!   host-streamer / host-recorder / host-controller / host-emergency / host-audio / host-legacy
//! - placeholder_blocks_and_exits_on_signal: host-agent 网关就绪 →
//!   阻塞存活 → SIGTERM → 退出码 0
//!
//! C1 修订: host-capturer 已替换为真实实现（需 --camera/--config/--token 参数），
//! 占位生命周期测试改用 host-agent（仍为占位）。
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

#[test]
fn all_bins_declared() {
    // 读取 Cargo.toml [[bin]] 段：必须含 mediaservo-host, host-agent, host-capturer,
    // host-streamer, host-recorder, host-controller, host-emergency, host-audio, host-legacy
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    for bin in [
        "mediaservo-host",
        "host-agent",
        "host-capturer",
        "host-streamer",
        "host-recorder",
        "host-controller",
        "host-emergency",
        "host-audio",
        "host-legacy",
    ] {
        assert!(
            manifest.contains(&format!("name = \"{bin}\"")),
            "missing bin {bin}"
        );
    }
}

#[test]
fn placeholder_blocks_and_exits_on_signal() {
    // D1 起 host-agent 为真实信令网关（本地 accept + 远端重连循环）——占位生命周期
    // 测试更新为网关语义：spawn → 等 200ms → 进程存活（日志含网关就绪）
    // → SIGTERM → 退出码 0。默认远端无 server 在线：连接失败走 10s 重试，不退出。
    let log = tempfile::NamedTempFile::new().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .stdout(Stdio::from(log.reopen().unwrap()))
        .spawn()
        .expect("spawn host-agent");
    // 断言失败也必须回收子进程（早期失败会遗留占 17980 的 agent 污染后续测试）
    struct Reap(Option<std::process::Child>);
    impl Drop for Reap {
        fn drop(&mut self) {
            if let Some(mut c) = self.0.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
    let mut reap = Reap(Some(child));

    thread::sleep(Duration::from_millis(200));
    assert!(
        reap.0.as_mut().unwrap().try_wait().unwrap().is_none(),
        "host-agent exited prematurely"
    );

    let mut out = String::new();
    log.reopen().unwrap().read_to_string(&mut out).unwrap();
    assert!(
        out.contains("网关就绪"),
        "stdout missing ready line, got: {out:?}"
    );

    // SIGTERM → 优雅退出（退出码 0）；正常路径取出 child（guard 不再 kill）
    unsafe { libc::kill(reap.0.as_ref().unwrap().id() as i32, libc::SIGTERM) };
    let status = reap.0.take().unwrap().wait().expect("wait host-agent");
    assert_eq!(status.code(), Some(0), "expected graceful exit 0, got {status:?}");
}
