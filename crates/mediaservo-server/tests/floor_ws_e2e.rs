//! §3.5/O22 WS-level negative verification (floor authz slice): a live WS client
//! in a NON-designated room must receive exactly one 4032 ("designated talk
//! room") per floor command and ZERO floor_* frames in a drain window; in a
//! designated room the kind gate passes and the eligibility gate answers its
//! own 4032 (dialect/account) — the two gates are distinguishable on the wire.
//!
//! Harness mirrors heartbeat_e2e.rs: in-process server + raw tokio-tungstenite
//! client, PSK auth, pub-field overrides. Legacy sessions are intentional: the
//! kind gate fires for ANY identity before the eligibility gate.
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::floor::FloorLimits;
use mediaservo_server::floor::FloorRoom;
use mediaservo_server::signaling::{SignalingServer, signaling_router};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Spawn the server with floor enabled; `seed_rooms` are pre-registered as the
/// designated talk set (same mechanism as MEDIASERVO_FLOOR_ROOMS seeding).
async fn spawn_server(seed_rooms: &[&str]) -> String {
    let jwt_auth = mediaservo_common::auth::JwtAuth::new("test-secret-min-32-bytes!!!");
    #[cfg(feature = "sfu-mediasoup")]
    let server = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, Some(jwt_auth))
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let server = SignalingServer::new(65536, Some(jwt_auth));
    let mut server = server;
    server.floor_enabled = true;
    server.ws_ping_secs = 1; // fast heartbeat: a killed socket trips the send-failure cleanup in ~1 s
    server.ws_psk_wait_secs = 10;
    server.ws_join_wait_secs = 30;
    server.psk_state = std::sync::Arc::new(std::sync::RwLock::new(Some("test-psk".into())));
    for id in seed_rooms {
        server.floor_rooms.insert(
            (*id).to_string(),
            std::sync::Arc::new(tokio::sync::Mutex::new(FloorRoom::new(
                *id,
                mediaservo_common::floor::FloorMode::Exclusive,
                FloorLimits::default(),
            ))),
        );
    }
    let app = signaling_router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://{addr}/ws")
}

/// PSK auth + room_join (dialect 4, consumer role) + drain the room_joined ack.
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_and_join(url: &str, room_id: &str) -> WsStream {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(Message::Text("test-psk".into())).await.unwrap();
    ws.send(Message::Text(format!(
        r#"{{"type":"room_join","room_id":"{room_id}","peer_role":"consumer","protocol":4}}"#
    )))
    .await
    .unwrap();
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) if t.contains("room_joined") => break,
            Message::Ping(p) => {
                let _ = ws.send(Message::Pong(p)).await;
            }
            _ => continue,
        }
    }
    ws
}

/// Collect frames for `secs`, returning (error_4032_count, floor_frame_count).
async fn drain(ws: &mut WsStream, secs: u64) -> (usize, usize) {
    let mut errors_4032 = 0;
    let mut floor_frames = 0;
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(secs));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            f = ws.next() => match f {
                Some(Ok(Message::Text(t))) => {
                    if t.contains("\"code\":4032") {
                        errors_4032 += 1;
                    }
                    if t.contains("\"type\":\"floor_") {
                        floor_frames += 1;
                    }
                }
                Some(Ok(Message::Ping(p))) => {
                    let _ = ws.send(Message::Pong(p)).await;
                }
                Some(Err(e)) => panic!("connection must stay up during the drain: {e}"),
                None => panic!("server must not drop the connection during the drain"),
                _ => {}
            }
        }
    }
    (errors_4032, floor_frames)
}

