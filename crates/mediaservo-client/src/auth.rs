//! 账号登录——POST {base}/api/auth/login（手写 HTTP/1.1，零新增外部依赖）。
//!
//! **v1 安全约束**：仅明文 http；TLS 面归 S4+ 裁决（避免 Cargo 生态 TLS 栈冲突面）。
//! **设计取舍**：hyper 会引入 TLS+http 传递依赖，当前手写 TCP 足以覆盖 v1 面。

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::error::ClientError;

/// 连接超时（手写 TCP——link 的 WS 面由 SignalClient 控制，此为 REST 面）。
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// 读写超时。
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// login path——只用于测试断言；生产直接拼入 `http_base`。
pub(crate) const LOGIN_PATH: &str = "/api/auth/login";

/// rooms 发现 path（GET /api/rooms——server 侧 p3 W2-A 端点）。
pub(crate) const ROOMS_PATH: &str = "/api/rooms";

/// 登录成功返回（精简 shape——server `admin.rs:163` LoginResponse）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOutcome {
    pub jwt: String,
    pub username: String,
    pub role: String,
    pub expires_in_secs: u64,
}

// ───────── wire 形状（私有，测试断言走 LoginOutcome）─────────

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct LoginWire {
    token: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    expires_in_secs: u64,
}

/// 消费面发现的房间（server wire = `{room_id, kind}` 二字段，serde 钉同源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomInfo {
    pub room_id: String,
    /// "video" | "audio"（server 按房间名前缀派生，未知前缀透传——客户端不判型）。
    pub kind: String,
}

#[derive(Deserialize)]
struct RoomsWire {
    rooms: Vec<RoomInfo>,
}

#[derive(Deserialize)]
struct ErrorWire {
    #[serde(default)]
    error: Option<String>,
}

// ───────── 纯函数（单元测试）─────────

/// 构造 HTTP/1.1 POST 请求报文。
pub(crate) fn build_request(host: &str, port: u16, body: &[u8]) -> Vec<u8> {
    let mut req = format!(
        "POST {LOGIN_PATH} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    )
    .into_bytes();
    req.extend_from_slice(body);
    req
}

/// 构造 HTTP/1.1 GET 请求报文（Bearer JWT——login 签发的同一凭证）。
pub(crate) fn build_get_request(host: &str, port: u16, path: &str, jwt: &str) -> Vec<u8> {
    format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Authorization: Bearer {jwt}\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\
         \r\n"
    )
    .into_bytes()
}

/// 解析 `http_base`（仅 http scheme）。
fn parse_http_base(url: &str) -> Result<(String, u16), ClientError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| ClientError::UnsupportedScheme(url.to_string()))?;
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => {
            let path_end = p.find('/').unwrap_or(p.len());
            (
                h.to_string(),
                p[..path_end].parse::<u16>().map_err(|e| {
                    ClientError::MalformedResponse(format!("bad port in {url}: {e}"))
                })?,
            )
        }
        None => (rest.to_string(), 80u16),
    };
    if host.is_empty() {
        return Err(ClientError::MalformedResponse(format!("empty host in {url}")));
    }
    Ok((host, port))
}

/// 解析 HTTP 响应：`("HTTP/1.1 200 OK\r\n...", body_bytes)` → (status, body)。
pub(crate) fn parse_response(raw: &[u8]) -> Result<(u16, &[u8]), ClientError> {
    let header_end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| {
        ClientError::MalformedResponse(format!(
            "no header-body separator ({} bytes: {:?})",
            raw.len(),
            String::from_utf8_lossy(&raw[..raw.len().min(80)]),
        ))
    })?;
    let (status_line, rest) = raw[..header_end]
        .split_at(raw[..header_end].iter().position(|&b| b == b'\r').unwrap_or(header_end));
    let _ = rest; // only need status line
    let code = parse_status_line(status_line)?;
    let body = &raw[header_end + 4..];
    Ok((code, body))
}

