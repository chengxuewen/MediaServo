//! Backend abstraction layer for multi-backend WebRTC support.
//!
//! Defines traits (PcBackend, DcBackend, TrackWriteBackend)
//! and compile-time type alias dispatch via cfg gates.
//! Zero dyn overhead — all dispatch is monomorphized.

use crate::RTCError;
use crate::data_channel::{RTCDataChannel, RTCDataChannelRx, RTCDataChannelState};
use crate::peer_connection::{
    RTCAnswerOptions, RTCConfiguration, RTCIceCandidate, RTCIceConnectionState,
    RTCIceGatheringState, RTCOfferOptions, RTCPeerConnectionState, RTCSignalingState,
};
use crate::rtp::{
    RTCRtpCapabilities, RTCRtpCodecCapability, RTCRtpParameters, RTCRtpTransceiver,
    RTCRtpTransceiverDirection, RTCRtpTransceiverInit,
};
use crate::sdp::RTCSessionDescription;
use crate::stats::RTCStats;
use crate::track::{RTCAudioTrackConfig, TrackKind, TrackReceiver, TrackSender};

// ── Traits ──

#[allow(dead_code)] // the W3C API mirror's default-method slot (kept for PIT-65 official alignment; unconsumed by the wrapper = C18)
pub(crate) trait PcBackend: Send + Sync + 'static {
    async fn create_offer(
        &self,
        options: &RTCOfferOptions,
    ) -> Result<RTCSessionDescription, RTCError>;
    async fn create_answer(
        &self,
        options: &RTCAnswerOptions,
    ) -> Result<RTCSessionDescription, RTCError>;
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
    /// connection-level stats (the candidate-pair rtt) -- only webrtc_sys implements; the rest None.
    fn connection_stats(&self) -> Option<crate::stats::ConnectionStats> {
        None
    }
    /// the legacy add_transceiver (str, str) -- renamed in v2, avoiding a same-name clash with the W3C signature (zero callers)
    #[allow(dead_code)]
    fn add_transceiver_legacy(&self, _media_type: &str, _direction: &str) -> Result<(), RTCError> {
        Err(RTCError::Internal("not supported".into()))
    }

    /// W3C getTransceivers -- returns a pure-data view of all transceivers (v2)
    /// default Err(NotSupported) + warn (C15 guards against silent degradation); stub overrides to Ok(vec![])
    fn get_transceivers(&self) -> Result<Vec<RTCRtpTransceiver>, RTCError> {
        tracing::warn!("get_transceivers: not supported by backend");
        Err(RTCError::NotSupported("get_transceivers".into()))
    }

    /// W3C addTransceiver(kind, init) -- the kind variant (sync, v2)
    fn add_transceiver(
        &self,
        _kind: TrackKind,
        _init: &RTCRtpTransceiverInit,
    ) -> Result<RTCRtpTransceiver, RTCError> {
        tracing::warn!("add_transceiver: not supported by backend");
        Err(RTCError::NotSupported("add_transceiver".into()))
    }

    /// W3C addTransceiver(track, init) -- the track-variant overload (v2, the P3 core: the kind variant produces an empty track that cannot receive frames)
    fn add_transceiver_with_track(
        &self,
        _track: &TrackSender,
        _init: &RTCRtpTransceiverInit,
    ) -> Result<RTCRtpTransceiver, RTCError> {
        tracing::warn!("add_transceiver_with_track: not supported by backend");
        Err(RTCError::NotSupported("add_transceiver_with_track".into()))
    }

    /// W3C RTCRtpSender.getParameters -- dispatched via track_id (sync)
    fn sender_get_parameters(&self, _track_id: &str) -> Result<RTCRtpParameters, RTCError> {
        tracing::warn!("sender_get_parameters: not supported by backend");
        Err(RTCError::NotSupported("sender_get_parameters".into()))
    }

    /// W3C RTCRtpReceiver.getParameters -- dispatched via track_id (sync)
    fn receiver_get_parameters(&self, _track_id: &str) -> Result<RTCRtpParameters, RTCError> {
        tracing::warn!("receiver_get_parameters: not supported by backend");
        Err(RTCError::NotSupported("receiver_get_parameters".into()))
    }

    /// W3C RTCRtpSender.setParameters -- added in v2
    fn sender_set_parameters(
        &self,
        _track_id: &str,
        _params: &RTCRtpParameters,
    ) -> Result<(), RTCError> {
        tracing::warn!("sender_set_parameters: not supported by backend");
        Err(RTCError::NotSupported("sender_set_parameters".into()))
    }

    /// v2 (encoder-bitrate): set the sending encoder's min/max bitrate (bps).
    /// default NotSupported + warn (C15); the webrtc-sys backend overrides as a cxx-faithful round trip.
    fn sender_set_encoding_bitrate(
        &self,
        _track_id: &str,
        _min_bps: Option<u64>,
        _max_bps: Option<u64>,
    ) -> Result<(), RTCError> {
        tracing::warn!("sender_set_encoding_bitrate: not supported by backend");
        Err(RTCError::NotSupported("sender_set_encoding_bitrate".into()))
    }

    /// v2 (multi-stream P1): set the sending encoder's framerate cap (fps).
    /// default NotSupported + warn (C15); the webrtc-sys backend overrides (copying the bitrate pattern,
    /// get_parameters → enc[].max_framerate → set_parameters）。
    fn sender_set_encoding_framerate(
        &self,
        _track_id: &str,
        _max_fps: Option<f64>,
    ) -> Result<(), RTCError> {
        tracing::warn!("sender_set_encoding_framerate: not supported by backend");
        Err(RTCError::NotSupported("sender_set_encoding_framerate".into()))
    }

    /// v2 (encoder-backend-codec-config T1): set the sending encoder backend (the soft/hard implementation choice).
    /// dispatched via track_id (matched on sender.track().id(), the same pattern as request_key_frame).
    /// default NotSupported + warn (C15: error branches must log).
    fn sender_set_video_encoder_backend(
        &self,
        track_id: &str,
        backend: crate::rtp::RTCVideoEncoderBackend,
    ) -> Result<(), RTCError> {
        tracing::warn!(
            "sender_set_video_encoder_backend({track_id}, {backend:?}): not supported by backend"
        );
        Err(RTCError::NotSupported("sender_set_video_encoder_backend".into()))
    }

    /// v2 (qos-framerate-priority): set the sender-side degradation preference (at the RtpParameters level).
    /// default NotSupported + warn (C15); the webrtc-sys backend overrides as a cxx-faithful round trip.
    fn sender_set_degradation_preference(
        &self,
        track_id: &str,
        pref: crate::rtp::RTCDegradationPreference,
    ) -> Result<(), RTCError> {
        tracing::warn!(
            "sender_set_degradation_preference({track_id}, {pref:?}): not supported by backend"
        );
        Err(RTCError::NotSupported("sender_set_degradation_preference".into()))
    }

    /// v2 (qos-framerate-priority): set the sender-side video track's content hint.
    /// default NotSupported + warn (C15); the webrtc-sys backend overrides (media_to_video -> set_content_hint).
    fn sender_set_content_hint(
        &self,
        track_id: &str,
        hint: crate::rtp::RTCRtpContentHint,
    ) -> Result<(), RTCError> {
        tracing::warn!("sender_set_content_hint({track_id}, {hint:?}): not supported by backend");
        Err(RTCError::NotSupported("sender_set_content_hint".into()))
    }

    /// W3C RTCRtpSender.requestKeyFrame -- periodic keyframe triggering (PIT-76).
    ///
    /// default impl: get -> set request_key_frame=true on all encodings -> set (an upper-layer round trip).
    /// the webrtc-sys backend overrides as a cxx-faithful round trip (libwebrtc's SetParameters validation
    /// of codecs/encodings counts/transaction_id against its internal state; the upper mapping loses information).
    /// libwebrtc clears the flag internally after each consumption: passing true per call triggers exactly one, no reset needed.
    fn request_key_frame(&self, track_id: &str) -> Result<(), RTCError> {
        let mut params = self.sender_get_parameters(track_id)?;
        for enc in &mut params.encodings {
            enc.request_key_frame = true;
        }
        self.sender_set_parameters(track_id, &params)
    }

    /// W3C RTCRtpSender.replaceTrack -- added in v2
    fn sender_replace_track(&self, _track_id: &str, _new_track_id: &str) -> Result<(), RTCError> {
        tracing::warn!("sender_replace_track: not supported by backend");
        Err(RTCError::NotSupported("sender_replace_track".into()))
    }

    /// W3C RTCRtpSender.setStreams -- added in v2
    fn sender_set_streams(&self, _track_id: &str, _stream_ids: &[String]) -> Result<(), RTCError> {
        tracing::warn!("sender_set_streams: not supported by backend");
        Err(RTCError::NotSupported("sender_set_streams".into()))
    }

    /// W3C RTCRtpSender.getStats (outbound stats, optional)
    /// S2c: the receive-side stats (the packetsReceived/framesDecoded bisection); an empty default surface = zero cost for stub/webrtc-rs.
    fn receiver_get_stats(&self, _track_id: &str) -> Vec<RTCStats> {
        Vec::new()
    }

    fn sender_get_stats(&self, _track_id: &str) -> Vec<RTCStats> {
        vec![]
    }

    /// W3C static RTCRtpSender.getCapabilities(kind)
    fn get_sender_capabilities(
        &self,
        _kind: TrackKind,
    ) -> Result<Option<RTCRtpCapabilities>, RTCError> {
        tracing::warn!("get_sender_capabilities: not supported by backend");
        Err(RTCError::NotSupported("get_sender_capabilities".into()))
    }

    /// W3C static RTCRtpReceiver.getCapabilities(kind)
    fn get_receiver_capabilities(
        &self,
        _kind: TrackKind,
    ) -> Result<Option<RTCRtpCapabilities>, RTCError> {
        tracing::warn!("get_receiver_capabilities: not supported by backend");
        Err(RTCError::NotSupported("get_receiver_capabilities".into()))
    }

    /// W3C restartIce -- a no-op-class default Ok + warn
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

    /// W3C RTCRtpTransceiver.setDirection -- added in v2 (dispatched via mid)
    fn transceiver_set_direction(
        &self,
        _mid: &str,
        _dir: RTCRtpTransceiverDirection,
    ) -> Result<(), RTCError> {
        tracing::warn!("transceiver_set_direction: not supported by backend");
        Err(RTCError::NotSupported("transceiver_set_direction".into()))
    }

    /// W3C RTCRtpTransceiver.stop -- added in v2
    fn transceiver_stop(&self, _mid: &str) -> Result<(), RTCError> {
        tracing::warn!("transceiver_stop: not supported by backend");
        Err(RTCError::NotSupported("transceiver_stop".into()))
    }

    /// W3C RTCRtpTransceiver.setCodecPreferences -- added in v2
    fn transceiver_set_codec_preferences(
        &self,
        _track_id: &str,
        _codecs: Vec<RTCRtpCodecCapability>,
    ) -> Result<(), RTCError> {
        tracing::warn!("transceiver_set_codec_preferences: not supported by backend");
        Err(RTCError::NotSupported("transceiver_set_codec_preferences".into()))
    }

    /// Register a local track with the RTCPeerConnection for RTP transmission.
    /// Backends that support track registration (webrtc-sys) call into
    /// libwebrtc to activate the track. Other backends store in the wrapper.
    fn register_track(&self, _track_id: &str, _kind: TrackKind) -> Result<(), RTCError> {
        Ok(())
    }

    /// Register callback for trickled local ICE candidates (P2P needs full forwarding).
    /// default empty impl (the webrtc-rs backend implements it itself if needed).
    fn set_on_ice_candidate(&self, _cb: Box<dyn Fn(RTCIceCandidate) + Send + Sync + 'static>) {}
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
    /// S6/K5: the outbound queue byte count (observable backpressure; the W3C bufferedAmount semantics).
    async fn buffered_amount(&self) -> u64;
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
    /// the default method delegates to with_ts(None) -- a single entry avoids dual-implementation drift (the PIT-54 lesson).
    async fn write_raw_i420(&self, data: &[u8], width: u32, height: u32) -> Result<(), RTCError> {
        self.write_raw_i420_with_ts(data, width, height, None).await
    }

    /// PIT-63/camera: timestamp parameterization -- `ts_us` is the frame capture moment (us, wall-clock semantics).
    /// None -> the backend's internal default timestamp (webrtc-sys: anchored monotonic wall-clock; webrtc-rs: pts=0).
    /// camera integration passes the V4L2 buffer timestamp (us).
    async fn write_raw_i420_with_ts(
        &self,
        _data: &[u8],
        _width: u32,
        _height: u32,
        _ts_us: Option<i64>,
    ) -> Result<(), RTCError> {
        Ok(())
    }
}

// ── Mutual exclusion guard ──

#[cfg(all(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys"))]
compile_error!(
    "Only one backend can be enabled at a time. Choose either backend-webrtc-rs or backend-webrtc-sys."
);

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
