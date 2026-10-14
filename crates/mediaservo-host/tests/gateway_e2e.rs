//! D1 host-agent signaling gateway tests (local mock WS server, no Docker dependency).
//!
//! Covered contracts (Momus HIGH-1 protocol semantics):
//! (1) many local clients -> one remote WS, bidirectional forwarding + room rewriting (vehicle room <-> child-process room)
//! (2) RoomJoin interception: child-process RoomJoins never go upstream (the agent joins once); children get a
//!    locally synthesized RoomJoined (carrying the vehicle peer_id)
//! (3) concurrent negotiations: two in-flight CreateWebRtcTransports -> responses route by FIFO without crossing
//!    (a NewProducer broadcast wedged between responses does not consume the FIFO)
//! (4) disconnect/reconnect: remote WS drops -> B1 reconnect -> forwarding resumes; in-flight requests clear on disconnect
//! (5) P2P Sdp/ICE single-negotiation routing: echo dedupe + negotiation ownership handover

#![allow(dead_code)] // the Ws alias = a suite-common slot (unconsumed on the current branch)
use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceParameters, MediaKind, PeerRole, SignalingMessage,
    TransportDirection,
};
use mediaservo_host::gateway::{GatewayConfig, LocalEnvelope, run_gateway};
use mediaservo_link::{DeviceIdentity, RetryConfig};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// The mock server side (accept_async over a raw TCP stream).
type WsServer = WebSocketStream<TcpStream>;
/// The local child-process side (connect_async, possibly a TLS stream).
type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Either WS stream (the generic bound of the client/server shared helper).
trait WsIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl WsIo for TcpStream {}
impl WsIo for MaybeTlsStream<TcpStream> {}

type Ws = WebSocketStream<TcpStream>;

const VEHICLE_ROOM: &str = "vehicle-1";
const VEHICLE_PEER: &str = "veh-peer";

fn cfg(remote: SocketAddr) -> GatewayConfig {
    GatewayConfig {
        local_port: 0, // ephemeral port
        remote_url: format!("ws://{remote}/ws"),
        psk: "test-psk".into(),
        device: None,
        room: VEHICLE_ROOM.into(),
        retry: RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(500),
        },
    }
}

fn env(src: &str, msg: SignalingMessage) -> String {
    serde_json::to_string(&LocalEnvelope { src: src.into(), msg }).unwrap()
}

/// Full mock-server handshake: PSK -> Error{0} ack -> RoomJoin -> RoomJoined.
/// Returns (ws, requested room, role).
async fn mock_handshake(listener: &TcpListener) -> (WsServer, String, PeerRole) {
    let (stream, _) = listener.accept().await.expect("mock accept");
    let mut ws = tokio_tungstenite::accept_async(stream).await.expect("mock ws handshake");
    let psk = ws.next().await.unwrap().unwrap();
    assert!(matches!(psk, Message::Text(_)), "first message should be PSK text");
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::Error { code: 0, message: String::new() })
            .unwrap(),
    ))
    .await
    .unwrap();
    let join = ws.next().await.unwrap().unwrap();
    let (room, role) = match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap())
        .expect("mock parses RoomJoin")
    {
        SignalingMessage::RoomJoin { room_id, peer_role, .. } => (room_id, peer_role),
        other => panic!("expected RoomJoin, got {other:?}"),
    };
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::RoomJoined {
            room_id: room.clone(),
            peer_id: VEHICLE_PEER.into(),
            protocol: None,
            server_version: None,
            session_nonce: None,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    (ws, room, role)
}

async fn local(port: u16) -> WsClient {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .expect("local client connect");
    ws
}

async fn read_env<S: WsIo>(ws: &mut WebSocketStream<S>) -> (String, SignalingMessage) {
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("read timeout")
        .expect("ws closed")
        .expect("ws error");
    let text = msg.to_text().expect("should be text").to_string();
    let env: LocalEnvelope = serde_json::from_str(&text).expect("envelope parses");
    (env.src, env.msg)
}