fn parse_status_line(line: &[u8]) -> Result<u16, ClientError> {
    // "HTTP/1.1 200 OK"
    let first_space = line
        .iter()
        .position(|&b| b == b' ')
        .ok_or_else(|| ClientError::MalformedResponse("missing space in status line".into()))?;
    let rest = &line[first_space + 1..];
    let code_end =
        rest.iter().position(|&b| b == b' ' || b == b'\r' || b == b'\n').unwrap_or(rest.len());
    let code_str = std::str::from_utf8(&rest[..code_end])
        .map_err(|_| ClientError::MalformedResponse("non-ASCII status code".into()))?;
    code_str
        .parse::<u16>()
        .map_err(|e| ClientError::MalformedResponse(format!("bad status code {code_str}: {e}")))
}

// ───────── async 入口─────────

/// 账号密码登录，返回 [`LoginOutcome`]（v1 scope = 基础 token + 角色）。
pub async fn login(
    http_base: &str,
    username: &str,
    password: &str,
) -> Result<LoginOutcome, ClientError> {
    let (host, port) = parse_http_base(http_base)?;
    let body = serde_json::to_vec(&serde_json::json!({
        "username": username,
        "password": password,
    }))
    .map_err(|e| ClientError::MalformedResponse(format!("body serialize: {e}")))?;
    let req = build_request(&host, port, &body);
    let (status, body_bytes) = request_raw(&host, port, &req, "http login").await?;

    match status {
        200 => {
            let wire: LoginWire = serde_json::from_slice(&body_bytes).map_err(|e| {
                tracing::warn!(body_len = body_bytes.len(), error = %e, "login 200 body parse failed");
                ClientError::MalformedResponse(format!("login body: {e}"))
            })?;
            if wire.token.is_empty() {
                tracing::warn!("login 200 returned empty token");
                return Err(ClientError::MalformedResponse("empty token".into()));
            }
            Ok(LoginOutcome {
                jwt: wire.token,
                username: wire.username,
                role: wire.role,
                expires_in_secs: wire.expires_in_secs,
            })
        }
        401 => {
            let msg = serde_json::from_slice::<ErrorWire>(&body_bytes)
                .ok()
                .and_then(|w| w.error)
                .unwrap_or_else(|| "invalid credentials".into());
            tracing::info!(user = %username, msg = %msg, "login 401");
            Err(ClientError::InvalidCredentials)
        }
        429 => Err(ClientError::Login("rate limited (429)".into())),
        other => {
            let msg = serde_json::from_slice::<ErrorWire>(&body_bytes)
                .ok()
                .and_then(|w| w.error)
                .unwrap_or_default();
            tracing::warn!(status = other, msg = %msg, "login failed");
            Err(ClientError::Login(format!("HTTP {other}: {msg}")))
        }
    }
}

/// 单请求单响应（Connection: close → read_to_end 以 EOF 终止）。
/// login 与 list_rooms 共用——同一手写 TCP 面，零第二实现。
async fn request_raw(
    host: &str,
    port: u16,
    req: &[u8],
    what: &'static str,
) -> Result<(u16, Vec<u8>), ClientError> {
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| ClientError::Timeout { what: "tcp connect" })?
        .map_err(|e| {
            tracing::warn!(host = %host, port, error = %e, "tcp connect failed");
            ClientError::Io(e)
        })?;
    stream.set_nodelay(true).ok();

    timeout(IO_TIMEOUT, stream.write_all(req))
        .await
        .map_err(|_| ClientError::Timeout { what })?
        .map_err(|e| {
            tracing::warn!(error = %e, "http write failed");
            ClientError::Io(e)
        })?;
    // 不做写半关（S2b 实锤：hyper 对"body 未读全即遇 half-close"的请求
    // 直接静默断连 = 0 字节）。响应由 Connection: close 保证服务端发完即关。

    let mut raw = Vec::with_capacity(4096);
    timeout(IO_TIMEOUT, stream.read_to_end(&mut raw))
        .await
        .map_err(|_| ClientError::Timeout { what })?
        .map_err(ClientError::Io)?;

    parse_response(&raw).map(|(code, body)| (code, body.to_vec()))
}

