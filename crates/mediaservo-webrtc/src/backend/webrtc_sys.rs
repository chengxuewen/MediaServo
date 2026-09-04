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

use std::sync::{Arc, Mutex};
use cxx::SharedPtr;

use super::DcBackend;
use super::PcBackend;
use super::TrackWriteBackend;
use crate::data_channel::{RTCDataChannelRx, RTCDataChannelState};
use crate::peer_connection::{
    RTCAnswerOptions, RTCIceCandidate, RTCIceConnectionState, RTCIceGatheringState, RTCIceTransportPolicy,
    RTCOfferOptions, RTCConfiguration, RTCPeerConnectionState, RTCSignalingState,
};
use crate::sdp::{RTCSdpType, RTCSessionDescription};
use crate::track::{RTCAudioTrackConfig, TrackKind, TrackReceiver};
use crate::RTCError;

// ── WebrtcSysPc ──

#[derive(Clone)]
pub(crate) struct WebrtcSysPc {
    pc: cxx::SharedPtr<webrtc_sys::peer_connection::ffi::PeerConnection>,
    callbacks: Arc<ObserverCallbacks>,
    local_sdp: Arc<std::sync::Mutex<Option<String>>>,
    /// v2: factory 引用（capabilities 查询需要）
    factory: cxx::SharedPtr<webrtc_sys::peer_connection_factory::ffi::PeerConnectionFactory>,
}

impl std::fmt::Debug for WebrtcSysPc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebrtcSysPc")
            .field("connection_state", &self.connection_state())
            .finish()
    }
}

/// Helper: wrap a oneshot sender in a PeerContext for FFI callbacks.
fn make_ctx<T: Send + 'static>(
    tx: tokio::sync::oneshot::Sender<T>,
) -> Box<webrtc_sys::peer_connection::PeerContext> {
    Box::new(webrtc_sys::peer_connection::PeerContext(Box::new(tx)))
}

/// Helper: extract a oneshot sender from a PeerContext via downcast.
fn extract_tx<T: Send + 'static>(
    ctx: Box<webrtc_sys::peer_connection::PeerContext>,
) -> tokio::sync::oneshot::Sender<T> {
    *ctx.0
        .downcast::<tokio::sync::oneshot::Sender<T>>()
        .unwrap_or_else(|_| panic!("PeerContext downcast failed"))
}


/// 解析 libwebrtc getStats ToJson（数组）→ outbound-rtp RTCStats。
/// 字段: framesEncoded/framesPerSecond/frameWidth/frameHeight/encoderImplementation（Oracle F2 实证）。
fn parse_outbound_stats_json(json: &str) -> Vec<crate::stats::RTCStats> {
    use crate::stats::{RTCStats, RTCOutboundRtpStreamStats};
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
                frames_per_second: v
                    .get("framesPerSecond")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0),
                // v3 (encode-time-stats T2): W3C outbound-rtp 标准字段 totalEncodeTime（秒）
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

// ── v2: webrtc-sys RtpParameters → crate RTCRtpParameters 映射 ──
fn map_rtp_parameters(p: webrtc_sys::rtp_parameters::ffi::RtpParameters) -> crate::rtp::RTCRtpParameters {
    use crate::rtp::{RTCRtpCodecParameters, RTCRtpEncodingParameters, RTCRtpHeaderExtensionParameters, RTCRtcpParameters};
    let codecs = p.codecs.iter().map(|c| RTCRtpCodecParameters {
        mime_type: c.mime_type.clone(),
        payload_type: c.payload_type as u8,
        clock_rate: c.clock_rate.max(0) as u32,
        channels: if c.has_num_channels { Some(c.num_channels as u16) } else { None },
        // PIT-54: mediasoup 严格按 codec parameters 匹配 (packetization-mode/profile-level-id)
        // webrtc-sys RtpCodecParameters.parameters (Vec<StringKeyValue>) -> sdp_fmtp_line
        sdp_fmtp_line: if c.parameters.is_empty() {
            None
        } else {
            Some(c.parameters.iter()
                .map(|kv| format!("{}={}", kv.key, kv.value))
                .collect::<Vec<_>>()
                .join(";"))
        },
    }).collect();
    let encodings = p.encodings.iter().map(|e| RTCRtpEncodingParameters {
        ssrc: if e.has_ssrc { Some(e.ssrc as u64) } else { None },
        active: e.active,
        max_bitrate: if e.has_max_bitrate_bps { Some(e.max_bitrate_bps.max(0) as u64) } else { None },
        min_bitrate: if e.has_min_bitrate_bps { Some(e.min_bitrate_bps.max(0) as u64) } else { None },
        max_framerate: if e.has_max_framerate { Some(e.max_framerate) } else { None },
        scale_resolution_down_by: if e.has_scale_resolution_down_by { Some(e.scale_resolution_down_by) } else { None },
        rid: if e.rid.is_empty() { None } else { Some(e.rid.clone()) },
        codec: None,
        dtx: None,
        request_key_frame: e.request_key_frame,
    }).collect();
    let header_extensions = p.header_extensions.iter().map(|h| RTCRtpHeaderExtensionParameters {
        uri: h.uri.clone(),
        id: h.id as u16,
        encrypted: h.encrypt,
    }).collect();
    let rtcp = RTCRtcpParameters {
        cname: Some(p.rtcp.cname.clone()),
        reduced_size: p.rtcp.reduced_size,
    };
    crate::rtp::RTCRtpParameters {
        transaction_id: p.transaction_id,
        mid: p.mid,
        codecs,
        encodings,
        header_extensions,
        rtcp,
    }
}

