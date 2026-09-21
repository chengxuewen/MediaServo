//! webrtc-rs backend — wraps the `webrtc` crate types.
//! Enabled via `backend-webrtc-rs` feature.

use std::sync::{Arc, Mutex};

use super::DcBackend;
use super::PcBackend;
use super::TrackWriteBackend;
use crate::RTCError;
use crate::data_channel::{
    RTCDataChannel as PubDataChannel, RTCDataChannelEvent, RTCDataChannelInit, RTCDataChannelRx,
    RTCDataChannelState, RTCDataMessage,
};
use crate::peer_connection::{
    RTCAnswerOptions, RTCIceCandidate, RTCIceConnectionState, RTCIceGatheringState,
    RTCOfferOptions, RTCPeerConnectionState, RTCSignalingState,
};
use crate::sdp::{RTCSdpType, RTCSessionDescription};
use crate::track::{RTCAudioTrackConfig, TrackKind, TrackReceiver};
use mediaservo_codec::{
    Bitrate, CodecFactory, CodecId, EncoderConfig, EncoderPreset, PixelFormat, Plane, VideoEncoder,
    VideoFormat, VideoFrame,
};

// ── WebrtcRsPc ──

#[derive(Clone)]
pub(crate) struct WebrtcRsPc {
    inner: Arc<webrtc::peer_connection::RTCPeerConnection>,
    on_track_cb:
        Arc<Mutex<Option<Box<dyn Fn(crate::track::TrackReceiver) + Send + Sync + 'static>>>>,
}

impl std::fmt::Debug for WebrtcRsPc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcRsPc")
            .field("connection_state", &self.inner.connection_state())
            .finish()
    }
}

impl WebrtcRsPc {
    pub(crate) fn new(inner: Arc<webrtc::peer_connection::RTCPeerConnection>) -> Self {
        Self { inner, on_track_cb: Arc::new(Mutex::new(None)) }
    }

    /// Set on_data_channel callback. Caller should use PubDataChannel's
    /// constructor to wrap the raw RTCDataChannel.
    pub(crate) fn on_data_channel(
        &self,
        f: Box<
            dyn FnMut(
                    Arc<webrtc::data_channel::RTCDataChannel>,
                )
                    -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>
                + Send
                + Sync
                + 'static,
        >,
    ) {
        self.inner.on_data_channel(f);
    }

    /// Set on_ice_candidate callback. None means gathering complete.
    pub(crate) fn on_ice_candidate(
        &self,
        f: Box<
            dyn FnMut(
                    Option<webrtc::ice_transport::ice_candidate::RTCIceCandidate>,
                )
                    -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>
                + Send
                + Sync
                + 'static,
        >,
    ) {
        self.inner.on_ice_candidate(f);
    }

    /// Set on_ice_connection_state_change callback for ICE monitoring.
    pub(crate) fn on_ice_connection_state_change(
        &self,
        f: Box<
            dyn FnMut(
                    webrtc::ice_transport::ice_connection_state::RTCIceConnectionState,
                )
                    -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>
                + Send
                + Sync
                + 'static,
        >,
    ) {
        self.inner.on_ice_connection_state_change(f);
    }

    pub(crate) async fn create_data_channel(
        &self,
        label: &str,
        _init: RTCDataChannelInit,
    ) -> Result<PubDataChannel, RTCError> {
        let dc = self
            .inner
            .create_data_channel(label, None)
            .await
            .map_err(|e| RTCError::RTCDataChannel(e.to_string()))?;
        let id = dc.id() as i32;
        Ok(PubDataChannel { label: label.to_string(), id, backend: WebrtcRsDc { inner: dc } })
    }
}

impl PcBackend for WebrtcRsPc {
    async fn create_offer(
        &self,
        options: &RTCOfferOptions,
    ) -> Result<RTCSessionDescription, RTCError> {
        let mut opts = webrtc::peer_connection::offer_answer_options::RTCOfferOptions::default();
        if options.ice_restart {
            opts.ice_restart = true;
        }
        let sdp = self.inner.create_offer(Some(opts)).await?;
        Ok(RTCSessionDescription { sdp_type: RTCSdpType::Offer, sdp: sdp.sdp })
    }

