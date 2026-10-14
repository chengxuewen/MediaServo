//! SFU media negotiation pure functions (mirrors host sfu_media.rs, aligned with the official answerer path per C18).
//!
//! Standard answerer negotiation (mirrors libmediasoupclient SdpUtils / the host e2e_sfu verified path):
//! build the remote SDP from mediasoup transport params → set_remote_description →
//! create_answer; produce's rtp_parameters derive from the negotiation result (get_sending_rtp_parameters),
//! not hand-hardcoded.

use mediaservo_common::protocol::{DtlsParameters, IceCandidate, IceParameters};
use mediaservo_webrtc::rtp::RTCRtpParameters;
use serde_json::{Value, json};

/// Codec spec (SDP PT/name/clock/fmtp; aligned with the mediasoup router defaults —
/// sfu.rs default_router_options: VP8 96 / H264 101 / VP9 99 / AV1 97）。
pub struct CodecSpec {
    pub payload_type: u16,
    pub name: &'static str,
    pub clock_rate: u32,
    pub fmtp: Option<&'static str>,
}

/// Resolve the SDP spec by configured codec name; unknown falls back to VP8 (VP8 first in the router order).
pub fn codec_spec(codec: &str) -> CodecSpec {
    match codec {
        "h264" => CodecSpec {
            payload_type: 101,
            name: "H264",
            clock_rate: 90000,
            fmtp: Some("profile-level-id=42e01f;packetization-mode=1"),
        },
        "vp9" => CodecSpec { payload_type: 99, name: "VP9", clock_rate: 90000, fmtp: None },
        "av1" => CodecSpec { payload_type: 97, name: "AV1", clock_rate: 90000, fmtp: None },
        _ => CodecSpec { payload_type: 96, name: "VP8", clock_rate: 90000, fmtp: None },
    }
}

/// Build the remote SDP from mediasoup transport params (ICE-Lite server offer).
/// PIT-48: a=candidate lines must come after the m= lines (inside the media section).
/// Media direction of the remote SDP.
/// - `ServerSendonly` (default): server sends, local receives (Pull consumer side)
/// - `ServerRecvonly` (push): server receives, local sends (Push publisher side)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDirection {
    /// Server-side a=sendonly (we receive)
    ServerSendonly,
    /// Server-side a=recvonly (we send)
    ServerRecvonly,
}

