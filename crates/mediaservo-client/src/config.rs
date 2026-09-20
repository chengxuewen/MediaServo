//! ClientConfig — 所有端点/凭证由调用方传入（S2 纪律：无默认 localhost/9800/psk）。
//!
//! URL 派生纯函数 + SFU peer key 镜像 field 语义。

use crate::error::ClientError;
use mediaservo_common::protocol::PeerRole;

/// client v2 配置——构造即用，无隐式默认值。
#[derive(Clone)]
pub struct ClientConfig {
    /// 信令 WS 地址（如 `ws://10.0.0.2:9800/ws`）——直传 link SignalClient。
    pub signaling_url: String,
    /// server 房间 ID。
    pub room_id: String,
    /// 可选 PSK 直传（server 未配时省略；空 ≠ 猜测默认值）。
    pub psk: Option<String>,
    /// 可选账号 JWT（登录输出；v1 WS 透传仅 link 手工注入）。
    pub jwt: Option<String>,
    /// 入房角色（cockpit 侧 = Remote 或 Consumer）。
    pub role: PeerRole,
    /// S4/T3.5：e-stop HMAC 预共享密钥（与车端 `MEDIASERVO_CONTROL_HMAC_KEY` 同值）。
    /// None = 急停不带 sig（车端未配置 key 时照常执行；车端配置后 estop 会被拒）。
    pub hmac_key: Option<String>,
}

// jwt 在 Debug 中脱敏——凭证永不入日志。
impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("signaling_url", &self.signaling_url)
            .field("room_id", &self.room_id)
            .field("psk", &self.psk.as_ref().map(|_| "[redacted]"))
            .field("jwt", &self.jwt.as_ref().map(|_| "[redacted]"))
            .field("role", &self.role)
            .finish()
    }
}

/// SFU peer key 派生（镜像 `field::peer_id(role)` 语义，I2 同形状）。
///
/// SFU 房间 peer ID 是"谁在说话"语义标签，非身份标识。
/// identity.json/device_id 才是身份。peer ID 重叠无实际影响（SFU per-conn）。
#[must_use]
pub fn sfu_peer_key(role: &PeerRole) -> &'static str {
    match role {
        PeerRole::Host => "host",
        PeerRole::Remote => "remote",
        PeerRole::Consumer => "consumer",
    }
}

/// 从信令 URL 派生 HTTP 基地址（v1 login 面）。
///
/// `"ws://10.0.0.2:9800/ws"` → `"http://10.0.0.2:9800"`
/// `"wss://host/ws"` → `"https://host"`（login 层面后续 Reject scheme）。
pub fn http_base_from_signaling(url: &str) -> Result<String, ClientError> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| {
        ClientError::MalformedResponse(format!("no scheme in signaling URL: {url}"))
    })?;
    let host_port = rest.split('/').next().unwrap_or(rest);
    if host_port.is_empty() {
        return Err(ClientError::MalformedResponse("empty host in signaling URL".into()));
    }
    let http_scheme = match scheme {
        "ws" => "http",
        "wss" => "https",
        other => return Err(ClientError::UnsupportedScheme(other.to_string())),
    };
    Ok(format!("{http_scheme}://{host_port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfu_peer_key_covers_all_roles() {
        assert_eq!(sfu_peer_key(&PeerRole::Host), "host");
        assert_eq!(sfu_peer_key(&PeerRole::Remote), "remote");
        assert_eq!(sfu_peer_key(&PeerRole::Consumer), "consumer");
    }

    #[test]
    fn http_base_ws_to_http() {
        assert_eq!(
            http_base_from_signaling("ws://10.0.0.2:9800/ws").unwrap(),
            "http://10.0.0.2:9800"
        );
    }

    #[test]
    fn http_base_wss_to_https() {
        assert_eq!(
            http_base_from_signaling("wss://host.example/ws").unwrap(),
            "https://host.example"
        );
    }

    #[test]
    fn http_base_no_path() {
        assert_eq!(http_base_from_signaling("ws://host:9800").unwrap(), "http://host:9800");
    }

    #[test]
    fn http_base_no_scheme_error() {
        assert!(matches!(
            http_base_from_signaling("10.0.0.2:9800/ws"),
            Err(ClientError::MalformedResponse(_))
        ));
    }
}
