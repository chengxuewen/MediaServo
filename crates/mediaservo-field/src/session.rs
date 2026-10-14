//! Field session facades: push (PushSession) / pull (PullSession).
//!
//! Contract §4: `PushSession` (capture → encode → push) / `PullSession` (subscribe → decode → frames out).
//! The push chain reuses the host SFU push sequence (CreateWebRtcTransport → answer negotiation →
//! Connect → Produce → WebRtcTrackSink frame injection) via the mediaservo-webrtc abstraction (C12).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceCandidate, IceParameters, MediaKind, PeerRole,
    SignalingMessage, TransportDirection,
};
use mediaservo_deck::DeckError;
use mediaservo_link::{SignalClient, SignalEvent, SignalSession};
use mediaservo_media::pipeline::generator::{
    Anchor, BitmapFont, ColorStrategy, PatternMode, SquaresConfig, TextBurner, TimestampFormat,
    TimestampOverlay, VideoFrameGenerator,
};
use mediaservo_media::pipeline::sink::VideoSinkWants;
use mediaservo_media::pipeline::source::VideoSource;
use mediaservo_webrtc::rtp::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCConfiguration, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnection,
    RTCPeerConnectionFactory, RTCSdpType, RTCSessionDescription, TrackKind, TrackRef, TrackSender,
    WebRtcTrackSink,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::config::{PublishOptions, PullConfig, PushConfig};
use crate::error::FieldError;
use crate::sfu;

/// Session event stream (contract §4).
#[derive(Debug)]
#[non_exhaustive]
pub enum SessionEvent {
    /// Connected and joined the room.
    Connected,
    /// A signaling message was received (pass-through; upper layers use it for SFU/device discovery etc.).
    Message(SignalingMessage),
    /// Push session: track published successfully.
    TrackPublished { track: String },
    /// Pull session: subscribed successfully.
    TrackSubscribed { track: String },
    /// Connection lost.
    Disconnected { reason: String },
    /// Error.
    Error(FieldError),
}

/// Session event receiver.
pub type SessionEvents = UnboundedReceiver<SessionEvent>;

/// Session event sender (internal).
pub(crate) type EventSender = UnboundedSender<SessionEvent>;

/// Push session (capture → encode → webrtc push).
pub struct PushSession {
    signal: SignalSession,
    /// Established PeerConnection (lazily initialized on the first track publish).
    pc: Option<RTCPeerConnection>,
    /// Published video sender (single video track in the current MVP).
    video_sender: Option<TrackSender>,
    /// Frame generator (PIT-81: must be owned and kept alive — Drop stops the thread; stop_video_frames stops explicitly).
    frame_generator: Option<VideoFrameGenerator>,
    events: EventSender,
    closed: Arc<AtomicBool>,
}

impl PushSession {
    /// Connect signaling and join the room; media (transport/produce) is established on the first `publish_video`.
    pub async fn connect(cfg: PushConfig) -> Result<(Self, SessionEvents), FieldError> {
        // D2: gateway_src = Some → local gateway mode (envelope wire, no PSK); None → direct to server
        let client = match &cfg.gateway_src {
            Some(src) => SignalClient::new_gateway(&cfg.url, src, &cfg.room, cfg.role.clone()),
            None => SignalClient::new(&cfg.url, &cfg.psk, &cfg.room, cfg.role.clone()),
        };
        let signal = client.connect().await.map_err(FieldError::Link)?;
        tracing::info!(room = %cfg.room, "PushSession connected to room");

        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut signal_events = signal.events();
        let events_tx_bridge = events_tx.clone();
        tokio::spawn(async move {
            while let Ok(ev) = signal_events.recv().await {
                let bridge = match ev {
                    SignalEvent::Message(SignalingMessage::Error { code, message })
                        if code != 0 =>
                    {
                        // H6: server/gateway error surface (code≠0) escalates to SessionEvent::Error — a Message pass-through
                        // would be swallowed by the downstream `_ =>{}` arm (the 5001 upstream switch becomes invisible).
                        SessionEvent::Error(FieldError::Link(mediaservo_link::LinkError::Signal(
                            format!("[{code}] {message}"),
                        )))
                    }
                    SignalEvent::Message(msg) => SessionEvent::Message(msg),
                    SignalEvent::Disconnected { reason } => SessionEvent::Disconnected { reason },
                    SignalEvent::Error(e) => {
                        SessionEvent::Error(FieldError::Link(mediaservo_link::LinkError::Signal(e)))
                    }
                    SignalEvent::Connected { .. } => continue, // connect already returned Connected
                    _ => continue, // Other signal events ignored (non_exhaustive fallback)
                };
                if events_tx_bridge.send(bridge).is_err() {
                    break;
                }
            }
        });

        let session = Self {
            signal,
            pc: None,
            video_sender: None,
            frame_generator: None,
            events: events_tx,
            closed: Arc::new(AtomicBool::new(false)),
        };
        Ok((session, events_rx))
    }

