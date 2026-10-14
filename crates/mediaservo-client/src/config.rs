//! ClientConfig -- every endpoint/credential is passed in by the caller (the S2 discipline: no default localhost/9800/psk).
//!
//! URL derivation pure functions + the SFU peer key mirroring field semantics.

use crate::error::ClientError;
use mediaservo_common::protocol::PeerRole;

/// The client v2 config -- ready at construction, no implicit defaults.
#[derive(Clone)]
pub struct ClientConfig {
    /// The signaling WS address (e.g. `ws://10.0.0.2:9800/ws`) -- passed straight to the link SignalClient.
    pub signaling_url: String,
    /// The server room ID.
    pub room_id: String,
    /// Optional PSK passed straight through (omit when the server has none; empty != guessing a default).
    pub psk: Option<String>,
    /// Optional account JWT (the login output; the v1 WS passthrough only injects by hand via link).
    pub jwt: Option<String>,
    /// The join role (the cockpit side = Remote or Consumer).
    pub role: PeerRole,
    /// S4/T3.5: the e-stop HMAC pre-shared key (the same value as the vehicle's `MEDIASERVO_CONTROL_HMAC_KEY`).
    /// None = e-stops carry no sig (the vehicle executes as usual without a key; once configured, unsigned estops get rejected).
    pub hmac_key: Option<String>,
    /// The device identity instance dir (the accountless-client-auth T3 / D283 public-key wiring): contains
    /// `identity.json` + `etc/link/signing.pem` (the same layout as `msrtc-host init`).
    /// Some with both files present -> RoomJoin carries device_pubkey + an Ed25519 challenge response (account-less entry,
    /// identity = Device); identity.json missing -> silently identity-less (the PSK/JWT paths unchanged);
    /// PEM missing/corrupt -> connect errors explicitly (never a silent downgrade, C15).
    pub identity_dir: Option<std::path::PathBuf>,
}

// jwt redacted in Debug -- credentials never enter logs.
impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("signaling_url", &self.signaling_url)
            .field("room_id", &self.room_id)
            .field("psk", &self.psk.as_ref().map(|_| "[redacted]"))
            .field("jwt", &self.jwt.as_ref().map(|_| "[redacted]"))
            .field("role", &self.role)
            .finish()
    }
}

/// SFU peer key derivation (mirrors `field::peer_id(role)` semantics, the I2 same shape).
///
/// An SFU room's peer ID is a "who is speaking" semantic label, not an identity.
/// identity.json/device_id is the identity. Peer ID overlap has no practical effect (SFU per-conn).
#[must_use]
pub fn sfu_peer_key(role: &PeerRole) -> &'static str {
    match role {
        PeerRole::Host => "host",
        PeerRole::Remote => "remote",
        PeerRole::Consumer => "consumer",
    }
}

/// Derive the HTTP base address from the signaling URL (the v1 login surface).
///
/// `"ws://10.0.0.2:9800/ws"` → `"http://10.0.0.2:9800"`
/// `"wss://host/ws"` -> `"https://host"` (the login layer rejects the scheme afterwards).
pub fn http_base_from_signaling(url: &str) -> Result<String, ClientError> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| {
        ClientError::MalformedResponse(format!("no scheme in signaling URL: {url}"))
    })?;
    let host_port = rest.split('/').next().unwrap_or(rest);
    if host_port.is_empty() {
        return Err(ClientError::MalformedResponse("empty host in signaling URL".into()));
    }
    let http_scheme = match scheme {
        "ws" => "http",
        "wss" => "https",
        other => return Err(ClientError::UnsupportedScheme(other.to_string())),
    };
    Ok(format!("{http_scheme}://{host_port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfu_peer_key_covers_all_roles() {
        assert_eq!(sfu_peer_key(&PeerRole::Host), "host");
        assert_eq!(sfu_peer_key(&PeerRole::Remote), "remote");
        assert_eq!(sfu_peer_key(&PeerRole::Consumer), "consumer");
    }

    #[test]
    fn http_base_ws_to_http() {
        assert_eq!(
            http_base_from_signaling("ws://10.0.0.2:9800/ws").unwrap(),
            "http://10.0.0.2:9800"
        );
    }

    #[test]
    fn http_base_wss_to_https() {
        assert_eq!(
            http_base_from_signaling("wss://host.example/ws").unwrap(),
            "https://host.example"
        );
    }

    #[test]
    fn http_base_no_path() {
        assert_eq!(http_base_from_signaling("ws://host:9800").unwrap(), "http://host:9800");
    }

    #[test]
    fn http_base_no_scheme_error() {
        assert!(matches!(
            http_base_from_signaling("10.0.0.2:9800/ws"),
            Err(ClientError::MalformedResponse(_))
        ));
    }
}