    async fn create_answer(&self, _: &RTCAnswerOptions) -> Result<RTCSessionDescription, RTCError> {
        let sdp = self.inner.create_answer(None).await?;
        Ok(RTCSessionDescription { sdp_type: RTCSdpType::Answer, sdp: sdp.sdp })
    }

    async fn set_local_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError> {
        let sdp = match desc.sdp_type {
            RTCSdpType::Offer => {
                webrtc::peer_connection::sdp::session_description::RTCSessionDescription::offer(
                    desc.sdp.clone(),
                )?
            }
            RTCSdpType::Answer => {
                webrtc::peer_connection::sdp::session_description::RTCSessionDescription::answer(
                    desc.sdp.clone(),
                )?
            }
            _ => return Err(RTCError::Sdp("unsupported SDP type".into())),
        };
        self.inner.set_local_description(sdp).await?;
        Ok(())
    }

    async fn set_remote_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError> {
        let sdp = match desc.sdp_type {
            RTCSdpType::Offer => {
                webrtc::peer_connection::sdp::session_description::RTCSessionDescription::offer(
                    desc.sdp.clone(),
                )?
            }
            RTCSdpType::Answer => {
                webrtc::peer_connection::sdp::session_description::RTCSessionDescription::answer(
                    desc.sdp.clone(),
                )?
            }
            _ => return Err(RTCError::Sdp("unsupported SDP type".into())),
        };
        self.inner.set_remote_description(sdp).await?;
        Ok(())
    }

