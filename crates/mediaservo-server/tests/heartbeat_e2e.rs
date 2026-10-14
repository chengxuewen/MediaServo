//! a1 heartbeat / pre-auth timeout (S0.5-B2) integration verification: an in-process server + a raw WS client.
//! All parameters go through SignalingServer pub-field overrides (1s scale); the budget-decision matrix is pinned in the signaling.rs unit tests.
use futures_util::{SinkExt, StreamExt};
use mediaservo_server::signaling::{SignalingServer, signaling_router};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_server(ping_secs: u64, psk_wait: u64, join_wait: u64) -> String {
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
    server.ws_ping_secs = ping_secs;
    server.ws_pong_miss = 2;
    server.ws_psk_wait_secs = psk_wait;
    server.ws_join_wait_secs = join_wait;
    server.psk_state = std::sync::Arc::new(std::sync::RwLock::new(Some("test-psk".into())));
    let app = signaling_router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://{addr}/ws")
}

/// 1. heartbeat positive: with ping=1s the client authenticates and keeps reading; within 3s it should see >=2 Message::Pings
/// and the connection stays alive (tokio-tungstenite's auto-pong maintains liveness -- the browser likewise, zero changes).
#[tokio::test]
async fn server_pings_and_alive_connection_survives() {
    let url = spawn_server(1, 10, 30).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::Text("test-psk".into())).await.unwrap();
    // the heartbeat starts only after the session is established (zero control frames in the handshake window -- protecting the device-enroll state machine's read order):
    // auth ack first -> room_join -> joined, then open the counting window.
    let mut joined_ok = false;
    let mut pings = 0;
    // read the ack (code:0)
    while !joined_ok {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) if t.contains("\"code\":0") => {
                ws.send(Message::Text(
                    r#"{"type":"room_join","room_id":"hb-test","peer_role":"consumer","protocol":3}"#.into(),
                ))
                .await
                .unwrap();
            }
            Message::Text(t) if t.contains("room_joined") => joined_ok = true,
            Message::Ping(p) => {
                let _ = ws.send(Message::Pong(p)).await;
            }
            _ => {}
        }
    }
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(3));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            f = ws.next() => match f {
                Some(Ok(Message::Ping(_))) => pings += 1,
                Some(Ok(Message::Text(t))) if t.contains("\"code\":0") => {}
                Some(Err(e)) => panic!("the connection must not be interrupted: {e}"),
                None => panic!("the server must not drop a connection that keeps answering pongs"),
                _ => {}
            }
        }
    }
    assert!(pings >= 2, "within 3s at least 2 heartbeat pings should be seen (got {pings})");
}

/// 2. pre-auth bare-wait hardening: connecting without sending PSK -> the psk_wait=1s timeout disconnects (a terminal read within 3s).
#[tokio::test]
async fn preauth_psk_wait_times_out() {
    let url = spawn_server(0, 1, 30).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let r = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => return true,
                Some(Ok(_)) => continue, // other frames are not terminal
            }
        }
    })
    .await;
    assert!(
        matches!(r, Ok(true)),
        "the PSK timeout must disconnect on time (Close/EOF/Err received)"
    );
}

/// 3. authenticated-but-not-joined timeout: silence after the auth ack -> the join_wait=1s disconnect.
#[tokio::test]
async fn authed_without_join_times_out() {
    let url = spawn_server(0, 10, 1).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::Text("test-psk".into())).await.unwrap();
    // read the auth ack
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) if t.contains("\"code\":0") => break,
            Message::Ping(p) => {
                let _ = ws.send(Message::Pong(p)).await;
            }
            _ => continue,
        }
    }
    let r = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => return true,
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(matches!(r, Ok(true)), "authenticated-but-not-joined must time out and disconnect");
}
