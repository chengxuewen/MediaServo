//! Health check infrastructure — trait and status types for component-level
//! health and readiness probes.

use serde::Serialize;

/// Health status of a component or the overall system.
///
/// Serialized with a `"status"` tag field so clients can pattern-match.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum HealthStatus {
    /// Component is operating normally.
    #[serde(rename = "healthy")]
    Healthy,
    /// Component is functional but degraded (e.g. high load, stale data).
    #[serde(rename = "degraded")]
    Degraded { reason: String },
    /// Component is unavailable or malfunctioning.
    #[serde(rename = "unhealthy")]
    Unhealthy { reason: String },
}

impl HealthStatus {
    /// A helper to construct a degraded status.
    pub fn degraded(reason: impl Into<String>) -> Self {
        Self::Degraded { reason: reason.into() }
    }

    /// A helper to construct an unhealthy status.
    pub fn unhealthy(reason: impl Into<String>) -> Self {
        Self::Unhealthy { reason: reason.into() }
    }

    /// True when the component is healthy or at least degraded (not fully down).
    pub fn is_alive(&self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded { .. })
    }
}

/// A named component that can report its own health.
///
/// Implementations must be `Send + Sync` so they can be shared across
/// axum handlers.
pub trait HealthChecker: Send + Sync {
    /// Human-readable name of this component (e.g. `"signaling"`).
    fn name(&self) -> &'static str;

    /// Return the current health of this component.
    fn check_health(&self) -> HealthStatus;
}

/// Extension of [`HealthChecker`] for readiness (startup-dependency) probes.
///
/// By default readiness is the same as health; override when a component
/// requires specific startup checks (e.g. waiting for a database connection).
pub trait ReadinessChecker: HealthChecker {
    /// Return the current readiness status. Defaults to [`HealthChecker::check_health`].
    fn check_readiness(&self) -> HealthStatus {
        self.check_health()
    }
}
