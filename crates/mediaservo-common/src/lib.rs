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

pub mod auth;
pub mod backup;
pub mod brand;
pub mod config;
pub mod error;
pub mod logging;
pub mod metrics;
pub mod protocol;

/// **Single source of truth** for deploy-side env help (msrtc.sh cats the same file at runtime; server/host -h embeds it at compile time).
/// `${...}` placeholders are resolved by env_usage_text, keeping the three key tables byte-identical (drift is structurally impossible).
pub const ENV_USAGE_MD: &str = include_str!("../assets/env-usage.md");

/// Render the env table: brand (env MEDIASERVO_BRAND > compile-time brand id), SFU media port (env > 20000).
pub fn env_usage_text() -> String {
    let brand =
        std::env::var("MEDIASERVO_BRAND").unwrap_or_else(|_| brand::media_brand().id.to_string());
    let port = std::env::var("MEDIASERVO_SFU_PORT")
        .unwrap_or_else(|_| config::DEFAULT_SFU_PORT.to_string());
    ENV_USAGE_MD.replace("${MEDIASERVO_BRAND}", &brand).replace("${MEDIASERVO_SFU_PORT}", &port)
}

#[cfg(test)]
mod deploy_help_tests {
    use super::*;

    #[test]
    fn env_usage_text_resolves_all_placeholders() {
        let t = env_usage_text();
        assert!(!t.contains("${"), "unresolved placeholder left: {t}");
        assert!(t.contains("ALLOW_DEV_ENROLL") && t.contains("MEDIASERVO_PSK"));
        assert!(t.contains("20000"));
    }
}
