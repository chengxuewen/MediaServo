//! H2 audio-meeting shared utilities (shared by the host-audio process and audio e2e):
//! opus remote SDP construction, produce rtp_parameters construction, tone PCM generation.
//!
//! Pure functions, no I/O -- kept unit-testable (C18: aligned with the official answerer negotiation path).

use mediaservo_common::protocol::{DtlsParameters, IceCandidate, IceParameters};
use mediaservo_webrtc::rtp::RTCRtpParameters;
use serde_json::{Value, json};

/// Standard opus PT (server default_router_options: opus 111/48000/2ch).
pub const OPUS_PT: u16 = 111;
/// Sample rate (libwebrtc AudioTrackSource is fixed at 48kHz).
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples per 10ms frame (48kHz / 100).
pub const FRAME_SAMPLES: usize = 480;

/// Builds the audio remote SDP from mediasoup transport parameters (opus, ICE-Lite server).
/// `sendonly` = server sends and local receives (consumer side); `recvonly` = server receives and local sends (producer side).
/// `ssrc` is injected only for sendonly (the consumer's encodings[0].ssrc -- required for libwebrtc demux).
pub fn build_remote_audio_sdp(
    ice_parameters: &IceParameters,
    dtls_parameters: &DtlsParameters,
    ice_candidates: Option<&Vec<IceCandidate>>,
    sendonly: bool,
    ssrc: Option<u64>,
    mid: &str,
) -> String {
    let fp = &dtls_parameters.fingerprints[0];
    let conn_ip = ice_candidates
        .and_then(|cs| cs.iter().find(|c| !c.ip.contains(".local")))
        .map(|c| c.ip.clone())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let mut lines = vec![
        "v=0".to_string(),
        "o=- 0 0 IN IP4 0.0.0.0".to_string(),
        "s=-".to_string(),
        "t=0 0".to_string(),
        format!("a=group:BUNDLE {mid}"),
        "a=ice-lite".to_string(),
        format!("a=ice-ufrag:{}", ice_parameters.username_fragment),
        format!("a=ice-pwd:{}", ice_parameters.password),
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".to_string(),
        format!("m=audio 7 UDP/TLS/RTP/SAVPF {OPUS_PT}"),
        format!("c=IN IP4 {conn_ip}"),
        "a=rtcp-mux".to_string(),
        "a=rtcp-rsize".to_string(),
        format!("a=mid:{mid}"),
        "a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid".to_string(),
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
            .to_string(),
        if sendonly { "a=sendonly" } else { "a=recvonly" }.to_string(),
        format!("a=rtpmap:{OPUS_PT} opus/{SAMPLE_RATE}/2"),
        format!("a=fmtp:{OPUS_PT} minptime=10;useinbandfec=1"),
    ];
    if sendonly && let Some(ssrc) = ssrc {
        lines.push(format!("a=ssrc:{ssrc} cname:mediaservo-audio"));
        lines.push(format!("a=ssrc:{ssrc} msid:audio audio"));
    }
    if let Some(candidates) = ice_candidates {
        for c in candidates {
            if c.ip.contains(".local") {
                continue;
            }
            let ctype = match c.candidate_type.as_str() {
                "srflx" => "srflx",
                "prflx" => "prflx",
                "relay" => "relay",
                _ => "host",
            };
            lines.push(format!(
                "a=candidate:{} 1 {} {} {} {} typ {}",
                c.foundation,
                c.protocol.to_uppercase(),
                c.priority,
                c.ip,
                c.port,
                ctype
            ));
        }
    }
    lines.push("a=end-of-candidates".to_string());
    lines.push(String::new());
    lines.join("\r\n")
}

/// Builds mediasoup produce rtp_parameters from the negotiation result (RTCRtpParameters).
/// opus must carry channels (mediasoup codec matching includes channels -- 2ch matches the router).
pub fn build_audio_produce_rtp_parameters(params: &RTCRtpParameters) -> Value {
    let codecs: Vec<Value> = params
        .codecs
        .iter()
        .map(|c| {
            let parameters: Value = c
                .sdp_fmtp_line
                .as_deref()
                .map(|line| {
                    let mut map = serde_json::Map::new();
                    for kv in line.split(';') {
                        if let Some((k, v)) = kv.split_once('=') {
                            let val: Value =
                                v.parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| json!(v));
                            map.insert(k.trim().to_string(), val);
                        }
                    }
                    Value::Object(map)
                })
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            let mut codec = json!({
                "mimeType": c.mime_type,
                "payloadType": c.payload_type,
                "clockRate": c.clock_rate,
                "parameters": parameters,
                "rtcpFeedback": [
                    {"type": "nack", "parameter": ""},
                    {"type": "nack", "parameter": "pli"},
                    {"type": "ccm", "parameter": "fir"},
                    {"type": "transport-cc", "parameter": ""},
                ],
            });
            if let Some(ch) = c.channels {
                codec["channels"] = json!(ch);
            }
            codec
        })
        .collect();
    let encodings: Vec<Value> = params
        .encodings
        .iter()
        .map(|e| {
            let mut enc = json!({});
            if let Some(ssrc) = e.ssrc {
                enc["ssrc"] = json!(ssrc);
            }
            enc
        })
        .collect();
    let header_extensions: Vec<Value> = params
        .header_extensions
        .iter()
        .map(|h| json!({"uri": h.uri, "id": h.id, "encrypt": h.encrypted}))
        .collect();
    json!({
        "codecs": codecs,
        "headerExtensions": header_extensions,
        "encodings": encodings,
        "rtcp": {"reducedSize": params.rtcp.reduced_size},
    })
}

