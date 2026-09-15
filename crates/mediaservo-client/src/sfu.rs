//! SFU 纯函数——SDP 构造 + codec 解析 + DC 通道参数。
//!
//! 镜像 field::sfu + controller::channel_init（C12 dep boundary：client 禁依 field/host）。
//! 签名钉 server 已验证形状（unit test 交叉）。

use mediaservo_common::protocol::{
    DtlsParameters, IceCandidate, IceParameters, SctpStreamParameters,
};
use mediaservo_webrtc::data_channel::RTCDataChannelInit;
use serde_json::Value;

// ── DC 通道参数（mirror controller::channel_init）──────────

/// 出程 DC 可靠性参数（D-H3：gimbal = partial-reliable，其余 = reliable ordered）。
#[must_use]
pub fn channel_init(label: &str) -> RTCDataChannelInit {
    match label {
        "gimbal" => RTCDataChannelInit {
            ordered: false,
            max_retransmits: Some(5),
            ..Default::default()
        },
        _ => RTCDataChannelInit::default(),
    }
}

/// DC 的 SCTP 流参数（mirror controller::sctp_stream_params）。
/// stream_id 取 libwebrtc 实配 DC id。
pub fn sctp_stream_params(label: &str, id: i32) -> Result<SctpStreamParameters, String> {
    let stream_id =
        u16::try_from(id).map_err(|_| format!("DC {label} id={id} unavailable as SCTP stream_id"))?;
    let init = channel_init(label);
    Ok(SctpStreamParameters {
        stream_id,
        ordered: init.ordered,
        max_packet_life_time: init.max_retransmit_time.and_then(|v| u16::try_from(v).ok()),
        max_retransmits: init.max_retransmits.and_then(|v| u16::try_from(v).ok()),
    })
}

// ── DC-only remote SDP（mirror controller::build_dc_remote_sdp）──────────

/// 用 mediasoup transport 参数合成 DC-only remote offer（application m-line）。
pub fn build_dc_remote_sdp(
    ice: &IceParameters,
    dtls: &DtlsParameters,
    candidates: Option<&Vec<IceCandidate>>,
) -> String {
    let Some(fp) = dtls.fingerprints.first() else {
        return String::new();
    };
    let conn_ip = candidates
        .and_then(|cs| cs.iter().find(|c| !c.ip.contains(".local")))
        .map(|c| c.ip.clone())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let mut lines = vec![
        "v=0".into(),
        "o=- 0 0 IN IP4 0.0.0.0".into(),
        "s=-".into(),
        "t=0 0".into(),
        "a=group:BUNDLE data".into(),
        "a=ice-lite".into(),
        format!("a=ice-ufrag:{}", ice.username_fragment),
        format!("a=ice-pwd:{}", ice.password),
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".into(),
        "m=application 9 UDP/DTLS/SCTP webrtc-datachannel".into(),
        format!("c=IN IP4 {conn_ip}"),
        "a=mid:data".into(),
        "a=sctp-port:5000".into(),
        "a=max-message-size:262144".into(),
    ];
    append_candidates(&mut lines, candidates);
    lines.push("a=end-of-candidates".into());
    lines.push(String::new());
    lines.join("\r\n")
}

// ── Video recv remote SDP（mirror field::sfu::build_remote_sdp ServerSendonly）──────────

/// 用 mediasoup transport 参数合成 video recv remote SDP（server sendonly -> 本地 recv）。
pub fn build_recv_video_sdp(
    ice: &IceParameters,
    dtls: &DtlsParameters,
    candidates: Option<&Vec<IceCandidate>>,
    payload_type: u16,
    codec_name: &str,
    clock_rate: u32,
    fmtp: Option<&str>,
) -> String {
    let Some(fp) = dtls.fingerprints.first() else {
        return String::new();
    };
    let conn_ip = candidates
        .and_then(|cs| cs.iter().find(|c| !c.ip.contains(".local")))
        .map(|c| c.ip.clone())
        .unwrap_or_else(|| "0.0.0.0".into());

    let mut lines = vec![
        "v=0".into(),
        "o=- 0 0 IN IP4 0.0.0.0".into(),
        "s=-".into(),
        "t=0 0".into(),
        "a=group:BUNDLE 0".into(),
        "a=ice-lite".into(),
        format!("a=ice-ufrag:{}", ice.username_fragment),
        format!("a=ice-pwd:{}", ice.password),
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".into(),
        format!("m=video 7 UDP/TLS/RTP/SAVPF {payload_type}"),
        format!("c=IN IP4 {conn_ip}"),
        "a=rtcp-mux".into(),
        "a=rtcp-rsize".into(),
        "a=mid:0".into(),
        "a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid".into(),
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01".into(),
        "a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time".into(),
        "a=sendonly".into(),
        format!("a=rtpmap:{payload_type} {codec_name}/{clock_rate}"),
        format!("a=rtcp-fb:{payload_type} nack"),
        format!("a=rtcp-fb:{payload_type} nack pli"),
        format!("a=rtcp-fb:{payload_type} ccm fir"),
    ];
    if let Some(fmtp_val) = fmtp {
        lines.push(format!("a=fmtp:{payload_type} {fmtp_val}"));
    }
    append_candidates(&mut lines, candidates);
    lines.push("a=end-of-candidates".into());
    lines.push(String::new());
    lines.join("\r\n")
}

// ── SSRC 注入（mirror field::sfu::inject_remote_ssrc）──────────

