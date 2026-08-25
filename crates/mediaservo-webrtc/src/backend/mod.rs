//! Backend abstraction layer for multi-backend WebRTC support.
//!
//! Defines traits (PcBackend, DcBackend, TrackWriteBackend)
//! and compile-time type alias dispatch via cfg gates.
//! Zero dyn overhead — all dispatch is monomorphized.

use crate::data_channel::{RTCDataChannel, RTCDataChannelRx, RTCDataChannelState};
use crate::peer_connection::{RTCAnswerOptions, RTCIceCandidate, RTCOfferOptions, RTCIceConnectionState, RTCIceGatheringState, RTCPeerConnectionState, RTCSignalingState, RTCConfiguration};
use crate::sdp::RTCSessionDescription;
use crate::stats::RTCStats;
use crate::track::{RTCAudioTrackConfig, TrackKind, TrackReceiver, TrackSender};
use crate::rtp::{RTCRtpCapabilities, RTCRtpCodecCapability, RTCRtpParameters, RTCRtpTransceiver, RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use crate::RTCError;

// ── Traits ──

pub(crate) trait PcBackend: Send + Sync + 'static {
    async fn create_offer(&self, options: &RTCOfferOptions) -> Result<RTCSessionDescription, RTCError>;
    async fn create_answer(&self, options: &RTCAnswerOptions) -> Result<RTCSessionDescription, RTCError>;
    async fn set_local_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError>;
    async fn set_remote_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError>;
    async fn add_ice_candidate(&self, candidate: &RTCIceCandidate) -> Result<(), RTCError>;
    fn connection_state(&self) -> RTCPeerConnectionState;
    fn ice_connection_state(&self) -> RTCIceConnectionState;
    fn ice_gathering_state(&self) -> RTCIceGatheringState;
    fn signaling_state(&self) -> RTCSignalingState;
    async fn close(&self);

    // ── Default methods (no-op for backends that skip these) ──

    /// Register callback for incoming data channels (receiver side).
    fn set_on_data_channel(&self, _cb: Box<dyn Fn(RTCDataChannel) + Send + Sync + 'static>) {}

    /// Register callback for incoming remote tracks (receiver side).
    fn set_on_track(&self, _cb: Box<dyn Fn(TrackReceiver) + Send + Sync + 'static>) {}

    /// Wait until ICE gathering is complete.
    fn gather_complete(&self) -> Result<(), RTCError> {
        Ok(())
    }

    /// Get structured statistics.
    fn get_stats(&self) -> Vec<RTCStats> {
        vec![]
    }
    /// 旧版 add_transceiver (str, str) — v2 重命名，避免与 W3C 签名同名冲突（零调用方）
    #[allow(dead_code)]
    fn add_transceiver_legacy(&self, _media_type: &str, _direction: &str) -> Result<(), RTCError> {
        Err(RTCError::Internal("not supported".into()))
    }

    /// W3C getTransceivers — 返回所有 transceiver 的纯数据视图（v2）
    /// 默认 Err(NotSupported) + warn（C15 防静默降级），stub 覆盖为 Ok(vec![])
    fn get_transceivers(&self) -> Result<Vec<RTCRtpTransceiver>, RTCError> {
        tracing::warn!("get_transceivers: not supported by backend");
        Err(RTCError::NotSupported("get_transceivers".into()))
    }

    /// W3C addTransceiver(kind, init) — kind 版（同步，v2）
    fn add_transceiver(&self, kind: TrackKind, init: &RTCRtpTransceiverInit) -> Result<RTCRtpTransceiver, RTCError> {
        tracing::warn!("add_transceiver: not supported by backend");
        Err(RTCError::NotSupported("add_transceiver".into()))
    }

    /// W3C addTransceiver(track, init) — track 版重载（v2，P3 核心：kind 版会产生无法写帧的空 track）
    fn add_transceiver_with_track(&self, track: &TrackSender, init: &RTCRtpTransceiverInit) -> Result<RTCRtpTransceiver, RTCError> {
        tracing::warn!("add_transceiver_with_track: not supported by backend");
        Err(RTCError::NotSupported("add_transceiver_with_track".into()))
    }

    /// W3C RTCRtpSender.getParameters — 经 track_id 分派（同步）
    fn sender_get_parameters(&self, _track_id: &str) -> Result<RTCRtpParameters, RTCError> {
        tracing::warn!("sender_get_parameters: not supported by backend");
        Err(RTCError::NotSupported("sender_get_parameters".into()))
    }

    /// W3C RTCRtpReceiver.getParameters — 经 track_id 分派（同步）
    fn receiver_get_parameters(&self, _track_id: &str) -> Result<RTCRtpParameters, RTCError> {
        tracing::warn!("receiver_get_parameters: not supported by backend");
        Err(RTCError::NotSupported("receiver_get_parameters".into()))
    }

/// W3C RTCRtpSender.setParameters — v2 补
fn sender_set_parameters(&self, _track_id: &str, _params: &RTCRtpParameters) -> Result<(), RTCError> {
tracing::warn!("sender_set_parameters: not supported by backend");
Err(RTCError::NotSupported("sender_set_parameters".into()))
    }

    /// v2 (encoder-bitrate): 设置发送编码器 min/max 码率（bps）。
    /// 默认 NotSupported + warn（C15）; webrtc-sys 后端 override 为 cxx 保真往返。
    fn sender_set_encoding_bitrate(&self, _track_id: &str, _min_bps: Option<u64>, _max_bps: Option<u64>) -> Result<(), RTCError> {
        tracing::warn!("sender_set_encoding_bitrate: not supported by backend");
        Err(RTCError::NotSupported("sender_set_encoding_bitrate".into()))
    }

    /// v2 (multi-stream P1): 设置发送编码器帧率上限（fps）。
    /// 默认 NotSupported + warn（C15）; webrtc-sys 后端 override（复制 bitrate 模式，
    /// get_parameters → enc[].max_framerate → set_parameters）。
    fn sender_set_encoding_framerate(&self, _track_id: &str, _max_fps: Option<f64>) -> Result<(), RTCError> {
        tracing::warn!("sender_set_encoding_framerate: not supported by backend");
        Err(RTCError::NotSupported("sender_set_encoding_framerate".into()))
    }

    /// v2 (encoder-backend-codec-config T1): 设置发送编码器后端（软/硬实现选择）。
    /// 经 track_id 分派（sender.track().id() 匹配, request_key_frame 同模式）。
    /// 默认 NotSupported + warn（C15: 错误分支必须打日志）。
    fn sender_set_video_encoder_backend(&self, track_id: &str, backend: crate::rtp::RTCVideoEncoderBackend) -> Result<(), RTCError> {
        tracing::warn!("sender_set_video_encoder_backend({track_id}, {backend:?}): not supported by backend");
        Err(RTCError::NotSupported("sender_set_video_encoder_backend".into()))
    }

    /// W3C RTCRtpSender.requestKeyFrame — 周期关键帧触发（PIT-76）。
    ///
    /// 默认实现: get → 全 encodings 设 request_key_frame=true → set（上层往返）。
    /// webrtc-sys 后端 override 为 cxx 保真往返（libwebrtc SetParameters 校验
    /// codecs/encodings 数量/transaction_id 与内部一致，上层映射有信息损失）。
    /// libwebrtc 每次消费后内部清标志：每次调用传 true 恰好触发一次，无需复位。
    fn request_key_frame(&self, track_id: &str) -> Result<(), RTCError> {
        let mut params = self.sender_get_parameters(track_id)?;
        for enc in &mut params.encodings {
            enc.request_key_frame = true;
        }
        self.sender_set_parameters(track_id, &params)
    }

    /// W3C RTCRtpSender.replaceTrack — v2 补
    fn sender_replace_track(&self, _track_id: &str, _new_track_id: &str) -> Result<(), RTCError> {
        tracing::warn!("sender_replace_track: not supported by backend");
        Err(RTCError::NotSupported("sender_replace_track".into()))
    }

    /// W3C RTCRtpSender.setStreams — v2 补
    fn sender_set_streams(&self, _track_id: &str, _stream_ids: &[String]) -> Result<(), RTCError> {
        tracing::warn!("sender_set_streams: not supported by backend");
        Err(RTCError::NotSupported("sender_set_streams".into()))
    }

    /// W3C RTCRtpSender.getStats（出站统计，可选）
    fn sender_get_stats(&self, _track_id: &str) -> Vec<RTCStats> {
        vec![]
    }

    /// W3C 静态 RTCRtpSender.getCapabilities(kind)
    fn get_sender_capabilities(&self, _kind: TrackKind) -> Result<Option<RTCRtpCapabilities>, RTCError> {
        tracing::warn!("get_sender_capabilities: not supported by backend");
        Err(RTCError::NotSupported("get_sender_capabilities".into()))
    }

    /// W3C 静态 RTCRtpReceiver.getCapabilities(kind)
    fn get_receiver_capabilities(&self, _kind: TrackKind) -> Result<Option<RTCRtpCapabilities>, RTCError> {
        tracing::warn!("get_receiver_capabilities: not supported by backend");
        Err(RTCError::NotSupported("get_receiver_capabilities".into()))
    }

    /// W3C restartIce — no-op 类默认 Ok + warn
    fn restart_ice(&self) -> Result<(), RTCError> {
        tracing::warn!("restart_ice: not supported by backend (no-op)");
        Ok(())
    }

    /// W3C getConfiguration
    fn pc_configuration(&self) -> RTCConfiguration {
        RTCConfiguration::default()
    }

    /// W3C setConfiguration
    fn set_configuration(&self, _config: &RTCConfiguration) -> Result<(), RTCError> {
        Ok(())
    }

    /// W3C currentLocalDescription
    fn current_local_description(&self) -> Result<Option<RTCSessionDescription>, RTCError> {
        tracing::warn!("current_local_description: not supported by backend");
        Err(RTCError::NotSupported("current_local_description".into()))
    }

    /// W3C currentRemoteDescription
    fn current_remote_description(&self) -> Result<Option<RTCSessionDescription>, RTCError> {
        tracing::warn!("current_remote_description: not supported by backend");
        Err(RTCError::NotSupported("current_remote_description".into()))
    }

    /// W3C RTCRtpTransceiver.setDirection — v2 补（经 mid 分派）
    fn transceiver_set_direction(&self, _mid: &str, _dir: RTCRtpTransceiverDirection) -> Result<(), RTCError> {
        tracing::warn!("transceiver_set_direction: not supported by backend");
        Err(RTCError::NotSupported("transceiver_set_direction".into()))
    }

    /// W3C RTCRtpTransceiver.stop — v2 补
    fn transceiver_stop(&self, _mid: &str) -> Result<(), RTCError> {
        tracing::warn!("transceiver_stop: not supported by backend");
        Err(RTCError::NotSupported("transceiver_stop".into()))
    }

    /// W3C RTCRtpTransceiver.setCodecPreferences — v2 补
    fn transceiver_set_codec_preferences(&self, _track_id: &str, _codecs: Vec<RTCRtpCodecCapability>) -> Result<(), RTCError> {
        tracing::warn!("transceiver_set_codec_preferences: not supported by backend");
        Err(RTCError::NotSupported("transceiver_set_codec_preferences".into()))
    }

    /// Register a local track with the RTCPeerConnection for RTP transmission.
    /// Backends that support track registration (webrtc-sys) call into
    /// libwebrtc to activate the track. Other backends store in the wrapper.
    fn register_track(
        &self, _track_id: &str, _kind: TrackKind,
    ) -> Result<(), RTCError> {
        Ok(())
    }

    /// Register callback for trickled local ICE candidates (P2P 需要完整转发).
    /// 默认空实现（webrtc-rs 后端如有需要自行实现）。
    fn set_on_ice_candidate(
        &self,
        _cb: Box<dyn Fn(RTCIceCandidate) + Send + Sync + 'static>,
    ) {
    }
    /// Register callback for ICE connection state changes (monitoring).
    fn set_on_ice_connection_state_change(
        &self,
        _cb: Box<dyn Fn(RTCIceConnectionState) + Send + Sync + 'static>,
    ) {
    }

    /// Register callback for peer connection state changes (monitoring).
    fn set_on_peer_connection_state_change(
        &self,
        _cb: Box<dyn Fn(RTCPeerConnectionState) + Send + Sync + 'static>,
    ) {
    }

    /// Return the local SDP string if available.
    fn local_description_sdp(&self) -> Option<String> {
        None
    }
}

pub(crate) trait DcBackend: Send + Sync + 'static {
    fn state(&self) -> RTCDataChannelState;
    async fn send(&self, data: &[u8]) -> Result<(), RTCError>;
    async fn send_text(&self, text: &str) -> Result<(), RTCError>;
    async fn spool(&self) -> RTCDataChannelRx;
    async fn close(&mut self);
}

pub trait TrackWriteBackend: Send + Sync + 'static {
    async fn write_frame(
        &self,
        data: &[u8],
        kind: TrackKind,
        audio_config: Option<&RTCAudioTrackConfig>,
    ) -> Result<(), RTCError>;

    /// Write a raw I420 (YUV 4:2:0 planar) frame to the video track.
    /// The backend handles encoding (webrtc-sys built-in x264) or no-ops (stub, webrtc-rs).
    /// For webrtc-rs: use mediaservo-codec crate to encode I420→H.264, then write_frame().
    ///
    /// `data` layout: Y plane (w*h) + U plane (w*h/4) + V plane (w*h/4).
    /// 默认方法委托 with_ts(None) — 单入口避免双实现漂移 (PIT-54 教训)。
    async fn write_raw_i420(
        &self, data: &[u8], width: u32, height: u32,
    ) -> Result<(), RTCError> {
        self.write_raw_i420_with_ts(data, width, height, None).await
    }

    /// PIT-63/相机: 时间戳参数化 — `ts_us` 为帧捕获时刻 (µs, wall-clock 语义)。
    /// None → 后端内部默认时间戳 (webrtc-sys: 锚定单调 wall-clock; webrtc-rs: pts=0)。
    /// 相机接入时传入 V4L2 buffer timestamp (µs)。
    async fn write_raw_i420_with_ts(
        &self, _data: &[u8], _width: u32, _height: u32, _ts_us: Option<i64>,
    ) -> Result<(), RTCError> {
        Ok(())
    }
}

