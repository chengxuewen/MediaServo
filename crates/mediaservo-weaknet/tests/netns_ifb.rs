//! W4 集成触发面（tasks.md T9）：环境全备时拉起 `tests/netns_ifb.sh`（unshare -rn
//! netns 内 veth+ifb 三案——命中铁证/定向对照/撤净零残留）；任一前置缺失 = SKIP 报因
//! 放行，W4 走 BLOCKED-env 挂账（「不可构=不闭环，禁注记即绿」）。
//!
//! 判据源与脚本内分叉一致：宿主 /proc/modules 无 ifb 行 ⇒ netns 必建不了（模块全局）。
//! 脚本自身仍会在 netns 里二次实证（负证据 transcript 归脚本输出）。

use std::path::Path;
use std::process::Command;

#[test]
fn netns_ifb_e2e_when_environment_allows() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let script = Path::new(manifest).join("tests/netns_ifb.sh");
    assert!(script.is_file(), "缺 {script:?}");
    let bin = env!("CARGO_BIN_EXE_mediaservo-weaknet");

    let mod_loaded = std::fs::read_to_string("/proc/modules")
        .map(|s| {
            s.lines()
                .any(|l| l.split_whitespace().next() == Some("ifb"))
        })
        .unwrap_or(false);
    let unshare_ok = Command::new("unshare")
        .args(["-rn", "true"])
        .status()
        .is_ok_and(|st| st.success());
    if !mod_loaded || !unshare_ok {
        eprintln!(
            "SKIP netns_ifb: ifb_module={mod_loaded} unshare_ok={unshare_ok} —— \
             W4 = BLOCKED-env（复验触发=宿主 root modprobe ifb 后重跑本用例/netns_ifb.sh）"
        );
        return;
    }

    let out = Command::new("bash")
        .arg(&script)
        .env("WEAKNET_BIN", bin)
        .output()
        .expect("netns_ifb.sh 执行失败");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    // 脚本内 netns 复测仍可能判不可构（竞态卸载）→ 77 同样按 SKIP 放行报因。
    if out.status.code() == Some(77) {
        eprintln!("SKIP netns_ifb: 脚本判定环境不可构（负证据见上方 transcript）——W4 BLOCKED-env");
        return;
    }
    assert!(
        out.status.success(),
        "netns_ifb.sh 失败 rc={:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        out.status.code()
    );
}
