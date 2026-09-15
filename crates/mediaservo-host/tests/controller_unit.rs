//! controller 逻辑纯单测（S1）——无网络/无 PC：channel_init 语义、SCTP 参数派生、
//! 合成 SDP 形状（PIT-48 顺序 + mDNS 过滤）、信封→Actuator→ControlAck 往返。

use mediaservo_common::protocol::{
    ControlAck, ControlEnvelope, DtlsParameters, Fingerprint, IceCandidate, IceParameters,
};

use mediaservo_host::control::{Actuator, StubActuator};
use mediaservo_host::controller::{
    ACK_LABEL, CommandPolicy, CollectAckSink, build_dc_remote_sdp, channel_init, estop_verdict,
    handle_command, sctp_stream_params,
};

    #[test]
    fn channel_init_reliability_semantics() {
        // D-H3: chassis/light 可靠有序；gimbal partial-reliable
        let chassis = channel_init("chassis");
        assert!(chassis.ordered);
        assert_eq!(chassis.max_retransmits, None);
        let light = channel_init("light");
        assert!(light.ordered);
        let gimbal = channel_init("gimbal");
        assert!(!gimbal.ordered, "云台 partial-reliable: 无序");
        assert_eq!(gimbal.max_retransmits, Some(5), "云台 5 次重传上限");
        let ack = channel_init(ACK_LABEL);
        assert!(ack.ordered, "ack 可靠有序");
    }

    #[test]
    fn sctp_params_track_channel_init_and_reject_unassigned_id() {
        assert!(
            sctp_stream_params("chassis", -1).is_err(),
            "未分配 id 的 DC 禁止 announce"
        );
        let p = sctp_stream_params("gimbal", 3).expect("id=3 合法");
        assert_eq!(p.stream_id, 3);
        assert!(!p.ordered, "gimbal 参数与 channel_init 同源");
        assert_eq!(p.max_retransmits, Some(5));
        let ack = sctp_stream_params(ACK_LABEL, 0).expect("id=0 合法");
        assert!(ack.ordered);
        assert_eq!(ack.stream_id, 0);
    }

    fn fake_transport_parts() -> (IceParameters, DtlsParameters, Vec<IceCandidate>) {
        (
            IceParameters {
                username_fragment: "ufrag1234".into(),
                password: "pwd123456789012".into(),
            },
            DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".into(),
                    value: "AA:BB".into(),
                }],
                role: "auto".into(),
            },
            vec![
                IceCandidate {
                    ip: "fe80::1.local".into(), // mDNS 应被跳过
                    port: 9,
                    protocol: "udp".into(),
                    foundation: "f0".into(),
                    priority: 1,
                    candidate_type: "host".into(),
                },
                IceCandidate {
                    ip: "10.0.0.5".into(),
                    port: 40000,
                    protocol: "udp".into(),
                    foundation: "f1".into(),
                    priority: 2,
                    candidate_type: "host".into(),
                },
            ],
        )
    }

    #[test]
    fn build_dc_remote_sdp_application_shape() {
        let (ice, dtls, cands) = fake_transport_parts();
        let sdp = build_dc_remote_sdp(&ice, &dtls, Some(&cands));
        assert!(sdp.contains("a=group:BUNDLE data"));
        assert!(sdp.contains("a=ice-lite"));
        assert!(sdp.contains("a=setup:actpass"));
        assert!(sdp.contains("m=application 9 UDP/DTLS/SCTP webrtc-datachannel"));
        assert!(sdp.contains("a=mid:data"));
        assert!(sdp.contains("a=sctp-port:5000"));
        // PIT-48: candidate 行必须位于 m= 之后；mDNS 跳过
        let m_at = sdp.find("m=application").expect("m 行");
        let cand_at = sdp.find("a=candidate:f1").expect("候选在 m 后");
        assert!(cand_at > m_at);
        assert!(!sdp.contains(".local"), "mDNS 候选应被过滤");
        assert!(sdp.contains("a=end-of-candidates"));
    }

    #[tokio::test]
    async fn handle_command_roundtrip_via_collect_sink() {
        let actuator = StubActuator;
        let sink = CollectAckSink::default();
        let env = ControlEnvelope {
            seq: 7,
            cmd: "steer".into(),
            payload: serde_json::json!({ "value": 0.35 }),
            sig: None,
        };
        handle_command(
            "chassis",
            serde_json::to_string(&env).unwrap().as_bytes(),
            &actuator,
            None,
            &sink,
            &mediaservo_host::controller::CommandPolicy::default(),
        )
        .await;
        let items = sink.items.lock().unwrap();
        assert_eq!(items.len(), 1, "恰好一条回执");
        let ack: ControlAck = serde_json::from_str(&items[0]).unwrap();
        assert_eq!(ack.ack, 7, "ack 原样回传 seq 配对");
        assert_eq!(ack.result["ok"], true);
        assert_eq!(ack.result["channel"], "chassis", "label 路由证据");
    }

    #[tokio::test]
    async fn handle_command_reports_actuator_error() {
        struct Failing;
        impl Actuator for Failing {
            fn on_command(
                &self,
                _: &str,
                _: &ControlEnvelope,
            ) -> Result<serde_json::Value, String> {
                Err("gpio down".into())
            }
        }
        let sink = CollectAckSink::default();
        let bytes = br#"{"seq":9,"cmd":"on","payload":{}}"#;
        handle_command("light", bytes, &Failing, None, &sink, &mediaservo_host::controller::CommandPolicy::default()).await;
        let items = sink.items.lock().unwrap();
        let ack: ControlAck = serde_json::from_str(&items[0]).unwrap();
        assert_eq!(ack.ack, 9);
        assert_eq!(ack.result["error"], "gpio down", "失败回执携带原因（非静默）");
    }

    #[tokio::test]
    async fn handle_command_ignores_garbage() {
        let sink = CollectAckSink::default();
        handle_command("chassis", b"not-json", &StubActuator, None, &sink, &mediaservo_host::controller::CommandPolicy::default()).await;
        assert!(sink.items.lock().unwrap().is_empty(), "坏信封不得产生回执");
    }

