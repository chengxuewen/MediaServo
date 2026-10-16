//! host-agent: signaling gateway (Task D1, D-H6) -- each host process connects to the local port over WS,
//! the agent aggregates them onto a single WS to the server (one session per vehicle). Protocol semantics: [`mediaservo_host::gateway`].
//!
//! Usage: `host-agent [--port <local port>] [--remote <ws url>] [--psk <psk>] [--room <vehicle room>]`
//! Defaults: port 17980; remote/psk come from `SFU_E2E_WS_URL`/`SFU_E2E_PSK` (default
//! `ws://127.0.0.1:9800/ws` / `mediaservo-dev`, aligned with the streamer/e2e convention); room defaults to
//! `vehicle` (wired in through the host.yaml config since D3).
//!
//! E1-E3 monitoring (topology/data-flow/signaling + status reporting) is driven by [`spawn_status_reporter`]
//! as a single loop (since E3 it replaces the standalone E1/E2 loops; behavior and logs unchanged).
//!
//! T1b3 (k2/k11/EX-3): all three config bells (push-immediate, file-watch, boot) execute through
//! [`mediaservo_host::config_core`] -- the file write is the bell, the core is the executor.

use mediaservo_host::config_core::{
    self, ExecuteOpts, Outcome, Topology, WatchAction, file_hash, short12, watch_tick,
};
use mediaservo_host::gateway::{GatewayConfig, GatewayHandle, run_gateway};
use mediaservo_host::init_logging;
use mediaservo_host::monitor::signal::{STATUS_INTERVAL, spawn_status_reporter};
use mediaservo_link::{CapabilityToken, Ed25519VerifyingKey};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
const USAGE: &str = "Usage: host-agent [--port <local port>] [--remote <ws url>] [--psk <psk>] [--room <room>] [--config <host.yaml>] [--token <token file>]";
fn parse_args() -> Result<(GatewayConfig, Option<String>, Option<String>), String> {
    let mut cfg = GatewayConfig::default();
    let mut config_path: Option<String> = None;
    let mut token_path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                cfg.local_port = args
                    .next()
                    .ok_or("--port missing value")?
                    .parse()
                    .map_err(|e| format!("--port invalid: {e}"))?
            }
            "--remote" => cfg.remote_url = args.next().ok_or("--remote missing value")?,
            "--psk" => cfg.psk = args.next().ok_or("--psk missing value")?,
            "--room" => cfg.room = args.next().ok_or("--room missing value")?,
            "--config" => config_path = Some(args.next().ok_or("--config missing value")?),
            "--token" => token_path = Some(args.next().ok_or("--token missing value")?),
            _ => return Err(format!("unknown argument: {arg}\n{USAGE}")),
        }
    }
    // Environment variable defaults (aligned with the field/e2e_sfu external-server convention)
    if cfg.remote_url == "ws://127.0.0.1:9800/ws" {
        cfg.remote_url = std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| cfg.remote_url.clone());
    }
    if cfg.psk == "mediaservo-dev" {
        cfg.psk = std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| cfg.psk.clone());
    }
    Ok((cfg, config_path, token_path))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging("agent");
    let (mut cfg, config_path, token_path) = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // G4: the instance directory (--config <dir>/etc/host.yaml -> <dir>; basis for oxfile/backup/identity)
    let config_dir = config_path
        .as_deref()
        .map(PathBuf::from)
        .and_then(|p| p.parent().and_then(|etc| etc.parent().map(Path::to_path_buf)));
    // device-enroll T6: device identity = identity.json(device_id) + signing.pem (public key fingerprint).
    // When both are present, the remote Join goes through the signature-verification chain; missing identity.json / missing or corrupt PEM ->
    // warn and fall back to PSK only (behavior = the old version without identity.json).
    if let Some(dir) = &config_dir {
        match mediaservo_host::identity::load_device_identity(dir) {
            Ok(Some(ident)) => {
                tracing::info!(device_id = %ident.device_id, pubkey = %ident.pubkey_b64, "device identity loaded (public key fingerprint admission)");
                cfg.device = Some(ident);
            }
            Ok(None) => {
                tracing::warn!(path = %dir.join(mediaservo_host::identity::IDENTITY_FILE).display(), "device identity missing -- the remote connection uses PSK auth")
            }
            Err(e) => tracing::warn!(
                "{e} -- device identity unavailable, the remote connection uses PSK auth"
            ),
        }
    } else {
        tracing::warn!(
            "--config not provided (the instance directory cannot be derived) -- device identity not loaded, the remote connection uses PSK auth"
        );
    }
    // E1 topology monitoring: the expected state from host.yaml (empty by default -> fixed-process expectations only + a warning)
    let host_toml = match &config_path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %p, "failed to read host.yaml: {e} -- topology expectations are fixed processes only");
                String::new()
            }
        },
        None => {
            tracing::warn!(
                "--config not provided, topology expectations are fixed processes only (capturer/streamer expectations need host.yaml)"
            );
            String::new()
        }
    };
    // E1 review: the grace window starts at process startup (the main entry, including the slow gateway-connect window), not after the monitor task starts
    let monitor_started = std::time::Instant::now();
    let (port, gateway) = run_gateway(cfg).await.map_err(std::io::Error::other)?;
    tracing::info!(port, "host-agent gateway ready");
    // E2 data-flow monitoring token (default -> skipped; enabled once the deployed oxfile issues a Monitor-role token)
    let token: Option<(CapabilityToken, Ed25519VerifyingKey)> = match &token_path {
        Some(p) => match std::fs::read(p) {
            Ok(b) => match mediaservo_link::TokenFile::decode(&b) {
                Ok((vk, tok)) => Some((tok, vk)),
                Err(e) => {
                    tracing::warn!(path = %p, "invalid token: {e} -- data-flow monitoring skipped");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(path = %p, "failed to read token: {e} -- data-flow monitoring skipped");
                None
            }
        },
        None => {
            tracing::warn!(
                "--token not provided, data-flow monitoring skipped (topology monitoring is unaffected)"
            );
            None
        }
    };
    // E1-E3: topology + data-flow + signaling monitoring + StatusReport reporting (single loop, 5s)
    // E4: the config version (= the most recent ConfigPush.version after a successful apply; associated in StatusReport)
    // F1: restore from the etc/host.yaml.bak-<version> backup at startup -- the agent is watched by oxfile
    // and the version must not reset to zero after a restart (contract binding the applied on-disk version to the report).
    let config_version = Arc::new(AtomicU64::new(
        config_dir.as_deref().map(mediaservo_host::translate::recover_config_version).unwrap_or(0),
    ));
    // T1b3 (EX-3): boot config pass BEFORE the bells go live -- pending-commit fast path first,
    // full core only when the ledger disagrees with the on-disk fingerprint. Blocking (subprocess +
    // flock waits) => spawn_blocking even though startup is serial by construction.
    if let Some(dir) = &config_dir {
        let d = dir.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || boot_config_pass(&d)).await {
            tracing::error!("boot config pass join failed: {e}");
        }
    }
    // E4: the cloud config apply loop (polls the gateway every 500ms for a pending ConfigPush; the latest wins over older)
    // T1b3 (k2): plus the agent-owned file-watch bell for local edits (2s sampling, debounce).
    match &config_dir {
        Some(dir) => {
            spawn_config_applier(gateway.clone(), dir.clone(), config_version.clone());
            spawn_config_watcher(dir.clone());
        }
        None => tracing::warn!(
            "--config not provided (<dir>/etc/host.yaml cannot be derived) -- cloud config push disabled"
        ),
    }
    spawn_status_reporter(
        host_toml,
        monitor_started,
        token,
        gateway,
        STATUS_INTERVAL,
        config_version,
        config_dir,
    );
    wait_shutdown().await;
    Ok(())
}