    async fn add_ice_candidate(&self, candidate: &RTCIceCandidate) -> Result<(), RTCError> {
        let c = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
            candidate: candidate.candidate.clone(),
            sdp_mid: candidate.sdp_mid.clone(),
            sdp_mline_index: candidate.sdp_mline_index,
            ..Default::default()
        };
        self.inner.add_ice_candidate(c).await?;
        Ok(())
    }

    fn connection_state(&self) -> RTCPeerConnectionState {
        use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::*;
        match self.inner.connection_state() {
            New => RTCPeerConnectionState::New,
            Connecting => RTCPeerConnectionState::Connecting,
            Connected => RTCPeerConnectionState::Connected,
            Disconnected => RTCPeerConnectionState::Disconnected,
            Failed => RTCPeerConnectionState::Failed,
            Closed => RTCPeerConnectionState::Closed,
            _ => RTCPeerConnectionState::New,
        }
    }

    fn ice_connection_state(&self) -> RTCIceConnectionState {
        use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::*;
        match self.inner.ice_connection_state() {
            New => RTCIceConnectionState::New,
            Checking => RTCIceConnectionState::Checking,
            Connected => RTCIceConnectionState::Connected,
            Completed => RTCIceConnectionState::Completed,
            Failed => RTCIceConnectionState::Failed,
            Disconnected => RTCIceConnectionState::Disconnected,
            Closed => RTCIceConnectionState::Closed,
            _ => RTCIceConnectionState::New,
        }
    }

    fn ice_gathering_state(&self) -> RTCIceGatheringState {
        use webrtc::ice_transport::ice_gathering_state::RTCIceGatheringState::*;
        match self.inner.ice_gathering_state() {
            Unspecified | New => RTCIceGatheringState::New,
            Gathering => RTCIceGatheringState::Gathering,
            Complete => RTCIceGatheringState::Complete,
        }
    }

    fn signaling_state(&self) -> RTCSignalingState {
        use webrtc::peer_connection::signaling_state::RTCSignalingState::*;
        match self.inner.signaling_state() {
            Stable => RTCSignalingState::Stable,
            HaveLocalOffer => RTCSignalingState::HaveLocalOffer,
            Closed => RTCSignalingState::Closed,
            _ => RTCSignalingState::Stable,
        }
    }

    async fn close(&self) {
        let _ = self.inner.close().await;
    }

    /// Bridge webrtc-rs on_track → crate TrackReceiver callback.
    fn set_on_track(&self, cb: Box<dyn Fn(crate::track::TrackReceiver) + Send + Sync + 'static>) {
        *self.on_track_cb.lock().unwrap() = Some(cb);
        let cb_clone = Arc::clone(&self.on_track_cb);
        self.inner.on_track(Box::new(move |track, _, _| {
            let kind = match track.kind() {
                webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio => TrackKind::Audio,
                webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video => TrackKind::Video,
                // ponytail: default to Video for Unspecified codec type
                _ => TrackKind::Video,
            };
            let receiver = TrackReceiver::new(track.id(), kind);
            if let Some(ref cb) = *cb_clone.lock().unwrap() {
                cb(receiver.clone());
            }
            // P1: spawn decode pipeline for incoming video tracks
            if matches!(kind, TrackKind::Video) {
                let receiver2 = receiver;
                let track2 = track.clone();
                tokio::spawn(async move {
                    run_decode_pipeline(track2, receiver2).await;
                });
            }
            Box::pin(async {})
        }));
    }

    fn set_on_ice_connection_state_change(
        &self,
        cb: Box<dyn Fn(RTCIceConnectionState) + Send + Sync + 'static>,
    ) {
        let cb = std::sync::Mutex::new(Some(cb));
        self.inner.on_ice_connection_state_change(Box::new(move |state: webrtc::ice_transport::ice_connection_state::RTCIceConnectionState| {
            let mapped = match state {
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::New => RTCIceConnectionState::New,
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Checking => RTCIceConnectionState::Checking,
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Connected => RTCIceConnectionState::Connected,
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Completed => RTCIceConnectionState::Completed,
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Failed => RTCIceConnectionState::Failed,
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Disconnected => RTCIceConnectionState::Disconnected,
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Closed => RTCIceConnectionState::Closed,
                _ => RTCIceConnectionState::New,
            };
            if let Some(ref f) = *cb.lock().unwrap() { f(mapped); }
            Box::pin(async {})
        }));
    }

    fn set_on_peer_connection_state_change(
        &self,
        cb: Box<dyn Fn(RTCPeerConnectionState) + Send + Sync + 'static>,
    ) {
        let cb = std::sync::Mutex::new(Some(cb));
        self.inner.on_peer_connection_state_change(Box::new(move |state: webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState| {
            let mapped = match state {
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::New => RTCPeerConnectionState::New,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Connecting => RTCPeerConnectionState::Connecting,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Connected => RTCPeerConnectionState::Connected,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Disconnected => RTCPeerConnectionState::Disconnected,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Failed => RTCPeerConnectionState::Failed,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed => RTCPeerConnectionState::Closed,
                _ => RTCPeerConnectionState::New,
            };
            if let Some(ref f) = *cb.lock().unwrap() { f(mapped); }
            Box::pin(async {})
        }));
    }
}

// ── Video decode pipeline helpers for webrtc-rs on_track ──

/// ponytail: H.264 FU-A reassembly + decode loop.
/// Polls `track.read_rtp()` in a loop, reassembles fragmented NAL units,
/// decodes H.264 → I420, and delivers frames to `receiver.sink`.
async fn run_decode_pipeline(
    track: Arc<webrtc::track::track_remote::TrackRemote>,
    receiver: TrackReceiver,
) {
    use mediaservo_codec::{CodecFactory, CodecId, DecoderConfig};

    let factory = CodecFactory::new();
    let decoder_config = DecoderConfig { codec: CodecId::H264 };
    let mut decoder = match factory.create_decoder(decoder_config.clone(), None) {
        Ok(mut d) => {
            if d.configure(&decoder_config).is_err() {
                return;
            }
            d
        }
        Err(_) => return,
    };

    let mut au_buffer: Vec<u8> = Vec::new();
    loop {
        let (pkt, _attrs) = match track.read_rtp().await {
            Ok(v) => v,
            Err(_) => break, // track closed or error
        };
        let payload = pkt.payload.to_vec();
        if payload.len() < 2 {
            continue;
        }
        let nal_type = payload[0] & 0x1F;
        if nal_type == 28 {
            // FU-A fragment
            let is_start = (payload[1] & 0x80) != 0;
            let is_end = (payload[1] & 0x40) != 0;
            if is_start {
                au_buffer.clear();
                // Annex B start code + reconstructed NAL header
                au_buffer.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                let nal_ref = payload[0] & 0x60;
                let fu_type = payload[1] & 0x1F;
                au_buffer.push(nal_ref | fu_type);
            }
            au_buffer.extend_from_slice(&payload[2..]);
            if is_end && !au_buffer.is_empty() {
                feed_and_drain(&mut decoder, &receiver, &au_buffer);
            }
        } else {
            // Single NAL unit — wrap in Annex B start code
            let mut single = Vec::with_capacity(4 + payload.len());
            single.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            single.extend_from_slice(&payload);
            feed_and_drain(&mut decoder, &receiver, &single);
        }
    }
}

