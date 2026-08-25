
use crate::track::{TrackKind, TrackRef};
use crate::RTCError;

/// 编码器后端选择（对齐 webrtc-sys VideoEncoderBackend）。
/// 语义: 偏好非强制 — 不可用时 libwebrtc 自动 fallback（video_encoder_factory.cpp:511-566 实证）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCVideoEncoderBackend {
    /// libwebrtc 默认策略（清空选择器）
    Auto,
    Software,
    Hardware,
    Nvenc,
    Vaapi,
    VideoToolbox,
    PreEncoded,
}

impl RTCVideoEncoderBackend {
    /// host.conf string ↔ enum（auto/software/hardware/nvenc/vaapi/videotoolbox）。
    pub fn from_config(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "software" => Some(Self::Software),
            "hardware" => Some(Self::Hardware),
            "nvenc" => Some(Self::Nvenc),
            "vaapi" => Some(Self::Vaapi),
            "videotoolbox" => Some(Self::VideoToolbox),
            "preencoded" => Some(Self::PreEncoded),
            _ => None,
        }
    }
}

/// W3C RTCRtpSender — wraps a TrackRef::Sender with sender metadata (D146).
#[derive(Debug, Clone)]
pub struct RTCRtpSender {
    pub track: TrackRef,
    pub track_id: String,
    pub kind: TrackKind,
    /// v2: backend 句柄（对象方法分派）— ActivePc 具体类型（async trait 非 object-safe）
    pub(crate) backend: Option<crate::backend::ActivePc>,
}

impl RTCRtpSender {
    pub fn new(track: TrackRef) -> Self {
        let track_id = track.id().to_string();
        let kind = track.kind();
        Self {
            track,
            track_id,
            kind,
            backend: None,
        }
    }

    /// v2: 绑定 backend 句柄（get_senders 构造时填充）
    pub fn with_backend(mut self, backend: crate::backend::ActivePc) -> Self {
        self.backend = Some(backend);
        self
    }

    /// W3C RTCRtpSender.getParameters — 经 backend 分派（PcBackend 同步方法）
    pub fn get_parameters(&self) -> Result<crate::rtp::RTCRtpParameters, RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_get_parameters(&self.track_id),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// W3C RTCRtpSender.setParameters — v2 补
    pub fn set_parameters(&self, _params: crate::rtp::RTCRtpParameters) -> Result<(), RTCError> {
        Err(RTCError::NotSupported("set_parameters: backend 未实现".into()))
    }

    /// v2 (encoder-bitrate): 设置发送编码器 min/max 码率（bps）。
    /// 聚焦式 API（非 W3C setParameters 全量）— 后端 cxx 保真往返只改 bitrate 字段（PIT-76）。
    /// min 为受限链路 best-effort 下限（libwebrtc 分配层生效, 编码器无硬下限）;
    /// max 为可靠硬上限。None = 不限制。
    pub fn set_encoding_bitrate(&self, min_bps: Option<u64>, max_bps: Option<u64>) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_encoding_bitrate(&self.track_id, min_bps, max_bps),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// v2 (multi-stream P1): 设置发送编码器帧率上限（fps）。聚焦式 API（非 W3C
    /// setParameters 全量）— 后端 cxx 保真往返只改 enc[].max_framerate 字段（bitrate 同模式）。
    /// None = 不限制（has_max_framerate=false）。
    pub fn set_encoding_framerate(&self, max_fps: Option<f64>) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_encoding_framerate(&self.track_id, max_fps),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// v2 (encoder-backend-codec-config T1): 设置编码器后端（软/硬, PcBackend track_id 分派）。
    /// SetEncoderSelector 语义: 偏好非强制（不可用自动 fallback）。
    pub fn set_video_encoder_backend(&self, backend: RTCVideoEncoderBackend) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_video_encoder_backend(&self.track_id, backend),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// W3C RTCRtpSender.replaceTrack — v2 补
    pub fn replace_track(&self, _new_track_id: &str) -> Result<(), RTCError> {
        Err(RTCError::NotSupported("replace_track: backend 未实现".into()))
    }