    /// Publish one video: SFU transport setup + answer negotiation + produce + encoder config.
    /// MVP supports a single video track; returns the track id (identical to mediaservo-webrtc `add_track`'s track id).
    pub async fn publish_video(
        &mut self,
        cfg: &PushConfig,
        opts: &PublishOptions,
    ) -> Result<String, FieldError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(FieldError::Closed);
        }
        if self.pc.is_some() {
            return Err(FieldError::InvalidState(
                "MVP PushSession supports a single video — an active track already exists".into(),
            ));
        }

        // 1. Subscribe to signaling events first (before sending CreateWebRtcTransport — avoids the server
        // responding faster than subscribe and losing WebRtcTransportCreated; broadcast has no history replay)
        let mut transport_events = self.signal.events();

        // 2. CreateWebRtcTransport (Send)
        let create_transport = SignalingMessage::CreateWebRtcTransport {
            room_id: cfg.room.clone(),
            peer_id: peer_id(&cfg.role, cfg),
            direction: TransportDirection::Send,
        };
        self.signal.send(create_transport).await.map_err(FieldError::Link)?;

        // 3. Wait for WebRtcTransportCreated (other messages ignored/pass through)
        let (transport_id, ice_parameters, dtls_parameters, ice_candidates) =
            self.await_transport_created(&mut transport_events).await?;

        // 4. Establish the RTCPeerConnection (mediaservo-webrtc abstraction, C12)
        let rtc_config = RTCConfiguration {
            ice_servers: Vec::<RTCIceServer>::new(), // SFU: mediasoup ICE-Lite, no STUN needed
            ice_transport_type: RTCIceTransportPolicy::All,
        };
        let factory = RTCPeerConnectionFactory::new();
        let pc = factory
            .create_peer_connection(rtc_config)
            .await
            .map_err(|e| FieldError::WebRtc(format!("create peer connection: {e}")))?;

        // 5. Standard answerer negotiation: remote SDP → add_track → create_answer (P3 v2, C18)
        let sfu::CodecSpec { payload_type, name, clock_rate, fmtp } = sfu::codec_spec(&opts.codec);
        let remote_sdp = sfu::build_remote_sdp(
            &ice_parameters,
            &dtls_parameters,
            ice_candidates.as_ref(),
            payload_type,
            name,
            clock_rate,
            fmtp,
            sfu::RemoteDirection::ServerRecvonly,
        );
        let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
        pc.set_remote_description(&remote_desc)
            .await
            .map_err(|e| FieldError::WebRtc(format!("set remote description: {e}")))?;

        let track_id = pc
            .add_track("video", TrackKind::Video)
            .map_err(|e| FieldError::WebRtc(format!("add_track: {e}")))?;
        let track_ref = pc
            .get_track(&track_id)
            .ok_or_else(|| FieldError::WebRtc("track not found after add_track".into()))?;
        let sender = match track_ref {
            TrackRef::Sender(s) => s,
            _ => return Err(FieldError::WebRtc("expected sender track".into())),
        };

        // Encoder backend (soft/hardware) preference — set before negotiation (matches host: RTCRtpSender via get_senders)
        if let Some(backend) =
            mediaservo_webrtc::rtp::RTCVideoEncoderBackend::from_config(&opts.encoder_backend)
            && backend != mediaservo_webrtc::rtp::RTCVideoEncoderBackend::Auto
        {
            match pc.get_senders().iter().find(|s| s.track_id == track_id) {
                Some(rtp_sender) => {
                    if let Err(e) = rtp_sender.set_video_encoder_backend(backend) {
                        tracing::warn!("set_video_encoder_backend({backend:?}): {e}");
                    }
                }
                None => tracing::warn!("sender not found for backend config: {track_id}"),
            }
        }

        let answer = pc
            .create_answer(&mediaservo_webrtc::RTCAnswerOptions)
            .await
            .map_err(|e| FieldError::WebRtc(format!("create answer: {e}")))?;
        pc.set_local_description(&answer)
            .await
            .map_err(|e| FieldError::WebRtc(format!("set local description: {e}")))?;

        // Encoder bitrate range (after negotiation, before produce)
        let max_bps = u64::from(cfg.bitrate_kbps) * 1000;
        let min_bps = cfg.min_bitrate_kbps.map(|k| u64::from(k) * 1000);
        match pc.get_senders().iter().find(|s| s.track_id == track_id) {
            Some(rtp_sender) => {
                if let Err(e) = rtp_sender.set_encoding_bitrate(min_bps, Some(max_bps)) {
                    tracing::warn!("set_encoding_bitrate: {e}");
                }
                // Encoder fps cap (multi-stream P1: host.yaml fps → libwebrtc
                // max_framerate via SetParameters) — frame timestamps anchored per C17, independent of fps
                if let Err(e) = rtp_sender.set_encoding_framerate(Some(f64::from(cfg.framerate))) {
                    tracing::warn!("set_encoding_framerate: {e}");
                }
                // qos-framerate-priority (AD-6): default values do not call the setters — the balanced tier keeps the live
                // SetParameters call sequence untouched; only non-defaults are sent, Err warns and falls back (C15).
                if cfg.degradation != mediaservo_webrtc::rtp::RTCDegradationPreference::Balanced
                    && let Err(e) = rtp_sender.set_degradation_preference(cfg.degradation)
                {
                    tracing::warn!("set_degradation_preference({:?}): {e}", cfg.degradation);
                }
                if cfg.content_hint != mediaservo_webrtc::rtp::RTCRtpContentHint::None
                    && let Err(e) = rtp_sender.set_content_hint(cfg.content_hint)
                {
                    tracing::warn!("set_content_hint({:?}): {e}", cfg.content_hint);
                }
            }
            None => tracing::warn!("sender not found for bitrate config: {track_id}"),
        }

        // 6. ConnectWebRtcTransport (DTLS fingerprint via API, not parsed from SDP)
        let fp_hex = pc
            .local_dtls_fingerprint()
            .ok_or_else(|| FieldError::WebRtc("no DTLS fingerprint".into()))?;
        let connect = SignalingMessage::ConnectWebRtcTransport {
            room_id: cfg.room.clone(),
            peer_id: peer_id(&cfg.role, cfg),
            transport_id: transport_id.clone(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
                role: "client".to_string(),
            },
        };
        self.signal.send(connect).await.map_err(FieldError::Link)?;

        // 7. Produce (rtp_parameters derived from the negotiation result, official P3 v2 path)
        let rtp_params = pc
            .get_sending_rtp_parameters("video")
            .map_err(|e| FieldError::WebRtc(format!("get_sending_rtp_parameters: {e}")))?;
        let produce = SignalingMessage::Produce {
            room_id: cfg.room.clone(),
            peer_id: peer_id(&cfg.role, cfg),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Video,
            rtp_parameters: sfu::build_produce_rtp_parameters_from_rtp(&rtp_params),
            // C1: bind by name to the send transport created by this session (when host child processes share a peer_id,
            // do not depend on the "most recently created" single slot — guards against boot-storm cross-talk)
            transport_id: Some(transport_id.clone()),
            // producer-label T3: publish label pass-through (None = legacy form)
            label: opts.label.clone(),
        };
        self.signal.send(produce).await.map_err(FieldError::Link)?;

        self.pc = Some(pc);
        self.video_sender = Some(sender);
        let _ = self.events.send(SessionEvent::TrackPublished { track: track_id.clone() });
        tracing::info!(track = %track_id, "PushSession video published (codec={})", opts.codec);
        Ok(track_id)
    }

    /// Consume signaling until `WebRtcTransportCreated` (delegates to the free function).
    async fn await_transport_created(
        &self,
        events: &mut tokio::sync::broadcast::Receiver<SignalEvent>,
    ) -> Result<(String, IceParameters, DtlsParameters, Option<Vec<IceCandidate>>), FieldError>
    {
        await_transport_created_msg(events).await
    }

    /// Get the underlying PeerConnection (escape hatch; None means nothing published yet).
    /// Read-only signaling session handle (periodic control message reporting like EncoderStatus — room broadcast is relayed by the server; failure handling belongs to the caller C15/C16).
    pub fn signal(&self) -> &SignalSession {
        &self.signal
    }

    pub fn peer_connection(&self) -> Option<&RTCPeerConnection> {
        self.pc.as_ref()
    }

    /// Get the TrackSender of the published video track (for external frame sources to inject frames directly, e.g. a FrameBus
    /// subscription; None before publish_video). C2 host-streamer consumption: writing frames goes through
    /// `write_raw_i420_with_ts` (timestamp from frame metadata, C17).
    pub fn video_sender(&self) -> Option<TrackSender> {
        self.video_sender.clone()
    }

    /// Start video frame generation (Squares color bars + timestamp watermark) → WebRtcTrackSink → TrackSender.
    /// Mirrors the host B5 chain (C17: anchored monotonic timestamps + absolute-timeline frame loop built into the generator).
    /// Requires `publish_video` first; repeated calls return InvalidState.
    pub fn start_video_frames(&mut self, cfg: &PushConfig) -> Result<(), FieldError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(FieldError::Closed);
        }
        if self.frame_generator.is_some() {
            return Err(FieldError::InvalidState("frames already running".into()));
        }
        let sender = self.video_sender.clone().ok_or_else(|| {
            FieldError::InvalidState("publish_video first (no video track)".into())
        })?;

        let fps = cfg.framerate.max(1);
        let (width, height) = (cfg.width, cfg.height);

        // Timestamp watermark (matches host: TopLeft + DateTime+FrameCount)
        let burner = TextBurner::new(BitmapFont::new(), false, Anchor::TopLeft);
        let overlay = TimestampOverlay::new(burner, TimestampFormat::Combined);
        let squares = SquaresConfig {
            count: 40,
            min_size: 20,
            max_size: 300,
            motion_speed: 10,
            color_strategy: ColorStrategy::default(),
        };

        let generator = VideoFrameGenerator::new();
        let sink = WebRtcTrackSink::new(sender)
            .map_err(|e| FieldError::WebRtc(format!("WebRtcTrackSink: {e}")))?;
        generator.add_or_update_sink(Box::new(sink), VideoSinkWants::default());
        generator.start(fps, PatternMode::Squares(squares), Some(overlay), width, height);

        // PIT-81: the generator must be owned and kept alive (Drop stops the thread)
        self.frame_generator = Some(generator);
        tracing::info!("PushSession frames started {width}x{height}@{fps}fps");
        Ok(())
    }

    /// Stop frame generation (idempotent; repeated calls have no effect).
    pub fn stop_video_frames(&mut self) {
        if let Some(g) = self.frame_generator.take() {
            g.stop();
            tracing::info!("PushSession frames stopped");
        }
    }

    /// Close the session: close signaling + mark closed (media sending is stopped by the frame task).
    pub async fn close(self) -> Result<(), FieldError> {
        self.closed.store(true, Ordering::Relaxed);
        self.signal.close().await.map_err(FieldError::Link)
    }
}

