//! Phase 1b: SignalClient tests (local mock WS server).

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{DeviceIdentity, LinkError, SignalClient, SignalEvent};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn connect_auth_join_and_roundtrip() {
    // local mock WS server: PSK auth -> RoomJoin -> RoomJoined -> echo
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

        // 1) receive PSK (first text frame)
        let psk_msg = ws.next().await.unwrap().unwrap();
        assert!(matches!(psk_msg, Message::Text(_)), "the first frame must be the PSK text");

        // 2) send the auth ack Error{code:0}
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap())).await.unwrap();

        // 3) receive RoomJoin -> send RoomJoined
        let join_msg = ws.next().await.unwrap().unwrap();
        let join: SignalingMessage = serde_json::from_str(join_msg.to_text().unwrap()).unwrap();
        let room_id = match join {
            SignalingMessage::RoomJoin { room_id, .. } => room_id,
            _ => panic!("expected RoomJoin"),
        };
        let joined = SignalingMessage::RoomJoined {
            room_id,
            peer_id: "peer-1".to_string(),
            protocol: None,
            server_version: None,
            session_nonce: None,
        };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap())).await.unwrap();

        // 4) echo loop: reply one for each received
        while let Some(Ok(msg)) = ws.next().await {
            if ws.send(msg).await.is_err() {
                break;
            }
        }
    });

    // client
    let client =
        SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "test-room", PeerRole::Host);
    let session = client.connect().await.expect("connect");
    assert_eq!(session.room_id(), "test-room");
    assert_eq!(session.peer_id(), "peer-1", "RoomJoined peer_id must be accessible");
    assert_eq!(session.room_id(), "test-room");
    let mut events = session.events();

    // send an Sdp, expect the server to echo it back
    session
        .send(SignalingMessage::Sdp {
            room_id: "test-room".to_string(),
            target: None,
            sdp: "v=0".to_string(),
        })
        .await
        .expect("send");

    // receive events (Connected first, then the echoed Message)
    let echoed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match events.recv().await.unwrap() {
                SignalEvent::Message(SignalingMessage::Sdp { sdp, .. }) => return sdp,
                _ => continue, // skip Connected and similar events
            }
        }
    })
    .await
    .expect("echo timeout");
    assert_eq!(echoed, "v=0");

    session.close().await.expect("close");
    server.await.unwrap();
}

// -- D2: local gateway mode (LocalEnvelope wire, no PSK challenge) --

#[tokio::test]
async fn gateway_mode_connects_with_envelope_wire() {
    // mock gateway: the first message must be a LocalEnvelope wrapping RoomJoin (no PSK challenge!);
    // it replies LocalEnvelope{RoomJoined}, then echoes envelopes.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

        // 1) the first frame must be an envelope (gateway local wire has no PSK challenge)
        let first = ws.next().await.unwrap().unwrap();
        let env: mediaservo_link::LocalEnvelope = serde_json::from_str(first.to_text().unwrap())
            .expect("the first frame must be a LocalEnvelope");
        assert_eq!(env.src, "child-1");
        let room_id = match env.msg {
            SignalingMessage::RoomJoin { room_id, .. } => room_id,
            other => panic!("expected RoomJoin in envelope, got {other:?}"),
        };

        // 2) the envelope replies RoomJoined
        let joined = mediaservo_link::LocalEnvelope {
            src: "server".into(),
            msg: SignalingMessage::RoomJoined {
                room_id,
                peer_id: "veh-peer".into(),
                protocol: None,
                server_version: None,
                session_nonce: None,
            },
        };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap())).await.unwrap();

        // 3) echo loop (envelopes)
        while let Some(Ok(msg)) = ws.next().await {
            if ws.send(msg).await.is_err() {
                break;
            }
        }
    });

    let client = SignalClient::new_gateway(
        &format!("ws://{addr}/ws"),
        "child-1",
        "stream-s0",
        PeerRole::Host,
    );
    let session = client.connect().await.expect("gateway connect");
    assert_eq!(session.room_id(), "stream-s0");
    assert_eq!(
        session.peer_id(),
        "veh-peer",
        "synthesized RoomJoined carries the vehicle-level peer_id"
    );
    let mut events = session.events();

    session
        .send(SignalingMessage::Sdp {
            room_id: "stream-s0".to_string(),
            target: None,
            sdp: "v=0".to_string(),
        })
        .await
        .expect("send");
    let echoed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match events.recv().await.unwrap() {
                SignalEvent::Message(SignalingMessage::Sdp { sdp, .. }) => return sdp,
                _ => continue,
            }
        }
    })
    .await
    .expect("echo timeout");
    assert_eq!(echoed, "v=0");

    session.close().await.expect("close");
    server.await.unwrap();
}

