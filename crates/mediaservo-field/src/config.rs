//! field 会话配置（契约 §4 落地）。
//!
//! `PushConfig`/`PullConfig` 是会话的入口配置；`PublishOptions` 为发布
//! 富选项（MVP 只落 codec + encoder_backend，其余待真传输接入后按需扩展）。

use mediaservo_common::protocol::PeerRole;
use mediaservo_webrtc::rtp::{RTCDegradationPreference, RTCRtpContentHint};

/// 推流策略档位（qos-framerate-priority，AD-1：用户面只暴露策略轴，
/// 原语展开由 bundle() 纯表完成，合并裁决在 host translate）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    /// 实时优先：保帧率降分辨率 + Fluid hint + 400kbps 地板（遥控推荐）
    Smooth,
    /// libwebrtc 均衡双降（缺省，行为与现状逐字节一致）
    Balanced,
    /// 画质优先：保分辨率降帧率 + 3000kbps 天花板（取证推荐，AD-3）
    Quality,
}

impl StreamMode {
    /// CLI/配置字符串形态（与 FromStr 对称，translate/streamer 共用）。
    pub fn to_str(self) -> &'static str {
        match self {
            StreamMode::Smooth => "smooth",
            StreamMode::Balanced => "balanced",
            StreamMode::Quality => "quality",
        }
    }

    /// 档位 → 原语 bundle（纯数据，表驱动单测友好）。
    pub fn bundle(self) -> PresetBundle {
        match self {
            StreamMode::Smooth => PresetBundle {
                degradation: RTCDegradationPreference::MaintainFramerate,
                content_hint: RTCRtpContentHint::Fluid,
                min_bitrate_kbps: Some(400),
                bitrate_kbps: None,
            },
            StreamMode::Balanced => PresetBundle::default(),
            StreamMode::Quality => PresetBundle {
                degradation: RTCDegradationPreference::MaintainResolution,
                content_hint: RTCRtpContentHint::None,
                min_bitrate_kbps: None,
                bitrate_kbps: Some(3000),
            },
        }
    }
}

impl std::str::FromStr for StreamMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "smooth" => Ok(Self::Smooth),
            "balanced" => Ok(Self::Balanced),
            "quality" => Ok(Self::Quality),
            _ => Err(format!("非法 stream_mode 值 {s:?}（合法: smooth|balanced|quality）")),
        }
    }
}

/// StreamMode 展开后的原语集（translate 消费：显式配置 > bundle）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresetBundle {
    pub degradation: RTCDegradationPreference,
    pub content_hint: RTCRtpContentHint,
    pub min_bitrate_kbps: Option<u32>,
    pub bitrate_kbps: Option<u32>,
}


/// 推流会话配置（契约 §4）。
#[derive(Debug, Clone)]
pub struct PushConfig {
    /// 信令 WS 地址，如 `ws://host:9800/ws`。
    pub url: String,
    /// PSK 认证密钥。
    pub psk: String,
    /// 房间 ID。
    pub room: String,
    /// 节点角色（Host/Pusher）。
    pub role: PeerRole,
    /// 推流视频分辨率（宽）。
    pub width: u32,
    /// 推流视频分辨率（高）。
    pub height: u32,
    /// 帧率（与 libwebrtc 编码器配置匹配，C17）。
    pub framerate: u32,
    /// 编码码率 kbps。
    pub bitrate_kbps: u32,
    /// 弱网降级偏好（qos-framerate-priority；缺省 Balanced=不调 setter，AD-6）。
    pub degradation: RTCDegradationPreference,
    /// 内容 hint（缺省 None=不调 setter；Smooth 档 bundle 为 Fluid）。
    pub content_hint: RTCRtpContentHint,
    /// 码率地板 bps 的 kbps 形态（None=不设下限；Smooth 档 bundle 为 400）。
    pub min_bitrate_kbps: Option<u32>,
    /// 关键帧间隔秒（GOP 上限，默认 2）。
    pub keyframe_interval: u64,
    /// D2 本地网关模式：Some(src) = 通过 host-agent 网关连接
    /// （LocalEnvelope 信封 wire，无 PSK；整车 PSK 在 agent 远端）；
    /// None = 直连 server（PSK 认证）。
    pub gateway_src: Option<String>,
}

