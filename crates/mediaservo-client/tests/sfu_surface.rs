//! client v2 SFU-surface signaling shape tests -- a mock server over WS (the direct shape: a PSK text frame + bare
//! SignalingMessage JSON, not gateway LocalEnvelopes). Reuses host controller_e2e's
//! TCP mock mode; no external server / no mediasoup.
//!
//! Asserts (item 9 of the S2 brief):
//! (1) consume_video -> uplink CreateWebRtcTransport(Recv) -> Consume{transport_id bound}
//!    -> ConnectWebRtcTransport{role=client, a real fingerprint}
//! ② open_control → CreateWebRtcTransport(Send) → Connect → CreateDataProducer×label
//!    (protocol=sctp / sctp_stream_parameters same-source as channel_init / transport_id explicit)
//! (3) CreateDataProducer meets Error{4012} -> ClientError::ControlDenied (typed terminal)
//! (4) RoomJoined without protocol (an old server) -> negotiated=1 -> open_control pre-rejects locally
//!    ProtocolTooLow, sending no transport request at all
//!
//! Precondition: backend-webrtc-sys real libwebrtc local negotiation -> Linux-gated (same as controller_e2e).

#![cfg(target_os = "linux")]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_client::error::ClientError;
use mediaservo_client::{ClientConfig, RoomSession};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceParameters, MediaKind, PeerRole, SctpStreamParameters,
    SignalingMessage, TransportDirection,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const ROOM: &str = "veh-test";
const PSK: &str = "mock-psk";
/// libwebrtc rejects a malformed fingerprint -- 32 bytes of colon-separated hex (it will never complete DTLS with the mock).
const FAKE_FP: &str = "01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";

type MockWs = WebSocketStream<tokio::net::TcpStream>;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// SFU all-positive replies (TransportCreated / Consumed / DataProducerCreated).
    Respond,
    /// CreateDataProducer always rejected with 4012.
    DenyControl,
}

fn text(msg: &SignalingMessage) -> Message {
    serde_json::to_string(msg).expect("serialize").into()
}

async fn send_msg(ws: &mut MockWs, msg: &SignalingMessage) {
    ws.send(text(msg)).await.expect("mock: write failed");
}

/// Receive one uplink message (an 8s timeout panic for pinpointing).
async fn read_msg(ws: &mut MockWs) -> SignalingMessage {
    let m = tokio::time::timeout(Duration::from_secs(8), ws.next())
        .await
        .expect("mock: timed out waiting for an uplink message")
        .expect("mock: the ws stream ended")
        .expect("mock: a ws read error");
    serde_json::from_str(m.to_text().expect("mock: a non-text frame")).expect("mock: message parse")
}

fn transport_created(id: &str) -> SignalingMessage {
    SignalingMessage::WebRtcTransportCreated {
        room_id: ROOM.into(),
        peer_id: "consumer".into(),
        transport_id: id.into(),
        ice_parameters: IceParameters {
            username_fragment: "ufrag1234".into(),
            password: "pwdpwdpwdpwdpwdpwdpwdpwdpwdpwdpwd".into(), // the libwebrtc floor is 22
        },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint { algorithm: "sha-256".into(), value: FAKE_FP.into() }],
            role: "auto".into(),
        },
        // None = no remote candidates -> ICE sends no checks, must not go Failed within the test window (the controller_e2e method)
        ice_candidates: None,
        sctp_parameters: None,
    }
}

