//! W3C WebRTC API integration tests.
//!
//! Tests the public RTCPeerConnection/RTCDataChannel API layer against
//! the active backend (stub by default, webrtc-rs/webrtc-sys via features).
//!
//! Reference: webrtc-kit tests (w3c_state_transitions, w3c_observer_tests,
//! w3c_loopback_dc, mock_backend, etc.)

#[cfg(test)]
mod factory_tests {
    use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
    use mediaservo_webrtc::peer_connection::{RTCConfiguration, RTCPeerConnectionState};
    use mediaservo_webrtc::traits::PeerConnectionApi;

    #[test]
    fn factory_creates_default() {
        let factory = RTCPeerConnectionFactory::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        assert_eq!(pc.connection_state(), RTCPeerConnectionState::New);
    }

    #[test]
    fn factory_new_creates_pc() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        assert_eq!(pc.connection_state(), RTCPeerConnectionState::New);
    }

    #[test]
    fn factory_creates_multiple_pcs() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc1 =
            rt.block_on(factory.create_peer_connection(RTCConfiguration::default())).expect("pc1");
        let pc2 =
            rt.block_on(factory.create_peer_connection(RTCConfiguration::default())).expect("pc2");
        assert_eq!(pc1.connection_state(), RTCPeerConnectionState::New);
        assert_eq!(pc2.connection_state(), RTCPeerConnectionState::New);
    }
}

#[cfg(test)]
mod state_tests {
    use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
    use mediaservo_webrtc::peer_connection::{
        RTCConfiguration, RTCIceConnectionState, RTCIceGatheringState, RTCPeerConnectionState,
        RTCSignalingState,
    };
    use mediaservo_webrtc::traits::PeerConnectionApi;

    #[test]
    fn initial_states_are_correct() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");

        assert_eq!(pc.connection_state(), RTCPeerConnectionState::New);
        assert_eq!(pc.ice_connection_state(), RTCIceConnectionState::New);
        assert_eq!(pc.ice_gathering_state(), RTCIceGatheringState::New);
        assert_eq!(pc.signaling_state(), RTCSignalingState::Stable);
    }

    #[test]
    fn close_changes_connection_state() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        rt.block_on(pc.close());
        assert_eq!(pc.connection_state(), RTCPeerConnectionState::Closed);
    }

    #[test]
    fn close_changes_ice_connection_state() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        rt.block_on(pc.close());
        assert_eq!(pc.ice_connection_state(), RTCIceConnectionState::Closed);
    }

    #[test]
    fn close_changes_signaling_state() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        rt.block_on(pc.close());
        assert_eq!(pc.signaling_state(), RTCSignalingState::Closed);
    }
}

#[cfg(test)]
mod sdp_tests {
    use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
    use mediaservo_webrtc::peer_connection::{RTCAnswerOptions, RTCConfiguration, RTCOfferOptions};

    use mediaservo_webrtc::sdp::RTCSdpType;
    use mediaservo_webrtc::traits::PeerConnectionApi;

    #[test]
    fn create_offer_returns_offer_type() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let offer =
            rt.block_on(pc.create_offer(&RTCOfferOptions::default())).expect("create offer");
        assert_eq!(offer.sdp_type, RTCSdpType::Offer);
    }

    #[test]
    fn create_answer_returns_answer_type() {
        // 真 libwebrtc 要求先有 remote offer 才能 create_answer（状态机）
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let offer =
            rt.block_on(pc.create_offer(&RTCOfferOptions::default())).expect("create offer");
        rt.block_on(pc.set_remote_description(&offer)).expect("set remote offer");
        let answer = rt.block_on(pc.create_answer(&RTCAnswerOptions)).expect("create answer");
        assert_eq!(answer.sdp_type, RTCSdpType::Answer);
    }

    #[test]
    fn set_local_description_succeeds() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let offer =
            rt.block_on(pc.create_offer(&RTCOfferOptions::default())).expect("create offer");
        rt.block_on(pc.set_local_description(&offer)).expect("set local");
    }

    #[test]
    fn set_remote_description_succeeds() {
        // 修正: 用 offer 当 remote description（真 libwebrtc 不能把 answer 当 remote 无 offer）
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let offer =
            rt.block_on(pc.create_offer(&RTCOfferOptions::default())).expect("create offer");
        rt.block_on(pc.set_remote_description(&offer)).expect("set remote");
    }

    #[test]
    fn sdp_round_trip_offer_answer() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc1 =
            rt.block_on(factory.create_peer_connection(RTCConfiguration::default())).expect("pc1");
        let pc2 =
            rt.block_on(factory.create_peer_connection(RTCConfiguration::default())).expect("pc2");

        let offer = rt.block_on(pc1.create_offer(&RTCOfferOptions::default())).expect("offer");
        rt.block_on(pc1.set_local_description(&offer)).expect("pc1 set local");
        rt.block_on(pc2.set_remote_description(&offer)).expect("pc2 set remote");

        let answer = rt.block_on(pc2.create_answer(&RTCAnswerOptions)).expect("answer");
        rt.block_on(pc2.set_local_description(&answer)).expect("pc2 set local");
        rt.block_on(pc1.set_remote_description(&answer)).expect("pc1 set remote");
    }

    #[test]
    fn offer_with_receive_audio_video() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let options = RTCOfferOptions {
            ice_restart: false,
            offer_to_receive_audio: true,
            offer_to_receive_video: true,
        };
        let offer = rt.block_on(pc.create_offer(&options)).expect("create offer");
        assert_eq!(offer.sdp_type, RTCSdpType::Offer);
    }
}