impl PushConfig {
    /// 便捷构造（默认 1280x720@30fps / 2000kbps / 2s GOP；直连 server 模式）。
    pub fn new(url: impl Into<String>, psk: impl Into<String>, room: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            psk: psk.into(),
            room: room.into(),
            role: PeerRole::Host,
            width: 1280,
            height: 720,
            framerate: 30,
            bitrate_kbps: 2000,
            degradation: RTCDegradationPreference::Balanced,
            content_hint: RTCRtpContentHint::None,
            min_bitrate_kbps: None,
            keyframe_interval: 2,
            gateway_src: None,
        }
    }

    /// 本地网关模式构造（D2）：url 为 host-agent 本地地址，无 PSK 挑战
    /// （信任边界 127.0.0.1；整车 PSK 在 agent 的远端连接）。
    pub fn via_gateway(url: impl Into<String>, src: impl Into<String>, room: impl Into<String>) -> Self {
        Self::new(url, "", room).with_gateway(src)
    }

    /// 启用本地网关模式（链式；供配置复用）。
    pub fn with_gateway(mut self, src: impl Into<String>) -> Self {
        self.gateway_src = Some(src.into());
        self
    }
}

/// 拉流会话配置（契约 §4；MVP 仅定义类型，connect 暂未接入 consume 链路）。
#[derive(Debug, Clone)]
pub struct PullConfig {
    /// 信令 WS 地址。
    pub url: String,
    /// PSK 认证密钥。
    pub psk: String,
    /// 房间 ID。
    pub room: String,
    /// 节点角色（Remote/Consumer）。
    pub role: PeerRole,
    /// 是否自动订阅房间内所有 producer。
    pub auto_subscribe: bool,
}

impl Default for PullConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            psk: String::new(),
            room: String::new(),
            role: PeerRole::Remote,
            auto_subscribe: true,
        }
    }
}

/// 发布选项（契约 §4；MVP 只落 codec + encoder_backend）。
#[derive(Debug, Clone)]
pub struct PublishOptions {
    /// 编码格式（VP8/H264/VP9/AV1，与 router 对齐）。
    pub codec: String,
    /// 编码器后端（auto/software/hardware）。
    pub encoder_backend: String,
}