/// Start the mock server: after handshake (PSK -> auth_ok -> RoomJoin -> RoomJoined{protocol}) it enters the
/// scripted reply loop; uplink messages are relayed through an observation channel back to the test asserts. The ws is held by a background task until the test ends.
async fn pair(
    protocol: Option<u32>,
    mode: Mode,
) -> (RoomSession, mpsc::UnboundedReceiver<SignalingMessage>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = ClientConfig {
        signaling_url: format!("ws://{addr}/ws"),
        room_id: ROOM.into(),
        psk: Some(PSK.into()),
        jwt: None,
        role: PeerRole::Consumer,
        hmac_key: None,
        identity_dir: None,
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut ws: MockWs = tokio_tungstenite::accept_async(sock).await.unwrap();

        // -- handshake (interleaved with RoomSession::connect: the mock must answer first or the two wait on each other)
        let frame = ws.next().await.unwrap().unwrap();
        assert_eq!(
            frame.to_text().unwrap(),
            PSK,
            "the direct shape's first frame = the PSK plaintext"
        );
        send_msg(&mut ws, &SignalingMessage::Error { code: 0, message: "auth_ok".into() }).await;
        let join = read_msg(&mut ws).await;
        match join {
            SignalingMessage::RoomJoin { room_id, peer_role, protocol: claim, .. } => {
                assert_eq!(room_id, ROOM);
                assert_eq!(peer_role, PeerRole::Consumer);
                assert_eq!(
                    claim,
                    Some(mediaservo_common::protocol::SIGNALING_PROTOCOL_VERSION),
                    "S0: the join must carry the dialect declaration"
                );
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        send_msg(
            &mut ws,
            &SignalingMessage::RoomJoined {
                room_id: ROOM.into(),
                peer_id: "consumer-test".into(),
                protocol,
                server_version: None,
                session_nonce: None,
            },
        )
        .await;

        // -- the scripted reply loop (peer-drop convergence)
        while let Some(m) = ws.next().await {
            let Ok(m) = m else { break };
            let Some(t) = m.to_text().ok() else { continue };
            let Ok(msg) = serde_json::from_str::<SignalingMessage>(t) else { continue };
            let _ = tx.send(msg.clone());
            match msg {
                SignalingMessage::CreateWebRtcTransport { direction, .. } => {
                    let id = match direction {
                        TransportDirection::Recv => "t-recv",
                        TransportDirection::Send => "t-send",
                    };
                    send_msg(&mut ws, &transport_created(id)).await;
                }
                SignalingMessage::GetRouterRtpCapabilities { .. } => {
                    send_msg(
                        &mut ws,
                        &SignalingMessage::RouterRtpCapabilities {
                            room_id: ROOM.into(),
                            capabilities: serde_json::json!({
                                "codecs": [{"mimeType": "video/VP8", "clockRate": 90000,
                                           "kind": "video", "payloadTypes": [96]}],
                                "headerExtensions": [],
                            }),
                        },
                    )
                    .await;
                }
                SignalingMessage::Consume { .. } => {
                    send_msg(
                        &mut ws,
                        &SignalingMessage::Consumed {
                            room_id: ROOM.into(),
                            consumer_id: "cons-1".into(),
                            producer_id: "prod-1".into(),
                            kind: MediaKind::Video,
                            rtp_parameters: serde_json::json!({
                                "codecs": [{"mimeType": "video/VP8", "payloadType": 96,
                                           "clockRate": 90000, "parameters": {}}],
                                "encodings": [{"ssrc": 11112222}]
                            }),
                        },
                    )
                    .await;
                }
                SignalingMessage::ConnectWebRtcTransport { .. } => {
                    send_msg(
                        &mut ws,
                        &SignalingMessage::Error { code: 0, message: "transport_connected".into() },
                    )
                    .await;
                }
                SignalingMessage::CreateDataProducer { label, .. } => {
                    let reply = if mode == Mode::DenyControl {
                        SignalingMessage::Error {
                            code: 4012,
                            message: format!("control_denied: {label}"),
                        }
                    } else {
                        SignalingMessage::DataProducerCreated {
                            room_id: ROOM.into(),
                            data_producer_id: format!("dp-{label}"),
                        }
                    };
                    send_msg(&mut ws, &reply).await;
                }
                other => panic!("mock: an unexpected uplink message {other:?}"),
            }
        }
    });
    let session = match RoomSession::connect(&cfg).await {
        Ok(s) => s,
        Err(e) => {
            // a mock-side panic surfaces via the JoinHandle (never swallowing the second scene)
            let reason = server.await.expect_err("the mock task should panic");
            panic!("connect failed: {e:?} (mock panic: {reason})");
        }
    };
    (session, rx)
}

/// Take n observed uplinks (a 5s/each timeout as backstop).
async fn take(
    rx: &mut mpsc::UnboundedReceiver<SignalingMessage>,
    n: usize,
) -> Vec<SignalingMessage> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(
            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("timed out waiting for an uplink message")
                .expect("the observation channel closed"),
        );
    }
    out
}

// -------- tests --------

