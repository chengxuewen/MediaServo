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
    // E4: the cloud config apply loop (polls the gateway every 500ms for a pending ConfigPush; the latest wins over older)
    match &config_dir {
        Some(dir) => spawn_config_applier(gateway.clone(), dir.clone(), config_version.clone()),
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
    );
    wait_shutdown().await;
    Ok(())
}

/// E4: the cloud config apply loop -- polls the gateway for a pending ConfigPush (500ms; the latest wins over older).
/// A successful apply -> the version goes into StatusReport.config_version; a rejection -> handle_config_push already
/// logged the warn audit (C15); an oxmgr apply failure -> the error audit (the apply already hit disk, and the next process restart self-heals).
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
                config_version.store(v, Ordering::Relaxed);
                if let Err(e) = mediaservo_host::translate::oxmgr_apply(&dir) {
                    tracing::error!(version = v, "ConfigPush oxmgr apply failed: {e}");
                }
            } // the rejection reason is already logged as a warn audit by handle_config_push
        }
    });
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
