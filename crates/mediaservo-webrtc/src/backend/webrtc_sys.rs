//! webrtc-sys backend — wraps libwebrtc via LiveKit's webrtc-sys FFI crate.
//!
//! Enabled via `backend-webrtc-sys` feature.
//! Uses tokio::sync::oneshot channels to convert callback-based FFI to async.
//!
//! Backend types:
//! - WebrtcSysPc: wraps webrtc_sys::peer_connection::ffi::PeerConnection
//! - WebrtcSysDc: wraps webrtc_sys::data_channel::ffi::DataChannel
//! - WebrtcSysTrack: real video track via VideoTrackSource (webrtc-sys)
//! - WebrtcSysFactory: wraps webrtc_sys::peer_connection_factory::ffi::PeerConnectionFactory

use cxx::SharedPtr;
use std::sync::{Arc, Mutex};

use super::DcBackend;
use super::PcBackend;
use super::TrackWriteBackend;
use crate::RTCError;
use crate::data_channel::{RTCDataChannelRx, RTCDataChannelState};
use crate::peer_connection::{
    RTCAnswerOptions, RTCConfiguration, RTCIceCandidate, RTCIceConnectionState,
    RTCIceGatheringState, RTCIceTransportPolicy, RTCOfferOptions, RTCPeerConnectionState,
    RTCSignalingState,
};
use crate::sdp::{RTCSdpType, RTCSessionDescription};
use crate::track::{RTCAudioTrackConfig, TrackKind, TrackReceiver};

// ── WebrtcSysPc ──

#[derive(Clone)]
pub(crate) struct WebrtcSysPc {
    pc: cxx::SharedPtr<webrtc_sys::peer_connection::ffi::PeerConnection>,
    callbacks: Arc<ObserverCallbacks>,
    local_sdp: Arc<std::sync::Mutex<Option<String>>>,
    /// v2: the factory reference (capabilities queries need it)
    factory: cxx::SharedPtr<webrtc_sys::peer_connection_factory::ffi::PeerConnectionFactory>,
}

impl std::fmt::Debug for WebrtcSysPc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcSysPc").field("connection_state", &self.connection_state()).finish()
    }
}

/// Helper: wrap a oneshot sender in a PeerContext for FFI callbacks.
fn make_ctx<T: Send + 'static>(
    tx: tokio::sync::oneshot::Sender<T>,
) -> Box<webrtc_sys::peer_connection::PeerContext> {
    Box::new(webrtc_sys::peer_connection::PeerContext(Box::new(tx)))
}

/// Helper: extract a oneshot sender from a PeerContext via downcast.
#[allow(clippy::boxed_local)] // Box<PeerContext> is fixed by the ffi callback signature (type erasure), not collapsible
fn extract_tx<T: Send + 'static>(
    ctx: Box<webrtc_sys::peer_connection::PeerContext>,
) -> tokio::sync::oneshot::Sender<T> {
    *ctx.0
        .downcast::<tokio::sync::oneshot::Sender<T>>()
        .unwrap_or_else(|_| panic!("PeerContext downcast failed"))
}

/// Parse a libwebrtc getStats ToJson (array) -> outbound-rtp RTCStats.
/// Fields: framesEncoded/framesPerSecond/frameWidth/frameHeight/encoderImplementation (proven by Oracle F2).
/// S2c: libwebrtc inbound-rtp record parsing (the receive-side bisection predicate: packetsReceived vs framesDecoded).
#[allow(clippy::unnecessary_filter_map)] // the parse closure is always Some = literally obvious; changing to map means touching the closing brace group
fn parse_inbound_stats_json(json: &str) -> Vec<crate::stats::RTCStats> {
    use crate::stats::{RTCInboundRtpStreamStats, RTCStats};
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return vec![];
    };
    arr.iter()
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("inbound-rtp"))
        .filter_map(|v| {
            Some(RTCStats::InboundRtp(RTCInboundRtpStreamStats {
                id: v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                timestamp: v.get("timestamp").and_then(|x| x.as_f64()).unwrap_or(0.0),
                ssrc: v.get("ssrc").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                kind: v.get("kind").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                packets_received: v.get("packetsReceived").and_then(|x| x.as_u64()).unwrap_or(0),
                packets_lost: v
                    .get("packets_lost")
                    .or_else(|| v.get("packetsLost"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
                bytes_received: v.get("bytesReceived").and_then(|x| x.as_u64()).unwrap_or(0),
                frames_decoded: v.get("framesDecoded").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                frame_width: v.get("frameWidth").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                frame_height: v.get("frameHeight").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                frames_per_second: v.get("framesPerSecond").and_then(|x| x.as_f64()).unwrap_or(0.0),
                jitter: v.get("jitter").and_then(|x| x.as_f64()).unwrap_or(0.0),
                total_decode_time: v.get("totalDecodeTime").and_then(|x| x.as_f64()).unwrap_or(0.0),
                frame_dropped: v.get("frameDropped").and_then(|x| x.as_u64()).unwrap_or(0),
                nack_count: v.get("nackCount").and_then(|x| x.as_u64()).unwrap_or(0),
                pli_count: v.get("pliCount").and_then(|x| x.as_u64()).unwrap_or(0),
                fir_count: v.get("firCount").and_then(|x| x.as_u64()).unwrap_or(0),
            }))
        })
        .collect()
}

#[allow(clippy::unnecessary_filter_map)] // same as above
fn parse_outbound_stats_json(json: &str) -> Vec<crate::stats::RTCStats> {
    use crate::stats::{RTCOutboundRtpStreamStats, RTCStats};
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return vec![];
    };
    arr.iter()
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("outbound-rtp"))
        .filter_map(|v| {
            Some(RTCStats::OutboundRtp(RTCOutboundRtpStreamStats {
                id: v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                timestamp: v.get("timestamp").and_then(|x| x.as_f64()).unwrap_or(0.0),
                encoder_implementation: v
                    .get("encoderImplementation")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                ssrc: v.get("ssrc").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                kind: v.get("kind").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                packets_sent: v.get("packetsSent").and_then(|x| x.as_u64()).unwrap_or(0),
                bytes_sent: v.get("bytesSent").and_then(|x| x.as_u64()).unwrap_or(0),
                frames_encoded: v.get("framesEncoded").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                frame_width: v.get("frameWidth").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                frame_height: v.get("frameHeight").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                frames_per_second: v.get("framesPerSecond").and_then(|x| x.as_f64()).unwrap_or(0.0),
                // v3 (encode-time-stats T2): the W3C outbound-rtp standard field totalEncodeTime (seconds)
                total_encode_time: v.get("totalEncodeTime").and_then(|x| x.as_f64()),
            }))
        })
        .collect()
}

// ponytail: map webrtc-sys RTCSdpType → crate RTCSdpType inline
fn map_sdp_type(st: webrtc_sys::jsep::ffi::SdpType) -> RTCSdpType {
    match st {
        webrtc_sys::jsep::ffi::SdpType::Offer => RTCSdpType::Offer,
        webrtc_sys::jsep::ffi::SdpType::Answer => RTCSdpType::Answer,
        webrtc_sys::jsep::ffi::SdpType::PrAnswer => RTCSdpType::PrAnswer,
        webrtc_sys::jsep::ffi::SdpType::Rollback => RTCSdpType::Rollback,
        _ => RTCSdpType::Offer, // ponytail: defensive fallback
    }
}

// -- v2: webrtc-sys RtpParameters -> crate RTCRtpParameters mapping --
fn map_rtp_parameters(
    p: webrtc_sys::rtp_parameters::ffi::RtpParameters,
) -> crate::rtp::RTCRtpParameters {
    use crate::rtp::{
        RTCRtcpParameters, RTCRtpCodecParameters, RTCRtpEncodingParameters,
        RTCRtpHeaderExtensionParameters,
    };
    let codecs = p
        .codecs
        .iter()
        .map(|c| RTCRtpCodecParameters {
            mime_type: c.mime_type.clone(),
            payload_type: c.payload_type as u8,
            clock_rate: c.clock_rate.max(0) as u32,
            channels: if c.has_num_channels { Some(c.num_channels as u16) } else { None },
            // PIT-54: mediasoup matches strictly on codec parameters (packetization-mode/profile-level-id)
            // webrtc-sys RtpCodecParameters.parameters (Vec<StringKeyValue>) -> sdp_fmtp_line
            sdp_fmtp_line: if c.parameters.is_empty() {
                None
            } else {
                Some(
                    c.parameters
                        .iter()
                        .map(|kv| format!("{}={}", kv.key, kv.value))
                        .collect::<Vec<_>>()
                        .join(";"),
                )
            },
        })
        .collect();
    let encodings = p
        .encodings
        .iter()
        .map(|e| RTCRtpEncodingParameters {
            ssrc: if e.has_ssrc { Some(e.ssrc as u64) } else { None },
            active: e.active,
            max_bitrate: if e.has_max_bitrate_bps {
                Some(e.max_bitrate_bps.max(0) as u64)
            } else {
                None
            },
            min_bitrate: if e.has_min_bitrate_bps {
                Some(e.min_bitrate_bps.max(0) as u64)
            } else {
                None
            },
            max_framerate: if e.has_max_framerate { Some(e.max_framerate) } else { None },
            scale_resolution_down_by: if e.has_scale_resolution_down_by {
                Some(e.scale_resolution_down_by)
            } else {
                None
            },
            rid: if e.rid.is_empty() { None } else { Some(e.rid.clone()) },
            codec: None,
            dtx: None,
            request_key_frame: e.request_key_frame,
        })
        .collect();
    let header_extensions = p
        .header_extensions
        .iter()
        .map(|h| RTCRtpHeaderExtensionParameters {
            uri: h.uri.clone(),
            id: h.id as u16,
            encrypted: h.encrypt,
        })
        .collect();
    let rtcp =
        RTCRtcpParameters { cname: Some(p.rtcp.cname.clone()), reduced_size: p.rtcp.reduced_size };
    crate::rtp::RTCRtpParameters {
        transaction_id: p.transaction_id,
        mid: p.mid,
        codecs,
        encodings,
        header_extensions,
        rtcp,
    }
}

// -- qos-framerate-priority: crate enum -> vendor enum mapping (private; vendor types never cross the boundary) --
fn map_pref(
    p: crate::rtp::RTCDegradationPreference,
) -> webrtc_sys::rtp_parameters::ffi::DegradationPreference {
    use crate::rtp::RTCDegradationPreference as P;
    use webrtc_sys::rtp_parameters::ffi::DegradationPreference as Sys;
    match p {
        P::Fixed => Sys::MaintainFramerateAndResolution,
        P::MaintainFramerate => Sys::MaintainFramerate,
        P::MaintainResolution => Sys::MaintainResolution,
        P::Balanced => Sys::Balanced,
    }
}

fn map_hint(h: crate::rtp::RTCRtpContentHint) -> webrtc_sys::video_track::ffi::ContentHint {
    use crate::rtp::RTCRtpContentHint as H;
    use webrtc_sys::video_track::ffi::ContentHint as Sys;
    match h {
        H::None => Sys::None,
        H::Fluid => Sys::Fluid,
    }
}