impl std::fmt::Debug for PushSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushSession")
            .field("signal", &self.signal)
            .field("pc", &self.pc)
            .field("video_sender", &self.video_sender)
            .field("frames_running", &self.frame_generator.is_some())
            .field("closed", &self.closed)
            .finish()
    }
}

/// Pull session (subscribe → decode → frames out).
pub struct PullSession {
    signal: SignalSession,
    /// Established PeerConnection (lazily initialized at subscribe).
    pc: Option<RTCPeerConnection>,
    events: EventSender,
    closed: Arc<AtomicBool>,
}

/// Decoded frame output by the pull session (I420 planar: Y + U + V).
#[derive(Debug)]
pub struct PullFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Monotonic-clock microsecond timestamp (C17 semantics).
    pub ts_us: i64,
}

impl PullSession {
    /// Connect signaling and join the room (consume/decode chain).
    pub async fn connect(cfg: PullConfig) -> Result<(Self, SessionEvents), FieldError> {
        let client = SignalClient::new(&cfg.url, &cfg.psk, &cfg.room, cfg.role.clone());
        let signal = client.connect().await.map_err(FieldError::Link)?;
        tracing::info!(room = %cfg.room, "PullSession connected to room");

        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut signal_events = signal.events();
        let events_tx_bridge = events_tx.clone();
        tokio::spawn(async move {
            while let Ok(ev) = signal_events.recv().await {
                let bridge = match ev {
                    SignalEvent::Message(SignalingMessage::Error { code, message })
                        if code != 0 =>
                    {
                        // H6: same as PushSession — escalate the error surface so it is not swallowed by Message.
                        SessionEvent::Error(FieldError::Link(mediaservo_link::LinkError::Signal(
                            format!("[{code}] {message}"),
                        )))
                    }
                    SignalEvent::Message(msg) => SessionEvent::Message(msg),
                    SignalEvent::Disconnected { reason } => SessionEvent::Disconnected { reason },
                    SignalEvent::Error(e) => {
                        SessionEvent::Error(FieldError::Link(mediaservo_link::LinkError::Signal(e)))
                    }
                    SignalEvent::Connected { .. } => continue,
                    _ => continue,
                };
                if events_tx_bridge.send(bridge).is_err() {
                    break;
                }
            }
        });

        let session =
            Self { signal, pc: None, events: events_tx, closed: Arc::new(AtomicBool::new(false)) };
        Ok((session, events_rx))
    }