/// 消费面房间发现（p3 W2-B）：`GET {base}/api/rooms`，账号 JWT（login 签发）。
///
/// 权限矩阵以 server 为准（admin/dispatcher 全量，viewer/operator allowlist），
/// 客户端不做二次过滤。非 2xx（token 失效/过期/角色不符）→ [`ClientError::RestRejected`]
/// ——不是 InvalidCredentials：凭证曾有效，此处是授权面拒绝。
pub async fn list_rooms(http_base: &str, jwt: &str) -> Result<Vec<RoomInfo>, ClientError> {
    let (host, port) = parse_http_base(http_base)?;
    let req = build_get_request(&host, port, ROOMS_PATH, jwt);
    let (status, body) = request_raw(&host, port, &req, "http list_rooms").await?;
    match status {
        200 => serde_json::from_slice::<RoomsWire>(&body).map(|w| w.rooms).map_err(|e| {
            tracing::warn!(body_len = body.len(), error = %e, "list_rooms 200 body parse failed");
            ClientError::MalformedResponse(format!("rooms body: {e}"))
        }),
        other => {
            let msg = String::from_utf8_lossy(&body).chars().take(120).collect::<String>();
            tracing::warn!(status = other, body = %msg, "list_rooms failed");
            Err(ClientError::RestRejected { code: other, message: msg })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_content_length_and_method() {
        let req = build_request("host", 9800, b"{\"x\":1}");
        let s = String::from_utf8(req).unwrap();
        println!("REQBYTES {:?}", s);
        assert!(s.starts_with("POST /api/auth/login HTTP/1.1\r\n"));
        assert!(s.contains("Content-Length: 7\r\n"));
        assert!(s.contains("Connection: close\r\n"));
        assert!(s.ends_with("{\"x\":1}"));
    }

    #[test]
    fn parse_status_line_200() {
        assert_eq!(parse_status_line(b"HTTP/1.1 200 OK").unwrap(), 200);
        assert_eq!(parse_status_line(b"HTTP/1.1 401 Unauthorized").unwrap(), 401);
        assert_eq!(parse_status_line(b"HTTP/1.1 429 ").unwrap(), 429);
    }

    #[test]
    fn parse_status_line_bad() {
        assert!(parse_status_line(b"NOPE").is_err());
        assert!(parse_status_line(b"HTTP/1.1 XXX").is_err());
    }

    #[test]
    fn parse_http_base_rejects_non_http() {
        assert!(matches!(parse_http_base("https://host"), Err(ClientError::UnsupportedScheme(_))));
    }

    #[test]
    fn parse_http_base_extracts_port() {
        assert_eq!(parse_http_base("http://10.0.0.2:9800").unwrap(), ("10.0.0.2".into(), 9800));
    }

    #[test]
    fn parse_http_base_default_port() {
        assert_eq!(parse_http_base("http://host").unwrap(), ("host".into(), 80));
    }

    #[test]
    fn parse_response_roundtrip() {
        let body = br#"{"token":"abc"}"#;
        let mut resp = b"HTTP/1.1 200 OK\r\nContent-Length: ".to_vec();
        resp.extend_from_slice(body.len().to_string().as_bytes());
        resp.extend_from_slice(b"\r\n\r\n");
        resp.extend_from_slice(body);
        let (code, body_bytes) = parse_response(&resp).unwrap();
        assert_eq!(code, 200);
        assert_eq!(body_bytes, body);
    }

    #[test]
    fn build_get_request_shape() {
        let req = String::from_utf8(build_get_request("h", 9800, "/api/rooms", "jwt-1")).unwrap();
        assert!(req.starts_with("GET /api/rooms HTTP/1.1\r\n"));
        assert!(req.contains("Authorization: Bearer jwt-1\r\n"));
        assert!(req.contains("Connection: close\r\n"));
        assert!(!req.contains("Content-Length"));
        assert!(req.ends_with("\r\n\r\n"));
    }

    #[test]
    fn rooms_wire_parses_server_shape() {
        // server wire 钉（rooms.rs 集成测同源形）：恰好二字段，多字段忽略不炸。
        let w: RoomsWire = serde_json::from_str(
            r#"{"rooms":[{"room_id":"vehicle_t1","kind":"video"},{"room_id":"audio-c1","kind":"audio"}]}"#,
        )
        .unwrap();
        assert_eq!(
            w.rooms,
            vec![
                RoomInfo { room_id: "vehicle_t1".into(), kind: "video".into() },
                RoomInfo { room_id: "audio-c1".into(), kind: "audio".into() },
            ]
        );
    }

    #[test]
    fn parse_response_missing_separator() {
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nNoSeparator").is_err());
    }
}
