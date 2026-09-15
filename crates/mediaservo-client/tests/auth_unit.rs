//! auth::login 的 TCP mock 单测——canned 响应各态（ok / 401 / 垃圾 / 空 token）。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use mediaservo_client::auth::login;
use mediaservo_client::error::ClientError;

/// 起一个单次应答的 mock HTTP 服务：读完请求（Connection: close → 客户端半关
/// 后 read_to_end 自然收敛）→ 回 canned 字节 → 断开。
async fn canned_server(resp: Vec<u8>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        sock.read_to_end(&mut req).await.ok();
        assert!(
            req.starts_with(b"POST /api/auth/login HTTP/1.1\r\n"),
            "请求报文头部形不符: {req:?}"
        );
        sock.write_all(&resp).await.ok();
        drop(sock);
    });
    port
}

fn response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[tokio::test]
async fn login_ok_returns_outcome() {
    let port = canned_server(response(
        "200 OK",
        r#"{"token":"jwt-abc","username":"op","role":"operator","expires_in_secs":3600}"#,
    ))
    .await;
    let out = login(&format!("http://127.0.0.1:{port}"), "op", "pw")
        .await
        .unwrap();
    assert_eq!(out.jwt, "jwt-abc");
    assert_eq!(out.username, "op");
    assert_eq!(out.role, "operator");
    assert_eq!(out.expires_in_secs, 3600);
}

#[tokio::test]
async fn login_401_maps_invalid_credentials() {
    let port = canned_server(response("401 Unauthorized", r#"{"error":"no such user"}"#)).await;
    let e = login(&format!("http://127.0.0.1:{port}"), "op", "bad")
        .await
        .unwrap_err();
    assert!(matches!(e, ClientError::InvalidCredentials), "got {e:?}");
}

#[tokio::test]
async fn login_garbage_maps_malformed() {
    let port = canned_server(b"totally-not-http".to_vec()).await;
    let e = login(&format!("http://127.0.0.1:{port}"), "op", "pw")
        .await
        .unwrap_err();
    assert!(matches!(e, ClientError::MalformedResponse(_)), "got {e:?}");
}

#[tokio::test]
async fn login_200_empty_token_rejected() {
    let port = canned_server(response("200 OK", r#"{"token":""}"#)).await;
    let e = login(&format!("http://127.0.0.1:{port}"), "op", "pw")
        .await
        .unwrap_err();
    assert!(matches!(e, ClientError::MalformedResponse(_)), "got {e:?}");
}

#[tokio::test]
async fn login_https_scheme_rejected_v1() {
    let e = login("https://127.0.0.1:1", "op", "pw").await.unwrap_err();
    assert!(matches!(e, ClientError::UnsupportedScheme(_)), "got {e:?}");
}