    /// Subscribe to one producer video stream: Recv transport → standard answerer negotiation →
    /// Connect → Consume → Consumed → on_track frame reception.
    ///
    /// Returns the decoded frame stream receiver (`recv()` for I420 frames; latest semantics guaranteed by channel capacity).
    pub async fn subscribe(
        &mut self,
        cfg: &PullConfig,
        producer_id: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<PullFrame>, FieldError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(FieldError::Closed);
        }
        if self.pc.is_some() {
            return Err(FieldError::InvalidState(
                "MVP PullSession supports a single subscription — an active subscription already exists".into(),
            ));
        }

        // 1. Subscribe to signaling events first (guard against broadcast races, same as PushSession)
        let mut transport_events = self.signal.events();

        // 2. CreateWebRtcTransport (Recv)
        self.signal
            .send(SignalingMessage::CreateWebRtcTransport {
                room_id: cfg.room.clone(),
                peer_id: pull_peer_id(&cfg.role),
                direction: TransportDirection::Recv,
            })
            .await
            .map_err(FieldError::Link)?;

        tracing::info!("PullSession CreateWebRtcTransport(Recv) sent, waiting transport");
        // 3. Wait for WebRtcTransportCreated (free function shared by Push/Pull)
        let (transport_id, ice_parameters, dtls_parameters, ice_candidates) =
            await_transport_created_msg(&mut transport_events).await?;
        tracing::info!("PullSession transport created: {transport_id}");

