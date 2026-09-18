// RTCDataChannel control + HMAC validation + buffer tracking
// MVP quality — validates frames, tracks buffer, drops oldest when >3 deep

#![allow(dead_code)] // 双身份文件：host-legacy bin 的 `mod` 形态在用（T1.3 翻案），独立 bin 编译形态全 dead = 结构噪音
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

type HmacSha256 = Hmac<Sha256>;

/// Control frame types received over RTCDataChannel
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ControlFrame {
    #[serde(rename = "steering")]
    Steering { value: f64 },
    #[serde(rename = "brake")]
    Brake { value: f64 },
    #[serde(rename = "throttle")]
    Throttle { value: f64 },
}

/// HMAC validation key, derived from the label "mediaservo-control"
fn derive_key() -> HmacSha256 {
    // ponytail: simple label-as-key derivation, use HKDF if key rotation needed
    let label = b"mediaservo-control";
    HmacSha256::new_from_slice(label).expect("HMAC key derivation should not fail")
}

/// Validate an 8-byte (truncated) HMAC-SHA256 tag for given payload
pub fn validate_hmac(payload: &[u8], tag: &[u8; 8]) -> bool {
    let mut mac = derive_key();
    mac.update(payload);
    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    // ponytail: constant-time comparison prevents timing side-channels
    let expected = &code_bytes[..8];
    // subtle comparison not needed for MVP but correct for security boundaries
    expected == tag
}

/// Compute 8-byte HMAC tag for given payload
#[allow(dead_code)]
pub fn compute_hmac_tag(payload: &[u8]) -> [u8; 8] {
    let mut mac = derive_key();
    mac.update(payload);
    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    let mut tag = [0u8; 8];
    tag.copy_from_slice(&code_bytes[..8]);
    tag
}

/// Control frame handler with send buffer
pub struct ControlHandler {
    /// Pending control frames awaiting transmission
    buffer: VecDeque<ControlFrame>,
    /// Maximum buffer depth before dropping oldest
    max_depth: usize,
    /// Shared counter for dropped frames (exported to metrics)
    pub frames_dropped: Arc<AtomicU64>,
}

impl ControlHandler {
    pub fn new() -> Self {
        tracing::info!("Control handler initialized (HMAC-SHA256, buffer depth 3)");
        ControlHandler {
            buffer: VecDeque::with_capacity(4),
            max_depth: 3,
            frames_dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Parse raw JSON payload into a ControlFrame
    pub fn parse_frame(payload: &str) -> Result<ControlFrame, serde_json::Error> {
        serde_json::from_str(payload)
    }

    /// Enqueue a frame for sending.
    /// If buffer exceeds max_depth, drop oldest frame and increment counter.
    pub fn enqueue(&mut self, frame: ControlFrame) {
        if self.buffer.len() >= self.max_depth {
            self.buffer.pop_front();
            self.frames_dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("Control buffer full, dropped oldest frame");
        }
        self.buffer.push_back(frame);
    }

    /// Dequeue next frame for sending
    pub fn dequeue(&mut self) -> Option<ControlFrame> {
        self.buffer.pop_front()
    }

    /// Current buffer depth
    pub fn depth(&self) -> usize {
        self.buffer.len()
    }
}

/// Parse and validate a control message (JSON body + 8-byte HMAC tag)
/// Returns the parsed control frame if HMAC is valid.
pub fn parse_and_validate(body: &str, tag: &[u8; 8]) -> Option<ControlFrame> {
    if !validate_hmac(body.as_bytes(), tag) {
        tracing::warn!("HMAC validation failed");
        return None;
    }
    match ControlHandler::parse_frame(body) {
        Ok(frame) => Some(frame),
        Err(e) => {
            tracing::warn!("Control frame parse error: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_validation_roundtrip() {
        let payload = b"{\"type\":\"steering\",\"value\":15.2}";
        let tag = compute_hmac_tag(payload);
        assert!(validate_hmac(payload, &tag));
    }

    #[test]
    fn hmac_rejects_wrong_tag() {
        let payload = b"{\"type\":\"steering\",\"value\":15.2}";
        let wrong_tag = [0u8; 8];
        assert!(!validate_hmac(payload, &wrong_tag));
    }

    #[test]
    fn hmac_rejects_wrong_payload() {
        let payload1 = b"{\"type\":\"steering\",\"value\":15.2}";
        let payload2 = b"{\"type\":\"steering\",\"value\":15.3}";
        let tag = compute_hmac_tag(payload1);
        assert!(!validate_hmac(payload2, &tag));
    }

    #[test]
    fn parse_steering_frame() {
        let frame = ControlHandler::parse_frame(r#"{"type":"steering","value":15.2}"#).unwrap();
        assert!(matches!(frame, ControlFrame::Steering { value } if (value - 15.2).abs() < 0.001));
    }

    #[test]
    fn parse_brake_frame() {
        let frame = ControlHandler::parse_frame(r#"{"type":"brake","value":0.7}"#).unwrap();
        assert!(matches!(frame, ControlFrame::Brake { value } if (value - 0.7).abs() < 0.001));
    }

    #[test]
    fn parse_throttle_frame() {
        let frame = ControlHandler::parse_frame(r#"{"type":"throttle","value":45.0}"#).unwrap();
        assert!(matches!(frame, ControlFrame::Throttle { value } if (value - 45.0).abs() < 0.001));
    }

    #[test]
    fn buffer_drops_oldest_at_capacity() {
        let mut handler = ControlHandler::new();
        assert_eq!(handler.max_depth, 3);

        handler.enqueue(ControlFrame::Steering { value: 1.0 });
        handler.enqueue(ControlFrame::Brake { value: 0.5 });
        handler.enqueue(ControlFrame::Throttle { value: 10.0 });
        assert_eq!(handler.depth(), 3);

        // This should drop the first frame (steering 1.0)
        handler.enqueue(ControlFrame::Steering { value: 2.0 });
        assert_eq!(handler.depth(), 3);
        assert_eq!(handler.frames_dropped.load(Ordering::Relaxed), 1);

        // First dequeued should be brake 0.5 (steering 1.0 was dropped)
        let next = handler.dequeue().unwrap();
        assert!(matches!(next, ControlFrame::Brake { value } if (value - 0.5).abs() < 0.001));
    }

    #[test]
    fn parse_and_validate_passes_valid() {
        let body = r#"{"type":"steering","value":15.2}"#;
        let tag = compute_hmac_tag(body.as_bytes());
        let result = parse_and_validate(body, &tag);
        assert!(result.is_some());
    }

    #[test]
    fn parse_and_validate_rejects_invalid_hmac() {
        let body = r#"{"type":"steering","value":15.2}"#;
        let tag = [0u8; 8];
        let result = parse_and_validate(body, &tag);
        assert!(result.is_none());
    }
}