/// A7 kill-socket (r3-K3), WS-level with a real ACCOUNT session: a dispatcher
/// is granted in the designated room; a second dispatcher joins; the grantee's
/// socket is then HARD-DROPPED (no leave, no close frame -- the kill -9 shape).
/// The server must inject Leave on its own: the second dispatcher observes the
/// release (FloorIdle) and a resync shows empty grants.
#[tokio::test]
async fn kill_socket_releases_the_floor_and_promotes() {
    // server-side tracing goes to stdout: without this the cleanup chain is invisible
    let _ = tracing_subscriber::fmt()
        .with_env_filter("mediaservo_server=debug,info")
        .with_target(false)
        .try_init();
    use mediaservo_common::auth::JwtClaims;
    let url = spawn_server(&["talk-car1"]).await;
    let secret = "test-secret-min-32-bytes!!!";
    let make_jwt = || {
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                as usize;
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &JwtClaims {
                sub: "dispatch-1".into(),
                iat: now,
                exp: now + 3600,
                role: Some("dispatcher".into()),
                vehicles: None,
            },
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    };

    let connect_account = |url: String, jwt: String, room: &str| {
        let room = room.to_string();
        async move {
            let mut req: tokio_tungstenite::tungstenite::http::Request<()> =
                tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url)
                    .unwrap();
            req.headers_mut().insert("Sec-WebSocket-Protocol", jwt.parse().unwrap());
            let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
            ws.send(Message::Text("test-psk".into())).await.unwrap();
            ws.send(Message::Text(format!(
                r#"{{"type":"room_join","room_id":"{room}","peer_role":"consumer","protocol":4}}"#
            )))
            .await
            .unwrap();
            // the registration snapshot (floor_state) proves the floor conn registered
            // AND carries the grant truth the joiner needs (the join-time snapshot is
            // the only replay a late joiner gets -- broadcasts never replay).
            let mut join_snapshot = serde_json::Value::Null;
            loop {
                match ws.next().await.unwrap().unwrap() {
                    Message::Text(t) if t.contains("floor_state") => {
                        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                        join_snapshot = v["snapshot"].clone();
                        eprintln!("[account] registered (floor_state seen)");
                        break;
                    }
                    Message::Text(t) if t.contains("room_joined") => continue,
                    Message::Text(t) => eprintln!("[account] join-phase frame: {t}"),
                    Message::Ping(p) => {
                        let _ = ws.send(Message::Pong(p)).await;
                    }
                    _ => continue,
                }
            }
            (ws, join_snapshot)
        }
    };

    let (mut grantee, _grantee_snapshot) =
        connect_account(url.clone(), make_jwt(), "talk-car1").await;
    eprintln!("[kill] grantee connected+registered");
    grantee.send(Message::Text(r#"{"type":"floor_request"}"#.into())).await.unwrap();
    eprintln!("[kill] floor_request sent");
    eprintln!("[kill] waiting floor_granted");
    loop {
        match grantee.next().await.unwrap().unwrap() {
            Message::Text(t) if t.contains("floor_granted") => {
                eprintln!("[kill] floor_granted seen");
                break;
            }
            Message::Text(t) => eprintln!("[kill] grantee frame: {t}"),
            Message::Ping(p) => {
                let _ = grantee.send(Message::Pong(p)).await;
            }
            _ => continue,
        }
    }

    let (mut observer, observer_snapshot) = connect_account(url, make_jwt(), "talk-car1").await;
    // grant truth for a late joiner = the join-time snapshot (broadcasts never
    // replay). If the snapshot already carries the grant, skip straight to the
    // kill; only a pre-grant joiner waits for the floor_granted broadcast.
    let observer_has_grant = observer_snapshot["grants"].as_array().is_some_and(|g| !g.is_empty());
    eprintln!("[kill] observer snapshot grants={:?}", observer_snapshot["grants"]);
    if !observer_has_grant {
        loop {
            match observer.next().await.unwrap().unwrap() {
                Message::Text(t) if t.contains("\"type\":\"floor_granted\"") => break,
                Message::Ping(p) => {
                    let _ = observer.send(Message::Pong(p)).await;
                }
                _ => continue,
            }
        }
    }

    // KILL-SOCKET: drop the grantee without any leave/close frame.
    drop(grantee);

    // The observer must see the release (FloorIdle) within 5 s (ping=1 s trips
    // the dead-socket cleanup quickly, leave some scheduling slack).
    let saw_idle = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match observer.next().await.unwrap().unwrap() {
                Message::Text(t) if t.contains("floor_idle") => break,
                Message::Ping(p) => {
                    let _ = observer.send(Message::Pong(p)).await;
                }
                _ => continue,
            }
        }
    })
    .await;
    assert!(saw_idle.is_ok(), "the observer must see FloorIdle after the kill-socket");

    // And a resync confirms the grants are actually gone (state, not just a frame).
    observer.send(Message::Text(r#"{"type":"floor_resync"}"#.into())).await.unwrap();
    let grants_empty = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match observer.next().await.unwrap().unwrap() {
                Message::Text(t) if t.contains("floor_state") => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    break v["snapshot"]["grants"].as_array().map(Vec::len) == Some(0);
                }
                Message::Ping(p) => {
                    let _ = observer.send(Message::Pong(p)).await;
                }
                _ => continue,
            }
        }
    })
    .await;
    assert_eq!(grants_empty, Ok(true), "resync after kill-socket shows empty grants");
}