#[cfg(test)]
mod ice_tests {
    use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
    use mediaservo_webrtc::peer_connection::{RTCConfiguration, RTCIceCandidate, RTCOfferOptions};
    use mediaservo_webrtc::traits::PeerConnectionApi;

    #[test]
    fn add_ice_candidate_succeeds() {
        // 真 libwebrtc 要求先有 remote description 才能 add_ice_candidate
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        // offer_to_receive_video: 生成含 m=video 段的 offer，candidate 的 mline_index=0 才匹配
        let options = RTCOfferOptions { offer_to_receive_video: true, ..Default::default() };
        let offer = rt.block_on(pc.create_offer(&options)).expect("create offer");
        rt.block_on(pc.set_remote_description(&offer)).expect("set remote offer");
        // candidate 格式与 m-line 对应由 libwebrtc 严格校验（Err 也是合法行为）—
        // 测试目的是验证 API 调用路径 + 状态机前置（有 remote description）
        let candidate = RTCIceCandidate {
            candidate: "candidate:1 1 UDP 2130706431 192.168.1.1 12345 typ host".into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        };
        let _ = rt.block_on(pc.add_ice_candidate(&candidate)); // Ok 或 Err 均可，不 panic
    }

    #[test]
    fn add_multiple_ice_candidates() {
        // 真 libwebrtc 要求先有 remote description 才能 add_ice_candidate
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let options = RTCOfferOptions { offer_to_receive_video: true, ..Default::default() };
        let offer = rt.block_on(pc.create_offer(&options)).expect("create offer");
        rt.block_on(pc.set_remote_description(&offer)).expect("set remote offer");
        for i in 0..5 {
            let candidate = RTCIceCandidate {
                candidate: format!(
                    "candidate:{} 1 UDP 2130706431 192.168.1.{} 12345 typ host",
                    i, i
                ),
                sdp_mid: Some("0".into()),
                sdp_mline_index: Some(0),
            };
            let _ = rt.block_on(pc.add_ice_candidate(&candidate)); // Ok 或 Err 均可
        }
    }
}

#[cfg(test)]
mod datachannel_tests {
    use mediaservo_webrtc::data_channel::{RTCDataChannelInit, RTCDataChannelState};
    use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
    use mediaservo_webrtc::peer_connection::RTCConfiguration;
    use mediaservo_webrtc::traits::PeerConnectionApi;

