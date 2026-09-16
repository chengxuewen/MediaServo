//! list_rooms 的 TCP mock 测试——GET 形断言 + 200/401/垃圾三态。
//! C21 纪律：零 server 依赖，canned 字节与 server rooms.rs 集成测同 wire 形。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use mediaservo_client::auth::{list_rooms, RoomInfo};
use mediaservo_client::error::ClientError;

fn response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// GET mock：无 body——读满头（\r\n\r\n）即回应；断言 GET 行 + Bearer 头
/// （auth_unit.rs canned_server 同法，方向参数化）。
async fn canned_get_server(resp: Vec<u8>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
        }
        let s = String::from_utf8_lossy(&req).to_string();
        assert!(s.starts_with("GET /api/rooms HTTP/1.1\r\n"), "req: {s}");
        assert!(s.contains("Authorization: Bearer jwt-abc\r\n"), "req: {s}");
        sock.write_all(&resp).await.ok();
        drop(sock);
    });
    port
}

#[tokio::test]
async fn list_rooms_ok_returns_entries() {
    let port = canned_get_server(response(
        "200 OK",
        r#"{"rooms":[{"room_id":"vehicle_t1","kind":"video"},{"room_id":"audio-c1","kind":"audio"}]}"#,
    ))
    .await;
    let rooms = list_rooms(&format!("http://127.0.0.1:{port}"), "jwt-abc").await.unwrap();
    assert_eq!(
        rooms,
        vec![
            RoomInfo { room_id: "vehicle_t1".into(), kind: "video".into() },
            RoomInfo { room_id: "audio-c1".into(), kind: "audio".into() },
        ]
    );
}

#[tokio::test]
async fn list_rooms_empty_list_is_ok() {
    let port = canned_get_server(response("200 OK", r#"{"rooms":[]}"#)).await;
    assert!(list_rooms(&format!("http://127.0.0.1:{port}"), "jwt-abc").await.unwrap().is_empty());
}

#[tokio::test]
async fn list_rooms_401_maps_rest_rejected() {
    let port = canned_get_server(response("401 Unauthorized", r#"{"error":"invalid token"}"#)).await;
    // token 串走 mock 断言（Bearer jwt-abc），401 语义=server 判无效——客户端不辨原因。
    let e = list_rooms(&format!("http://127.0.0.1:{port}"), "jwt-abc").await.unwrap_err();
    assert!(matches!(e, ClientError::RestRejected { code: 401, .. }), "got {e:?}");
}

#[tokio::test]
async fn list_rooms_garbage_maps_malformed() {
    let port = canned_get_server(b"totally-not-http".to_vec()).await;
    let e = list_rooms(&format!("http://127.0.0.1:{port}"), "jwt-abc").await.unwrap_err();
    assert!(matches!(e, ClientError::MalformedResponse(_)), "got {e:?}");
}