/// Child-process RoomJoin (interception semantics): Error 5001 when the gateway is not ready -> retry.
/// Returns the synthesized RoomJoined's peer_id.
async fn join<S: WsIo>(ws: &mut WebSocketStream<S>, room: &str) -> String {
    for _ in 0..100 {
        ws.send(Message::Text(env(
            "child",
            SignalingMessage::RoomJoin {
                device_id: None,
                device_secret: None,
                device_pubkey: None,
                protocol: None,
                client_version: None,
                room_id: room.into(),
                peer_role: PeerRole::Host,
                stream_id: None,
                resume: None,
            },
        )))
        .await
        .unwrap();
        let (_src, msg) = read_env(ws).await;
        match msg {
            SignalingMessage::RoomJoined { room_id, peer_id, .. } => {
                assert_eq!(
                    room_id, room,
                    "the synthesized RoomJoined should echo the child process's room"
                );
                return peer_id;
            }
            SignalingMessage::Error { code: 5001, .. } => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => panic!("unexpected join response: {other:?}"),
        }
    }
    panic!("join retries exhausted");
}

/// Read 2 messages: the transport response + the NewProducer broadcast (any order).
/// Returns (transport_id, response peer_id) - the mock echoes the request marker (peer_id as a tag),
/// so any crossing is detectable (A receiving req-b's response = mismatch).
async fn collect_transport(ws: &mut WsClient, room: &str) -> (String, String) {
    let mut transport: Option<String> = None;
    let mut peer: Option<String> = None;
    for _ in 0..2 {
        let (_, m) = read_env(ws).await;
        match m {
            SignalingMessage::WebRtcTransportCreated { transport_id, room_id, peer_id, .. } => {
                assert_eq!(
                    room_id, room,
                    "the transport response room should be rewritten to {room}"
                );
                transport = Some(transport_id);
                peer = Some(peer_id);
            }
            SignalingMessage::NewProducer { room_id, .. } => {
                assert_eq!(
                    room_id, room,
                    "the NewProducer broadcast room should be rewritten to {room}"
                );
            }
            other => panic!("unexpected message in {room}: {other:?}"),
        }
    }
    (
        transport.expect("a transport response should arrive"),
        peer.expect("the response should carry the identity echo"),
    )
}

fn transport_created_for(transport_id: &str, peer_id: &str) -> SignalingMessage {
    SignalingMessage::WebRtcTransportCreated {
        room_id: VEHICLE_ROOM.into(),
        peer_id: peer_id.into(),
        transport_id: transport_id.into(),
        sctp_parameters: None, // P1 additive field - the fixture does not consume it
        ice_parameters: IceParameters { username_fragment: "ufrag".into(), password: "pwd".into() },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB:CC".into(),
            }],
            role: "auto".into(),
        },
        ice_candidates: None,
    }
}

/// mock response: the transport creation ack (peer_id identity echo).
fn transport_created(transport_id: &str) -> SignalingMessage {
    transport_created_for(transport_id, VEHICLE_PEER)
}

/// CreateWebRtcTransport request (peer_id as the request marker; the room is rewritten by the gateway).
fn create(room: &str, peer_id: &str) -> SignalingMessage {
    SignalingMessage::CreateWebRtcTransport {
        room_id: room.into(),
        peer_id: peer_id.into(),
        direction: TransportDirection::Send,
    }
}

// -- (1) multi-local-client forwarding correctness (bidirectional room rewriting + src=server delivery) --────────────

