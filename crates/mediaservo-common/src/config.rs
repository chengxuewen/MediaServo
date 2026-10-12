//! Shared config schemas for all MediaServo components.
//!
//! Each component reads its YAML config file and deserializes into these types.

use serde::{Deserialize, Serialize};

/// Config for mediaservo-host (capture + encode + push — field/vehicle side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    /// Config schema version for migration compatibility.
    #[serde(default = "default_version")]
    pub version: u8,

    /// Server address (WebSocket signaling + WebRTC ICE).
    pub server: ServerAddress,

    /// Capture source configuration.
    pub capture: CaptureConfig,

    /// Encoding parameters.
    pub encoder: EncoderConfig,

    /// WebRTC configuration.
    pub webrtc: Option<WebRtcPushConfig>,

    /// PSK for signaling auth.
    pub psk: Option<String>,

    /// Room configuration.
    #[serde(default)]
    pub room: RoomConfig,

    /// SFU produce mode (mediasoup). When true, host pushes via SFU instead of P2P.
    #[serde(default)]
    pub sfu_produce: bool,
}

/// Config for mediaservo-server (signaling + relay + monitoring).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Config schema version.
    #[serde(default = "default_version")]
    pub version: u8,

    /// Listen address for HTTP/WS server.
    pub listen: ListenConfig,

    /// Room capacity limit.
    #[serde(default = "default_room_capacity")]
    pub room_capacity: usize,

    /// PSK for signaling auth.
    pub psk: Option<String>,
    /// JWT secret for token-based signaling auth (optional, PSK used as fallback).
    #[serde(default)]
    pub jwt_secret: Option<String>,

    /// JWT secret for admin API authentication (optional).
    #[serde(default)]
    pub admin_jwt_secret: Option<String>,

    /// Rate limit (requests per second per connection).
    #[serde(default = "default_rate_limit")]
    pub rate_limit: u32,

    /// WebSocket max message size in bytes (default: 65536 = 64KB).
    #[serde(default = "default_ws_max_message_size")]
    pub ws_max_message_size: usize,

    /// Limit on Consumer peers per stream (default: 50).
    #[serde(default = "default_consumer_limit_per_stream")]
    pub consumer_limit_per_stream: usize,

    /// TLS configuration (optional). When set, server listens on WSS/HTTPS.
    #[serde(default)]
    pub tls: Option<TlsConfig>,

    /// G2 设备注册表文件（YAML: `devices: {<id>: {secret_hash}}`）。
    /// 缺省 = `/opt/mediaservo/etc/devices.yaml`；文件缺失 = 空注册表（PSK 路径不受影响）。
    #[serde(default)]
    pub devices_file: Option<String>,

    /// G3 舱端账号文件（YAML: `accounts: {<username>: {password_hash, role, vehicles}}`）。
    /// 缺省 = `/opt/mediaservo/etc/accounts.yaml`；文件缺失 = 无账号（仅 PSK/设备路径）。
    #[serde(default)]
    pub accounts_file: Option<String>,

    /// API-key 注册表文件（accountless-client-auth T1，YAML: `api_keys: {<id>: {secret_hash,
    /// role, vehicles}}`）——`POST /api/auth/exchange` 免账号换短 JWT 的凭证源。
    /// 缺省 = server.yaml 同目录 `api_keys.yaml`；文件缺失 = 空注册表（exchange 恒 401，
    /// 不影响账号/PSK/设备通路）。
    #[serde(default)]
    pub api_keys_file: Option<String>,

    /// exchange 签发 JWT 的存活秒数（缺省 43200=12h，对齐账号登录 TTL；吊销粒度见 F10）。
    #[serde(default = "default_api_token_ttl")]
    pub api_token_ttl_secs: u64,

    /// SFU 配置（mediasoup WebRtcServer）。
    #[serde(default)]
    pub sfu: SfuConfig,

    /// Web 静态层（caddy）配置。缺省全 None = 现行为（8080 + 实例 `web/`）。
    #[serde(default)]
    pub web: WebConfig,
}

/// mediasoup WebRtcServer 缺省监听口——`sfu.port` 解析链（yaml > env MEDIASERVO_SFU_PORT >
/// 本值，D-B 10-12）的缺省位单源：sfu.rs 消费 + common lib.rs env-usage 渲染共引，禁再写字面量。
pub const DEFAULT_SFU_PORT: u16 = 20000;

