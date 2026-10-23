//! config-validate — port of the retired python `config validate` face
//! (SR6 batch-beta-1 F3; N4 pattern: thin bash driver + in-workspace Rust
//! checker). Verdict parity with `mediaservo_cli.py::_cmd_config` (validate
//! branch, pyyaml twin):
//!
//! | case                | stream | line                       | effect     |
//! |---------------------|--------|----------------------------|------------|
//! | parses cleanly      | stdout | `OK: <rel-path>`           |            |
//! | file absent         | stderr | `missing: <rel-path>`      | fail       |
//! | YAML invalid        | stderr | `YAML error: <rel>: <err>` | fail       |
//!
//! rc 0 iff every checked file is OK (python `sys.exit(0 if ok else 1)`).
//! The two dev-track template paths are repo-root-relative (python anchored
//! them on `ROOT`; the bash driver passes `--root "$ROOT"`).

/// The dev-track template pair (python HOST_CONF/SERVER_YAML, verbatim).
pub const FILES: [&str; 2] =
    ["crates/mediaservo-host/config/host.conf", "config/server.docker.yaml"];

pub fn check(root: &std::path::Path) -> i32 {
    let mut ok = true;
    for rel in FILES {
        let path = root.join(rel);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                // Option<Value> models yaml.safe_load's empty-file None
                // (python: an empty template parses OK); a non-empty invalid
                // document errors on both sides.
                match serde_yaml::from_str::<Option<serde_yaml::Value>>(&text) {
                    Ok(_) => println!("OK: {rel}"),
                    Err(e) => {
                        eprintln!("YAML error: {rel}: {e}");
                        ok = false;
                    }
                }
            }
            Err(_) => {
                eprintln!("missing: {rel}");
                ok = false;
            }
        }
    }
    if ok { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn seed(root: &Path, good: bool) {
        std::fs::create_dir_all(root.join("crates/mediaservo-host/config")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join(FILES[0]),
            "server:\n  signaling_url: \"ws://localhost:9800/ws\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join(FILES[1]),
            if good { "listen: \"0.0.0.0:9800\"\n" } else { "a: [unclosed\n" },
        )
        .unwrap();
    }

    #[test]
    fn ok_pair_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), true);
        assert_eq!(check(dir.path()), 0);
    }

    #[test]
    fn invalid_yaml_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), false);
        assert_eq!(check(dir.path()), 1);
    }

    #[test]
    fn missing_file_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join(FILES[1]), "listen: 1\n").unwrap();
        assert_eq!(check(dir.path()), 1);
    }
}
