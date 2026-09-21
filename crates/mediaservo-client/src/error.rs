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

impl ClientError {
    /// S6/K2: wire 码反向可查——`from_wire_error` 分类源的机读回读面。
    ///
    /// 单源纪律：4012/4101/4003|4010|4011 的归属与 [`from_wire_error`] 一致，
    /// 不重抄映射表；`Signal` 变体经 [`extract_wire_code`] 从 link 嵌入串恢复。
    /// 返回 `None` = 该错误无 wire 码（本地/REST/IO 域）。
    #[must_use]
    pub fn wire_code(&self) -> Option<u32> {
        match self {
            // 无 wire 码的本地/REST/传输域。
            Self::Login(_)
            | Self::InvalidCredentials
            | Self::RestRejected { .. }
            | Self::UnsupportedScheme(_)
            | Self::MalformedResponse(_)
            | Self::WebRtc(_)
            | Self::Io(_)
            | Self::Timeout { .. }
            | Self::InvalidState(_)
            // I5 本地预拒（未发出任何请求，无 server 应答码）。
            | Self::ProtocolTooLow { .. } => None,
            Self::AuthRejected { code, .. } => Some(u32::from(*code)),
            Self::ProtocolUnsupported(_) => Some(4101),
            Self::ControlDenied(_) => Some(4012),
            Self::Server { code, .. } => Some(u32::from(*code)),
            Self::Signal(e) => extract_wire_code(&e.to_string()).map(u32::from),
        }
    }

    /// S6/K2: 重试语义一等位（K1 重连环的判据源）。
    ///
    /// 出处 = D273 红牌家族语义（web 半区已实盘）：auth 族（4003/4010/4011/4012）
    /// 与方言拒（4101）= 终态（重连必然同败，重试只掩盖凭证问题）；连接类
    /// （IO/Signal 无码/WebRtc 建连/Timeout）与 server 域 ≥5000（5001=SFU 暂不可用
    /// 等瞬态）= 可重试。REST 拒/协议错位/状态违例按保守不重试（4xx 语义）。
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            // 终态族（D273 红牌 = auth 族 + 4101；ProtocolTooLow = 本地门同族）。
            Self::InvalidCredentials
            | Self::AuthRejected { .. }
            | Self::ProtocolUnsupported(_)
            | Self::ControlDenied(_)
            | Self::ProtocolTooLow { .. }
            | Self::UnsupportedScheme(_)
            | Self::RestRejected { .. }
            | Self::MalformedResponse(_)
            | Self::InvalidState(_) => false,
            // 连接/瞬态族。
            Self::Io(_) | Self::WebRtc(_) | Self::Timeout { .. } | Self::Login(_) => true,
            Self::Server { code, .. } => *code >= 5000,
            // link 透传：嵌入 auth 族码 = 终态；纯连接错 = 可重试。
            Self::Signal(e) => {
                !matches!(extract_wire_code(&e.to_string()), Some(4003 | 4010 | 4011 | 4012 | 4101))
            }
        }
    }
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

    /// K2 钉：wire_code 与 from_wire_error 分类源一致（穷尽 match 的表外回归网）。
    #[test]
    fn wire_code_round_trips_classification_source() {
        assert_eq!(from_wire_error(4012, "x").wire_code(), Some(4012));
        assert_eq!(from_wire_error(4101, "x").wire_code(), Some(4101));
        assert_eq!(from_wire_error(4010, "x").wire_code(), Some(4010));
        assert_eq!(from_wire_error(5001, "x").wire_code(), Some(5001));
        // 本地域无码。
        assert_eq!(ClientError::InvalidCredentials.wire_code(), None);
        assert_eq!(
            ClientError::ProtocolTooLow { need: 2, got: 1 }.wire_code(),
            None,
            "本地预拒不发请求 = 无 wire 码"
        );
        // Signal 嵌入码恢复（classify 未命中的未知码从串里捞回）。
        let e = ClientError::Signal(LinkError::Signal("room join failed [4031]: denied".into()));
        assert_eq!(e.wire_code(), Some(4031));
    }

    /// K2 钉：重试分类对齐 D273 红牌家族（auth 族/4101/4012 终态；连接类/5001 可重试）。
    #[test]
    fn retryable_matches_d273_red_card_family() {
        for terminal in [
            from_wire_error(4003, "psk"),
            from_wire_error(4010, "device"),
            from_wire_error(4011, "role"),
            from_wire_error(4012, "control"),
            from_wire_error(4101, "protocol"),
            ClientError::InvalidCredentials,
            ClientError::RestRejected { code: 401, message: String::new() },
        ] {
            assert!(!terminal.is_retryable(), "auth/方言族必须终态: {terminal:?}");
        }
        for retry in [
            ClientError::Io(std::io::Error::other("refused")),
            ClientError::WebRtc("ice".into()),
            ClientError::Timeout { what: "Consumed" },
            from_wire_error(5000, "boom"),
            from_wire_error(5001, "sfu unavailable"),
            ClientError::Signal(LinkError::Signal("connect ws://x: refused".into())),
        ] {
            assert!(retry.is_retryable(), "连接/≥5000 族必须可重试: {retry:?}");
        }
        // Signal 嵌入 auth 码 = 终态（link 透传形与 typed 形同判）。
        assert!(!ClientError::Signal(LinkError::Signal("join [4010] nope".into())).is_retryable());
        // 4xxx 非 auth 的 Server 码（如 4031 produce 拒）保守不重试。
        assert!(!from_wire_error(4031, "video in audio room").is_retryable());
    }
}
