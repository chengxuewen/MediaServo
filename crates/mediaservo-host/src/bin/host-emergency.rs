//! host-emergency: emergency-stop process (Task F2) — dedicated PC + "emergency" DC + local fallback.
//!
//! Usage: `host-emergency [--gateway <local gateway ws url>] [--audit <audit file>]`
//! Defaults: gateway `ws://127.0.0.1:17980/ws` (D2 local gateway); audit file
//! `/tmp/mediaservo-emergency-audit.jsonl` (D-H11 strong-audit, local side).
//!
//! Structure = sibling process of host-controller (F1), three differences (task-F2-brief):
//! 1. Single DC label "emergency" (reliable-ordered — D-H3: emergency stop must be reliable + ordered)
//! 2. Sole command semantics: `{"seq":N,"cmd":"stop"}` → `EmergencyActuator::trigger`
//!    (one-shot latch by design: first trigger arms; repeats still audit + ack with latched=false)
//! 3. Local fallback (D-H3 network-independent): SIGUSR1 → same trigger() (source=local)
//!
//! Negotiation: **emergency is the offerer** (legacy webrtc_transport convention, same as
//! controller); separate PC from controller (D-H3: a PC crash must not affect emergency stop).
//! The gateway `p2p_owner` single-slot handover is safe under two **sequential** negotiations
//! (a completed negotiation no longer forwards Sdp/ICE upstream; concurrent negotiation is out
//! of F2 scope, see task-F2-report).
//!
//! Failure semantics (C15 + PIT-87 self-heal convention): signaling disconnect / ICE Failed /
//! session error → log and exit 1; deployment-side restart_policy=always relaunches.
//! Audit file unwritable → exit 2 at startup (cannot run without strong audit).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_host::control::{ControlAck, ControlEnvelope, parse_envelope};
use mediaservo_host::emergency::{
    EMERGENCY_STOP, EmergencyActuator, EmergencySource, StubEmergencyActuator,
};
use mediaservo_link::{SignalClient, SignalEvent};
use mediaservo_webrtc::data_channel::{RTCDataChannel, RTCDataChannelEvent, RTCDataChannelInit};
use mediaservo_webrtc::peer_connection::{RTCConfiguration, RTCIceCandidate};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{RTCPeerConnection, RTCPeerConnectionFactory};
use tokio::sync::mpsc;

/// Local room name (gateway intercepts and rewrites to the vehicle room; same room as controller, each with its own PC).
const ROOM: &str = "control";
/// Local envelope src (identifies the gateway child process).
const SRC: &str = "host-emergency";
const DEFAULT_GATEWAY: &str = "ws://127.0.0.1:17980/ws";
const DEFAULT_AUDIT: &str = "/tmp/mediaservo-emergency-audit.jsonl";
/// Wait before self-heal exit on ICE Failed (same as PIT-87: exit after state settles, restart relaunches).
const ICE_FAILED_WAIT: Duration = Duration::from_secs(1);

const USAGE: &str =
    "Usage: host-emergency [--gateway <local gateway ws url>] [--audit <audit file>]";

/// Pure arg parsing (unit-testable): `--gateway <url>` / `--audit <path>`, both with defaults.
fn args_from(args: impl Iterator<Item = String>) -> Result<(String, PathBuf), String> {
    let mut args = args.peekable();
    let mut gateway: Option<String> = None;
    let mut audit: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gateway" => gateway = Some(args.next().ok_or("--gateway missing value")?),
            "--audit" => audit = Some(args.next().ok_or("--audit missing value")?.into()),
            _ => return Err(format!("unknown arg: {arg}\n{USAGE}")),
        }
    }
    Ok((
        gateway.unwrap_or_else(|| DEFAULT_GATEWAY.to_string()),
        audit.unwrap_or_else(|| PathBuf::from(DEFAULT_AUDIT)),
    ))
}

fn parse_args() -> Result<(String, PathBuf), String> {
    args_from(std::env::args().skip(1))
}

/// Wait for SIGINT/SIGTERM (unix main path; other platforms only ctrl_c).
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// DC envelope → emergency actuator → ACK (pure function, unit-testable).
/// Sole command semantics: `cmd == EMERGENCY_STOP` → trigger; anything else rejected (Err ack).
fn handle_envelope(actuator: &dyn EmergencyActuator, env: &ControlEnvelope) -> ControlAck {
    if env.cmd != EMERGENCY_STOP {
        return ControlAck::err(
            env.seq,
            format!("unknown cmd {:?} (only {EMERGENCY_STOP:?} accepted)", env.cmd),
        );
    }
    match actuator.trigger(EmergencySource::Dc, Some(env.seq)) {
        Ok(t) => {
            let audit = match &t.audit {
                Ok(()) => serde_json::json!("ok"),
                Err(e) => serde_json::json!({ "error": e }),
            };
            ControlAck::ok(
                env.seq,
                serde_json::json!({
                    "ok": true,
                    "source": "dc",
                    "latched": t.latched,
                    "trigger_count": t.trigger_count,
                    "audit": audit,
                }),
            )
        }
        Err(e) => ControlAck::err(env.seq, e),
    }
}