        // 4. Establish the RTCPeerConnection (mediaservo-webrtc abstraction, C12)
        let rtc_config = RTCConfiguration {
            ice_servers: Vec::<RTCIceServer>::new(),
            ice_transport_type: RTCIceTransportPolicy::All,
        };
        let factory = RTCPeerConnectionFactory::new();
        let pc = factory
            .create_peer_connection(rtc_config)
            .await
            .map_err(|e| FieldError::WebRtc(format!("create peer connection: {e}")))?;

        // Diagnostics: ICE/PC connection state
        {
            let pc_dbg = pc.clone();
            pc.on_ice_connection_state_change(move |s| {
                tracing::info!("PullSession ICE state: {:?}", s);
            });
            let _ = pc_dbg;
            let pc_dbg2 = pc.clone();
            pc.on_peer_connection_state_change(move |s| {
                tracing::info!("PullSession PC state: {:?}", s);
            });
            let _ = pc_dbg2;
        }

        // Frame reception: on_track must be registered before set_remote_description!
        // (remote sendonly m-line → libwebrtc fires on_track during setRemoteDescription;
        //   late registration loses the first track)
        let (frame_tx, frame_rx) = tokio::sync::mpsc::channel::<PullFrame>(3);
        let frame_tx_for_cb = frame_tx.clone();
        pc.on_track(move |receiver| {
            let tx = frame_tx_for_cb.clone();
            tracing::info!(
                "PullSession on_track: kind={:?} id={}",
                receiver.kind,
                receiver.track_id
            );
            if let TrackRef::Receiver(r) = receiver.track {
                tracing::info!("PullSession attaching FrameSink to receiver {}", r.id);
                r.set_frame_sink(Box::new(PullFrameSink { tx }));
            }
        });

