//! client SDK error surface (thiserror -- library posture; C15: every Err branch carries type and context).

use mediaservo_link::LinkError;

/// The client v2 unified error.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Login request/parse failure (network, non-401 status, malformed response).
    #[error("login failed: {0}")]
    Login(String),
    /// 401 -- wrong username or password (the server's anti-enumeration wording; details only WARN locally).
    #[error("invalid credentials")]
    InvalidCredentials,
    /// REST discovery surface (GET /api/rooms) non-2xx -- token invalid/expired/authorization mismatch (p3 W2-B).
    #[error("REST rejected [{code}]: {message}")]
    RestRejected { code: u16, message: String },
    /// Connection-level auth rejected (4003 PSK / 4010 device / 4011 illegal role / 4013 JWT signature failure).
    #[error("auth rejected [{code}]: {message}")]
    AuthRejected { code: u16, message: String },
    /// The server rejects the dialect version (4101, S0) -- no silent downgrade, terminal.
    #[error("protocol version unsupported (4101): {0}")]
    ProtocolUnsupported(String),
    /// The v1 login surface only supports plaintext http (TLS dependency is an S4+ decision).
    #[error("unsupported scheme \"{0}\" (login v1 = plain http only)")]
    UnsupportedScheme(String),
    /// Response unparseable (HTTP status line/JSON).
    #[error("malformed response: {0}")]
    MalformedResponse(String),
    /// Signaling-layer error (link passthrough: WS connect/auth/send-receive/join).
    #[error("signal error: {0}")]
    Signal(#[from] LinkError),
    /// SFU-domain server error (the generic carrier for non-terminal codes).
    #[error("server error [{code}]: {message}")]
    Server { code: u16, message: String },
    /// Control DC establishment rejected (4012 -- role without can_control or dialect too low, terminal).
    #[error("control denied (4012): {0}")]
    ControlDenied(String),
    /// I5 capability-gate pre-check: the locally negotiated dialect is below the control domain's requirement (rejected before any request is sent).
    #[error("negotiated protocol {got} < required {need} for control domain")]
    ProtocolTooLow { need: u32, got: u32 },
    /// WebRTC local error (PC negotiation/DC write).
    #[error("webrtc error: {0}")]
    WebRtc(String),
    /// IO (TcpStream read/write etc.).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Bounded-wait timeout (tagged with the operation name for attribution).
    #[error("timed out waiting for {what}")]
    Timeout { what: &'static str },
    /// Stream not found (msrtc-stream-not-found: the label is absent from the room's full known label set
    /// and also never arrived within the fallback window -- a deterministic verdict for a wrong stream name in the URL, distinct from a blind network timeout).
    #[error("stream not found: {label}")]
    StreamNotFound { label: String },
    /// State-machine violation (unknown channel, session already closed, etc.).
    #[error("invalid state: {0}")]
    InvalidState(String),
}

impl ClientError {
    /// S6/K2: wire code reverse-lookup -- the machine-readable read-back of the `from_wire_error` classification source.
    ///
    /// Single-source discipline: the attribution of 4012/4101/4003|4010|4011 matches [`from_wire_error`];
    /// the mapping table is never duplicated; the `Signal` variant recovers via [`extract_wire_code`] from the link-embedded string.
    /// Returns `None` = the error carries no wire code (local/REST/IO domains).
    #[must_use]
    pub fn wire_code(&self) -> Option<u32> {
        match self {
// local/REST/transport domains without a wire code.
            Self::Login(_)
            | Self::InvalidCredentials
            | Self::RestRejected { .. }
            | Self::UnsupportedScheme(_)
            | Self::MalformedResponse(_)
            | Self::WebRtc(_)
            | Self::Io(_)
            | Self::Timeout { .. }
            | Self::StreamNotFound { .. }
            | Self::InvalidState(_)
// I5 local pre-rejection (no request sent, no server reply code).
            | Self::ProtocolTooLow { .. } => None,
            Self::AuthRejected { code, .. } => Some(u32::from(*code)),
            Self::ProtocolUnsupported(_) => Some(4101),
            Self::ControlDenied(_) => Some(4012),
            Self::Server { code, .. } => Some(u32::from(*code)),
            Self::Signal(e) => extract_wire_code(&e.to_string()).map(u32::from),
        }
    }

    /// S6/K2: retry semantics as a first-class predicate (the classification source for the K1 reconnect chain).
    ///
    /// Provenance = the D273 red-card family semantics (field-proven on the web half): the auth family (4003/4010/4011/4012)
    /// and the dialect rejection (4101) = terminal (a reconnect necessarily fails the same way; retries only mask credential problems); connection-class
    /// (IO/Signal codeless/WebRtc connect/Timeout) and server-domain >=5000 (5001 = SFU temporarily unavailable
    /// and similar transients) = retryable. REST rejections/protocol mismatches/state violations conservatively non-retryable (4xx semantics).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            // the terminal family (D273 red cards = the auth family + 4101; ProtocolTooLow = the same family as the local gate).
            Self::InvalidCredentials
            | Self::AuthRejected { .. }
            | Self::ProtocolUnsupported(_)
            | Self::ControlDenied(_)
            | Self::ProtocolTooLow { .. }
            | Self::UnsupportedScheme(_)
            | Self::RestRejected { .. }
            | Self::MalformedResponse(_)
            | Self::InvalidState(_)
            | Self::StreamNotFound { .. } => false,
            // the connection/transient family.
            Self::Io(_) | Self::WebRtc(_) | Self::Timeout { .. } | Self::Login(_) => true,
            Self::Server { code, .. } => *code >= 5000,
            // link passthrough: an embedded auth-family code = terminal; a pure connection error = retryable.
            Self::Signal(e) => !matches!(
                extract_wire_code(&e.to_string()),
                Some(4003 | 4010 | 4011 | 4012 | 4013 | 4101)
            ),
        }
    }
}