/// Single-channel route: spool receive → parse envelope → handle_envelope → ACK on the same channel.
async fn route_channel(dc: RTCDataChannel, actuator: Arc<dyn EmergencyActuator>) {
    let label = dc.label().to_string();
    let mut rx = dc.spool().await;
    tracing::info!(label, "DC route started");
    while let Some(ev) = rx.recv().await {
        match ev {
            RTCDataChannelEvent::Open => tracing::info!(label, "DC open"),
            RTCDataChannelEvent::Closed => {
                tracing::info!(label, "DC closed");
                break;
            }
            RTCDataChannelEvent::Error(e) => tracing::warn!(label, "DC error: {e}"),
            RTCDataChannelEvent::Message(m) => {
                let env = match parse_envelope(&m.data) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(label, len = m.data.len(), "envelope parse failed: {e}");
                        continue;
                    }
                };
                let ack = handle_envelope(actuator.as_ref(), &env);
                if let Ok(json) = serde_json::to_string(&ack)
                    && let Err(e) = dc.send_text(&json).await
                {
                    tracing::warn!(label, seq = env.seq, "ACK send failed: {e}");
                }
            }
        }
    }
}

/// Build a DC-only PC: register callbacks, create the "emergency" channel (reliable-ordered), send the offer.
async fn setup_pc(
    pc: &RTCPeerConnection,
    actuator: Arc<dyn EmergencyActuator>,
    ice_tx: mpsc::UnboundedSender<RTCIceCandidate>,
) -> Result<(), String> {
    // Peer-created channel (not expected on the offerer path; defensive routing)
    let a = actuator.clone();
    pc.on_data_channel(move |dc| {
        tokio::spawn(route_channel(dc, a.clone()));
    });
    pc.on_ice_candidate(move |candidate| {
        let _ = ice_tx.send(candidate);
    });
    let dc = pc
        .create_data_channel("emergency", RTCDataChannelInit::default())
        .await
        .map_err(|e| format!("create_data_channel emergency: {e}"))?;
    tokio::spawn(route_channel(dc, actuator.clone()));
    Ok(())
}

