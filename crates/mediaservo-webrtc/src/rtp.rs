use crate::RTCError;
use crate::track::{TrackKind, TrackRef};

/// encoder backend selection (aligned with webrtc-sys VideoEncoderBackend).
/// semantics: a preference, not a mandate -- libwebrtc falls back automatically when unavailable (proven at video_encoder_factory.cpp:511-566).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTCVideoEncoderBackend {
    /// the libwebrtc default policy (selector cleared)
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

/// sender-side degradation preference (aligned with vendor webrtc-sys DegradationPreference's 4 variants, C18 no trimming).
/// Fixed = vendor MaintainFramerateAndResolution (both framerate/resolution kept, frames dropped only).
/// The string forms (fixed/framerate/resolution/balanced) are reused by the translate/streamer parsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RTCDegradationPreference {
    /// no degradation (frames dropped only) -- vendor MaintainFramerateAndResolution
    Fixed,
    /// keep framerate, drop resolution (weak-network realtime first, the teleop scenario)
    MaintainFramerate,
    /// keep resolution, drop framerate (image quality/forensics first)
    MaintainResolution,
    /// balance both (the libwebrtc default semantics, this lib's default)
    #[default]
    Balanced,
}

impl std::str::FromStr for RTCDegradationPreference {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "fixed" => Ok(Self::Fixed),
            "framerate" => Ok(Self::MaintainFramerate),
            "resolution" => Ok(Self::MaintainResolution),
            "balanced" => Ok(Self::Balanced),
            _ => Err(format!(
                "illegal degradation value {s:?} (legal: fixed|framerate|resolution|balanced)"
            )),
        }
    }
}

/// sender-side content hint (aligned with the W3C contentHint; this lib only lands {None, Fluid}, AD-5 --
/// Detailed/Text have no monitoring-camera scenario, YAGNI, extending the enum is purely additive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RTCRtpContentHint {
    #[default]
    None,
    /// high-motion content (libwebrtc leans toward keeping framerate)
    Fluid,
}

impl std::str::FromStr for RTCRtpContentHint {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "fluid" => Ok(Self::Fluid),
            _ => Err(format!("illegal content-hint value {s:?} (legal: fluid)")),
        }
    }
}

/// W3C RTCRtpSender — wraps a TrackRef::Sender with sender metadata (D146).
#[derive(Debug, Clone)]
pub struct RTCRtpSender {
    pub track: TrackRef,
    pub track_id: String,
    pub kind: TrackKind,
    /// v2: the backend handle (object-method dispatch) -- the ActivePc concrete type (async traits are not object-safe)
    pub(crate) backend: Option<crate::backend::ActivePc>,
}

impl RTCRtpSender {
    pub fn new(track: TrackRef) -> Self {
        let track_id = track.id().to_string();
        let kind = track.kind();
        Self { track, track_id, kind, backend: None }
    }

    /// v2: bind the backend handle (filled when get_senders constructs)
    pub fn with_backend(mut self, backend: crate::backend::ActivePc) -> Self {
        self.backend = Some(backend);
        self
    }

    /// W3C RTCRtpSender.getParameters -- dispatched via the backend (a PcBackend sync method)
    pub fn get_parameters(&self) -> Result<crate::rtp::RTCRtpParameters, RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_get_parameters(&self.track_id),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// W3C RTCRtpSender.setParameters -- added in v2
    pub fn set_parameters(&self, _params: crate::rtp::RTCRtpParameters) -> Result<(), RTCError> {
        Err(RTCError::NotSupported("set_parameters: the backend has not implemented it".into()))
    }

    /// v2 (encoder-bitrate): set the sending encoder's min/max bitrate (bps).
    /// A focused API (not the full W3C setParameters) -- the backend's cxx-faithful round trip touches only the bitrate fields (PIT-76).
    /// min = the best-effort floor on constrained links (effective at the libwebrtc allocation layer, no hard encoder floor);
    /// max = the reliable hard ceiling. None = unlimited.
    pub fn set_encoding_bitrate(
        &self,
        min_bps: Option<u64>,
        max_bps: Option<u64>,
    ) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_encoding_bitrate(&self.track_id, min_bps, max_bps),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// v2 (multi-stream P1): set the sending encoder's framerate cap (fps). A focused API (not the full W3C
    /// setParameters) -- the backend's cxx-faithful round trip touches only the enc[].max_framerate field (the same pattern as bitrate).
    /// None = unlimited (has_max_framerate=false).
    pub fn set_encoding_framerate(&self, max_fps: Option<f64>) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_encoding_framerate(&self.track_id, max_fps),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// v2 (encoder-backend-codec-config T1): set the encoder backend (soft/hard, dispatched via PcBackend track_id).
    /// SetEncoderSelector semantics: a preference, not a mandate (automatic fallback when unavailable).
    pub fn set_video_encoder_backend(
        &self,
        backend: RTCVideoEncoderBackend,
    ) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_video_encoder_backend(&self.track_id, backend),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// v2 (qos-framerate-priority): set the degradation preference -- a focused API (the same pattern as bitrate),
    /// the backend's cxx-faithful round trip touches only the RtpParameters.degradation_preference field.
    pub fn set_degradation_preference(
        &self,
        pref: RTCDegradationPreference,
    ) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_degradation_preference(&self.track_id, pref),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// v2 (qos-framerate-priority): set the content hint (track-level, not SetParameters) --
    /// the backend goes through media_to_video to the VideoTrack then set_content_hint.
    pub fn set_content_hint(&self, hint: RTCRtpContentHint) -> Result<(), RTCError> {
        use crate::backend::PcBackend as _;
        match &self.backend {
            Some(b) => b.sender_set_content_hint(&self.track_id, hint),
            None => Err(RTCError::Internal("sender not bound to a peer connection".into())),
        }
    }

