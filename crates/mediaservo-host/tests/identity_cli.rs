//! G4: `host init` generates identity.json (D-H13 instance root, 0600, idempotent - only when missing).

use std::process::Command;

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mediaservo-host"))
}

#[test]
fn init_generates_identity_json_idempotently() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = host().arg("init").arg(dir.path()).output().expect("spawn host init");
    assert!(out.status.success(), "host init failed: {}", String::from_utf8_lossy(&out.stderr));
    let path = dir.path().join("identity.json");
    assert!(
        path.exists(),
        "identity.json should be generated at the instance root {}",
        dir.path().display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "identity.json must be 0600");
    }
    let first: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).expect("identity.json parseable");
    assert!(
        first["device_id"].as_str().unwrap().starts_with("ms-"),
        "device_id should have the ms- prefix"
    );
    // device-enroll new shape: device_id only (the public-key fingerprint is the credential), no secret written
    assert_eq!(
        first.get("device_secret"),
        None,
        "the new-shape identity.json must not contain device_secret"
    );
    assert_eq!(first.as_object().unwrap().len(), 1, "the new shape has exactly one field");

    // Idempotent: repeated init must not overwrite (overwriting invalidates the server-side registration)
    let out2 = host().arg("init").arg(dir.path()).output().expect("spawn host init again");
    assert!(
        out2.status.success(),
        "repeated init failed: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let second: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap())
        .expect("identity.json still parseable");
    assert_eq!(first, second, "repeated init must not overwrite identity.json");
}
