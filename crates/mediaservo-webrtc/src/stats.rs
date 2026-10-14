//! W3C RTCStats types — structured getStats return type.
//!
//! Ported from webrtc-kit rtc/core.rs.
//! Provides 5 core stat types matching the W3C WebRTC Stats API.

/// W3C RTCStats with 5 core stat types.
#[derive(Debug, Clone, serde::Serialize)]
pub enum RTCStats {
    RTCPeerConnection(RTCPeerConnectionStats),
    Transport(RTCTransportStats),
    Codec(RTCCodecStats),
    InboundRtp(RTCInboundRtpStreamStats),
    OutboundRtp(RTCOutboundRtpStreamStats),
}

/// Peer connection statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RTCPeerConnectionStats {
    pub id: String,
    pub timestamp: f64,
    pub data_channels_opened: u32,
    pub data_channels_closed: u32,
}

/// Transport-level statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RTCTransportStats {
    pub id: String,
    pub timestamp: f64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub dtls_state: Option<String>,
    pub selected_candidate_pair_id: Option<String>,
}

/// Codec statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RTCCodecStats {
    pub id: String,
    pub timestamp: f64,
    pub payload_type: u8,
    pub mime_type: String,
    pub clock_rate: u32,
    pub channels: Option<u16>,
}

/// Inbound RTP statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RTCInboundRtpStreamStats {
    pub id: String,
    pub timestamp: f64,
    pub ssrc: u32,
    pub kind: String,
    pub packets_received: u64,
    pub packets_lost: u64,
    pub bytes_received: u64,
    pub frames_decoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frames_per_second: f64,
    // the W3C convention completion (09-24 reconciled against web play: libwebrtc's stats JSON always carried them all; previously only 10 fields were ported)
    pub jitter: f64, // seconds (W3C) -- the panel multiplies by 1000 for display
    /// The cumulative decode time (seconds, W3C totalDecodeTime) -- avg = the delta-decode/delta-frames differential ratio
    pub total_decode_time: f64,
    pub frame_dropped: u64,
    pub nack_count: u64,
    pub pli_count: u64,
    pub fir_count: u64,
}

/// Connection-level stats (candidate-pair) -- the sole W3C-convention source of rtt (receiver-level getStats has no such row).
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct ConnectionStats {
    /// The selected pair's round-trip time (ms; currentRoundTripTime seconds x 1000)
    pub rtt_ms: f64,
}

/// Outbound RTP statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RTCOutboundRtpStreamStats {
    pub id: String,
    pub timestamp: f64,
    /// v2 (web-stream-stats T1.5): libwebrtc outbound-rtp's actual encoder implementation name
    /// (e.g. "libvpx"/"OpenH264"/"VideoToolbox") -- software/hardware encoder identification.
    pub encoder_implementation: Option<String>,
    pub ssrc: u32,
    pub kind: String,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frames_per_second: f64,
    /// v3 (encode-time-stats T2): the cumulative encode time (seconds, the W3C outbound-rtp standard field) --
    /// average encode ms/frame = delta totalEncodeTime / delta framesEncoded (computed incrementally on the host side).
    pub total_encode_time: Option<f64>,
}