    /// W3C RTCRtpSender.replaceTrack -- added in v2
    pub fn replace_track(&self, _new_track_id: &str) -> Result<(), RTCError> {
        Err(RTCError::NotSupported("replace_track: the backend has not implemented it".into()))
    }

    /// W3C RTCRtpSender.setStreams -- added in v2
    pub fn set_streams(&self, _stream_ids: Vec<String>) -> Result<(), RTCError> {
        Err(RTCError::NotSupported("set_streams: the backend has not implemented it".into()))
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
        Self { track, track_id, kind }
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
    /// v2 (encoder-bitrate): the min bitrate (bps) -- the best-effort floor on constrained links, None = unlimited
    pub min_bitrate: Option<u64>,
    pub max_framerate: Option<f64>,
    pub scale_resolution_down_by: Option<f64>,
    pub rid: Option<String>,
    /// W3C: codec selection (references mime_type within codecs) -- added in v2
    pub codec: Option<String>,
    /// W3C: discontinuous transmission (DTX) -- added in v2
    pub dtx: Option<bool>,
    /// W3C requestKeyFrame -- periodic keyframe triggering (PIT-76).
    /// libwebrtc clears the flag internally after consuming; each set_parameters passing true triggers exactly one.
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
    /// W3C: the m-line association ID -- added in v2
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

/// W3C RTCRtpTransceiver -- a pure-data view (post-negotiation snapshot semantics).
/// Internally holds an opaque backend handle (SharedPtr/Arc, not pub) to defeat GC.
/// Object methods (set_direction/stop/set_codec_preferences) dispatch via the backend.
#[derive(Debug, Clone)]
pub struct RTCRtpTransceiver {
    pub mid: Option<String>,
    /// The desired direction (writable, via set_direction)
    pub direction: RTCRtpTransceiverDirection,
    /// The negotiated direction (read-only)
    pub current_direction: Option<RTCRtpTransceiverDirection>,
    /// W3C: whether stopped -- added in v2
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
        Self { mid, direction, current_direction, stopped, sender, receiver, kind }
    }
}

/// W3C RTCRtpCapabilities (spec: the getCapabilities return value)
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
        assert_eq!(
            RTCVideoEncoderBackend::from_config("software"),
            Some(RTCVideoEncoderBackend::Software)
        );
        assert_eq!(
            RTCVideoEncoderBackend::from_config("hardware"),
            Some(RTCVideoEncoderBackend::Hardware)
        );
        assert_eq!(
            RTCVideoEncoderBackend::from_config("nvenc"),
            Some(RTCVideoEncoderBackend::Nvenc)
        );
        assert_eq!(
            RTCVideoEncoderBackend::from_config("vaapi"),
            Some(RTCVideoEncoderBackend::Vaapi)
        );
        assert_eq!(
            RTCVideoEncoderBackend::from_config("videotoolbox"),
            Some(RTCVideoEncoderBackend::VideoToolbox)
        );
        // case-insensitive + unknown values None
        assert_eq!(
            RTCVideoEncoderBackend::from_config("HARDWARE"),
            Some(RTCVideoEncoderBackend::Hardware)
        );
        assert_eq!(RTCVideoEncoderBackend::from_config("cuda"), None);
        assert_eq!(RTCVideoEncoderBackend::from_config(""), None);
    }

    // qos-framerate-priority T1: DegradationPreference / ContentHint parsing and defaults
    #[test]
    fn degradation_preference_from_str_legal() {
        assert_eq!(
            "fixed".parse::<RTCDegradationPreference>().unwrap(),
            RTCDegradationPreference::Fixed
        );
        assert_eq!(
            "framerate".parse::<RTCDegradationPreference>().unwrap(),
            RTCDegradationPreference::MaintainFramerate
        );
        assert_eq!(
            "resolution".parse::<RTCDegradationPreference>().unwrap(),
            RTCDegradationPreference::MaintainResolution
        );
        assert_eq!(
            "balanced".parse::<RTCDegradationPreference>().unwrap(),
            RTCDegradationPreference::Balanced
        );
        // case/whitespace-insensitive (the from_config style)
        assert_eq!(
            " FRAMERATE ".parse::<RTCDegradationPreference>().unwrap(),
            RTCDegradationPreference::MaintainFramerate
        );
    }

    #[test]
    fn degradation_preference_from_str_illegal() {
        assert!("cuda".parse::<RTCDegradationPreference>().is_err());
        assert!("".parse::<RTCDegradationPreference>().is_err());
    }

    #[test]
    fn degradation_preference_default_is_balanced() {
        assert_eq!(RTCDegradationPreference::default(), RTCDegradationPreference::Balanced);
    }

    #[test]
    fn content_hint_fluid_parses_others_err() {
        assert_eq!("fluid".parse::<RTCRtpContentHint>().unwrap(), RTCRtpContentHint::Fluid);
        assert!("detailed".parse::<RTCRtpContentHint>().is_err());
        assert!("none".parse::<RTCRtpContentHint>().is_err());
        assert!("".parse::<RTCRtpContentHint>().is_err());
    }

    #[test]
    fn content_hint_default_is_none() {
        assert_eq!(RTCRtpContentHint::default(), RTCRtpContentHint::None);
    }
}
