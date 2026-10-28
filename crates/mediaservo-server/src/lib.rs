//! MediaServo Server library.
//!
//! Re-exports all server modules for binary and integration test use.

pub mod accounts;
pub mod admin;
pub mod apikeys;
pub mod audit;
pub mod config;
pub mod devices;
pub mod health;
pub mod monitor;
pub mod relay;
pub mod roles;
pub mod room;
pub mod rooms;
pub mod sfu;
pub mod signaling;
pub mod static_files;
pub mod status;
pub mod tls;

// Re-export key dependencies for integration tests
pub use axum;
pub use tokio;
