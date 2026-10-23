//! OpenAPI spec validation — port of `scripts/gate-openapi.py`.
//!
//! Verdict rules (migrated verbatim from the 8-line python face):
//! 1. docs/openapi.yaml must parse as a YAML mapping (python `yaml.safe_load`
//!    + `.get` — a non-mapping document raises = rc 1; missing/parse error too).
//! 2. `spec["openapi"] == "3.0.3"` (string equality — python `assert`, rc 1).
//! 3. `spec.get("info", {})["title"]` truthy (python truthiness: null/""/0/
//!    false/[]/{} are falsy) — rc 1 otherwise. `info` present but not a
//!    mapping = python `.get` AttributeError = rc 1.
//! 4. Same for `info.version`.
//! 5. Pass face: stdout `openapi ok: {N} paths` where N = len(spec["paths"])
//!    (mapping or sequence length; key absent counts 0; a non-len type or null
//!    = python len() TypeError = rc 1).
//!
//! Note: the old ci.yml inline twin additionally walked per-operation
//! `responses`; the .py execution face never did (its docstring claimed
//! "verbatim-synced" while the copies had drifted). The Rust face ports the
//! .py rules — the responses walk was never part of the executed gate; if it
//! should exist, it lands as a NEW ruled rule, not a silent port.

use serde_yaml::Value;
use std::path::Path;

/// Rules 1-5 as a pure function over the YAML text (fixture-testable).
/// Err = the fail message (python assertion / TypeError class names kept).
pub fn validate(text: &str) -> Result<String, String> {
    let spec: Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(e) => return Err(format!("not a valid YAML document: {e}")),
    };
    let spec = match &spec {
        Value::Mapping(m) => m,
        _ => return Err("not OpenAPI 3.0.3".to_string()), // python .get on non-dict
    };
    if spec.get(Value::from("openapi")).and_then(Value::as_str) != Some("3.0.3") {
        return Err("not OpenAPI 3.0.3".to_string());
    }
    let info = match spec.get(Value::from("info")) {
        Some(Value::Mapping(m)) => m.clone(),
        Some(_) => return Err("missing info.title".to_string()), // .get on non-mapping
        None => serde_yaml::Mapping::new(),
    };
    for key in ["title", "version"] {
        let present = info.get(Value::from(key)).is_some_and(is_truthy);
        if !present {
            return Err(format!("missing info.{key}"));
        }
    }
    let n = match spec.get(Value::from("paths")) {
        None => 0,
        Some(Value::Mapping(m)) => m.len(),
        Some(Value::Sequence(s)) => s.len(),
        Some(_) => return Err("paths is not a mapping".to_string()), // len() TypeError
    };
    Ok(format!("openapi ok: {n} paths"))
}

/// python truthiness over YAML scalars/collections.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Sequence(seq) => !seq.is_empty(),
        Value::Mapping(m) => !m.is_empty(),
        _ => true,
    }
}

/// Face: rc table in lib.rs doc (stdout pass / stderr fail).
pub fn check(root: &Path) -> i32 {
    let text = match std::fs::read_to_string(root.join("docs/openapi.yaml")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read docs/openapi.yaml: {e}");
            return 1;
        }
    };
    match validate(&text) {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(msg) => {
            eprintln!("FAIL: {msg}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const OK_YAML: &str = "openapi: 3.0.3\ninfo:\n  title: MediaServo API\n  version: 0.1.2\npaths:\n  /ready:\n    get:\n      responses:\n        '200':\n          description: ok\n  /health:\n    get:\n      description: x\n";

    fn tree_with(yaml: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/openapi.yaml"), yaml).unwrap();
        dir
    }

    #[test]
    fn green_counts_paths() {
        assert_eq!(validate(OK_YAML).unwrap(), "openapi ok: 2 paths");
        let dir = tree_with(OK_YAML);
        assert_eq!(check(dir.path()), 0);
    }

    #[test]
    fn version_and_shape_rules() {
        assert_eq!(
            validate("openapi: 3.1.0\ninfo: {title: t, version: v}\n").unwrap_err(),
            "not OpenAPI 3.0.3"
        );
        assert_eq!(validate("info: {title: t, version: v}\n").unwrap_err(), "not OpenAPI 3.0.3");
        // unquoted 3.0.3 parses as string; openapi: 3.0 parses as float -> fail
        assert_eq!(
            validate("openapi: 3.0\ninfo: {title: t, version: v}\n").unwrap_err(),
            "not OpenAPI 3.0.3"
        );
        assert!(validate("openapi: \"3.0.3\"\ninfo: {title: t, version: v}\n").is_ok()); // paths absent = 0
    }

    #[test]
    fn info_rules() {
        assert_eq!(validate("openapi: 3.0.3\n").unwrap_err(), "missing info.title");
        assert_eq!(
            validate("openapi: 3.0.3\ninfo: {title: '', version: v}\n").unwrap_err(),
            "missing info.title"
        );
        assert_eq!(
            validate("openapi: 3.0.3\ninfo: {title: t}\n").unwrap_err(),
            "missing info.version"
        );
        assert_eq!(validate("openapi: 3.0.3\ninfo: scalar\n").unwrap_err(), "missing info.title");
        assert_eq!(
            validate("openapi: 3.0.3\ninfo: {title: t, version: 0}\n").unwrap_err(),
            "missing info.version"
        );
    }

    #[test]
    fn paths_and_parse_rules() {
        assert_eq!(
            validate("openapi: 3.0.3\ninfo: {title: t, version: v}\npaths: nope\n").unwrap_err(),
            "paths is not a mapping"
        );
        assert!(validate("openapi: 3.0.3\ninfo: {title: t, version: v}\npaths: []\n").is_ok());
        assert!(validate("openapi: 3.0.3\n\t bogus indent here").is_err());
        // empty document -> None -> python .get AttributeError -> fail rc1
        assert!(validate("").is_err());
    }

    #[test]
    fn cli_shapes() {
        let dir = tree_with("openapi: 2.0\n");
        assert_eq!(check(dir.path()), 1);
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(check(empty.path()), 1); // missing file
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("docs/openapi.yaml"))
            .unwrap();
        writeln!(f).unwrap();
        assert_eq!(check(dir.path()), 1); // still bad after touch
    }
}