// ── S4/a4·T3.5 estop 门 ──────────────────────────────────────

#[test]
fn estop_verdict_matrix() {
    let env_with = |sig: Option<&str>| ControlEnvelope {
        seq: 1,
        cmd: "estop".into(),
        payload: serde_json::json!({}),
        sig: sig.map(str::to_string),
    };
    let pol = |key: Option<&str>, allow: bool| CommandPolicy {
        hmac_key: key.map(str::to_string),
        allow_unsigned_estop: allow,
        actuation_log: None,
    };
    assert!(estop_verdict(&pol(None, false), &env_with(None)).1, "未配 key 迁移放行");
    let signed = mediaservo_common::protocol::control_hmac_sign("k", &env_with(None));
    assert!(estop_verdict(&pol(Some("k"), false), &env_with(Some(&signed))).1, "正确签名放行");
    assert!(!estop_verdict(&pol(Some("k"), false), &env_with(Some("bad"))).1, "错签必拒");
    assert!(!estop_verdict(&pol(Some("k"), false), &env_with(None)).1, "有 key 无 sig 必拒");
    assert!(estop_verdict(&pol(Some("k"), true), &env_with(None)).1, "逃生位显式开放行");
    let steer = ControlEnvelope { cmd: "steer".into(), ..env_with(None) };
    assert!(estop_verdict(&pol(Some("k"), false), &steer).1, "非 estop 族不涉签名");
}

#[tokio::test]
async fn estop_rejected_does_not_reach_actuator() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counting(Arc<AtomicUsize>);
    impl Actuator for Counting {
        fn on_command(
            &self,
            _l: &str,
            _e: &ControlEnvelope,
        ) -> Result<serde_json::Value, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({ "ok": true }))
        }
    }
    let n = Arc::new(AtomicUsize::new(0));
    let pol = CommandPolicy {
        hmac_key: Some("k".into()),
        allow_unsigned_estop: false,
        actuation_log: None,
    };
    let env = ControlEnvelope {
        seq: 7,
        cmd: "estop_all".into(),
        payload: serde_json::json!({}),
        sig: None,
    };
    let sink = CollectAckSink::default();
    handle_command(
        "chassis",
        serde_json::to_vec(&env).unwrap().as_slice(),
        &Counting(n.clone()),
        None,
        &sink,
        &pol,
    )
    .await;
    assert_eq!(n.load(Ordering::SeqCst), 0, "验签失败 estop 不得触达执行器");
    let acks = sink.items.lock().unwrap_or_else(|e| e.into_inner());
    assert!(
        acks[0].contains("estop_signature_rejected"),
        "拒执必须错误回执: {acks:?}"
    );
}