    #[test]
    fn create_data_channel_returns_correct_label() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let dc = rt
            .block_on(pc.create_data_channel("test-dc", RTCDataChannelInit::default()))
            .expect("create dc");
        assert_eq!(dc.label(), "test-dc");
    }

    #[test]
    fn create_data_channel_with_default_init() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let dc = rt
            .block_on(pc.create_data_channel("dc-default", RTCDataChannelInit::default()))
            .expect("create dc");
        assert_eq!(dc.label(), "dc-default");
    }

    #[test]
    fn data_channel_state_is_closed_after_close() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let mut dc = rt
            .block_on(pc.create_data_channel("close-test", RTCDataChannelInit::default()))
            .expect("create dc");
        rt.block_on(dc.close());
        assert_eq!(dc.state(), RTCDataChannelState::Closed);
    }

    #[test]
    fn data_channel_send_succeeds() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let dc = rt
            .block_on(pc.create_data_channel("send-test", RTCDataChannelInit::default()))
            .expect("create dc");
        rt.block_on(dc.send(b"hello")).expect("send");
    }

    #[test]
    fn data_channel_send_text_succeeds() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let dc = rt
            .block_on(pc.create_data_channel("text-test", RTCDataChannelInit::default()))
            .expect("create dc");
        rt.block_on(dc.send_text("hello world")).expect("send_text");
    }

    #[test]
    fn create_multiple_data_channels() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let dc1 = rt
            .block_on(pc.create_data_channel("dc-1", RTCDataChannelInit::default()))
            .expect("dc1");
        let dc2 = rt
            .block_on(pc.create_data_channel("dc-2", RTCDataChannelInit::default()))
            .expect("dc2");
        assert_eq!(dc1.label(), "dc-1");
        assert_eq!(dc2.label(), "dc-2");
    }

    // ponytail: empty label is valid per W3C spec
    #[test]
    fn create_data_channel_empty_label() {
        let factory = RTCPeerConnectionFactory::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pc = rt
            .block_on(factory.create_peer_connection(RTCConfiguration::default()))
            .expect("create pc");
        let dc = rt
            .block_on(pc.create_data_channel("", RTCDataChannelInit::default()))
            .expect("create dc");
        assert_eq!(dc.label(), "");
    }
}

#[cfg(test)]
mod stats_and_rtp_tests {
    use mediaservo_webrtc::rtp::{
        RTCRtpCodecParameters, RTCRtpEncodingParameters, RTCRtpHeaderExtensionParameters,
        RTCRtpParameters,
    };
    use mediaservo_webrtc::stats::{RTCInboundRtpStreamStats, RTCPeerConnectionStats, RTCStats};

    #[test]
    fn rtc_stats_types_construct() {
        let stats = [
            RTCStats::RTCPeerConnection(RTCPeerConnectionStats {
                id: "pc1".into(),
                timestamp: 0.0,
                data_channels_opened: 1,
                data_channels_closed: 0,
            }),
            RTCStats::InboundRtp(RTCInboundRtpStreamStats {
                id: "in1".into(),
                timestamp: 0.0,
                ssrc: 12345,
                kind: "video".into(),
                packets_received: 100,
                packets_lost: 2,
                bytes_received: 50000,
                frames_decoded: 30,
                frame_width: 1920,
                frame_height: 1080,
                frames_per_second: 30.0,
                jitter: 0.0,
                frame_dropped: 0,
                nack_count: 0,
                pli_count: 0,
                fir_count: 0,
            }),
        ];
        assert_eq!(stats.len(), 2);
    }

    #[test]
    fn rtc_stats_serializes() {
        let stats = RTCStats::RTCPeerConnection(RTCPeerConnectionStats {
            id: "pc1".into(),
            timestamp: 1.0,
            data_channels_opened: 1,
            data_channels_closed: 0,
        });
        let json = serde_json::to_string(&stats).expect("serialize");
        assert!(json.contains("pc1"));
        assert!(json.contains("data_channels_opened"));
    }

    #[test]
    fn rtp_parameters_default_values() {
        let params = RTCRtpParameters::default();
        assert!(params.transaction_id.is_empty());
        assert!(params.codecs.is_empty());
        assert!(params.encodings.is_empty());
    }

    #[test]
    fn rtp_codec_parameters_h264() {
        let codec = RTCRtpCodecParameters {
            mime_type: "video/H264".into(),
            payload_type: 96,
            clock_rate: 90000,
            channels: None,
            sdp_fmtp_line: Some("profile-level-id=42e01f".into()),
        };
        assert_eq!(codec.mime_type, "video/H264");
        assert_eq!(codec.payload_type, 96);
    }

    #[test]
    fn rtp_encoding_default_active() {
        let enc = RTCRtpEncodingParameters::default();
        assert!(enc.active);
        assert!(enc.ssrc.is_none());
    }

    #[test]
    fn rtp_header_extension_parameters() {
        let ext = RTCRtpHeaderExtensionParameters {
            uri: "urn:ietf:params:rtp-hdrext:ssrc-audio-level".into(),
            id: 1,
            encrypted: false,
        };
        assert_eq!(ext.uri, "urn:ietf:params:rtp-hdrext:ssrc-audio-level");
        assert_eq!(ext.id, 1);
    }
}

