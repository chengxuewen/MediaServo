//! Structured JSON logging with trace_id propagation.
//!
//! Uses `tracing` + `tracing-subscriber` with JSON output. Every log event
//! includes a `trace_id` field from the current span for cross-service correlation.
//!
//! # Quick start
//! ```ignore
//! mediaservo_common::logging::init(mediaservo_common::logging::LoggingConfig::default());
//! tracing::info!("Server started");
//! ```

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fmt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// An opaque trace identifier — uuid v4 hex string.
///
/// Generated once per operation boundary (request, connection, pipeline run)
/// and propagated via `tracing::Span`. All log events inside that span
/// automatically carry the same `trace_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(String);

impl TraceId {
    /// Generate a new random trace identifier.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Return the `trace_id` from thread-local storage.
///
/// Use `set_current_trace_id` at entry points to populate.
pub fn current_trace_id() -> Option<TraceId> {
    get_current_trace_id()
}

/// Logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Default log level filter (e.g. "info", "debug", "warn").
    /// Can be overridden by `RUST_LOG` environment variable.
    #[serde(default = "default_level")]
    pub level: String,

    /// Output format: "json" (default) or "pretty" (human-readable).
    #[serde(default = "default_format")]
    pub format: LogFormat,

    /// Optional log file path. If set, logs are written to this file
    /// in addition to stderr. Rotated by external tooling.
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Pretty,
}

fn default_level() -> String {
    "info".into()
}
fn default_format() -> LogFormat {
    LogFormat::Json
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self { level: default_level(), format: default_format(), file: None }
    }
}

/// Initialize the tracing subscriber with JSON formatting.
///
/// Reads `RUST_LOG` env var as override for the configured level.
/// All spans with a `trace_id` field include it in structured output.
pub fn init(config: LoggingConfig) {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.level));

    match config.format {
        LogFormat::Json => {
            let layer = tracing_subscriber::fmt::layer()
                .json()
                .with_span_events(FmtSpan::CLOSE)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true);

            let subscriber = tracing_subscriber::registry().with(env_filter).with(layer);

            subscriber.init();
        }
        LogFormat::Pretty => {
            let layer = tracing_subscriber::fmt::layer()
                .pretty()
                .with_span_events(FmtSpan::CLOSE)
                .with_target(false)
                .with_thread_ids(false);

            let subscriber = tracing_subscriber::registry().with(env_filter).with(layer);

            subscriber.init();
        }
    }
}

/// Create a tracing span with a fresh `trace_id`.
///
/// Use at the boundary of each logical operation (request handler,
/// pipeline run, etc.).
///
/// # Example
/// ```ignore
/// let span = mediaservo_common::logging::trace_span("handle_request");
/// let _guard = span.enter();
/// // all logs inside here get the same trace_id
/// tracing::info!("Processing request");
/// ```
pub fn trace_span(name: &str) -> tracing::Span {
    let trace_id = TraceId::new();
    tracing::info_span!("operation", %name, trace_id = %trace_id)
}

thread_local! {
    static CURRENT_TRACE_ID: RefCell<Option<TraceId>> = const { RefCell::new(None) };
}

/// Set the trace ID for the current thread context.
pub fn set_current_trace_id(id: TraceId) {
    CURRENT_TRACE_ID.with(|cell| {
        *cell.borrow_mut() = Some(id);
    });
}

/// Get the trace ID for the current thread context.
pub fn get_current_trace_id() -> Option<TraceId> {
    CURRENT_TRACE_ID.with(|cell| cell.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_generation() {
        let a = TraceId::new();
        let b = TraceId::new();
        assert_ne!(a, b);
        assert_eq!(a.to_string().len(), 36); // uuid v4 standard length
    }

    #[test]
    fn trace_id_default() {
        let id = TraceId::default();
        assert_eq!(id.to_string().len(), 36);
    }

    #[test]
    fn trace_id_serde_roundtrip() {
        let id = TraceId::new();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: TraceId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn set_and_get_trace_id() {
        let id = TraceId::new();
        set_current_trace_id(id.clone());
        let got = get_current_trace_id();
        assert_eq!(got, Some(id));
    }

    #[test]
    fn logging_config_defaults() {
        let cfg = LoggingConfig::default();
        assert_eq!(cfg.level, "info");
        assert_eq!(cfg.format, LogFormat::Json);
        assert!(cfg.file.is_none());
    }

    #[test]
    fn logging_config_serde() {
        let yaml = "level: debug\nformat: pretty\nfile: /var/log/app.log";
        let cfg: LoggingConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.level, "debug");
        assert_eq!(cfg.format, LogFormat::Pretty);
        assert_eq!(cfg.file, Some("/var/log/app.log".into()));
    }

    #[test]
    fn trace_span_creates_valid_span() {
        let span = trace_span("test_op");
        let _guard = span.enter();
    }
}
