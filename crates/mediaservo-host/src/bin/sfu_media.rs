//! SFU 媒体协商的纯函数构造 — 从 main.rs 抽出以便单测。
//!
//! P3 (v2)+P2 修复: Host 走标准 answerer 协商 — 用 server transport 参数构造 remote SDP
//! → set_remote_description → create_answer (对齐 libmediasoupclient SdpUtils / e2e_sfu 验证路径)。
//! 本模块提供 remote SDP 构造 + 从协商结果 RTCRtpParameters 构造 produce 请求。

use mediaservo_common::protocol::{DtlsParameters, IceCandidate, IceParameters};
use mediaservo_webrtc::rtp::RTCRtpParameters;
use serde_json::{Value, json};

/// 用 mediasoup transport 参数构造 remote SDP (ICE-Lite server offer)。
/// PIT-48: a=candidate 行必须位于 m= 行之后（media section 内）——
/// 会话级 candidate 被 libwebrtc 忽略 → remote candidate 丢失 → ICE 不发起 STUN
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
        // v3 (sfu-negotiation-completion T1): extmap 声明 transport-cc + abs-capture-time —
        // BWE 反馈链路必需（mediasoup 端按 produce headerExtensions 映射, 生成/转发
        // transport-cc feedback 给 host）; id 3/5 对齐官方 libmediasoupclient 惯例。
        // RFC 8285: answer 只能收 offer 集合 → 自构 offer 必须先声明。
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
            .to_string(),
        "a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time".to_string(),
        "a=recvonly".to_string(),
        format!("a=rtpmap:{} {}/{}", payload_type, codec_name, clock_rate),
        // rtcp-fb: 完整协商语义（nack/pli/fir）— libwebrtc answerer 回带,
        // 丢包重传 + 关键帧请求正式声明。
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

/// P3 (v2): 从协商结果 (RTCRtpParameters) 构造 mediasoup produce 的 rtp_parameters。
/// 对齐官方客户端 — 数据来自 transceiver.sender.get_parameters()，非手工硬编码。
pub fn build_produce_rtp_parameters_from_rtp(params: &RTCRtpParameters) -> Value {
    let codecs: Vec<Value> = params
        .codecs
        .iter()
        .map(|c| {
            // v2 (encoder-backend-codec-config T4 实证): H264 必须带 parameters（PIT-54 严格匹配）—
            // VP8 router parameters 为空侥幸匹配; H264 router 有 profile/packetization 参数, 缺失必败
            // (Unsupported codec). sdp_fmtp_line "k=v;k=v" → mediasoup parameters JSON。
            let parameters: Value = c
                .sdp_fmtp_line
                .as_deref()
                .map(|line| {
                    let mut map = serde_json::Map::new();
                    for kv in line.split(';') {
                        if let Some((k, v)) = kv.split_once('=') {
                            // 数字参数转 number（mediasoup 参数类型敏感）
                            let val: Value =
                                v.parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| json!(v));
                            map.insert(k.trim().to_string(), val);
                        }
                    }
                    Value::Object(map)
                })
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            // v3 (sfu-negotiation-completion T4): rtcpFeedback 必须声明 transport-cc —
            // mediasoup Transport.cpp:699-724 TCCS 启用条件 = headerExtension transportWideCc01
            // + codecs rtcpFeedback 含 "transport-cc"（缺失 → 不生成 transport-cc feedback
            // → host BWE 无输入）。nack/pli/fir 与 host answer 协商结果一致。
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
    // v3 (sfu-negotiation-completion T2): headerExtensions 从协商结果推导（非硬编码 []）—
    // T1 自构 offer 声明 transport-cc 后, answer 协商成功 → sender.get_parameters()
    // header_extensions 含 transport-cc → mediasoup 端获得 transport-cc 上下文,
    // 生成/转发 feedback 给 host（BWE 自适应链路）。
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
        // 协商结果: ssrc + codec 字段来自 get_sending_rtp_parameters
        assert_eq!(v["codecs"][0]["mimeType"], "video/H264");
        assert_eq!(v["codecs"][0]["payloadType"], 101);
        assert_eq!(v["codecs"][0]["clockRate"], 90000);
        assert_eq!(v["encodings"][0]["ssrc"], 1949911776);
        // v3 (T4): rtcpFeedback 含 transport-cc（mediasoup TCCS 启用条件）
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
        // v3 (sfu-negotiation-completion T2): transport-cc extmap 协商后 → produce
        // headerExtensions 反射（不再硬编码 []）, mediasoup 获得 transport-cc 上下文。
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
        // nack/pli/fir rtcp-fb — PT 动态跟随 payload_type。
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
        // H264 (101): rtcp-fb 行必须跟随协商 PT, 非硬编码 96
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
