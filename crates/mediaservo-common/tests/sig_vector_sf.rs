//! P1/T1.3 L1 契约夹具回放（sig_vector/sfu/*.json）——跨语言单一真源：
//! 同一 JSON 集由本测试（Rust）与 www/packages/client vitest（TS）各自回放，
//! 断言 wire 形 + 语义字段。锚模式 = device-enroll 轮 devices.rs 钉值常量的文件形升级。

use mediaservo_common::protocol::{ControlAck, ControlEnvelope, SignalingMessage};
use serde_json::Value;

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sig_vector/sfu")
}

/// dot-path（含数组下标）取 JSON 值："capabilities.codecs.0.mimeType"
fn get<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

#[test]
fn sig_vector_fixtures_replay() {
    let dir = fixture_dir();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("sig_vector/sfu 目录存在")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "夹具非空");
    let mut n_signal = 0;
    let mut n_envelope = 0;
    for f in files {
        let raw = std::fs::read_to_string(&f).unwrap();
        let doc: Value = serde_json::from_str(&raw).unwrap();
        let name = doc["name"].as_str().unwrap().to_string();
        let kind = doc["kind"].as_str().unwrap();
        let wire = doc["wire"].as_str().map(String::from).unwrap_or_else(|| doc["wire"].to_string());
        let parsed: Value = serde_json::from_str(&wire).unwrap_or_else(|e| panic!("{name}: wire JSON {e}"));
        match kind {
            "signal" => {
                let msg: SignalingMessage = serde_json::from_value(parsed.clone())
                    .unwrap_or_else(|e| panic!("{name}: SignalingMessage 解析 {e}"));
                // 回列语义：再序列化可解析（枚举闭环）+ wire 含 type 键。
                let back: SignalingMessage =
                    serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
                let _ = back;
                assert_eq!(parsed["type"], Value::String(msg_wire_type(&msg)), "{name}: type 一致");
                n_signal += 1;
            }
            "envelope" => {
                let env: ControlEnvelope = serde_json::from_str(&wire)
                    .unwrap_or_else(|e| panic!("{name}: ControlEnvelope {e}"));
                assert_eq!(env.seq, parsed["seq"].as_u64().unwrap(), "{name}: seq");
                assert_eq!(env.cmd, parsed["cmd"].as_str().unwrap(), "{name}: cmd");
                if parsed.get("payload").is_none() {
                    assert_eq!(env.payload, serde_json::json!({}), "{name}: 缺省 payload 空对象");
                }
                n_envelope += 1;
            }
            "ack" => {
                let ack: ControlAck =
                    serde_json::from_str(&wire).unwrap_or_else(|e| panic!("{name}: ControlAck {e}"));
                assert_eq!(ack.ack, parsed["ack"].as_u64().unwrap(), "{name}: ack");
                n_envelope += 1;
            }
            other => panic!("未知夹具 kind: {other}"),
        }
        for (path, want) in doc["checks"].as_object().unwrap() {
            if path == "type" {
                continue; // 已在 signal 分支核对
            }
            let got = get(&parsed, path).unwrap_or_else(|| panic!("{name}: 缺检查路径 {path}"));
            assert_eq!(got, want, "{name}: {path}");
        }
    }
    assert!(n_signal >= 6 && n_envelope >= 5, "覆盖面：signal={n_signal} envelope/ack={n_envelope}");
}

/// SignalingMessage → wire type 名（serde snake_case tag）。
fn msg_wire_type(m: &SignalingMessage) -> String {
    let v = serde_json::to_value(m).unwrap();
    v["type"].as_str().unwrap_or_default().to_string()
}