#[tokio::test]
async fn multiple_clients_forward_with_room_rewrite() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, room, role) = mock_handshake(&listener).await;
        assert_eq!(
            room, VEHICLE_ROOM,
            "the agent should join exactly once with the vehicle identity"
        );
        assert_eq!(role, PeerRole::Host);

        // two upstream messages (from different child processes): both rooms rewritten to the vehicle room
        let m1: SignalingMessage =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(
            matches!(&m1, SignalingMessage::Frame { room_id, .. } if room_id == VEHICLE_ROOM),
            "the Frame room should be rewritten to the vehicle room, got {m1:?}"
        );
        let m2: SignalingMessage =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(
            matches!(&m2, SignalingMessage::EncoderStatus { room_id, .. } if room_id == VEHICLE_ROOM),
            "the EncoderStatus room should be rewritten to the vehicle room, got {m2:?}"
        );

        // downstream room-level broadcast: each child process receives the rewrite for its own room
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::NewProducer {
                room_id: VEHICLE_ROOM.into(),
                producer_id: "p1".into(),
                peer_id: VEHICLE_PEER.into(),
                kind: MediaKind::Video,
                label: None,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await; // keep the connection until the assertions complete
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    a.send(Message::Text(env(
        "a",
        SignalingMessage::Frame {
            room_id: "room-a".into(),
            codec: "vp8".into(),
            sequence: 1,
            is_keyframe: true,
            data_base64: "AA==".into(),
        },
    )))
    .await
    .unwrap();
    b.send(Message::Text(env(
        "b",
        SignalingMessage::EncoderStatus {
            room_id: "room-b".into(),
            peer_id: "host".into(),
            codec: "vp8".into(),
            encoder_backend: "software".into(),
            encoder_implementation: None,
            frames_per_second: 30.0,
            frame_width: 640,
            frame_height: 360,
            avg_encode_ms: None,
        },
    )))
    .await
    .unwrap();

    // downstream broadcast: each child receives its own room rewrite + src=server
    let (src, m) = read_env(&mut a).await;
    assert_eq!(src, "server");
    assert!(
        matches!(&m, SignalingMessage::NewProducer { room_id, .. } if room_id == "room-a"),
        "child process a should receive the room-a rewrite, got {m:?}"
    );
    let (src, m) = read_env(&mut b).await;
    assert_eq!(src, "server");
    assert!(
        matches!(&m, SignalingMessage::NewProducer { room_id, .. } if room_id == "room-b"),
        "child process b should receive the room-b rewrite, got {m:?}"
    );
    server.await.unwrap();
}

// -- (2) RoomJoin interception: nothing upstream + local synthesized RoomJoined --─────────────────────

#[tokio::test]
async fn room_join_intercepted_not_forwarded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, room, role) = mock_handshake(&listener).await;
        assert_eq!(room, VEHICLE_ROOM);
        assert_eq!(role, PeerRole::Host);
        // the child's RoomJoin never goes upstream: no message should arrive within 2s (and certainly not a RoomJoin)
        let leaked = tokio::time::timeout(Duration::from_secs(2), ws.next()).await;
        assert!(
            leaked.is_err(),
            "a child's RoomJoin leaked to the remote: {:?}",
            leaked.ok().flatten().map(|m| format!("{m:?}"))
        );
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let peer_id = join(&mut a, "child-room").await;
    assert_eq!(
        peer_id, VEHICLE_PEER,
        "the synthesized RoomJoined should carry the vehicle peer_id"
    );
    server.await.unwrap();
}

// -- (3) concurrent negotiations: two in-flight Creates -> FIFO response routing without crossing --──────────────────