/// Shared outcome audit for the three config bells (push / local / boot).
/// NoOp carries the "already current" meaning: the in-lock fingerprint re-check inside the core
/// is the correctness boundary, so a bell-level short-circuit is always honest to log.
fn log_core_outcome(bell: &str, o: &Outcome) {
    match o {
        Outcome::Applied { hash } => {
            tracing::info!(hash = short12(hash), "config-core ({bell}): applied")
        }
        Outcome::NoOp { hash } => {
            tracing::info!(hash = short12(hash), "config-core ({bell}): config already current")
        }
        Outcome::Refused { reason } => tracing::error!("config-core ({bell}): refused -- {reason}"),
        Outcome::Residue { survivors } => tracing::error!(
            "config-core ({bell}): refused-residue -- {} survivor(s) still alive: {survivors:?}",
            survivors.len()
        ),
    }
}

/// E4: the cloud config apply loop -- polls the gateway for a pending ConfigPush (500ms; the latest wins over older).
/// T1b3 (k2): the push-immediate bell runs the config-core instead of the raw `oxmgr_apply` bypass:
/// `handle_config_push` (validate -> backup -> atomic write) stays the BELL (the write is the fact),
/// the core is the EXECUTOR (teardown gate, staging, ledger). ConfigVersion advances ONLY on
/// Applied/NoOp -- a Refused/Residue keeps the old version in StatusReport so the cloud sees the
/// push did not land (honest reporting; ConfigPush stays one-way: the ack is the heartbeat echo).
fn spawn_config_applier(handle: GatewayHandle, dir: PathBuf, config_version: Arc<AtomicU64>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            let Some(push) = handle.take_config_push() else { continue };
            if let Ok(v) = mediaservo_host::translate::handle_config_push(
                &dir,
                config_version.load(Ordering::Relaxed),
                &push,
            ) {
                let dir2 = dir.clone();
                let res = tokio::task::spawn_blocking(move || {
                    config_core::execute(
                        &dir2,
                        Topology::AgentRun,
                        false,
                        &ExecuteOpts { origin: "push", actor: None },
                    )
                })
                .await;
                match res {
                    Ok(Ok(o)) => {
                        if matches!(o, Outcome::Applied { .. } | Outcome::NoOp { .. }) {
                            config_version.store(v, Ordering::Relaxed);
                            // T1b5 (k8): the cloud belief lands only when the push went live;
                            // refused pushes keep the previous last_push (honest conflict lamp).
                            if let Err(e) = config_core::record_push(&dir, None) {
                                tracing::error!("config-core (push): last_push record failed: {e}");
                            }
                        }
                        log_core_outcome("push", &o);
                    }
                    Ok(Err(e)) => tracing::error!(version = v, "config-core (push) failed: {e}"),
                    Err(e) => {
                        tracing::error!(version = v, "config-core (push) join failed: {e}")
                    }
                }
            } // the rejection reason is already logged as a warn audit by handle_config_push
        }
    });
}

