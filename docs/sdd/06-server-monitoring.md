# SDD 06: Server Monitoring

## 1. Overview

Server monitoring and observability module. Provides health checks, Prometheus metrics, and alert rules.

**Decision refs**: D86-D91 (server architecture), D99 (tracing + prometheus), D-OPS-09 (alert rules), D111 (health check)

## 2. Interface Definition

```rust
/// Prometheus metric registry
pub struct ServerMetrics {
    // HTTP
    pub http_requests_total: Counter,
    pub http_request_duration_ms: Histogram,
    // WebSocket
    pub ws_connections: Gauge,
    pub ws_messages_total: Counter,
    // Relay
    pub relay_active_sessions: Gauge,
    pub relay_bitrate_kbps: Gauge,
    pub relay_rtt_ms: Gauge,
    pub relay_packets_lost: Counter,
    // System
    pub session_duration_seconds: Histogram,
    pub process_rss_mb: Gauge,
}

/// Health check
pub struct HealthResponse {
    pub status: String,       // "ok"
}

pub struct ReadyResponse {
    pub ready: bool,
    pub checks: HashMap<String, String>,  // component → status
}
```

### Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| /health | GET | K8s livenessProbe, lightweight liveness check |
| /ready  | GET | K8s readinessProbe, full component status |
| /metrics | GET | Prometheus scrape |

## 3. Metric Design (D99)

```
# HELP mediaservo_http_requests_total Total HTTP requests
# TYPE mediaservo_http_requests_total counter
mediaservo_http_requests_total{method="GET",path="/health"} 1024

# HELP mediaservo_ws_connections WebSocket connection count
# TYPE mediaservo_ws_connections gauge
mediaservo_ws_connections 2

# HELP mediaservo_relay_rtt_ms Relay RTT in milliseconds
# TYPE mediaservo_relay_rtt_ms gauge
mediaservo_relay_rtt_ms{peer="host-001"} 45
mediaservo_relay_rtt_ms{peer="remote-001"} 52
```

## 4. Alert Rules (D-OPS-09)

| Alert Name | Condition | Level |
|------------|-----------|-------|
| push_rtt_high | rtt_ms > 500 for 30s | warning |
| push_rtt_critical | rtt_ms > 1000 for 10s | critical |
| camera_lost | fps == 0 for 5s | critical |
| encoder_lag | encode_queue_depth > 5 for 10s | warning |
| gpu_mem_high | gpu_mem_pct > 90 for 30s | warning |
| signaling_lost | ws_connected == 0 for > 30s | critical |
| host_restart_loop | restart count > 5 in 5min | critical |
| frame_dropped_rate | control_frames_dropped / total > 0.1 | warning |

## 5. Logging (D99)

- Framework: tracing + tracing-subscriber
- Format: JSON stdout, collected via Docker logs
- traceId: auto-injected by axum TraceLayer
- Rotation: Docker json-file (max-size: 10m, max-file: 3)

## 6. Test Plan

| Test | Type | Description |
|------|------|-------------|
| metric_accuracy | Integration | Counter increments after HTTP requests |
| metric_persistence | Integration | /metrics returns Prometheus-format text |
| health_endpoint | Integration | /health returns 200 {"status":"ok"} |
| ready_all_green | Integration | /ready returns ready:true when all components are healthy |
| ready_partial_fail | Integration | /ready returns ready:false when a component is degraded |
| alert_firing | Integration | Simulated high RTT fires the push_rtt_high alert |
| alert_resolve | Integration | Alert auto-resolves after RTT recovers |
| ws_connection_gauge | Integration | WS connection gauge follows client connect/disconnect |
| tracing_json_format | Unit | Log output is valid JSON lines |
