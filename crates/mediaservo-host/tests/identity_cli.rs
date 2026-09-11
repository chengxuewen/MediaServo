//! G4: `host init` 生成 identity.json（D-H13 实例根，0600，幂等——仅缺失时生成）。

use std::process::Command;

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mediaservo-host"))
}

#[test]
fn init_generates_identity_json_idempotently() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = host().arg("init").arg(dir.path()).output().expect("spawn host init");
    assert!(
        out.status.success(),
        "host init 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = dir.path().join("identity.json");
    assert!(path.exists(), "identity.json 应生成于实例根 {}", dir.path().display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "identity.json 必须 0600");
    }
    let first: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).expect("identity.json 可解析");
    assert!(first["device_id"].as_str().unwrap().starts_with("ms-"), "device_id 应 ms- 前缀");
    // device-enroll 新形状：仅 device_id（公钥指纹即凭据），不再写 secret
    assert_eq!(first.get("device_secret"), None, "新形状 identity.json 不得含 device_secret");
    assert_eq!(first.as_object().unwrap().len(), 1, "新形状仅一个字段");

    // 幂等：重复 init 不得覆盖（覆盖会使 server 侧注册失效）
    let out2 = host().arg("init").arg(dir.path()).output().expect("spawn host init again");
    assert!(out2.status.success(), "重复 init 失败: {}", String::from_utf8_lossy(&out2.stderr));
    let second: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).expect("identity.json 仍可解析");
    assert_eq!(first, second, "重复 init 不得覆盖 identity.json");
}
