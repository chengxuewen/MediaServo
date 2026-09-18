//! Stub backend — all operations are no-ops or return defaults.
//! Used when no WebRTC backend feature is enabled (compilation-only mode).

use super::DcBackend;
use super::PcBackend;
use super::TrackWriteBackend;
use crate::RTCError;
use crate::data_channel::{
    RTCDataChannel, RTCDataChannelInit, RTCDataChannelRx, RTCDataChannelState,
};
use crate::peer_connection::{
    RTCAnswerOptions, RTCConfiguration, RTCIceCandidate, RTCIceConnectionState,
    RTCIceGatheringState, RTCOfferOptions, RTCPeerConnectionState, RTCSignalingState,
};
use crate::sdp::{RTCSdpType, RTCSessionDescription};
use crate::track::{RTCAudioTrackConfig, TrackKind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ── StubPc ──

#[derive(Debug, Default)]
pub(crate) struct StubPc {
    closed: AtomicBool,
    /// v2: 状态化 transceivers（T2 测试需 add 后 get 非空）
    transceivers: std::sync::Mutex<Vec<crate::rtp::RTCRtpTransceiver>>,
}

impl PcBackend for StubPc {
    async fn create_offer(&self, _: &RTCOfferOptions) -> Result<RTCSessionDescription, RTCError> {
        Ok(RTCSessionDescription::new(RTCSdpType::Offer, String::new()))
    }

    async fn create_answer(&self, _: &RTCAnswerOptions) -> Result<RTCSessionDescription, RTCError> {
        Ok(RTCSessionDescription::new(RTCSdpType::Answer, String::new()))
    }

    async fn set_local_description(&self, _: &RTCSessionDescription) -> Result<(), RTCError> {
        Ok(())
    }

    async fn set_remote_description(&self, _: &RTCSessionDescription) -> Result<(), RTCError> {
        Ok(())
    }

    async fn add_ice_candidate(&self, _: &RTCIceCandidate) -> Result<(), RTCError> {
        Ok(())
    }

    fn connection_state(&self) -> RTCPeerConnectionState {
        if self.closed.load(Ordering::Relaxed) {
            RTCPeerConnectionState::Closed
        } else {
            RTCPeerConnectionState::New
        }
    }

    fn ice_connection_state(&self) -> RTCIceConnectionState {
        if self.closed.load(Ordering::Relaxed) {
            RTCIceConnectionState::Closed
        } else {
            RTCIceConnectionState::New
        }
    }

    fn ice_gathering_state(&self) -> RTCIceGatheringState {
        RTCIceGatheringState::New
    }

    fn signaling_state(&self) -> RTCSignalingState {
        if self.closed.load(Ordering::Relaxed) {
            RTCSignalingState::Closed
        } else {
            RTCSignalingState::Stable
        }
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    // ── v2: W3C API stub overrides（状态化）──
    fn get_transceivers(&self) -> Result<Vec<crate::rtp::RTCRtpTransceiver>, crate::RTCError> {
        Ok(self.transceivers.lock().unwrap().clone())
    }

    fn add_transceiver(
        &self,
        kind: TrackKind,
        init: &crate::rtp::RTCRtpTransceiverInit,
    ) -> Result<crate::rtp::RTCRtpTransceiver, crate::RTCError> {
        let sender = crate::rtp::RTCRtpSender::new(crate::track::TrackRef::Sender(
            crate::track::TrackSender::new("stub-".to_string() + kind.as_str(), kind),
        ));
        let receiver = crate::rtp::RTCRtpReceiver::new(crate::track::TrackRef::Receiver(
            crate::track::TrackReceiver::new("stub-r-".to_string() + kind.as_str(), kind),
        ));
        let tc = crate::rtp::RTCRtpTransceiver::new(
            Some("0".into()),
            init.direction,
            Some(init.direction),
            false,
            sender,
            receiver,
            kind,
        );
        self.transceivers.lock().unwrap().push(tc.clone());
        Ok(tc)
    }

    fn add_transceiver_with_track(
        &self,
        track: &crate::track::TrackSender,
        init: &crate::rtp::RTCRtpTransceiverInit,
    ) -> Result<crate::rtp::RTCRtpTransceiver, crate::RTCError> {
        let sender = crate::rtp::RTCRtpSender::new(crate::track::TrackRef::Sender(track.clone()));
        let receiver = crate::rtp::RTCRtpReceiver::new(crate::track::TrackRef::Receiver(
            crate::track::TrackReceiver::new(
                "stub-r-".to_string() + track.kind.as_str(),
                track.kind,
            ),
        ));
        let tc = crate::rtp::RTCRtpTransceiver::new(
            Some("0".into()),
            init.direction,
            Some(init.direction),
            false,
            sender,
            receiver,
            track.kind,
        );
        self.transceivers.lock().unwrap().push(tc.clone());
        Ok(tc)
    }

    fn sender_get_parameters(
        &self,
        _track_id: &str,
    ) -> Result<crate::rtp::RTCRtpParameters, crate::RTCError> {
        Ok(crate::rtp::RTCRtpParameters::default())
    }

    fn receiver_get_parameters(
        &self,
        _track_id: &str,
    ) -> Result<crate::rtp::RTCRtpParameters, crate::RTCError> {
        Ok(crate::rtp::RTCRtpParameters::default())
    }

    fn sender_set_parameters(
        &self,
        _track_id: &str,
        _params: &crate::rtp::RTCRtpParameters,
    ) -> Result<(), crate::RTCError> {
        Ok(())
    }

    fn sender_set_video_encoder_backend(
        &self,
        _track_id: &str,
        _backend: crate::rtp::RTCVideoEncoderBackend,
    ) -> Result<(), crate::RTCError> {
        Ok(())
    }

    fn sender_replace_track(
        &self,
        _track_id: &str,
        _new_track_id: &str,
    ) -> Result<(), crate::RTCError> {
        Ok(())
    }

    fn sender_set_streams(
        &self,
        _track_id: &str,
        _stream_ids: &[String],
    ) -> Result<(), crate::RTCError> {
        Ok(())
    }

    fn get_sender_capabilities(
        &self,
        _kind: TrackKind,
    ) -> Result<Option<crate::rtp::RTCRtpCapabilities>, crate::RTCError> {
        Ok(None)
    }

    fn get_receiver_capabilities(
        &self,
        _kind: TrackKind,
    ) -> Result<Option<crate::rtp::RTCRtpCapabilities>, crate::RTCError> {
        Ok(None)
    }

    fn restart_ice(&self) -> Result<(), crate::RTCError> {
        Ok(())
    }

    fn current_local_description(
        &self,
    ) -> Result<Option<crate::sdp::RTCSessionDescription>, crate::RTCError> {
        Ok(None)
    }

    fn current_remote_description(
        &self,
    ) -> Result<Option<crate::sdp::RTCSessionDescription>, crate::RTCError> {
        Ok(None)
    }

    fn transceiver_set_direction(
        &self,
        _mid: &str,
        _dir: crate::rtp::RTCRtpTransceiverDirection,
    ) -> Result<(), crate::RTCError> {
        Ok(())
    }

    fn transceiver_stop(&self, _mid: &str) -> Result<(), crate::RTCError> {
        Ok(())
    }

    fn transceiver_set_codec_preferences(
        &self,
        _track_id: &str,
        _codecs: Vec<crate::rtp::RTCRtpCodecCapability>,
    ) -> Result<(), crate::RTCError> {
        Ok(())
    }
}

// ponytail: manual Clone for AtomicBool-backed struct
impl Clone for StubPc {
    fn clone(&self) -> Self {
        Self {
            closed: AtomicBool::new(self.closed.load(Ordering::Relaxed)),
            transceivers: std::sync::Mutex::new(self.transceivers.lock().unwrap().clone()),
        }
    }
}

#[cfg(not(feature = "backend-webrtc-rs"))]
impl StubPc {
    pub(crate) async fn create_data_channel(
        &self,
        label: &str,
        _init: RTCDataChannelInit,
    ) -> Result<RTCDataChannel, RTCError> {
        Ok(RTCDataChannel { label: label.to_string(), id: 0, backend: StubDc })
    }
}

// ── StubDc ──

#[derive(Debug, Default, Clone)]
pub(crate) struct StubDc;

impl DcBackend for StubDc {
    fn state(&self) -> RTCDataChannelState {
        RTCDataChannelState::Closed
    }

    async fn send(&self, _: &[u8]) -> Result<(), RTCError> {
        Ok(())
    }

    async fn send_text(&self, _: &str) -> Result<(), RTCError> {
        Ok(())
    }

    async fn spool(&self) -> RTCDataChannelRx {
        RTCDataChannelRx::stub()
    }

    async fn close(&mut self) {}
}

// ── StubTrack ──

/// Stub 轨道后端 — 写帧 no-op，但记录 I420 写入供测试观测（T2 透传断言）。
#[derive(Debug, Clone, Default)]
pub(crate) struct StubTrack {
    /// 已写 I420 帧数（write_raw_i420_with_ts 计数）。
    frames_written: Arc<AtomicU64>,
    /// 已写帧时间戳序列（C17 透传断言）。
    ts_history: Arc<Mutex<Vec<i64>>>,
}

#[cfg(test)]
impl StubTrack {
    pub(crate) fn frames_written(&self) -> u64 {
        self.frames_written.load(Ordering::Relaxed)
    }

    pub(crate) fn ts_history(&self) -> Vec<i64> {
        self.ts_history.lock().unwrap().clone()
    }
}

impl TrackWriteBackend for StubTrack {
    async fn write_frame(
        &self,
        data: &[u8],
        _kind: TrackKind,
        _audio_config: Option<&RTCAudioTrackConfig>,
    ) -> Result<(), RTCError> {
        tracing::debug!("TrackSender::write_frame (stub): {} bytes", data.len());
        Ok(())
    }

    /// 记录 I420 写入（帧数 + 时间戳），测试断言透传/单调性。
    async fn write_raw_i420_with_ts(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        ts_us: Option<i64>,
    ) -> Result<(), RTCError> {
        self.frames_written.fetch_add(1, Ordering::Relaxed);
        if let Some(ts) = ts_us {
            self.ts_history.lock().unwrap().push(ts);
        }
        tracing::debug!(
            "TrackSender::write_raw_i420_with_ts (stub): {width}x{height} {} bytes ts={ts_us:?}",
            data.len(),
        );
        Ok(())
    }
}

// ── StubFactory ──

#[derive(Debug, Default, Clone)]
pub(crate) struct StubFactory;

impl StubFactory {
    pub(crate) async fn create_peer_connection(
        &self,
        _config: RTCConfiguration,
    ) -> Result<StubPc, RTCError> {
        tracing::info!("Creating RTCPeerConnection (stub)");
        Ok(StubPc::default())
    }

    /// Create a stub video track — no-op.
    pub(crate) fn create_video_track(&self) -> (StubTrack, ()) {
        (StubTrack::default(), ())
    }
}

// ── T2: W3C PcBackend stub 状态化测试（v2）──
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtp::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};

    fn new_pc() -> StubPc {
        StubPc::default()
    }

    #[test]
    fn add_transceiver_returns_transceiver() {
        let pc = new_pc();
        let init = RTCRtpTransceiverInit {
            direction: RTCRtpTransceiverDirection::Sendonly,
            send_encodings: vec![],
            stream_ids: vec![],
        };
        let tc = pc.add_transceiver(TrackKind::Video, &init).unwrap();
        assert_eq!(tc.kind, TrackKind::Video);
        assert_eq!(tc.direction, RTCRtpTransceiverDirection::Sendonly);
        assert!(!tc.stopped);
    }

    #[test]
    fn get_transceivers_reflects_added() {
        let pc = new_pc();
        let init = RTCRtpTransceiverInit::default();
        pc.add_transceiver(TrackKind::Video, &init).unwrap();
        let tcs = pc.get_transceivers().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].kind, TrackKind::Video);
    }

    #[test]
    fn sender_get_parameters_roundtrip() {
        let pc = new_pc();
        let params = pc.sender_get_parameters("t1").unwrap();
        assert!(params.codecs.is_empty()); // stub 默认
    }

    #[test]
    fn receiver_get_parameters_roundtrip() {
        let pc = new_pc();
        let params = pc.receiver_get_parameters("t1").unwrap();
        assert!(params.codecs.is_empty()); // stub 默认
    }

    #[test]
    fn get_sender_capabilities_video() {
        let pc = new_pc();
        let caps = pc.get_sender_capabilities(TrackKind::Video).unwrap();
        assert!(caps.is_none()); // stub 允许 None
    }

    #[test]
    fn restart_ice_noop() {
        let pc = new_pc();
        pc.restart_ice().unwrap();
    }

    #[test]
    fn configuration_roundtrip() {
        let pc = new_pc();
        let cfg = pc.pc_configuration();
        assert!(cfg.ice_servers.is_empty());
        pc.set_configuration(&cfg).unwrap();
    }

    #[test]
    fn current_descriptions_none_before_set() {
        let pc = new_pc();
        assert!(pc.current_local_description().unwrap().is_none());
        assert!(pc.current_remote_description().unwrap().is_none());
    }

    #[test]
    fn sender_object_methods_roundtrip() {
        let pc = new_pc();
        let params = crate::rtp::RTCRtpParameters::default();
        pc.sender_set_parameters("t1", &params).unwrap();
        pc.sender_replace_track("t1", "t2").unwrap();
        pc.sender_set_streams("t1", &[]).unwrap();
    }

    #[test]
    fn transceiver_set_direction_stop() {
        let pc = new_pc();
        pc.transceiver_set_direction("0", RTCRtpTransceiverDirection::Recvonly).unwrap();
        pc.transceiver_stop("0").unwrap();
    }
}
