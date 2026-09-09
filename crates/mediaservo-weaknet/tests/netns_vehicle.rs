//! W8 车端替身集成触发面（tasks.md T12/T13）：环境全备时拉起 `tests/netns_vehicle.sh`
//! （unshare -rn netns 内 veth 拓扑——反锁死负案/命中排除定量/watch/down/内嵌兜底/零残留）；
//! 任一前置缺失 = SKIP 报因放行（「不可构=不闭环」同 netns_ifb 纪律，W8 走替身挂账）。

use std::path::Path;
use std::process::Command;

#[test]
fn netns_vehicle_e2e_when_environment_allows() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let script = Path::new(manifest).join("tests/netns_vehicle.sh");
    assert!(script.is_file(), "缺 {script:?}");
    let bin = env!("CARGO_BIN_EXE_mediaservo-weaknet");

    let have = |prog: &str| {
        Command::new("sh")
            .args(["-c", "command -v \"$1\"", "sh", prog])
            .status()
            .is_ok_and(|st| st.success())
    };
    let unshare_ok = Command::new("unshare")
        .args(["-rn", "true"])
        .status()
        .is_ok_and(|st| st.success());
    let (tc_ok, py_ok) = (have("tc"), have("python3"));
    if !tc_ok || !py_ok || !unshare_ok {
        eprintln!(
            "SKIP netns_vehicle: tc={tc_ok} python3={py_ok} unshare_ok={unshare_ok} —— \
             W8 替身 BLOCKED-env（复验触发=补齐 iproute2/python3/userns 后重跑本用例）"
        );
        return;
    }

    let out = Command::new("bash")
        .arg(&script)
        .env("WEAKNET_BIN", bin)
        .output()
        .expect("netns_vehicle.sh 执行失败");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    // 脚本内部竞态判不可构（userns 被禁等）→ 77 同 SKIP 报因放行。
    if out.status.code() == Some(77) {
        eprintln!("SKIP netns_vehicle: 脚本判定环境不可构（负证据见上方 transcript）——W8 BLOCKED-env");
        return;
    }
    assert!(
        out.status.success(),
        "netns_vehicle.sh 失败 rc={:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        out.status.code()
    );
}
