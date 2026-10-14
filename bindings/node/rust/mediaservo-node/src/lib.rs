//! MediaServo Node.js bindings — napi-rs direct bindings to the Rust SDK (same shape as livekit rtc-ffi-bindings).
//!
//! Layers: mediaservo-{field,link,deck} (Rust async API) → napi .node binary → thin TS wrapper.
//! Event model: Rust broadcast/channel → napi ThreadsafeFunction → JS main-thread callback (livekit async_queue shape).

pub mod deck;
pub mod field;
pub mod link;

/// Shared event-pump runtime (sync napi methods have no tokio context — same global runtime pattern as field-c).
pub(crate) fn event_runtime() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("mediaservo-node event runtime")
    })
}

// Module exports (napi namespace merge)
pub use deck::*;
pub use field::*;
pub use link::*;