/// Push access unit to decoder and drain decoded frames to FrameSink.
fn feed_and_drain(
    decoder: &mut Box<dyn mediaservo_codec::VideoDecoder>,
    receiver: &TrackReceiver,
    au: &[u8],
) {
    if decoder.push_packet(au).is_err() {
        return;
    }
    while let Ok(Some(frame)) = decoder.pull_frame() {
        let raw = flatten_i420_planes(&frame);
        if let Ok(guard) = receiver.sink.lock() {
            if let Some(ref sink) = *guard {
                sink.on_frame(&raw, frame.format.width, frame.format.height);
            }
        }
    }
}

/// ponytail: concatenate Y, U, V planes into a contiguous I420 buffer.
fn flatten_i420_planes(frame: &VideoFrame) -> Vec<u8> {
    let len: usize = frame.planes.iter().map(|p| p.data.len()).sum();
    let mut out = Vec::with_capacity(len);
    for plane in &frame.planes {
        out.extend_from_slice(&plane.data);
    }
    out
}
// ── WebrtcRsDc ──

#[derive(Clone)]
pub(crate) struct WebrtcRsDc {
    inner: Arc<webrtc::data_channel::RTCDataChannel>,
}

impl std::fmt::Debug for WebrtcRsDc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcRsDc")
            .field("id", &self.inner.id())
            .field("label", &self.inner.label())
            .finish()
    }
}
impl WebrtcRsDc {
    pub(crate) fn new(inner: Arc<webrtc::data_channel::RTCDataChannel>) -> Self {
        Self { inner }
    }
}
impl DcBackend for WebrtcRsDc {
    fn state(&self) -> RTCDataChannelState {
        use webrtc::data_channel::data_channel_state::RTCDataChannelState::*;
        match self.inner.ready_state() {
            Connecting => RTCDataChannelState::Connecting,
            Open => RTCDataChannelState::Open,
            Closing => RTCDataChannelState::Closing,
            Closed => RTCDataChannelState::Closed,
            _ => RTCDataChannelState::Closed,
        }
    }

    /// S6/K5: webrtc-rs buffered_amount 为 async（内部锁）——u64 拓宽透传。
    async fn buffered_amount(&self) -> u64 {
        self.inner.buffered_amount().await as u64
    }

    async fn send(&self, data: &[u8]) -> Result<(), RTCError> {
        let b = bytes::Bytes::copy_from_slice(data);
        self.inner.send(&b).await.map(|_| ()).map_err(|e| RTCError::RTCDataChannel(e.to_string()))
    }

    async fn send_text(&self, text: &str) -> Result<(), RTCError> {
        self.inner
            .send_text(text)
            .await
            .map(|_| ())
            .map_err(|e| RTCError::RTCDataChannel(e.to_string()))
    }

