//! host-controller: control process (Task F1 → S1 SFU-DC migration) — control channels over SFU SCTP.
//!
//! Usage: `host-controller [--gateway <local gateway ws url>] [--token <FrameBus token>]`
//! (default `ws://127.0.0.1:17980/ws`, D2 local gateway).
//!
//! Flow (since S1, replacing the old P2P offer/answer dead end — after the all-SFU decision,
//! Sdp/ICE of registered rooms is dropped by the server frame filter, so the cockpit answer never
//! arrives): SignalClient joins local room `control` via the local gateway (envelope wire, no PSK;
//! the vehicle PSK lives at the remote host-agent; the gateway intercepts and rewrites to the
//! vehicle room) → [`mediaservo_host::controller::control_loop`]: Send transport +
//! create_data_channel × labels (chassis/gimbal/light/ack, D-H3 reliability semantics) +
//! CreateDataProducer announce → Recv transport + NewDataProducer→ConsumeData → parse_envelope on
//! receipt → Actuator → ControlAck (ack DC main path + same-channel echo) → sidecar mirror to
//! FrameBus control/cmd, control/ack.
//!
//! Failure semantics (C15 + PIT-87 self-heal convention): signaling disconnect / ICE Failed /
//! setup failure → log and exit 1; deployment-side restart_policy=always relaunches.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use mediaservo_common::protocol::PeerRole;
use mediaservo_host::control::{Actuator, StubActuator};
use mediaservo_host::controller::{
    CommandPolicy as ControllerPolicy, ControllerConfig, control_loop,
};
use mediaservo_link::{FrameBus, SignalClient, TokenFile};

/// Local room name (gateway intercepts and rewrites to the vehicle room; the child's local room is only the downlink rewrite target).
const ROOM: &str = "control";
/// Local envelope src (identifies the gateway child process).
const SRC: &str = "host-controller";

const USAGE: &str = "Usage: host-controller [--gateway <local gateway ws url>] [--token <FrameBus token path>] \
    [--hmac-key-file <e-stop signing key file>]";

#[derive(Debug, Default)]
struct Args {
    gateway: Option<String>,
    /// FrameBus token (optional mirror surface: absent/failing only closes the bus sidecar, never affects execution).
    token: Option<PathBuf>,
    /// Emergency-stop HMAC verification key file (V5: rendered from [control].hmac_key_file;
    /// explicit > env MEDIASERVO_CONTROL_HMAC_KEY_FILE; read failure = early exit, no silent downgrade).
    hmac_key_file: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    args_from(std::env::args().skip(1))
}

/// Pure arg parsing (unit-testable): `--gateway <url>` / `--token <path>`; defaults to the local gateway (D2).
fn args_from(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = args.peekable();
    let mut out = Args::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gateway" => out.gateway = Some(args.next().ok_or("--gateway missing value")?),
            "--token" => {
                out.token = Some(PathBuf::from(args.next().ok_or("--token missing value")?))
            }
            "--hmac-key-file" => {
                out.hmac_key_file =
                    Some(PathBuf::from(args.next().ok_or("--hmac-key-file missing value")?))
            }
            _ => return Err(format!("unknown arg: {arg}\n{USAGE}")),
        }
    }
    Ok(out)
}