#[tokio::test]
async fn gateway_mode_room_join_denied_returns_error() {
    // gateway not connected to the remote server -> intercepted RoomJoin replies Error 5001 (inside an envelope)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _env = ws.next().await.unwrap().unwrap();
        let deny = mediaservo_link::LocalEnvelope {
            src: "server".into(),
            msg: SignalingMessage::Error {
                code: 5001,
                message: "gateway not connected to server".into(),
            },
        };
        ws.send(Message::Text(serde_json::to_string(&deny).unwrap())).await.unwrap();
    });

    let client =
        SignalClient::new_gateway(&format!("ws://{addr}/ws"), "child-1", "r", PeerRole::Host);
    let err = client.connect().await.unwrap_err();
    assert!(err.to_string().contains("5001"), "must report 5001, got: {err}");
    server.await.unwrap();
}

#[tokio::test]
async fn auth_denied_returns_error() {
    // mock server: auth denied
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let deny = SignalingMessage::Error {
            code: 4003,
            message: "PSK authentication failed".to_string(),
        };
        ws.send(Message::Text(serde_json::to_string(&deny).unwrap())).await.unwrap();
    });

    let client = SignalClient::new(&format!("ws://{addr}/ws"), "wrong-psk", "r", PeerRole::Host);
    let err = client.connect().await.unwrap_err();
    assert!(err.to_string().contains("auth denied [4003]"), "must report auth denial, got: {err}");

    server.await.unwrap();
}

// ---- G4: device credential (D-H11) - carried by RoomJoin (additive; default = PSK path) ----

#[tokio::test]
async fn room_join_carries_device_credentials() {
    // mock server: after the PSK ack, assert RoomJoin carries device_id/device_secret
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap())).await.unwrap();
        let join_msg = ws.next().await.unwrap().unwrap();
        let join: SignalingMessage = serde_json::from_str(join_msg.to_text().unwrap()).unwrap();
        match join {
            SignalingMessage::RoomJoin { device_id, device_secret, .. } => {
                assert_eq!(
                    device_id.as_deref(),
                    Some("ms-001122334455"),
                    "RoomJoin must carry device_id"
                );
                assert_eq!(
                    device_secret.as_deref(),
                    Some("s3cr3t"),
                    "RoomJoin must carry device_secret"
                );
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        let joined = SignalingMessage::RoomJoined {
            room_id: "r".into(),
            peer_id: "peer-1".into(),
            protocol: None,
            server_version: None,
            session_nonce: None,
        };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap())).await.unwrap();
    });
    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "r", PeerRole::Host)
        .with_device_credentials(mediaservo_link::DeviceCredential {
            device_id: "ms-001122334455".into(),
            device_secret: "s3cr3t".into(),
        });
    let session = client.connect().await.expect("connect with device credentials");
    assert_eq!(session.peer_id(), "peer-1");
    session.close().await.expect("close");
    server.await.unwrap();
}

