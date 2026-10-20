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

    /// G2 device registry file (YAML: `devices: {<id>: {secret_hash}}`).
    /// Default = `/opt/mediaservo/etc/devices.yaml`; missing file = empty registry (PSK path unaffected).
    #[serde(default)]
    pub devices_file: Option<String>,

    /// G3 cockpit account file (YAML: `accounts: {<username>: {password_hash, role, vehicles}}`).
    /// Default = `/opt/mediaservo/etc/accounts.yaml`; missing file = no accounts (PSK/device paths only).
    #[serde(default)]
    pub accounts_file: Option<String>,

    /// API-key registry file (accountless-client-auth T1, YAML: `api_keys: {<id>: {secret_hash,
    /// role, vehicles}}`) - credential source for the account-free `POST /api/auth/exchange` short JWT.
    /// Default = `api_keys.yaml` next to server.yaml; missing file = empty registry (exchange always 401,
    /// accounts/PSK/device paths unaffected).
    #[serde(default)]
    pub api_keys_file: Option<String>,

    /// Dev-credentials exemption (PIT-160 root fix, 2026-10-20): config-first equivalent of env
    /// MEDIASERVO_ALLOW_DEV_CREDENTIALS=1. When true, the startup guard accepts the known dev
    /// placeholder accounts (admin123/dispatch123/operator123) in accounts.yaml. Lives in server.yaml
    /// (etc/ is never overwritten by deploy — PIT-160 guard) so it survives tree rebuilds; the env var
    /// remains as an override for container/compose forms (env wins over config).
    #[serde(default)]
    pub allow_dev_credentials: bool,

    /// Device auto-enroll (device-enroll S7 config-first equivalent of env ALLOW_DEV_ENROLL=1):
    /// unknown devices auto-register after pubkey verify passes (controlled private networks only).
    /// Config-first, env override wins (same precedence as allow_dev_credentials).
    #[serde(default)]
    pub allow_dev_enroll: bool,

    /// TTL in seconds of JWTs minted by exchange (default 43200=12h, aligned with account login TTL; revocation granularity see F10).
    #[serde(default = "default_api_token_ttl")]
    pub api_token_ttl_secs: u64,

    /// SFU config (mediasoup WebRtcServer).
    #[serde(default)]
    pub sfu: SfuConfig,

    /// Web static layer (caddy) config. All-None default = current behavior (8080 + instance `web/`).
    #[serde(default)]
    pub web: WebConfig,
}

/// Default listen port of the mediasoup WebRtcServer - single source of the `sfu.port`
/// resolution chain (yaml > env MEDIASERVO_SFU_PORT > this value, D-B 10-12): shared by sfu.rs and the common lib.rs env-usage renderer; do not repeat the literal.
pub const DEFAULT_SFU_PORT: u16 = 20000;

/// Default port of the web(caddy) front end - single source of the `web.port`
/// resolution chain (yaml > env MEDIASERVO_WEB_PORT > this value) (T6b, same pattern as DEFAULT_SFU_PORT):
/// shared by templates.rs init rendering and start/doctor Caddyfile drift detection; do not repeat the literal.
pub const DEFAULT_WEB_PORT: u16 = 8080;

/// Default port of the weaknet-serve panel - single source of the `sfu.weaknet_port` chain (T6b).
/// Cross anchor (zero-new-dependency discipline): the weaknet crate does not depend on common, so the
/// port segment of the `DEFAULT_LISTEN` string literal in crates/mediaservo-weaknet/src/server.rs and this value
/// are **hand-synced** - the cluster startup chain always passes an explicit --listen; the two literals only matter when serve is started by hand.
pub const WEAKNET_SERVE_PORT: u16 = 9810;