/// Sine tone frame (10ms i16 mono PCM, LE byte order; freq_hz is any frequency).
/// Real payload -> opus will not silence-compress (prevents DTX zero-packets); a synthetic source standing in for a hardware microphone (stub source).
pub fn tone_frame(phase: &mut f64, freq_hz: f64) -> Vec<u8> {
    let step = 2.0 * std::f64::consts::PI * freq_hz / SAMPLE_RATE as f64;
    let mut out = Vec::with_capacity(FRAME_SAMPLES * 2);
    for _ in 0..FRAME_SAMPLES {
        let sample = (*phase).sin() * 0.1 * i16::MAX as f64;
        out.extend_from_slice(&(sample as i16).to_le_bytes());
        *phase += step;
        if *phase > 2.0 * std::f64::consts::PI {
            *phase -= 2.0 * std::f64::consts::PI;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::protocol::Fingerprint;
    use mediaservo_webrtc::rtp::{
        RTCRtcpParameters, RTCRtpCodecParameters, RTCRtpEncodingParameters,
    };

    #[test]
    fn remote_sdp_recvonly_has_opus() {
        let dtls = DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB:CC".into(),
            }],
            role: "client".into(),
        };
        let sdp = build_remote_audio_sdp(
            &IceParameters { username_fragment: "u".into(), password: "p".into() },
            &dtls,
            None,
            false,
            None,
            "audio",
        );
        assert!(sdp.contains("m=audio 7 UDP/TLS/RTP/SAVPF 111"));
        assert!(sdp.contains("a=rtpmap:111 opus/48000/2"));
        assert!(sdp.contains("a=recvonly"));
        assert!(!sdp.contains("a=ssrc:"));
    }

    #[test]
    fn remote_sdp_sendonly_injects_ssrc() {
        let dtls = DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB:CC".into(),
            }],
            role: "client".into(),
        };
        let sdp = build_remote_audio_sdp(
            &IceParameters { username_fragment: "u".into(), password: "p".into() },
            &dtls,
            None,
            true,
            Some(3721454228),
            "0",
        );
        assert!(sdp.contains("a=sendonly"));
        assert!(sdp.contains("a=ssrc:3721454228 cname:mediaservo-audio"));
        assert!(sdp.contains("a=mid:0"));
    }

    #[test]
    fn produce_params_include_opus_channels() {
        let params = RTCRtpParameters {
            transaction_id: "tx".into(),
            mid: "audio".into(),
            codecs: vec![RTCRtpCodecParameters {
                mime_type: "audio/opus".into(),
                payload_type: 111,
                clock_rate: 48000,
                channels: Some(2),
                sdp_fmtp_line: Some("minptime=10;useinbandfec=1".into()),
            }],
            encodings: vec![RTCRtpEncodingParameters {
                ssrc: Some(3721454228),
                ..Default::default()
            }],
            header_extensions: vec![],
            rtcp: RTCRtcpParameters { cname: None, reduced_size: true },
        };
        let v = build_audio_produce_rtp_parameters(&params);
        assert_eq!(v["codecs"][0]["mimeType"], "audio/opus");
        assert_eq!(v["codecs"][0]["channels"], 2, "opus must carry channels (mediasoup matching)");
        assert_eq!(v["encodings"][0]["ssrc"], 3_721_454_228u64);
    }

    #[test]
    fn tone_frame_is_10ms_mono_pcm() {
        let mut phase = 0.0;
        let frame = tone_frame(&mut phase, 440.0);
        assert_eq!(frame.len(), FRAME_SAMPLES * 2);
        assert_ne!(frame, vec![0u8; FRAME_SAMPLES * 2], "tone is non-zero payload");
        let mut phase2 = 0.0;
        tone_frame(&mut phase2, 880.0);
        assert!(phase2 > 0.0, "phase advances");
    }
}