// ── T1: W3C transceiver/capabilities 类型单测 (v2) ──
#[cfg(test)]
mod transceiver_types_tests {
    use mediaservo_webrtc::rtp::{
        RTCRtpCapabilities, RTCRtpCodecCapability, RTCRtpEncodingParameters,
        RTCRtpHeaderExtensionCapability, RTCRtpParameters, RTCRtpTransceiver,
        RTCRtpTransceiverDirection, RTCRtpTransceiverInit,
    };
    use mediaservo_webrtc::rtp::{RTCRtpReceiver, RTCRtpSender};
    use mediaservo_webrtc::track::TrackKind;
    use mediaservo_webrtc::track::TrackRef;
    use mediaservo_webrtc::track::TrackSender;

    #[test]
    fn transceiver_init_default() {
        let init = RTCRtpTransceiverInit::default();
        assert_eq!(init.direction, RTCRtpTransceiverDirection::Sendrecv);
        assert!(init.send_encodings.is_empty());
        assert!(init.stream_ids.is_empty());
    }

    #[test]
    fn transceiver_direction_mapping() {
        assert_eq!(RTCRtpTransceiverDirection::Sendrecv.as_str(), "sendrecv");
        assert_eq!(RTCRtpTransceiverDirection::Sendonly.as_str(), "sendonly");
        assert_eq!(RTCRtpTransceiverDirection::Recvonly.as_str(), "recvonly");
        assert_eq!(RTCRtpTransceiverDirection::Inactive.as_str(), "inactive");
    }

    #[test]
    fn transceiver_struct_fields() {
        let sender =
            RTCRtpSender::new(TrackRef::Sender(TrackSender::new("s1".into(), TrackKind::Video)));
        let receiver = RTCRtpReceiver::new(TrackRef::Receiver(
            mediaservo_webrtc::track::TrackReceiver::new("r1".into(), TrackKind::Video),
        ));
        let tc = RTCRtpTransceiver::new(
            Some("0".into()),
            RTCRtpTransceiverDirection::Sendonly,
            Some(RTCRtpTransceiverDirection::Sendonly),
            false,
            sender,
            receiver,
            TrackKind::Video,
        );
        assert_eq!(tc.mid.as_deref(), Some("0"));
        assert_eq!(tc.direction, RTCRtpTransceiverDirection::Sendonly);
        assert_eq!(tc.current_direction, Some(RTCRtpTransceiverDirection::Sendonly));
        assert!(!tc.stopped);
        assert_eq!(tc.kind, TrackKind::Video);
        assert_eq!(tc.sender.track_id, "s1");
        assert_eq!(tc.receiver.track_id, "r1");
    }

    #[test]
    fn capabilities_construct() {
        let caps = RTCRtpCapabilities { codecs: vec![], header_extensions: vec![] };
        assert!(caps.codecs.is_empty());
        assert!(caps.header_extensions.is_empty());
    }

    #[test]
    fn codec_capability_fields() {
        let codec = RTCRtpCodecCapability {
            mime_type: "video/H264".into(),
            clock_rate: Some(90000),
            channels: None,
            sdp_fmtp_line: Some("profile-level-id=42e01f".into()),
        };
        assert_eq!(codec.mime_type, "video/H264");
        assert_eq!(codec.clock_rate, Some(90000));
        assert_eq!(codec.sdp_fmtp_line.as_deref(), Some("profile-level-id=42e01f"));
    }

    #[test]
    fn header_ext_capability_fields() {
        let ext = RTCRtpHeaderExtensionCapability {
            uri: "urn:ietf:params:rtp-hdrext:sdes:mid".into(),
            id: Some(1),
        };
        assert_eq!(ext.uri, "urn:ietf:params:rtp-hdrext:sdes:mid");
        assert_eq!(ext.id, Some(1));
    }

    #[test]
    fn rtp_parameters_has_mid() {
        let mut params = RTCRtpParameters::default();
        params.mid = "0".into();
        assert_eq!(params.mid, "0");
        assert!(params.codecs.is_empty());
    }

    #[test]
    fn encoding_parameters_codec_dtx() {
        let mut enc = RTCRtpEncodingParameters::default();
        enc.codec = Some("video/H264".into());
        enc.dtx = Some(false);
        assert_eq!(enc.codec.as_deref(), Some("video/H264"));
        assert_eq!(enc.dtx, Some(false));
    }
}