#[tokio::test]
async fn room_join_denied_surfaces_device_auth_error() {
    // credential rejected by the server (since G2: Error 4010 device auth failed) -> the client must error explicitly (C15/C16)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap())).await.unwrap();
        let _join = ws.next().await.unwrap().unwrap();
        let deny = SignalingMessage::Error {
            code: 4010,
            message: "device authentication failed".to_string(),
        };
        ws.send(Message::Text(serde_json::to_string(&deny).unwrap())).await.unwrap();
    });
    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "r", PeerRole::Host)
        .with_device_credentials(mediaservo_link::DeviceCredential {
            device_id: "ms-bad".into(),
            device_secret: "wrong".into(),
        });
    let err = client.connect().await.unwrap_err();
    assert!(
        err.to_string().contains("4010")
            && err.to_string().contains("device authentication failed"),
        "must explicitly report device auth failure, got: {err}"
    );
    server.await.unwrap();
}

// ---- Phase B (B1): reconnect (exponential backoff + jitter) and disconnect notice ----

/// mock server: the first `refuse` connections get an instant TCP drop (WS handshake fails), then full auth + join;
/// exits after serving `total` connections in total (so accept never blocks forever once the client gives up).
async fn refuse_then_serve(
    refuse: usize,
    total: usize,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = tokio::spawn({
        let attempts = attempts.clone();
        async move {
            let mut conn = 0usize;
            while conn < total {
                let (stream, _) = listener.accept().await.unwrap();
                conn += 1;
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if conn <= refuse {
                    drop(stream); // refuse: instant TCP close -> connect_async errors
                    continue;
                }
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let psk_msg = ws.next().await.unwrap().unwrap();
                assert!(matches!(psk_msg, Message::Text(_)));
                let ack = SignalingMessage::Error { code: 0, message: String::new() };
                ws.send(Message::Text(serde_json::to_string(&ack).unwrap())).await.unwrap();
                let join_msg = ws.next().await.unwrap().unwrap();
                let join: SignalingMessage =
                    serde_json::from_str(join_msg.to_text().unwrap()).unwrap();
                let room_id = match join {
                    SignalingMessage::RoomJoin { room_id, .. } => room_id,
                    _ => panic!("expected RoomJoin"),
                };
                let joined = SignalingMessage::RoomJoined {
                    room_id,
                    peer_id: "peer-1".to_string(),
                    protocol: None,
                    server_version: None,
                    session_nonce: None,
                };
                ws.send(Message::Text(serde_json::to_string(&joined).unwrap())).await.unwrap();
                return; // this round is done, remaining connections are not served
            }
        }
    });
    (addr, server, attempts)
}

#[tokio::test]
async fn connect_with_retry_refuses_then_succeeds() {
    // refuse twice first (2 retry rounds), the 3rd connect succeeds -> session usable
    let (addr, server, attempts) = refuse_then_serve(2, 3).await;
    let client =
        SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "test-room", PeerRole::Host);
    let session = client
        .connect_with_retry(mediaservo_link::RetryConfig {
            max_retries: 3,
            base_delay: std::time::Duration::from_millis(50),
            max_delay: std::time::Duration::from_secs(1),
        })
        .await
        .expect("retry should succeed");
    assert_eq!(session.room_id(), "test-room");
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "must attempt 3 times (2 refused + 1 accepted)"
    );
    session.close().await.expect("close");
    server.await.unwrap();
}