#[tokio::test]
async fn concurrent_sfu_negotiation_routes_responses() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, _room, _role) = mock_handshake(&listener).await;

        // two Creates (order = response order): extract the request markers (peer_id as the tag)
        let mut markers = Vec::new();
        for _ in 0..2 {
            let c: SignalingMessage =
                serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            match &c {
                SignalingMessage::CreateWebRtcTransport { peer_id, room_id, direction, .. } => {
                    assert_eq!(
                        room_id, VEHICLE_ROOM,
                        "the Create room should be rewritten to the vehicle room"
                    );
                    assert_eq!(direction, &TransportDirection::Send);
                    markers.push(peer_id.clone());
                }
                other => panic!("expected CreateWebRtcTransport, got {other:?}"),
            }
        }
        // response sequence: t1(markers[0]) -> NewProducer broadcast (wedged between responses) -> t2(markers[1])
        ws.send(Message::Text(
            serde_json::to_string(&transport_created_for("t1", &markers[0])).unwrap(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::NewProducer {
                room_id: VEHICLE_ROOM.into(),
                producer_id: "p9".into(),
                peer_id: VEHICLE_PEER.into(),
                kind: MediaKind::Video,
                label: None,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&transport_created_for("t2", &markers[1])).unwrap(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // both send Create at the same time (in flight; peer_id carries each request marker)
    a.send(Message::Text(env("a", create("room-a", "req-a")))).await.unwrap();
    b.send(Message::Text(env("b", create("room-b", "req-b")))).await.unwrap();

    // each reads 2: its own transport response + the NewProducer broadcast (order varies - the server's
    // broadcast channel races the direct response, so the broadcast may arrive before the second response)
    let (ta, pa) = collect_transport(&mut a, "room-a").await;
    let (tb, pb) = collect_transport(&mut b, "room-b").await;
    assert_eq!(pa, "req-a", "A must receive the response to its own request (identity echo)");
    assert_eq!(pb, "req-b", "B must receive the response to its own request (identity echo)");
    assert_ne!(ta, tb, "the two transport_ids must not cross");
    assert!(ta == "t1" || ta == "t2");
    assert!(tb == "t1" || tb == "t2");
    server.await.unwrap();
}

// -- (4) disconnect/reconnect: forwarding recovery + in-flight request clearing --───────────────────────────────

#[tokio::test]
async fn reconnect_resumes_forwarding() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        // connection 1: full handshake -> receive one message -> close (simulates a remote drop)
        let (mut ws1, _r, _role) = mock_handshake(&listener).await;
        let m1 = ws1.next().await.unwrap().unwrap();
        assert!(
            m1.to_text().unwrap().contains("\"type\":\"sdp\""),
            "connection 1 should receive the child process Sdp"
        );
        drop(ws1);

        // connection 2: handshake -> receive Create -> respond
        let (mut ws2, _r2, _role2) = mock_handshake(&listener).await;
        let c = ws2.next().await.unwrap().unwrap();
        let c: SignalingMessage = serde_json::from_str(c.to_text().unwrap()).unwrap();
        assert!(
            matches!(&c, SignalingMessage::CreateWebRtcTransport { room_id, .. }
                if room_id == VEHICLE_ROOM),
            "the Create room after reconnect should be the vehicle room, got {c:?}"
        );
        ws2.send(Message::Text(serde_json::to_string(&transport_created("t-after")).unwrap()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    join(&mut a, "room-a").await;

    // before the drop: Sdp forwarded (mock connection 1 receives it then closes)
    a.send(Message::Text(env(
        "a",
        SignalingMessage::Sdp { room_id: "room-a".into(), target: None, sdp: "v=0 offer".into() },
    )))
    .await
    .unwrap();

    // after reconnect: Create in flight (5001 or no answer during the disconnect window -> retry) -> the response routes back to this connection
    let response = loop {
        a.send(Message::Text(env(
            "a",
            SignalingMessage::CreateWebRtcTransport {
                room_id: "room-a".into(),
                peer_id: "host".into(),
                direction: TransportDirection::Send,
            },
        )))
        .await
        .unwrap();
        match tokio::time::timeout(Duration::from_millis(300), read_env(&mut a)).await {
            Ok((_, SignalingMessage::Error { code: 5001, .. })) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok((_, m)) => break m,
            Err(_) => {} // disconnect window: no answer for the request, resend
        }
    };
    assert!(
        matches!(&response, SignalingMessage::WebRtcTransportCreated { transport_id, room_id, .. }
            if transport_id == "t-after" && room_id == "room-a"),
        "after reconnect the response should route to this connection with the room rewritten, got {response:?}"
    );
    server.await.unwrap();
}

// -- (5) P2P Sdp/ICE single-negotiation routing: echo dedupe + ownership handover --─────────────────────

#[tokio::test]
async fn p2p_sdp_ice_single_negotiation_routing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, _r, _role) = mock_handshake(&listener).await;

        // receive A's offer -> echo it verbatim (simulates the server room broadcast) + answer from the remote
        let offer = ws.next().await.unwrap().unwrap();
        assert!(offer.to_text().unwrap().contains("\"type\":\"sdp\""));
        ws.send(offer).await.unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::Sdp {
                room_id: VEHICLE_ROOM.into(),
                target: Some("remote-peer".into()),
                sdp: "b-answer".into(),
            })
            .unwrap(),
        ))
        .await
        .unwrap();

        // receive B's ICE -> echo verbatim + remote candidate
        let ice = ws.next().await.unwrap().unwrap();
        assert!(ice.to_text().unwrap().contains("\"type\":\"r_t_c_ice_candidate\""));
        ws.send(ice).await.unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::RTCIceCandidate {
                room_id: VEHICLE_ROOM.into(),
                target: None,
                candidate: "peer-cand".into(),
                sdp_mid: Some("0".into()),
                sdp_mline_index: Some(0),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // A sends the offer (negotiation ownership = A)
    a.send(Message::Text(env(
        "a",
        SignalingMessage::Sdp { room_id: "room-a".into(), target: None, sdp: "a-offer".into() },
    )))
    .await
    .unwrap();
    // the first message A receives must be the remote answer (its own echo was deduped)
    let (_, m) = read_env(&mut a).await;
    assert!(
        matches!(&m, SignalingMessage::Sdp { sdp, room_id, .. }
            if sdp == "b-answer" && room_id == "room-a"),
        "A should receive the remote answer (not its own echo), room rewritten, got {m:?}"
    );

    // B sends ICE (ownership switches to B) -> receives the remote candidate (not its own echo)
    b.send(Message::Text(env(
        "b",
        SignalingMessage::RTCIceCandidate {
            room_id: "room-b".into(),
            target: None,
            candidate: "b-cand".into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        },
    )))
    .await
    .unwrap();
    let (_, m) = read_env(&mut b).await;
    assert!(
        matches!(&m, SignalingMessage::RTCIceCandidate { candidate, room_id, .. }
            if candidate == "peer-cand" && room_id == "room-b"),
        "B should receive the remote candidate (not its own echo), room rewritten, got {m:?}"
    );
    server.await.unwrap();
}

