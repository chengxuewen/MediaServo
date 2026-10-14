//! T2: golden fixture rendering gate -- all 12 cases consumed (9 netem_spec/rate_arg bash-inherited + 3 legs dir new-contract).
//! The expect string/structure is law (fixtures/golden.json header note); counts derived from the fixture, never hardcoded.
//! Classification completeness: every case lands in exactly one of the three kinds and is asserted; an orphan case = test failure.

use mediaservo_weaknet::spec::{
    Dir, IfaceKind, ImpairSpec, ScopeSel, build_filter_legs, render_netem_spec, render_rate_arg,
};
use serde_json::Value;

fn load_cases() -> Vec<Case> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden.json");
    let raw = std::fs::read_to_string(path).expect("golden.json must exist alongside the crate");
    let doc: Value = serde_json::from_str(&raw).expect("golden.json must be valid JSON");
    assert_eq!(doc["schema"].as_str(), Some("weaknet-golden/1"), "fixture schema gate");
    doc["cases"]
        .as_array()
        .expect("cases must be an array")
        .iter()
        .map(|c| Case {
            id: c["id"].as_str().expect("id string").to_string(),
            kind: c["kind"].as_str().expect("kind string").to_string(),
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
                    .unwrap_or_else(|e| panic!("{}: input parse failed: {e}", case.id));
                Value::String(render_netem_spec(&spec))
            }
            "rate_arg" => Value::String(render_rate_arg(case.input["rate_mbps"].as_f64())),
            "legs" => {
                let legs = legs_from_input(&case.input)
                    .unwrap_or_else(|e| panic!("{}: leg assembly failed: {e}", case.id));
                serde_json::to_value(&legs).expect("LegFilter must be serializable")
            }
            other => panic!("{}: unknown kind {other} (fixture and tests out of sync)", case.id),
        };
        assert_eq!(got, case.expect, "case {} render does not match expect", case.id);
        *seen.entry(case.kind.as_str()).or_insert(0usize) += 1;
    }
    // derived counts + full classification coverage (at least one case per kind present, guarding the fixture against silent reshaping)
    assert_eq!(seen.len(), 3, "all three kinds must be covered, got {seen:?}");
    assert!(seen["netem_spec"] >= 1 && seen["rate_arg"] >= 1 && seen["legs"] >= 1, "{seen:?}");
    assert_eq!(
        seen.values().sum::<usize>(),
        cases.len(),
        "every case must be consumed (no orphan kind leaks)"
    );
    println!("golden_render: {} cases all asserted {seen:?}", cases.len());
}

fn legs_from_input(input: &Value) -> Result<Value, String> {
    let scope = match req_str(input, "scope")? {
        "media" => ScopeSel::Media,
        "stream" => ScopeSel::Stream,
        "device" => ScopeSel::Device,
        other => return Err(format!("scope invalid: {other}")),
    };
    let dir = match req_str(input, "dir")? {
        "out" => Dir::Out,
        "in" => Dir::In,
        "both" => Dir::Both,
        other => return Err(format!("dir invalid: {other}")),
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
        .ok_or_else(|| format!("missing field or not a string: {key}"))
}

fn opt_u16_array(v: &Value, key: &str) -> Vec<u16> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|n| n.as_u64().expect("port is a number") as u16).collect())
        .unwrap_or_default()
}
