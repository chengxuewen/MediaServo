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
    // W3C 口径补全（09-24 对表 web play：libwebrtc stats JSON 一直全带，此前只搬 10 字段）
    pub jitter: f64, // 秒（W3C）——面板显示乘 1000
    pub frame_dropped: u64,
    pub nack_count: u64,
    pub pli_count: u64,
    pub fir_count: u64,
}

/// Outbound RTP statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RTCOutboundRtpStreamStats {
    pub id: String,
    pub timestamp: f64,
    /// v2 (web-stream-stats T1.5): libwebrtc outbound-rtp 实际编码器实现名
    /// （如 "libvpx"/"OpenH264"/"VideoToolbox"）— 软编/硬编识别。
    pub encoder_implementation: Option<String>,
    pub ssrc: u32,
    pub kind: String,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frames_per_second: f64,
    /// v3 (encode-time-stats T2): 累计编码耗时（秒, W3C outbound-rtp 标准字段）—
    /// 平均每帧编码耗时 = ΔtotalEncodeTime / ΔframesEncoded（host 侧增量计算）。
    pub total_encode_time: Option<f64>,
}
