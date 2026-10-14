//! Pure construction functions for SFU media negotiation -- extracted from main.rs so they can be unit tested.
//!
//! P3 (v2) + P2 fix: the Host goes through the standard answerer negotiation -- build the remote SDP from the server transport parameters
//! -> set_remote_description -> create_answer (aligned with the libmediasoupclient SdpUtils / e2e_sfu verification path).
//! This module provides the remote SDP construction + building the produce request from the negotiated RTCRtpParameters.

use mediaservo_common::protocol::{DtlsParameters, IceCandidate, IceParameters};
use mediaservo_webrtc::rtp::RTCRtpParameters;
use serde_json::{Value, json};

/// Build the remote SDP (ICE-Lite server offer) from the mediasoup transport parameters.
/// PIT-48: the a=candidate lines must come after the m= line (inside the media section) --
/// a session-level candidate is ignored by libwebrtc -> the remote candidates are lost -> ICE never starts STUN
pub fn build_remote_sdp(
    ice_parameters: &IceParameters,
    dtls_parameters: &DtlsParameters,
    ice_candidates: Option<&Vec<IceCandidate>>,
    payload_type: u16,
    codec_name: &str,
    clock_rate: u32,
    fmtp: Option<&str>,
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
        "a=group:BUNDLE video".to_string(),
        "a=ice-lite".to_string(),
        format!("a=ice-ufrag:{}", ice_parameters.username_fragment),
        format!("a=ice-pwd:{}", ice_parameters.password),
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".to_string(), // ICE-Lite responder expects client to initiate
    ];

    lines.extend_from_slice(&[
        format!("m=video 7 UDP/TLS/RTP/SAVPF {}", payload_type),
        format!("c=IN IP4 {}", conn_ip),
        "a=rtcp-mux".to_string(),
        "a=mid:video".to_string(),
        // v3 (sfu-negotiation-completion T1): declare the transport-cc + abs-capture-time extmap --
        // required for the BWE feedback path (the mediasoup side maps the produce headerExtensions, generates/forwards
        // the transport-cc feedback to the host); ids 3/5 follow the official libmediasoupclient convention.
        // RFC 8285: an answer can only contain the offer's set -> a self-built offer must declare them first.
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
            .to_string(),
        "a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time".to_string(),
        "a=recvonly".to_string(),
        format!("a=rtpmap:{} {}/{}", payload_type, codec_name, clock_rate),
        // rtcp-fb: the full negotiation semantics (nack/pli/fir) -- carried back by the libwebrtc answerer,
        // formally declaring packet retransmission and key-frame requests.
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

/// P3 (v2): build the mediasoup produce rtp_parameters from the negotiation result (RTCRtpParameters).
/// Aligned with the official client -- the data comes from transceiver.sender.get_parameters(), not from hand-coded constants.
pub fn build_produce_rtp_parameters_from_rtp(params: &RTCRtpParameters) -> Value {
    let codecs: Vec<Value> = params
        .codecs
        .iter()
        .map(|c| {
            // v2 (encoder-backend-codec-config T4 evidence): H264 must carry parameters (PIT-54 strict match) --
            // the empty VP8 router parameters matched by luck; the H264 router has profile/packetization parameters, and omitting them always fails
            // (Unsupported codec). sdp_fmtp_line "k=v;k=v" -> the mediasoup parameters JSON.
            let parameters: Value = c
                .sdp_fmtp_line
                .as_deref()
                .map(|line| {
                    let mut map = serde_json::Map::new();
                    for kv in line.split(';') {
                        if let Some((k, v)) = kv.split_once('=') {
                            // Convert numeric parameters to numbers (mediasoup is sensitive to the parameter type)
                            let val: Value =
                                v.parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| json!(v));
                            map.insert(k.trim().to_string(), val);
                        }
                    }
                    Value::Object(map)
                })
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            // v3 (sfu-negotiation-completion T4): rtcpFeedback must declare transport-cc --
            // the mediasoup Transport.cpp:699-724 TCCS enable condition = the headerExtension transportWideCc01
            // + codecs rtcpFeedback containing "transport-cc" (omitting it -> no transport-cc feedback is generated
            // -> the host BWE has no input). nack/pli/fir match the host answer negotiation result.
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
    // v3 (sfu-negotiation-completion T2): derive headerExtensions from the negotiation result (not a hard-coded []) --
    // after T1's self-built offer declares transport-cc, the answer negotiation succeeds -> sender.get_parameters()
    // header_extensions includes transport-cc -> the mediasoup side gets the transport-cc context and
    // generates/forwards the feedback to the host (the adaptive BWE path).
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
        // Negotiation result: the ssrc + codec fields come from get_sending_rtp_parameters
        assert_eq!(v["codecs"][0]["mimeType"], "video/H264");
        assert_eq!(v["codecs"][0]["payloadType"], 101);
        assert_eq!(v["codecs"][0]["clockRate"], 90000);
        assert_eq!(v["encodings"][0]["ssrc"], 1949911776);
        // v3 (T4): rtcpFeedback includes transport-cc (the mediasoup TCCS enable condition)
        let fb: Vec<&str> = v["codecs"][0]["rtcpFeedback"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["type"].as_str())
            .collect();
        assert!(fb.contains(&"transport-cc"), "rtcpFeedback must declare transport-cc: {fb:?}");
        assert_eq!(v["encodings"][0]["maxBitrate"], 2_000_000);
        assert_eq!(v["rtcp"]["reducedSize"], true);
    }

    #[test]
    fn produce_from_rtp_empty_encodings() {
        let params = RTCRtpParameters::default();
        let v = build_produce_rtp_parameters_from_rtp(&params);
        assert!(v["codecs"].as_array().unwrap().is_empty());
        assert!(v["encodings"].as_array().unwrap().is_empty());
    }

    #[test]
    fn produce_from_rtp_reflects_header_extensions() {
        // v3 (sfu-negotiation-completion T2): after the transport-cc extmap negotiation, the produce
        // headerExtensions reflects it (no longer hard-coded []), so mediasoup gets the transport-cc context.
        let params = RTCRtpParameters {
            transaction_id: "tx1".into(),
            mid: "0".into(),
            codecs: vec![RTCRtpCodecParameters {
                mime_type: "video/VP8".into(),
                payload_type: 96,
                clock_rate: 90000,
                channels: None,
                sdp_fmtp_line: None,
            }],
            encodings: vec![],
            header_extensions: vec![mediaservo_webrtc::rtp::RTCRtpHeaderExtensionParameters {
                uri: "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
                    .into(),
                id: 3,
                encrypted: false,
            }],
            rtcp: RTCRtcpParameters { cname: None, reduced_size: true },
        };
        let v = build_produce_rtp_parameters_from_rtp(&params);
        let he = v["headerExtensions"].as_array().unwrap();
        assert_eq!(he.len(), 1);
        assert_eq!(
            he[0]["uri"],
            "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
        );
        assert_eq!(he[0]["id"], 3);
        assert_eq!(he[0]["encrypt"], false);
    }

    #[test]
    fn remote_sdp_has_extmap_and_rtcp_fb() {
        // v3 (sfu-negotiation-completion T1): transport-cc + abs-capture-time extmap,
        // nack/pli/fir rtcp-fb -- the PT follows the negotiated payload_type dynamically.
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
        );
        assert!(sdp.contains(
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
        ));
        assert!(
            sdp.contains(
                "a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time"
            )
        );
        assert!(sdp.contains("a=rtcp-fb:96 nack"));
        assert!(sdp.contains("a=rtcp-fb:96 nack pli"));
        assert!(sdp.contains("a=rtcp-fb:96 ccm fir"));
    }

    #[test]
    fn remote_sdp_rtcp_fb_pt_follows_payload_type() {
        // H264 (101): the rtcp-fb lines must follow the negotiated PT, not a hard-coded 96
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
            101,
            "H264",
            90000,
            Some("profile-level-id=42e01f;packetization-mode=1"),
        );
        assert!(sdp.contains("a=rtcp-fb:101 nack pli"));
        assert!(sdp.contains("a=rtcp-fb:101 ccm fir"));
        assert!(!sdp.contains("a=rtcp-fb:96"));
    }
}