/// SFU（mediasoup WebRtcServer）配置。
///
/// ⚠️ 手写 Default（勿改回 derive）：`Option<u16>` 字段若 derive(Default)
/// 不会出问题，但历史教训是 u16 数值字段 derive 会得 0；手写集中管控，
/// 钉死测试 `default_impl_matches_serde_absent`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SfuConfig {
    /// 对外公告地址（多网卡/多网络可达 IP，如 `["192.168.2.127", "10.144.0.3"]`）。
    /// 优先级: env `MEDIASERVO_SFU_ANNOUNCED_IP` > 本字段 > 自动探测（出网 IP）。
    /// mediasoup 要求 0.0.0.0 必须配 announced（PIT-44/58/138——容器内探测不可达）。
    #[serde(default)]
    pub announced_ips: Vec<String>,
    /// mediasoup WebRtcServer 监听口。None = env `MEDIASERVO_SFU_PORT` 兜底 → 20000。
    #[serde(default)]
    pub port: Option<u16>,
    /// weaknet-serve 面板口。None = 9810（`WEAKNET_SERVE_PORT` 常量）。仅改端口数字，恒 loopback。
    #[serde(default)]
    pub weaknet_port: Option<u16>,
}

#[allow(clippy::derivable_impls)] // 勿改回 derive——见结构体 doc「手写 Default」决策 + 钉死测试
impl Default for SfuConfig {
    fn default() -> Self {
        Self { announced_ips: Vec::new(), port: None, weaknet_port: None }
    }
}

/// Web 静态层（caddy）配置 — server.yaml `web:` 段（init 渲染 Caddyfile 的单一来源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebConfig {
    /// Caddy 对外端口。None = env `MEDIASERVO_WEB_PORT` 兜底 → 8080。
    /// Option 而非带值缺省的原因：区分“用户写 8080”与“serde 填 8080”，保 env 兜底语义存活。
    #[serde(default)]
    pub port: Option<u16>,
    /// 静态站点根目录。None = 实例目录 `web/`。相对路径按实例根目录解析。
    #[serde(default)]
    pub static_root: Option<String>,
}

#[allow(clippy::derivable_impls)] // 同 SfuConfig——手写 Default 项目决策
impl Default for WebConfig {
    fn default() -> Self {
        Self { port: None, static_root: None }
    }
}

/// Config for mediaservo-client (pull + decode + control — cockpit/operator side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Config schema version.
    #[serde(default = "default_version")]
    pub version: u8,

    /// Server address.
    pub server: ServerAddress,

    /// PSK for signaling auth.
    pub psk: Option<String>,

    /// Room configuration.
    #[serde(default)]
    pub room: RoomConfig,

    /// Render window configuration (platform-specific, optional).
    pub render: Option<RenderConfig>,
}

// --- Sub-types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerAddress {
    /// WebSocket signaling URL (e.g., "ws://192.168.1.1:9800/ws").
    pub signaling_url: String,

    /// ICE server addresses (STUN/TURN URIs, optional).
    #[serde(default)]
    pub ice_servers: Vec<String>,
}

/// Room configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomConfig {
    /// Room ID (default: "default").
    #[serde(default = "default_room_id")]
    pub id: String,
}

