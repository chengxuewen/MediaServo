//! T1: golden fixture 结构门——解析/分类/计数派生（渲染断言随 T2 spec.rs 到场扩展本文件）。
//! 计数**从 fixture 派生**，禁硬写 9/12 陈旧文案（harness 计数不同步教训）。

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
    #[allow(dead_code)] // T2 起消费 input/expect 做渲染断言
    input: Value,
    expect: Value,
}

fn load() -> Fixture {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden.json");
    let raw = std::fs::read_to_string(path).expect("golden.json 必须随 crate 存在");
    serde_json::from_str(&raw).expect("golden.json 结构 = {schema, cases[]}")
}

#[test]
fn golden_fixture_parses_with_derived_counts() {
    let f = load();
    assert_eq!(f.schema, "weaknet-golden/1");
    assert!(!f.cases.is_empty(), "fixture 不得为空");

    let mut bash = 0usize;
    let mut contract = 0usize;
    for c in &f.cases {
        assert!(!c.id.is_empty(), "案 id 非空");
        match c.source.as_str() {
            "bash" => {
                assert!(
                    matches!(c.expect, Value::String(_)),
                    "bash 继承案 expect 必须是渲染字符串: {}",
                    c.id
                );
                bash += 1;
            }
            "new-contract" => {
                assert!(
                    c.expect.get("filters").and_then(Value::as_array).is_some(),
                    "dir 新契约案 expect.filters 为数組: {}",
                    c.id
                );
                contract += 1;
            }
            other => panic!("未知 source {other} @ {}", c.id),
        }
        assert!(
            matches!(c.kind.as_str(), "netem_spec" | "rate_arg" | "legs"),
            "kind 枚举: {}",
            c.id
        );
    }
    assert!(bash >= 1 && contract >= 1, "两族案都必须在场");
    println!(
        "golden fixture: {} 案（bash 继承 {bash} · dir 新契约 {contract}）",
        f.cases.len()
    );
}
