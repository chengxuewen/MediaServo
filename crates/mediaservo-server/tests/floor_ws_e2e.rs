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
    #[cfg(feature = "sfu-mediasoup")]
    let server = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let server = SignalingServer::new(65536, None);
    let mut server = server;
    server.floor_enabled = true;
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