#[tokio::test]
async fn consume_video_drives_recv_sfu_sequence() {
    let (mut session, mut obs) = pair(Some(3), Mode::Respond).await;
    assert_eq!(session.negotiated(), 3);

    session.consume_video("prod-1").await.expect("consume_video should establish");

    let msgs = take(&mut obs, 4).await;
    match &msgs[0] {
        SignalingMessage::CreateWebRtcTransport { room_id, peer_id, direction } => {
            assert_eq!(room_id, ROOM);
            assert_eq!(peer_id, "consumer", "the SFU peer key = role-derived (the C1 convention)");
            assert_eq!(direction, &TransportDirection::Recv);
        }
        other => panic!("(1) expected CreateWebRtcTransport(Recv), got {other:?}"),
    }
    match &msgs[1] {
        // S2b: must query the router caps before consume (hand-built caps are rejected by real mediasoup -- never regress)
        SignalingMessage::GetRouterRtpCapabilities { room_id } => assert_eq!(room_id, ROOM),
        other => panic!("(1) expected GetRouterRtpCapabilities, got {other:?}"),
    }
    match &msgs[2] {
        SignalingMessage::Consume { producer_id, transport_id, rtp_capabilities, .. } => {
            assert_eq!(producer_id, "prod-1");
            assert_eq!(
                transport_id.as_deref(),
                Some("t-recv"),
                "C1 explicitly binds the recv transport"
            );
            assert!(
                rtp_capabilities["codecs"].to_string().contains("VP8"),
                "Consume must carry the router's reply caps, got {rtp_capabilities}"
            );
        }
        other => panic!("(1) expected Consume, got {other:?}"),
    }
    match &msgs[3] {
        SignalingMessage::ConnectWebRtcTransport { transport_id, dtls_parameters, .. } => {
            assert_eq!(transport_id, "t-recv");
            assert_eq!(dtls_parameters.role, "client", "the endpoint = DTLS client");
            assert_eq!(dtls_parameters.fingerprints.len(), 1);
            assert!(
                dtls_parameters.fingerprints[0].value.len() > 32,
                "the fingerprint should be the local PC's real value"
            );
        }
        other => panic!("(1) expected ConnectWebRtcTransport, got {other:?}"),
    }
}

#[tokio::test]
async fn open_control_announces_data_producers() {
    let (mut session, mut obs) = pair(Some(3), Mode::Respond).await;
    let ctl =
        session.open_control(&["chassis", "gimbal"]).await.expect("open_control should establish");
    assert_eq!(ctl.labels(), ["chassis", "gimbal"]);
    assert_eq!(ctl.producer_ids(), ["dp-chassis", "dp-gimbal"]);

    let msgs = take(&mut obs, 4).await;
    match &msgs[0] {
        SignalingMessage::CreateWebRtcTransport { direction, .. } => {
            assert_eq!(direction, &TransportDirection::Send);
        }
        other => panic!("(2) expected CreateWebRtcTransport(Send), got {other:?}"),
    }
    assert!(
        matches!(msgs[1], SignalingMessage::ConnectWebRtcTransport { .. }),
        "(2) Connect must precede CreateDataProducer (the S1 ordering contract)"
    );
    let mut stream_ids = Vec::new();
    for (i, (want_label, want_ordered, want_retrans)) in
        [("chassis", true, None), ("gimbal", false, Some(5u16))].iter().enumerate()
    {
        match &msgs[2 + i] {
            SignalingMessage::CreateDataProducer {
                label,
                protocol,
                sctp_stream_parameters,
                transport_id,
                transport_direction,
                ..
            } => {
                assert_eq!(label, *want_label);
                assert_eq!(protocol, "sctp");
                assert_eq!(transport_direction, &TransportDirection::Send);
                assert_eq!(
                    transport_id.as_deref(),
                    Some("t-send"),
                    "C1 explicitly binds the send transport"
                );
                let sp: &SctpStreamParameters = sctp_stream_parameters
                    .as_ref()
                    .unwrap_or_else(|| panic!("(2) {want_label} missing sctp_stream_parameters"));
                assert_eq!(sp.ordered, *want_ordered, "{want_label} ordered (D-H3 same source)");
                assert_eq!(sp.max_retransmits, *want_retrans, "{want_label} retransmits");
                stream_ids.push(sp.stream_id);
            }
            other => panic!("(2) expected CreateDataProducer, got {other:?}"),
        }
    }
    assert_ne!(
        stream_ids[0], stream_ids[1],
        "stream_id = the libwebrtc real-config DC id, must differ"
    );
}

#[tokio::test]
async fn control_denied_4012_maps_typed_terminal() {
    let (mut session, _obs) = pair(Some(3), Mode::DenyControl).await;
    let e = session.open_control(&["chassis"]).await.unwrap_err();
    assert!(matches!(&e, ClientError::ControlDenied(m) if m.contains("chassis")), "got {e:?}");
}

#[tokio::test]
async fn old_server_negotiated_1_pre_refuses_control() {
    let (mut session, mut obs) = pair(None, Mode::Respond).await;
    assert_eq!(session.negotiated(), 1, "RoomJoined without protocol = an old server's dialect 1");
    let e = session.open_control(&["chassis"]).await.unwrap_err();
    assert!(
        matches!(e, ClientError::ProtocolTooLow { need: 2, got: 1 }),
        "the I5 gate pre-rejects locally, got {e:?}"
    );
    // no transport request sent (the observation channel stays empty)
    assert!(
        tokio::time::timeout(Duration::from_millis(300), obs.recv()).await.is_err(),
        "(4) the pre-rejection must produce no uplink"
    );
}