#[tokio::test]
async fn connect_with_retry_exhausts_max_retries() {
    // always refuse -> errors after max_retries, each wait using exponential backoff
    let (addr, server, attempts) = refuse_then_serve(3, 3).await;
    let client =
        SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "test-room", PeerRole::Host);
    let started = std::time::Instant::now();
    let err = client
        .connect_with_retry(mediaservo_link::RetryConfig {
            max_retries: 2,
            base_delay: std::time::Duration::from_millis(50),
            max_delay: std::time::Duration::from_secs(1),
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("after 2 retries"), "must report the retry count, got: {err}");
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "must attempt 3 times (1 initial + 2 retries)"
    );
    // ±25% jitter: backoffs 50ms/100ms at their 75% floors sum to >= 100ms
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(100),
        "must have waited for backoff, elapsed={:?}",
        started.elapsed()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn on_disconnect_fires_when_server_closes() {
    // mock server: after the full handshake, waits for the client's ready message then sends Close
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap())).await.unwrap();
        let _join = ws.next().await.unwrap().unwrap();
        let joined = SignalingMessage::RoomJoined {
            room_id: "r".to_string(),
            peer_id: "peer-1".to_string(),
            protocol: None,
            server_version: None,
            session_nonce: None,
        };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap())).await.unwrap();
        let _ready = ws.next().await.unwrap().unwrap(); // wait for the client to be ready
        ws.close(None).await.unwrap();
    });

    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "r", PeerRole::Host);
    let session = client.connect().await.expect("connect");
    let (tx, mut rx) = tokio::sync::watch::channel(());
    session.on_disconnect(Box::new(move || {
        let _ = tx.send(());
    }));
    // tell the server to close
    session
        .send(SignalingMessage::Sdp {
            room_id: "r".to_string(),
            target: None,
            sdp: "v=0".to_string(),
        })
        .await
        .expect("send ready");
    tokio::time::timeout(std::time::Duration::from_secs(3), rx.changed())
        .await
        .expect("on_disconnect must fire when the server closes (3s timeout)")
        .expect("watch channel must not close");
    server.await.unwrap();
}

// -- device-enroll T7: public-key fingerprint signature chain (challenge -> reply -> terminal) + single-anchor cross-reverify --
// constants and expectations share the server devices.rs::sig_vector anchor (pinned in batch 2), copied verbatim - never recomputed.

const SIG_VECTOR_VK_B64: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";
const SIG_VECTOR_SIG_CAM0_B64: &str =
    "Gnz2kGCFH6igsOfv5QW0+8aRyu/lP5ytAa8fJA0CPYP3fIX5UsYr6uTjFjqOEEFBUB2scnDffIZ1WfP9O2ECCg==";

fn sig_nonce() -> Vec<u8> {
    (0x40u8..0x60).collect()
}

/// sig_vector identity: seed = bytes(0..=31), device_id = ms-0a1b2c3d4e5f.
fn ident_seed0() -> DeviceIdentity {
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    DeviceIdentity::new("ms-0a1b2c3d4e5f", ed25519_dalek::SigningKey::from_bytes(&seed))
}

#[test]
fn device_auth_sig_cross_anchor_matches_server_vector() {
    // single-anchor cross-reverify: this crate's signing function x the server's pinned expectation - same inputs must yield identical bytes.
    let ident = ident_seed0();
    assert_eq!(
        ident.pubkey_b64, SIG_VECTOR_VK_B64,
        "the public-key fingerprint must equal the server's pinned value"
    );
    assert_eq!(
        ident.sign_device_auth(&sig_nonce(), "vehicle_cam0"),
        SIG_VECTOR_SIG_CAM0_B64,
        "same seed/nonce/device_id/room must produce the same sig (byte contract §3 holds on both sides)"
    );
}