// -- v2: webrtc-sys RtpCapabilities -> crate RTCRtpCapabilities mapping --
fn map_rtp_capabilities(
    c: webrtc_sys::rtp_parameters::ffi::RtpCapabilities,
) -> crate::rtp::RTCRtpCapabilities {
    use crate::rtp::{RTCRtpCodecCapability, RTCRtpHeaderExtensionCapability};
    let codecs = c
        .codecs
        .iter()
        .map(|cc| RTCRtpCodecCapability {
            mime_type: cc.mime_type.clone(),
            clock_rate: if cc.has_clock_rate { Some(cc.clock_rate.max(0) as u32) } else { None },
            channels: if cc.has_num_channels { Some(cc.num_channels as u16) } else { None },
            // v2 (set-codec-preferences T2): fmtp restoration -- reuses map_rtp_parameters:84-91's serialization pattern;
            // libwebrtc matching is exact map equality (MatchesCapability); the round trip must be byte-exact.
            sdp_fmtp_line: if cc.parameters.is_empty() {
                None
            } else {
                Some(
                    cc.parameters
                        .iter()
                        .map(|kv| format!("{}={}", kv.key, kv.value))
                        .collect::<Vec<_>>()
                        .join(";"),
                )
            },
        })
        .collect();
    let header_extensions = c
        .header_extensions
        .iter()
        .map(|h| RTCRtpHeaderExtensionCapability {
            uri: h.uri.clone(),
            id: if h.has_preferred_id { Some(h.preferred_id as u16) } else { None },
        })
        .collect();
    crate::rtp::RTCRtpCapabilities { codecs, header_extensions }
}

/// Parse a "k=v;k=v" fmtp line -> a webrtc-sys StringKeyValue list (the setCodecPreferences input).
fn parse_fmtp_line(line: &str) -> Vec<webrtc_sys::rtp_parameters::ffi::StringKeyValue> {
    line.split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv.trim(), ""));
            webrtc_sys::rtp_parameters::ffi::StringKeyValue {
                key: k.trim().to_string(),
                value: v.trim().to_string(),
            }
        })
        .collect()
}

// -- v2 (set-codec-preferences T1): crate RTCRtpCodecCapability -> webrtc-sys RtpCodecCapability mapping --
// note: webrtc-sys to_native ignores mime_type (proven at rtp_parameters.cpp:47) -> name + kind must be filled explicitly.
fn map_codec_capability_to_sys(
    c: &crate::rtp::RTCRtpCodecCapability,
) -> webrtc_sys::rtp_parameters::ffi::RtpCodecCapability {
    use webrtc_sys::webrtc::ffi::MediaType;
    let (name, kind) = match c.mime_type.split_once('/') {
        Some((_, name)) => {
            let kind =
                if c.mime_type.starts_with("video/") { MediaType::Video } else { MediaType::Audio };
            (name.to_string(), kind)
        }
        // ponytail: a slashless mime is treated as video (callers should pass a full mime)
        None => (c.mime_type.clone(), MediaType::Video),
    };
    let parameters = c.sdp_fmtp_line.as_deref().map(parse_fmtp_line).unwrap_or_default();
    webrtc_sys::rtp_parameters::ffi::RtpCodecCapability {
        mime_type: c.mime_type.clone(),
        name,
        kind,
        has_clock_rate: c.clock_rate.is_some(),
        clock_rate: c.clock_rate.unwrap_or(0) as i32,
        has_preferred_payload_type: false,
        preferred_payload_type: 0,
        has_num_channels: c.channels.is_some(),
        num_channels: c.channels.unwrap_or(0) as i32,
        rtcp_feedback: vec![],
        parameters,
    }
}

impl PcBackend for WebrtcSysPc {
    async fn create_offer(
        &self,
        options: &RTCOfferOptions,
    ) -> Result<RTCSessionDescription, RTCError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ctx = make_ctx(tx);

        let mut opts = webrtc_sys::peer_connection::ffi::RtcOfferAnswerOptions::default();
        // ponytail: ICE restart toggle; full options mapping deferred
        if options.ice_restart {
            opts.ice_restart = true;
        }

        self.pc.create_offer(
            opts,
            ctx,
            |ctx, sdp| {
                let tx: tokio::sync::oneshot::Sender<Result<RTCSessionDescription, RTCError>> =
                    extract_tx(ctx);
                let sdp_type = map_sdp_type(sdp.sdp_type());
                let sdp_str = sdp.stringify();
                let _ = tx.send(Ok(RTCSessionDescription { sdp_type, sdp: sdp_str }));
            },
            |ctx, error| {
                let tx: tokio::sync::oneshot::Sender<Result<RTCSessionDescription, RTCError>> =
                    extract_tx(ctx);
                let _ = tx.send(Err(RTCError::Internal(error.message)));
            },
        );

