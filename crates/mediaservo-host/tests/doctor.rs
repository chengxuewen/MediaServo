//! `host doctor` 子命令测试（Task A3）。
//!
//! 断言三检查（oxmgr 可用 / host.yaml 可解析 / oxfile 可生成）与
//! 退出码 = 失败检查数的语义。环境无关性：oxmgr 是否在 PATH 由测试探测，
//! 期望退出码按探测结果推导（CI 无 oxmgr 也成立）。

use std::path::PathBuf;
use std::process::Command;

fn host_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mediaservo-host")
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("host-doctor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("etc")).unwrap();
    d
}

fn run_doctor(dir: &PathBuf) -> std::process::Output {
    Command::new(host_bin())
        .arg("doctor")
        .arg(dir)
        .output()
        .unwrap()
}

/// oxmgr 存在性探测（spawn 成功即可执行，与 oxmgr 对参数的响应无关）。
fn oxmgr_present() -> bool {
    Command::new("oxmgr").arg("--version").output().is_ok()
}

#[test]
fn doctor_healthy_dir_checks_pass() {
    // oxmgr 缺失（如 CI）时健康全过断言无意义 → 跳过；config 检查仍各自独立。
    if !oxmgr_present() {
        return;
    }
    let dir = tmp_dir("healthy");
    std::fs::write(dir.join("etc").join("host.yaml"), "host:\n  device_id: \"car-01\"\n")
        .unwrap();
    let out = run_doctor(&dir);
    assert!(
        out.status.success(),
        "doctor 应退出 0，实际 {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for needle in ["[ok] oxmgr", "[ok] host.yaml", "[ok] oxfile"] {
        assert!(stdout.contains(needle), "stdout 缺 {needle}:\n{stdout}");
    }
}

#[test]
fn doctor_broken_config_counts_failures() {
    // 语法损坏的 host.yaml：检查 ②③ 恒定失败 → 退出码 = 2 + (oxmgr 缺失 ? 1 : 0)。
    let dir = tmp_dir("broken");
    std::fs::write(dir.join("etc").join("host.yaml"), "[host\nbroken yaml").unwrap();
    let out = run_doctor(&dir);
    let expected = if oxmgr_present() { 2 } else { 3 };
    assert_eq!(
        out.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // 输出必须逐检查标记失败原因（旧二进制无 doctor 子命令时此断言失败）
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[fail] host.yaml"), "stdout 缺 host.yaml 失败标记:\n{stdout}");
    assert!(stdout.contains("[fail] oxfile"), "stdout 缺 oxfile 失败标记:\n{stdout}");
}

#[test]
fn doctor_missing_config_counts_both_config_failures() {
    // 空 etc/（无 host.yaml）：检查 ②③ 均失败且各有一条 [fail] 标记 →
    // 退出码 = 2 + (oxmgr 缺失 ? 1 : 0)，与打印的失败数一致。
    let dir = tmp_dir("missing"); // tmp_dir 只建 etc/，不写 host.yaml
    let out = run_doctor(&dir);
    let expected = if oxmgr_present() { 2 } else { 3 };
    assert_eq!(
        out.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[fail] 读取"), "stdout 缺读取失败标记:\n{stdout}");
    assert!(stdout.contains("[fail] oxfile"), "stdout 缺 oxfile 失败标记:\n{stdout}");
}
