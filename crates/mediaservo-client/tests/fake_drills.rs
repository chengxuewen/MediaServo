//! K11 batch-0 deterministic drills -- FakeEngine + mock WS signaling over the full chain (the fake-engine posture of sfu_surface).
//!
//! The division of labor with sfu_surface.rs: that file pins "uplink signaling shapes" (real libwebrtc local negotiation),
//! this file pins the "engine behavior -> SDK semantics" mapping (frame injection/ack return/failures/DC states), CI-second scale,
//! no OS gate (the fake never touches the libwebrtc runtime; the crate's link deps match the default posture).
//!
//! Drill surface (ticket batch-0 acceptance (2)):
//! a) join -> consume -> frame callback -> ack round trip, the full chain
//! b) an engine failure -> correctly mapped to ClientError::WebRtc, the session stays usable without panicking
//! c) send's state rejection before/after DC open (+ the drop_dc_mid_send fake-delivery semantics)
//! d) two consumers never coupling (a multi-path consume surface; the ack pump single-path = the batch-1 precondition observation, see the test tail note)
//! h) K5 ready-state aggregation: the worst-state mapping (the Connecting->Open->Closed dominance) + the buffered sum

#![cfg(feature = "engine-fake")]

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use mediaservo_client::engine::Engine;
use mediaservo_client::engine::fake::FakeEngine;
use mediaservo_client::error::ClientError;
use mediaservo_client::{ClientConfig, RoomSession};
use mediaservo_common::protocol::{
    ControlAck, DtlsParameters, Fingerprint, IceParameters, MediaKind, PeerRole,
    SctpStreamParameters, SignalingMessage,
};

const ROOM: &str = "veh-fake";
const PSK: &str = "mock-psk";
const FAKE_FP: &str = "01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";

type MockWs = WebSocketStream<tokio::net::TcpStream>;

fn text(msg: &SignalingMessage) -> Message {
    serde_json::to_string(msg).expect("serialize").into()
}

async fn send_msg(ws: &mut MockWs, msg: &SignalingMessage) {
    ws.send(text(msg)).await.expect("mock: write failed");
}

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
            password: "pwdpwdpwdpwdpwdpwdpwdpwdpwdpwdpwd".into(),
        },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint { algorithm: "sha-256".into(), value: FAKE_FP.into() }],
            role: "auto".into(),
        },
        ice_candidates: None,
        sctp_parameters: None,
    }
}

struct Harness {
    session: RoomSession,
    fake: FakeEngine,
    /// The uplink signaling observation channel.
    obs: mpsc::UnboundedReceiver<SignalingMessage>,
    /// The downlink push channel (for the server's proactive NewProducer/NewDataProducer broadcasts).
    push: mpsc::UnboundedSender<SignalingMessage>,
}