/// T1b3 (k2): the file-watch bell -- local edits to etc/host.yaml are picked up without the
/// cloud (2s sampling, [`watch_tick`] debounce: two equal samples differing from the baseline).
/// On trigger the same AgentRun core executes (origin="local"); the core's flock serializes it
/// against the push bell. This is the agent side of "edit the file on the vehicle, then apply": with the
/// vendor oxmgr watch retired (T1b7), this bell is the sole local-edit path — the core owns every change
/// class (content and roster) via fingerprint-gated teardown+fresh.
fn spawn_config_watcher(dir: PathBuf) {
    tokio::spawn(async move {
        let cfg_path = dir.join("etc").join("host.yaml");
        let mut baseline = std::fs::read(&cfg_path).map(|b| file_hash(&b)).unwrap_or_default();
        let mut prev: Option<String> = None;
        let mut io_warned = false;
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            let cur = match std::fs::read(&cfg_path) {
                Ok(b) => Some(file_hash(&b)),
                Err(e) => {
                    if !io_warned {
                        tracing::warn!(path = %cfg_path.display(), "file-watch bell: host.yaml unreadable: {e} (warned once, keeps polling)");
                        io_warned = true;
                    }
                    None
                }
            };
            let next = cur.clone();
            let action = watch_tick(cur, std::mem::take(&mut prev), &baseline);
            prev = next;
            if let WatchAction::Trigger(hash) = action {
                baseline = hash;
                tracing::info!(
                    hash = short12(&baseline),
                    "file-watch bell: local host.yaml change confirmed"
                );
                let d = dir.clone();
                let res = tokio::task::spawn_blocking(move || {
                    config_core::execute(
                        &d,
                        Topology::AgentRun,
                        false,
                        &ExecuteOpts { origin: "local", actor: None },
                    )
                })
                .await;
                match res {
                    Ok(Ok(o)) => log_core_outcome("local", &o),
                    Ok(Err(e)) => tracing::error!("config-core (local) failed: {e}"),
                    Err(e) => tracing::error!("config-core (local) join failed: {e}"),
                }
            }
        }
    });
}

/// T1b3 (EX-3) / T1b4: the boot config pass -- now a thin logging shell over
/// [`config_core::startup_reconcile_with`] with `Topology::AgentRun` (the agent is alive:
/// the gate tolerates the agent-only registry, and the declarative apply swaps it last).
/// Fast path, fingerprint gate and the full core all live on the shared side -- the same
/// decision table the CLI startup wake uses, zero duplication.
fn boot_config_pass(dir: &Path) {
    match config_core::startup_reconcile_with(dir, Topology::AgentRun, "boot") {
        Ok(o) => log_core_outcome("boot", &o),
        Err(e) => tracing::warn!("config-core (boot) unavailable: {e}"),
    }
}

/// Wait for SIGINT/SIGTERM (the unix main path; other platforms only get ctrl_c).
async fn wait_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("ctrl_c handler");
    }
}
