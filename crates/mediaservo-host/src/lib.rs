//! MediaServo Host library -- shared infrastructure for the multi-process architecture (Phase A skeleton).
//!
//! Currently provides only log initialization and a placeholder process runner; media/signaling
//! features (B/C) migrate here incrementally from host-legacy. The old single-process implementation
//! stays at `src/bin/host-legacy.rs` and is removed once the Phase C migration is done.
/// Unified config executor core (T1″ batch2, annex §2): ledger + flock + validate/token
/// gate + staging + dual-topology teardown gate + prepare/commit.
/// Batch4 routes the CLI/agent verbs through it; this batch ships the callable spine only.
pub mod config_core;
/// Control plane (Task F1): control envelope + actuator interface.
pub mod control;
/// Control plane over SFU-DC (S1): controller transport setup + command loop + bus mirroring.
pub mod controller;
/// Emergency-stop plane (Task F2): e-stop actuator latch + strong audit.
pub mod emergency;
/// host.yaml -> oxfile.toml translator (Task A2).
pub mod translate;
/// Initializes the logging system; role is the identifying field of the first log event.
pub fn init_logging(role: &str) {
    mediaservo_common::logging::init(mediaservo_common::logging::LoggingConfig::default());
    tracing::info!(role, "host process starting");
}
/// Device identity identity.json generation/loading (Task G4, D-H13).
pub mod conflict;
/// host-agent signaling gateway (Task D1).
pub mod gateway;
pub mod identity;

/// Topology monitoring (Task E1: declarative expectation vs discovered reality).
pub mod monitor;

/// H2 audio-meeting shared utilities (host-audio process + audio e2e): opus SDP / produce params / tone generation.
pub mod audio;

/// Placeholder runner: prints `<role> placeholder ready`, then blocks waiting for
/// SIGINT (Ctrl-C) or SIGTERM and returns `Ok(())` to exit cleanly once a signal arrives.
///
/// Phase A process shells use this as a placeholder only; real functionality replaces it phase by phase.
pub async fn run_placeholder(role: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("{role} placeholder ready");
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
    }
    Ok(())
}