/// Start the mock WS server (handshake + the full SFU scripted replies + proactive pushes merged), joining with the fake engine.
async fn pair_fake() -> Harness {
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
    let (obs_tx, obs_rx) = mpsc::unbounded_channel();
    let (push_tx, mut push_rx) = mpsc::unbounded_channel::<SignalingMessage>();
    tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut ws: MockWs = tokio_tungstenite::accept_async(sock).await.unwrap();

        // -- handshake (same shape as sfu_surface)
        let frame = ws.next().await.unwrap().unwrap();
        assert_eq!(
            frame.to_text().unwrap(),
            PSK,
            "the direct shape's first frame = the PSK plaintext"
        );
        send_msg(&mut ws, &SignalingMessage::Error { code: 0, message: "auth_ok".into() }).await;
        match read_msg(&mut ws).await {
            SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
                assert_eq!(room_id, ROOM);
                assert_eq!(peer_role, PeerRole::Consumer);
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        send_msg(
            &mut ws,
            &SignalingMessage::RoomJoined {
                room_id: ROOM.into(),
                peer_id: "consumer-test".into(),
                protocol: Some(3),
                server_version: None,
                session_nonce: None,
            },
        )
        .await;

        // -- scripted replies + push merging (peer-drop convergence)
        let mut tseq = 0u32;
        loop {
            tokio::select! {
                m = ws.next() => {
                    let Some(m) = m else { break };
                    let Ok(m) = m else { break };
                    let Some(t) = m.to_text().ok() else { continue };
                    let Ok(msg) = serde_json::from_str::<SignalingMessage>(t) else { continue };
                    let _ = obs_tx.send(msg.clone());
                    let reply = match msg {
                        SignalingMessage::CreateWebRtcTransport { .. } => {
                            tseq += 1;
                            transport_created(&format!("t-{tseq}"))
                        }
                        SignalingMessage::GetRouterRtpCapabilities { .. } => {
                            SignalingMessage::RouterRtpCapabilities {
                                room_id: ROOM.into(),
                                capabilities: serde_json::json!({
                                    "codecs": [{"mimeType": "video/VP8", "clockRate": 90000,
                                               "kind": "video", "payloadTypes": [96]}],
                                    "headerExtensions": [],
                                }),
                            }
                        }
                        SignalingMessage::Consume { .. } => SignalingMessage::Consumed {
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
                        SignalingMessage::ConnectWebRtcTransport { .. } => {
                            SignalingMessage::Error { code: 0, message: "transport_connected".into() }
                        }
                        SignalingMessage::CreateDataProducer { label, .. } => {
                            SignalingMessage::DataProducerCreated {
                                room_id: ROOM.into(),
                                data_producer_id: format!("dp-{label}"),
                            }
                        }
                        SignalingMessage::ConsumeData { data_producer_id, .. } => {
                            SignalingMessage::DataConsumed {
                                room_id: ROOM.into(),
                                data_consumer_id: "dc-ack".into(),
                                data_producer_id,
                                sctp_stream_parameters: Some(SctpStreamParameters {
                                    stream_id: 100,
                                    ordered: true,
                                    max_packet_life_time: None,
                                    max_retransmits: None,
                                }),
                                label: "ack".into(),
                                protocol: "sctp".into(),
                            }
                        }
                        other => panic!("mock: an unexpected uplink message {other:?}"),
                    };
                    send_msg(&mut ws, &reply).await;
                }
                Some(p) = push_rx.recv() => {
                    send_msg(&mut ws, &p).await;
                }
            }
        }
    });

    let fake = FakeEngine::new();
    let engine: Arc<dyn Engine> = Arc::new(fake.clone());
    let session = RoomSession::connect_with_engine(&cfg, engine)
        .await
        .expect("the fake posture's connect should succeed");
    Harness { session, fake, obs: obs_rx, push: push_tx }
}

