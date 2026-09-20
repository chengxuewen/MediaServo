//! client SDK 错误面（thiserror——库姿态；C15：所有 Err 分支带类型与上下文）。

use mediaservo_link::LinkError;

/// client v2 统一错误。
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// 登录请求/解析失败（网络、状态码非 401、响应畸形）。
    #[error("login failed: {0}")]
    Login(String),
    /// 401 —— 用户名或口令错（server 防枚举措辞，细节仅本侧 WARN）。
    #[error("invalid credentials")]
    InvalidCredentials,
    /// REST 发现面（GET /api/rooms）非 2xx——token 失效/过期/授权不符（p3 W2-B）。
    #[error("REST rejected [{code}]: {message}")]
    RestRejected { code: u16, message: String },
    /// 连接级认证被拒（4003 PSK / 4010 设备 / 4011 role 非法）。
    #[error("auth rejected [{code}]: {message}")]
    AuthRejected { code: u16, message: String },
    /// server 拒绝方言版本（4101，S0）——不静默降级，终态。
    #[error("protocol version unsupported (4101): {0}")]
    ProtocolUnsupported(String),
    /// v1 登录面仅支持明文 http（TLS 依赖引入归 S4+ 裁决）。
    #[error("unsupported scheme \"{0}\" (login v1 = plain http only)")]
    UnsupportedScheme(String),
    /// 响应无法解析（HTTP 状态行/JSON）。
    #[error("malformed response: {0}")]
    MalformedResponse(String),
    /// 信令层错误（link 透传：WS 连接/认证/收发/入房）。
    #[error("signal error: {0}")]
    Signal(#[from] LinkError),
    /// SFU 域 server 错误（非终态码通用承载）。
    #[error("server error [{code}]: {message}")]
    Server { code: u16, message: String },
    /// 控制 DC 建立被拒（4012——role 无 can_control 或方言不足，终态）。
    #[error("control denied (4012): {0}")]
    ControlDenied(String),
    /// I5 能力门前置判定：本地谈成方言低于控制域要求（未发出任何请求即拒）。
    #[error("negotiated protocol {got} < required {need} for control domain")]
    ProtocolTooLow { need: u32, got: u32 },
    /// WebRTC 本地错误（PC 协商/DC 写）。
    #[error("webrtc error: {0}")]
    WebRtc(String),
    /// IO（TcpStream 读写等）。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// 有界等待超时（操作名标注，便于归因）。
    #[error("timed out waiting for {what}")]
    Timeout { what: &'static str },
    /// 状态机违例（未知通道、会话已关闭等）。
    #[error("invalid state: {0}")]
    InvalidState(String),
}

/// wire `Error{code,message}` → 终态分类（D273 terminal 族在 SDK 侧的落点）。
///
/// 4012=控制拒、4101=方言拒单独成 typed；其余承载为 [`ClientError::Server`]。
#[must_use]
pub fn from_wire_error(code: u16, message: &str) -> ClientError {
    match code {
        4012 => ClientError::ControlDenied(message.to_string()),
        4101 => ClientError::ProtocolUnsupported(message.to_string()),
        4003 | 4010 | 4011 => ClientError::AuthRejected { code, message: message.to_string() },
        _ => ClientError::Server { code, message: message.to_string() },
    }
}

/// 从 link 错误串提取 wire 码（`LinkError::Signal("... [4101] ...")` 形，
/// link::connect 在入房阶段把 server Error 码嵌进消息串——这里恢复 typed 分类）。
/// 从 link 错误串提取 wire 码（`LinkError::Signal("... [4101] ...")` 形），
/// link::connect 在入房阶段把 server Error 码嵌进消息串——这里恢复 typed 分类）。
/// LinkError 无 Clone——取值转换，fallback 转为 Signal 变体。
#[must_use]
pub fn classify_link_error(err: LinkError) -> ClientError {
    let text = err.to_string();
    match extract_wire_code(&text) {
        Some(code) if matches!(code, 4003 | 4010 | 4011 | 4012 | 4101) => {
            from_wire_error(code, &text)
        }
        _ => ClientError::Signal(err),
    }
}

/// 提取首个 `[<u16>]` 数字（无 = None）。纯函数，测试钉。
#[must_use]
pub fn extract_wire_code(text: &str) -> Option<u16> {
    let start = text.find('[')? + 1;
    let rest = &text[start..];
    let end = rest.find(']')?;
    rest[..end].parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_code_classification() {
        assert!(matches!(
            from_wire_error(4012, "control_denied: viewer"),
            ClientError::ControlDenied(_)
        ));
        assert!(matches!(
            from_wire_error(4101, "protocol too old"),
            ClientError::ProtocolUnsupported(_)
        ));
        assert!(matches!(
            from_wire_error(4003, "psk"),
            ClientError::AuthRejected { code: 4003, .. }
        ));
        assert!(matches!(from_wire_error(5000, "boom"), ClientError::Server { code: 5000, .. }));
    }

    #[test]
    fn extract_wire_code_parses_first_bracket() {
        assert_eq!(
            extract_wire_code("room join failed [4101] protocol_version_unsupported"),
            Some(4101)
        );
        assert_eq!(extract_wire_code("no code here"), None);
        assert_eq!(extract_wire_code("[999999]"), None); // u16 溢出 → None
    }

    #[test]
    fn classify_link_error_maps_4101_to_typed() {
        let e = LinkError::Signal("room join failed [4101]: too old".into());
        assert!(matches!(classify_link_error(e), ClientError::ProtocolUnsupported(_)));
        let other = LinkError::Signal("connect refused".into());
        assert!(matches!(classify_link_error(other), ClientError::Signal(_)));
    }
}