        // 5. Consume first (take the consumer's ssrc — it must be injected into the remote SDP, otherwise libwebrtc
        // demux does not recognize the ssrc and drops RTP). Consumed may return right after transport creation.
        self.signal
            .send(SignalingMessage::Consume {
                room_id: cfg.room.clone(),
                peer_id: pull_peer_id(&cfg.role),
                producer_id: producer_id.to_string(),
                rtp_capabilities: serde_json::json!({
                    // Keep the minimal codec verified by e2e_sfu D2 (adding fields triggers a mediasoup
                    // RtpCodecCapability deserialization failure: missing field kind)
                    "codecs": [{"mimeType": "video/VP8", "clockRate": 90000, "kind": "video"}],
                    // headerExtensions aligned with the producer-negotiated extmap (mid/transport-cc/abs-capture-time)
                    "headerExtensions": [
                        {"uri": "urn:ietf:params:rtp-hdrext:sdes:mid", "preferredId": 1, "kind": "video", "preferredEncrypt": false, "direction": "sendrecv"},
                        {"uri": "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01", "preferredId": 3, "kind": "video", "preferredEncrypt": false, "direction": "sendrecv"},
                        {"uri": "http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time", "preferredId": 5, "kind": "video", "preferredEncrypt": false, "direction": "sendrecv"},
                    ],
                }),
                // C1: bind by name to the recv transport created by this session (when multiple connections share a peer_id,
                // do not depend on the "most recently created" single slot — guards against a consumer attaching to the wrong transport and a black screen)
                transport_id: Some(transport_id.clone()),
            })
            .await
            .map_err(FieldError::Link)?;
        let consumer_rtp = self.await_consumed(&mut transport_events).await?;