impl Default for RoomConfig {
    fn default() -> Self {
        Self { id: default_room_id() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenConfig {
    /// Bind host.
    #[serde(default = "default_host")]
    pub host: String,

    /// HTTP/WS port.
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// Capture source type: "screen", "camera", or "test_pattern".
    pub source: String,

    /// Resolution in format "WIDTHxHEIGHT" (e.g., "1280x720").
    #[serde(default = "default_resolution")]
    pub resolution: String,

    /// Frame rate.
    #[serde(default = "default_framerate")]
    pub framerate: u32,

    /// Device path (e.g., /dev/video0 for V4L2). Optional.
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderConfig {
    /// Encoder backend: "nvenc", "videotoolbox", "vaapi", or "software".
    #[serde(default = "default_encoder")]
    pub backend: String,

    /// Target bitrate in kbps — ⚠️ 仅作用于 GStreamer 捕获/编码管线（pipeline.rs）;
    /// SFU 主路径（test_pattern→WebRtcTrackSink→libwebrtc 编码器）不经过 GStreamer 编码,
    /// 对 WebRTC 编码器零影响 — 用 min/max_bitrate_kbps 控制 WebRTC 编码码率。
    #[serde(default = "default_bitrate")]
    pub bitrate_kbps: u32,

    /// Keyframe interval in **seconds**（PIT-76: 周期 request_key_frame 触发，
    /// 语义从 GOP 帧数改为秒；0 视为 1 防御）。
    #[serde(default = "default_gop")]
    pub keyframe_interval: u32,

    /// 编码 codec 偏好（v2, encoder-backend-codec-config）: "auto"/"vp8"/"h264"/"vp9"/"av1"。
    /// auto = router 决定（现状）; 指定值 = 构造远程 offer 时只声明目标 codec
    /// （libwebrtc 实际编码 = 协商交集 = offer codec, 三者一致）。
    #[serde(default = "default_codec")]
    pub codec: String,
    /// 最高码率 (kbps) — v2 (encoder-bitrate): libwebrtc 可靠硬上限, None=不限制
    pub max_bitrate_kbps: Option<u32>,
    /// 最低码率 (kbps) — v2 (encoder-bitrate): 受限链路 best-effort 下限, None=不限制
    pub min_bitrate_kbps: Option<u32>,
}

impl EncoderConfig {
    /// 码率区间校验（encoder-bitrate）: 若同时设置, 必须 0 < min < max,
    /// 否则 libwebrtc 双失效（video_stream_encoder.cc:444 min>=max → 两者都不应用）。
    pub fn validate_bitrate(&self) -> Result<(), String> {
        if let (Some(min), Some(max)) = (self.min_bitrate_kbps, self.max_bitrate_kbps) {
            if min == 0 {
                return Err("encoder.min_bitrate_kbps 必须 > 0".into());
            }
            if min >= max {
                return Err(format!(
                    "encoder.min_bitrate_kbps({min}) 必须 < max_bitrate_kbps({max}), 否则 libwebrtc 双失效"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRtcPushConfig {
    /// ICE connection timeout in seconds.
    #[serde(default = "default_ice_timeout")]
    pub ice_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderConfig {
    /// Render backend: "auto" (platform default) or explicit like "gtk", "metal", etc.
    #[serde(default = "default_render_backend")]
    pub backend: String,
}
/// TLS configuration for native rustls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to PEM-encoded certificate chain file.
    pub cert_path: String,

    /// Path to PEM-encoded private key file.
    pub key_path: String,
}

// --- Defaults ---

fn default_version() -> u8 {
    1
}
fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    9800
}
fn default_resolution() -> String {
    "1280x720".into()
}
fn default_framerate() -> u32 {
    30
}
fn default_encoder() -> String {
    "auto".into()
}
fn default_bitrate() -> u32 {
    2000
}
fn default_codec() -> String {
    "auto".to_string()
}
fn default_gop() -> u32 {
    // PIT-76: 语义从帧改为秒（周期关键帧触发间隔）
    2
}
fn default_room_capacity() -> usize {
    10
}
#[allow(dead_code)]
fn default_rate_limit() -> u32 {
    100
}
fn default_ws_max_message_size() -> usize {
    65536
}
fn default_api_token_ttl() -> u64 {
    43200
}

fn default_consumer_limit_per_stream() -> usize {
    50
}
fn default_room_id() -> String {
    "default".to_string()
}
fn default_ice_timeout() -> u64 {
    30
}
fn default_render_backend() -> String {
    "auto".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_config_roundtrip() {
        let yaml = r#"
server:
  signaling_url: "ws://server:9800/ws"
  ice_servers: ["stun:stun.example.com:3478"]
capture:
  source: "camera"
  resolution: "1920x1080"
  framerate: 60
  device: "/dev/video0"
encoder:
  backend: "nvenc"
  bitrate_kbps: 4000
  keyframe_interval: 2
psk: "secret123"
webrtc:
  ice_timeout_secs: 45
"#;
        let parsed: HostConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.server.signaling_url, "ws://server:9800/ws");
        assert_eq!(parsed.server.ice_servers, vec!["stun:stun.example.com:3478"]);
        assert_eq!(parsed.capture.source, "camera");
        assert_eq!(parsed.capture.resolution, "1920x1080");
        assert_eq!(parsed.capture.framerate, 60);
        assert_eq!(parsed.capture.device.as_deref(), Some("/dev/video0"));
        assert_eq!(parsed.encoder.backend, "nvenc");
        assert_eq!(parsed.encoder.bitrate_kbps, 4000);
        assert_eq!(parsed.encoder.keyframe_interval, 2); // PIT-76: 秒
        assert_eq!(parsed.psk.as_deref(), Some("secret123"));
        assert_eq!(parsed.webrtc.as_ref().unwrap().ice_timeout_secs, 45);

        // serialize → parse round-trip
        let re_serialized = serde_yaml::to_string(&parsed).unwrap();
        let re_parsed: HostConfig = serde_yaml::from_str(&re_serialized).unwrap();
        assert_eq!(re_parsed.server.signaling_url, parsed.server.signaling_url);
        assert_eq!(re_parsed.capture.framerate, parsed.capture.framerate);
    }

    #[test]
    fn server_config_roundtrip() {
        let yaml = r#"
listen:
  host: "127.0.0.1"
  port: 8080
room_capacity: 50
rate_limit: 200
psk: "server-psk"
jwt_secret: "jwt-secret-256bit-min"
"#;
        let parsed: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.listen.host, "127.0.0.1");
        assert_eq!(parsed.listen.port, 8080);
        assert_eq!(parsed.room_capacity, 50);
        assert_eq!(parsed.rate_limit, 200);
        assert_eq!(parsed.psk.as_deref(), Some("server-psk"));
        assert_eq!(parsed.jwt_secret.as_deref(), Some("jwt-secret-256bit-min"));

        let re_serialized = serde_yaml::to_string(&parsed).unwrap();
        let re_parsed: ServerConfig = serde_yaml::from_str(&re_serialized).unwrap();
        assert_eq!(re_parsed.listen.port, parsed.listen.port);
        assert_eq!(re_parsed.room_capacity, parsed.room_capacity);
    }

    #[test]
    fn remote_config_roundtrip() {
        let yaml = r#"
server:
  signaling_url: "ws://remote:9800/ws"
psk: "remote-psk"
render:
  backend: "metal"
"#;
        let parsed: RemoteConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.server.signaling_url, "ws://remote:9800/ws");
        assert_eq!(parsed.psk.as_deref(), Some("remote-psk"));
        assert_eq!(parsed.render.as_ref().unwrap().backend, "metal");

        let re_serialized = serde_yaml::to_string(&parsed).unwrap();
        let re_parsed: RemoteConfig = serde_yaml::from_str(&re_serialized).unwrap();
        assert_eq!(re_parsed.server.signaling_url, parsed.server.signaling_url);
    }

    #[test]
    fn version_default() {
        let yaml = r#"
server:
  signaling_url: "ws://host:9800/ws"
capture:
  source: "screen"
encoder:
  backend: "auto"
"#;
        let parsed: HostConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.version, 1);
    }

    #[test]
    fn optional_psk_field() {
        let with_psk = r#"
server:
  signaling_url: "ws://host:9800/ws"
capture:
  source: "screen"
encoder:
  backend: "auto"
psk: "my-psk"
"#;
        let parsed: HostConfig = serde_yaml::from_str(with_psk).unwrap();
        assert_eq!(parsed.psk.as_deref(), Some("my-psk"));

        let without_psk = r#"
server:
  signaling_url: "ws://host:9800/ws"
capture:
  source: "screen"
encoder:
  backend: "auto"
"#;
        let parsed: HostConfig = serde_yaml::from_str(without_psk).unwrap();
        assert_eq!(parsed.psk, None);
    }

    #[test]
    fn capture_config_defaults() {
        let yaml = r#"
source: "test_pattern"
"#;
        let parsed: CaptureConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.source, "test_pattern");
        assert_eq!(parsed.resolution, "1280x720");
        assert_eq!(parsed.framerate, 30);
        assert_eq!(parsed.device, None);
    }

    #[test]
    fn encoder_config_defaults() {
        let yaml = r#"
backend: "software"
"#;
        let parsed: EncoderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.backend, "software");
        assert_eq!(parsed.bitrate_kbps, 2000);
        assert_eq!(parsed.keyframe_interval, 2); // PIT-76: 秒
    }

    #[test]
    fn listen_config_defaults() {
        let yaml = r#"
host: "192.168.1.1"
"#;
        let parsed: ListenConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.host, "192.168.1.1");
        assert_eq!(parsed.port, 9800);
    }

    #[test]
    fn server_config_defaults() {
        let yaml = "listen: {}";
        let parsed: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.listen.host, "0.0.0.0");
        assert_eq!(parsed.listen.port, 9800);
        assert_eq!(parsed.room_capacity, 10);
        assert_eq!(parsed.rate_limit, 100);
        assert_eq!(parsed.psk, None);
        assert_eq!(parsed.ws_max_message_size, 65536);
        assert_eq!(parsed.jwt_secret, None);
        assert_eq!(parsed.admin_jwt_secret, None);
        assert!(parsed.tls.is_none());
        assert_eq!(parsed.consumer_limit_per_stream, 50);
    }

    #[test]
    fn server_config_with_tls() {
        let yaml = r#"
listen:
  port: 443
tls:
  cert_path: "/etc/certs/server.crt"
  key_path: "/etc/certs/server.key"
"#;
        let parsed: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        let tls = parsed.tls.unwrap();
        assert_eq!(tls.cert_path, "/etc/certs/server.crt");
        assert_eq!(tls.key_path, "/etc/certs/server.key");
        assert_eq!(tls.key_path, "/etc/certs/server.key");
    }

    #[test]
    fn consumer_limit_default_and_override() {
        // Default
        let yaml = "listen: {}";
        let parsed: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.consumer_limit_per_stream, 50);

        // Override
        let yaml = r#"
listen: {}
consumer_limit_per_stream: 100
"#;
        let parsed: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.consumer_limit_per_stream, 100);
    }

    #[test]
    fn encoder_bitrate_validate() {
        // 默认（无 min/max）→ OK
        let cfg = EncoderConfig {
            backend: "auto".into(),
            bitrate_kbps: 2000,
            keyframe_interval: 2,
            codec: "h264".into(),
            max_bitrate_kbps: None,
            min_bitrate_kbps: None,
        };
        assert!(cfg.validate_bitrate().is_ok());

        // 只设 max → OK
        let cfg = EncoderConfig { max_bitrate_kbps: Some(4000), ..cfg.clone() };
        assert!(cfg.validate_bitrate().is_ok());

        // min < max → OK
        let cfg = EncoderConfig {
            min_bitrate_kbps: Some(500),
            max_bitrate_kbps: Some(4000),
            ..cfg.clone()
        };
        assert!(cfg.validate_bitrate().is_ok());

        // min >= max → Err（防双失效）
        let cfg = EncoderConfig {
            min_bitrate_kbps: Some(4000),
            max_bitrate_kbps: Some(4000),
            ..cfg.clone()
        };
        assert!(cfg.validate_bitrate().is_err());

        // min == 0 → Err
        let cfg = EncoderConfig {
            min_bitrate_kbps: Some(0),
            max_bitrate_kbps: Some(4000),
            ..cfg.clone()
        };
        assert!(cfg.validate_bitrate().is_err());
    }

    // ---- config-unification P1: web/sfu 端口键（additive, 旧 yaml 零改动） ----

    #[test]
    fn server_config_old_yaml_backward_compat() {
        // 1.1 版前的完整旧 yaml（含 sfu 旧形）→ 新键全 None = env/default 兜底存活
        let yaml = r#"
version: 1
listen: {host: "0.0.0.0", port: 9800}
room_capacity: 50
psk: "old-psk"
jwt_secret: "old-jwt"
admin_jwt_secret: "old-jwt"
rate_limit: 100
consumer_limit_per_stream: 50
devices_file: "devices.yaml"
accounts_file: "accounts.yaml"
sfu:
  announced_ips: ["192.168.2.127"]
"#;
        let cfg: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.web.port, None);
        assert_eq!(cfg.web.static_root, None);
        assert_eq!(cfg.sfu.port, None);
        assert_eq!(cfg.sfu.weaknet_port, None);
        assert_eq!(
            cfg.sfu.announced_ips,
            ["192.168.2.127"].iter().map(|s| s.to_string()).collect::<Vec<String>>()
        );
        // roundtrip 全等
        let re: ServerConfig = serde_yaml::from_str(&serde_yaml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(re.sfu.weaknet_port, None);
        assert_eq!(re.sfu.port, None);
        assert_eq!(re.web, cfg.web);
    }

    #[test]
    fn web_config_full_partial_absent() {
        // full
        let full: WebConfig = serde_yaml::from_str("{port: 9000, static_root: /var/www}").unwrap();
        assert_eq!(full.port, Some(9000));
        assert_eq!(full.static_root.as_deref(), Some("/var/www"));
        // empty
        let empty: WebConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty, WebConfig::default());
        // absent → ServerConfig 层 default
        let parsed: ServerConfig = serde_yaml::from_str("listen: {}").unwrap();
        assert_eq!(parsed.web, WebConfig::default());
    }

    #[test]
    fn sfu_config_port_keys() {
        // 两端口键同时解析
        let sfu: SfuConfig = serde_yaml::from_str("{port: 21000, weaknet_port: 9811}").unwrap();
        assert_eq!(sfu.port, Some(21000));
        assert_eq!(sfu.weaknet_port, Some(9811));
        // announced_ips 单独在场 → 两端口仍 None（per-field 缺省不串味）
        let sfu: SfuConfig = serde_yaml::from_str("{announced_ips: [x]}").unwrap();
        assert_eq!(sfu.port, None);
        assert_eq!(sfu.weaknet_port, None);
    }

    #[test]
    fn default_impl_matches_serde_absent() {
        // 防未来有人改回 derive(Default) 引入 port=0 回归
        let parsed: SfuConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(parsed, SfuConfig::default());
        assert_eq!(parsed.port, None);
        assert_eq!(parsed.weaknet_port, None);
        let parsed: WebConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(parsed, WebConfig::default());
    }
}