// -- CRITICAL-1 regression: a disconnect-window 5001 must not leave a stale pending slot --───────────

#[tokio::test]
async fn disconnect_window_stale_pending_not_cross_routed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // gate: mock2 does not accept until the test releases it -> the agent blocks in the WS handshake, joined stays
    // false (a deterministic disconnect window - connect in flight with no timeout)
    let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        // connection 1: receive A's in-flight request -> close without responding
        let (mut ws1, _r, _role) = mock_handshake(&listener).await;
        let m1 = ws1.next().await.unwrap().unwrap();
        let m1: SignalingMessage = serde_json::from_str(m1.to_text().unwrap()).unwrap();
        assert!(
            matches!(&m1, SignalingMessage::CreateWebRtcTransport { peer_id, .. } if peer_id == "req-a1"),
            "mock1 should receive A's in-flight request, got {m1:?}"
        );
        drop(ws1);
        // wait for the test to release mock2
        let mut g = gate_rx.clone();
        while !*g.borrow() {
            g.changed().await.unwrap();
        }
        // connection 2: receive B's request -> respond (identity echo)
        let (mut ws2, _r2, _role2) = mock_handshake(&listener).await;
        let c2 = ws2.next().await.unwrap().unwrap();
        let c2: SignalingMessage = serde_json::from_str(c2.to_text().unwrap()).unwrap();
        assert!(
            matches!(&c2, SignalingMessage::CreateWebRtcTransport { peer_id, .. } if peer_id == "req-b"),
            "mock2 should receive B's request, got {c2:?}"
        );
        ws2.send(Message::Text(
            serde_json::to_string(&transport_created_for("t-b", "req-b")).unwrap(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // A sends its in-flight request before the disconnect (mock1 receives it then closes without responding -> the disconnect window begins)
    a.send(Message::Text(env("a", create("room-a", "req-a1")))).await.unwrap();

    // window: A repeatedly sends Create until 5001 (after the fix: not joined -> not queued in pending)
    let mut saw_5001 = false;
    for _ in 0..50 {
        a.send(Message::Text(env("a", create("room-a", "req-a2")))).await.unwrap();
        match tokio::time::timeout(Duration::from_millis(300), read_env(&mut a)).await {
            Ok((_, SignalingMessage::Error { code: 5001, .. })) => {
                saw_5001 = true;
                break;
            }
            Ok((_, m)) => panic!("the disconnect window must not yield a response: {m:?}"),
            Err(_) => {} // forwarding to a dead connection: no answer, retry
        }
    }
    assert!(saw_5001, "A should hit the disconnect window (5001)");

    // release mock2 -> the agent completes its reconnect
    gate_tx.send(true).unwrap();

    // B sends a request (5001/no-answer retries, bounded to prevent hangs) -> must receive its own response
    let resp = 'b_loop: {
        for _ in 0..50 {
            b.send(Message::Text(env("b", create("room-b", "req-b")))).await.unwrap();
            match tokio::time::timeout(Duration::from_millis(300), read_env(&mut b)).await {
                Ok((_, SignalingMessage::Error { code: 5001, .. })) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok((_, m)) => break 'b_loop m,
                Err(_) => {}
            }
        }
        panic!("B's retries exhausted without a response (stale pending crossing)");
    };
    assert!(
        matches!(&resp, SignalingMessage::WebRtcTransportCreated { peer_id, transport_id, .. }
            if peer_id == "req-b" && transport_id == "t-b"),
        "B should receive the response to its own request, got {resp:?}"
    );

    // A must not receive any response (before the fix: the stale slot pops -> A gets B's response = crossing)
    let extra = tokio::time::timeout(Duration::from_millis(300), read_env(&mut a)).await;
    assert!(
        extra.is_err(),
        "A should not receive a response (stale pending crossing), got {:?}",
        extra.ok().map(|(_, m)| m)
    );
    server.await.unwrap();
}

// -- IMPORTANT-3 regression: relayed Frames must not steal P2P negotiation ownership --─────────────

#[tokio::test]
async fn frame_relay_does_not_steal_p2p_ownership() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, _r, _role) = mock_handshake(&listener).await;
        // receive the offer (A) + the Frame (B), any order; then echo both + answer from the remote
        let mut offer: Option<Message> = None;
        let mut frame: Option<Message> = None;
        for _ in 0..2 {
            let m = ws.next().await.unwrap().unwrap();
            let t = m.to_text().unwrap().to_string();
            if t.contains("\"type\":\"sdp\"") {
                offer = Some(m);
            } else if t.contains("\"type\":\"frame\"") {
                frame = Some(m);
            } else {
                panic!("mock got an unexpected message: {t}");
            }
        }
        ws.send(offer.unwrap()).await.unwrap();
        ws.send(frame.unwrap()).await.unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::Sdp {
                room_id: VEHICLE_ROOM.into(),
                target: Some("remote-peer".into()),
                sdp: "b-answer".into(),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // A sends the offer (negotiation ownership = A)
    a.send(Message::Text(env(
        "a",
        SignalingMessage::Sdp { room_id: "room-a".into(), target: None, sdp: "a-offer".into() },
    )))
    .await
    .unwrap();
    // B sends a Frame (a relay message; before the fix is_relay_msg would steal ownership)
    b.send(Message::Text(env(
        "b",
        SignalingMessage::Frame {
            room_id: "room-b".into(),
            codec: "vp8".into(),
            sequence: 1,
            is_keyframe: true,
            data_base64: "AA==".into(),
        },
    )))
    .await
    .unwrap();

    // A receives the remote answer (its own offer echo deduped; ownership not stolen by the Frame)
    let (_, m) = read_env(&mut a).await;
    assert!(
        matches!(&m, SignalingMessage::Sdp { sdp, room_id, .. }
            if sdp == "b-answer" && room_id == "room-a"),
        "A should receive the remote answer (ownership not stolen by the Frame), got {m:?}"
    );
    // B must not receive anything (the Frame echo is deduped; the answer is not B's)
    let extra = tokio::time::timeout(Duration::from_millis(300), read_env(&mut b)).await;
    assert!(extra.is_err(), "B should not receive a message, got {:?}", extra.ok().map(|(_, m)| m));
    server.await.unwrap();
}

