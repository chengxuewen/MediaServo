//! Task C4: `host token issue` CLI tests - minimal token issuance (G1 prerequisite; C-phase e2e needs it).
//!
//! Flow: `host init <dir>` (generates etc/link/signing.pem PKCS#8) ->
//! `host token issue --role <R> --node <id> [--topic <T>...] --out <path> [<dir>]
//! -> TokenFile::decode verifies the signature + claims assertions (role/node/ACL correct).
//!
//! Negatives: unknown role exit 2, missing signing.pem exit 1, missing required args exit 2.

use std::path::Path;
use std::process::{Command, Output};

use mediaservo_link::{Claims, Role, TokenFile};

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mediaservo-host"))
}

/// `host init <dir>` generates etc/host.toml + etc/link/signing.pem.
fn init(dir: &Path) {
    let out = host().arg("init").arg(dir).output().expect("spawn host init");
    assert!(out.status.success(), "host init failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn issue(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = host();
    cmd.arg("token").arg("issue");
    for a in args {
        cmd.arg(a);
    }
    cmd.arg(dir);
    cmd.output().expect("spawn host token issue")
}

fn decode_claims(path: &Path) -> Claims {
    let bytes = std::fs::read(path).expect("read token file");
    let (_vk, token) =
        TokenFile::decode(&bytes).expect("token file decode should pass verification");
    token.verify(&_vk).expect("verify claims")
}

#[test]
fn issue_capture_token_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let tok = dir.path().join("cam0.token");

    let out = issue(
        dir.path(),
        &[
            "--role",
            "capture",
            "--node",
            "cap-1",
            "--topic",
            "camera/cam0",
            "--out",
            tok.to_str().expect("utf8"),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(tok.exists(), "the token file should have been written");

    let claims = decode_claims(&tok);
    assert_eq!(claims.role, Role::Capture);
    assert_eq!(claims.node_id, "cap-1");
    assert_eq!(claims.acl.publish_allow, vec!["camera/cam0"]);
    assert!(claims.acl.subscribe_allow.is_empty(), "capture has no subscribe rights");
    assert!(claims.exp > 0, "exp should be filled");
}

#[test]
fn issue_defaults_topics_per_role_matrix() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let tok = dir.path().join("cap.token");

    // no --topic -> ACL matrix default (Capture publishes camera/*)
    let out = issue(
        dir.path(),
        &["--role", "capture", "--node", "n", "--out", tok.to_str().expect("utf8")],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let claims = decode_claims(&tok);
    assert_eq!(claims.acl.publish_allow, vec!["camera/*"]);
}

#[test]
fn issue_subscribe_role_puts_topics_in_subscribe_allow() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let tok = dir.path().join("streamer.token");

    // streamer role: pusher (subscribes camera/*, publishes stats/*)
    let out = issue(
        dir.path(),
        &[
            "--role",
            "pusher",
            "--node",
            "stream-1",
            "--topic",
            "camera/cam0",
            "--out",
            tok.to_str().expect("utf8"),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let claims = decode_claims(&tok);
    assert_eq!(claims.role, Role::Pusher);
    assert_eq!(claims.acl.publish_allow, vec!["stats/*"], "pusher publish right for stats/*");
    assert_eq!(claims.acl.subscribe_allow, vec!["camera/cam0"]);
}

#[test]
fn issue_recorder_defaults_to_matrix_subscribe() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let tok = dir.path().join("recorder.token");

    let out = issue(
        dir.path(),
        &["--role", "recorder", "--node", "rec-1", "--out", tok.to_str().expect("utf8")],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let claims = decode_claims(&tok);
    assert_eq!(claims.role, Role::Recorder);
    assert_eq!(claims.acl.subscribe_allow, vec!["camera/*", "video/*", "vision/*"]);
}

#[test]
fn issue_invalid_role_exits_2_with_clear_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let tok = dir.path().join("bad.token");
    let out = issue(
        dir.path(),
        &["--role", "bogus", "--node", "n", "--out", tok.to_str().expect("utf8")],
    );
    assert_eq!(out.status.code(), Some(2), "unknown role should exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("bogus"), "stderr should name the role, got: {stderr}");
    assert!(!tok.exists(), "no file should be written on failure");
}

#[test]
fn issue_missing_signing_pem_exits_1_with_hint() {
    let dir = tempfile::tempdir().expect("tempdir");
    // no host init -> no signing.pem
    let tok = dir.path().join("t.token");
    let out = issue(
        dir.path(),
        &["--role", "capture", "--node", "n", "--out", tok.to_str().expect("utf8")],
    );
    assert_eq!(out.status.code(), Some(1), "missing signing.pem should exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("signing.pem"), "stderr should name the file, got: {stderr}");
    assert!(!tok.exists());
}

#[test]
fn issue_missing_required_args_exit_2() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    // missing --role / missing --node / missing --out -> exit 2
    for args in [
        vec!["--node", "n", "--out", "x.token"],
        vec!["--role", "capture", "--out", "x.token"],
        vec!["--role", "capture", "--node", "n"],
    ] {
        let out = issue(dir.path(), &args);
        assert_eq!(out.status.code(), Some(2), "args {args:?} should exit 2");
    }
}

/// C4 review: capability token files must be 0600 (credential protection on par with signing.pem).
#[test]
fn issue_token_file_is_0600() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        init(dir.path());
        let tok = dir.path().join("cam0.token");
        let out = issue(
            dir.path(),
            &[
                "--role",
                "capture",
                "--node",
                "cap-1",
                "--topic",
                "camera/cam0",
                "--out",
                tok.to_str().expect("utf8"),
            ],
        );
        assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        let mode = std::fs::metadata(&tok).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "the token file must be 0600 (capability credential), got {mode:o}"
        );
    }
}