/// SFU (mediasoup WebRtcServer) config.
///
/// Hand-written Default (do not revert to derive): an `Option<u16>` field is fine with derive(Default),
/// but the historical lesson is that deriving u16 numeric fields yields 0; hand-writing keeps it centralized,
/// pinned by the test `default_impl_matches_serde_absent`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SfuConfig {
    /// Announced outbound addresses (reachable IPs across NICs/networks, e.g. `["192.168.2.127", "10.144.0.3"]`).
    /// Priority: env `MEDIASERVO_SFU_ANNOUNCED_IP` > this field > auto-detection (public IP).
    /// mediasoup requires an announced address when binding 0.0.0.0 (PIT-44/58/138 - in-container detection is unreachable).
    #[serde(default)]
    pub announced_ips: Vec<String>,
    /// Listen port of the mediasoup WebRtcServer. None = env `MEDIASERVO_SFU_PORT` fallback -> 20000.
    #[serde(default)]
    pub port: Option<u16>,
    /// weaknet-serve panel port. None = 9810 (`WEAKNET_SERVE_PORT`). Only the port number changes; always loopback.
    #[serde(default)]
    pub weaknet_port: Option<u16>,
}

#[allow(clippy::derivable_impls)] // do not revert to derive - see the struct doc "hand-written Default" decision + pinned test
impl Default for SfuConfig {
    fn default() -> Self {
        Self { announced_ips: Vec::new(), port: None, weaknet_port: None }
    }
}

/// Web static layer (caddy) config - the `web:` section of server.yaml (single source of the init-rendered Caddyfile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebConfig {
    /// Public Caddy port. None = env `MEDIASERVO_WEB_PORT` fallback -> 8080.
    /// Option rather than a defaulted value: distinguishes "user wrote 8080" from "serde filled 8080", keeping the env fallback semantics alive.
    #[serde(default)]
    pub port: Option<u16>,
    /// Static site root. None = instance `web/` directory. Relative paths resolve against the instance root.
    #[serde(default)]
    pub static_root: Option<String>,
}

#[allow(clippy::derivable_impls)] // same as SfuConfig - hand-written Default by project decision
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

    /// Target bitrate in kbps - only affects the GStreamer capture/encode pipeline (pipeline.rs);
    /// the SFU main path (test_pattern -> WebRtcTrackSink -> libwebrtc encoder) does not run GStreamer encoding,
    /// so it has zero effect on the WebRTC encoder - use min/max_bitrate_kbps to control WebRTC encode bitrate.
    #[serde(default = "default_bitrate")]
    pub bitrate_kbps: u32,

    /// Keyframe interval in **seconds** (PIT-76: periodic request_key_frame trigger;
    /// semantics changed from GOP frame count to seconds; 0 treated as 1 as a guard).
    #[serde(default = "default_gop")]
    pub keyframe_interval: u32,

    /// Encode codec preference (v2, encoder-backend-codec-config): "auto"/"vp8"/"h264"/"vp9"/"av1".
    /// auto = router decides (current state); explicit value = only the target codec is declared when building the remote offer
    /// (libwebrtc actual encode = negotiated intersection = offer codec, the three agree).
    #[serde(default = "default_codec")]
    pub codec: String,
    /// Max bitrate (kbps) - v2 (encoder-bitrate): libwebrtc hard ceiling, None = unlimited
    pub max_bitrate_kbps: Option<u32>,
    /// Min bitrate (kbps) - v2 (encoder-bitrate): best-effort floor on constrained links, None = unlimited
    pub min_bitrate_kbps: Option<u32>,
}

