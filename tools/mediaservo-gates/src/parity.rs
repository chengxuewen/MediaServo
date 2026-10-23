//! Bindings version parity gate — port of `scripts/gate-parity.py` (C43③).
//!
//! Verdict rules (migrated verbatim):
//! 1. Truth source = `cargo metadata --no-deps --format-version 1` run in the
//!    cwd; the delivery-domain version is the `mediaservo-field` package
//!    version (F12: bindings align with field, NOT the workspace literal).
//!    Metadata failure / missing package / unparsable JSON = rc 1 (python
//!    ValueError / StopIteration tracebacks).
//! 2. Three hand-copies must carry that version as an EXACT pattern (python
//!    substring `in` test, not a line rule):
//!      - bindings/python/mediaservo/pyproject.toml            `version = "{v}"`
//!      - bindings/python/mediaservo/mediaservo/__init__.py    `__version__ = "{v}"`
//!      - bindings/node/package.json                            `"version": "{v}"`
//!        A missing/unreadable file counts as failing that copy (python open()
//!        traceback = rc 1 too; the message shape stays `{path} != {v}`).
//! 3. Pass face: stdout `bindings parity ok ({v})` rc 0; fail face: the
//!    flagged list joined with newlines on stderr, rc 1.

use std::path::Path;

const COPIES: [(&str, &str); 3] = [
    ("bindings/python/mediaservo/pyproject.toml", "version = \""),
    ("bindings/python/mediaservo/mediaservo/__init__.py", "__version__ = \""),
    ("bindings/node/package.json", "\"version\": \""),
];

/// Rule 2 as a pure function over (root, version) — fixture-testable.
pub fn check_copies(root: &Path, v: &str) -> Vec<String> {
    COPIES
        .iter()
        .filter(|(path, prefix)| {
            let pat = format!("{prefix}{v}\"");
            match std::fs::read_to_string(root.join(path)) {
                Ok(text) => !text.contains(&pat),
                Err(_) => true, // unreadable = stale copy for verdict purposes
            }
        })
        .map(|(path, _)| format!("{path} != {v}"))
        .collect()
}

/// Rule 1: field-domain version via cargo metadata (same subprocess the
/// python used — one truth source, no second Cargo.toml parser).
pub fn field_version() -> Result<String, String> {
    let out = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cargo metadata failed to run: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata exited nonzero".to_string());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata JSON: {e}"))?;
    json["packages"]
        .as_array()
        .and_then(|pkgs| {
            pkgs.iter().find_map(|p| {
                (p["name"].as_str() == Some("mediaservo-field"))
                    .then(|| p["version"].as_str().map(str::to_string))
                    .flatten()
            })
        })
        .ok_or_else(|| "mediaservo-field package not found in cargo metadata".to_string())
}

/// Face: rc table in lib.rs doc.
pub fn check(root: &Path) -> i32 {
    let v = match field_version() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return 1;
        }
    };
    let bad = check_copies(root, &v);
    if bad.is_empty() {
        println!("bindings parity ok ({v})");
        0
    } else {
        eprintln!("{}", bad.join("\n"));
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(pyproject: &str, init: &str, pkgjson: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("bindings/python/mediaservo/mediaservo")).unwrap();
        std::fs::create_dir_all(dir.path().join("bindings/node")).unwrap();
        std::fs::write(dir.path().join("bindings/python/mediaservo/pyproject.toml"), pyproject)
            .unwrap();
        std::fs::write(dir.path().join("bindings/python/mediaservo/mediaservo/__init__.py"), init)
            .unwrap();
        std::fs::write(dir.path().join("bindings/node/package.json"), pkgjson).unwrap();
        dir
    }

    #[test]
    fn green_when_all_three_carry_the_version() {
        let dir = tree(
            "[project]\nversion = \"1.2.3\"\n",
            "__version__ = \"1.2.3\"\n",
            "{ \"name\": \"x\", \"version\": \"1.2.3\" }\n",
        );
        assert!(check_copies(dir.path(), "1.2.3").is_empty());
    }

    #[test]
    fn planted_drift_flags_exactly_the_stale_copy() {
        let dir = tree(
            "[project]\nversion = \"1.2.3\"\n",
            "__version__ = \"9.9.9\"\n", // planted stale
            "{ \"version\": \"1.2.3\" }\n",
        );
        assert_eq!(
            check_copies(dir.path(), "1.2.3"),
            ["bindings/python/mediaservo/mediaservo/__init__.py != 1.2.3"]
        );
    }

    #[test]
    fn prefix_mismatch_is_stale_too() {
        // `__version__ = "1.2.3"` must appear with the EXACT pattern — a bare
        // `1.2.3` elsewhere does not satisfy it (python substring test pinned)
        let dir = tree("version = 1.2.3\n", "__version__ = '1.2.3'\n", "{ \"version\": 1.2.3 }\n");
        assert_eq!(check_copies(dir.path(), "1.2.3").len(), 3);
    }

    #[test]
    fn missing_file_counts_as_stale() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(check_copies(empty.path(), "1.0.0").len(), 3);
    }
}