/// Local fallback (D-H3 network-independent): SIGUSR1 → same EmergencyActuator (source=local).
async fn local_trigger(actuator: &dyn EmergencyActuator) {
    match actuator.trigger(EmergencySource::Local, None) {
        Ok(t) => tracing::info!(
            latched = t.latched,
            trigger_count = t.trigger_count,
            "local fallback emergency stop triggered (SIGUSR1)"
        ),
        // C15: error branches must log
        Err(e) => tracing::error!("local emergency actuator failed: {e}"),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("emergency");
    let (gateway, audit_path) = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    // Strong audit (D-H11 local side): file unavailable = cannot run (fail at startup)
    let actuator: Arc<dyn EmergencyActuator> = match StubEmergencyActuator::new(&audit_path) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("emergency: {e}");
            return ExitCode::from(2);
        }
    };
    tracing::info!(audit = %audit_path.display(), "strong-audit file ready");

    // Register the local fallback signal early — SIGUSR1 arriving before handler registration = default terminate
    #[cfg(unix)]
    let mut usr1 =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("emergency: SIGUSR1 registration failed: {e}");
                return ExitCode::from(1);
            }
        };

    // Signaling: via local gateway (D2 envelope wire; gateway intercepts RoomJoin and synthesizes RoomJoined)
    let signal =
        match SignalClient::new_gateway(&gateway, SRC, ROOM, PeerRole::Host).connect().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("emergency: signaling connect failed: {e}");
                return ExitCode::from(1);
            }
        };
    tracing::info!(room = %signal.room_id(), "emergency joined local room");

    // DC-only PC (no tracks) — independent of controller's PC (D-H3)
    let factory = RTCPeerConnectionFactory::new();
    let pc = match factory.create_peer_connection(RTCConfiguration::default()).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("emergency: create_peer_connection failed: {e}");
            return ExitCode::from(1);
        }
    };

    // ICE candidate uplink channel (sync callback thread → async send in main loop)
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidate>();
    if let Err(e) = setup_pc(&pc, actuator.clone(), ice_tx).await {
        eprintln!("emergency: {e}");
        return ExitCode::from(1);
    }

    // ICE Failed self-heal (PIT-87 pattern: exit after state settles, wait for restart)
    let (fail_tx, mut fail_rx) = mpsc::unbounded_channel::<()>();
    pc.on_ice_connection_state_change(move |state| {
        if state == mediaservo_webrtc::peer_connection::RTCIceConnectionState::Failed {
            tracing::error!("ICE Failed — exiting for restart (PIT-87 self-heal)");
            let _ = fail_tx.send(());
        }
    });

    // Standard offerer negotiation (legacy convention: host initiates the offer)
    let offer = match pc.create_offer(&Default::default()).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("emergency: create_offer failed: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = pc.set_local_description(&offer).await {
        eprintln!("emergency: set_local_description failed: {e}");
        return ExitCode::from(1);
    }
    let offer_json = match serde_json::to_string(&offer) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("emergency: serialize offer failed: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = signal
        .send(SignalingMessage::Sdp { room_id: ROOM.into(), target: None, sdp: offer_json })
        .await
    {
        eprintln!("emergency: send offer failed: {e}");
        return ExitCode::from(1);
    }
    tracing::info!("offer sent (awaiting cockpit answer)");

    // Buffer remote candidates until the answer lands (libwebrtc add_ice_candidate before negotiation is unreliable)
    let mut pending_ice: Vec<RTCIceCandidate> = Vec::new();
    let mut remote_set = false;
    let mut signal_events = signal.events();
    let mut exit_code: u8 = 0;

    println!(
        "emergency ready: gateway={gateway} room={ROOM} label=emergency audit={}",
        audit_path.display()
    );

    'run: loop {
        tokio::select! {
            sig = shutdown_signal() => match sig {
                Ok(()) => break 'run,
                Err(e) => {
                    eprintln!("emergency: signal handling failed: {e}");
                    exit_code = 1;
                    break 'run;
                }
            },
            _ = fail_rx.recv() => {
                // ICE Failed: brief wait for state to settle, then exit (PIT-87)
                tokio::time::sleep(ICE_FAILED_WAIT).await;
                exit_code = 1;
                break 'run;
            }
            ev = signal_events.recv() => match ev {
                Ok(SignalEvent::Message(SignalingMessage::Sdp { sdp, .. })) => {
                    let desc = match serde_json::from_str::<RTCSessionDescription>(&sdp) {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!("answer parse failed: {e}");
                            continue;
                        }
                    };
                    if desc.sdp_type != RTCSdpType::Answer {
                        tracing::warn!(sdp_type = %desc.sdp_type, "non-answer Sdp, ignored");
                        continue;
                    }
                    match pc.set_remote_description(&desc).await {
                        Ok(()) => {
                            tracing::info!("answer set — P2P negotiation complete");
                            remote_set = true;
                            for c in pending_ice.drain(..) {
                                if let Err(e) = pc.add_ice_candidate(&c).await {
                                    tracing::warn!("add_ice_candidate: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("set_remote_description failed: {e}");
                            exit_code = 1;
                            break 'run;
                        }
                    }
                }
                Ok(SignalEvent::Message(SignalingMessage::RTCIceCandidate {
                    candidate, sdp_mid, sdp_mline_index, ..
                })) => {
                    let c = RTCIceCandidate {
                        candidate,
                        sdp_mid,
                        sdp_mline_index,
                    };
                    if remote_set {
                        if let Err(e) = pc.add_ice_candidate(&c).await {
                            tracing::warn!("add_ice_candidate: {e}");
                        }
                    } else {
                        pending_ice.push(c);
                    }
                }
                Ok(SignalEvent::Message(_)) => {} // RoomJoined/others: passthrough ignored
                Ok(SignalEvent::Error(e)) => {
                    tracing::error!("signaling error: {e}");
                    exit_code = 1;
                    break 'run;
                }
                Ok(SignalEvent::Disconnected { reason }) => {
                    tracing::error!("signaling disconnected: {reason} — exiting for restart");
                    exit_code = 1;
                    break 'run;
                }
                Ok(SignalEvent::Connected { .. }) => {}
                // SignalEvent non_exhaustive catch-all
                Ok(_) => {}
                Err(_) => {
                    tracing::error!("signaling event stream closed");
                    exit_code = 1;
                    break 'run;
                }
            },
            Some(candidate) = ice_rx.recv() => {
                if let Err(e) = signal
                    .send(SignalingMessage::RTCIceCandidate {
                        room_id: ROOM.into(),
                        target: None,
                        candidate: candidate.candidate,
                        sdp_mid: candidate.sdp_mid,
                        sdp_mline_index: candidate.sdp_mline_index,
                    })
                    .await
                {
                    tracing::warn!("ICE candidate uplink failed: {e}");
                }
            }
            _ = async {
                // Local fallback (SIGUSR1); non-unix platforms have no local fallback path (DC stop only)
                #[cfg(unix)]
                usr1.recv().await;
                #[cfg(not(unix))]
                std::future::pending::<()>().await;
            } => {
                local_trigger(actuator.as_ref()).await;
            }
        }
    }

    if let Err(e) = signal.close().await {
        tracing::warn!("close: {e}");
    }
    pc.close().await;
    tracing::info!("emergency stopped (exit={exit_code})");
    ExitCode::from(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_host::emergency::EmergencyTrigger;
    use std::path::Path;

    fn actuator(temp: &tempfile::NamedTempFile) -> StubEmergencyActuator {
        StubEmergencyActuator::new(temp.path()).unwrap()
    }

    #[test]
    fn args_from_defaults() {
        let (gw, audit) = args_from(vec![].into_iter()).unwrap();
        assert_eq!(gw, DEFAULT_GATEWAY);
        assert_eq!(audit, PathBuf::from(DEFAULT_AUDIT));
    }

    #[test]
    fn args_from_override_wins() {
        let (gw, audit) = args_from(
            vec![
                "--gateway".into(),
                "ws://127.0.0.1:18888/ws".into(),
                "--audit".into(),
                "/tmp/x.jsonl".into(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(gw, "ws://127.0.0.1:18888/ws");
        assert_eq!(audit, PathBuf::from("/tmp/x.jsonl"));
    }

    #[test]
    fn args_from_rejects_unknown_and_missing() {
        assert!(args_from(vec!["--bogus".into()].into_iter()).is_err());
        assert!(args_from(vec!["--gateway".into()].into_iter()).is_err());
        assert!(args_from(vec!["--audit".into()].into_iter()).is_err());
    }

    #[test]
    fn handle_envelope_rejects_non_stop_cmd() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = actuator(&f);
        let env = ControlEnvelope {
            seq: 9,
            cmd: "panic".into(),
            payload: serde_json::json!({}),
            sig: None,
        };
        let ack = handle_envelope(&a, &env);
        assert_eq!(ack.ack, 9);
        let err = ack.result["error"].as_str().expect("rejection must carry error");
        assert!(err.contains("unknown cmd"), "err: {err}");
        assert_eq!(
            std::fs::read_to_string(f.path()).unwrap(),
            "",
            "rejected commands must not trigger/audit"
        );
    }

    #[test]
    fn handle_envelope_stop_first_arms_then_holds() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = actuator(&f);
        let env = |seq| ControlEnvelope {
            seq,
            cmd: EMERGENCY_STOP.into(),
            payload: serde_json::json!({}),
            sig: None,
        };
        let ack1 = handle_envelope(&a, &env(1));
        assert_eq!(ack1.result["ok"], true);
        assert_eq!(ack1.result["source"], "dc");
        assert_eq!(ack1.result["latched"], true, "first trigger must arm");
        assert_eq!(ack1.result["trigger_count"], 1);
        assert_eq!(ack1.result["audit"], "ok");
        let ack2 = handle_envelope(&a, &env(2));
        assert_eq!(ack2.result["ok"], true);
        assert_eq!(ack2.result["latched"], false, "repeats must not re-arm");
        assert_eq!(ack2.result["trigger_count"], 2);
    }

    #[test]
    fn audit_failure_surfaces_in_ack() {
        // Unwritable audit (/dev/full, Linux) → ack audit carries error, but the stop is still ok
        #[cfg(target_os = "linux")]
        {
            let a = StubEmergencyActuator::new(Path::new("/dev/full")).unwrap();
            let env = ControlEnvelope {
                seq: 1,
                cmd: EMERGENCY_STOP.into(),
                payload: serde_json::json!({}),
                sig: None,
            };
            let ack = handle_envelope(&a, &env);
            assert_eq!(ack.result["ok"], true, "the stop itself is unaffected by audit failure");
            assert!(ack.result["audit"]["error"].is_string(), "audit error must surface in ack");
        }
    }

    #[test]
    fn local_trigger_uses_same_latch() {
        // Local fallback shares the same latch as DC (identical EmergencyTrigger shape)
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = actuator(&f);
        let t: EmergencyTrigger = a.trigger(EmergencySource::Local, None).unwrap();
        assert!(t.latched);
        assert_eq!(t.trigger_count, 1);
    }
}
