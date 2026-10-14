//! PSK (Pre-Shared Key) HMAC-SHA256 and JWT HS256 authentication.
//!
//! Phase 1 MVP: Simple PSK handshake. Client sends HMAC(challenge, psk),
//! Server verifies. Used for WebSocket signaling auth.
//! Phase 2+: JWT token auth as primary, PSK as fallback.

use crate::error::CoreError;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Authentication result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResult {
    /// Authentication succeeded.
    Success,
    /// Authentication failed — invalid PSK.
    Denied,
    /// Challenge expired.
    Expired,
}

/// PSK authenticator trait.
///
/// Components implement this trait with their PSK sourcing strategy.
#[async_trait::async_trait]
pub trait PskAuthenticator: Send + Sync {
    /// Verify a signed challenge.
    async fn verify_challenge(
        &self,
        challenge: &[u8],
        signature: &[u8],
    ) -> Result<AuthResult, CoreError>;
}

/// Simple PSK authenticator that holds the key in memory.
///
/// # Security
/// Phase 1: key from env var or config file. Not meant for multi-tenant production.
/// Phase 2+: integrate with vault / LDAP.
pub struct SimplePskAuth {
    psk: Vec<u8>,
}

impl SimplePskAuth {
    /// Create from a PSK string (base64-encoded or raw).
    pub fn new(psk: impl AsRef<[u8]>) -> Self {
        Self { psk: psk.as_ref().to_vec() }
    }

    /// Compute HMAC-SHA256(challenge, psk) → truncated to 8 bytes.
    pub fn sign(&self, challenge: &[u8]) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.psk).expect("HMAC key can be any length");
        mac.update(challenge);
        // ponytail: 8-byte tag is enough for Phase 1 challenge-response; full 32 bytes if collision rate rises
        mac.finalize().into_bytes()[..8].to_vec()
    }
}

#[async_trait::async_trait]
impl PskAuthenticator for SimplePskAuth {
    async fn verify_challenge(
        &self,
        challenge: &[u8],
        signature: &[u8],
    ) -> Result<AuthResult, CoreError> {
        let expected = self.sign(challenge);
        if constant_time_eq(&expected, signature) {
            Ok(AuthResult::Success)
        } else {
            Ok(AuthResult::Denied)
        }
    }
}

/// Constant-time comparison to prevent timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ── JWT Authentication ──────────────────────────────────────────────────────

/// JWT claims for signaling authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject (peer identifier).
    pub sub: String,
    /// Issued at (Unix timestamp in seconds).
    pub iat: usize,
    /// Expiration time (Unix timestamp in seconds).
    pub exp: usize,

    /// Role of the subject (e.g., "admin", "user").
    #[serde(default)]
    pub role: Option<String>,

    /// G3 vehicle x cockpit authorization allowlist: vehicle device_id list this account may access (viewer/operator only;
    /// admin/dispatcher ignore it = any vehicle). additive - old tokens default to empty.
    #[serde(default)]
    pub vehicles: Option<Vec<String>>,
}

/// JWT authenticator using HS256.
#[derive(Clone)]
pub struct JwtAuth {
    secret: String,
}

impl JwtAuth {
    /// Create from a secret string (at least 256-bit entropy recommended).
    pub fn new(secret: impl Into<String>) -> Self {
        Self { secret: secret.into() }
    }

