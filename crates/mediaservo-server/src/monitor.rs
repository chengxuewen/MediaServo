use crate::signaling::SignalingServer;
use crate::health::{HealthChecker, HealthStatus, ReadinessChecker};
use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use mediaservo_common::metrics::CoreMetrics;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

// ── Metrics ──────────────────────────────────────────────────────────────────

pub type SharedMetrics = Arc<CoreMetrics>;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct StatsResponse {
    active_rooms: usize,
    connected_peers: usize,
    uptime_seconds: u64,
}

/// A single component's health report.
#[derive(Serialize)]
struct ComponentHealth {
    component: &'static str,
    #[serde(flatten)]
    status: HealthStatus,
}

/// Overall health response with per-component breakdown.
#[derive(Serialize)]
struct HealthResponse {
    overall: HealthStatus,
    components: Vec<ComponentHealth>,
    uptime_seconds: u64,
}

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct MonitorState {
    signaling: SignalingServer,
    metrics: SharedMetrics,
    start_time: Instant,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn monitor_router(signaling: SignalingServer) -> Router {
    let metrics = Arc::new(CoreMetrics::new());
    let start_time = Instant::now();

    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/stats", get(stats_handler))
        .route("/metrics", get(metrics_handler))
        .with_state(MonitorState {
            signaling,
            metrics,
            start_time,
        })
}

// ── Health check helpers ─────────────────────────────────────────────────────

/// Gather component health from all registered checkers.
fn gather_health(state: &MonitorState) -> Vec<ComponentHealth> {
    let checker: &dyn HealthChecker = &state.signaling;
    vec![ComponentHealth {
        component: checker.name(),
        status: checker.check_health(),
    }]
}

/// Gather readiness（worker 存活纳入——T7）：/ready 专用；/health 仍纯 liveness。
fn gather_readiness(state: &MonitorState) -> Vec<ComponentHealth> {
    let checker: &dyn ReadinessChecker = &state.signaling;
    vec![ComponentHealth {
        component: checker.name(),
        status: checker.check_readiness(),
    }]
}

/// Compute overall health: worst status wins (unhealthy > degraded > healthy).
fn overall_health(components: &[ComponentHealth]) -> HealthStatus {
    let mut worst = HealthStatus::Healthy;
    for c in components {
        match &c.status {
            HealthStatus::Unhealthy { .. } => return c.status.clone(),
            HealthStatus::Degraded { .. } => worst = c.status.clone(),
            HealthStatus::Healthy => {}
        }
    }
    worst
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// /health — full liveness probe with per-component status.
async fn health_handler(State(state): State<MonitorState>) -> Json<HealthResponse> {
    let components = gather_health(&state);
    let overall = overall_health(&components);
    let uptime_seconds = state.start_time.elapsed().as_secs();

    Json(HealthResponse {
        overall,
        components,
        uptime_seconds,
    })
}

/// /ready — startup readiness probe.
/// Returns 503 if any component is unhealthy.
async fn ready_handler(
    State(state): State<MonitorState>,
) -> (axum::http::StatusCode, Json<HealthResponse>) {
    let components = gather_readiness(&state);
    let overall = overall_health(&components);
    let uptime_seconds = state.start_time.elapsed().as_secs();

    let status = if overall.is_alive() {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(HealthResponse {
            overall,
            components,
            uptime_seconds,
        }),
    )
}

async fn stats_handler(State(state): State<MonitorState>) -> Json<StatsResponse> {
    let active_rooms = state.signaling.room_manager.active_rooms();
    let connected_peers = state.signaling.room_manager.get_peer_count();
    let uptime_seconds = state.start_time.elapsed().as_secs();

    Json(StatsResponse {
        active_rooms,
        connected_peers,
        uptime_seconds,
    })
}

async fn metrics_handler(State(state): State<MonitorState>) -> String {
    // Update gauges from live state
    let connected_peers = state.signaling.room_manager.get_peer_count() as i64;

    state
        .metrics
        .active_connections
        .set(connected_peers);

    // H4: 补注册的有数据源 gauge — rooms_active / component_status
    let rooms = state.signaling.room_manager.active_rooms() as i64;
    state.metrics.rooms_active.set(rooms);

    let health = state.signaling.check_health();
    state
        .metrics
        .component_status
        .set(if health.is_alive() { 1 } else { 0 });

    // Encode all metrics in Prometheus text format
    state.metrics.encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::Request;
    use http::StatusCode;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn health_returns_200_with_components() {
        let signaling = {
            #[cfg(feature = "sfu-mediasoup")]
            {
                let sfu = std::sync::Arc::new(
                crate::sfu::SfuManager::new_with_port(crate::sfu::random_udp_port())
                    .await
                    .unwrap());
                crate::signaling::SignalingServer::new(sfu, 65536, None)
            }
            #[cfg(not(feature = "sfu-mediasoup"))]
            {
                crate::signaling::SignalingServer::new(65536, None)
            }
        };
        let app = monitor_router(signaling);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Must have components array
        assert!(health.get("components").unwrap().as_array().unwrap().len() > 0);
        assert_eq!(health["overall"]["status"], "healthy");
    }

    #[tokio::test]
    async fn ready_returns_200_when_healthy() {
        let signaling = {
            #[cfg(feature = "sfu-mediasoup")]
            {
                let sfu = std::sync::Arc::new(
                crate::sfu::SfuManager::new_with_port(crate::sfu::random_udp_port())
                    .await
                    .unwrap());
                crate::signaling::SignalingServer::new(sfu, 65536, None)
            }
            #[cfg(not(feature = "sfu-mediasoup"))]
            {
                crate::signaling::SignalingServer::new(65536, None)
            }
        };
        let app = monitor_router(signaling);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