    /// W3C RTCRtpSender.setStreams — v2 补
    pub fn set_streams(&self, _stream_ids: Vec<String>) -> Result<(), RTCError> {
        Err(RTCError::NotSupported("set_streams: backend 未实现".into()))
    }
}

/// W3C RTCRtpReceiver — wraps a TrackRef::Receiver with receiver metadata (D146).
#[derive(Debug, Clone)]
pub struct RTCRtpReceiver {
    pub track: TrackRef,
    pub track_id: String,
    pub kind: TrackKind,
}

impl RTCRtpReceiver {
    pub fn new(track: TrackRef) -> Self {
        let track_id = track.id().to_string();
        let kind = track.kind();
        Self {
            track,
            track_id,
            kind,
        }
    }
}


/// W3C RTCRtpCodecParameters
#[derive(Debug, Clone)]
pub struct RTCRtpCodecParameters {
    pub mime_type: String, // "video/H264", "video/VP8", etc.
    pub payload_type: u8,
    pub clock_rate: u32,
    pub channels: Option<u16>, // for audio
    pub sdp_fmtp_line: Option<String>,
}

/// W3C RTCRtpEncodingParameters
#[derive(Debug, Clone)]
pub struct RTCRtpEncodingParameters {
    pub ssrc: Option<u64>,
    pub active: bool,
    pub max_bitrate: Option<u64>,
    /// v2 (encoder-bitrate): 最低码率 (bps) — 受限链路 best-effort 下限, None=不限制
    pub min_bitrate: Option<u64>,
    pub max_framerate: Option<f64>,
    pub scale_resolution_down_by: Option<f64>,
    pub rid: Option<String>,
    /// W3C: 编码选择（引用 codecs 中的 mime_type）— v2 补
    pub codec: Option<String>,
    /// W3C: 音频断续传输 (discontinuous transmission) — v2 补
    pub dtx: Option<bool>,
    /// W3C requestKeyFrame — 周期关键帧触发（PIT-76）。
    /// libwebrtc 消费后内部清标志；每次 set_parameters 传 true 恰好触发一次。
    pub request_key_frame: bool,
}

impl Default for RTCRtpEncodingParameters {
    fn default() -> Self {
        Self {
            ssrc: None,
            active: true,
            max_bitrate: None,
            min_bitrate: None,
            max_framerate: None,
            scale_resolution_down_by: None,
            rid: None,
            codec: None,
            dtx: None,
            request_key_frame: false,
        }
    }
}

/// W3C RTCRtpHeaderExtensionParameters
#[derive(Debug, Clone)]
pub struct RTCRtpHeaderExtensionParameters {
    pub uri: String,
    pub id: u16,
    pub encrypted: bool,
}

/// W3C RTCRtcpParameters
#[derive(Debug, Clone, Default)]
pub struct RTCRtcpParameters {
    pub cname: Option<String>,
    /// When true, indicates reduced-size RTCP.
    #[allow(dead_code)]
    pub reduced_size: bool,
}

/// W3C RTCRtpParameters
#[derive(Debug, Clone, Default)]
pub struct RTCRtpParameters {
    pub transaction_id: String,
    /// W3C: m-line 关联 ID — v2 补
    pub mid: String,
    pub codecs: Vec<RTCRtpCodecParameters>,
    pub encodings: Vec<RTCRtpEncodingParameters>,
    pub header_extensions: Vec<RTCRtpHeaderExtensionParameters>,
    pub rtcp: RTCRtcpParameters,
}

/// W3C RTCRtpTransceiverDirection (spec: RTCRtpTransceiver.direction)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCRtpTransceiverDirection {
    Sendrecv,
    Sendonly,
    Recvonly,
    Inactive,
}

impl RTCRtpTransceiverDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            RTCRtpTransceiverDirection::Sendrecv => "sendrecv",
            RTCRtpTransceiverDirection::Sendonly => "sendonly",
            RTCRtpTransceiverDirection::Recvonly => "recvonly",
            RTCRtpTransceiverDirection::Inactive => "inactive",
        }
    }
}