// ── qos-framerate-priority: crate 枚举 → vendor 枚举映射（私有，vendor 类型不出边界） ──
fn map_pref(p: crate::rtp::RTCDegradationPreference) -> webrtc_sys::rtp_parameters::ffi::DegradationPreference {
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

// ── v2: webrtc-sys RtpCapabilities → crate RTCRtpCapabilities 映射 ──
fn map_rtp_capabilities(c: webrtc_sys::rtp_parameters::ffi::RtpCapabilities) -> crate::rtp::RTCRtpCapabilities {
    use crate::rtp::{RTCRtpCodecCapability, RTCRtpHeaderExtensionCapability};
    let codecs = c.codecs.iter().map(|cc| RTCRtpCodecCapability {
        mime_type: cc.mime_type.clone(),
        clock_rate: if cc.has_clock_rate { Some(cc.clock_rate.max(0) as u32) } else { None },
        channels: if cc.has_num_channels { Some(cc.num_channels as u16) } else { None },
        // v2 (set-codec-preferences T2): fmtp 还原 — 复用 map_rtp_parameters:84-91 序列化模式,
        // libwebrtc 匹配是精确 map 相等 (MatchesCapability), 往返必须字节精确。
        sdp_fmtp_line: if cc.parameters.is_empty() {
            None
        } else {
            Some(cc.parameters.iter()
                .map(|kv| format!("{}={}", kv.key, kv.value))
                .collect::<Vec<_>>()
                .join(";"))
        },
    }).collect();
    let header_extensions = c.header_extensions.iter().map(|h| RTCRtpHeaderExtensionCapability {
        uri: h.uri.clone(),
        id: if h.has_preferred_id { Some(h.preferred_id as u16) } else { None },
    }).collect();
    crate::rtp::RTCRtpCapabilities { codecs, header_extensions }
}

/// 解析 "k=v;k=v" fmtp 行 → webrtc-sys StringKeyValue 列表（setCodecPreferences 输入）。
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

// ── v2 (set-codec-preferences T1): crate RTCRtpCodecCapability → webrtc-sys RtpCodecCapability 映射 ──
// 注意: webrtc-sys to_native 忽略 mime_type（rtp_parameters.cpp:47 实证）→ 必须显式填 name + kind。
fn map_codec_capability_to_sys(c: &crate::rtp::RTCRtpCodecCapability) -> webrtc_sys::rtp_parameters::ffi::RtpCodecCapability {
    use webrtc_sys::webrtc::ffi::MediaType;
    let (name, kind) = match c.mime_type.split_once('/') {
        Some((_, name)) => {
            let kind = if c.mime_type.starts_with("video/") {
                MediaType::Video
            } else {
                MediaType::Audio
            };
            (name.to_string(), kind)
        }
        // ponytail: 无斜杠的 mime 按 video 处理（调用方应传完整 mime）
        None => (c.mime_type.clone(), MediaType::Video),
    };
    let parameters = c
        .sdp_fmtp_line
        .as_deref()
        .map(parse_fmtp_line)
        .unwrap_or_default();
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
    async fn create_offer(&self, options: &RTCOfferOptions) -> Result<RTCSessionDescription, RTCError> {
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
                let _ = tx.send(Ok(RTCSessionDescription {
                    sdp_type,
                    sdp: sdp_str,
                }));
            },
            |ctx, error| {
                let tx: tokio::sync::oneshot::Sender<Result<RTCSessionDescription, RTCError>> =
                    extract_tx(ctx);
                let _ = tx.send(Err(RTCError::Internal(error.message)));
            },
        );

        rx.await
            .map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
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
                let _ = tx.send(Ok(RTCSessionDescription {
                    sdp_type,
                    sdp: sdp_str,
                }));
            },
            |ctx, error| {
                let tx: tokio::sync::oneshot::Sender<Result<RTCSessionDescription, RTCError>> =
                    extract_tx(ctx);
                let _ = tx.send(Err(RTCError::Internal(error.message)));
            },
        );

        rx.await
            .map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
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

        let _result = rx.await
            .map_err(|_| RTCError::Internal("oneshot cancelled".into()))?;

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

        rx.await
            .map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
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

        rx.await
            .map_err(|_| RTCError::Internal("oneshot cancelled".into()))?
    }

    fn connection_state(&self) -> RTCPeerConnectionState {
        match self.pc.connection_state() {
            webrtc_sys::peer_connection::ffi::PeerConnectionState::New => RTCPeerConnectionState::New,
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

    fn set_on_data_channel(&self, cb: Box<dyn Fn(crate::data_channel::RTCDataChannel) + Send + Sync + 'static>) {
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

    fn set_on_ice_candidate(
        &self,
        cb: Box<dyn Fn(RTCIceCandidate) + Send + Sync + 'static>,
    ) {
        *self.callbacks.on_ice_candidate.lock().unwrap() = Some(cb);
    }

    fn local_description_sdp(&self) -> Option<String> {
        self.local_sdp.lock().unwrap().clone()
    }

    /// Override: consume a staged media track and add it to libwebrtc.
    fn register_track(
        &self, _track_id: &str, _kind: TrackKind,
    ) -> Result<(), RTCError> {
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
            let receiver = crate::rtp::RTCRtpReceiver::new(
                crate::track::TrackRef::Receiver(crate::track::TrackReceiver::new(
                    format!("sys-recv-{}", mid.as_deref().unwrap_or("")), kind,
                )),
            );
            out.push(crate::rtp::RTCRtpTransceiver::new(
                mid, direction, current, stopped, sender, receiver, kind,
            ));
        }
        Ok(out)
    }

    /// W3C addTransceiver(kind, init) — 无 track 版（消费侧 recvonly 必需）。
    /// libwebrtc 原生 AddTransceiver(MediaType, init) 支持 recvonly 纯接收。
    fn add_transceiver(&self, kind: crate::track::TrackKind, init: &crate::rtp::RTCRtpTransceiverInit) -> Result<crate::rtp::RTCRtpTransceiver, RTCError> {
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
                format!("sys-recv-{}", mid.as_deref().unwrap_or("")), kind2,
            ),
        ));
        // 无 track → 无 sender
        let sender = crate::rtp::RTCRtpSender::new(crate::track::TrackRef::Receiver(
            crate::track::TrackReceiver::new(format!("sys-send-{}", mid.as_deref().unwrap_or("")), kind2),
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

    fn add_transceiver_with_track(&self, track: &crate::track::TrackSender, init: &crate::rtp::RTCRtpTransceiverInit) -> Result<crate::rtp::RTCRtpTransceiver, RTCError> {
        use webrtc_sys::rtp_transceiver::ffi::RtpTransceiverDirection as SysDir;
        // 从 staged 队列取出 media_track（create_track_sender 时 stage 的）
        let media_track = self.callbacks.staged_media_tracks.lock().unwrap().pop()
            .ok_or_else(|| RTCError::Track("no staged media track for add_transceiver_with_track".into()))?;
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
        let tc = self.pc.add_transceiver(media_track, sys_init)
            .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
        let mid = tc.mid().ok();
        let sender = crate::rtp::RTCRtpSender::new(crate::track::TrackRef::Sender(track.clone()));
        let receiver = crate::rtp::RTCRtpReceiver::new(crate::track::TrackRef::Receiver(
            crate::track::TrackReceiver::new(
                format!("sys-recv-{}", mid.as_deref().unwrap_or("")), track.kind,
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

fn sender_get_parameters(&self, track_id: &str) -> Result<crate::rtp::RTCRtpParameters, RTCError> {
// 遍历 transceivers，按 sender track_id 匹配
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

    /// PIT-76: 周期关键帧触发 — cxx 保真往返（override 默认实现）。
    ///
    /// 必须走 cxx 原样往返（不经 map_rtp_parameters 上层映射）：
    /// libwebrtc RtpSenderBase::SetParameters 校验 codecs / encodings 数量 /
    /// transaction_id 与内部存储一致，上层映射丢弃 codec name/kind/rtcp_feedback，
    /// 重建必然 INVALID_MODIFICATION。get 原样 → 只改 request_key_frame → set，
    /// 三个校验全部天然满足。libwebrtc 每次消费后内部清标志：每次调用传 true
    /// 恰好触发一次 GenerateKeyFrame（Oracle 已确认，无需复位）。
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
    /// v2 (encoder-backend-codec-config T1): 编码器后端选择 — SetEncoderSelector 机制。
    /// 遍历 transceivers 匹配 sender.track().id()（request_key_frame 同模式）。
    fn sender_set_video_encoder_backend(&self, track_id: &str, backend: crate::rtp::RTCVideoEncoderBackend) -> Result<(), RTCError> {
        let sys_backend = match backend {
            crate::rtp::RTCVideoEncoderBackend::Auto => webrtc_sys::webrtc::ffi::VideoEncoderBackend::Auto,
            crate::rtp::RTCVideoEncoderBackend::Software => webrtc_sys::webrtc::ffi::VideoEncoderBackend::Software,
            crate::rtp::RTCVideoEncoderBackend::Hardware => webrtc_sys::webrtc::ffi::VideoEncoderBackend::Hardware,
            crate::rtp::RTCVideoEncoderBackend::Nvenc => webrtc_sys::webrtc::ffi::VideoEncoderBackend::Nvenc,
            crate::rtp::RTCVideoEncoderBackend::Vaapi => webrtc_sys::webrtc::ffi::VideoEncoderBackend::Vaapi,
            crate::rtp::RTCVideoEncoderBackend::VideoToolbox => webrtc_sys::webrtc::ffi::VideoEncoderBackend::VideoToolbox,
            crate::rtp::RTCVideoEncoderBackend::PreEncoded => webrtc_sys::webrtc::ffi::VideoEncoderBackend::PreEncoded,
        };
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                sender.set_video_encoder_backend(sys_backend);
                tracing::info!("sender_set_video_encoder_backend({track_id}, {backend:?}) — SetEncoderSelector");
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }
    /// v2 (encoder-bitrate): min/max 码率设置 — cxx 保真往返（request_key_frame 同模式）。
    /// 只改 enc[].min/max_bitrate_bps 字段, 天然满足 SetParameters 的 codecs/encodings
    /// 数量/transaction_id 校验（PIT-76: 禁 lossy roundtrip）。
    /// min 为受限链路 best-effort 下限（libwebrtc 分配层生效, 编码器无硬下限）;
    /// max 为可靠硬上限。None → has_*_bitrate_bps=false（不限制）。
    fn sender_set_encoding_bitrate(&self, track_id: &str, min_bps: Option<u64>, max_bps: Option<u64>) -> Result<(), RTCError> {
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
                tracing::info!("sender_set_encoding_bitrate({track_id}, min={min_bps:?}, max={max_bps:?}) — SetParameters");
                return sender
                    .set_parameters(params)
                    .map_err(|e| RTCError::Internal(e.what().to_owned()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// v2 (qos-framerate-priority): 降级偏好 — cxx 保真往返（bitrate 同模式）。
    /// degradation_preference 在 **RtpParameters 级**（vendor rtp_parameters.rs:191-192，
    /// 非 per-enc）：get 原样 → has/preference 两字段 → set，天然满足 SetParameters
    /// 校验（PIT-76）。Fixed → vendor MaintainFramerateAndResolution。
    fn sender_set_degradation_preference(&self, track_id: &str, pref: crate::rtp::RTCDegradationPreference) -> Result<(), RTCError> {
        let sys_pref = map_pref(pref);
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                let mut params = sender.get_parameters();
                params.has_degradation_preference = true;
                params.degradation_preference = sys_pref;
                tracing::info!("sender_set_degradation_preference({track_id}, {pref:?}) — SetParameters");
                return sender
                    .set_parameters(params)
                    .map_err(|e| RTCError::Internal(e.what().to_owned()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// v2 (qos-framerate-priority): 内容 hint — track 级属性（非 SetParameters）。
    /// sender.track() → media_to_video 下转型（:1444 VideoSink 先例同法）→ set_content_hint；
    /// 非视频 track 转型失败 → Err（调用方 warn，C15）。
    fn sender_set_content_hint(&self, track_id: &str, hint: crate::rtp::RTCRtpContentHint) -> Result<(), RTCError> {
        let sys_hint = map_hint(hint);
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                // track_id 命中即调用方 add_track("video") 建的视频 track（publish 路径保证）；
                // 非视频 track 下转型属未定义行为，故仅 video kind 路径调用（:1490 VideoSink 先例同法）。
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

    /// v2 (multi-stream P1): 编码帧率上限（fps）— cxx 保真往返（bitrate 同模式）。
    /// 只改 enc[].has_max_framerate/max_framerate 字段, 天然满足 SetParameters 的
    /// codecs/encodings 数量/transaction_id 校验（PIT-76）。
    /// libwebrtc 收到 SetParameters 后对编码器 reconfigure（官方行为）;
    /// 帧时间戳由 C17 单调锚定（与 fps 无关）。None → has_max_framerate=false（不限制）。
    fn sender_set_encoding_framerate(&self, track_id: &str, max_fps: Option<f64>) -> Result<(), RTCError> {
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
                tracing::info!("sender_set_encoding_framerate({track_id}, max_fps={max_fps:?}) — SetParameters");
                return sender
                    .set_parameters(params)
                    .map_err(|e| RTCError::Internal(e.what().to_owned()));
            }
        }
        Err(RTCError::Track(format!("sender not found: {track_id}")))
    }

    /// v2 (web-stream-stats T1.5): W3C RTCRtpSender.getStats（出站统计）。
    /// 调 webrtc-sys FFI（RtpSender::get_stats → ToJson, 同步回调）→ 解析 outbound-rtp 字段。
    /// 纯 Rust 零 C++ 改动（Oracle F2: libwebrtc ToJson 已含 framesEncoded/encoderImplementation 等）。
    fn sender_get_stats(&self, track_id: &str) -> Vec<crate::stats::RTCStats> {
        use crate::stats::{RTCStats, RTCOutboundRtpStreamStats};
        for tc in self.pc.get_transceivers() {
            let t = &tc.ptr;
            let sender = t.sender();
            let track = sender.track();
            if track.id() == track_id {
                let (tx, rx) = std::sync::mpsc::sync_channel::<String>(1);
                let ctx = Box::new(webrtc_sys::rtp_sender::SenderContext(Box::new(tx)));
                sender.get_stats(ctx, |ctx, json| {
                    // 同步回调（C++ OnStatsDelivered 同线程）— mpsc 立即投递
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

    fn get_sender_capabilities(&self, kind: TrackKind) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError> {
        let media_type = match kind {
            TrackKind::Video => webrtc_sys::webrtc::ffi::MediaType::Video,
            TrackKind::Audio => webrtc_sys::webrtc::ffi::MediaType::Audio,
        };
        Ok(Some(map_rtp_capabilities(
            self.factory.rtp_sender_capabilities(media_type),
        )))
    }

    fn get_receiver_capabilities(&self, kind: TrackKind) -> Result<Option<crate::rtp::RTCRtpCapabilities>, RTCError> {
        let media_type = match kind {
            TrackKind::Video => webrtc_sys::webrtc::ffi::MediaType::Video,
            TrackKind::Audio => webrtc_sys::webrtc::ffi::MediaType::Audio,
        };
        Ok(Some(map_rtp_capabilities(
            self.factory.rtp_receiver_capabilities(media_type),
        )))
    }

    fn restart_ice(&self) -> Result<(), RTCError> {
        self.pc.restart_ice();
        Ok(())
    }

    fn current_local_description(&self) -> Result<Option<crate::sdp::RTCSessionDescription>, RTCError> {
        let sd = self.pc.current_local_description();
        if sd.is_null() { return Ok(None); }
        Ok(Some(crate::sdp::RTCSessionDescription::new(
            map_sdp_type(sd.sdp_type()),
            sd.stringify(),
        )))
    }

    fn current_remote_description(&self) -> Result<Option<crate::sdp::RTCSessionDescription>, RTCError> {
        let sd = self.pc.current_remote_description();
        if sd.is_null() { return Ok(None); }
        Ok(Some(crate::sdp::RTCSessionDescription::new(
            map_sdp_type(sd.sdp_type()),
            sd.stringify(),
        )))
    }

    fn transceiver_set_direction(&self, mid: &str, dir: crate::rtp::RTCRtpTransceiverDirection) -> Result<(), RTCError> {
        use webrtc_sys::rtp_transceiver::ffi::RtpTransceiverDirection as SysDir;
        let sys_dir = match dir {
            crate::rtp::RTCRtpTransceiverDirection::Sendrecv => SysDir::SendRecv,
            crate::rtp::RTCRtpTransceiverDirection::Sendonly => SysDir::SendOnly,
            crate::rtp::RTCRtpTransceiverDirection::Recvonly => SysDir::RecvOnly,
            crate::rtp::RTCRtpTransceiverDirection::Inactive => SysDir::Inactive,
        };
        for tc in self.pc.get_transceivers() {
            if tc.ptr.mid().ok().as_deref() == Some(mid) {
                tc.ptr.set_direction(sys_dir).map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("transceiver not found: {mid}")))
    }

    fn transceiver_stop(&self, mid: &str) -> Result<(), RTCError> {
        for tc in self.pc.get_transceivers() {
            if tc.ptr.mid().ok().as_deref() == Some(mid) {
                tc.ptr.stop_standard().map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;
                return Ok(());
            }
        }
        Err(RTCError::Track(format!("transceiver not found: {mid}")))
    }

    /// v2 (set-codec-preferences T3+T5 实证修正): W3C setCodecPreferences。
    /// 按 sender.track().id() 匹配 transceiver（协商前 mid 不存在 — offerer 核心场景；
    /// request_key_frame 同模式）。answerer 场景协商后设置对 answer 无效（libwebrtc
    /// 按 offer 序取交集）— T5 实测结论，固定 codec 走 reduceCodecs。
    fn transceiver_set_codec_preferences(&self, track_id: &str, codecs: Vec<crate::rtp::RTCRtpCodecCapability>) -> Result<(), RTCError> {
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
    /// 接收观察者缓存（F1 审查 #1）: register-once — 首次 spool() 注册观察者并
    /// 存入 broadcast Sender；后续 spool() 只 subscribe，绝不再注册。
    /// Clone 共享同一 Arc（同一底层 DataChannel）。
    rx: std::sync::Arc<
        std::sync::OnceLock<tokio::sync::broadcast::Sender<crate::data_channel::RTCDataChannelEvent>>,
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
        // F1 审查 #1（FFI 线程约束）: libwebrtc 要求 RegisterObserver/
        // UnregisterObserver 在其内部信令线程执行（DCHECK_RUN_ON，release 编译掉
        // 后为静默数据竞争），且每次重注册会 reset 前一个观察者（unique_ptr）—
        // 网络线程可能正 mid-OnMessage → rust::Box use-after-free。
        // 故**每个通道只注册一次**（首次 spool()，OnceLock 保证；upstream livekit
        // 在 configure() 一次性注册同模式）；后续 spool() 仅 subscribe 已缓存事件流。
        // 注册线程仍可能是首次调用者（tokio worker）——DCHECK 约束的完全满足需要
        // 把注册调度到信令线程（livekit configure 时点即信令线程），此处以
        // register-once 消除重注册 UAF；跨线程注册治理留给后续专项。
        let tx = self
            .rx
            .get_or_init(|| {
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

/// libwebrtc DataChannel observer：转发消息/状态到 broadcast（Task F1 接收路径）。
/// 回调在 libwebrtc 信令线程触发；仅做无锁 send，快速返回。
/// 注意: 观察者 Box 移交给 C++ 侧持有（register_observer），本 struct 仅借出
/// Sender 克隆 — 观察者生命周期 = C++ unique_ptr，不被 Rust 侧释放。
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
            // ponytail: cxx 非穷尽 enum 未知态兜底（同 state() 惯例）
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
    // PIT-63: 锚定单调 wall-clock 时间戳 — BASE(SystemTime 锚点) + Instant::elapsed()(单调增量)。
    // 裸 SystemTime::now() 非单调 (NTP 跳变/挂起恢复 → ts 倒退 → aligned 倒退 → FramerateController 重置);
    // 假时钟 (+33333us/次) 与 libwebrtc 时间域不一致 (PIT-61 实验史)。
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
        Self { video_source: Mutex::new(None), audio_source: Mutex::new(None), ts_base_us, ts_anchor }
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
        Self { video_source: Mutex::new(video), audio_source: Mutex::new(audio), ts_base_us, ts_anchor }
    }
}

impl WebrtcSysTrack {
    pub(crate) fn with_video_source(
        source: SharedPtr<webrtc_sys::video_track::ffi::VideoTrackSource>,
    ) -> Self {
        let (ts_base_us, ts_anchor) = Self::new_clock_anchor();
        Self { video_source: Mutex::new(Some(source)), audio_source: Mutex::new(None), ts_base_us, ts_anchor }
    }
    pub(crate) fn with_audio_source(
        source: SharedPtr<webrtc_sys::audio_track::ffi::AudioTrackSource>,
    ) -> Self {
        let (ts_base_us, ts_anchor) = Self::new_clock_anchor();
        Self { video_source: Mutex::new(None), audio_source: Mutex::new(Some(source)), ts_base_us, ts_anchor }
    }
}

/// capture_frame 完成回调（queue 模式 backpressure 通知）— H2 无背压需求，no-op。
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
        &self, data: &[u8], width: u32, height: u32, ts_us: Option<i64>,
    ) -> Result<(), RTCError> {
use webrtc_sys::video_frame::ffi as vf;
        use webrtc_sys::video_frame_buffer::ffi as vfb;
        use webrtc_sys::video_track::ffi as vt;

        let source = self.video_source.lock().unwrap()
            .clone()
            .ok_or_else(|| RTCError::Track("video source not initialized".into()))?;

        // PIT-76 诊断: PLI 到达检测 — libwebrtc 收到 PLI 时置位共享 flag
        // （VideoStreamEncoder → RtpVideoSender → EncoderRtcpFeedback 链路）
        if source.take_keyframe_request() {
            tracing::warn!("PLI/KeyFrameRequest 到达 VideoTrackSource (take_keyframe_request=true)");
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
            let y_slice = std::slice::from_raw_parts_mut(
                (*yuv).data_y() as *mut u8, y_size,
            );
            let u_slice = std::slice::from_raw_parts_mut(
                (*yuv).data_u() as *mut u8, uv_size,
            );
            let v_slice = std::slice::from_raw_parts_mut(
                (*yuv).data_v() as *mut u8, uv_size,
            );
            y_slice.copy_from_slice(&data[..y_size]);
            u_slice.copy_from_slice(&data[y_size..y_size + uv_size]);
            v_slice.copy_from_slice(&data[y_size + uv_size..y_size + 2 * uv_size]);
        }

        // Build VideoFrame and push to source
        let mut builder = vf::new_video_frame_builder();
        // PIT-63: 锚定单调 wall-clock — 与 livekit TimestampAligner 期望一致 (帧 ts 与 wall-clock 可比)
        // ts_us 参数: Some(捕获时刻) 相机时间源; None → 内部锚定单调
        let ts_us = ts_us.unwrap_or_else(|| self.ts_base_us + self.ts_anchor.elapsed().as_micros() as i64);
        builder.pin_mut().set_timestamp_us(ts_us);
        builder.pin_mut().set_video_frame_buffer(
            // SAFETY: i420 → yuv8 → yuv → vfb upcast chain
            unsafe { &*vfb::yuv_to_vfb(
                vfb::yuv8_to_yuv(vfb::i420_to_yuv8(&*i420))
            ) },
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
    /// H2: 推 PCM i16（交织）到 AudioTrackSource — libwebrtc 内部 opus 编码后走 RTP。
    /// 帧长必须恰 10ms（48000Hz/100 = 480 样本/通道）— livekit capture_frame 语义。
    fn write_pcm(&self, data: &[u8], cfg: &RTCAudioTrackConfig) -> Result<(), RTCError> {
        let source = self.audio_source.lock().unwrap().clone()
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
        // SAFETY: 调用方（tone generator / e2e）保证 data 来自 i16 对齐缓冲（Vec<i16> 即 2 字节对齐）;
        // 帧长已按 10ms 校验; capture_frame 内部加锁拷贝，无生命周期逃逸。
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
        // 临时诊断（H2）: capture_frame 成功计数
        static PCM_PUSHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = PCM_PUSHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n % 50 == 0 {
            tracing::debug!("audio capture_frame pushed (total {})", n + 1);
        }
        Ok(())
    }
}

// ponytail: WebrtcSysTrack has interior Mutex for VideoTrackSource;
// C++ side handles actual thread safety for on_captured_frame.

// ── RealObserver ──

/// Holds user-registered callbacks and active video sinks.
pub(crate) struct ObserverCallbacks {
    pub on_track: Mutex<Option<Box<dyn Fn(TrackReceiver) + Send + Sync + 'static>>>,
    /// Retain NativeVideoSink references to prevent GC
    pub video_sinks: Mutex<Vec<cxx::SharedPtr<webrtc_sys::video_track::ffi::NativeVideoSink>>>,
    pub on_ice_connection_state_change: Mutex<Option<Box<dyn Fn(RTCIceConnectionState) + Send + Sync + 'static>>>,
    pub on_peer_connection_state_change: Mutex<Option<Box<dyn Fn(RTCPeerConnectionState) + Send + Sync + 'static>>>,
    pub on_ice_candidate: Mutex<Option<Box<dyn Fn(RTCIceCandidate) + Send + Sync + 'static>>>,
    /// v2: incoming data channel callback (通用 on_data_channel)
    pub on_data_channel: Mutex<Option<Box<dyn Fn(crate::data_channel::RTCDataChannel) + Send + Sync + 'static>>>,
    /// Staged media tracks awaiting register_track consumption (webrtc-sys only).
    pub staged_media_tracks: Mutex<Vec<cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>>>,
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
    fn on_ice_connection_change(&self, state: webrtc_sys::peer_connection::ffi::IceConnectionState) {
        let mapped = match state {
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionNew => RTCIceConnectionState::New,
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionChecking => RTCIceConnectionState::Checking,
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionConnected => RTCIceConnectionState::Connected,
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionCompleted => RTCIceConnectionState::Completed,
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionFailed => RTCIceConnectionState::Failed,
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionDisconnected => RTCIceConnectionState::Disconnected,
            webrtc_sys::peer_connection::ffi::IceConnectionState::IceConnectionClosed => RTCIceConnectionState::Closed,
            _ => RTCIceConnectionState::New,
        };
        tracing::info!("ICE connection state changed: {:?}", mapped);
        if let Some(ref cb) = *self.callbacks.on_ice_connection_state_change.lock().unwrap() {
            cb(mapped);
        }
    }
    fn on_standardized_ice_connection_change(&self, _: webrtc_sys::peer_connection::ffi::IceConnectionState) {}
    fn on_connection_change(&self, state: webrtc_sys::peer_connection::ffi::PeerConnectionState) {
        let mapped = match state {
            webrtc_sys::peer_connection::ffi::PeerConnectionState::New => RTCPeerConnectionState::New,
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Connecting => RTCPeerConnectionState::Connecting,
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Connected => RTCPeerConnectionState::Connected,
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Disconnected => RTCPeerConnectionState::Disconnected,
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Failed => RTCPeerConnectionState::Failed,
            webrtc_sys::peer_connection::ffi::PeerConnectionState::Closed => RTCPeerConnectionState::Closed,
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
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringGathering => "gathering",
            webrtc_sys::peer_connection::ffi::IceGatheringState::IceGatheringComplete => "complete",
            _ => "unknown",
        };
        tracing::debug!("ICE gathering state: {}", s);
    }
    fn on_ice_candidate(&self, candidate: cxx::SharedPtr<webrtc_sys::jsep::ffi::IceCandidate>) {
        // PIT-43/52: 完整转发 trickled 本地候选（P2P 路径必需）
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
    fn on_ice_candidates_removed(&self, _: Vec<cxx::SharedPtr<webrtc_sys::candidate::ffi::Candidate>>) {}
    fn on_ice_connection_receiving_change(&self, _: bool) {}
    fn on_ice_selected_candidate_pair_changed(&self, _: webrtc_sys::peer_connection_factory::ffi::CandidatePairChangeEvent) {}
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

        // PIT 诊断: receiver 协商参数（确认 ssrc/PT 是否配置到接收流）
        {
            let params = receiver.get_parameters();
            tracing::info!(
                "webrtc-sys on_track receiver params: codecs={} encodings={} mid={:?}",
                params.codecs.len(), params.encodings.len(), params.mid
            );
            for enc in &params.encodings {
                tracing::info!("  encoding ssrc={:?} rid={:?}", enc.ssrc, enc.rid);
            }
        }

        // PIT 诊断: 远端 track 状态（Live/Ended + enabled）— 判断解码流是否绑定
        let track_state = match track.state() {
            webrtc_sys::media_stream_track::ffi::TrackState::Live => "Live",
            webrtc_sys::media_stream_track::ffi::TrackState::Ended => "Ended",
            _ => "Unknown",
        };
        tracing::info!(
            "webrtc-sys on_track: track_id={} kind={:?} state={} enabled={}",
            track.id(), kind, track_state, track.enabled()
        );

        // Invoke user callback
        if let Some(ref cb) = *self.callbacks.on_track.lock().unwrap() {
            cb(tr.clone());
        }

        // If the user registered a FrameSink, create native VideoSink bridge
        if kind == TrackKind::Video {
            let sink_arc = tr.sink.clone();
            if let Some(_) = *sink_arc.lock().unwrap() {
                let callbacks = self.callbacks.clone();
                #[allow(dead_code)]
                struct VideoSinkAdapter {
                    sink: std::sync::Arc<std::sync::Mutex<Option<Box<dyn crate::track::FrameSink>>>>,
                }
                impl webrtc_sys::video_track::VideoSink for VideoSinkAdapter {
                    fn on_frame(&self, frame: cxx::UniquePtr<vff::VideoFrame>) {
                        tracing::debug!("VideoSinkAdapter::on_frame fired (w={} h={})", frame.width(), frame.height());
                        if let Some(ref sink) = *self.sink.lock().unwrap() {
                            let w = frame.width();
                            let h = frame.height();
                            let buf = unsafe { frame.video_frame_buffer() };
                            let i420 = unsafe { (*buf).to_i420() };
                            let yuv = unsafe { vfb::i420_to_yuv8(&*i420) };
                            let y_size = (w * h) as usize;
                            let uv_size = ((w / 2) * (h / 2)) as usize;
                            let mut data = vec![0u8; y_size + 2 * uv_size];
                            unsafe {
                                std::ptr::copy_nonoverlapping(
                                    (*yuv).data_y(), data.as_mut_ptr(), y_size,
                                );
                                std::ptr::copy_nonoverlapping(
                                    (*yuv).data_u(), data.as_mut_ptr().add(y_size), uv_size,
                                );
                                std::ptr::copy_nonoverlapping(
                                    (*yuv).data_v(), data.as_mut_ptr().add(y_size + uv_size), uv_size,
                                );
                            }
                            sink.on_frame(&data, w, h);
                        }
                    }
                    fn on_discarded_frame(&self) {}
                    fn on_constraints_changed(&self, _: webrtc_sys::video_track::ffi::VideoTrackSourceConstraints) {}
                }

                let adapter = VideoSinkAdapter { sink: sink_arc.clone() };
                let wrapper = webrtc_sys::video_track::VideoSinkWrapper::new(std::sync::Arc::new(adapter));

                // Register sink with the video track
                tracing::debug!("VideoSinkAdapter: attaching native sink to video track");
                unsafe {
                    let video_track = webrtc_sys::video_track::ffi::media_to_video(track);
                    let native_sink = webrtc_sys::video_track::ffi::new_native_video_sink(Box::new(wrapper));
                    video_track.add_sink(&native_sink);
                    callbacks.video_sinks.lock().unwrap().push(native_sink);
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
        // 注册 libwebrtc 内部日志（RTC_LOG → tracing）— PIT-45 调试：ICE gathering 内部状态
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
        let factory =
            webrtc_sys::peer_connection_factory::ffi::create_peer_connection_factory();
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

        let pc = self
            .factory
            .create_peer_connection(rtc_config, Box::new(observer))
            .map_err(|e| RTCError::RTCPeerConnection(e.what().to_owned()))?;

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
    ) -> (
        WebrtcSysTrack,
        cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>,
    ) {
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
    /// 48kHz mono, queue_size_ms=100（容忍 10ms 节奏抖动; 0 = fast path 需严格 10ms）。
    pub(crate) fn create_audio_track(
        &self,
    ) -> (
        WebrtcSysTrack,
        cxx::SharedPtr<webrtc_sys::media_stream_track::ffi::MediaStreamTrack>,
    ) {
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
        // PIT-63: 锚定单调 — 单调递增 + wall-clock 量级 (~1.78e15 µs @2026)
        let (base, anchor) = WebrtcSysTrack::new_clock_anchor();
        let t1 = base + anchor.elapsed().as_micros() as i64;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let t2 = base + anchor.elapsed().as_micros() as i64;
        assert!(t2 > t1, "时间戳必须单调递增");
        assert!(t1 > 1_000_000_000_000, "wall-clock 量级 (µs): {t1}");
    }

    #[test]
    fn map_rtp_parameters_codec_fmtp_mapping() {
        // PIT-54: mediasoup 严格按 codec parameters 匹配 — 验证 map_rtp_parameters 映射 H264 fmtp
        use webrtc_sys::rtp_parameters::ffi::{RtpParameters, RtpCodecParameters, RtpEncodingParameters, RtcpParameters, RtpExtension, StringKeyValue};
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
                has_ssrc: true, ssrc: 1949911776, bitrate_priority: 1.0,
                network_priority: webrtc_sys::webrtc::ffi::Priority::Medium,
                has_max_bitrate_bps: false, max_bitrate_bps: 0,
                has_min_bitrate_bps: false, min_bitrate_bps: 0,
                has_max_framerate: false, max_framerate: 0.0,
                has_num_temporal_layers: false, num_temporal_layers: 0,
                has_scale_resolution_down_by: false, scale_resolution_down_by: 1.0,
                has_scalability_mode: false, scalability_mode: String::new(),
                active: true, rid: String::new(), adaptive_ptime: false,
                request_key_frame: true,  // PIT-76: 验证透传
            }],
            rtcp: RtcpParameters { has_ssrc: false, ssrc: 0, cname: String::new(), reduced_size: true, mux: true },
            has_degradation_preference: false,
            degradation_preference: webrtc_sys::rtp_parameters::ffi::DegradationPreference::Balanced,
        };
        let mapped = map_rtp_parameters(params);
        assert_eq!(mapped.codecs.len(), 1);
        let fmtp = mapped.codecs[0].sdp_fmtp_line.as_deref().unwrap_or("");
        // PIT-54: 必须含 packetization-mode 和 profile-level-id
        assert!(fmtp.contains("packetization-mode=1"), "fmtp: {fmtp}");
        assert!(fmtp.contains("profile-level-id=42e01f"), "fmtp: {fmtp}");
        assert_eq!(mapped.encodings[0].ssrc, Some(1949911776));
        // PIT-76: request_key_frame 字段必须透传
        assert_eq!(mapped.encodings[0].request_key_frame, true);
    }

    #[test]
    fn map_rtp_parameters_request_key_frame_false() {
        // PIT-76: request_key_frame 默认 false 也必须透传（非默认值依赖）
        use webrtc_sys::rtp_parameters::ffi::{RtpParameters, RtcpParameters, RtpEncodingParameters};
        let params = RtpParameters {
            transaction_id: "tx".into(),
            mid: "0".into(),
            codecs: vec![],
            header_extensions: vec![],
            encodings: vec![RtpEncodingParameters {
                has_ssrc: false, ssrc: 0, bitrate_priority: 1.0,
                network_priority: webrtc_sys::webrtc::ffi::Priority::Medium,
                has_max_bitrate_bps: false, max_bitrate_bps: 0,
                has_min_bitrate_bps: false, min_bitrate_bps: 0,
                has_max_framerate: false, max_framerate: 0.0,
                has_num_temporal_layers: false, num_temporal_layers: 0,
                has_scale_resolution_down_by: false, scale_resolution_down_by: 1.0,
                has_scalability_mode: false, scalability_mode: String::new(),
                active: true, rid: String::new(), adaptive_ptime: false,
                request_key_frame: false,
            }],
            rtcp: RtcpParameters { has_ssrc: false, ssrc: 0, cname: String::new(), reduced_size: true, mux: true },
            has_degradation_preference: false,
            degradation_preference: webrtc_sys::rtp_parameters::ffi::DegradationPreference::Balanced,
        };
        let mapped = map_rtp_parameters(params);
        assert_eq!(mapped.encodings[0].request_key_frame, false);
    }

    #[test]
    fn map_codec_capability_to_sys_h264() {
        // v2 (set-codec-preferences T4): W3C → sys — name/kind 必须显式填（mime_type 被 to_native 忽略）
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
        // fmtp 解析: 顺序保留（libwebrtc 精确 map 匹配）
        assert_eq!(sys.parameters.len(), 2);
        assert_eq!(sys.parameters[0].key, "profile-level-id");
        assert_eq!(sys.parameters[0].value, "42e01f");
        assert_eq!(sys.parameters[1].key, "packetization-mode");
        assert_eq!(sys.parameters[1].value, "1");
    }

    #[test]
    fn map_codec_capability_to_sys_audio_and_empty_fmtp() {
        use crate::rtp::RTCRtpCodecCapability;
        // 音频: kind 正确；无 fmtp → parameters 空
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
        // v2 (set-codec-preferences T2): sys → W3C fmtp 还原（字节精确）
        use webrtc_sys::rtp_parameters::ffi::{RtpCapabilities, RtpCodecCapability, RtpHeaderExtensionCapability, StringKeyValue};
        let caps = RtpCapabilities {
            codecs: vec![RtpCodecCapability {
                mime_type: "video/H264".into(),
                name: "H264".into(),
                kind: webrtc_sys::webrtc::ffi::MediaType::Video,
                has_clock_rate: true, clock_rate: 90000,
                has_preferred_payload_type: true, preferred_payload_type: 101,
                has_num_channels: false, num_channels: 0,
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
        // v2 (web-stream-stats T1.5): libwebrtc ToJson outbound-rtp 字段解析
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
                // v3 (encode-time-stats T2): totalEncodeTime 解析 — 120 帧 × 30ms = 3.6s
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
        // v3: totalEncodeTime 缺失 → None（旧版 libwebrtc / 字段不可用）
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