    async fn spool(&self) -> RTCDataChannelRx {
        let dc = self.inner.clone();
        // broadcast（F1 审查 #1）: 多订阅端共享事件流；Sender 克隆进各回调
        let (tx, rx) = tokio::sync::broadcast::channel(64);
        let tx2 = tx.clone();
        dc.on_open(Box::new(move || {
            let _ = tx2.send(RTCDataChannelEvent::Open);
            Box::pin(async {})
        }));
        let tx2 = tx.clone();
        dc.on_close(Box::new(move || {
            let _ = tx2.send(RTCDataChannelEvent::Closed);
            Box::pin(async {})
        }));
        let tx2 = tx.clone();
        dc.on_message(Box::new(move |msg| {
            let data = msg.data.to_vec();
            let _ = tx2.send(RTCDataChannelEvent::Message(RTCDataMessage { data }));
            Box::pin(async {})
        }));
        dc.on_error(Box::new(move |err| {
            let _ = tx.send(RTCDataChannelEvent::Error(err.to_string()));
            Box::pin(async {})
        }));
        RTCDataChannelRx::new(Some(rx))
    }

    async fn close(&mut self) {
        self.inner.close().await.ok();
    }
}

// ── WebrtcRsTrack ──

pub(crate) struct WebrtcRsTrack {
    inner:
        Option<Arc<webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample>>,
    encoder: Mutex<Option<Box<dyn VideoEncoder>>>,
}

impl WebrtcRsTrack {
    pub(crate) fn new(
        track: Arc<webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample>,
    ) -> Self {
        Self { inner: Some(track), encoder: Mutex::new(None) }
    }

    /// Initialize the H.264 encoder. Called before first write_raw_i420.
    pub(crate) fn init_encoder(&self, width: u32, height: u32) -> Result<(), RTCError> {
        let config = EncoderConfig {
            codec: CodecId::H264,
            format: VideoFormat { width, height, pixel_format: PixelFormat::Yuv420p },
            bitrate: Bitrate::Vbr { target: 2_000_000, max: 4_000_000 },
            fps: mediaservo_codec::FrameRate { num: 30, den: 1 },
            preset: EncoderPreset::P1UltraFast,
            gop: 30,
        };
        let factory = CodecFactory::new();
        let mut encoder = factory
            .create_encoder(config.clone(), None)
            .map_err(|e| RTCError::Track(format!("codec: {e}")))?;
        encoder.configure(&config).map_err(|e| RTCError::Track(format!("codec configure: {e}")))?;
        *self.encoder.lock().unwrap() = Some(encoder);
        Ok(())
    }
}

impl Default for WebrtcRsTrack {
    fn default() -> Self {
        Self { inner: None, encoder: Mutex::new(None) }
    }
}

impl std::fmt::Debug for WebrtcRsTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcRsTrack").field("has_track", &self.inner.is_some()).finish()
    }
}

impl Clone for WebrtcRsTrack {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), encoder: Mutex::new(None) }
    }
}

impl TrackWriteBackend for WebrtcRsTrack {
    async fn write_frame(
        &self,
        data: &[u8],
        _kind: TrackKind,
        audio_config: Option<&RTCAudioTrackConfig>,
    ) -> Result<(), RTCError> {
        if let Some(ref track) = self.inner {
            // ponytail: audio uses config frame duration, video uses 30fps
            let duration_ms = audio_config.map(|c| c.frame_duration_ms()).unwrap_or(33);
            let sample = webrtc::media::Sample {
                data: bytes::Bytes::copy_from_slice(data),
                duration: std::time::Duration::from_millis(duration_ms),
                ..Default::default()
            };
            track.write_sample(&sample).await.map_err(|e| RTCError::Track(e.to_string()))?;
        }
        Ok(())
    }
    async fn write_raw_i420_with_ts(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        ts_us: Option<i64>,
    ) -> Result<(), RTCError> {
        // ponytail: auto-init encoder on first call
        if self.encoder.lock().unwrap().is_none() {
            self.init_encoder(width, height)?;
        }
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;
        if data.len() < y_size + 2 * uv_size {
            return Err(RTCError::Track("I420 data too short".into()));
        }
        let frame = VideoFrame {
            format: VideoFormat { width, height, pixel_format: PixelFormat::Yuv420p },
            planes: vec![
                Plane { data: data[..y_size].to_vec(), stride: width },
                Plane { data: data[y_size..y_size + uv_size].to_vec(), stride: width / 2 },
                Plane {
                    data: data[y_size + uv_size..y_size + 2 * uv_size].to_vec(),
                    stride: width / 2,
                },
            ],
            pts: ts_us.unwrap_or(0) as u64, // PIT-63: 时间戳参数化 (None → 保持 0)
            keyframe: false,
        };
        // Push frame under lock, release before writing
        {
            let mut guard = self.encoder.lock().unwrap();
            let enc =
                guard.as_mut().ok_or_else(|| RTCError::Track("encoder not initialized".into()))?;
            enc.push_frame(&frame).map_err(|e| RTCError::Track(format!("codec push: {e}")))?;
        }
        // Drain packets — acquire/release lock per iteration
        loop {
            let packet = {
                let mut guard = self.encoder.lock().unwrap();
                let enc = guard
                    .as_mut()
                    .ok_or_else(|| RTCError::Track("encoder not initialized".into()))?;
                enc.pull_packet().map_err(|e| RTCError::Track(format!("codec pull: {e}")))?
            };
            match packet {
                Some(p) => self.write_frame(&p.data, TrackKind::Video, None).await?,
                None => break,
            }
        }
        Ok(())
    }
}