    /// Create a signed JWT token for the given subject.
    pub fn sign(&self, sub: &str, ttl_secs: u64) -> Result<String, CoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| CoreError::Unknown(format!("clock error: {e}")))?
            .as_secs() as usize;
        let claims = JwtClaims {
            sub: sub.to_string(),
            role: None,
            vehicles: None,
            iat: now,
            exp: now + ttl_secs as usize,
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .map_err(|e| CoreError::Unknown(format!("JWT sign error: {e}")))?;
        Ok(token)
    }

    /// Verify a JWT token and return the validated claims.
    pub fn verify(&self, token: &str) -> Result<JwtClaims, CoreError> {
        let token_data = jsonwebtoken::decode::<JwtClaims>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(self.secret.as_bytes()),
            &jsonwebtoken::Validation::default(),
        )
        .map_err(|e| CoreError::Unknown(format!("JWT verify error: {e}")))?;
        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PSK tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn psk_auth_success() {
        let auth = SimplePskAuth::new("test-secret-key");
        let challenge = b"server-challenge-123";
        let sig = auth.sign(challenge);
        let result = auth.verify_challenge(challenge, &sig).await.unwrap();
        assert_eq!(result, AuthResult::Success);
    }

    #[tokio::test]
    async fn psk_auth_denied_wrong_key() {
        let auth = SimplePskAuth::new("right-key");
        let other = SimplePskAuth::new("wrong-key");
        let challenge = b"challenge";
        let sig = other.sign(challenge);
        let result = auth.verify_challenge(challenge, &sig).await.unwrap();
        assert_eq!(result, AuthResult::Denied);
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn sign_different_inputs_different_tags() {
        let auth = SimplePskAuth::new("my-key");
        let tag1 = auth.sign(b"challenge-1");
        let tag2 = auth.sign(b"challenge-2");
        assert_ne!(tag1, tag2, "different challenges must produce different HMAC tags");
    }

    #[test]
    fn sign_same_input_same_tag() {
        let auth = SimplePskAuth::new("my-key");
        let tag1 = auth.sign(b"same-challenge");
        let tag2 = auth.sign(b"same-challenge");
        assert_eq!(tag1, tag2, "same challenge must produce same HMAC tag");
    }

    #[test]
    fn different_psk_different_tag() {
        let auth1 = SimplePskAuth::new("key-alpha");
        let auth2 = SimplePskAuth::new("key-beta");
        let challenge = b"shared-challenge";
        assert_ne!(auth1.sign(challenge), auth2.sign(challenge));
    }

    #[test]
    fn auth_result_debug() {
        assert_eq!(format!("{:?}", AuthResult::Success), "Success");
        assert_eq!(format!("{:?}", AuthResult::Denied), "Denied");
        assert_eq!(format!("{:?}", AuthResult::Expired), "Expired");
    }

    #[test]
    fn auth_result_equality() {
        assert_eq!(AuthResult::Success, AuthResult::Success);
        assert_ne!(AuthResult::Success, AuthResult::Denied);
        assert_ne!(AuthResult::Denied, AuthResult::Expired);
    }

    // ── JWT tests ────────────────────────────────────────────────────────

    #[test]
    fn jwt_sign_and_verify() {
        let auth = JwtAuth::new("my-jwt-secret-256-bit-minimum-key");
        let token = auth.sign("peer-42", 3600).unwrap();
        let claims = auth.verify(&token).unwrap();
        assert_eq!(claims.sub, "peer-42");
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn jwt_verify_wrong_secret() {
        let auth1 = JwtAuth::new("secret-one");
        let auth2 = JwtAuth::new("secret-two");
        let token = auth1.sign("peer-1", 3600).unwrap();
        let result = auth2.verify(&token);
        assert!(result.is_err(), "verification with wrong secret should fail");
    }

    #[test]
    fn jwt_verify_expired() {
        let auth = JwtAuth::new("secret");
        // ponytail: use direct claims construction for deterministic expiry test
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                as usize;
        let claims = JwtClaims {
            sub: "peer-1".into(),
            iat: now - 7200,
            exp: now - 3600, // expired 1 hour ago
            role: None,
            vehicles: None,
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(auth.secret.as_bytes()),
        )
        .unwrap();
        let result = auth.verify(&token);
        assert!(result.is_err(), "expired token should fail verification");
    }

    #[test]
    fn jwt_verify_tampered() {
        let auth = JwtAuth::new("secret");
        let mut token = auth.sign("peer-1", 3600).unwrap();
        // Tamper with the payload by appending garbage
        token.push('x');
        let result = auth.verify(&token);
        assert!(result.is_err(), "tampered token should fail verification");
    }

    #[test]
    fn jwt_claims_vehicles_roundtrip() {
        // G3: account token carries the vehicle allowlist (serde additive, defaults to None)
        let auth = JwtAuth::new("my-jwt-secret-256-bit-minimum-key");
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                as usize;
        let claims = JwtClaims {
            sub: "carol".into(),
            iat: now,
            exp: now + 3600,
            role: Some("operator".into()),
            vehicles: Some(vec!["ms-car1".into()]),
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(auth.secret.as_bytes()),
        )
        .unwrap();
        let parsed = auth.verify(&token).unwrap();
        assert_eq!(parsed.role.as_deref(), Some("operator"));
        assert_eq!(parsed.vehicles.as_deref(), Some(&["ms-car1".to_string()][..]));
        // Old token (no vehicles field) defaults to None
        let legacy = JwtClaims {
            sub: "old".into(),
            iat: now,
            exp: now + 3600,
            role: Some("admin".into()),
            vehicles: None,
        };
        let json = serde_json::to_string(&legacy).unwrap();
        let parsed_legacy: JwtClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed_legacy.vehicles, None);
    }
}
