//! MediaServo shared abstractions.
//!
//! # Modules
//! - `config` — Host/Server/Remote YAML config schemas (serde)
//! - `error` — Unified error codes (1xxx–9xxx)
//! - `metrics` — Prometheus metrics helpers
//! - `protocol` — Signaling message types (WebSocket JSON)
//! - `auth` — PSK HMAC-SHA256 authentication trait
//! - `logging` — JSON structured logging with trace_id propagation
//! - `backup` — Atomic state backup/restore

pub mod config;
pub mod error;
pub mod metrics;
pub mod protocol;
pub mod auth;
pub mod logging;
pub mod backup;
pub mod brand;

/// 部署侧 env 帮助**单一真源**（msrtc.sh 运行时 cat 同一文件；server/host -h 编译期内嵌）。
/// `${...}` 占位符由 env_usage_text 解析，保证三面键表逐字节同源、漂移结构上不可能。
pub const ENV_USAGE_MD: &str = include_str!("../assets/env-usage.md");

/// 渲染 env 总表：品牌（env MEDIASERVO_BRAND > 编译期品牌 id）、SFU 媒体端口（env > 20000）。
pub fn env_usage_text() -> String {
    let brand = std::env::var("MEDIASERVO_BRAND")
        .unwrap_or_else(|_| brand::media_brand().id.to_string());
    let port =
        std::env::var("MEDIASERVO_SFU_PORT").unwrap_or_else(|_| "20000".to_string());
    ENV_USAGE_MD
        .replace("${MEDIASERVO_BRAND}", &brand)
        .replace("${MEDIASERVO_SFU_PORT}", &port)
}

#[cfg(test)]
mod deploy_help_tests {
    use super::*;

    #[test]
    fn env_usage_text_resolves_all_placeholders() {
        let t = env_usage_text();
        assert!(!t.contains("${"), "残留未解析占位符: {t}");
        assert!(t.contains("ALLOW_DEV_ENROLL") && t.contains("MEDIASERVO_PSK"));
        assert!(t.contains("20000"));
    }
}
