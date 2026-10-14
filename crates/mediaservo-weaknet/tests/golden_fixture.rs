//! T1: golden fixture structure gate -- parsing/classification/derived counts (rendering asserts extend this file when T2 spec.rs arrives).
//! Counts are **derived from the fixture**, never hardcoded 9/12 stale text (the harness count-desync lesson).

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    schema: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    source: String,
    kind: String,
    #[allow(dead_code)] // from T2 on, input/expect feed rendering asserts
    input: Value,
    expect: Value,
}

fn load() -> Fixture {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden.json");
    let raw = std::fs::read_to_string(path).expect("golden.json must exist alongside the crate");
    serde_json::from_str(&raw).expect("golden.json structure = {schema, cases[]}")
}

#[test]
fn golden_fixture_parses_with_derived_counts() {
    let f = load();
    assert_eq!(f.schema, "weaknet-golden/1");
    assert!(!f.cases.is_empty(), "fixture must not be empty");

    let mut bash = 0usize;
    let mut contract = 0usize;
    for c in &f.cases {
        assert!(!c.id.is_empty(), "case id non-empty");
        match c.source.as_str() {
            "bash" => {
                assert!(
                    matches!(c.expect, Value::String(_)),
                    "a bash-inherited case's expect must be a rendered string: {}",
                    c.id
                );
                bash += 1;
            }
            "new-contract" => {
                assert!(
                    c.expect.get("filters").and_then(Value::as_array).is_some(),
                    "a dir-new-contract case's expect.filters must be an array: {}",
                    c.id
                );
                contract += 1;
            }
            other => panic!("unknown source {other} @ {}", c.id),
        }
        assert!(
            matches!(c.kind.as_str(), "netem_spec" | "rate_arg" | "legs"),
            "kind enum: {}",
            c.id
        );
    }
    assert!(bash >= 1 && contract >= 1, "both case families must be present");
    println!(
        "golden fixture: {} cases (bash inherited {bash} · dir new-contract {contract})",
        f.cases.len()
    );
}
