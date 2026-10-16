//! P1/T1.3 L1 contract fixture replay (sig_vector/sfu/*.json) - cross-language single source:
//! the same JSON set is replayed by this test (Rust) and by www/packages/client vitest (TS),
//! asserting wire shape + semantic fields. Anchor pattern = the device-enroll devices.rs pinned constants, upgraded to a file form.
use mediaservo_common::protocol::{ControlAck, ControlEnvelope, SignalingMessage};
use serde_json::Value;

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sig_vector/sfu")
}

/// Look up a JSON value by dot-path (array indices included): "capabilities.codecs.0.mimeType"
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
        .expect("sig_vector/sfu directory exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "fixtures not empty");
    let mut n_signal = 0;
    let mut n_envelope = 0;
    for f in files {
        let raw = std::fs::read_to_string(&f).unwrap();
        let doc: Value = serde_json::from_str(&raw).unwrap();
        let name = doc["name"].as_str().unwrap().to_string();
        let kind = doc["kind"].as_str().unwrap();
        let wire =
            doc["wire"].as_str().map(String::from).unwrap_or_else(|| doc["wire"].to_string());
        let parsed: Value =
            serde_json::from_str(&wire).unwrap_or_else(|e| panic!("{name}: wire JSON {e}"));
        match kind {
            "signal" => {
                let msg: SignalingMessage = serde_json::from_value(parsed.clone())
                    .unwrap_or_else(|e| panic!("{name}: SignalingMessage parse {e}"));
                // Round-trip semantics: re-serialization must parse (enum closed loop) + wire carries a type key.
                let back: SignalingMessage =
                    serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
                let _ = back;
                assert_eq!(
                    parsed["type"],
                    Value::String(msg_wire_type(&msg)),
                    "{name}: type agrees"
                );
                n_signal += 1;
            }
            "envelope" => {
                let env: ControlEnvelope = serde_json::from_str(&wire)
                    .unwrap_or_else(|e| panic!("{name}: ControlEnvelope {e}"));
                assert_eq!(env.seq, parsed["seq"].as_u64().unwrap(), "{name}: seq");
                assert_eq!(env.cmd, parsed["cmd"].as_str().unwrap(), "{name}: cmd");
                if parsed.get("payload").is_none() {
                    assert_eq!(
                        env.payload,
                        serde_json::json!({}),
                        "{name}: default payload is an empty object"
                    );
                }
                n_envelope += 1;
            }
            "ack" => {
                let ack: ControlAck = serde_json::from_str(&wire)
                    .unwrap_or_else(|e| panic!("{name}: ControlAck {e}"));
                assert_eq!(ack.ack, parsed["ack"].as_u64().unwrap(), "{name}: ack");
                n_envelope += 1;
            }
            other => panic!("unknown fixture kind: {other}"),
        }
        for (path, want) in doc["checks"].as_object().unwrap() {
            if path == "type" {
                continue; // already checked in the signal branch
            }
            let got =
                get(&parsed, path).unwrap_or_else(|| panic!("{name}: missing check path {path}"));
            assert_eq!(got, want, "{name}: {path}");
        }
    }
    assert!(
        n_signal >= 6 && n_envelope >= 5,
        "coverage: signal={n_signal} envelope/ack={n_envelope}"
    );
}

/// SignalingMessage -> wire type name (serde snake_case tag).
fn msg_wire_type(m: &SignalingMessage) -> String {
    let v = serde_json::to_value(m).unwrap();
    v["type"].as_str().unwrap_or_default().to_string()
}