/// wire `Error{code,message}` -> terminal classification (the D273 terminal family's SDK-side landing).
///
/// 4012 = control rejected, 4101 = dialect rejected as dedicated typed variants; everything else carried by [`ClientError::Server`].
#[must_use]
pub fn from_wire_error(code: u16, message: &str) -> ClientError {
    match code {
        4012 => ClientError::ControlDenied(message.to_string()),
        4101 => ClientError::ProtocolUnsupported(message.to_string()),
        4003 | 4010 | 4011 | 4013 => {
            ClientError::AuthRejected { code, message: message.to_string() }
        }
        _ => ClientError::Server { code, message: message.to_string() },
    }
}

/// Extract the wire code from a link error string (the `LinkError::Signal("... [4101] ...")` shape:
/// link::connect embeds the server Error code into the message string at the join phase -- typed classification recovered here).
/// Extract the wire code from a link error string (the `LinkError::Signal("... [4101] ...")` shape);
/// link::connect embeds the server Error code into the message string at the join phase -- typed classification recovered here).
/// LinkError is not Clone -- consumed by value conversion; the fallback converts into a Signal variant.
#[must_use]
pub fn classify_link_error(err: LinkError) -> ClientError {
    let text = err.to_string();
    match extract_wire_code(&text) {
        Some(code) if matches!(code, 4003 | 4010 | 4011 | 4012 | 4013 | 4101) => {
            from_wire_error(code, &text)
        }
        _ => ClientError::Signal(err),
    }
}

