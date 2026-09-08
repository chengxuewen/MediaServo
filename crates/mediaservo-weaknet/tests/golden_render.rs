//! T2: golden fixture 渲染门——12 案全消费（9 netem_spec/rate_arg bash 继承 + 3 legs dir 新契约）。
//! expect 字符串/结构为法（fixtures/golden.json 头注）；计数从 fixture 派生，禁硬写。
//! 分类完整性：每案必落三 kind 之一且被断言，孤儿案 = 测试失败。

use mediaservo_weaknet::spec::{
    Dir, IfaceKind, ImpairSpec, ScopeSel, build_filter_legs, render_netem_spec, render_rate_arg,
};
use serde_json::Value;

fn load_cases() -> Vec<Case> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden.json");
    let raw = std::fs::read_to_string(path).expect("golden.json 必须随 crate 存在");
    let doc: Value = serde_json::from_str(&raw).expect("golden.json 必须是合法 JSON");
    assert_eq!(
        doc["schema"].as_str(),
        Some("weaknet-golden/1"),
        "fixture schema 门"
    );
    doc["cases"]
        .as_array()
        .expect("cases 为数组")
        .iter()
        .map(|c| Case {
            id: c["id"].as_str().expect("id 字符串").to_string(),
            kind: c["kind"].as_str().expect("kind 字符串").to_string(),
            input: c["input"].clone(),
            expect: c["expect"].clone(),
        })
        .collect()
}

struct Case {
    id: String,
    kind: String,
    input: Value,
    expect: Value,
}

#[test]
fn golden_all_cases_render() {
    let cases = load_cases();
    let mut seen = std::collections::BTreeMap::new();
    for case in &cases {
        let got: Value = match case.kind.as_str() {
            "netem_spec" => {
                let spec = ImpairSpec::from_flat_json(&case.input)
                    .unwrap_or_else(|e| panic!("{}: input 解析失败: {e}", case.id));
                Value::String(render_netem_spec(&spec))
            }
            "rate_arg" => Value::String(render_rate_arg(
                case.input["rate_mbps"].as_f64(),
            )),
            "legs" => {
                let legs = legs_from_input(&case.input)
                    .unwrap_or_else(|e| panic!("{}: 腿装配失败: {e}", case.id));
                serde_json::to_value(&legs).expect("LegFilter 必可序列化")
            }
            other => panic!("{}: 未知 kind {other}（fixture 与测试脱节）", case.id),
        };
        assert_eq!(got, case.expect, "案 {} 渲染不符 expect", case.id);
        *seen.entry(case.kind.as_str()).or_insert(0usize) += 1;
    }
    // 派生计数 + 分类全覆盖（每 kind 至少一案在场，防 fixture 静默改形）
    assert_eq!(seen.len(), 3, "三 kind 必须全覆盖，实得 {seen:?}");
    assert!(
        seen["netem_spec"] >= 1 && seen["rate_arg"] >= 1 && seen["legs"] >= 1,
        "{seen:?}"
    );
    assert_eq!(
        seen.values().sum::<usize>(),
        cases.len(),
        "每案必被消费（无孤儿 kind 泄漏）"
    );
    println!(
        "golden_render: {} 案全断言 {seen:?}",
        cases.len()
    );
}

fn legs_from_input(input: &Value) -> Result<Value, String> {
    let scope = match req_str(input, "scope")? {
        "media" => ScopeSel::Media,
        "stream" => ScopeSel::Stream,
        "device" => ScopeSel::Device,
        other => return Err(format!("scope 非法: {other}")),
    };
    let dir = match req_str(input, "dir")? {
        "out" => Dir::Out,
        "in" => Dir::In,
        "both" => Dir::Both,
        other => return Err(format!("dir 非法: {other}")),
    };
    let iface = match req_str(input, "iface")? {
        "lo" => IfaceKind::Loopback,
        _ => IfaceKind::Physical,
    };
    let ports = opt_u16_array(input, "local_ports");
    let pairs: Vec<(u16, u16)> = input["pairs"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| {
                    (
                        p["local"].as_u64().expect("pair.local") as u16,
                        p["remote"].as_u64().expect("pair.remote") as u16,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let legs = build_filter_legs(scope, iface, dir, &ports, &pairs)?;
    let filters = serde_json::to_value(&legs).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "filters": filters }))
}

fn req_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("缺字段或非字符串: {key}"))
}

fn opt_u16_array(v: &Value, key: &str) -> Vec<u16> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|n| n.as_u64().expect("端口为数字") as u16)
                .collect()
        })
        .unwrap_or_default()
}