/// mock server: PSK ack -> assert Join is the pubkey form -> challenge(sig_vector nonce) ->
/// assert the reply passes verify_strict -> reply with the `final_msg` terminal state.
async fn spawn_pubkey_server(final_msg: SignalingMessage) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap())).await.unwrap();
        let join_msg = ws.next().await.unwrap().unwrap();
        let room_id = match serde_json::from_str::<SignalingMessage>(join_msg.to_text().unwrap())
            .unwrap()
        {
            SignalingMessage::RoomJoin {
                room_id, device_id, device_secret, device_pubkey, ..
            } => {
                assert_eq!(device_id.as_deref(), Some("ms-0a1b2c3d4e5f"));
                assert_eq!(device_secret, None, "a pubkey-form Join must not carry a secret");
                assert_eq!(
                    device_pubkey.as_deref(),
                    Some(SIG_VECTOR_VK_B64),
                    "Join must carry device_pubkey (= sig_vector vk)"
                );
                room_id
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        };
        let nonce_raw = sig_nonce();
        let ch = SignalingMessage::DeviceAuthChallenge {
            nonce: base64::engine::general_purpose::STANDARD.encode(&nonce_raw),
        };
        ws.send(Message::Text(serde_json::to_string(&ch).unwrap())).await.unwrap();
        // verify with the server's own discipline (verify_strict, §3) + same-anchor sig comparison
        let resp_msg = ws.next().await.unwrap().unwrap();
        match serde_json::from_str::<SignalingMessage>(resp_msg.to_text().unwrap()).unwrap() {
            SignalingMessage::DeviceAuthResponse { room_id: r, sig } => {
                assert_eq!(r, room_id, "the reply's room must echo the Join room");
                assert_eq!(
                    sig, SIG_VECTOR_SIG_CAM0_B64,
                    "same nonce/ids/room anchor must produce the same sig"
                );
                let vk_arr: [u8; 32] = base64::engine::general_purpose::STANDARD
                    .decode(SIG_VECTOR_VK_B64)
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
                msg.extend_from_slice(b"ms-0a1b2c3d4e5f");
                msg.extend_from_slice(room_id.as_bytes());
                ed25519_dalek::VerifyingKey::from_bytes(&vk_arr)
                    .unwrap()
                    .verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&sig_arr))
                    .expect("the host reply must pass verification");
            }
            other => panic!("expected DeviceAuthResponse, got {other:?}"),
        }
        ws.send(Message::Text(serde_json::to_string(&final_msg).unwrap())).await.unwrap();
    });
    addr
}

fn pubkey_client(addr: std::net::SocketAddr) -> SignalClient {
    SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "vehicle_cam0", PeerRole::Host)
        .with_device_identity(ident_seed0())
}

#[tokio::test]
async fn pubkey_join_challenge_answered_receives_joined() {
    // full chain passes: Join(pubkey) -> challenge -> reply -> RoomJoined -> session usable
    let addr = spawn_pubkey_server(SignalingMessage::RoomJoined {
        room_id: "vehicle_cam0".into(),
        peer_id: "peer-vk".into(),
        protocol: None,
        server_version: None,
        session_nonce: None,
    })
    .await;
    let session =
        pubkey_client(addr).connect().await.expect("the pubkey-form full chain must pass");
    assert_eq!(session.peer_id(), "peer-vk");
    session.close().await.expect("close");
}

#[tokio::test]
async fn pubkey_join_pending_maps_to_enroll_pending_error() {
    // manual mode, not approved: DeviceAuthPending -> typed EnrollPending (distinguishable in host logs)
    let addr = spawn_pubkey_server(SignalingMessage::DeviceAuthPending {
        device_id: "ms-0a1b2c3d4e5f".into(),
    })
    .await;
    let err = pubkey_client(addr).connect().await.expect_err("pending must report a typed error");
    assert!(
        matches!(&err, LinkError::EnrollPending { device_id } if device_id == "ms-0a1b2c3d4e5f"),
        "must be EnrollPending{{device_id}}, got: {err:?}"
    );
    assert!(
        err.to_string().contains("device enroll pending"),
        "Display contains the family string: {err}"
    );
}

#[tokio::test]
async fn pubkey_join_rejected_after_answer_surfaces_error() {
    // signature rejected (4010 uniform anti-enumeration message) -> error surfaces, no session
    let addr = spawn_pubkey_server(SignalingMessage::Error {
        code: 4010,
        message: "device authentication failed: invalid device credentials".into(),
    })
    .await;
    let err = pubkey_client(addr).connect().await.expect_err("rejection must error");
    assert!(
        err.to_string().contains("4010")
            && err.to_string().contains("device authentication failed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn pubkey_join_silence_hits_five_second_disconnect() {
    // §1 contract: server silence after a pubkey Join -> 5s-timeout disconnect error (start_paused advances instantly)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap())).await.unwrap();
        let _join = ws.next().await; // silence from here on
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    });
    let err = pubkey_client(addr).connect().await.expect_err("silence must time out");
    assert!(err.to_string().contains("no server response within 5s"), "got: {err}");
    server.abort();
}