/// Extract the first `[<u16>]` number (none = None). Pure function, pinned by tests.
#[must_use]
pub fn extract_wire_code(text: &str) -> Option<u16> {
    let start = text.find('[')? + 1;
    let rest = &text[start..];
    let end = rest.find(']')?;
    rest[..end].parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_code_classification() {
        assert!(matches!(
            from_wire_error(4012, "control_denied: viewer"),
            ClientError::ControlDenied(_)
        ));
        assert!(matches!(
            from_wire_error(4101, "protocol too old"),
            ClientError::ProtocolUnsupported(_)
        ));
        assert!(matches!(
            from_wire_error(4003, "psk"),
            ClientError::AuthRejected { code: 4003, .. }
        ));
        assert!(matches!(
            from_wire_error(4013, "jwt expired"),
            ClientError::AuthRejected { code: 4013, .. }
        ));
        assert!(matches!(from_wire_error(5000, "boom"), ClientError::Server { code: 5000, .. }));
    }

    #[test]
    fn extract_wire_code_parses_first_bracket() {
        assert_eq!(
            extract_wire_code("room join failed [4101] protocol_version_unsupported"),
            Some(4101)
        );
        assert_eq!(extract_wire_code("no code here"), None);
        assert_eq!(extract_wire_code("[999999]"), None); // u16 overflow -> None
    }

    #[test]
    fn classify_link_error_maps_4101_to_typed() {
        let e = LinkError::Signal("room join failed [4101]: too old".into());
        assert!(matches!(classify_link_error(e), ClientError::ProtocolUnsupported(_)));
        let other = LinkError::Signal("connect refused".into());
        assert!(matches!(classify_link_error(other), ClientError::Signal(_)));
    }

    /// K2 pin: wire_code agrees with the from_wire_error classification source (a regression net over the exhaustive match's table).
    #[test]
    fn wire_code_round_trips_classification_source() {
        assert_eq!(from_wire_error(4012, "x").wire_code(), Some(4012));
        assert_eq!(from_wire_error(4101, "x").wire_code(), Some(4101));
        assert_eq!(from_wire_error(4010, "x").wire_code(), Some(4010));
        assert_eq!(from_wire_error(5001, "x").wire_code(), Some(5001));
        // local-domain errors carry no code.
        assert_eq!(ClientError::InvalidCredentials.wire_code(), None);
        assert_eq!(
            ClientError::ProtocolTooLow { need: 2, got: 1 }.wire_code(),
            None,
            "a local pre-rejection sends no request = no wire code"
        );
        // Signal embedded-code recovery (unknown codes that classify misses are fished back out of the string).
        let e = ClientError::Signal(LinkError::Signal("room join failed [4031]: denied".into()));
        assert_eq!(e.wire_code(), Some(4031));
    }

    /// K2 pin: retry classification aligned with the D273 red-card family (auth family/4101/4012 terminal; connection class/5001 retryable).
    #[test]
    fn retryable_matches_d273_red_card_family() {
        for terminal in [
            from_wire_error(4003, "psk"),
            from_wire_error(4010, "device"),
            from_wire_error(4011, "role"),
            from_wire_error(4012, "control"),
            from_wire_error(4101, "protocol"),
            ClientError::InvalidCredentials,
            ClientError::RestRejected { code: 401, message: String::new() },
        ] {
            assert!(
                !terminal.is_retryable(),
                "the auth/dialect family must be terminal: {terminal:?}"
            );
        }
        for retry in [
            ClientError::Io(std::io::Error::other("refused")),
            ClientError::WebRtc("ice".into()),
            ClientError::Timeout { what: "Consumed" },
            from_wire_error(5000, "boom"),
            from_wire_error(5001, "sfu unavailable"),
            ClientError::Signal(LinkError::Signal("connect ws://x: refused".into())),
        ] {
            assert!(
                retry.is_retryable(),
                "the connection/>=5000 family must be retryable: {retry:?}"
            );
        }
        // Signal with an embedded auth code = terminal (the link-passthrough form judges the same as the typed form).
        assert!(!ClientError::Signal(LinkError::Signal("join [4010] nope".into())).is_retryable());
        // non-auth 4xxx Server codes (e.g. 4031 produce rejection) conservatively non-retryable.
        assert!(!from_wire_error(4031, "video in audio room").is_retryable());
    }
}
