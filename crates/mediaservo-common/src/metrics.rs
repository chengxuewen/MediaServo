//! Prometheus metrics helpers.
//!
//! Provides convenience functions to register standard metrics
//! used by all three components.
//!
//! 命名规范 (H4, doc-audit): 所有指标 `mediaservo_` 前缀 — Prometheus namespace
//! 惯例 + D209 改名一致 + 避免 host/server/client 三进程同名冲突。

use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;

/// Common metrics registry shared across all components.
pub struct CoreMetrics {
    registry: Registry,
    pub active_connections: Gauge,
    pub relayed_bytes: Counter<u64>,
    pub signaling_latency_us: Gauge,
    pub error_count: Counter<u64>,
    pub rooms_active: Gauge,
    pub component_status: Gauge,
}

impl CoreMetrics {
    /// Create a new metrics instance.
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let active_connections = Gauge::default();
        registry.register(
            "mediaservo_active_connections",
            "Number of active WebSocket/WebRTC connections",
            active_connections.clone(),
        );

        let relayed_bytes = Counter::default();
        registry.register(
            "mediaservo_relayed_bytes",
            "Total bytes relayed (Server only)",
            relayed_bytes.clone(),
        );

        let signaling_latency_us = Gauge::default();
        registry.register(
            "mediaservo_signaling_latency_us",
            "Signaling message latency in microseconds",
            signaling_latency_us.clone(),
        );

        let error_count = Counter::default();
        registry.register(
            "mediaservo_error_count",
            "Total error count by error code",
            error_count.clone(),
        );

        let rooms_active = Gauge::default();
        registry.register(
            "mediaservo_rooms_active",
            "Number of active rooms (Server only)",
            rooms_active.clone(),
        );

        let component_status = Gauge::default();
        registry.register(
            "mediaservo_component_status",
            "Component health status: 1=healthy, 0=unhealthy",
            component_status.clone(),
        );

        Self {
            registry,
            active_connections,
            relayed_bytes,
            signaling_latency_us,
            error_count,
            rooms_active,
            component_status,
        }
    }

    /// Encode all metrics in Prometheus text format.
    pub fn encode(&self) -> String {
        let mut buf = String::new();
        encode(&mut buf, &self.registry).expect("metrics encoding infallible");
        buf
    }
}

impl Default for CoreMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_metrics_encodes_valid_prometheus() {
        let metrics = CoreMetrics::new();
        let output = metrics.encode();
        // Prometheus text format should have HELP and TYPE for each metric
        assert!(
            output.contains("# HELP mediaservo_active_connections"),
            "missing HELP mediaservo_active_connections"
        );
        assert!(
            output.contains("# TYPE mediaservo_active_connections gauge"),
            "missing TYPE mediaservo_active_connections"
        );
        assert!(
            output.contains("# HELP mediaservo_relayed_bytes"),
            "missing HELP mediaservo_relayed_bytes"
        );
        assert!(
            output.contains("# TYPE mediaservo_relayed_bytes counter"),
            "missing TYPE mediaservo_relayed_bytes"
        );
        assert!(
            output.contains("# HELP mediaservo_signaling_latency_us"),
            "missing HELP mediaservo_signaling_latency_us"
        );
        assert!(
            output.contains("# TYPE mediaservo_signaling_latency_us gauge"),
            "missing TYPE mediaservo_signaling_latency_us"
        );
        assert!(
            output.contains("# HELP mediaservo_error_count"),
            "missing HELP mediaservo_error_count"
        );
        assert!(
            output.contains("# TYPE mediaservo_error_count counter"),
            "missing TYPE mediaservo_error_count"
        );
        assert!(
            output.contains("# HELP mediaservo_rooms_active"),
            "missing HELP mediaservo_rooms_active"
        );
        assert!(
            output.contains("# HELP mediaservo_component_status"),
            "missing HELP mediaservo_component_status"
        );
    }

    #[test]
    fn new_metrics_starts_at_zero() {
        let metrics = CoreMetrics::new();
        let output = metrics.encode();
        // All metrics should start at 0
        assert!(
            output.contains("mediaservo_active_connections 0"),
            "mediaservo_active_connections should be 0"
        );
        assert!(
            output.contains("mediaservo_relayed_bytes_total 0"),
            "mediaservo_relayed_bytes should be 0"
        );
        assert!(
            output.contains("mediaservo_signaling_latency_us 0"),
            "mediaservo_signaling_latency_us should be 0"
        );
        assert!(
            output.contains("mediaservo_error_count_total 0"),
            "mediaservo_error_count should be 0"
        );
        assert!(
            output.contains("mediaservo_rooms_active 0"),
            "mediaservo_rooms_active should be 0"
        );
        assert!(
            output.contains("mediaservo_component_status 0"),
            "mediaservo_component_status should be 0"
        );
    }

    #[test]
    fn counter_increment_reflected_in_encode() {
        let metrics = CoreMetrics::new();
        metrics.relayed_bytes.inc_by(1024);
        metrics.error_count.inc_by(3);
        let output = metrics.encode();
        assert!(
            output.contains("mediaservo_relayed_bytes_total 1024"),
            "expected mediaservo_relayed_bytes_total 1024"
        );
        assert!(
            output.contains("mediaservo_error_count_total 3"),
            "expected mediaservo_error_count_total 3"
        );
    }

    #[test]
    fn gauge_set_reflected_in_encode() {
        let metrics = CoreMetrics::new();
        metrics.active_connections.set(5);
        metrics.signaling_latency_us.set(1500);
        metrics.rooms_active.set(2);
        metrics.component_status.set(1);
        let output = metrics.encode();
        assert!(
            output.contains("mediaservo_active_connections 5"),
            "expected mediaservo_active_connections 5"
        );
        assert!(
            output.contains("mediaservo_signaling_latency_us 1500"),
            "expected mediaservo_signaling_latency_us 1500"
        );
        assert!(output.contains("mediaservo_rooms_active 2"), "expected mediaservo_rooms_active 2");
        assert!(
            output.contains("mediaservo_component_status 1"),
            "expected mediaservo_component_status 1"
        );
    }

    #[test]
    fn counter_multiple_increments() {
        let metrics = CoreMetrics::new();
        metrics.relayed_bytes.inc_by(100);
        metrics.relayed_bytes.inc_by(200);
        let output = metrics.encode();
        assert!(
            output.contains("mediaservo_relayed_bytes_total 300"),
            "expected mediaservo_relayed_bytes_total 300"
        );
    }

    #[test]
    fn default_impl_works() {
        let metrics: CoreMetrics = Default::default();
        let output = metrics.encode();
        assert!(output.contains("# HELP mediaservo_active_connections"));
        assert!(output.contains("mediaservo_active_connections 0"));
    }
}