/// 将 consumer rtp_parameters 的 ssrc 注入 remote SDP（libwebrtc demux 需要）。
pub fn inject_remote_ssrc(remote_sdp: &str, consumer_rtp: &Value) -> String {
    let ssrc = consumer_rtp
        .get("encodings")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .and_then(|enc| enc.get("ssrc"))
        .and_then(|s| s.as_u64());
    let Some(ssrc) = ssrc else {
        return remote_sdp.to_string();
    };
    let sep = if remote_sdp.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = remote_sdp.split(sep).collect();
    let mut out = Vec::with_capacity(lines.len() + 2);
    let mut injected = false;
    for line in &lines {
        out.push(line.to_string());
        if !injected && line.starts_with("a=mid:") {
            out.push(format!("a=ssrc:{ssrc} cname:mediaservo-client"));
            out.push(format!("a=ssrc:{ssrc} msid:remote video"));
            injected = true;
        }
    }
    if !injected {
        out.push(format!("a=ssrc:{ssrc} cname:mediaservo-client"));
    }
    out.join(sep)
}

// ── Codec 解析（从 Consumed.rtp_parameters 提取）──────────

/// 从 Consumed 返回的 rtp_parameters 解析首个 video codec（非 rtx）。
/// 返回 (pt, name, clock_rate, fmtp_option)。
pub fn codec_from_consumer(rtp: &Value) -> Option<(u16, String, u32, Option<String>)> {
    let codecs = rtp.get("codecs")?.as_array()?;
    let codec = codecs.iter().find(|c| {
        c.get("mimeType")
            .and_then(|m| m.as_str())
            .map(|m| !m.ends_with("/rtx"))
            .unwrap_or(true)
    })?;
    let pt = codec.get("payloadType")?.as_u64()? as u16;
    let mime = codec.get("mimeType")?.as_str()?.to_string();
    let name = mime.rsplit('/').next()?.to_string();
    let clock = codec.get("clockRate")?.as_u64()? as u32;
    let params = codec.get("parameters")?;
    let fmtp = if params.is_object() && !params.as_object().unwrap().is_empty() {
        let pairs: Vec<String> = params
            .as_object()?
            .iter()
            .map(|(k, v)| {
                if let Some(s) = v.as_str() {
                    format!("{k}={s}")
                } else {
                    format!("{k}={v}")
                }
            })
            .collect();
        Some(pairs.join(";"))
    } else {
        None
    };
    Some((pt, name, clock, fmtp))
}

// ── internal ──

fn append_candidates(lines: &mut Vec<String>, candidates: Option<&Vec<IceCandidate>>) {
    if let Some(cands) = candidates {
        for c in cands {
            if c.ip.contains(".local") {
                continue;
            }
            let ctype = match c.candidate_type.as_str() {
                "host" | "srflx" | "prflx" | "relay" => c.candidate_type.as_str(),
                _ => "host",
            };
            lines.push(format!(
                "a=candidate:{} 1 {} {} {} {} typ {}",
                c.foundation, c.protocol.to_uppercase(), c.priority, c.ip, c.port, ctype
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sctp_stream_params_ordered() {
        let sp = sctp_stream_params("chassis", 42).unwrap();
        assert!(sp.ordered);
        assert_eq!(sp.stream_id, 42);
        assert_eq!(sp.max_retransmits, None);
    }

    #[test]
    fn sctp_stream_params_gimbal_partial() {
        let sp = sctp_stream_params("gimbal", 1).unwrap();
        assert!(!sp.ordered);
        assert_eq!(sp.max_retransmits, Some(5));
    }

    #[test]
    fn codec_from_consumer_vp8() {
        let rtp = serde_json::json!({
            "codecs": [{"mimeType": "video/VP8", "payloadType": 96, "clockRate": 90000, "parameters": {}}],
            "encodings": [{"ssrc": 12345}]
        });
        let (pt, name, clock, fmtp) = codec_from_consumer(&rtp).unwrap();
        assert_eq!(pt, 96);
        assert_eq!(name, "VP8");
        assert_eq!(clock, 90000);
        assert!(fmtp.is_none());
    }

    #[test]
    fn codec_from_consumer_h264_with_params() {
        let rtp = serde_json::json!({
            "codecs": [{"mimeType": "video/H264", "payloadType": 101, "clockRate": 90000,
                "parameters": {"profile-level-id": "42e01f", "packetization-mode": 1}}],
            "encodings": [{"ssrc": 67890}]
        });
        let (pt, name, clock, fmtp) = codec_from_consumer(&rtp).unwrap();
        assert_eq!(pt, 101);
        assert_eq!(name, "H264");
        assert_eq!(clock, 90000);
        let f = fmtp.unwrap();
        assert!(f.contains("profile-level-id=42e01f"));
        assert!(f.contains("packetization-mode=1"));
    }

    #[test]
    fn codec_from_consumer_skips_rtx() {
        let rtp = serde_json::json!({
            "codecs": [
                {"mimeType": "video/rtx", "payloadType": 97, "clockRate": 90000},
                {"mimeType": "video/VP8", "payloadType": 96, "clockRate": 90000, "parameters": {}}
            ],
            "encodings": [{"ssrc": 111}]
        });
        let (pt, name, _, _) = codec_from_consumer(&rtp).unwrap();
        assert_eq!(pt, 96);
        assert_eq!(name, "VP8");
    }

    #[test]
    fn codec_from_consumer_empty() {
        assert!(codec_from_consumer(&serde_json::json!({})).is_none());
    }
}
