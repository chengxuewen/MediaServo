//! Unified error codes for all MediaServo components.
//!
//! Error code ranges:
//! - 1xxx — connectivity (WebSocket, ICE, network)
//! - 2xxx — encoding
//! - 3xxx — capture
//! - 4xxx — relay / server
//! - 5xxx — decoding / render
//! - 6xxx — control
//! - 9xxx — system (OOM, config, unknown)

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    // --- 1xxx: Connectivity ---
    /// WebSocket disconnect (Host/Remote ↔ Server signaling)
    #[error("[1001] WebSocket disconnected: {0}")]
    WebSocketDisconnect(String),

    /// ICE connection timeout
    #[error("[1003] ICE connection timeout")]
    IceTimeout,

    /// RTCPeerConnection creation failed
    #[error("[1004] RTCPeerConnection creation failed: {0}")]
    PeerConnectionFailure(String),

    // --- 2xxx: Encoding ---
    /// Encoder initialization failed
    #[error("[2001] Encoder init failed: {0}")]
    EncoderInit(String),

    // --- 3xxx: Capture ---
    /// Capture source not found
    #[error("[3001] Capture source not found: {0}")]
    CaptureSourceNotFound(String),

    /// Capture source disconnected
    #[error("[3002] Capture source disconnected")]
    CaptureDisconnected,

    // --- 4xxx: Relay / Server ---
    /// Relay track binding failed
    #[error("[4001] Relay track bind failed: {0}")]
    RelayTrackBind(String),

    /// Room is full
    #[error("[4002] Room full")]
    RoomFull,

    /// PSK authentication failed
    #[error("[4003] PSK authentication failed")]
    PskAuthFailed,

    // --- 5xxx: Decode / Render ---
    /// Decoder init failed
    #[error("[5001] Decoder init failed: {0}")]
    DecoderInit(String),

    // --- 6xxx: Control ---
    /// HMAC signature verification failed
    #[error("[6001] Control HMAC verification failed")]
    ControlHmacFailed,

    // --- 9xxx: System ---
    /// Out of memory
    #[error("[9001] Out of memory (RSS limit exceeded)")]
    OutOfMemory,

    /// Config parse error
    #[error("[9002] Config parse error: {0}")]
    ConfigParse(String),

    /// Unknown system error
    #[error("[9003] Unknown system error: {0}")]
    Unknown(String),
}