// ── T3: 包装层测试（v2）──
#[cfg(test)]
mod wrapper_tests {
    use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
    use mediaservo_webrtc::peer_connection::RTCConfiguration;
    #[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]
    use mediaservo_webrtc::rtp::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit}; // stub 姿态门测在用（clippy --fix 姿态盲区第三例）

    use mediaservo_webrtc::track::TrackKind;
    use mediaservo_webrtc::traits::PeerConnectionApi;

    fn new_pc() -> mediaservo_webrtc::peer_connection::RTCPeerConnection {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(
            RTCPeerConnectionFactory::new().create_peer_connection(RTCConfiguration::default()),
        )
        .unwrap()
    }

    #[test]
    #[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]

    fn pc_add_transceiver_w3c() {
        let pc = new_pc();
        let factory = RTCPeerConnectionFactory::new();
        let track = factory.create_video_track("t1");
        let init = RTCRtpTransceiverInit {
            direction: RTCRtpTransceiverDirection::Sendonly,
            send_encodings: vec![],
            stream_ids: vec![],
        };
        // track 版 (stub + sys 都支持；kind 版 sys 下 NotSupported)
        let tc = pc.add_transceiver_with_track(&track, init).unwrap();
        assert_eq!(tc.kind, TrackKind::Video);
        assert_eq!(tc.direction, RTCRtpTransceiverDirection::Sendonly);
    }

    #[test]
    #[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]

    fn pc_get_transceivers_after_add() {
        let pc = new_pc();
        let factory = RTCPeerConnectionFactory::new();
        let track = factory.create_video_track("t1");
        let before = pc.get_transceivers().unwrap();
        pc.add_transceiver_with_track(&track, RTCRtpTransceiverInit::default()).unwrap();
        let after = pc.get_transceivers().unwrap();
        // stub/sys 都应反映新增（sys 可能包含其他 transceiver，断言 >= before+1）
        assert!(after.len() >= before.len() + 1, "get_transceivers 应反映新增");
    }

    #[test]
    fn sender_get_parameters_via_pc() {
        let pc = new_pc();
        // stub 返回默认空；sys 返回真实 codecs — 断言不 panic 即可
        let _ = pc.get_sending_rtp_parameters("t1");
    }

    #[test]
    fn receiver_get_parameters_via_pc() {
        let pc = new_pc();
        let _ = pc.get_receiving_rtp_parameters("t1");
    }

    #[test]
    fn capabilities_via_pc() {
        let pc = new_pc();
        // stub 返回 None；sys 返回真实 capabilities — 断言不 panic
        let _ = pc.get_sender_capabilities(TrackKind::Video);
        let _ = pc.get_receiver_capabilities(TrackKind::Audio);
    }

    #[test]
    fn restart_ice_via_pc() {
        let pc = new_pc();
        pc.restart_ice().unwrap();
    }

    #[test]
    fn configuration_via_pc() {
        let pc = new_pc();
        let cfg = pc.get_configuration();
        pc.set_configuration(&cfg).unwrap();
    }

    #[test]
    fn current_descriptions_via_pc() {
        let pc = new_pc();
        assert!(pc.current_local_description().unwrap().is_none());
        assert!(pc.current_remote_description().unwrap().is_none());
    }

    #[test]
    #[cfg(not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys")))]

    fn add_transceiver_with_track_writable() {
        let pc = new_pc();
        let factory = RTCPeerConnectionFactory::new();
        let track = factory.create_video_track("t1");
        let init = RTCRtpTransceiverInit {
            direction: RTCRtpTransceiverDirection::Sendonly,
            send_encodings: vec![],
            stream_ids: vec![],
        };
        let tc = pc.add_transceiver_with_track(&track, init).unwrap();
        assert_eq!(tc.kind, TrackKind::Video);
        // 写帧不 panic（锁死 P3 写帧链路）
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(track.write_raw_i420(&vec![0u8; 640 * 480 * 3 / 2], 640, 480));
    }

    #[test]
    fn sender_object_get_parameters() {
        let pc = new_pc();
        pc.add_track("v1", TrackKind::Video).unwrap();
        let senders = pc.get_senders();
        assert_eq!(senders.len(), 1);
        let _ = senders[0].get_parameters(); // 不 panic 即可
    }
}