impl Default for PublishOptions {
    fn default() -> Self {
        Self {
            codec: "vp8".to_string(),
            encoder_backend: "auto".to_string(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_config_defaults_sane() {
        let cfg = PushConfig::new("ws://x", "psk", "room");
        assert_eq!(cfg.width, 1280);
        assert_eq!(cfg.height, 720);
        assert_eq!(cfg.framerate, 30);
        assert_eq!(cfg.bitrate_kbps, 2000);
        assert_eq!(cfg.keyframe_interval, 2);
        assert_eq!(cfg.role, PeerRole::Host);
    }

    #[test]
    fn push_config_custom_values_preserved() {
        let mut cfg = PushConfig::new("ws://x", "psk", "room");
        cfg.width = 640;
        cfg.height = 480;
        cfg.framerate = 15;
        cfg.bitrate_kbps = 800;
        cfg.keyframe_interval = 4;
        assert_eq!(cfg.width, 640);
        assert_eq!(cfg.framerate, 15);
        assert_eq!(cfg.bitrate_kbps, 800);
        assert_eq!(cfg.keyframe_interval, 4);
    }

    #[test]
    fn publish_options_defaults_vp8_auto() {
        let opts = PublishOptions::default();
        assert_eq!(opts.codec, "vp8");
        assert_eq!(opts.encoder_backend, "auto");
    }

    #[test]
    fn pull_config_default_auto_subscribe() {
        let cfg = PullConfig::default();
        assert!(cfg.auto_subscribe);
        assert_eq!(cfg.role, PeerRole::Remote);
    }

    #[test]
    fn push_config_url_trims_trailing_slash_in_connect() {
        // SignalClient 内部 trim_end_matches('/') — 配置保持原样, 连接时处理
        let cfg = PushConfig::new("ws://host:9800/ws/", "psk", "room");
        assert_eq!(cfg.url, "ws://host:9800/ws/");
    }

    #[test]
    fn push_config_gateway_mode_defaults_off_and_switchable() {
        // D2: 默认直连 server（gateway_src=None）；via_gateway/with_gateway 切换
        let direct = PushConfig::new("ws://x", "psk", "room");
        assert_eq!(direct.gateway_src, None, "默认应直连 server");
        let gw = PushConfig::via_gateway("ws://127.0.0.1:17980/ws", "child-1", "room");
        assert_eq!(gw.gateway_src.as_deref(), Some("child-1"));
        assert_eq!(gw.psk, "", "网关模式无 PSK");
        let chained = direct.clone().with_gateway("child-2");
        assert_eq!(chained.gateway_src.as_deref(), Some("child-2"));
    }

    // qos-framerate-priority T3: StreamMode preset 表 + PushConfig 缺省等价
    #[test]
    fn stream_mode_from_str_legal_and_illegal() {
        assert_eq!("smooth".parse::<StreamMode>().unwrap(), StreamMode::Smooth);
        assert_eq!("balanced".parse::<StreamMode>().unwrap(), StreamMode::Balanced);
        assert_eq!("quality".parse::<StreamMode>().unwrap(), StreamMode::Quality);
        assert!("SMOOTH ".parse::<StreamMode>().is_ok(), "大小写/空白不敏感");
        let e = "turbo".parse::<StreamMode>().unwrap_err();
        assert!(e.contains("smooth|balanced|quality"), "错误信息含合法集: {e}");
        assert!("".parse::<StreamMode>().is_err());
    }

    #[test]
    fn preset_bundle_table_per_ad2_ad3_ad4() {
        use mediaservo_webrtc::rtp::{RTCDegradationPreference, RTCRtpContentHint};
        let s = StreamMode::Smooth.bundle();
        assert_eq!(s.degradation, RTCDegradationPreference::MaintainFramerate);
        assert_eq!(s.content_hint, RTCRtpContentHint::Fluid);
        assert_eq!(s.min_bitrate_kbps, Some(400));
        assert_eq!(s.bitrate_kbps, None, "smooth 不动码率天花板");
        let b = StreamMode::Balanced.bundle();
        assert_eq!(b.degradation, RTCDegradationPreference::Balanced);
        assert_eq!(b.content_hint, RTCRtpContentHint::None);
        assert_eq!(b.min_bitrate_kbps, None);
        assert_eq!(b.bitrate_kbps, None);
        let q = StreamMode::Quality.bundle();
        assert_eq!(q.degradation, RTCDegradationPreference::MaintainResolution);
        assert_eq!(q.content_hint, RTCRtpContentHint::None);
        assert_eq!(q.min_bitrate_kbps, None);
        assert_eq!(q.bitrate_kbps, Some(3000));
    }

    #[test]
    fn push_config_new_defaults_match_legacy_behavior() {
        // AD-6: 缺省构造 = 现状行为（Balanced/None/None → session 不调新 setter）
        let cfg = PushConfig::new("ws://x", "psk", "room");
        assert_eq!(cfg.degradation, mediaservo_webrtc::rtp::RTCDegradationPreference::Balanced);
        assert_eq!(cfg.content_hint, mediaservo_webrtc::rtp::RTCRtpContentHint::None);
        assert_eq!(cfg.min_bitrate_kbps, None);
    }
}
