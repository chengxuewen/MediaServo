//! a1 心跳/pre-auth 超时（S0.5-B2）集成验证：进程内 server + 裸 WS 客户端。
//! 参数全走 SignalingServer pub 字段覆写（1s 级），预算判定矩阵在 signaling.rs 单测钉。
use mediaservo_server::signaling::{signaling_router, SignalingServer};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use futures_util::{SinkExt, StreamExt};

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

/// ① 心跳正向：ping=1s 下 client 认证并持续读，3s 内应收到 ≥2 个 Message::Ping
/// 且连接存活（tokio-tungstenite 自动 pong 维持活性——browser 同理零改动）。
#[tokio::test]
async fn server_pings_and_alive_connection_survives() {
    let url = spawn_server(1, 10, 30).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::Text("test-psk".into())).await.unwrap();
    // 心跳仅在会话建立后启动（握手窗口零控制帧——device-enroll 状态机读序保护）：
    // 先 auth ack → room_join → joined，再开计数窗。
    let mut joined_ok = false;
    let mut pings = 0;
    // 读满 ack（code:0）
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
            Message::Ping(p) => { let _ = ws.send(Message::Pong(p)).await; }
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
                Some(Err(e)) => panic!("连接不应被打断: {e}"),
                None => panic!("server 不应在存活 pong 下断链"),
                _ => {}
            }
        }
    }
    assert!(pings >= 2, "3s 内应见 ≥2 心跳 ping（实得 {pings}）");
}

/// ② pre-auth 裸等加固：连接后不发 PSK → psk_wait=1s 超时断链（3s 内读到终结）。
#[tokio::test]
async fn preauth_psk_wait_times_out() {
    let url = spawn_server(0, 1, 30).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let r = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => return true,
                Some(Ok(_)) => continue, // 其他帧不算终结
            }
        }
    })
    .await;
    assert!(matches!(r, Ok(true)), "PSK 超时必须按时断链（收到 Close/EOF/Err）");
}

/// ③ 已认证未 join 超时：auth ack 后沉默 → join_wait=1s 断链。
#[tokio::test]
async fn authed_without_join_times_out() {
    let url = spawn_server(0, 10, 1).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::Text("test-psk".into())).await.unwrap();
    // 读 auth ack
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) if t.contains("\"code\":0") => break,
            Message::Ping(p) => { let _ = ws.send(Message::Pong(p)).await; }
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
    assert!(matches!(r, Ok(true)), "已认证未 join 必须超时断链");
}