/// C4 review: empty --node/--topic strings are rejected (exit 2 + explicit error).
#[test]
fn issue_rejects_empty_node_and_topic() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    for args in [
        vec!["--role", "capture", "--node", "", "--out", "x.token"],
        vec!["--role", "pusher", "--node", "n", "--topic", "", "--out", "x.token"],
    ] {
        let out = issue(dir.path(), &args);
        assert_eq!(out.status.code(), Some(2), "args {args:?} with an empty value should exit 2");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.is_empty(), "empty values must error");
    }
}

// -- G1: --all standard set / --for-ros / --ttl / validation / audit --

/// G1: `host token issue --all` issues the standard vehicle token set from host.toml (skips existing ones,
/// D-H10 fixed tokens). Template config = cam0 + cam0-stream.
#[test]
fn issue_all_standard_set_with_correct_claims() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let out = issue(dir.path(), &["--all"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    // etc/link/<cam>.token: Capture only publishes camera/<cam>
    let cam = decode_claims(&dir.path().join("etc/link/cam0.token"));
    assert_eq!(cam.role, Role::Capture);
    assert_eq!(cam.node_id, "host-capturer-cam0");
    assert_eq!(cam.acl.publish_allow, vec!["camera/cam0"]);
    assert!(cam.acl.subscribe_allow.is_empty());

    // etc/link/<stream>.token: Pusher subscribes camera/<cam> + vision/<cam> (F3 vision DC)
    let stream = decode_claims(&dir.path().join("etc/link/cam0-stream.token"));
    assert_eq!(stream.role, Role::Pusher);
    assert_eq!(stream.node_id, "host-streamer-cam0-stream");
    assert_eq!(stream.acl.subscribe_allow, vec!["camera/cam0", "vision/cam0"]);
    assert_eq!(stream.acl.publish_allow, vec!["stats/*"]);

    // etc/link/recorder.token: Recorder matrix default (subscribes camera/video/vision + publishes stats)
    let rec = decode_claims(&dir.path().join("etc/link/recorder.token"));
    assert_eq!(rec.role, Role::Recorder);
    assert_eq!(rec.node_id, "host-recorder");
    assert_eq!(rec.acl.subscribe_allow, vec!["camera/*", "video/*", "vision/*"]);
    assert_eq!(rec.acl.publish_allow, vec!["stats/*"]);

    // etc/link/agent.token: Monitor matrix default (subscribes camera/* + stats/*, no publish)
    let agent = decode_claims(&dir.path().join("etc/link/agent.token"));
    assert_eq!(agent.role, Role::Monitor);
    assert_eq!(agent.node_id, "host-agent");
    assert_eq!(agent.acl.subscribe_allow, vec!["camera/*", "stats/*"]);
    assert!(agent.acl.publish_allow.is_empty());

    // the ROS token is not auto-issued (explicit --for-ros)
    assert!(!dir.path().join("etc/link/ros-vision.token").exists());
}

#[test]
fn issue_all_skips_existing_tokens() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    // pre-issue cam0.token (custom node) -> --all must not overwrite it (D-H10 fixed tokens)
    let tok = dir.path().join("etc/link/cam0.token");
    let out = issue(
        dir.path(),
        &[
            "--role",
            "capture",
            "--node",
            "custom-node",
            "--topic",
            "camera/cam0",
            "--out",
            tok.to_str().expect("utf8"),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let out = issue(dir.path(), &["--all"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let cam = decode_claims(&tok);
    assert_eq!(cam.node_id, "custom-node", "--all must not overwrite existing tokens");
}

#[test]
fn issue_all_missing_host_toml_exits_1() {
    let dir = tempfile::tempdir().expect("tempdir");
    // no host init -> no host.toml/signing.pem
    let out = issue(dir.path(), &["--all"]);
    assert_eq!(out.status.code(), Some(1), "missing host.toml should exit 1");
}

#[test]
fn issue_for_ros_preset_issues_perception_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let out = issue(dir.path(), &["--for-ros"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    // the path matches token_path in ros_bridge.yaml
    let tok = dir.path().join("etc/link/ros-vision.token");
    assert!(tok.exists(), "ros-vision.token should have been written");
    let claims = decode_claims(&tok);
    assert_eq!(claims.role, Role::Perception);
    assert_eq!(claims.node_id, "ros-vision");
    assert_eq!(claims.acl.publish_allow, vec!["perception/*", "vision/*"]);
    assert_eq!(claims.acl.subscribe_allow, vec!["camera/*"]);
}

#[test]
fn issue_for_ros_conflicts_with_explicit_args() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let out =
        issue(dir.path(), &["--for-ros", "--role", "capture", "--node", "n", "--out", "x.token"]);
    assert_eq!(out.status.code(), Some(2), "--for-ros conflicts with explicit --role/--node/--out");
}

/// G1 hardening: an explicit --topic must stay within the role's ACL matrix (over-privilege rejected).
#[test]
fn issue_rejects_topic_outside_role_matrix() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    // capture is camera/* only - vision/cam0 is over-privilege
    let out = issue(
        dir.path(),
        &["--role", "capture", "--node", "n", "--topic", "vision/cam0", "--out", "x.token"],
    );
    assert_eq!(out.status.code(), Some(2), "capture publishing vision/* must be rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("vision/cam0"),
        "stderr should name the over-privilege topic, got: {stderr}"
    );
    // pusher subscribes camera/video/vision only - stats/x is over-privilege
    let out = issue(
        dir.path(),
        &["--role", "pusher", "--node", "n", "--topic", "stats/x", "--out", "y.token"],
    );
    assert_eq!(out.status.code(), Some(2), "pusher subscribing stats/* must be rejected");
    assert!(!dir.path().join("x.token").exists(), "no file should be written on failure");
}

/// G1: --node charset guard (same rule as host.toml ids: [A-Za-z0-9_-]+, prevents path traversal / malformed claims).
#[test]
fn issue_rejects_node_with_invalid_chars() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let out = issue(dir.path(), &["--role", "capture", "--node", "bad/node", "--out", "x.token"]);
    assert_eq!(out.status.code(), Some(2), "a node containing / must be rejected");
    let out = issue(dir.path(), &["--role", "capture", "--node", "sp ace", "--out", "x.token"]);
    assert_eq!(out.status.code(), Some(2), "a node containing a space must be rejected");
}

/// G1: --ttl <secs> overrides the default 10 years (for testing).
#[test]
fn issue_ttl_override_respected() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let tok = dir.path().join("ttl.token");
    let out = issue(
        dir.path(),
        &[
            "--role",
            "capture",
            "--node",
            "n",
            "--topic",
            "camera/cam0",
            "--out",
            tok.to_str().expect("utf8"),
            "--ttl",
            "3600",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let claims = decode_claims(&tok);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock");
    let now = now.as_secs();
    assert!(
        claims.exp > now && claims.exp <= now + 3600 + 60,
        "ttl=3600 should take effect, exp={}",
        claims.exp
    );
}

#[test]
fn issue_ttl_rejects_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let out =
        issue(dir.path(), &["--role", "capture", "--node", "n", "--out", "x.token", "--ttl", "0"]);
    assert_eq!(out.status.code(), Some(2), "ttl=0 must be rejected");
}

/// G1: every issuance writes an audit line (etc/link/issuance.jsonl JSONL, D-H10 audit discipline).
#[test]
fn issue_writes_audit_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    init(dir.path());
    let tok = dir.path().join("audit.token");
    let out = issue(
        dir.path(),
        &[
            "--role",
            "pusher",
            "--node",
            "audit-1",
            "--topic",
            "camera/cam0",
            "--out",
            tok.to_str().expect("utf8"),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let log = std::fs::read_to_string(dir.path().join("etc/link/issuance.jsonl"))
        .expect("issuance.jsonl should exist");
    let line = log.lines().last().expect("at least one audit line");
    let v: serde_json::Value = serde_json::from_str(line).expect("the audit line should parse");
    assert_eq!(v["role"], "pusher");
    assert_eq!(v["node"], "audit-1");
    assert_eq!(v["topics"], serde_json::json!(["camera/cam0"]));
    assert_eq!(v["out"], serde_json::json!(tok.to_string_lossy()));
    assert!(v["ts"].as_u64().unwrap() > 0, "ts should be filled");
    assert_eq!(v["ttl"], serde_json::json!(10 * 365 * 24 * 3600), "the default ttl = 10 years");
}
