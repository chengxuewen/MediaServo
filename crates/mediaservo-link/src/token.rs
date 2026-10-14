//! Capability tokens (D238: ACL signed into the JWT; Ed25519 asymmetric - device key signs, nodes verify with the public key).

use crate::acl::{NodeAcl, Role};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};

/// Ed25519 signing key (private PEM, held by the device authority).
#[derive(Debug, Clone)]
pub struct Ed25519SigningKey(Vec<u8>);

impl Ed25519SigningKey {
    pub fn from_pem(pem: &[u8]) -> Self {
        Self(pem.to_vec())
    }
    pub fn pem(&self) -> &[u8] {
        &self.0
    }
}

/// Ed25519 verifying key (public PEM, held at each check point; from `MEDIASERVO_DEVICE_PUBKEY` env / device config).
#[derive(Debug, Clone)]
pub struct Ed25519VerifyingKey(Vec<u8>);

impl Ed25519VerifyingKey {
    pub fn from_pem(pem: &[u8]) -> Self {
        Self(pem.to_vec())
    }
    pub fn pem(&self) -> &[u8] {
        &self.0
    }
}

/// Token claims: node_id + role + acl + exp.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Claims {
    pub node_id: String,
    pub role: Role,
    /// ACL embedded in the token (D238).
    pub acl: NodeAcl,
    pub exp: u64,
}

/// Capability token (JWT string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityToken(String);

impl CapabilityToken {
    /// Ed25519 signature (jsonwebtoken `Algorithm::EdDSA`). **HS256 is forbidden** (symmetric - the key holder can forge, D238).
    pub fn sign(
        acl: &NodeAcl,
        ttl_secs: u64,
        signing_key: &Ed25519SigningKey,
    ) -> Result<Self, crate::LinkError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| crate::LinkError::Token(e.to_string()))?
            .as_secs();
        let claims = Claims {
            node_id: acl.node_id.as_str().to_string(),
            role: acl.role,
            acl: acl.clone(),
            exp: now + ttl_secs,
        };
        let key = EncodingKey::from_ed_pem(signing_key.pem())
            .map_err(|e| crate::LinkError::Token(e.to_string()))?;
        let token = encode(&Header::new(Algorithm::EdDSA), &claims, &key)
            .map_err(|e| crate::LinkError::Token(e.to_string()))?;
        Ok(Self(token))
    }

    /// Verify the signature with the public key + check exp.
    pub fn verify(&self, verifying_key: &Ed25519VerifyingKey) -> Result<Claims, crate::LinkError> {
        let key = DecodingKey::from_ed_pem(verifying_key.pem())
            .map_err(|e| crate::LinkError::Token(e.to_string()))?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = true;
        validation.leeway = 0; // exp enforced exactly (capability tokens get no leeway)
        let data = decode::<Claims>(&self.0, &key, &validation)
            .map_err(|e| crate::LinkError::Token(e.to_string()))?;
        Ok(data.claims)
    }

    /// Raw JWT string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Construct from a raw JWT string (diagnostics/tests).
    pub fn from_raw(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    /// Serialize into self-describing single-file bytes (D-H10: deploy ships one file; SDK-side `from_file` loads and verifies).
    pub fn to_file(&self, verifying_key: &Ed25519VerifyingKey) -> Vec<u8> {
        TokenFile::encode(self, verifying_key)
    }

    /// Load and verify from self-describing single-file bytes (tampered / wrong key / expired all fail).
    pub fn from_file(
        bytes: &[u8],
    ) -> Result<(Ed25519VerifyingKey, CapabilityToken), crate::LinkError> {
        TokenFile::decode(bytes)
    }
}

/// Self-describing single-file token (D-H10/D-H13): verifying key + claims + signature merged into one file;
/// deploy ships one file and [`CapabilityToken::from_file`] completes verification on load - no external key needed.
///
/// Byte layout (all little-endian):
/// ```text
/// magic    4B   "MSTK"
/// version  1B   0x01
/// key_len  2B   verifying-key PEM length
/// key      key_len B
/// tok_len  2B   JWT length
/// token    tok_len B (full JWT: header.payload.signature; its Ed25519 signature = file-level integrity)
/// ```
/// No separate file-level signature: `encode` holds only the verifying key (no private key, cannot re-sign); the embedded JWT's Ed25519
/// signature covers the whole file - tampering the key region breaks embedded verification, tampering claims/signature breaks JWT verification.
#[derive(Debug, Clone, Copy)]
pub struct TokenFile;

impl TokenFile {
    /// File magic (`b"MSTK"`).
    pub const MAGIC: &[u8; 4] = b"MSTK";
    /// File format version.
    pub const VERSION: u8 = 1;

    /// Encode: merge the verifying-key PEM + full JWT into single-file bytes.
    pub fn encode(token: &CapabilityToken, key: &Ed25519VerifyingKey) -> Vec<u8> {
        let pem = key.pem();
        let raw = token.as_str().as_bytes();
        let mut out = Vec::with_capacity(4 + 1 + 2 + pem.len() + 2 + raw.len());
        out.extend_from_slice(Self::MAGIC);
        out.push(Self::VERSION);
        out.extend_from_slice(&(pem.len() as u16).to_le_bytes());
        out.extend_from_slice(pem);
        out.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        out.extend_from_slice(raw);
        out
    }

    /// Decode and verify: check the JWT signature + exp with the embedded key (same semantics as `CapabilityToken::verify`).
    /// Tampering any region (key/claims/signature), truncation/extra bytes, wrong key, and expiry all return errors.
    pub fn decode(
        bytes: &[u8],
    ) -> Result<(Ed25519VerifyingKey, CapabilityToken), crate::LinkError> {
        let err = |m: String| crate::LinkError::Token(format!("token file: {m}"));
        if bytes.len() < 9 || &bytes[..4] != Self::MAGIC {
            return Err(err("bad magic or too short".into()));
        }
        if bytes[4] != Self::VERSION {
            return Err(err(format!("unsupported version {}", bytes[4])));
        }
        let key_len = u16::from_le_bytes([bytes[5], bytes[6]]) as usize;
        let rest = &bytes[7..];
        if rest.len() < key_len + 2 {
            return Err(err("truncated key".into()));
        }
        let (key_pem, rest) = rest.split_at(key_len);
        let tok_len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
        let rest = &rest[2..];
        if rest.len() != tok_len {
            return Err(err("truncated token".into()));
        }
        let token_str = String::from_utf8(rest.to_vec()).map_err(|e| err(e.to_string()))?;
        let key = Ed25519VerifyingKey::from_pem(key_pem);
        let token = CapabilityToken::from_raw(token_str);
        token.verify(&key)?; // embedded verification covers the whole file (any key/claims/sig tamper fails) + exp
        Ok((key, token))
    }
}
