//! SDP session description types.
//!
//! Follows the webrtc-kit convention: SDP strings use "\n" as separator
//! between type and body (e.g., "offer\nv=0\r\n...").

use serde::{Deserialize, Serialize};
use std::fmt;

/// SDP type as per W3C RTCSdpType.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RTCSdpType {
    Offer,
    PrAnswer,
    Answer,
    Rollback,
}

impl fmt::Display for RTCSdpType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RTCSdpType::Offer => "offer",
            RTCSdpType::PrAnswer => "pranswer",
            RTCSdpType::Answer => "answer",
            RTCSdpType::Rollback => "rollback",
        };
        write!(f, "{s}")
    }
}

/// A parsed session description with type and SDP body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RTCSessionDescription {
    #[serde(rename = "type")]
    pub sdp_type: RTCSdpType,
    pub sdp: String,
}

impl RTCSessionDescription {
    /// Create from type+body (webrtc-kit compatible format).
    pub fn new(sdp_type: RTCSdpType, sdp: String) -> Self {
        Self { sdp_type, sdp }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_each_type() {
        assert_eq!(RTCSdpType::Offer.to_string(), "offer");
        assert_eq!(RTCSdpType::PrAnswer.to_string(), "pranswer");
        assert_eq!(RTCSdpType::Answer.to_string(), "answer");
        assert_eq!(RTCSdpType::Rollback.to_string(), "rollback");
    }

    #[test]
    fn session_description_new() {
        let sd = RTCSessionDescription::new(RTCSdpType::Offer, "v=0".into());
        assert_eq!(sd.sdp_type, RTCSdpType::Offer);
        assert_eq!(sd.sdp, "v=0");
    }

    #[test]
    fn serde_camel_case_roundtrip() {
        let sd = RTCSessionDescription { sdp_type: RTCSdpType::Answer, sdp: "v=0\r\n".into() };
        let json = serde_json::to_string(&sd).unwrap();
        // camelCase: {"type":"answer","sdp":"v=0\r\n"}
        assert!(json.contains("\"type\":\"answer\""));
        let parsed: RTCSessionDescription = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sdp_type, RTCSdpType::Answer);
        assert_eq!(parsed.sdp, "v=0\r\n");
    }
}
