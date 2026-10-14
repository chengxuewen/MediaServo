//! auth::login's TCP mock unit tests -- canned responses per state (ok / 401 / garbage / empty token).

use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use mediaservo_client::auth::{exchange, login};
use mediaservo_client::error::ClientError;

/// Start a single-reply mock HTTP server: read the request fully (Connection: close -> the client's half-close
/// lets read_to_end converge naturally) -> reply with canned bytes -> disconnect.
async fn canned_server(resp: Vec<u8>) -> u16 {
    canned_server_on(b"POST /api/auth/login HTTP/1.1\r\n".to_vec(), resp).await
}

/// Same mock, but asserts a given request-line prefix (the exchange surface reuses the same body-processing chain).
async fn canned_server_on(prefix: Vec<u8>, resp: Vec<u8>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // the client no longer write-half-closes (S2b: real hyper silently disconnects on a request with a leading half-close) --
        // the mock shares the real server's semantics: reply once the full header+body is read. On loopback
        // write_all lands in one go, a single read suffices.
        use tokio::io::AsyncReadExt;
        let mut req = Vec::new();
        let mut tmp = [0u8; 4096];
        let mut body_needed = false;
        loop {
            if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n")
                && !body_needed
            {
                let heads = String::from_utf8_lossy(&req[..pos]).to_string();
                let cl: usize = heads
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.trim()
                            .eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().ok())?
                    })
                    .unwrap_or(0);
                body_needed = true;
                let have = req.len() - pos - 4;
                if have >= cl {
                    break;
                }
            }
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
        }
        assert!(
            req.starts_with(prefix.as_slice()),
            "the request message's header shape mismatches: {req:?}"
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
    let out = login(&format!("http://127.0.0.1:{port}"), "op", "pw").await.unwrap();
    assert_eq!(out.jwt, "jwt-abc");
    assert_eq!(out.username, "op");
    assert_eq!(out.role, "operator");
    assert_eq!(out.expires_in_secs, 3600);
}

#[tokio::test]
async fn login_401_maps_invalid_credentials() {
    let port = canned_server(response("401 Unauthorized", r#"{"error":"no such user"}"#)).await;
    let e = login(&format!("http://127.0.0.1:{port}"), "op", "bad").await.unwrap_err();
    assert!(matches!(e, ClientError::InvalidCredentials), "got {e:?}");
}

#[tokio::test]
async fn login_garbage_maps_malformed() {
    let port = canned_server(b"totally-not-http".to_vec()).await;
    let e = login(&format!("http://127.0.0.1:{port}"), "op", "pw").await.unwrap_err();
    assert!(matches!(e, ClientError::MalformedResponse(_)), "got {e:?}");
}

#[tokio::test]
async fn login_200_empty_token_rejected() {
    let port = canned_server(response("200 OK", r#"{"token":""}"#)).await;
    let e = login(&format!("http://127.0.0.1:{port}"), "op", "pw").await.unwrap_err();
    assert!(matches!(e, ClientError::MalformedResponse(_)), "got {e:?}");
}

#[tokio::test]
async fn login_https_scheme_rejected_v1() {
    let e = login("https://127.0.0.1:1", "op", "pw").await.unwrap_err();
    assert!(matches!(e, ClientError::UnsupportedScheme(_)), "got {e:?}");
}

// -- T2: the exchange surface (the same body chain, a different path/request body) --

#[tokio::test]
async fn exchange_ok_returns_outcome_with_key_id_as_username() {
    let port = canned_server_on(
        b"POST /api/auth/exchange HTTP/1.1\r\n".to_vec(),
        response(
            "200 OK",
            r#"{"token":"jwt.k.x","username":"ci-1","role":"viewer","expires_in_secs":43200}"#,
        ),
    )
    .await;
    let out = exchange(&format!("http://127.0.0.1:{port}"), "ci-1", "s3cret").await.unwrap();
    assert_eq!(out.jwt, "jwt.k.x");
    assert_eq!(
        out.username, "ci-1",
        "exchange's username slot = key_id (the same shape as the server's LoginResponse)"
    );
    assert_eq!(out.role, "viewer");
    assert_eq!(out.expires_in_secs, 43200);
}

#[tokio::test]
async fn exchange_401_is_terminal_invalid_credentials() {
    let port = canned_server_on(
        b"POST /api/auth/exchange HTTP/1.1\r\n".to_vec(),
        response(
            "401 Unauthorized",
            r#"{"error":"api key authentication failed: invalid credentials"}"#,
        ),
    )
    .await;
    let err = exchange(&format!("http://127.0.0.1:{port}"), "ci-1", "typo").await.unwrap_err();
    assert!(matches!(err, ClientError::InvalidCredentials), "{err:?}");
    // the D273 red-card family: bad credentials must never retry (the client-side mirror of the T0 semantics).
    assert!(!err.is_retryable());
}