/// Recoverability classification.
impl CoreError {
    /// Returns true if the error is transient and retry may succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CoreError::WebSocketDisconnect(_)
                | CoreError::IceTimeout
                | CoreError::CaptureDisconnected
                | CoreError::RoomFull
        )
    }

    /// Returns true if the error is fatal (component must restart).
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            CoreError::EncoderInit(_)
                | CoreError::CaptureSourceNotFound(_)
                | CoreError::DecoderInit(_)
                | CoreError::OutOfMemory
                | CoreError::ConfigParse(_)
        )
    }

    /// Return the numeric error code as a stable, context-free key.
    /// Used by future i18n layers to look up locale-specific text.
    pub fn locale_key(&self) -> &'static str {
        match self {
            CoreError::WebSocketDisconnect(_) => "1001",
            CoreError::IceTimeout => "1003",
            CoreError::PeerConnectionFailure(_) => "1004",
            CoreError::EncoderInit(_) => "2001",
            CoreError::CaptureSourceNotFound(_) => "3001",
            CoreError::CaptureDisconnected => "3002",
            CoreError::RelayTrackBind(_) => "4001",
            CoreError::RoomFull => "4002",
            CoreError::PskAuthFailed => "4003",
            CoreError::DecoderInit(_) => "5001",
            CoreError::ControlHmacFailed => "6001",
            CoreError::OutOfMemory => "9001",
            CoreError::ConfigParse(_) => "9002",
            CoreError::Unknown(_) => "9003",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_errors() {
        assert!(CoreError::WebSocketDisconnect("lost".into()).is_retryable());
        assert!(CoreError::IceTimeout.is_retryable());
        assert!(CoreError::CaptureDisconnected.is_retryable());
        assert!(CoreError::RoomFull.is_retryable());
    }

    #[test]
    fn non_retryable_errors() {
        assert!(!CoreError::EncoderInit("hw".into()).is_retryable());
        assert!(!CoreError::CaptureSourceNotFound("dev".into()).is_retryable());
        assert!(!CoreError::RelayTrackBind("bind".into()).is_retryable());
        assert!(!CoreError::PskAuthFailed.is_retryable());
        assert!(!CoreError::DecoderInit("dec".into()).is_retryable());
        assert!(!CoreError::ControlHmacFailed.is_retryable());
        assert!(!CoreError::OutOfMemory.is_retryable());
        assert!(!CoreError::ConfigParse("bad".into()).is_retryable());
        assert!(!CoreError::Unknown("?".into()).is_retryable());
        assert!(!CoreError::PeerConnectionFailure("pc".into()).is_retryable());
    }

    #[test]
    fn fatal_errors() {
        assert!(CoreError::EncoderInit("hw".into()).is_fatal());
        assert!(CoreError::CaptureSourceNotFound("dev".into()).is_fatal());
        assert!(CoreError::DecoderInit("dec".into()).is_fatal());
        assert!(CoreError::OutOfMemory.is_fatal());
        assert!(CoreError::ConfigParse("bad".into()).is_fatal());
    }

    #[test]
    fn non_fatal_errors() {
        assert!(!CoreError::WebSocketDisconnect("lost".into()).is_fatal());
        assert!(!CoreError::IceTimeout.is_fatal());
        assert!(!CoreError::CaptureDisconnected.is_fatal());
        assert!(!CoreError::RoomFull.is_fatal());
        assert!(!CoreError::RelayTrackBind("bind".into()).is_fatal());
        assert!(!CoreError::PskAuthFailed.is_fatal());
        assert!(!CoreError::ControlHmacFailed.is_fatal());
        assert!(!CoreError::Unknown("?".into()).is_fatal());
        assert!(!CoreError::PeerConnectionFailure("pc".into()).is_fatal());
    }

    #[test]
    fn display_includes_error_code_and_message() {
        let err = CoreError::WebSocketDisconnect("timeout".into());
        let s = err.to_string();
        assert!(s.contains("[1001]"), "expected [1001] in '{s}'");
        assert!(s.contains("timeout"), "expected 'timeout' in '{s}'");

        let err = CoreError::OutOfMemory;
        let s = err.to_string();
        assert!(s.contains("[9001]"), "expected [9001] in '{s}'");

        let err = CoreError::ConfigParse("invalid yaml".into());
        let s = err.to_string();
        assert!(s.contains("[9002]"), "expected [9002] in '{s}'");
        assert!(s.contains("invalid yaml"), "expected 'invalid yaml' in '{s}'");

        let err = CoreError::RoomFull;
        let s = err.to_string();
        assert!(s.contains("[4002]"), "expected [4002] in '{s}'");
    }

    #[test]
    fn error_codes_span_expected_ranges() {
        // Verify representative error codes across all defined ranges
        let err_1xxx = CoreError::WebSocketDisconnect("x".into()).to_string();
        assert!(err_1xxx.contains("[1001]"), "1xxx range");
        let err_2xxx = CoreError::EncoderInit("x".into()).to_string();
        assert!(err_2xxx.contains("[2001]"), "2xxx range");
        let err_3xxx = CoreError::CaptureSourceNotFound("x".into()).to_string();
        assert!(err_3xxx.contains("[3001]"), "3xxx range");
        let err_4xxx = CoreError::RelayTrackBind("x".into()).to_string();
        assert!(err_4xxx.contains("[4001]"), "4xxx range");
        let err_5xxx = CoreError::DecoderInit("x".into()).to_string();
        assert!(err_5xxx.contains("[5001]"), "5xxx range");
        let err_6xxx = CoreError::ControlHmacFailed.to_string();
        assert!(err_6xxx.contains("[6001]"), "6xxx range");
        let err_9xxx = CoreError::Unknown("x".into()).to_string();
        assert!(err_9xxx.contains("[9003]"), "9xxx range");
    }
}