#[allow(clippy::too_many_arguments)] // C18 legacy debt (P1 standard-negotiation rewrite on record); the debt war does not refactor
pub fn build_remote_sdp(
    ice_parameters: &IceParameters,
    dtls_parameters: &DtlsParameters,
    ice_candidates: Option<&Vec<IceCandidate>>,
    payload_type: u16,
    codec_name: &str,
    clock_rate: u32,
    fmtp: Option<&str>,
    direction: RemoteDirection,
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
        match direction {
            RemoteDirection::ServerSendonly => "a=group:BUNDLE 0".to_string(),
            RemoteDirection::ServerRecvonly => "a=group:BUNDLE video".to_string(),
        },
        "a=ice-lite".to_string(),
        format!("a=ice-ufrag:{}", ice_parameters.username_fragment),
        format!("a=ice-pwd:{}", ice_parameters.password),
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".to_string(),
    ];

    lines.extend_from_slice(&[
        format!("m=video 7 UDP/TLS/RTP/SAVPF {}", payload_type),
        format!("c=IN IP4 {}", conn_ip),
        "a=rtcp-mux".to_string(),
        "a=rtcp-rsize".to_string(), // matches the libmediasoupclient recv offer (rtcp reduced size)
        match direction {
            // The mediasoup consumer's RTP mid is fixed to "0" — the receiving-side answer must match
            // (mismatch → libwebrtc demux drops RTP → no frames received)
            RemoteDirection::ServerSendonly => "a=mid:0".to_string(),
            RemoteDirection::ServerRecvonly => "a=mid:video".to_string(),
        },
        // extmap declarations (aligned with mediasoup defaults):
        //   id=1 mid (critical for BUNDLE demux — consumer RTP carries the mid extension; without declaring it libwebrtc cannot parse)
        //   id=3 transport-cc, id=5 abs-capture-time (required for the BWE feedback loop)
        "a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid".to_string(),
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
            .to_string(),
        "a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time".to_string(),
        match direction {
            RemoteDirection::ServerSendonly => "a=sendonly".to_string(),
            RemoteDirection::ServerRecvonly => "a=recvonly".to_string(),
        },
        format!("a=rtpmap:{} {}/{}", payload_type, codec_name, clock_rate),
        format!("a=rtcp-fb:{} nack", payload_type),
        format!("a=rtcp-fb:{} nack pli", payload_type),
        format!("a=rtcp-fb:{} ccm fir", payload_type),
    ]);

    if let Some(fmtp_val) = fmtp {
        lines.push(format!("a=fmtp:{} {}", payload_type, fmtp_val));
    }

    if let Some(candidates) = ice_candidates {
        for c in candidates {
            if c.ip.contains(".local") {
                continue;
            } // skip mDNS
            let ctype = match c.candidate_type.as_str() {
                "host" => "host",
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

/// Inject the ssrc from the consumer's rtp_parameters (returned by Consumed) into the remote SDP.
/// libwebrtc demux needs the remote offer to declare the received ssrc, otherwise RTP is dropped.
pub fn inject_remote_ssrc(remote_sdp: &str, consumer_rtp: &serde_json::Value) -> String {
    let ssrc = consumer_rtp
        .get("encodings")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .and_then(|enc| enc.get("ssrc"))
        .and_then(|s| s.as_u64());
    let Some(ssrc) = ssrc else {
        return remote_sdp.to_string();
    };
    // Append a=ssrc lines after the a=mid line (keep the original CRLF separators, avoid breaking the SDP)
    let sep = if remote_sdp.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = remote_sdp.split(sep).collect();
    let mut out = Vec::with_capacity(lines.len() + 2);
    let mut injected = false;
    for line in lines {
        out.push(line.to_string());
        if !injected && line.starts_with("a=mid:") {
            out.push(format!("a=ssrc:{ssrc} cname:mediaservo-pull"));
            out.push(format!("a=ssrc:{ssrc} msid:pull video"));
            injected = true;
        }
    }
    if !injected {
        // No mid line (anomaly) → append at the end
        out.push(format!("a=ssrc:{ssrc} cname:mediaservo-pull"));
    }
    out.join(sep)
}

/// Build mediasoup produce rtp_parameters from the negotiation result (RTCRtpParameters).
/// Data comes from transceiver.sender.get_parameters(), not hand-hardcoded (C18/P3 v2).
pub fn build_produce_rtp_parameters_from_rtp(params: &RTCRtpParameters) -> Value {
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
            // rtcpFeedback must declare transport-cc (mediasoup TCCS enable condition)
            json!({
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
            })
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
            if let Some(max_bitrate) = e.max_bitrate {
                enc["maxBitrate"] = json!(max_bitrate);
            }
            enc
        })
        .collect();
    let header_extensions: Vec<Value> = params
        .header_extensions
        .iter()
        .map(|h| {
            json!({
                "uri": h.uri,
                "id": h.id,
                "encrypt": h.encrypted,
            })
        })
        .collect();
    json!({
        "codecs": codecs,
        "headerExtensions": header_extensions,
        "encodings": encodings,
        "rtcp": {"reducedSize": params.rtcp.reduced_size},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::protocol::Fingerprint;
    use mediaservo_webrtc::rtp::{
        RTCRtcpParameters, RTCRtpCodecParameters, RTCRtpEncodingParameters,
    };

    #[test]
    fn codec_spec_maps_all_supported() {
        assert_eq!(codec_spec("h264").payload_type, 101);
        assert_eq!(codec_spec("vp8").payload_type, 96);
        assert_eq!(codec_spec("vp9").payload_type, 99);
        assert_eq!(codec_spec("av1").payload_type, 97);
        assert_eq!(codec_spec("unknown").payload_type, 96, "fallback VP8");
    }

    #[test]
    fn produce_from_rtp_includes_negotiated_ssrc() {
        let params = RTCRtpParameters {
            transaction_id: "tx1".into(),
            mid: "0".into(),
            codecs: vec![RTCRtpCodecParameters {
                mime_type: "video/H264".into(),
                payload_type: 101,
                clock_rate: 90000,
                channels: None,
                sdp_fmtp_line: Some("profile-level-id=42e01f".into()),
            }],
            encodings: vec![RTCRtpEncodingParameters {
                ssrc: Some(1949911776),
                active: true,
                max_bitrate: Some(2_000_000),
                ..Default::default()
            }],
            header_extensions: vec![],
            rtcp: RTCRtcpParameters { cname: None, reduced_size: true },
        };
        let v = build_produce_rtp_parameters_from_rtp(&params);
        assert_eq!(v["codecs"][0]["mimeType"], "video/H264");
        assert_eq!(v["encodings"][0]["ssrc"], 1949911776);
        let fb: Vec<&str> = v["codecs"][0]["rtcpFeedback"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["type"].as_str())
            .collect();
        assert!(fb.contains(&"transport-cc"), "must declare transport-cc: {fb:?}");
        assert_eq!(v["encodings"][0]["maxBitrate"], 2_000_000);
        assert_eq!(v["rtcp"]["reducedSize"], true);
    }

    #[test]
    fn remote_sdp_has_extmap_and_rtcp_fb() {
        let dtls = DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB:CC:DD".into(),
            }],
            role: "client".into(),
        };
        let sdp = build_remote_sdp(
            &IceParameters { username_fragment: "ufrag".into(), password: "pwd".into() },
            &dtls,
            None,
            96,
            "VP8",
            90000,
            None,
            RemoteDirection::ServerRecvonly,
        );
        assert!(sdp.contains(
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
        ));
        assert!(sdp.contains("a=rtcp-fb:96 nack pli"));
        assert!(sdp.contains("a=rtcp-fb:96 ccm fir"));
        assert!(sdp.contains("a=ice-lite"));
    }
}
