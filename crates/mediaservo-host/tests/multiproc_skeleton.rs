//! Multiprocess skeleton tests (Task A1): 9 bin declarations + placeholder process lifecycle.
//!
//! - all_bins_declared: Cargo.toml must declare mediaservo-host / host-agent / host-capturer /
//!   host-streamer / host-recorder / host-controller / host-emergency / host-audio / host-legacy
//! - placeholder_blocks_and_exits_on_signal: host-agent gateway ready ->
//!   blocks alive -> SIGTERM -> exit code 0
//!
//! C1 revision: host-capturer has been replaced by a real implementation (needs --camera/--config/--token args),
//! the placeholder lifecycle test now uses host-agent (still a placeholder).
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

#[test]
fn all_bins_declared() {
    // Read the Cargo.toml [[bin]] section: must contain mediaservo-host, host-agent, host-capturer,
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
        assert!(manifest.contains(&format!("name = \"{bin}\"")), "missing bin {bin}");
    }
}

#[test]
fn placeholder_blocks_and_exits_on_signal() {
    // Since D1 host-agent is a real signaling gateway (local accept + remote reconnect loop) - the placeholder lifecycle
    // test is updated to gateway semantics: spawn -> wait 200ms -> process alive (log shows gateway ready)
    // -> SIGTERM -> exit code 0. By default no remote server is online: connect failures retry every 10s, no exit.
    let log = tempfile::NamedTempFile::new().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .stdout(Stdio::from(log.reopen().unwrap()))
        .spawn()
        .expect("spawn host-agent");
    // The child must be reaped even on assertion failure (an early failure would leave an agent holding 17980, polluting later tests)
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
    assert!(out.contains("gateway ready"), "stdout missing ready line, got: {out:?}");

    // SIGTERM -> graceful exit (exit code 0); the happy path takes the child out (guard no longer kills)
    unsafe { libc::kill(reap.0.as_ref().unwrap().id() as i32, libc::SIGTERM) };
    let status = reap.0.take().unwrap().wait().expect("wait host-agent");
    assert_eq!(status.code(), Some(0), "expected graceful exit 0, got {status:?}");
}