// ── Mutual exclusion guard ──

#[cfg(all(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys"))]
compile_error!("Only one backend can be enabled at a time. Choose either backend-webrtc-rs or backend-webrtc-sys.");

// ── Module declarations ──

#[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]
pub(crate) mod stub;
#[cfg(feature = "backend-webrtc-rs")]
pub(crate) mod webrtc_rs;
#[cfg(feature = "backend-webrtc-sys")]
pub mod webrtc_sys;
// ── Type alias dispatch (compile-time, monomorphized) ──


#[cfg(feature = "backend-webrtc-rs")]
pub type ActivePc = webrtc_rs::WebrtcRsPc;
#[cfg(feature = "backend-webrtc-sys")]
pub type ActivePc = webrtc_sys::WebrtcSysPc;
#[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]
pub type ActivePc = stub::StubPc;

#[cfg(feature = "backend-webrtc-rs")]
pub(crate) type ActiveDc = webrtc_rs::WebrtcRsDc;
#[cfg(feature = "backend-webrtc-sys")]
pub(crate) type ActiveDc = webrtc_sys::WebrtcSysDc;
#[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]
pub(crate) type ActiveDc = stub::StubDc;

#[cfg(feature = "backend-webrtc-rs")]
pub type ActiveTrack = webrtc_rs::WebrtcRsTrack;
#[cfg(feature = "backend-webrtc-sys")]
pub type ActiveTrack = webrtc_sys::WebrtcSysTrack;
#[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]
pub type ActiveTrack = stub::StubTrack;

#[cfg(feature = "backend-webrtc-rs")]
pub(crate) type ActiveFactory = webrtc_rs::WebrtcRsFactory;
#[cfg(feature = "backend-webrtc-sys")]
pub type ActiveFactory = webrtc_sys::WebrtcSysFactory;
#[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]
pub(crate) type ActiveFactory = stub::StubFactory;
