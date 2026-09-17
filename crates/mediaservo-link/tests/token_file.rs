//! Task B2: 单文件自描述令牌测试（D-H10/D-H13：verifying key + claims + signature 合并单文件）。

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameTopic, NodeAcl, NodeId, Role,
    TokenFile,
};

// 测试用 Ed25519 密钥对（openssl 生成，仅测试用）。
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";
// 另一个不相关公钥（openssl 再生成，仅测试用）。
const WRONG_PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAkCGEQfZ6DyEUyzKgaQSvGbABtQs/W9ghbNgT0DLl9pY=\n-----END PUBLIC KEY-----\n";

fn keys() -> (Ed25519SigningKey, Ed25519VerifyingKey) {
    (
        Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes()),
        Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes()),
    )
}

fn sample_token() -> CapabilityToken {
    let (sk, _) = keys();
    let acl = NodeAcl::for_role(NodeId::new("ros-stitcher"), Role::Processor);
    CapabilityToken::sign(&acl, 3600, &sk).unwrap()
}

/// 文件布局（encode 产出）: magic(4) + version(1) + key_len(2 LE) + key + tok_len(2 LE) + token。
fn layout(bytes: &[u8]) -> (usize, usize) {
    let key_len = u16::from_le_bytes([bytes[5], bytes[6]]) as usize;
    let key_end = 7 + key_len;
    let tok_len = u16::from_le_bytes([bytes[key_end], bytes[key_end + 1]]) as usize;
    (key_end + 2, tok_len)
}

#[test]
fn encode_decode_roundtrip() {
    let (_, vk) = keys();
    let tok = sample_token();
    let file = TokenFile::encode(&tok, &vk);
    let (key, decoded) = TokenFile::decode(&file).unwrap();
    assert_eq!(key.pem(), vk.pem(), "验证密钥应原样恢复");
    assert_eq!(decoded.as_str(), tok.as_str(), "令牌字符串应原样恢复");
    let claims = decoded.verify(&key).unwrap();
    assert_eq!(claims.role, Role::Processor);
    assert_eq!(claims.node_id, "ros-stitcher");
    assert!(claims.acl.can_publish(&FrameTopic::new("video/stitched")));
}

#[test]
fn tampered_key_region_rejected() {
    let (_, vk) = keys();
    let file = TokenFile::encode(&sample_token(), &vk);
    let (key_end, _) = layout(&file);
    let mut tampered = file.clone();
    tampered[7 + (key_end - 7) / 2] ^= 0x01; // 翻转 key 区域中间一个字节
    assert!(TokenFile::decode(&tampered).is_err(), "篡改 key 区域必须验签失败");
}

#[test]
fn tampered_claims_region_rejected() {
    let (_, vk) = keys();
    let file = TokenFile::encode(&sample_token(), &vk);
    let (tok_start, tok_len) = layout(&file);
    let sig_start = file[tok_start..tok_start + tok_len]
        .iter()
        .rposition(|&b| b == b'.')
        .unwrap()
        + tok_start; // JWT 最后一个 '.' 之后是签名段
    let mut tampered = file.clone();
    tampered[tok_start + (sig_start - tok_start) / 2] ^= 0x01; // 翻转 claims(header/payload) 区域
    assert!(TokenFile::decode(&tampered).is_err(), "篡改 claims 区域必须验签失败");
}

#[test]
fn tampered_signature_region_rejected() {
    let (_, vk) = keys();
    let file = TokenFile::encode(&sample_token(), &vk);
    let (tok_start, tok_len) = layout(&file);
    let sig_start = file[tok_start..tok_start + tok_len]
        .iter()
        .rposition(|&b| b == b'.')
        .unwrap()
        + tok_start;
    let mut tampered = file.clone();
    tampered[sig_start + (tok_start + tok_len - sig_start) / 2] ^= 0x01; // 翻转 JWT 签名段
    assert!(TokenFile::decode(&tampered).is_err(), "篡改签名区域必须验签失败");
}

#[test]
fn truncated_or_wrong_length_rejected() {
    let (_, vk) = keys();
    let file = TokenFile::encode(&sample_token(), &vk);
    // 空输入 / 比 magic+version 短 / 各边界截断
    assert!(TokenFile::decode(&[]).is_err());
    assert!(TokenFile::decode(&file[..4]).is_err());
    assert!(TokenFile::decode(&file[..8]).is_err());
    for cut in [9, 20, 30, 40, file.len() - 1] {
        assert!(TokenFile::decode(&file[..cut]).is_err(), "截断到 {cut} 必须失败");
    }
    // 尾随多余字节 = 格式错（严格长度）
    let mut padded = file.clone();
    padded.push(0x00);
    assert!(TokenFile::decode(&padded).is_err());
    // 错误 magic
    let mut bad_magic = file.clone();
    bad_magic[0] = b'X';
    assert!(TokenFile::decode(&bad_magic).is_err());
    // 错误 version
    let mut bad_ver = file.clone();
    bad_ver[4] = 0x7F;
    assert!(TokenFile::decode(&bad_ver).is_err());
}

#[test]
fn wrong_signing_key_rejected() {
    let (sk, _vk) = keys();
    let acl = NodeAcl::for_role(NodeId::new("capture-0"), Role::Capture);
    let tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    // 用另一密钥对的公钥编码 → 文件内验签必须失败
    let wrong_vk = Ed25519VerifyingKey::from_pem(WRONG_PUB_PEM.as_bytes());
    let file = TokenFile::encode(&tok, &wrong_vk);
    assert!(TokenFile::decode(&file).is_err(), "不同签名密钥的文件必须失败");
}

#[test]
fn to_file_from_file_symmetry() {
    let (_, vk) = keys();
    let tok = sample_token();
    let file = tok.to_file(&vk);
    let (key, decoded) = CapabilityToken::from_file(&file).unwrap();
    assert_eq!(key.pem(), vk.pem());
    assert_eq!(decoded, tok);
    assert_eq!(decoded.verify(&key).unwrap().node_id, "ros-stitcher");
}