// -- device-enroll: remote Join public-key fingerprint full chain (challenge -> answer -> joined) --────────

#[tokio::test]
async fn remote_join_carries_device_credentials() {
    use base64::Engine as _;
    use ed25519_dalek::VerifyingKey;
    // the gateway-side identity = the same sig_vector anchor seed (batch-3 cross-verification of the same constant system)
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    let ident = DeviceIdentity::new("ms-gw-device", ed25519_dalek::SigningKey::from_bytes(&seed));
    let expect_pk = ident.pubkey_b64.clone();
    // the mock remote server: after the PSK ack, assert RoomJoin carries device_pubkey (pubkey form, no secret),
    // send a challenge -> assert the answer passes verification (the server's own verify_strict) -> RoomJoined.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("mock accept");
        let mut ws = tokio_tungstenite::accept_async(stream).await.expect("mock ws handshake");
        let psk = ws.next().await.unwrap().unwrap();
        assert!(matches!(psk, Message::Text(_)), "first message should be PSK text");
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::Error { code: 0, message: String::new() })
                .unwrap(),
        ))
        .await
        .unwrap();
        let join = ws.next().await.unwrap().unwrap();
        let nonce_raw: Vec<u8> = (0x40u8..0x60).collect();
        match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap()).unwrap() {
            SignalingMessage::RoomJoin { device_id, device_secret, device_pubkey, .. } => {
                assert_eq!(
                    device_id.as_deref(),
                    Some("ms-gw-device"),
                    "the remote RoomJoin should carry device_id"
                );
                assert_eq!(
                    device_secret, None,
                    "the pubkey form carries no secret (D-E3 mutual exclusion of the two forms)"
                );
                assert_eq!(
                    device_pubkey.as_deref(),
                    Some(expect_pk.as_str()),
                    "it should carry the assembled public-key fingerprint"
                );
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::DeviceAuthChallenge {
                nonce: base64::engine::general_purpose::STANDARD.encode(&nonce_raw),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        let resp = ws.next().await.unwrap().unwrap();
        match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
            SignalingMessage::DeviceAuthResponse { room_id, sig } => {
                assert_eq!(room_id, VEHICLE_ROOM, "the answer's room echoes the vehicle room");
                let vk_arr: [u8; 32] = base64::engine::general_purpose::STANDARD
                    .decode(&expect_pk)
                    .unwrap()
                    .as_slice()
                    .try_into()
                    .unwrap();
                let sig_arr: [u8; 64] = base64::engine::general_purpose::STANDARD
                    .decode(&sig)
                    .unwrap()
                    .as_slice()
                    .try_into()
                    .unwrap();
                let mut msg = nonce_raw;
                msg.extend_from_slice(b"ms-gw-device");
                msg.extend_from_slice(VEHICLE_ROOM.as_bytes());
                VerifyingKey::from_bytes(&vk_arr)
                    .unwrap()
                    .verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&sig_arr))
                    .expect("the gateway answer must pass verification");
            }
            other => panic!("expected DeviceAuthResponse, got {other:?}"),
        }
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::RoomJoined {
                room_id: VEHICLE_ROOM.into(),
                peer_id: VEHICLE_PEER.into(),
                protocol: None,
                server_version: None,
                session_nonce: None,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await; // keep the connection until the assertions complete
    });

    // the gateway carries the device identity (host-agent assembles identity.json + signing.pem - device-enroll T6/T7)
    let mut gateway_cfg = cfg(addr);
    gateway_cfg.device = Some(ident);
    let (port, _handle) = run_gateway(gateway_cfg).await.expect("run_gateway");
    // local child-process join: the synthesized RoomJoined proves the remote session is ready (full connection chain)
    let mut child = local(port).await;
    let peer = join(&mut child, "child-room").await;
    assert_eq!(peer, VEHICLE_PEER);
    server.await.unwrap();
}