        // 6. Build the remote SDP containing the consumer ssrc (a=ssrc declares the received stream)
        let codec = sfu::codec_spec("vp8");
        let remote_sdp = sfu::build_remote_sdp(
            &ice_parameters,
            &dtls_parameters,
            ice_candidates.as_ref(),
            codec.payload_type,
            codec.name,
            codec.clock_rate,
            codec.fmtp,
            sfu::RemoteDirection::ServerSendonly,
        );
        // Inject a=ssrc (the consumer's encodings[0].ssrc)
        let remote_sdp = sfu::inject_remote_ssrc(&remote_sdp, &consumer_rtp);
        tracing::info!("PullSession remote SDP (after ssrc inject):\n{remote_sdp}");
        // W3C-recommended order: add_transceiver first (pre-build the recv transceiver) → set_remote_description
        // (the remote sendonly m-line matches the existing transceiver — otherwise libwebrtc auto-creates
        //   a second transceiver and the receiver ends up with incomplete params, codecs=0)
        pc.add_transceiver(
            TrackKind::Video,
            RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                ..Default::default()
            },
        )
        .map_err(|e| FieldError::WebRtc(format!("add_transceiver: {e}")))?;

        let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
        pc.set_remote_description(&remote_desc)
            .await
            .map_err(|e| FieldError::WebRtc(format!("set remote description: {e}")))?;

        let answer = pc
            .create_answer(&mediaservo_webrtc::RTCAnswerOptions)
            .await
            .map_err(|e| FieldError::WebRtc(format!("create answer: {e}")))?;
        tracing::info!("PullSession answer SDP:\n{}", answer.sdp);
        pc.set_local_description(&answer)
            .await
            .map_err(|e| FieldError::WebRtc(format!("set local description: {e}")))?;

        // 6. ConnectWebRtcTransport（DTLS fingerprint）
        let fp_hex = pc
            .local_dtls_fingerprint()
            .ok_or_else(|| FieldError::WebRtc("no DTLS fingerprint".into()))?;
        self.signal
            .send(SignalingMessage::ConnectWebRtcTransport {
                room_id: cfg.room.clone(),
                peer_id: pull_peer_id(&cfg.role),
                transport_id: transport_id.clone(),
                dtls_parameters: DtlsParameters {
                    fingerprints: vec![Fingerprint {
                        algorithm: "sha-256".to_string(),
                        value: fp_hex,
                    }],
                    role: "client".to_string(),
                },
            })
            .await
            .map_err(FieldError::Link)?;

        let pc_state = pc.connection_state();
        let ice_state = pc.ice_connection_state();
        self.pc = Some(pc);
        let _ = self.events.send(SessionEvent::TrackSubscribed { track: producer_id.to_string() });
        tracing::info!(
            "PullSession subscribed to producer {producer_id} (pc={pc_state:?} ice={ice_state:?})"
        );
        Ok(frame_rx)
    }

    /// Consume signaling until the `Consumed` ack; returns the consumer's rtp_parameters (including ssrc).
    async fn await_consumed(
        &self,
        events: &mut tokio::sync::broadcast::Receiver<SignalEvent>,
    ) -> Result<serde_json::Value, FieldError> {
        loop {
            match events.recv().await {
                Ok(SignalEvent::Message(SignalingMessage::Consumed {
                    producer_id,
                    rtp_parameters,
                    ..
                })) => {
                    tracing::info!(
                        "PullSession consumer created for {producer_id} rtp={rtp_parameters}"
                    );
                    return Ok(rtp_parameters);
                }
                // transport_connected is the Connect ack (not a real error)
                Ok(SignalEvent::Message(SignalingMessage::Error { code: 0, message }))
                    if message == "transport_connected" => {}
                Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                    return Err(FieldError::WebRtc(format!("SFU error [{code}]: {message}")));
                }
                Ok(SignalEvent::Disconnected { reason }) => {
                    return Err(FieldError::InvalidState(format!(
                        "signal disconnected during consume: {reason}"
                    )));
                }
                Ok(_) => {} // Other messages ignored (NewProducer/TransportCreated etc.)
                Err(_) => {
                    return Err(FieldError::InvalidState(
                        "signal event stream closed during consume".into(),
                    ));
                }
            }
        }
    }

    /// Get the underlying PeerConnection (escape hatch).
    pub fn peer_connection(&self) -> Option<&RTCPeerConnection> {
        self.pc.as_ref()
    }

    /// Close the session.
    pub async fn close(self) -> Result<(), FieldError> {
        self.closed.store(true, Ordering::Relaxed);
        self.signal.close().await.map_err(FieldError::Link)
    }
}