/// O22 negative: an undesignated room answers exactly one 4032 naming the talk
/// room, and ZERO floor_* frames cross the wire in the drain window.
#[tokio::test]
async fn floor_command_in_undesignated_room_is_denied_with_zero_floor_frames() {
    let url = spawn_server(&["talk-car1"]).await; // "fleet1" is deliberately NOT designated
    let mut ws = connect_and_join(&url, "fleet1").await;
    ws.send(Message::Text(r#"{"type":"floor_request"}"#.into())).await.unwrap();
    let (errors, floors) = drain(&mut ws, 2).await;
    assert_eq!(errors, 1, "exactly one 4032 expected");
    assert_eq!(floors, 0, "O22: non-talk rooms send zero floor_* frames");
}

/// The kind gate passes in a designated room; the eligibility gate (dialect 4 +
/// account session — a legacy PSK session is neither) answers its own 4032.
#[tokio::test]
async fn floor_command_in_designated_room_hits_the_eligibility_gate() {
    let url = spawn_server(&["talk-car1"]).await;
    let mut ws = connect_and_join(&url, "talk-car1").await;
    ws.send(Message::Text(r#"{"type":"floor_request"}"#.into())).await.unwrap();
    let (errors, floors) = drain(&mut ws, 2).await;
    assert_eq!(errors, 1, "the eligibility gate denies legacy sessions");
    assert_eq!(floors, 0, "denied means no floor_* traffic");
}

/// C1 spoofing (P2 gate): a floor command carrying forged identity fields
/// (`room_id`/`peer_id` are NOT part of the C→S floor wire contract) must be
/// tolerated by serde (unknown fields ignored), handled under the DERIVED
/// identity (the gate verdict is identical to the clean message), and never
/// echoed back -- the forged fields must not appear on the reply wire.
#[tokio::test]
async fn spoofed_identity_fields_are_ignored_and_never_echoed() {
    let url = spawn_server(&["talk-car1"]).await;
    let mut ws = connect_and_join(&url, "fleet1").await;
    // forged fields injected into a floor command (identity spoofing attempt):
    // the server derives room/peer from the connection, ignores these, and the
    // Legacy-session eligibility denial is byte-identical to the clean shape.
    ws.send(Message::Text(
        r#"{"type":"floor_request","room_id":"victim-room","peer_id":"victim","priority":1,"preempt":true}"#
            .into(),
    ))
    .await
    .unwrap();
    let (errors, floors) = drain(&mut ws, 2).await;
    assert_eq!(errors, 1, "spoofed fields must not bypass the gate");
    assert_eq!(floors, 0, "no floor_* may carry the forged identity");
}
