//! S1: host-controller SFU-DC signaling e2e - a mock local gateway WS (fixture reuses
//! gateway_e2e.rs's accept_async + LocalEnvelope shape, no external server / no mediasoup).
//!
//! Topology: the controller takes the production path (SignalClient::new_gateway -> control_loop) with its
//! "gateway" played by an in-test mock WS: asserts upstream request shapes, answers with mediasoup semantics.
//!
//! Assertions (the signaling layer = the highest reachable layer without a worker environment; live DTLS/SCTP is a separate round):
//! (1) RoomJoin/RoomJoined envelope handshake (D2 gateway shape)
//! (2) CreateWebRtcTransport(Send) -> ConnectWebRtcTransport (DTLS fingerprint role=client)
//!    -> CreateDataProducer x{chassis,gimbal,light,ack}: protocol=sctp,
//!    explicit transport_id binding, gimbal ordered=false/max_retransmits=5 (D-H3 same source as
//!    channel_init), unique stream_id (= the DC id libwebrtc actually assigned)
//! (3) CreateWebRtcTransport(Recv) -> Connect -> inject NewDataProducer ->
//!    ConsumeData{dp, transport_id=recv}; the broadcast echo of one's own producer must not trigger consumption
//! (4) the main loop stays alive after DataConsumed (no crash-loop on unknown messages); mock closes the link -> self-heal exit code 1
//!
//! Prerequisite: mediaservo-webrtc backend-webrtc-sys (real libwebrtc local negotiation, no network IO).

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, PeerRole, SctpStreamParameters, SignalingMessage,
    TransportDirection,
};
use mediaservo_host::control::StubActuator;
use mediaservo_host::controller::{ControllerConfig, control_loop};
use mediaservo_link::{LocalEnvelope, SignalClient};
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const ROOM: &str = "control";
/// libwebrtc rejects malformed fingerprints at setRemoteDescription outright - give it a full 32 bytes of
/// colon-hex (the value is meaningless; DTLS will never complete a handshake with the mock).
const FAKE_FP: &str = "01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";

fn env_text(msg: SignalingMessage) -> Message {
    let text = serde_json::to_string(&LocalEnvelope { src: "server".into(), msg })
        .expect("envelope serializes");
    // (useless_conversion pin: tungstenite From<String> for Message)
    text.into()
}

fn transport_created(transport_id: &str, ufrag: &str) -> SignalingMessage {
    SignalingMessage::WebRtcTransportCreated {
        room_id: ROOM.into(),
        peer_id: "veh-peer".into(),
        transport_id: transport_id.into(),
        sctp_parameters: None,
        ice_parameters: mediaservo_common::protocol::IceParameters {
            username_fragment: ufrag.into(),
            password: "pwdpwdpwdpwdpwdpwdpwdpwdpwdpwdpwd".into(), // libwebrtc lower bound is 22 chars
        },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint { algorithm: "sha-256".into(), value: FAKE_FP.into() }],
            role: "auto".into(),
        },
        // candidates=None: the synthetic SDP has no remote candidates -> ICE starts no checks -> never
        // Failed within the test window (the live path with a real server's inline candidates is live acceptance, see file header (4)).
        ice_candidates: None,
    }
}

/// Receive one upstream envelope message (10s timeout; a timeout panics with location).
async fn read_msg(ws: &mut WebSocketStream<tokio::net::TcpStream>) -> SignalingMessage {
    let m = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("mock: timed out waiting for an upstream message")
        .expect("mock: ws stream ended")
        .expect("mock: ws read error");
    let env: LocalEnvelope = serde_json::from_str(m.to_text().expect("mock: non-text frame"))
        .expect("mock: envelope parses");
    env.msg
}

fn expect_create_transport(msg: SignalingMessage, direction: TransportDirection) {
    match msg {
        SignalingMessage::CreateWebRtcTransport { room_id, peer_id, direction: d } => {
            assert_eq!(room_id, ROOM, "the CreateWebRtcTransport room");
            assert_eq!(peer_id, "host", "SFU peer key = host (C1 convention)");
            assert_eq!(d, direction, "transport direction");
        }
        other => panic!("expected CreateWebRtcTransport({direction:?}), got {other:?}"),
    }
}