        rx.await.map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
    }

    async fn create_answer(
        &self,
        _options: &RTCAnswerOptions,
    ) -> Result<RTCSessionDescription, RTCError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ctx = make_ctx(tx);

        let opts = webrtc_sys::peer_connection::ffi::RtcOfferAnswerOptions::default();
        // ponytail: RTCAnswerOptions has no fields currently, pass defaults

        self.pc.create_answer(
            opts,
            ctx,
            |ctx, sdp| {
                let tx: tokio::sync::oneshot::Sender<Result<RTCSessionDescription, RTCError>> =
                    extract_tx(ctx);
                let sdp_type = map_sdp_type(sdp.sdp_type());
                let sdp_str = sdp.stringify();
                let _ = tx.send(Ok(RTCSessionDescription { sdp_type, sdp: sdp_str }));
            },
            |ctx, error| {
                let tx: tokio::sync::oneshot::Sender<Result<RTCSessionDescription, RTCError>> =
                    extract_tx(ctx);
                let _ = tx.send(Err(RTCError::Internal(error.message)));
            },
        );

        rx.await.map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
    }

    async fn set_local_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ctx = make_ctx(tx);

        let sdp_type = match desc.sdp_type {
            RTCSdpType::Offer => webrtc_sys::jsep::ffi::SdpType::Offer,
            RTCSdpType::Answer => webrtc_sys::jsep::ffi::SdpType::Answer,
            RTCSdpType::PrAnswer => webrtc_sys::jsep::ffi::SdpType::PrAnswer,
            RTCSdpType::Rollback => webrtc_sys::jsep::ffi::SdpType::Rollback,
        };

        let sd = webrtc_sys::jsep::ffi::create_session_description(sdp_type, desc.sdp.clone())
            .map_err(|e| RTCError::Sdp(e.what().to_owned()))?;

        // ponytail: set_local_description has a single on_complete callback (ctx, error)
        self.pc.set_local_description(sd, ctx, |ctx, error| {
            let tx: tokio::sync::oneshot::Sender<Result<(), RTCError>> = extract_tx(ctx);
            if error.ok() {
                let _ = tx.send(Ok(()));
            } else {
                let _ = tx.send(Err(RTCError::Sdp(error.message)));
            }
        });

        let _result = rx.await.map_err(|_| RTCError::Internal("oneshot cancelled".into()))?;

        *self.local_sdp.lock().unwrap() = Some(desc.sdp.clone());

        _result
    }

    async fn set_remote_description(&self, desc: &RTCSessionDescription) -> Result<(), RTCError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ctx = make_ctx(tx);

        let sdp_type = match desc.sdp_type {
            RTCSdpType::Offer => webrtc_sys::jsep::ffi::SdpType::Offer,
            RTCSdpType::Answer => webrtc_sys::jsep::ffi::SdpType::Answer,
            RTCSdpType::PrAnswer => webrtc_sys::jsep::ffi::SdpType::PrAnswer,
            RTCSdpType::Rollback => webrtc_sys::jsep::ffi::SdpType::Rollback,
        };

        let sd = webrtc_sys::jsep::ffi::create_session_description(sdp_type, desc.sdp.clone())
            .map_err(|e| RTCError::Sdp(e.what().to_owned()))?;

        self.pc.set_remote_description(sd, ctx, |ctx, error| {
            let tx: tokio::sync::oneshot::Sender<Result<(), RTCError>> = extract_tx(ctx);
            if error.ok() {
                let _ = tx.send(Ok(()));
            } else {
                let _ = tx.send(Err(RTCError::Sdp(error.message)));
            }
        });

        rx.await.map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
    }

    async fn add_ice_candidate(&self, candidate: &RTCIceCandidate) -> Result<(), RTCError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ctx = make_ctx(tx);

        let ic = webrtc_sys::jsep::ffi::create_ice_candidate(
            candidate.sdp_mid.clone().unwrap_or_default(),
            candidate.sdp_mline_index.map(|v| v as i32).unwrap_or(0),
            candidate.candidate.clone(),
        )
        .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;

        self.pc.add_ice_candidate(ic, ctx, |ctx, error| {
            let tx: tokio::sync::oneshot::Sender<Result<(), RTCError>> = extract_tx(ctx);
            if error.ok() {
                let _ = tx.send(Ok(()));
            } else {
                let _ = tx.send(Err(RTCError::RTCPeerConnection(error.message)));
            }
        });

        rx.await.map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
    }

    fn connection_state(&self) -> RTCPeerConnectionState {
        match self.pc.connection_state() {
            webrtc_sys::peer_connection::ffi::PeerConnectionState::New => {
                RTCPeerConnectionState::New
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Connecting => {
                RTCPeerConnectionState::Connecting
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Connected => {
                RTCPeerConnectionState::Connected
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Disconnected => {
                RTCPeerConnectionState::Disconnected
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Failed => {
                RTCPeerConnectionState::Failed
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Closed => {
                RTCPeerConnectionState::Closed
            }
            _ => RTCPeerConnectionState::New, // ponytail: defensive fallback
        }
    }

    fn ice_connection_state(&self) -> RTCIceConnectionState {
        match self.pc.ice_connection_state() {
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionNew => {
                RTCIceConnectionState::New
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionChecking => {
                RTCIceConnectionState::Checking
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionConnected => {
                RTCIceConnectionState::Connected
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionCompleted => {
                RTCIceConnectionState::Completed
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionFailed => {
                RTCIceConnectionState::Failed
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionDisconnected => {
                RTCIceConnectionState::Disconnected
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionClosed => {
                RTCIceConnectionState::Closed
            }
            _ => RTCIceConnectionState::New, // ponytail: defensive fallback
        }
    }

    fn ice_gathering_state(&self) -> RTCIceGatheringState {
        match self.pc.ice_gathering_state() {
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringNew => {
                RTCIceGatheringState::New
            }
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringGathering => {
                RTCIceGatheringState::Gathering
            }
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringComplete => {
                RTCIceGatheringState::Complete
            }
            _ => RTCIceGatheringState::New, // ponytail: defensive fallback
        }
    }

    fn signaling_state(&self) -> RTCSignalingState {
        match self.pc.signaling_state() {
            webrtc_sys::peer_connection::ffi::SignalingState::Stable => RTCSignalingState::Stable,
            webrtc_sys::peer_connection::ffi::SignalingState::HaveLocalOffer => {
                RTCSignalingState::HaveLocalOffer
            }
            webrtc_sys::peer_connection::ffi::SignalingState::HaveLocalPrAnswer => {
                RTCSignalingState::HaveLocalPrAnswer
            }
            webrtc_sys::peer_connection::ffi::SignalingState::HaveRemoteOffer => {
                RTCSignalingState::HaveRemoteOffer
            }
            webrtc_sys::peer_connection::ffi::SignalingState::HaveRemotePrAnswer => {
                RTCSignalingState::HaveRemotePrAnswer
            }
            webrtc_sys::peer_connection::ffi::SignalingState::Closed => RTCSignalingState::Closed,
            _ => RTCSignalingState::Stable, // ponytail: defensive fallback
        }
    }

    async fn close(&self) {
        self.pc.close();
    }
    /// Override: store the on_track callback so RealObserver can invoke it.
    fn set_on_track(&self, cb: Box<dyn Fn(TrackReceiver) + Send + Sync + 'static>) {
        *self.callbacks.on_track.lock().unwrap() = Some(cb);
    }

    fn set_on_data_channel(
        &self,
        cb: Box<dyn Fn(crate::data_channel::RTCDataChannel) + Send + Sync + 'static>,
    ) {
        *self.callbacks.on_data_channel.lock().unwrap() = Some(cb);
    }

    fn set_on_ice_connection_state_change(
        &self,
        cb: Box<dyn Fn(RTCIceConnectionState) + Send + Sync + 'static>,
    ) {
        *self.callbacks.on_ice_connection_state_change.lock().unwrap() = Some(cb);
    }

    fn set_on_peer_connection_state_change(
        &self,
        cb: Box<dyn Fn(RTCPeerConnectionState) + Send + Sync + 'static>,
    ) {
        *self.callbacks.on_peer_connection_state_change.lock().unwrap() = Some(cb);
    }

    fn set_on_ice_candidate(&self, cb: Box<dyn Fn(RTCIceCandidate) + Send + Sync + 'static>) {
        *self.callbacks.on_ice_candidate.lock().unwrap() = Some(cb);
    }

    fn local_description_sdp(&self) -> Option<String> {
        self.local_sdp.lock().unwrap().clone()
    }

    /// Override: consume a staged media track and add it to libwebrtc.
    fn register_track(&self, _track_id: &str, _kind: TrackKind) -> Result<(), RTCError> {
        let mut guard = self.callbacks.staged_media_tracks.lock().unwrap();
        while let Some(media_track) = guard.pop() {
            let _ = self.pc.add_track(media_track, &vec![]);
        }
        Ok(())
    }

    // ── v2: W3C API overrides ──

    fn get_transceivers(&self) -> Result<Vec<crate::rtp::RTCRtpTransceiver>, RTCError> {
        use webrtc_sys::rtp_transceiver::ffi::RtpTransceiverDirection as SysDir;
        let tcs = self.pc.get_transceivers();
        let mut out = Vec::with_capacity(tcs.len());
        for tc in tcs {
            let t = &tc.ptr;
            let kind = match t.media_type() {
                webrtc_sys::webrtc::ffi::MediaType::Video => TrackKind::Video,
                webrtc_sys::webrtc::ffi::MediaType::Audio => TrackKind::Audio,
                _ => TrackKind::Video,
            };
            let direction = match t.direction() {
                SysDir::SendRecv => crate::rtp::RTCRtpTransceiverDirection::Sendrecv,
                SysDir::SendOnly => crate::rtp::RTCRtpTransceiverDirection::Sendonly,
                SysDir::RecvOnly => crate::rtp::RTCRtpTransceiverDirection::Recvonly,
                SysDir::Inactive => crate::rtp::RTCRtpTransceiverDirection::Inactive,
                SysDir::Stopped => crate::rtp::RTCRtpTransceiverDirection::Inactive,
                _ => crate::rtp::RTCRtpTransceiverDirection::Inactive,
            };
            let current = t.current_direction().ok().map(|d| match d {
                SysDir::SendRecv => crate::rtp::RTCRtpTransceiverDirection::Sendrecv,
                SysDir::SendOnly => crate::rtp::RTCRtpTransceiverDirection::Sendonly,
                SysDir::RecvOnly => crate::rtp::RTCRtpTransceiverDirection::Recvonly,
                SysDir::Inactive => crate::rtp::RTCRtpTransceiverDirection::Inactive,
                SysDir::Stopped => crate::rtp::RTCRtpTransceiverDirection::Inactive,
                _ => crate::rtp::RTCRtpTransceiverDirection::Inactive,
            });
            let stopped = t.stopped();
            let mid = t.mid().ok();
            let sender = crate::rtp::RTCRtpSender::new(crate::track::TrackRef::Sender(
                crate::track::TrackSender::new(
                    format!("sys-sender-{}", mid.as_deref().unwrap_or("")),
                    kind,
                ),
            ));
            let receiver = crate::rtp::RTCRtpReceiver::new(crate::track::TrackRef::Receiver(
                crate::track::TrackReceiver::new(
                    format!("sys-recv-{}", mid.as_deref().unwrap_or("")),
                    kind,
                ),
            ));
            out.push(crate::rtp::RTCRtpTransceiver::new(
                mid, direction, current, stopped, sender, receiver, kind,
            ));
        }
        Ok(out)
    }

    /// W3C addTransceiver(kind, init) -- the no-track variant (required by the consume side's recvonly).
    /// libwebrtc's native AddTransceiver(MediaType, init) supports pure-receive recvonly.
    fn add_transceiver(
        &self,
        kind: crate::track::TrackKind,
        init: &crate::rtp::RTCRtpTransceiverInit,
    ) -> Result<crate::rtp::RTCRtpTransceiver, RTCError> {
        use webrtc_sys::rtp_transceiver::ffi::RtpTransceiverDirection as SysDir;
        use webrtc_sys::webrtc::ffi::MediaType;
        let sys_dir = match init.direction {
            crate::rtp::RTCRtpTransceiverDirection::Sendrecv => SysDir::SendRecv,
            crate::rtp::RTCRtpTransceiverDirection::Sendonly => SysDir::SendOnly,
            crate::rtp::RTCRtpTransceiverDirection::Recvonly => SysDir::RecvOnly,
            crate::rtp::RTCRtpTransceiverDirection::Inactive => SysDir::Inactive,
        };
        let media_type = match kind {
            crate::track::TrackKind::Video => MediaType::Video,
            crate::track::TrackKind::Audio => MediaType::Audio,
        };
        let sys_init = webrtc_sys::rtp_transceiver::ffi::RtpTransceiverInit {
            direction: sys_dir,
            stream_ids: init.stream_ids.clone(),
            send_encodings: vec![],
        };
        let tc = self
            .pc
            .add_transceiver_for_media(media_type, sys_init)
            .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
        let mid = tc.mid().ok();
        let kind2 = kind;
        let receiver = crate::rtp::RTCRtpReceiver::new(crate::track::TrackRef::Receiver(
            crate::track::TrackReceiver::new(
                format!("sys-recv-{}", mid.as_deref().unwrap_or("")),
                kind2,
            ),
        ));
        // no track -> no sender
        let sender = crate::rtp::RTCRtpSender::new(crate::track::TrackRef::Receiver(
            crate::track::TrackReceiver::new(
                format!("sys-send-{}", mid.as_deref().unwrap_or("")),
                kind2,
            ),
        ));
        Ok(crate::rtp::RTCRtpTransceiver::new(
            mid,
            init.direction,
            Some(init.direction),
            false,
            sender,
            receiver,
            kind2,
        ))
    }

    fn add_transceiver_with_track(
        &self,
        track: &crate::track::TrackSender,
        init: &crate::rtp::RTCRtpTransceiverInit,
    ) -> Result<crate::rtp::RTCRtpTransceiver, RTCError> {
        use webrtc_sys::rtp_transceiver::ffi::RtpTransceiverDirection as SysDir;
        // take the media_track out of the staged queue (staged at create_track_sender time)
        let media_track =
            self.callbacks.staged_media_tracks.lock().unwrap().pop().ok_or_else(|| {
                RTCError::Track("no staged media track for add_transceiver_with_track".into())
            })?;
        let sys_dir = match init.direction {
            crate::rtp::RTCRtpTransceiverDirection::Sendrecv => SysDir::SendRecv,
            crate::rtp::RTCRtpTransceiverDirection::Sendonly => SysDir::SendOnly,
            crate::rtp::RTCRtpTransceiverDirection::Recvonly => SysDir::RecvOnly,
            crate::rtp::RTCRtpTransceiverDirection::Inactive => SysDir::Inactive,
        };
        let sys_init = webrtc_sys::rtp_transceiver::ffi::RtpTransceiverInit {
            direction: sys_dir,
            stream_ids: init.stream_ids.clone(),
            send_encodings: vec![],
        };
        let tc = self
            .pc
            .add_transceiver(media_track, sys_init)
            .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
        let mid = tc.mid().ok();
        let sender = crate::rtp::RTCRtpSender::new(crate::track::TrackRef::Sender(track.clone()));
        let receiver = crate::rtp::RTCRtpReceiver::new(crate::track::TrackRef::Receiver(
            crate::track::TrackReceiver::new(
                format!("sys-recv-{}", mid.as_deref().unwrap_or("")),
                track.kind,
            ),
        ));
        Ok(crate::rtp::RTCRtpTransceiver::new(
            mid,
            init.direction,
            Some(init.direction),
            false,
            sender,
            receiver,
            track.kind,
        ))
    }

    fn sender_get_parameters(
        &self,
        track_id: &str,
    ) -> Result<crate::rtp::RTCRtpParameters, RTCError> {
        // iterate the transceivers, matching on the sender's track_id
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                return Ok(map_rtp_parameters(sender.get_parameters()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// PIT-76: periodic keyframe triggering -- a cxx-faithful round trip (overriding the default impl).
    ///
    /// Must go through the cxx verbatim round trip (not the upper map_rtp_parameters mapping):
    /// libwebrtc's RtpSenderBase::SetParameters validates that codecs / encodings counts /
    /// transaction_id match its internal storage; the upper mapping drops codec name/kind/rtcp_feedback,
    /// so a rebuild is always INVALID_MODIFICATION. get verbatim -> change only request_key_frame -> set;
    /// all three validations naturally hold. libwebrtc clears the flag internally after each consumption: passing true per call
    /// triggers exactly one GenerateKeyFrame (Oracle confirmed, no reset needed).
    fn request_key_frame(&self, track_id: &str) -> Result<(), RTCError> {
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                let mut params = sender.get_parameters();
                for enc in &mut params.encodings {
                    enc.request_key_frame = true;
                }
                return sender
                    .set_parameters(params)
                    .map_err(|e| RTCError::Internal(e.what().to_owned()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }
    /// v2 (encoder-backend-codec-config T1): encoder backend selection -- the SetEncoderSelector mechanism.
    /// Iterate the transceivers matching sender.track().id() (the same pattern as request_key_frame).
    fn sender_set_video_encoder_backend(
        &self,
        track_id: &str,
        backend: crate::rtp::RTCVideoEncoderBackend,
    ) -> Result<(), RTCError> {
        let sys_backend = match backend {
            crate::rtp::RTCVideoEncoderBackend::Auto => {
                webrtc_sys::webrtc::ffi::VideoEncoderBackend::Auto
            }
            crate::rtp::RTCVideoEncoderBackend::Software => {
                webrtc_sys::webrtc::ffi::VideoEncoderBackend::Software
            }
            crate::rtp::RTCVideoEncoderBackend::Hardware => {
                webrtc_sys::webrtc::ffi::VideoEncoderBackend::Hardware
            }
            crate::rtp::RTCVideoEncoderBackend::Nvenc => {
                webrtc_sys::webrtc::ffi::VideoEncoderBackend::Nvenc
            }
            crate::rtp::RTCVideoEncoderBackend::Vaapi => {
                webrtc_sys::webrtc::ffi::VideoEncoderBackend::Vaapi
            }
            crate::rtp::RTCVideoEncoderBackend::VideoToolbox => {
                webrtc_sys::webrtc::ffi::VideoEncoderBackend::VideoToolbox
            }
            crate::rtp::RTCVideoEncoderBackend::PreEncoded => {
                webrtc_sys::webrtc::ffi::VideoEncoderBackend::PreEncoded
            }
        };
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                sender.set_video_encoder_backend(sys_backend);
                tracing::info!(
                    "sender_set_video_encoder_backend({track_id}, {backend:?}) — SetEncoderSelector"
                );
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }
    /// v2 (encoder-bitrate): min/max bitrate setting -- a cxx-faithful round trip (the same pattern as request_key_frame).
    /// Only touches the enc[].min/max_bitrate_bps fields, naturally satisfying SetParameters' codecs/encodings
    /// count/transaction_id validation (PIT-76: lossy roundtrips forbidden).
    /// min = the best-effort floor on constrained links (effective at the libwebrtc allocation layer, no hard encoder floor);
    /// max = the reliable hard ceiling. None -> has_*_bitrate_bps=false (unlimited).
    fn sender_set_encoding_bitrate(
        &self,
        track_id: &str,
        min_bps: Option<u64>,
        max_bps: Option<u64>,
    ) -> Result<(), RTCError> {
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                let mut params = sender.get_parameters();
                for enc in &mut params.encodings {
                    enc.has_min_bitrate_bps = min_bps.is_some();
                    enc.min_bitrate_bps = min_bps.unwrap_or(0) as i32;
                    enc.has_max_bitrate_bps = max_bps.is_some();
                    enc.max_bitrate_bps = max_bps.unwrap_or(0) as i32;
                }
                tracing::info!(
                    "sender_set_encoding_bitrate({track_id}, min={min_bps:?}, max={max_bps:?}) — SetParameters"
                );
                return sender
                    .set_parameters(params)
                    .map_err(|e| RTCError::Internal(e.what().to_owned()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// v2 (qos-framerate-priority): degradation preference -- a cxx-faithful round trip (the same pattern as bitrate).
    /// degradation_preference sits at the **RtpParameters level** (vendor rtp_parameters.rs:191-192,
    /// not per-enc): get verbatim -> the has/preference two fields -> set, naturally satisfying SetParameters'
    /// validation (PIT-76). Fixed -> vendor MaintainFramerateAndResolution.
    fn sender_set_degradation_preference(
        &self,
        track_id: &str,
        pref: crate::rtp::RTCDegradationPreference,
    ) -> Result<(), RTCError> {
        let sys_pref = map_pref(pref);
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                let mut params = sender.get_parameters();
                params.has_degradation_preference = true;
                params.degradation_preference = sys_pref;
                tracing::info!(
                    "sender_set_degradation_preference({track_id}, {pref:?}) — SetParameters"
                );
                return sender
                    .set_parameters(params)
                    .map_err(|e| RTCError::Internal(e.what().to_owned()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// v2 (qos-framerate-priority): content hint -- a track-level property (not SetParameters).
    /// sender.track() -> media_to_video downcast (the :1444 VideoSink precedent's method) -> set_content_hint;
    /// a non-video track failing the downcast -> Err (the caller warns, C15).
    fn sender_set_content_hint(
        &self,
        track_id: &str,
        hint: crate::rtp::RTCRtpContentHint,
    ) -> Result<(), RTCError> {
        let sys_hint = map_hint(hint);
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                // a track_id hit = the video track built by the caller's add_track("video") (guaranteed by the publish path);
                // downcasting a non-video track would be undefined behavior, so only the video-kind path calls it (the :1490 VideoSink precedent's method).
                unsafe {
                    let video_track = webrtc_sys::video_track::ffi::media_to_video(track);
                    video_track.set_content_hint(sys_hint);
                }
                tracing::info!("sender_set_content_hint({track_id}, {hint:?}) — VideoTrack");
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// v2 (multi-stream P1): the encoded-framerate cap (fps) -- a cxx-faithful round trip (the same pattern as bitrate).
    /// Only touches the enc[].has_max_framerate/max_framerate fields, naturally satisfying SetParameters'
    /// codecs/encodings count/transaction_id validation (PIT-76).
    /// libwebrtc reconfigures the encoder upon receiving SetParameters (official behavior);
    /// frame timestamps are anchored monotonically per C17 (independent of fps). None -> has_max_framerate=false (unlimited).
    fn sender_set_encoding_framerate(
        &self,
        track_id: &str,
        max_fps: Option<f64>,
    ) -> Result<(), RTCError> {
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                let mut params = sender.get_parameters();
                for enc in &mut params.encodings {
                    enc.has_max_framerate = max_fps.is_some();
                    enc.max_framerate = max_fps.unwrap_or(0.0);
                }
                tracing::info!(
                    "sender_set_encoding_framerate({track_id}, max_fps={max_fps:?}) — SetParameters"
                );
                return sender
                    .set_parameters(params)
                    .map_err(|e| RTCError::Internal(e.what().to_owned()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// v2 (web-stream-stats T1.5): W3C RTCRtpSender.getStats (outbound stats).
    /// Calls the webrtc-sys FFI (RtpSender::get_stats -> ToJson, a synchronous callback) -> parses the outbound-rtp fields.
    /// Pure Rust with zero C++ changes (Oracle F2: libwebrtc's ToJson already carries framesEncoded/encoderImplementation etc.).
    fn receiver_get_stats(&self, track_id: &str) -> Vec<crate::stats::RTCStats> {
        for tc in self.pc.get_transceivers() {
            let receiver = tc.ptr.receiver();
            if receiver.track().id() != track_id {
                continue;
            }
            let (tx, rx) = std::sync::mpsc::sync_channel::<String>(1);
            let ctx = Box::new(webrtc_sys::rtp_receiver::ReceiverContext(Box::new(tx)));
            receiver.get_stats(ctx, |ctx, json| {
                // synchronous callback (the C++ OnStatsDelivered same thread) -- the mpsc delivers immediately (the sender's same shape).
                if let Some(tx) = ctx.0.downcast_ref::<std::sync::mpsc::SyncSender<String>>() {
                    let _ = tx.send(json);
                }
            });
            return match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(json) => parse_inbound_stats_json(&json),
                Err(_) => vec![],
            };
        }
        vec![]
    }

    /// qoe-full-metrics S1: the pc-level getStats full table -> candidate-pair(succeeded)'s
    /// currentRoundTripTime (seconds -> ms). nominated first, else the first succeeded.
    fn connection_stats(&self) -> Option<crate::stats::ConnectionStats> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<String>(1);
        let ctx = Box::new(webrtc_sys::peer_connection::PeerContext(Box::new(tx)));
        self.pc.get_stats(ctx, |ctx, json| {
            if let Some(tx) = ctx.0.downcast_ref::<std::sync::mpsc::SyncSender<String>>() {
                let _ = tx.send(json);
            }
        });
        let json = rx.recv_timeout(std::time::Duration::from_millis(200)).ok()?;
        let arr = serde_json::from_str::<Vec<serde_json::Value>>(&json).ok()?;
        // the RTT convention (measured 2026-10-10): when client is an ICE-lite passive end
        // currentRoundTripTime is always 0 (no request sent, no response counted) -- the proven usable truth =
        // totalRoundTripTime / responsesReceived (the average RTT, the same convention as libwebrtc internally).
        // currentRoundTripTime preferred (the standard active end), falling back to the average convention.
        let mut best: Option<crate::stats::ConnectionStats> = None;
        for v in arr.iter() {
            if v.get("type").and_then(|t| t.as_str()) != Some("candidate-pair") {
                continue;
            }
            if v.get("state").and_then(|x| x.as_str()) != Some("succeeded") {
                continue;
            }
            let rtt_s = v
                .get("currentRoundTripTime")
                .and_then(|x| x.as_f64())
                .filter(|r| *r > 0.0)
                .or_else(|| {
                    let total = v.get("totalRoundTripTime").and_then(|x| x.as_f64())?;
                    let n = v.get("responsesReceived").and_then(|x| x.as_u64())?.max(1);
                    (total > 0.0).then(|| total / n as f64)
                });
            let Some(rtt) = rtt_s else { continue };
            // ceil: sub-millisecond intranet rounds (0.2ms scale) would wipe to 0 -- show at least 1ms
            let c = crate::stats::ConnectionStats { rtt_ms: (rtt * 1000.0).ceil() };
            if v.get("nominated").and_then(|x| x.as_bool()) == Some(true) {
                return Some(c);
            }
            best = best.or(Some(c));
        }
        best
    }

    fn sender_get_stats(&self, track_id: &str) -> Vec<crate::stats::RTCStats> {
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                let (tx, rx) = std::sync::mpsc::sync_channel::<String>(1);
                let ctx = Box::new(webrtc_sys::rtp_sender::SenderContext(Box::new(tx)));
                sender.get_stats(ctx, |ctx, json| {
                    // synchronous callback (the C++ OnStatsDelivered same thread) -- the mpsc delivers immediately
                    if let Some(tx) = ctx.0.downcast_ref::<std::sync::mpsc::SyncSender<String>>() {
                        let _ = tx.send(json);
                    }
                });
                return match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(json) => parse_outbound_stats_json(&json),
                    Err(_) => vec![],
                };
            }
        }
        vec![]
    }

    fn get_sender_capabilities(
        &self,
        kind: TrackKind,
    ) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError> {
        let media_type = match kind {
            TrackKind::Video => webrtc_sys::webrtc::ffi::MediaType::Video,
            TrackKind::Audio => webrtc_sys::webrtc::ffi::MediaType::Audio,
        };
        Ok(Some(map_rtp_capabilities(self.factory.rtp_sender_capabilities(media_type))))
    }

    fn get_receiver_capabilities(
        &self,
        kind: TrackKind,
    ) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError> {
        let media_type = match kind {
            TrackKind::Video => webrtc_sys::webrtc::ffi::MediaType::Video,
            TrackKind::Audio => webrtc_sys::webrtc::ffi::MediaType::Audio,
        };
        Ok(Some(map_rtp_capabilities(self.factory.rtp_receiver_capabilities(media_type))))
    }

    fn restart_ice(&self) -> Result<(), RTCError> {
        self.pc.restart_ice();
        Ok(())
    }

    fn current_local_description(
        &self,
    ) -> Result<Option<crate::sdp::RTCSessionDescription>, RTCError> {
        let sd = self.pc.current_local_description();
        if sd.is_null() {
            return Ok(None);
        }
        Ok(Some(crate::sdp::RTCSessionDescription::new(
            map_sdp_type(sd.sdp_type()),
            sd.stringify(),
        )))
    }

    fn current_remote_description(
        &self,
    ) -> Result<Option<crate::sdp::RTCSessionDescription>, RTCError> {
        let sd = self.pc.current_remote_description();
        if sd.is_null() {
            return Ok(None);
        }
        Ok(Some(crate::sdp::RTCSessionDescription::new(
            map_sdp_type(sd.sdp_type()),
            sd.stringify(),
        )))
    }

    fn transceiver_set_direction(
        &self,
        mid: &str,
        dir: crate::rtp::RTCRtpTransceiverDirection,
    ) -> Result<(), RTCError> {
        use webrtc_sys::rtp_transceiver::ffi::RtpTransceiverDirection as SysDir;
        let sys_dir = match dir {
            crate::rtp::RTCRtpTransceiverDirection::Sendrecv => SysDir::SendRecv,
            crate::rtp::RTCRtpTransceiverDirection::Sendonly => SysDir::SendOnly,
            crate::rtp::RTCRtpTransceiverDirection::Recvonly => SysDir::RecvOnly,
            crate::rtp::RTCRtpTransceiverDirection::Inactive => SysDir::Inactive,
        };
        for tc in self.pc.get_transceivers() {
            if tc.ptr.mid().ok().as_deref() == Some(mid) {
                tc.ptr
                    .set_direction(sys_dir)
                    .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("transceiver not found: {mid}")))
    }

    fn transceiver_stop(&self, mid: &str) -> Result<(), RTCError> {
        for tc in self.pc.get_transceivers() {
            if tc.ptr.mid().ok().as_deref() == Some(mid) {
                tc.ptr
                    .stop_standard()
                    .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("transceiver not found: {mid}")))
    }

    /// v2 (set-codec-preferences T3+T5 field-proven revision): W3C setCodecPreferences.
    /// Matches a transceiver by sender.track().id() (mid doesn't exist pre-negotiation -- the offerer's core scenario;
    /// the same pattern as request_key_frame). Setting it post-negotiation on the answerer has no effect on the answer (libwebrtc
    /// takes the intersection in offer order) -- the T5 measured conclusion; fixed codecs go through reduceCodecs.
    fn transceiver_set_codec_preferences(
        &self,
        track_id: &str,
        codecs: Vec<crate::rtp::RTCRtpCodecCapability>,
    ) -> Result<(), RTCError> {
        let sys_codecs = codecs.iter().map(map_codec_capability_to_sys).collect::<Vec<_>>();
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                t.set_codec_preferences(sys_codecs)
                    .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }
}

// ── create_data_channel (method on WebrtcSysPc, called directly by peer.rs) ──

impl WebrtcSysPc {
    /// Stage a media track to be consumed by the next register_track call.
    pub(crate) fn stage_media_track(
        &self,
        track: cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>,
    ) {
        self.callbacks.staged_media_tracks.lock().unwrap().push(track);
    }

    pub(crate) async fn create_data_channel(
        &self,
        label: &str,
        init: crate::data_channel::RTCDataChannelInit,
    ) -> Result<crate::data_channel::RTCDataChannel, RTCError> {
        use crate::data_channel::RTCDataChannel;

        let sys_init = webrtc_sys::data_channel::ffi::DataChannelInit {
            ordered: init.ordered,
            has_max_retransmit_time: init.max_retransmit_time.is_some(),
            max_retransmit_time: init.max_retransmit_time.unwrap_or(-1),
            has_max_retransmits: init.max_retransmits.is_some(),
            max_retransmits: init.max_retransmits.unwrap_or(-1),
            protocol: init.protocol,
            negotiated: init.negotiated,
            id: init.id,
            has_priority: false,
            priority: webrtc_sys::data_channel::ffi::Priority::Low,
        };

        let dc = self
            .pc
            .create_data_channel(label.to_string(), sys_init)
            .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;

        Ok(RTCDataChannel {
            label: label.to_string(),
            id: dc.id(),
            backend: WebrtcSysDc { dc, rx: std::sync::Arc::new(std::sync::OnceLock::new()) },
        })
    }
}

// ── WebrtcSysDc ──

#[derive(Clone)]
pub(crate) struct WebrtcSysDc {
    dc: cxx::SharedPtr<webrtc_sys::data_channel::ffi::DataChannel>,
    /// The receive-observer cache (F1 review #1): register-once -- the first spool() registers the observer and
    /// stores the broadcast Sender; later spool() calls only subscribe, never re-registering.
    /// Clone shares the same Arc (the same underlying DataChannel).
    rx: std::sync::Arc<
        std::sync::OnceLock<
            tokio::sync::broadcast::Sender<crate::data_channel::RTCDataChannelEvent>,
        >,
    >,
}

impl std::fmt::Debug for WebrtcSysDc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcSysDc").finish()
    }
}

impl DcBackend for WebrtcSysDc {
    fn state(&self) -> RTCDataChannelState {
        match self.dc.state() {
            webrtc_sys::data_channel::ffi::DataState::Connecting => RTCDataChannelState::Connecting,
            webrtc_sys::data_channel::ffi::DataState::Open => RTCDataChannelState::Open,
            webrtc_sys::data_channel::ffi::DataState::Closing => RTCDataChannelState::Closing,
            webrtc_sys::data_channel::ffi::DataState::Closed => RTCDataChannelState::Closed,
            _ => RTCDataChannelState::Closed, // ponytail: defensive fallback
        }
    }

    /// S6/K5: libwebrtc's native synchronous read (the FFI already carries buffered_amount; this adds the last wrapper segment).
    async fn buffered_amount(&self) -> u64 {
        self.dc.buffered_amount()
    }

    async fn send(&self, data: &[u8]) -> Result<(), RTCError> {
        let buf = webrtc_sys::data_channel::ffi::DataBuffer {
            ptr: data.as_ptr(),
            len: data.len(),
            binary: true,
        };
        // ponytail: webrtc-sys send may fail if channel not open; log and ignore
        self.dc.send(&buf);
        Ok(())
    }

    async fn send_text(&self, text: &str) -> Result<(), RTCError> {
        let buf = webrtc_sys::data_channel::ffi::DataBuffer {
            ptr: text.as_ptr(),
            len: text.len(),
            binary: false,
        };
        self.dc.send(&buf);
        Ok(())
    }

    async fn spool(&self) -> RTCDataChannelRx {
        // F1 review #1 (the FFI thread constraint): libwebrtc requires RegisterObserver/
        // UnregisterObserver to run on its internal signaling thread (DCHECK_RUN_ON, compiled out in release
        // leaving a silent data race), and each re-registration resets the previous observer (unique_ptr) --
        // the network thread may be mid-OnMessage -> rust::Box use-after-free.
        // hence **register exactly once per channel** (the first spool(), OnceLock-guaranteed; upstream livekit
        // registers once in configure() the same way); later spool() calls only subscribe the cached event stream.
        // the registering thread may still be the first caller (a tokio worker) -- fully satisfying the DCHECK would need
        // scheduling registration onto the signaling thread (livekit's configure moment IS the signaling thread); here
        // register-once eliminates the re-registration UAF; cross-thread registration governance is deferred to a dedicated task.
        let tx = self.rx.get_or_init(|| {
            let (tx, _rx) = tokio::sync::broadcast::channel(256);
            let observer = std::sync::Arc::new(DcObserver { tx: tx.clone() });
            let wrapper = webrtc_sys::data_channel::DataChannelObserverWrapper::new(observer);
            self.dc.register_observer(Box::new(wrapper));
            tx
        });
        RTCDataChannelRx::new(Some(tx.subscribe()))
    }

    async fn close(&mut self) {
        self.dc.close();
    }
}

/// The libwebrtc DataChannel observer: forwards messages/state to broadcast (Task F1's receive path).
/// Callbacks fire on the libwebrtc signaling thread; only a lock-free send, returning quickly.
/// note: the observer Box is handed to the C++ side (register_observer); this struct only lends
/// Sender clones -- the observer's lifetime = the C++ unique_ptr, never freed by the Rust side.
struct DcObserver {
    tx: tokio::sync::broadcast::Sender<crate::data_channel::RTCDataChannelEvent>,
}

impl webrtc_sys::data_channel::DataChannelObserver for DcObserver {
    fn on_state_change(&self, state: webrtc_sys::data_channel::ffi::DataState) {
        let ev = match state {
            webrtc_sys::data_channel::ffi::DataState::Open => {
                Some(crate::data_channel::RTCDataChannelEvent::Open)
            }
            webrtc_sys::data_channel::ffi::DataState::Closed
            | webrtc_sys::data_channel::ffi::DataState::Closing => {
                Some(crate::data_channel::RTCDataChannelEvent::Closed)
            }
            webrtc_sys::data_channel::ffi::DataState::Connecting => None,
            // ponytail: the cxx non-exhaustive enum's unknown-state fallback (same as the state() convention)
            _ => None,
        };
        if let Some(ev) = ev {
            let _ = self.tx.send(ev);
        }
    }

    fn on_message(&self, data: &[u8], _is_binary: bool) {
        let _ = self.tx.send(crate::data_channel::RTCDataChannelEvent::Message(
            crate::data_channel::RTCDataMessage { data: data.to_vec() },
        ));
    }

    fn on_buffered_amount_change(&self, _sent_data_size: u64) {}
}

// ── WebrtcSysTrack ──

// ── WebrtcSysTrack ──

/// webrtc-sys media track backend.
/// Holds a libwebrtc VideoTrackSource (raw I420 push) and/or an
/// AudioTrackSource (PCM i16 push — H2; libwebrtc encodes to opus internally).
pub(crate) struct WebrtcSysTrack {
    video_source: Mutex<Option<SharedPtr<webrtc_sys::video_track::ffi::VideoTrackSource>>>,
    audio_source: Mutex<Option<SharedPtr<webrtc_sys::audio_track::ffi::AudioTrackSource>>>,
    // PIT-63: an anchored monotonic wall-clock timestamp -- BASE(SystemTime anchor) + Instant::elapsed() (a monotonic increment).
    // bare SystemTime::now() is not monotonic (NTP jumps/suspend-resume -> ts regressing -> aligned regressing -> FramerateController resets);
    // a fake clock (+33333us/each) disagrees with libwebrtc's time domain (the PIT-61 experiment history).
    ts_base_us: i64,
    ts_anchor: std::time::Instant,
}

impl WebrtcSysTrack {
    fn new_clock_anchor() -> (i64, std::time::Instant) {
        let base_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        (base_us, std::time::Instant::now())
    }
}

impl Default for WebrtcSysTrack {
    fn default() -> Self {
        let (ts_base_us, ts_anchor) = Self::new_clock_anchor();
        Self {
            video_source: Mutex::new(None),
            audio_source: Mutex::new(None),
            ts_base_us,
            ts_anchor,
        }
    }
}

impl std::fmt::Debug for WebrtcSysTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcSysTrack")
            .field("video_source", &self.video_source.lock().unwrap().is_some())
            .field("audio_source", &self.audio_source.lock().unwrap().is_some())
            .finish()
    }
}

impl Clone for WebrtcSysTrack {
    fn clone(&self) -> Self {
        let video = self.video_source.lock().unwrap().clone();
        let audio = self.audio_source.lock().unwrap().clone();
        let (ts_base_us, ts_anchor) = Self::new_clock_anchor();
        Self {
            video_source: Mutex::new(video),
            audio_source: Mutex::new(audio),
            ts_base_us,
            ts_anchor,
        }
    }
}

impl WebrtcSysTrack {
    pub(crate) fn with_video_source(
        source: SharedPtr<webrtc_sys::video_track::ffi::VideoTrackSource>,
    ) -> Self {
        let (ts_base_us, ts_anchor) = Self::new_clock_anchor();
        Self {
            video_source: Mutex::new(Some(source)),
            audio_source: Mutex::new(None),
            ts_base_us,
            ts_anchor,
        }
    }
    pub(crate) fn with_audio_source(
        source: SharedPtr<webrtc_sys::audio_track::ffi::AudioTrackSource>,
    ) -> Self {
        let (ts_base_us, ts_anchor) = Self::new_clock_anchor();
        Self {
            video_source: Mutex::new(None),
            audio_source: Mutex::new(Some(source)),
            ts_base_us,
            ts_anchor,
        }
    }
}

/// The capture_frame completion callback (queue-mode backpressure notification) -- H2 has no backpressure need, no-op.
extern "C" fn noop_audio_complete(_ctx: *const webrtc_sys::audio_track::SourceContext) {}

impl TrackWriteBackend for WebrtcSysTrack {
    async fn write_frame(
        &self,
        data: &[u8],
        kind: TrackKind,
        audio_config: Option<&RTCAudioTrackConfig>,
    ) -> Result<(), RTCError> {
        if kind == TrackKind::Audio {
            let cfg = audio_config
                .ok_or_else(|| RTCError::Track("audio write_frame requires audio_config".into()))?;
            return self.write_pcm(data, cfg);
        }
        tracing::debug!(
            "TrackSender::write_frame (webrtc-sys): {} bytes (encoded video pass-through)",
            data.len()
        );
        // ponytail: encoded video frame passthrough — stub for now, real encoding deferred
        Ok(())
    }

    async fn write_raw_i420_with_ts(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        ts_us: Option<i64>,
    ) -> Result<(), RTCError> {
        use webrtc_sys::video_frame::ffi as vf;
        use webrtc_sys::video_frame_buffer::ffi as vfb;
        use webrtc_sys::video_track::ffi as vt;

        let source = self
            .video_source
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| RTCError::Track("video source not initialized".into()))?;

        // PIT-76 diagnostics: PLI arrival detection -- libwebrtc sets a shared flag on receiving a PLI
        // (the VideoStreamEncoder -> RtpVideoSender -> EncoderRtcpFeedback chain)
        if source.take_keyframe_request() {
            tracing::warn!(
                "PLI/KeyFrameRequest reached VideoTrackSource (take_keyframe_request=true)"
            );
        }

        let w: i32 = width as i32;
        let h: i32 = height as i32;
        // I420 layout: Y plane (W×H) + U plane (W/2×H/2) + V plane (W/2×H/2)
        let y_size = (w * h) as usize;
        let uv_size = ((w / 2) * (h / 2)) as usize;
        if data.len() < y_size + 2 * uv_size {
            return Err(RTCError::Track("I420 data too short".into()));
        }

        let i420 = vfb::new_i420_buffer(w, h, w, w / 2, w / 2);

        // SAFETY: I420Buffer owns the memory; slices live within the call scope.
        // The frame builder consumes the buffer via set_video_frame_buffer before build().
        unsafe {
            let yuv = vfb::i420_to_yuv8(&*i420);
            let y_slice = std::slice::from_raw_parts_mut((*yuv).data_y() as *mut u8, y_size);
            let u_slice = std::slice::from_raw_parts_mut((*yuv).data_u() as *mut u8, uv_size);
            let v_slice = std::slice::from_raw_parts_mut((*yuv).data_v() as *mut u8, uv_size);
            y_slice.copy_from_slice(&data[..y_size]);
            u_slice.copy_from_slice(&data[y_size..y_size + uv_size]);
            v_slice.copy_from_slice(&data[y_size + uv_size..y_size + 2 * uv_size]);
        }

        // Build VideoFrame and push to source
        let mut builder = vf::new_video_frame_builder();
        // PIT-63: anchored monotonic wall-clock -- consistent with livekit TimestampAligner's expectation (frame ts comparable with wall-clock)
        // the ts_us param: Some(the capture moment) for a camera time source; None -> internally anchored monotonic
        let ts_us =
            ts_us.unwrap_or_else(|| self.ts_base_us + self.ts_anchor.elapsed().as_micros() as i64);
        builder.pin_mut().set_timestamp_us(ts_us);
        builder.pin_mut().set_video_frame_buffer(
            // SAFETY: i420 → yuv8 → yuv → vfb upcast chain
            unsafe { &*vfb::yuv_to_vfb(vfb::yuv8_to_yuv(vfb::i420_to_yuv8(&*i420))) },
        );
        let frame = builder.pin_mut().build();

        let metadata = vt::FrameMetadata {
            has_packet_trailer: false,
            user_timestamp: 0,
            frame_id: 0,
            user_data: vec![],
        };

        source.on_captured_frame(&frame, &metadata);
        Ok(())
    }
}

impl WebrtcSysTrack {
    /// H2: push PCM i16 (interleaved) into AudioTrackSource -- libwebrtc encodes opus internally then sends RTP.
    /// The frame length must be exactly 10ms (48000Hz/100 = 480 samples/channel) -- livekit capture_frame semantics.
    fn write_pcm(&self, data: &[u8], cfg: &RTCAudioTrackConfig) -> Result<(), RTCError> {
        let source = self
            .audio_source
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| RTCError::Track("audio source not initialized".into()))?;
        let channels = cfg.channels.max(1);
        let samples_total = data.len() / 2; // i16 = 2 bytes
        let frames = samples_total / channels as usize;
        if frames == 0 {
            return Ok(());
        }
        let frames_10ms = (cfg.sample_rate / 100) as usize;
        if frames != frames_10ms {
            return Err(RTCError::Track(format!(
                "audio capture_frame requires 10ms frames ({frames_10ms} samples/ch), got {frames}"
            )));
        }
        // SAFETY: the caller (tone generator / e2e) guarantees data comes from an i16-aligned buffer (Vec<i16> is 2-byte aligned);
        // the frame length is validated as 10ms; capture_frame locks and copies internally, no lifetime escape.
        let pcm = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const i16, samples_total) };
        let ok = unsafe {
            source.capture_frame(
                pcm,
                cfg.sample_rate,
                channels,
                frames,
                std::ptr::null(),
                webrtc_sys::audio_track::CompleteCallback(noop_audio_complete),
            )
        };
        if !ok {
            tracing::warn!("audio capture_frame rejected (queue full)");
            return Err(RTCError::Track("audio capture_frame rejected (source queue full)".into()));
        }
        // temporary diagnostics (H2): a capture_frame success counter
        static PCM_PUSHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = PCM_PUSHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n.is_multiple_of(50) {
            tracing::debug!("audio capture_frame pushed (total {})", n + 1);
        }
        Ok(())
    }
}

// ponytail: WebrtcSysTrack has interior Mutex for VideoTrackSource;
// C++ side handles actual thread safety for on_captured_frame.

// ── RealObserver ──

/// Holds user-registered callbacks and active video sinks.
#[allow(clippy::type_complexity)] // the callback container's five fields (Fn+SharedPtr shapes) -- an alias's benefit < the noise
pub(crate) struct ObserverCallbacks {
    pub on_track: Mutex<Option<Box<dyn Fn(TrackReceiver) + Send + Sync + 'static>>>,
    /// R3b root fix: **the wrapper that added the sink and the sink are held as a pair**.
    /// webrtc-sys's `~VideoTrack` runs `track()->RemoveSink()` on every sink that wrapper registered --
    /// `receiver().track()` builds a new wrapper per call;
    /// storing only the sink without the wrapper = the wrapper leaving scope drags the just-attached sink with it
    /// (the PIT-197 stream break = the 29-frame window was really the staging-reference lifetime of the on_track argument wrapper).
    pub video_sinks: Mutex<
        Vec<(
            cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>,
            cxx::SharedPtr<webrtc_sys::video_track::ffi::NativeVideoSink>,
        )>,
    >,
    pub on_ice_connection_state_change:
        Mutex<Option<Box<dyn Fn(RTCIceConnectionState) + Send + Sync + 'static>>>,
    pub on_peer_connection_state_change:
        Mutex<Option<Box<dyn Fn(RTCPeerConnectionState) + Send + Sync + 'static>>>,
    pub on_ice_candidate: Mutex<Option<Box<dyn Fn(RTCIceCandidate) + Send + Sync + 'static>>>,
    /// v2: incoming data channel callback (generic on_data_channel)
    pub on_data_channel:
        Mutex<Option<Box<dyn Fn(crate::data_channel::RTCDataChannel) + Send + Sync + 'static>>>,
    /// Staged media tracks awaiting register_track consumption (webrtc-sys only).
    pub staged_media_tracks:
        Mutex<Vec<cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>>>,
}

/// Real observer that forwards libwebrtc events to Rust callbacks.
struct RealObserver {
    callbacks: Arc<ObserverCallbacks>,
}

impl webrtc_sys::peer_connection_factory::PeerConnectionObserver for RealObserver {
    fn on_signaling_change(&self, _: webrtc_sys::peer_connection::ffi::SignalingState) {}

    fn on_add_stream(&self, _: cxx::SharedPtr<webrtc_sys::media_stream::ffi::MediaStream>) {}
    fn on_remove_stream(&self, _: cxx::SharedPtr<webrtc_sys::media_stream::ffi::MediaStream>) {}
    fn on_data_channel(&self, dc: cxx::SharedPtr<webrtc_sys::data_channel::ffi::DataChannel>) {
        let rtc_dc = crate::data_channel::RTCDataChannel {
            label: dc.label(),
            id: dc.id(),
            backend: WebrtcSysDc { dc, rx: std::sync::Arc::new(std::sync::OnceLock::new()) },
        };
        if let Some(ref cb) = *self.callbacks.on_data_channel.lock().unwrap() {
            cb(rtc_dc);
        }
    }
    fn on_renegotiation_needed(&self) {}
    fn on_negotiation_needed_event(&self, _: u32) {}
    fn on_ice_connection_change(
        &self,
        state: webrtc_sys::peer_connection::ffi::IceConnectionState,
    ) {
        let mapped = match state {
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionNew => {
                RTCIceConnectionState::New
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionChecking => {
                RTCIceConnectionState::Checking
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionConnected => {
                RTCIceConnectionState::Connected
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionCompleted => {
                RTCIceConnectionState::Completed
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionFailed => {
                RTCIceConnectionState::Failed
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionDisconnected => {
                RTCIceConnectionState::Disconnected
            }
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionClosed => {
                RTCIceConnectionState::Closed
            }
            _ => RTCIceConnectionState::New,
        };
        tracing::info!("ICE connection state changed: {:?}", mapped);
        if let Some(ref cb) = *self.callbacks.on_ice_connection_state_change.lock().unwrap() {
            cb(mapped);
        }
    }
    fn on_standardized_ice_connection_change(
        &self,
        _: webrtc_sys::peer_connection::ffi::IceConnectionState,
    ) {
    }
    fn on_connection_change(&self, state: webrtc_sys::peer_connection::ffi::PeerConnectionState) {
        let mapped = match state {
            webrtc_sys::peer_connection::ffi::PeerConnectionState::New => {
                RTCPeerConnectionState::New
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Connecting => {
                RTCPeerConnectionState::Connecting
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Connected => {
                RTCPeerConnectionState::Connected
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Disconnected => {
                RTCPeerConnectionState::Disconnected
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Failed => {
                RTCPeerConnectionState::Failed
            }
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Closed => {
                RTCPeerConnectionState::Closed
            }
            _ => RTCPeerConnectionState::New,
        };
        tracing::info!("PC connection state changed: {:?}", mapped);
        if let Some(ref cb) = *self.callbacks.on_peer_connection_state_change.lock().unwrap() {
            cb(mapped);
        }
    }
    fn on_ice_gathering_change(&self, state: webrtc_sys::peer_connection::ffi::IceGatheringState) {
        let s = match state {
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringNew => "new",
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringGathering => {
                "gathering"
            }
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringComplete => "complete",
            _ => "unknown",
        };
        tracing::debug!("ICE gathering state: {}", s);
    }
    fn on_ice_candidate(&self, candidate: cxx::SharedPtr<webrtc_sys::jsep::ffi::IceCandidate>) {
        // PIT-43/52: fully forward trickled local candidates (required by the P2P path)
        let rtc_candidate = crate::peer_connection::RTCIceCandidate {
            candidate: candidate.candidate(),
            sdp_mid: Some(candidate.sdp_mid()),
            sdp_mline_index: None,
        };
        if let Some(ref cb) = *self.callbacks.on_ice_candidate.lock().unwrap() {
            cb(rtc_candidate);
        } else {
            tracing::debug!("ICE local candidate (unhandled): {}", rtc_candidate.candidate);
        }
    }
    fn on_ice_candidate_error(&self, _: String, _: i32, _: String, _: i32, _: String) {}
    fn on_ice_candidates_removed(
        &self,
        _: Vec<cxx::SharedPtr<webrtc_sys::candidate::ffi::Candidate>>,
    ) {
    }
    fn on_ice_connection_receiving_change(&self, _: bool) {}
    fn on_ice_selected_candidate_pair_changed(
        &self,
        _: webrtc_sys::peer_connection_factory::ffi::CandidatePairChangeEvent,
    ) {
    }
    fn on_remove_track(&self, _: cxx::SharedPtr<webrtc_sys::rtp_receiver::ffi::RtpReceiver>) {}
    fn on_interesting_usage(&self, _: i32) {}

    fn on_add_track(
        &self,
        _receiver: cxx::SharedPtr<webrtc_sys::rtp_receiver::ffi::RtpReceiver>,
        _streams: Vec<cxx::SharedPtr<webrtc_sys::media_stream::ffi::MediaStream>>,
    ) {
        // ponytail: on_add_track falls through to on_track for unified handling
    }

    fn on_track(
        &self,
        transceiver: cxx::SharedPtr<webrtc_sys::rtp_transceiver::ffi::RtpTransceiver>,
    ) {
        use webrtc_sys::video_frame::ffi as vff;
        use webrtc_sys::video_frame_buffer::ffi as vfb;

        let receiver = transceiver.receiver();
        let track = receiver.track();
        let kind = match receiver.media_type() {
            webrtc_sys::webrtc::ffi::MediaType::Video => TrackKind::Video,
            _ => TrackKind::Audio,
        };
        let tr = TrackReceiver::new(track.id(), kind);
        tracing::debug!(
            "webrtc-sys on_track: track_id={} ptr=0x{:x} kind={:?}",
            track.id(),
            &*track as *const _ as usize,
            kind
        );

        // PIT diagnostics: the receiver's negotiated params (confirming whether ssrc/PT made it onto the receive stream)
        {
            let params = receiver.get_parameters();
            tracing::info!(
                "webrtc-sys on_track receiver params: codecs={} encodings={} mid={:?}",
                params.codecs.len(),
                params.encodings.len(),
                params.mid
            );
            for enc in &params.encodings {
                tracing::info!("  encoding ssrc={:?} rid={:?}", enc.ssrc, enc.rid);
            }
        }

        // PIT diagnostics: the remote track state (Live/Ended + enabled) -- judging whether the decoded stream is bound
        let track_state = match track.state() {
            webrtc_sys::media_stream_track::ffi::TrackState::Live => "Live",
            webrtc_sys::media_stream_track::ffi::TrackState::Ended => "Ended",
            _ => "Unknown",
        };
        tracing::info!(
            "webrtc-sys on_track: track_id={} kind={:?} state={} enabled={}",
            track.id(),
            kind,
            track_state,
            track.enabled()
        );

        // Invoke user callback
        if let Some(ref cb) = *self.callbacks.on_track.lock().unwrap() {
            cb(tr.clone());
        }

        // If the user registered a FrameSink, create native VideoSink bridge
        if kind == TrackKind::Video {
            let sink_arc = tr.sink.clone();
            if sink_arc.lock().unwrap().is_some() {
                let callbacks = self.callbacks.clone();
                #[allow(dead_code)]
                struct VideoSinkAdapter {
                    sink:
                        std::sync::Arc<std::sync::Mutex<Option<Box<dyn crate::track::FrameSink>>>>,
                    hits: std::sync::Arc<std::sync::atomic::AtomicU64>,
                }
                impl webrtc_sys::video_track::VideoSink for VideoSinkAdapter {
                    fn on_frame(&self, frame: cxx::UniquePtr<vff::VideoFrame>) {
                        self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::debug!(
                            "VideoSinkAdapter::on_frame fired (w={} h={})",
                            frame.width(),
                            frame.height()
                        );
                        if let Some(ref sink) = *self.sink.lock().unwrap() {
                            // FIX(msrtc-corruption P1): frame.width()/height() are the RENDER
                            // dimensions; the underlying buffer is the CODED one (H.264 pads to
                            // macroblocks, e.g. 540→544) with per-plane row strides. The old code
                            // sliced plane offsets by render size and copied linearly by render
                            // size — padding rows leaked into U and stride>width misaligned every
                            // chroma row => full-frame corruption at non-16-multiple heights.
                            let w = frame.width() as usize;
                            let h = frame.height() as usize;
                            let buf = unsafe { frame.video_frame_buffer() };
                            let i420 = unsafe { (*buf).to_i420() };
                            let yuv = unsafe { vfb::i420_to_yuv8(&*i420) };
                            let sy = unsafe { (*yuv).stride_y() } as usize;
                            let su = unsafe { (*yuv).stride_u() } as usize;
                            let sv = unsafe { (*yuv).stride_v() } as usize;
                            let cw = (w + 1) / 2;
                            let ch = (h + 1) / 2;
                            let y_size = w * h;
                            let uv_size = cw * ch;
                            let mut data = vec![0u8; y_size + 2 * uv_size];
                            unsafe {
                                let yp = (*yuv).data_y();
                                let up = (*yuv).data_u();
                                let vp = (*yuv).data_v();
                                // Fast path: buffer is exactly render-sized and compact — bulk copy.
                                if sy == w && su == cw && sv == cw
                                    && (*yuv).width() as usize == w && (*yuv).height() as usize == h
                                {
                                    std::ptr::copy_nonoverlapping(yp, data.as_mut_ptr(), y_size);
                                    std::ptr::copy_nonoverlapping(up, data.as_mut_ptr().add(y_size), uv_size);
                                    std::ptr::copy_nonoverlapping(vp, data.as_mut_ptr().add(y_size + uv_size), uv_size);
                                } else {
                                    // Stride-aware row copy; top-left crop (H.264 conformance window pads bottom/right).
                                    for r in 0..h {
                                        std::ptr::copy_nonoverlapping(
                                            yp.add(r * sy),
                                            data.as_mut_ptr().add(r * w),
                                            w,
                                        );
                                    }
                                    for r in 0..ch {
                                        std::ptr::copy_nonoverlapping(
                                            up.add(r * su),
                                            data.as_mut_ptr().add(y_size + r * cw),
                                            cw,
                                        );
                                        std::ptr::copy_nonoverlapping(
                                            vp.add(r * sv),
                                            data.as_mut_ptr().add(y_size + uv_size + r * cw),
                                            cw,
                                        );
                                    }
                                }
                            }
                            sink.on_frame(&data, w as u32, h as u32);
                        }
                    }
                    fn on_discarded_frame(&self) {
                        let n =
                            self.hits.fetch_add(1_000_000, std::sync::atomic::Ordering::Relaxed);
                        if n % 1_000_000 < 3 {
                            tracing::warn!(
                                "video sink on_discarded_frame (adapter drop, frames={})",
                                n % 1_000_000
                            );
                        }
                    }
                    fn on_constraints_changed(
                        &self,
                        _: webrtc_sys::video_track::ffi::VideoTrackSourceConstraints,
                    ) {
                    }
                }

                let hits = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let adapter = VideoSinkAdapter { sink: sink_arc.clone(), hits: hits.clone() };
                let wrapper =
                    webrtc_sys::video_track::VideoSinkWrapper::new(std::sync::Arc::new(adapter));

                // Register sink with the video track (once immediately -- if the channel is already ready it goes straight through)
                tracing::debug!(
                    "VideoSinkAdapter: attaching native sink to video track 0x{:x}",
                    &*track as *const _ as usize
                );
                unsafe {
                    let video_track = webrtc_sys::video_track::ffi::media_to_video(track.clone());
                    let native_sink =
                        webrtc_sys::video_track::ffi::new_native_video_sink(Box::new(wrapper));
                    video_track.add_sink(&native_sink);
                    callbacks.video_sinks.lock().unwrap().push((track.clone(), native_sink));
                }
            }
        }
    }
}

// ── WebrtcSysFactory ──

pub(crate) struct WebrtcSysFactory {
    factory: cxx::SharedPtr<webrtc_sys::peer_connection_factory::ffi::PeerConnectionFactory>,
}

impl Default for WebrtcSysFactory {
    fn default() -> Self {
        // register libwebrtc's internal logging (RTC_LOG -> tracing) -- PIT-45 debugging: the ICE gathering internal state
        let log_sink = webrtc_sys::webrtc::ffi::new_log_sink(|message, severity| {
            let line = message.trim_end().to_string();
            match severity {
                webrtc_sys::webrtc::ffi::LoggingSeverity::Info => {
                    tracing::info!("[libwebrtc] {}", line);
                }
                webrtc_sys::webrtc::ffi::LoggingSeverity::Warning => {
                    tracing::warn!("[libwebrtc] {}", line);
                }
                webrtc_sys::webrtc::ffi::LoggingSeverity::Error => {
                    tracing::error!("[libwebrtc] {}", line);
                }
                _ => {
                    tracing::debug!("[libwebrtc] {}", line);
                }
            }
        });
        let _ = log_sink; // keep alive for process lifetime
        let factory = webrtc_sys::peer_connection_factory::ffi::create_peer_connection_factory();
        Self { factory }
    }
}

impl Clone for WebrtcSysFactory {
    fn clone(&self) -> Self {
        Self { factory: self.factory.clone() }
    }
}

impl std::fmt::Debug for WebrtcSysFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcSysFactory").finish()
    }
}

impl WebrtcSysFactory {
    pub(crate) async fn create_peer_connection(
        &self,
        config: RTCConfiguration,
    ) -> Result<WebrtcSysPc, RTCError> {
        tracing::info!("Creating RTCPeerConnection (webrtc-sys)");

        let ice_servers: Vec<webrtc_sys::peer_connection::ffi::IceServer> = config
            .ice_servers
            .iter()
            .map(|srv| webrtc_sys::peer_connection::ffi::IceServer {
                urls: srv.urls.clone(),
                username: srv.username.clone(),
                password: srv.password.clone(),
            })
            .collect();

        let ice_transport_type = match config.ice_transport_type {
            RTCIceTransportPolicy::Relay => {
                webrtc_sys::peer_connection::ffi::IceTransportsType::Relay
            }
            RTCIceTransportPolicy::NoHost => {
                webrtc_sys::peer_connection::ffi::IceTransportsType::NoHost
            }
            RTCIceTransportPolicy::All => webrtc_sys::peer_connection::ffi::IceTransportsType::All,
        };

        let rtc_config = webrtc_sys::peer_connection::ffi::RtcConfiguration {
            ice_servers,
            continual_gathering_policy:
                webrtc_sys::peer_connection::ffi::ContinualGatheringPolicy::GatherOnce,
            ice_transport_type,
        };

        // Create RealObserver with shared callback state
        let callbacks = Arc::new(ObserverCallbacks {
            on_track: Mutex::new(None),
            video_sinks: Mutex::new(Vec::new()),
            on_ice_connection_state_change: Mutex::new(None),
            on_peer_connection_state_change: Mutex::new(None),
            on_ice_candidate: Mutex::new(None),
            on_data_channel: Mutex::new(None),
            staged_media_tracks: Mutex::new(Vec::new()),
        });
        let observer = webrtc_sys::peer_connection_factory::PeerConnectionObserverWrapper::new(
            Arc::new(RealObserver { callbacks: callbacks.clone() }),
        );

        // the C++ CreatePeerConnection first call carries full subsystem init (VAAPI DRM scan/PulseAudio/
        // X11 display -- measured can exceed 15s); running it synchronously would hog a tokio worker -> the same runtime's
        // ws heartbeat task starves -> link's 15s silence judge kills the connection -> pulling streams "fails on the first try and succeeds on retry"
        // (flaky, proven by both sides' logs). spawn_blocking moves it off the worker: heartbeat/signaling never starve.
        // the factory is Clone + Send (an Arc-internal lock) -- cross-thread safe.
        let factory = self.factory.clone();
        let pc = tokio::task::spawn_blocking(move || {
            factory
                .create_peer_connection(rtc_config, Box::new(observer))
                .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))
        })
        .await
        .map_err(|e| RTCError::RTCPeerConnection(format!("create_pc task join: {e}")))??;

        Ok(WebrtcSysPc {
            pc,
            callbacks,
            local_sdp: Arc::new(std::sync::Mutex::new(None)),
            factory: self.factory.clone(),
        })
    }

    /// Create a video track with a new VideoTrackSource.
    /// Returns (WebrtcSysTrack, SharedPtr<MediaStreamTrack>) —
    /// the media track can be added to the RTCPeerConnection via add_track.
    pub(crate) fn create_video_track(
        &self,
    ) -> (WebrtcSysTrack, cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>)
    {
        use webrtc_sys::video_track::ffi as vt;

        let resolution = vt::VideoResolution { width: 640, height: 480 };
        let source = vt::new_video_track_source(&resolution, false);
        let backend = WebrtcSysTrack::with_video_source(source.clone());

        // Create VideoTrack from factory, then convert to MediaStreamTrack
        let video_track = self.factory.create_video_track("video".into(), source);
        let media_track = vt::video_to_media(video_track);

        (backend, media_track)
    }

    /// Create an audio track with a new AudioTrackSource (H2).
    /// Returns (WebrtcSysTrack, SharedPtr<MediaStreamTrack>) —
    /// the media track can be added to the RTCPeerConnection via add_track.
    /// 48kHz mono, queue_size_ms=100 (tolerating 10ms cadence jitter; 0 = the fast path needs a strict 10ms).
    pub(crate) fn create_audio_track(
        &self,
    ) -> (WebrtcSysTrack, cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>)
    {
        use webrtc_sys::audio_track::ffi as at;

        let source = at::new_audio_track_source(
            at::AudioSourceOptions {
                echo_cancellation: false,
                noise_suppression: false,
                auto_gain_control: false,
            },
            48000,
            1,
            100,
        );
        let backend = WebrtcSysTrack::with_audio_source(source.clone());

        let audio_track = self.factory.create_audio_track("audio".into(), source);
        let media_track = at::audio_to_media(audio_track);

        (backend, media_track)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_timestamp_is_monotonic_and_wallclock_magnitude() {
        // PIT-63: anchored monotonic -- monotonically increasing + wall-clock magnitude (~1.78e15 us @2026)
        let (base, anchor) = WebrtcSysTrack::new_clock_anchor();
        let t1 = base + anchor.elapsed().as_micros() as i64;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let t2 = base + anchor.elapsed().as_micros() as i64;
        assert!(t2 > t1, "timestamps must increase monotonically");
        assert!(t1 > 1_000_000_000_000, "wall-clock magnitude (us): {t1}");
    }

    #[test]
    fn map_rtp_parameters_codec_fmtp_mapping() {
        // PIT-54: mediasoup matches strictly on codec parameters -- verifying map_rtp_parameters maps the H264 fmtp
        use webrtc_sys::rtp_parameters::ffi::{
            RtcpParameters, RtpCodecParameters, RtpEncodingParameters, RtpExtension, RtpParameters,
            StringKeyValue,
        };
        let params = RtpParameters {
            transaction_id: "tx".into(),
            mid: "0".into(),
            codecs: vec![RtpCodecParameters {
                mime_type: "video/H264".into(),
                name: "H264".into(),
                kind: webrtc_sys::webrtc::ffi::MediaType::Video,
                payload_type: 101,
                has_clock_rate: true,
                clock_rate: 90000,
                has_num_channels: false,
                num_channels: 0,
                has_max_ptime: false,
                max_ptime: 0,
                has_ptime: false,
                ptime: 0,
                rtcp_feedback: vec![],
                parameters: vec![
                    StringKeyValue { key: "packetization-mode".into(), value: "1".into() },
                    StringKeyValue { key: "profile-level-id".into(), value: "42e01f".into() },
                ],
            }],
            header_extensions: vec![],
            encodings: vec![RtpEncodingParameters {
                has_ssrc: true,
                ssrc: 1949911776,
                bitrate_priority: 1.0,
                network_priority: webrtc_sys::webrtc::ffi::Priority::Medium,
                has_max_bitrate_bps: false,
                max_bitrate_bps: 0,
                has_min_bitrate_bps: false,
                min_bitrate_bps: 0,
                has_max_framerate: false,
                max_framerate: 0.0,
                has_num_temporal_layers: false,
                num_temporal_layers: 0,
                has_scale_resolution_down_by: false,
                scale_resolution_down_by: 1.0,
                has_scalability_mode: false,
                scalability_mode: String::new(),
                active: true,
                rid: String::new(),
                adaptive_ptime: false,
                request_key_frame: true, // PIT-76: verifying passthrough
            }],
            rtcp: RtcpParameters {
                has_ssrc: false,
                ssrc: 0,
                cname: String::new(),
                reduced_size: true,
                mux: true,
            },
            has_degradation_preference: false,
            degradation_preference:
                webrtc_sys::rtp_parameters::ffi::DegradationPreference::Balanced,
        };
        let mapped = map_rtp_parameters(params);
        assert_eq!(mapped.codecs.len(), 1);
        let fmtp = mapped.codecs[0].sdp_fmtp_line.as_deref().unwrap_or("");
        // PIT-54: must contain packetization-mode and profile-level-id
        assert!(fmtp.contains("packetization-mode=1"), "fmtp: {fmtp}");
        assert!(fmtp.contains("profile-level-id=42e01f"), "fmtp: {fmtp}");
        assert_eq!(mapped.encodings[0].ssrc, Some(1949911776));
        // PIT-76: the request_key_frame field must pass through
        assert_eq!(mapped.encodings[0].request_key_frame, true);
    }

    #[test]
    fn map_rtp_parameters_request_key_frame_false() {
        // PIT-76: request_key_frame default false must also pass through (non-default-value dependence)
        use webrtc_sys::rtp_parameters::ffi::{
            RtcpParameters, RtpEncodingParameters, RtpParameters,
        };
        let params = RtpParameters {
            transaction_id: "tx".into(),
            mid: "0".into(),
            codecs: vec![],
            header_extensions: vec![],
            encodings: vec![RtpEncodingParameters {
                has_ssrc: false,
                ssrc: 0,
                bitrate_priority: 1.0,
                network_priority: webrtc_sys::webrtc::ffi::Priority::Medium,
                has_max_bitrate_bps: false,
                max_bitrate_bps: 0,
                has_min_bitrate_bps: false,
                min_bitrate_bps: 0,
                has_max_framerate: false,
                max_framerate: 0.0,
                has_num_temporal_layers: false,
                num_temporal_layers: 0,
                has_scale_resolution_down_by: false,
                scale_resolution_down_by: 1.0,
                has_scalability_mode: false,
                scalability_mode: String::new(),
                active: true,
                rid: String::new(),
                adaptive_ptime: false,
                request_key_frame: false,
            }],
            rtcp: RtcpParameters {
                has_ssrc: false,
                ssrc: 0,
                cname: String::new(),
                reduced_size: true,
                mux: true,
            },
            has_degradation_preference: false,
            degradation_preference:
                webrtc_sys::rtp_parameters::ffi::DegradationPreference::Balanced,
        };
        let mapped = map_rtp_parameters(params);
        assert_eq!(mapped.encodings[0].request_key_frame, false);
    }

    #[test]
    fn map_codec_capability_to_sys_h264() {
        // v2 (set-codec-preferences T4): W3C -> sys -- name/kind must be filled explicitly (mime_type is ignored by to_native)
        use crate::rtp::RTCRtpCodecCapability;
        let cap = RTCRtpCodecCapability {
            mime_type: "video/H264".into(),
            clock_rate: Some(90000),
            channels: None,
            sdp_fmtp_line: Some("profile-level-id=42e01f;packetization-mode=1".into()),
        };
        let sys = map_codec_capability_to_sys(&cap);
        assert_eq!(sys.name, "H264");
        assert_eq!(sys.kind, webrtc_sys::webrtc::ffi::MediaType::Video);
        assert!(sys.has_clock_rate);
        assert_eq!(sys.clock_rate, 90000);
        assert!(!sys.has_num_channels, "channels=None → has_num_channels=false");
        // fmtp parsing: order preserved (libwebrtc's exact map matching)
        assert_eq!(sys.parameters.len(), 2);
        assert_eq!(sys.parameters[0].key, "profile-level-id");
        assert_eq!(sys.parameters[0].value, "42e01f");
        assert_eq!(sys.parameters[1].key, "packetization-mode");
        assert_eq!(sys.parameters[1].value, "1");
    }

    #[test]
    fn map_codec_capability_to_sys_audio_and_empty_fmtp() {
        use crate::rtp::RTCRtpCodecCapability;
        // audio: kind correct; no fmtp -> empty parameters
        let cap = RTCRtpCodecCapability {
            mime_type: "audio/opus".into(),
            clock_rate: Some(48000),
            channels: Some(2),
            sdp_fmtp_line: None,
        };
        let sys = map_codec_capability_to_sys(&cap);
        assert_eq!(sys.name, "opus");
        assert_eq!(sys.kind, webrtc_sys::webrtc::ffi::MediaType::Audio);
        assert_eq!(sys.num_channels, 2);
        assert!(sys.has_num_channels);
        assert!(sys.parameters.is_empty());
    }

    #[test]
    fn map_rtp_capabilities_restores_fmtp() {
        // v2 (set-codec-preferences T2): sys -> W3C fmtp restoration (byte-exact)
        use webrtc_sys::rtp_parameters::ffi::{
            RtpCapabilities, RtpCodecCapability, RtpHeaderExtensionCapability, StringKeyValue,
        };
        let caps = RtpCapabilities {
            codecs: vec![RtpCodecCapability {
                mime_type: "video/H264".into(),
                name: "H264".into(),
                kind: webrtc_sys::webrtc::ffi::MediaType::Video,
                has_clock_rate: true,
                clock_rate: 90000,
                has_preferred_payload_type: true,
                preferred_payload_type: 101,
                has_num_channels: false,
                num_channels: 0,
                rtcp_feedback: vec![],
                parameters: vec![
                    StringKeyValue { key: "profile-level-id".into(), value: "42e01f".into() },
                    StringKeyValue { key: "packetization-mode".into(), value: "1".into() },
                ],
            }],
            header_extensions: vec![],
            fec: vec![],
        };
        let mapped = map_rtp_capabilities(caps);
        assert_eq!(mapped.codecs.len(), 1);
        assert_eq!(mapped.codecs[0].mime_type, "video/H264");
        assert_eq!(mapped.codecs[0].clock_rate, Some(90000));
        let fmtp = mapped.codecs[0].sdp_fmtp_line.as_deref().unwrap_or("");
        assert!(fmtp.contains("profile-level-id=42e01f"), "fmtp: {fmtp}");
        assert!(fmtp.contains("packetization-mode=1"), "fmtp: {fmtp}");
    }
}

#[cfg(test)]
mod stats_tests {
    use super::parse_outbound_stats_json;
    use crate::stats::RTCStats;

    #[test]
    fn parses_outbound_rtp_with_encoder_implementation() {
        // v2 (web-stream-stats T1.5): libwebrtc ToJson outbound-rtp field parsing
        let json = r#"[{
            "type": "outbound-rtp",
            "id": "RTC_rtp_video_1",
            "timestamp": 1786000000000.0,
            "ssrc": 12345,
            "kind": "video",
            "framesEncoded": 120,
            "frameWidth": 1280,
            "frameHeight": 720,
            "framesPerSecond": 30.0,
            "packetsSent": 999,
            "bytesSent": 123456,
            "encoderImplementation": "OpenH264",
            "totalEncodeTime": 3.6
        }, {
            "type": "inbound-rtp",
            "id": "RTC_rtp_video_2"
        }]"#;
        let stats = parse_outbound_stats_json(json);
        assert_eq!(stats.len(), 1);
        match &stats[0] {
            RTCStats::OutboundRtp(o) => {
                assert_eq!(o.encoder_implementation.as_deref(), Some("OpenH264"));
                assert_eq!(o.frames_encoded, 120);
                assert_eq!(o.frame_width, 1280);
                assert_eq!(o.frame_height, 720);
                assert_eq!(o.frames_per_second, 30.0);
                assert_eq!(o.ssrc, 12345);
                // v3 (encode-time-stats T2): totalEncodeTime parsing -- 120 frames x 30ms = 3.6s
                assert_eq!(o.total_encode_time, Some(3.6));
            }
            other => panic!("expected OutboundRtp, got {other:?}"),
        }
    }

    #[test]
    fn parses_empty_and_missing_encoder() {
        assert!(parse_outbound_stats_json("not-json").is_empty());
        assert!(parse_outbound_stats_json("[]").is_empty());
        let json = r#"[{"type": "outbound-rtp", "id": "x", "timestamp": 0.0, "ssrc": 1, "kind": "video"}]"#;
        // v3: totalEncodeTime missing -> None (an older libwebrtc / the field unavailable)
        let stats = parse_outbound_stats_json(json);
        match &stats[0] {
            RTCStats::OutboundRtp(o) => assert_eq!(o.total_encode_time, None),
            other => panic!("expected OutboundRtp, got {other:?}"),
        }
        let stats = parse_outbound_stats_json(json);
        match &stats[0] {
            RTCStats::OutboundRtp(o) => assert_eq!(o.encoder_implementation, None),
            _ => panic!(),
        }
    }
}
