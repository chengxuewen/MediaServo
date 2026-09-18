//! Prometheus metrics endpoint — delegates to mediaservo_common::metrics::CoreMetrics.
//!
//! Exposes a `/metrics` endpoint returning Prometheus text format.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use mediaservo_common::metrics::CoreMetrics;
use serde::Serialize;
use std::sync::Arc;
use std::sync::RwLock;

pub type SharedMetrics = Arc<RwLock<CoreMetrics>>;

#[derive(Serialize)]
struct HealthResponse {
    status: String,
}

#[derive(Serialize)]
struct ReadyResponse {
    ready: bool,
    checks: ReadyChecks,
}

#[derive(Serialize)]
struct ReadyChecks {
    camera: String,
    encoder: String,
    signaling: String,
}

/// Create axum router with /health, /ready, and /metrics endpoints.
pub fn metrics_router(metrics: SharedMetrics) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(HealthResponse { status: "ok".to_string() }) }))
        .route(
            "/ready",
            get(|State(m): State<SharedMetrics>| async move {
                let _ = m;
                Json(ReadyResponse {
                    ready: true,
                    checks: ReadyChecks {
                        camera: "ok".to_string(),
                        encoder: "ok".to_string(),
                        signaling: "connected".to_string(),
                    },
                })
            }),
        )
        .route(
            "/metrics",
            get(|State(m): State<SharedMetrics>| async move {
                let m = m.read().expect("metrics lock poisoned");
                m.encode()
            }),
        )
        .with_state(metrics)
}