impl EncoderConfig {
    /// Bitrate range validation (encoder-bitrate): when both are set, 0 < min < max is required,
    /// otherwise libwebrtc disables both (video_stream_encoder.cc:444 min>=max -> neither is applied).
    pub fn validate_bitrate(&self) -> Result<(), String> {
        if let (Some(min), Some(max)) = (self.min_bitrate_kbps, self.max_bitrate_kbps) {
            if min == 0 {
                return Err("encoder.min_bitrate_kbps must be > 0".into());
            }
            if min >= max {
                return Err(format!(
                    "encoder.min_bitrate_kbps({min}) must be < max_bitrate_kbps({max}), otherwise libwebrtc disables both"
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
    // PIT-76: semantics changed from frames to seconds (periodic keyframe trigger interval)
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
        assert_eq!(parsed.encoder.keyframe_interval, 2); // PIT-76: seconds
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
        // PIT-160 root fix: dev-exemption fields default false (production-safe);
        // absent keys parse clean on old configs (additive, serde default).
        assert!(!parsed.allow_dev_credentials);
        assert!(!parsed.allow_dev_enroll);

        let re_serialized = serde_yaml::to_string(&parsed).unwrap();
        let re_parsed: ServerConfig = serde_yaml::from_str(&re_serialized).unwrap();
        assert_eq!(re_parsed.listen.port, parsed.listen.port);
        assert_eq!(re_parsed.room_capacity, parsed.room_capacity);
    }

    #[test]
    fn server_config_dev_exemption_fields() {
        // config-first form of MEDIASERVO_ALLOW_DEV_CREDENTIALS / ALLOW_DEV_ENROLL envs —
        // survives tree rebuilds (etc/ never overwritten by deploy), env still overrides.
        let yaml = r#"
listen:
  host: "127.0.0.1"
  port: 9800
allow_dev_credentials: true
allow_dev_enroll: true
"#;
        let parsed: ServerConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.allow_dev_credentials);
        assert!(parsed.allow_dev_enroll);

        let re_serialized = serde_yaml::to_string(&parsed).unwrap();
        let re_parsed: ServerConfig = serde_yaml::from_str(&re_serialized).unwrap();
        assert!(re_parsed.allow_dev_credentials && re_parsed.allow_dev_enroll);
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
        assert_eq!(parsed.keyframe_interval, 2); // PIT-76: seconds
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
        // default (no min/max) -> Ok
        let cfg = EncoderConfig {
            backend: "auto".into(),
            bitrate_kbps: 2000,
            keyframe_interval: 2,
            codec: "h264".into(),
            max_bitrate_kbps: None,
            min_bitrate_kbps: None,
        };
        assert!(cfg.validate_bitrate().is_ok());

        // only max set -> Ok
        let cfg = EncoderConfig { max_bitrate_kbps: Some(4000), ..cfg.clone() };
        assert!(cfg.validate_bitrate().is_ok());

        // min < max → OK
        let cfg = EncoderConfig {
            min_bitrate_kbps: Some(500),
            max_bitrate_kbps: Some(4000),
            ..cfg.clone()
        };
        assert!(cfg.validate_bitrate().is_ok());

        // min >= max -> Err (guard against double-disabling)
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

    // ---- config-unification P1: web/sfu port keys (additive, zero change to old yaml) ----

    #[test]
    fn server_config_old_yaml_backward_compat() {
        // full pre-1.1 legacy yaml (including the old sfu shape) -> new keys all None = env/default fallback survives
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
        // roundtrip fully equal
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
        // absent -> ServerConfig-level default
        let parsed: ServerConfig = serde_yaml::from_str("listen: {}").unwrap();
        assert_eq!(parsed.web, WebConfig::default());
    }

    #[test]
    fn sfu_config_port_keys() {
        // both port keys parsed together
        let sfu: SfuConfig = serde_yaml::from_str("{port: 21000, weaknet_port: 9811}").unwrap();
        assert_eq!(sfu.port, Some(21000));
        assert_eq!(sfu.weaknet_port, Some(9811));
        // announced_ips present alone -> both ports still None (per-field defaults do not leak)
        let sfu: SfuConfig = serde_yaml::from_str("{announced_ips: [x]}").unwrap();
        assert_eq!(sfu.port, None);
        assert_eq!(sfu.weaknet_port, None);
    }

    #[test]
    fn default_impl_matches_serde_absent() {
        // guard against a future revert to derive(Default) reintroducing the port=0 regression
        let parsed: SfuConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(parsed, SfuConfig::default());
        assert_eq!(parsed.port, None);
        assert_eq!(parsed.weaknet_port, None);
        let parsed: WebConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(parsed, WebConfig::default());
    }
}