/// FrameSink impl: on_track's I420 frames → bounded channel (drop when full = latest).
struct PullFrameSink {
    tx: tokio::sync::mpsc::Sender<PullFrame>,
}

impl mediaservo_webrtc::track::FrameSink for PullFrameSink {
    fn on_frame(&self, data: &[u8], width: u32, height: u32) {
        let _ = self.tx.try_send(PullFrame {
            width,
            height,
            data: data.to_vec(),
            ts_us: 0, // Decoded frames carry no timestamp source (webrtc-sys does not expose one); consumers use arrival order
        });
    }
}

/// Convenience conversion from deck/link errors into FieldError (for later slices)./// Consume signaling until `WebRtcTransportCreated` (shared by Push/Pull).
/// `events` must be subscribed before the request is sent (broadcast has no history replay; subscribe first to avoid loss).
async fn await_transport_created_msg(
    events: &mut tokio::sync::broadcast::Receiver<SignalEvent>,
) -> Result<(String, IceParameters, DtlsParameters, Option<Vec<IceCandidate>>), FieldError> {
    loop {
        match events.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            })) => {
                return Ok((transport_id, ice_parameters, dtls_parameters, ice_candidates));
            }
            // transport_connected is the ack of ConnectWebRtcTransport (not a real error, server convention)
            Ok(SignalEvent::Message(SignalingMessage::Error { code: _, message }))
                if message == "transport_connected" => {}
            Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                return Err(FieldError::WebRtc(format!("SFU error [{code}]: {message}")));
            }
            Ok(SignalEvent::Disconnected { reason }) => {
                return Err(FieldError::InvalidState(format!(
                    "signal disconnected during transport create: {reason}"
                )));
            }
            Ok(SignalEvent::Connected { .. }) => {}
            Ok(_) => {} // Other messages ignored (NewProducer/RoomLeave etc., same class as PIT-59/65)
            Err(_) => {
                return Err(FieldError::InvalidState(
                    "signal event stream closed during transport create".into(),
                ));
            }
        }
    }
}

/// peer_id: reuses SignalClient's role semantics (host uses "host"),
/// `cfg.peer_suffix` appends a unique suffix (required in device rooms with multiple streams in one room — six streamers with
/// peer_id="host" replace each other and produce fails with "Peer host not found" (same class as PIT-65,
/// the 2026-10-09 generator3/5 black-screen root cause).
fn pull_peer_id(role: &PeerRole) -> String {
    match role {
        PeerRole::Host => "host",
        PeerRole::Remote => "remote",
        PeerRole::Consumer => "consumer",
    }
    .to_string()
}

fn peer_id(role: &PeerRole, cfg: &PushConfig) -> String {
    let base = match role {
        PeerRole::Host => "host",
        PeerRole::Remote => "remote",
        PeerRole::Consumer => "consumer",
    };
    match &cfg.peer_suffix {
        Some(s) => format!("{base}-{s}"),
        None => base.to_string(),
    }
}

#[allow(dead_code)] // conversion slots for some feature postures (PIT-203 family: unused in the default posture ≠ unused in all postures)
pub(crate) fn deck_err(e: DeckError) -> FieldError {
    FieldError::Deck(e)
}