/// Observe uplinks until the predicate hits (intermediate messages skipped).
async fn wait_uplink<F>(obs: &mut mpsc::UnboundedReceiver<SignalingMessage>, what: &str, pred: F)
where
    F: Fn(&SignalingMessage) -> bool,
{
    for _ in 0..64 {
        let m = tokio::time::timeout(Duration::from_secs(5), obs.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for an uplink ({what})"))
            .expect("the observation channel closed");
        if pred(&m) {
            return;
        }
    }
    panic!("never observed {what} (the skip window exhausted)");
}

/// Deliver DC ingress messages to the fake world until the ack pump's consumer DC is established (retrying through the race).
async fn deliver_until_hit(fake: &FakeEngine, label: &str, bytes: Vec<u8>) {
    for _ in 0..100 {
        if fake.deliver_dc_message(label, bytes.clone()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the consumer DC for label={label} was never established");
}

// -------- drills --------

/// a) join -> consume -> frame callback -> ack round trip, the full chain (fake engine + mock signaling, zero real webrtc).
#[tokio::test]
async fn drill_a_join_consume_frames_ack_roundtrip() {
    let h = pair_fake().await;
    let Harness { mut session, fake, mut obs, push } = h;

    // discover the producer (the server broadcasts NewProducer)
    push.send(SignalingMessage::NewProducer {
        room_id: ROOM.into(),
        producer_id: "prod-1".into(),
        peer_id: "host-1".into(),
        kind: MediaKind::Video,
        label: None,
    })
    .unwrap();
    assert_eq!(session.wait_video_producer(Duration::from_secs(5)).await.unwrap(), "prod-1");

    // consume -> engine negotiation (fake) -> inject a track + frames -> VideoFrame out the stream
    let mut frames = session.consume_video("prod-1").await.expect("consume_video");
    fake.inject_video_track();
    fake.inject_frame(640, 360);
    let f = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .expect("the frame callback timed out")
        .expect("the frame stream closed");
    assert_eq!((f.width, f.height), (640, 360));
    assert_eq!(f.data.len(), 640 * 360 * 3 / 2, "the I420 size");
    let s = session.video_stats_summary();
    assert_eq!(s.frames_decoded, 1, "an injected frame should count in the stats");
    assert_eq!(s.frame_width, 640);

    // the control plane: egress DC announce + the ack pump consuming the return path
    let mut ctl = session.open_control(&["chassis"]).await.expect("open_control");
    push.send(SignalingMessage::NewDataProducer {
        room_id: ROOM.into(),
        data_producer_id: "dp-ack".into(),
        peer_id: "host-1".into(),
        label: "ack".into(),
        protocol: "sctp".into(),
    })
    .unwrap();
    wait_uplink(&mut obs, "ConsumeData", |m| matches!(m, SignalingMessage::ConsumeData { .. }))
        .await;

    let env_json = serde_json::json!({"seq": 7, "cmd": "steer", "payload": {"deg": 10}});
    ctl.send("chassis", 7, "steer", env_json["payload"].clone()).await.expect("send");
    assert!(
        fake.sent_texts("chassis").first().is_some_and(|t| t.contains("\"seq\":7")),
        "the egress envelope should be booked by the fake: {:?}",
        fake.sent_texts("chassis")
    );

    let ack_bytes =
        serde_json::to_vec(&ControlAck { ack: 7, result: serde_json::json!({"ok": true}) })
            .unwrap();
    deliver_until_hit(&fake, "ack", ack_bytes).await;
    let ack = ctl.recv_ack_for(7, Duration::from_secs(5)).await.expect("recv_ack_for");
    assert_eq!(ack.ack, 7);
    assert_eq!(ack.result["ok"], serde_json::json!(true));
}

/// b) an engine connect failure -> a WebRtc typed error; the session doesn't panic and later ops succeed (no leak coupling).
#[tokio::test]
async fn drill_b_engine_fault_maps_error_and_session_survives() {
    let h = pair_fake().await;
    let Harness { mut session, fake, obs: _obs, push: _push } = h;

    fake.fail_next_connect(1);
    let e = session.consume_video("prod-x").await.unwrap_err();
    assert!(
        matches!(&e, ClientError::WebRtc(m) if m.contains("failure injected")),
        "an engine failure should map to WebRtc, got {e:?}"
    );

    // one-shot flag consumption: a second consume in the same session succeeds (no dirty-state coupling)
    let mut frames =
        session.consume_video("prod-x").await.expect("a second consume should succeed");
    fake.inject_video_track();
    fake.inject_frame(320, 240);
    assert!(tokio::time::timeout(Duration::from_secs(2), frames.recv()).await.is_ok());

    // the same-slot control-plane failure: the failure happens at create_pc (the transport reply already consumed), no pump spawned
    fake.fail_next_connect(1);
    let e2 = session.open_control(&["chassis"]).await.unwrap_err();
    assert!(matches!(e2, ClientError::WebRtc(_)), "got {e2:?}");
}

/// c) send rejected before DC open, allowed after; drop_dc_mid_send = a fake delivery (Ok but never booked).
#[tokio::test]
async fn drill_c_dc_send_state_and_mid_drop() {
    let h = pair_fake().await;
    let Harness { mut session, fake, obs: _obs, push: _push } = h;

    fake.set_dc_auto_open(false);
    let ctl = session.open_control(&["chassis"]).await.expect("open_control");

    let e = ctl.send("chassis", 1, "steer", serde_json::json!({})).await.unwrap_err();
    assert!(
        matches!(&e, ClientError::WebRtc(m) if m.contains("not open")),
        "a non-open DC must reject sends, got {e:?}"
    );
    assert!(fake.sent_texts("chassis").is_empty());

    fake.open_data_channels();
    ctl.send("chassis", 2, "steer", serde_json::json!({}))
        .await
        .expect("after open, sending should work");
    assert_eq!(fake.sent_texts("chassis").len(), 1);

    fake.drop_dc_mid_send();
    ctl.send("chassis", 3, "steer", serde_json::json!({})).await.expect("a fake delivery = Ok");
    assert_eq!(fake.sent_texts("chassis").len(), 1, "a dropped injection must not book");
}

/// d) two consumers independent (the K4 true multi-path): each frame stream never coupling; closing one keeps the other receiving;
/// a single path's stats vs the whole-session union readable in tiers.
///
/// The batch-1 precondition observation (on the ledger, not invented): the consume surface is naturally multi-path (slots registered per path), but
/// **the ack pump is once-per-session** (pump_events.take()) -- a second path's control domain needs batch 1 to lift the single-path gate.
#[tokio::test]
async fn drill_d_two_consumers_independent() {
    let h = pair_fake().await;
    let Harness { session, fake, obs: _obs, push: _push } = h;

    let mut c1 = session.consume("p1").await.expect("consume p1");
    let mut c2 = session.consume("p2").await.expect("consume p2");
    assert_eq!((c1.id(), c2.id()), ("p1", "p2"));
    fake.inject_video_track(); // both pcs each get a track
    fake.inject_frame(160, 90);
    assert!(tokio::time::timeout(Duration::from_secs(2), c1.frames().recv()).await.is_ok());
    assert!(tokio::time::timeout(Duration::from_secs(2), c2.frames().recv()).await.is_ok());

    // the K4 gain: single-path stats (each pc holds 1 frame, never cross-booked).
    assert_eq!(c1.stats().frames_decoded, 1);
    assert_eq!(c2.stats().frames_decoded, 1);

    c1.close(); // one consumer withdrew (the receiver dropped -> the slot declared dead)
    fake.inject_frame(160, 90); // both pcs get more frames: c1's sink try_send fails silently
    let f = tokio::time::timeout(Duration::from_secs(2), c2.frames().recv())
        .await
        .expect("c2 must not be dragged down by c1")
        .expect("c2's stream closed");
    assert_eq!((f.width, f.height), (160, 90));
    // the session union = folding all slots (the old _pcs semantics): two paths x 2 frames = 4 (frames went to both sinks;
    // close only affects the outflow, not the pc count).
    assert_eq!(session.video_stats_summary().frames_decoded, 4);
    assert_eq!(session.video_receiver_stats().len(), 2, "both pcs in the ledger");
}

/// h) K5 aggregation semantics: ready_state = the worst state across DCs (Connecting dominates Open, Closed dominates everything);
/// buffered_amount = the summed waterline over all DCs.
#[tokio::test]
async fn drill_h_ready_state_worst_and_buffered_sum() {
    use mediaservo_client::RTCDataChannelState;

    let h = pair_fake().await;
    let Harness { mut session, fake, obs: _obs, push: _push } = h;

    // the not-yet-open window: both channels Connecting -> aggregated Connecting
    fake.set_dc_auto_open(false);
    let ctl = session.open_control(&["chassis", "gimbal"]).await.expect("open_control");
    assert_eq!(
        ctl.ready_state(),
        RTCDataChannelState::Connecting,
        "not-open should report Connecting"
    );

    // all open -> Open; the waterline sum (the set overwrite form: 100 + 50 = 150)
    fake.open_data_channels();
    assert_eq!(ctl.ready_state(), RTCDataChannelState::Open);
    fake.set_dc_buffered("chassis", 100);
    fake.set_dc_buffered("gimbal", 50);
    assert_eq!(ctl.buffered_amount().await, 150, "the sum = each channel's waterline total");

    // one channel Closed dominates: the other still Open -> aggregated Closed
    fake.close_dc("gimbal");
    assert_eq!(
        ctl.ready_state(),
        RTCDataChannelState::Closed,
        "any channel closing should dominate the set"
    );
}

// ================== K1 resilience drills (S6/batch1a CUT3) ==================

use mediaservo_client::ConnectionState;

/// Per-connection scripted replies (pair_fake's inline logic extracted; reuses pair_fake_scripted).
fn scripted_reply(msg: &SignalingMessage, tseq: &mut u32) -> Option<SignalingMessage> {
    match msg {
        SignalingMessage::CreateWebRtcTransport { .. } => {
            *tseq += 1;
            Some(transport_created(&format!("t-{tseq}")))
        }
        SignalingMessage::GetRouterRtpCapabilities { .. } => {
            Some(SignalingMessage::RouterRtpCapabilities {
                room_id: ROOM.into(),
                capabilities: serde_json::json!({
                    "codecs": [{"mimeType": "video/VP8", "clockRate": 90000,
                               "kind": "video", "payloadTypes": [96]}],
                    "headerExtensions": [],
                }),
            })
        }
        SignalingMessage::Consume { .. } => Some(SignalingMessage::Consumed {
            room_id: ROOM.into(),
            consumer_id: "cons-script".into(),
            producer_id: "prod-1".into(),
            kind: MediaKind::Video,
            rtp_parameters: serde_json::json!({
                "codecs": [{"mimeType": "video/VP8", "payloadType": 96,
                           "clockRate": 90000, "parameters": {}}],
                "encodings": [{"ssrc": 22223333}]
            }),
        }),
        SignalingMessage::ConnectWebRtcTransport { .. } => {
            Some(SignalingMessage::Error { code: 0, message: "transport_connected".into() })
        }
        SignalingMessage::CreateDataProducer { label, .. } => {
            Some(SignalingMessage::DataProducerCreated {
                room_id: ROOM.into(),
                data_producer_id: format!("dp-{label}"),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum JoinPlan {
    Accept { nonce: Option<String> },
    Reject { code: u16 },
}

struct ScriptedHarness {
    session: RoomSession,
    fake: FakeEngine,
    obs: mpsc::UnboundedReceiver<SignalingMessage>,
    push: mpsc::UnboundedSender<SignalingMessage>,
    /// Kicking the ws simulates a break -- dropping the receiver drops the mock-server-side ws.
    kick: mpsc::UnboundedSender<()>,
    /// The server-issued session_nonce sequence (downlink observation -- separate from the uplink obs).
    nonces: mpsc::UnboundedReceiver<Option<String>>,
}

/// The scripted mock: the plan indexed by join_seq (out of range = Accept{nonce:None});
/// the outer accept loop controls each generation's connection per the plan's reject/accept; the inner select handles
/// ws pushes + push + kick.
async fn pair_fake_scripted(plan: Vec<JoinPlan>) -> ScriptedHarness {
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
    let (obs_tx, obs_rx) = mpsc::unbounded_channel();
    let (push_tx, mut push_rx) = mpsc::unbounded_channel::<SignalingMessage>();
    let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<()>();
    let (nonce_tx, nonce_rx) = mpsc::unbounded_channel::<Option<String>>();
    let plan = std::sync::Arc::new(plan);

    tokio::spawn(async move {
        let mut join_seq: usize = 0;
        loop {
            let Ok((sock, _)) = listener.accept().await else { break };
            let mut ws: MockWs = match tokio_tungstenite::accept_async(sock).await {
                Ok(ws) => ws,
                Err(_) => continue,
            };

            // -- handshake --
            let frame = match ws.next().await {
                Some(Ok(m)) => m,
                _ => break,
            };
            if frame.to_text().unwrap_or("") != PSK {
                break;
            }
            send_msg(&mut ws, &SignalingMessage::Error { code: 0, message: "auth_ok".into() })
                .await;

            // ── RoomJoin ──
            let join = read_msg(&mut ws).await;
            let _ = obs_tx.send(join.clone()); // the handshake-phase join is also observed (first connect/rejects land here)
            match join {
                SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
                    assert_eq!(room_id, ROOM);
                    assert_eq!(peer_role, PeerRole::Consumer);
                }
                other => panic!("expected RoomJoin, got {other:?}"),
            }
            let seq = join_seq;
            join_seq += 1;

            // -- plan lookup --
            let outcome = plan.get(seq).cloned().unwrap_or(JoinPlan::Accept { nonce: None });
            match outcome {
                JoinPlan::Reject { code } => {
                    tracing::info!(seq, code, "mock: rejected a connection");
                    send_msg(
                        &mut ws,
                        &SignalingMessage::Error { code, message: format!("reject-{code}") },
                    )
                    .await;
                    // flush the TCP buffer before closing -- a bare drop may send an RST so the client can't read the Error frame.
                    let _ = ws.flush().await;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    drop(ws);
                    continue;
                }
                JoinPlan::Accept { nonce } => {
                    let _ = nonce_tx.send(nonce.clone());
                    send_msg(
                        &mut ws,
                        &SignalingMessage::RoomJoined {
                            room_id: ROOM.into(),
                            peer_id: "consumer-script".into(),
                            protocol: Some(3),
                            server_version: None,
                            session_nonce: nonce,
                        },
                    )
                    .await;
                }
            }

            // -- the inner select (structurally identical to pair_fake) --
            let mut tseq = 0u32;
            loop {
                tokio::select! {
                    biased;
                    _ = kick_rx.recv() => {
                        eprintln!("[mock] kick received, dropping ws");
                        drop(ws);
                        eprintln!("[mock] ws dropped, breaking inner loop");
                        break;
                    }
                    m = ws.next() => {
                        let Some(m) = m else { break };
                        let Ok(m) = m else { break };
                        let Some(t) = m.to_text().ok() else { continue };
                        let Ok(msg) = serde_json::from_str::<SignalingMessage>(t) else { continue };
                        let _ = obs_tx.send(msg.clone());
                        if let Some(reply) = scripted_reply(&msg, &mut tseq) {
                            send_msg(&mut ws, &reply).await;
                        }
                    }
                    Some(p) = push_rx.recv() => {
                        send_msg(&mut ws, &p).await;
                    }
                }
            }
        }
    });

    let fake = FakeEngine::new();
    let engine: Arc<dyn Engine> = Arc::new(fake.clone());
    let session = RoomSession::connect_with_engine(&cfg, engine)
        .await
        .expect("the scripted posture's connect should succeed");
    ScriptedHarness { session, fake, obs: obs_rx, push: push_tx, kick: kick_tx, nonces: nonce_rx }
}

/// Wait for connection_state to reach the pred-expected value (a watch subscription).
/// Skips the initial value (avoiding a Connected -> Connected immediate hit).
async fn wait_state(
    rx: &mut tokio::sync::watch::Receiver<ConnectionState>,
    pred: impl Fn(ConnectionState) -> bool,
) {
    for _ in 0..60 {
        tokio::time::timeout(Duration::from_secs(2), rx.changed())
            .await
            .expect("the state watch timed out")
            .expect("the state watch closed");
        if pred(*rx.borrow()) {
            return;
        }
    }
    panic!("connection_state never reached the expectation (last={:?})", *rx.borrow());
}

/// Minimized break-detection diagnostics -- after a kick, wait 5s and see whether the state changes.
#[tokio::test]
async fn drill_diag_disconnect_detection() {
    let h = pair_fake_scripted(vec![JoinPlan::Accept { nonce: None }]).await;
    let ScriptedHarness { session, fake: _fake, obs: _obs, push: _push, kick, nonces: _nonces } = h;
    let mut state_rx = session.subscribe_connection_state();
    let initial = *state_rx.borrow();
    eprintln!("[diag] initial state = {initial:?}");
    eprintln!("[diag] sending kick...");
    kick.send(()).unwrap();
    for i in 1..=10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let current = *state_rx.borrow();
        eprintln!("[diag] t+{:.1}s state = {current:?}", i as f64 * 0.5);
        if current != initial {
            eprintln!(
                "[diag] state changed from {initial:?} to {current:?} at t+{:.1}s",
                i as f64 * 0.5
            );
            return;
        }
    }
    eprintln!("[diag] FAILED: state never changed from {initial:?} after 5s");
    panic!("disconnect detection failed");
}

/// e) reconnect resume -- after a break, enter Reconnecting -> the reconnect succeeds -> replay the consumer path -> frames continue
///   + the one-shot nonce triple-hit ([None, Some("n1"), None]).
#[tokio::test]
async fn drill_e_reconnect_resume_replay() {
    let h = pair_fake_scripted(vec![
        JoinPlan::Accept { nonce: Some("nonce-epoch1".into()) },
        JoinPlan::Reject { code: 5001 },
        JoinPlan::Accept { nonce: None },
    ])
    .await;
    let ScriptedHarness { mut session, fake, mut obs, push, kick, nonces } = h;

    // consume + control (the first-generation connection)
    push.send(SignalingMessage::NewProducer {
        room_id: ROOM.into(),
        producer_id: "prod-1".into(),
        peer_id: "host-1".into(),
        kind: MediaKind::Video,
        label: None,
    })
    .unwrap();
    let producer_id = session.wait_video_producer(Duration::from_secs(5)).await.unwrap();
    let mut c = session.consume(&producer_id).await.expect("consume prod-1");

    // first-frame verification (the first-generation pc)
    fake.inject_video_track();
    fake.inject_frame(160, 120);
    let f = tokio::time::timeout(Duration::from_secs(2), c.frames().recv())
        .await
        .expect("the first frame timed out")
        .expect("the stream closed");
    assert_eq!((f.width, f.height), (160, 120));

    // -- kick (simulating the break) --
    let mut nonces = nonces;
    let mut state_rx = session.subscribe_connection_state();
    kick.send(()).unwrap();

    // wait for Reconnecting -> Connected (the reconnect succeeds after retrying 5001)
    wait_state(&mut state_rx, |s| s == ConnectionState::Connected).await;

    // the nonce triple observation (downlink: the session_nonce the mock server issues in RoomJoined)
    // expected: [None(join-0), Some("nonce-epoch1")(join-1), None(join-2)]
    let mut observed_nonces: Vec<Option<String>> = Vec::new();
    for _ in 0..64 {
        if let Ok(Some(n)) = tokio::time::timeout(Duration::from_secs(2), nonces.recv()).await {
            observed_nonces.push(n);
        } else {
            break;
        }
        if observed_nonces.len() >= 3 {
            break;
        }
    }
    // the initial connection Accept{n1} -> nonce Some("n1"); after the kick the reconnect 5001(retry) -> Accept{None} -> nonce None.
    assert_eq!(
        observed_nonces,
        vec![Some("nonce-epoch1".into()), None],
        "the RoomJoined nonce sequence"
    );

    // -- post-replay frame continuation --
    fake.inject_video_track();
    fake.inject_frame(160, 120);
    let f2 = tokio::time::timeout(Duration::from_secs(2), c.frames().recv())
        .await
        .expect("the first frame after replay timed out")
        .expect("the post-replay stream closed");
    assert_eq!((f2.width, f2.height), (160, 120));
    assert_eq!(
        c.stats().frames_decoded,
        1,
        "after a pc swap the frame counts in the new pc's stats"
    );
}

/// f) the auth family terminal -- Reject{4010} -> Failed -> the supervisor exits the reconnect chain -> never retries.
#[tokio::test]
async fn drill_f_auth_fail_terminal_failed_state() {
    let h =
        pair_fake_scripted(vec![JoinPlan::Accept { nonce: None }, JoinPlan::Reject { code: 4010 }])
            .await;
    let ScriptedHarness { session, fake: _fake, mut obs, push: _push, kick, nonces: _nonces } = h;

    let mut state_rx = session.subscribe_connection_state();
    kick.send(()).unwrap();

    // wait for Failed (4010 = the auth family terminal, no retry)
    wait_state(&mut state_rx, |s| s == ConnectionState::Failed).await;

    // sleep 2s verifying no more reconnects (the supervisor has exited its loop)
    tokio::time::sleep(Duration::from_secs(2)).await;

    // confirm only 2 RoomJoins were sent (the first + 1 break-reject, no third)
    let mut join_count = 0u32;
    for _ in 0..32 {
        if let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(200), obs.recv()).await {
            if matches!(m, SignalingMessage::RoomJoin { .. }) {
                join_count += 1;
            }
        } else {
            break;
        }
    }
    assert_eq!(join_count, 2, "exactly 2 RoomJoins (the first + 1 reject then exit)");
}

/// g) Drop semantics -- after drop(session) the session destructs normally (ctx.shutdown idempotent; no panic/hang).
#[tokio::test]
async fn drill_g_drop_session_no_lingering_tasks() {
    let h = pair_fake_scripted(vec![JoinPlan::Accept { nonce: None }]).await;
    let ScriptedHarness {
        session,
        fake: _fake,
        obs: _obs,
        push: _push,
        kick: _kick,
        nonces: _nonces,
    } = h;

    // drop session → ctx.shutdown() → supervisor abort + forwarder signaled
    // timeout = hang detection (a normal drop completes instantly)
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        drop(session);
    })
    .await;
    assert!(result.is_ok(), "drop(session) should complete within 3s (no hang)");
}

// -------- msrtc-stream-not-found drills --------

/// The fast path: after the join replay collects the full label set, wait for a label not in the set -> within the 3s fallback window
/// StreamNotFound is judged (distinct from the 30s blind-wait Timeout).
#[tokio::test]
async fn drill_stream_not_found_fast_path() {
    let h = pair_fake().await;
    let Harness { mut session, fake: _fake, obs: _obs, push } = h;

    // the join replay: generator1 (labeled) arrives -> the full set = {generator1}
    push.send(SignalingMessage::NewProducer {
        room_id: ROOM.into(),
        producer_id: "prod-g1".into(),
        peer_id: "host-1".into(),
        kind: MediaKind::Video,
        label: Some("generator1".into()),
    })
    .unwrap();
    assert_eq!(
        session
            .wait_video_producer_labeled(Duration::from_secs(10), Some("generator1"))
            .await
            .unwrap(),
        "prod-g1",
        "a label in the full set matches normally"
    );

    // wait for the nonexistent generator8: the fast path's 3s fallback window -> StreamNotFound (far below the 10s cap)
    let t0 = std::time::Instant::now();
    let err = session
        .wait_video_producer_labeled(Duration::from_secs(30), Some("generator8"))
        .await
        .expect_err("a nonexistent label should error");
    assert!(
        matches!(err, ClientError::StreamNotFound { ref label } if label == "generator8"),
        "should report StreamNotFound, got {err:?}"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(10),
        "the fast path should return within the fallback window, got {:?}",
        t0.elapsed()
    );
}

/// The race window: the full set doesn't contain the target label, but a new producer arrives within the 3s fallback window -> it's waited for (no misjudgment).
#[tokio::test]
async fn drill_stream_not_found_grace_window_catches_late_producer() {
    let h = pair_fake().await;
    let Harness { mut session, fake: _fake, obs: _obs, push } = h;

    // the full set has generator1 first
    push.send(SignalingMessage::NewProducer {
        room_id: ROOM.into(),
        producer_id: "prod-g1".into(),
        peer_id: "host-1".into(),
        kind: MediaKind::Video,
        label: Some("generator1".into()),
    })
    .unwrap();
    let _ = session.wait_video_producer_labeled(Duration::from_secs(5), Some("generator1")).await;

    // generator8 starts pushing 250ms later (simulating consumer-first)
    let push_late = push.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let _ = push_late.send(SignalingMessage::NewProducer {
            room_id: ROOM.into(),
            producer_id: "prod-g8".into(),
            peer_id: "host-8".into(),
            kind: MediaKind::Video,
            label: Some("generator8".into()),
        });
    });

    // the fallback window should catch the late producer
    assert_eq!(
        session
            .wait_video_producer_labeled(Duration::from_secs(30), Some("generator8"))
            .await
            .unwrap(),
        "prod-g8",
        "a producer arriving within the 3s fallback window should be waited for"
    );
}