fn expect_connect(msg: SignalingMessage, transport_id: &str) {
    match msg {
        SignalingMessage::ConnectWebRtcTransport {
            transport_id: tid,
            dtls_parameters,
            peer_id,
            ..
        } => {
            assert_eq!(tid, transport_id, "the transport_id bound by Connect");
            assert_eq!(peer_id, "host");
            assert_eq!(
                dtls_parameters.role, "client",
                "endpoint = DTLS client (mediasoup server side)"
            );
            assert_eq!(dtls_parameters.fingerprints.len(), 1);
            assert!(
                dtls_parameters.fingerprints[0].value.len() > 32,
                "the fingerprint should be the local PC's real value (sha-256 colon-hex)"
            );
        }
        other => panic!("expected ConnectWebRtcTransport, got {other:?}"),
    }
}

fn expect_data_producer(
    msg: SignalingMessage,
    label: &str,
    ordered: bool,
    max_retransmits: Option<u16>,
    seen_stream_ids: &mut Vec<u16>,
) -> SctpStreamParameters {
    let dp = match msg {
        SignalingMessage::CreateDataProducer {
            room_id,
            transport_direction,
            label: l,
            protocol,
            sctp_stream_parameters,
            transport_id,
            ..
        } => {
            assert_eq!(room_id, ROOM);
            assert_eq!(
                transport_direction,
                TransportDirection::Send,
                "{label} must use the send transport"
            );
            assert_eq!(l, label, "announce label");
            assert_eq!(protocol, "sctp", "the v1 sub-protocol name");
            assert_eq!(
                transport_id.as_deref(),
                Some("t-send"),
                "C1 explicit binding to the send transport"
            );
            sctp_stream_parameters
                .expect("sctp_stream_parameters is required (the server must reject without it)")
        }
        other => panic!("expected CreateDataProducer({label}), got {other:?}"),
    };
    assert_eq!(dp.ordered, ordered, "{label} ordered is the same source as channel_init");
    assert_eq!(
        dp.max_retransmits, max_retransmits,
        "{label} retransmit params are the same source as channel_init"
    );
    assert!(
        !seen_stream_ids.contains(&dp.stream_id),
        "stream_id unique (no duplicate DC ids): {seen_stream_ids:?} + {}",
        dp.stream_id
    );
    seen_stream_ids.push(dp.stream_id);
    dp
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_sfu_dc_signaling_sequence() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,mediaservo_host=debug".into()),
        )
        .try_init();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // -- mock gateway: answer per the S1 contract and assert each upstream shape --
    let mock = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("mock accept");
        let mut ws = tokio_tungstenite::accept_async(stream).await.expect("mock ws");

        // (1) RoomJoin -> RoomJoined (synthesized locally by the gateway, protocol=3 S0.5 dialect)
        match read_msg(&mut ws).await {
            SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
                assert_eq!(room_id, ROOM);
                assert_eq!(peer_role, PeerRole::Host);
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        ws.send(env_text(SignalingMessage::RoomJoined {
            room_id: ROOM.into(),
            peer_id: "veh-peer".into(),
            protocol: Some(3),
            server_version: None,
            session_nonce: None,
        }))
        .await
        .unwrap();

        // (2) outbound: Send transport -> connect -> 4x CreateDataProducer announce
        expect_create_transport(read_msg(&mut ws).await, TransportDirection::Send);
        ws.send(env_text(transport_created("t-send", "ufragSend1"))).await.unwrap();
        expect_connect(read_msg(&mut ws).await, "t-send");
        ws.send(env_text(SignalingMessage::Error {
            code: 0,
            message: "transport_connected".into(),
        }))
        .await
        .unwrap();

        let mut stream_ids: Vec<u16> = Vec::new();
        let cases = [
            ("chassis", true, None),
            ("gimbal", false, Some(5u16)),
            ("light", true, None),
            ("ack", true, None),
        ];
        let mut own_ids: Vec<String> = Vec::new();
        for (i, (label, ordered, retrans)) in cases.iter().enumerate() {
            expect_data_producer(
                read_msg(&mut ws).await,
                label,
                *ordered,
                *retrans,
                &mut stream_ids,
            );
            let id = format!("dp-{i}-{label}");
            own_ids.push(id.clone());
            ws.send(env_text(SignalingMessage::DataProducerCreated {
                room_id: ROOM.into(),
                data_producer_id: id,
            }))
            .await
            .unwrap();
        }

        // (3) inbound: Recv transport -> connect -> inject the cockpit producer + the own echo
        expect_create_transport(read_msg(&mut ws).await, TransportDirection::Recv);
        ws.send(env_text(transport_created("t-recv", "ufragRecv1"))).await.unwrap();
        expect_connect(read_msg(&mut ws).await, "t-recv");
        ws.send(env_text(SignalingMessage::Error {
            code: 0,
            message: "transport_connected".into(),
        }))
        .await
        .unwrap();

        // The room broadcast echo of one's own announce (server semantics: broadcast to the whole room including self) -
        // the controller must skip it by id and never consume its own
        for id in &own_ids {
            ws.send(env_text(SignalingMessage::NewDataProducer {
                room_id: ROOM.into(),
                data_producer_id: id.clone(),
                peer_id: "host".into(),
                label: "echo".into(),
                protocol: "sctp".into(),
            }))
            .await
            .unwrap();
        }
        // the cockpit's control producer (v1 single-label convention)
        ws.send(env_text(SignalingMessage::NewDataProducer {
            room_id: ROOM.into(),
            data_producer_id: "dp-cockpit".into(),
            peer_id: "veh-peer".into(),
            label: "control".into(),
            protocol: "sctp".into(),
        }))
        .await
        .unwrap();

        // exactly one ConsumeData: the cockpit producer, Recv, bound to t-recv
        let mut consumed_seen = false;
        for _ in 0..8 {
            let msg = read_msg(&mut ws).await;
            match msg {
                SignalingMessage::ConsumeData {
                    room_id,
                    transport_direction,
                    data_producer_id,
                    transport_id,
                    ..
                } => {
                    assert_eq!(room_id, ROOM);
                    assert_eq!(transport_direction, TransportDirection::Recv);
                    assert_eq!(
                        data_producer_id, "dp-cockpit",
                        "consume only the cockpit producer (no self-consumption of the own echo)"
                    );
                    assert_eq!(
                        transport_id.as_deref(),
                        Some("t-recv"),
                        "C1 explicit binding to the recv transport"
                    );
                    consumed_seen = true;
                    ws.send(env_text(SignalingMessage::DataConsumed {
                        room_id: ROOM.into(),
                        data_consumer_id: "dc-cockpit".into(),
                        data_producer_id,
                        // S2d: out-of-band negotiated DC params (assigned by the worker on real machines; fixture values here).
                        sctp_stream_parameters: Some(SctpStreamParameters {
                            stream_id: 9,
                            ordered: true,
                            max_packet_life_time: None,
                            max_retransmits: None,
                        }),
                        label: "chassis".into(),
                        protocol: String::new(),
                    }))
                    .await
                    .unwrap();
                    break;
                }
                // the gateway broadcast echo of DataProducerCreated during setup may arrive late - ignore it
                SignalingMessage::DataProducerCreated { .. } => continue,
                other => panic!("expected ConsumeData, got {other:?}"),
            }
        }
        assert!(consumed_seen, "ConsumeData must arrive after the NewDataProducer injection");

        // (4) stay alive one beat (the main loop must not crash-loop after DataConsumed), then cut the link to trigger the self-heal exit
        tokio::time::sleep(Duration::from_millis(300)).await;
        ws.close(None).await.expect("mock close");
    });

    // -- controller: the production client chain (new_gateway + control_loop) --
    let signal = SignalClient::new_gateway(
        &format!("ws://127.0.0.1:{port}/ws"),
        "host-controller",
        ROOM,
        PeerRole::Host,
    )
    .connect()
    .await
    .expect("controller signaling connect (mock gateway)");

    let code = tokio::time::timeout(
        Duration::from_secs(30),
        control_loop(
            signal,
            ControllerConfig::default(),
            Arc::new(StubActuator),
            None,
            Default::default(),
        ),
    )
    .await
    .expect("control_loop did not exit within 30s");
    // the mock drops the link on purpose = production self-heal semantics (non-zero exit awaits restart_policy)
    assert_eq!(
        code, 1,
        "a signaling disconnect should use self-heal exit code 1 (PIT-87 convention)"
    );
    mock.await.expect("mock gateway assertion failed (see the panic trace)");
}