/// Token → FrameBus attach (command/ack mirror surface). Any failure only warns — the bus is a
/// sidecar and never blocks control execution (token issuance on the deploy side is additive;
/// no token = the pure DC path works as usual).
fn attach_bus(token: Option<&PathBuf>) -> Option<Arc<FrameBus>> {
    let Some(path) = token else {
        tracing::info!("no --token — FrameBus mirror surface off (control path unaffected)");
        return None;
    };
    let bus = (|| -> Result<FrameBus, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("read token {} failed: {e}", path.display()))?;
        let (verifying_key, cap) = TokenFile::decode(&bytes)
            .map_err(|e| format!("token {} invalid: {e}", path.display()))?;
        FrameBus::attach("", &cap, &verifying_key)
            .map_err(|e| format!("FrameBus attach failed: {e}"))
    })();
    match bus {
        Ok(b) => Some(Arc::new(b)),
        Err(e) => {
            tracing::warn!("{e} — FrameBus mirror surface off (control path unaffected)");
            None
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("controller");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let gateway = args.gateway.unwrap_or_else(|| "ws://127.0.0.1:17980/ws".to_string());

    // Signaling: via local gateway (D2 envelope wire; gateway intercepts RoomJoin and synthesizes RoomJoined)
    // S4′(knife 2): cold-start race — while the agent uplink WS has not reached the server, join
    // returns 5001 (the local gateway starts in seconds and is not at fault). Exponential-backoff
    // retry covers the server-cluster startup window (S1 booked knife; the old form = 5001×N
    // tripping the oxmgr breaker). Retries exhausted → exit 1, leave it to restart_policy.
    let retry = mediaservo_link::RetryConfig {
        max_retries: 12,
        base_delay: std::time::Duration::from_secs(1),
        max_delay: std::time::Duration::from_secs(8),
    };
    let signal = match SignalClient::new_gateway(&gateway, SRC, ROOM, PeerRole::Host)
        .connect_with_retry(retry)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("controller: signaling connect failed (retries exhausted): {e}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(room = %signal.room_id(), "controller joined local room");

    let bus = attach_bus(args.token.as_ref());
    let actuator: Arc<dyn Actuator> = Arc::new(StubActuator);
    // S4/a4·T3.5: e-stop verification policy (env-driven; vehicle and cockpit share the same key = deployment config surface).
    // V5: the explicit --hmac-key-file form (rendered from host.yaml [control]) wins over env;
    // key read failure = the security feature refuses silent downgrade — error + exit 1 to
    // restart_policy (same env discipline).
    let mut policy = ControllerPolicy::from_env();
    if let Some(f) = &args.hmac_key_file {
        match mediaservo_common::protocol::control_hmac_key_from_file(&f.to_string_lossy()) {
            Ok(k) => policy.hmac_key = Some(k),
            Err(e) => {
                tracing::error!(
                    "--hmac-key-file {} load failed: {e} — e-stop verification must not silently degrade",
                    f.display()
                );
                return ExitCode::from(1);
            }
        }
    }
    let code = control_loop(signal, ControllerConfig::default(), actuator, bus, policy).await;
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_from_override_wins() {
        let gw = args_from(vec!["--gateway".into(), "ws://127.0.0.1:18888/ws".into()].into_iter())
            .unwrap()
            .gateway
            .unwrap();
        assert_eq!(gw, "ws://127.0.0.1:18888/ws");
    }

    #[test]
    fn gateway_from_defaults_to_local_gateway() {
        // no args → default local gateway (D2); token defaults to None
        let args = args_from(vec![].into_iter()).unwrap();
        assert_eq!(
            args.gateway.unwrap_or_else(|| "ws://127.0.0.1:17980/ws".to_string()),
            "ws://127.0.0.1:17980/ws"
        );
        assert_eq!(args.token, None);
    }

    #[test]
    fn token_flag_parsed() {
        let args = args_from(
            vec!["--gateway".into(), "ws://h/ws".into(), "--token".into(), "t.bin".into()]
                .into_iter(),
        )
        .unwrap();
        assert_eq!(args.token, Some(PathBuf::from("t.bin")));
    }

    #[test]
    fn args_from_rejects_unknown_arg() {
        assert!(args_from(vec!["--bogus".into(), "x".into()].into_iter()).is_err());
    }

    #[test]
    fn args_from_requires_values() {
        assert!(args_from(vec!["--gateway".into()].into_iter()).is_err());
        assert!(args_from(vec!["--token".into()].into_iter()).is_err());
    }
}