// ── WebrtcRsFactory ──

pub(crate) struct WebrtcRsFactory {
    api: webrtc::api::API,
}

impl std::fmt::Debug for WebrtcRsFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcRsFactory").finish()
    }
}

impl Default for WebrtcRsFactory {
    fn default() -> Self {
        let api = webrtc::api::APIBuilder::new().build();
        Self { api }
    }
}

impl WebrtcRsFactory {
    pub(crate) async fn create_peer_connection(
        &self,
        config: crate::peer_connection::RTCConfiguration,
    ) -> Result<WebrtcRsPc, RTCError> {
        tracing::info!("Creating RTCPeerConnection (webrtc-rs)");
        let mut cfg = webrtc::peer_connection::configuration::RTCConfiguration::default();
        for srv in &config.ice_servers {
            cfg.ice_servers.push(webrtc::ice_transport::ice_server::RTCIceServer {
                urls: srv.urls.clone(),
                username: srv.username.clone(),
                credential: srv.password.clone(),
            });
        }
        let pc = self
            .api
            .new_peer_connection(cfg)
            .await
            .map_err(|e| RTCError::RTCPeerConnection(e.to_string()))?;
        let rs_pc = WebrtcRsPc::new(Arc::new(pc));

        // Register ICE connection state monitoring callback
        rs_pc.on_ice_connection_state_change(Box::new(|state| {
            let name = match state {
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::New => "New",
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Checking => "Checking",
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Connected => "Connected",
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Completed => "Completed",
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Failed => "Failed",
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Disconnected => "Disconnected",
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Closed => "Closed",
                _ => "Unknown",
            };
            if matches!(state, webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Failed) {
                tracing::warn!("ICE connection state: {name} — ICE restart may be needed");
            } else {
                tracing::info!("ICE connection state changed: {name}");
            }
            Box::pin(async {})
        }));

        Ok(rs_pc)
    }

    /// Create a video track with H.264 RTP codec.
    /// Returns (WebrtcRsTrack, Arc<dyn TrackLocal + Send + Sync>) —
    /// the media track can be added to the RTCPeerConnection via add_track.
    pub(crate) fn create_video_track(
        &self,
    ) -> (WebrtcRsTrack, std::sync::Arc<dyn webrtc::track::track_local::TrackLocal + Send + Sync>)
    {
        use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
        use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

        let track = std::sync::Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "video/h264".to_owned(),
                clock_rate: 90000,
                ..Default::default()
            },
            "video".to_owned(),
            "video".to_owned(),
        ));

        let backend = WebrtcRsTrack::new(track.clone());
        let media_track: std::sync::Arc<dyn webrtc::track::track_local::TrackLocal + Send + Sync> =
            track;

        (backend, media_track)
    }
}
