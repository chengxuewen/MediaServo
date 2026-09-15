
/// S4/T3.5 L1 向量：estop HMAC 签名跨语言参考形（python hmac 生成 fixture，
/// 本测回放 = canonical 字节序/编码双语言一致的锁）。
#[test]
fn ctl_estop_signature_vector() {
    use mediaservo_common::protocol::{control_hmac_sign, control_hmac_verify, ControlEnvelope};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sig_vector/ctl");
    let raw = std::fs::read_to_string(dir.join("estop-signed.json")).expect("fixture");
    let fx: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let key = fx["key"].as_str().unwrap();
    let env: ControlEnvelope = serde_json::from_value(fx["wire"].clone()).unwrap();
    let sig = fx["sig"].as_str().unwrap();
    assert_eq!(control_hmac_sign(key, &env), sig, "canonical 必须与 python 字节一致");
    assert!(control_hmac_verify(key, &env, sig));
    assert!(!control_hmac_verify("other-key", &env, sig), "错 key 必拒");
    let tampered = ControlEnvelope { seq: env.seq + 1, ..env.clone() };
    assert!(!control_hmac_verify(key, &tampered, sig), "seq 篡改必拒");
}