/// W3C RTCRtpTransceiverInit (spec: addTransceiver init)
#[derive(Debug, Clone)]
pub struct RTCRtpTransceiverInit {
    pub direction: RTCRtpTransceiverDirection,
    pub send_encodings: Vec<RTCRtpEncodingParameters>,
    pub stream_ids: Vec<String>,
}

impl Default for RTCRtpTransceiverInit {
    fn default() -> Self {
        Self {
            direction: RTCRtpTransceiverDirection::Sendrecv,
            send_encodings: vec![],
            stream_ids: vec![],
        }
    }
}

/// W3C RTCRtpTransceiver — 纯数据视图（协商后快照语义）。
/// 内部持有不透明 backend 句柄（SharedPtr/Arc，不 pub）防 GC。
/// 对象方法（set_direction/stop/set_codec_preferences）经 backend 分派。
#[derive(Debug, Clone)]
pub struct RTCRtpTransceiver {
    pub mid: Option<String>,
    /// 期望方向（可写，经 set_direction）
    pub direction: RTCRtpTransceiverDirection,
    /// 协商后方向（只读）
    pub current_direction: Option<RTCRtpTransceiverDirection>,
    /// W3C: 是否已停止 — v2 补
    pub stopped: bool,
    pub sender: RTCRtpSender,
    pub receiver: RTCRtpReceiver,
    pub kind: TrackKind,
}

impl RTCRtpTransceiver {
    pub fn new(
        mid: Option<String>,
        direction: RTCRtpTransceiverDirection,
        current_direction: Option<RTCRtpTransceiverDirection>,
        stopped: bool,
        sender: RTCRtpSender,
        receiver: RTCRtpReceiver,
        kind: TrackKind,
    ) -> Self {
        Self {
            mid,
            direction,
            current_direction,
            stopped,
            sender,
            receiver,
            kind,
        }
    }
}

/// W3C RTCRtpCapabilities (spec: getCapabilities 返回值)
#[derive(Debug, Clone, Default)]
pub struct RTCRtpCapabilities {
    pub codecs: Vec<RTCRtpCodecCapability>,
    pub header_extensions: Vec<RTCRtpHeaderExtensionCapability>,
}

/// W3C RTCRtpCodecCapability
#[derive(Debug, Clone)]
pub struct RTCRtpCodecCapability {
    pub mime_type: String,
    pub clock_rate: Option<u32>,
    pub channels: Option<u16>,
    pub sdp_fmtp_line: Option<String>,
}

/// W3C RTCRtpHeaderExtensionCapability
#[derive(Debug, Clone)]
pub struct RTCRtpHeaderExtensionCapability {
    pub uri: String,
    pub id: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_from_config_mapping() {
        // v2 (encoder-backend-codec-config T2): host.conf string ↔ enum
        assert_eq!(RTCVideoEncoderBackend::from_config("auto"), Some(RTCVideoEncoderBackend::Auto));
        assert_eq!(RTCVideoEncoderBackend::from_config("software"), Some(RTCVideoEncoderBackend::Software));
        assert_eq!(RTCVideoEncoderBackend::from_config("hardware"), Some(RTCVideoEncoderBackend::Hardware));
        assert_eq!(RTCVideoEncoderBackend::from_config("nvenc"), Some(RTCVideoEncoderBackend::Nvenc));
        assert_eq!(RTCVideoEncoderBackend::from_config("vaapi"), Some(RTCVideoEncoderBackend::Vaapi));
        assert_eq!(RTCVideoEncoderBackend::from_config("videotoolbox"), Some(RTCVideoEncoderBackend::VideoToolbox));
        // 大小写不敏感 + 未知值 None
        assert_eq!(RTCVideoEncoderBackend::from_config("HARDWARE"), Some(RTCVideoEncoderBackend::Hardware));
        assert_eq!(RTCVideoEncoderBackend::from_config("cuda"), None);
        assert_eq!(RTCVideoEncoderBackend::from_config(""), None);
    }
}
