//! Task F2: host-emergency process e2e - independent PC + DC "emergency" + local fallback.
//!
//! Topology (production path, same shape as D2/D3; the sibling test of the F1 controller e2e):
//! ```text
//! host-agent (gateway process)     ──WS──▶ external mediasoup server (SFU_E2E_WS_URL)
//! host-controller (process)    ──WS──▶ host-agent    (3 DC: chassis/gimbal/light)
//! host-emergency (process)     ──WS──▶ host-agent    (1 DC: emergency)
//! mock cockpit (in-test, one independent answerer PC per offer) ──WS──▶ server (direct, PSK)
//! ```
//!
//! Assertions:
//! (1) two sequential negotiations complete through the gateway (controller first -> emergency second, both negotiators alive -
//!    stressing the gateway's p2p_owner single-slot ownership: a finished negotiation stops sending Sdp/ICE upstream -> safe ownership handover,
//!    see task-F2-report)
//! (2) emergency DC "stop" -> actuator fires + ack {latched:true}; repeat -> latched:false
//!    (one-shot latch semantics)
//! (3) emergency can still stop after the controller is SIGKILLed (crash) - proving the independent PC (D-H3)
//! (4) local fallback: SIGUSR1 -> the same actuator fires (source=local, network-independent, D-H3)
//! (5) strict audit JSONL: dc x2 + local x1, fields {ts, source, seq, cmd, latched, trigger_count}
//! (6) SIGTERM -> emergency + agent exit gracefully with 0
//!
//! Prerequisite: `SFU_E2E_WS_URL` pointing at an external mediasoup server (C21 pure external mode);
//! the room name is unique (vehicle-<pid>). The agent must join first (room-type guard, F1 lesson).

#![cfg(target_os = "linux")]

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{SignalClient, SignalEvent};
use mediaservo_webrtc::data_channel::{RTCDataChannel, RTCDataChannelEvent};
use mediaservo_webrtc::peer_connection::RTCIceCandidate;
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{RTCConfiguration, RTCPeerConnection, RTCPeerConnectionFactory};
use tokio::sync::mpsc;

fn ws_url() -> String {
    std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| {
        panic!(
            "SFU_E2E_WS_URL not set - emergency e2e needs an external mediasoup server (C21);
            e.g.: SFU_E2E_WS_URL=ws://127.0.0.1:9800/ws"
        )
    })
}

fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "mediaservo-dev".to_string())
}

/// C25: clean iceoryx2 residue (agent monitoring may touch FrameBus).
fn cleanup_iceoryx() {
    let _ = std::fs::remove_dir_all("/tmp/iceoryx2");
    if let Ok(entries) = std::fs::read_dir("/dev/shm") {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("iox2_") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn free_local_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("probe addr").port()
}

fn read_log(file: &tempfile::NamedTempFile) -> String {
    let mut out = String::new();
    file.reopen().expect("reopen log").read_to_string(&mut out).expect("read log");
    out
}

fn wait_for(log: &tempfile::NamedTempFile, needle: &str) {
    for _ in 0..30 {
        if read_log(log).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("needle {needle:?} not seen, log:\n{}", read_log(log));
}

// -- mock cockpit: direct server connect (PSK), one independent answerer PC per offer --
// F2's two sequential negotiations need two peer PCs (one PC cannot set_remote_description two offers):
// offer#1 (controller, 3 DC) -> answerer 0; offer#2 (emergency, 1 DC) -> answerer 1.
// The order is guaranteed by the test's timing (emergency is spawned only after all controller channels Open).

struct AnswererPc {
    pc: RTCPeerConnection,
    channels: Arc<Mutex<HashMap<String, RTCDataChannel>>>,
}

struct MockCockpit {
    answerers: Arc<Mutex<Vec<AnswererPc>>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockCockpit {
    async fn connect(url: &str, psk: &str, room: &str) -> Result<Self, String> {
        let signal = SignalClient::new(url, psk, room, PeerRole::Remote)
            .connect()
            .await
            .map_err(|e| format!("mock signaling connect failed: {e}"))?;

        let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidate>();
        // Texts of sent messages (the server's room broadcast echoes back to self -> dedupe by text)
        let sent: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        // answerers are created inside the task, shared via Arc for test-side checks
        let answerers: Arc<Mutex<Vec<AnswererPc>>> = Arc::new(Mutex::new(Vec::new()));
        let answerers_task = answerers.clone();

        let mut events = signal.events();
        let room_owned = room.to_string();
        let task = tokio::spawn(async move {
            let s = signal; // the session moves into the event task (SignalSession has no Clone)
            let answerers = answerers_task;
            // In-flight negotiation ownership = the most recently created answerer (under sequential negotiation ICE belongs only to the active negotiation)
            let mut active: Option<usize> = None;
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(SignalEvent::Message(m)) => {
                            let text = serde_json::to_string(&m).unwrap_or_default();
                            if sent.lock().unwrap().contains(&text) {
                                continue; // echo of one's own message
                            }
                            match m {
                                SignalingMessage::Sdp { sdp, .. } => {
                                    let desc: RTCSessionDescription =
                                        serde_json::from_str(&sdp).expect("mock parses Sdp JSON");
                                    if desc.sdp_type != RTCSdpType::Offer {
                                        continue;
                                    }
                                    // new offer -> new answerer PC (independent peer per negotiation)
                                    let factory = RTCPeerConnectionFactory::new();
                                    let pc = factory
                                        .create_peer_connection(RTCConfiguration::default())
                                        .await
                                        .expect("mock create_peer_connection");
                                    let channels: Arc<Mutex<HashMap<String, RTCDataChannel>>> =
                                        Arc::new(Mutex::new(HashMap::new()));
                                    let ch = channels.clone();
                                    pc.on_data_channel(move |dc| {
                                        ch.lock().unwrap().insert(dc.label().to_string(), dc);
                                    });
                                    let itx = ice_tx.clone();
                                    pc.on_ice_candidate(move |c| {
                                        let _ = itx.send(c);
                                    });
                                    pc.set_remote_description(&desc)
                                        .await
                                        .expect("mock set_remote_description");
                                    let answer = pc
                                        .create_answer(&Default::default())
                                        .await
                                        .expect("mock create_answer");
                                    pc.set_local_description(&answer)
                                        .await
                                        .expect("mock set_local_description");
                                    let json =
                                        serde_json::to_string(&answer).expect("serializes the answer");
                                    sent.lock().unwrap().insert(
                                        serde_json::to_string(&SignalingMessage::Sdp {
                                            room_id: room_owned.clone(),
                                            target: None,
                                            sdp: json.clone(),
                                        })
                                        .expect("serializes the Sdp message"),
                                    );
                                    s.send(SignalingMessage::Sdp {
                                        room_id: room_owned.clone(),
                                        target: None,
                                        sdp: json,
                                    })
                                    .await
                                    .expect("mock sends the answer");
                                    let mut v = answerers.lock().unwrap();
                                    v.push(AnswererPc { pc, channels });
                                    active = Some(v.len() - 1);
                                }
                                SignalingMessage::RTCIceCandidate {
                                    candidate,
                                    sdp_mid,
                                    sdp_mline_index,
                                    ..
                                } => {
                                    let c = RTCIceCandidate {
                                        candidate,
                                        sdp_mid,
                                        sdp_mline_index,
                                    };
                                    // Sequential negotiation: in-flight ICE belongs to the most recently created answerer
                                    // (once the previous negotiation completes there is no more ICE for it)
                                    match active {
                                        Some(idx) => {
                                            let pc = {
                                                let v = answerers.lock().unwrap();
                                                v[idx].pc.clone()
                                            };
                                            pc.add_ice_candidate(&c)
                                                .await
                                                .expect("mock add ice");
                                        }
                                        None => tracing::warn!("ICE with no active answerer, dropping"),
                                    }
                                }
                                _ => {}
                            }
                        }
                        Ok(SignalEvent::Disconnected { .. }) | Err(_) => break,
                        _ => {}
                    },
                    Some(c) = ice_rx.recv() => {
                        let msg = SignalingMessage::RTCIceCandidate {
                            room_id: room_owned.clone(),
                            target: None,
                            candidate: c.candidate,
                            sdp_mid: c.sdp_mid,
                            sdp_mline_index: c.sdp_mline_index,
                        };
                        sent.lock().unwrap().insert(
                            serde_json::to_string(&msg).expect("serializes ICE"),
                        );
                        s.send(msg).await.expect("mock sends ICE");
                    }
                }
            }
        });

        Ok(Self { answerers, task })
    }

    fn channel(&self, answerer: usize, label: &str) -> Option<RTCDataChannel> {
        self.answerers
            .lock()
            .unwrap()
            .get(answerer)
            .and_then(|a| a.channels.lock().unwrap().get(label).cloned())
    }

    fn answerer_channel_count(&self, answerer: usize) -> usize {
        self.answerers
            .lock()
            .unwrap()
            .get(answerer)
            .map(|a| a.channels.lock().unwrap().len())
            .unwrap_or(0)
    }

    async fn close(self) {
        self.task.abort();
        let pcs: Vec<_> = self.answerers.lock().unwrap().drain(..).map(|a| a.pc).collect();
        for pc in pcs {
            pc.close().await;
        }
    }
}

/// Wait for the answerer's label channel to appear and Open (polls state(), <=15s).
/// Note: the spool's Open event cannot be relied on - the channel may Open before the observer registers
/// (libwebrtc does not replay historical state), so polling state() is the reliable gate (F1 lesson).
async fn wait_channel_open(cockpit: &MockCockpit, answerer: usize, label: &str) -> RTCDataChannel {
    let dc = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(dc) = cockpit.channel(answerer, label) {
                return dc;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("answerer {answerer} channel {label} did not appear")
    .clone();
    tokio::time::timeout(Duration::from_secs(15), async {
        while dc.state() != mediaservo_webrtc::data_channel::RTCDataChannelState::Open {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("channel {label} not Open");
    dc
}

/// Send the envelope on the label channel and wait for the ACK on the same channel (ack seq pairing).
async fn send_and_await_ack(
    dc: &RTCDataChannel,
    seq: u64,
    cmd: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let mut rx = dc.spool().await;
    let env = serde_json::json!({ "seq": seq, "cmd": cmd, "payload": payload });
    dc.send_text(&env.to_string()).await.expect("mock sends the envelope");
    let deadline = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(ev) = rx.recv().await {
            if let RTCDataChannelEvent::Message(m) = ev {
                let v: serde_json::Value =
                    serde_json::from_slice(&m.data).unwrap_or(serde_json::Value::Null);
                if v["ack"] == serde_json::json!(seq) {
                    return v;
                }
            }
        }
        serde_json::Value::Null
    })
    .await
    .expect("seq {seq} ACK timeout");
    assert_ne!(deadline, serde_json::Value::Null, "no ACK for seq {seq}");
    deadline
}

fn audit_lines(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn wait_audit_lines(path: &Path, n: usize) -> Vec<serde_json::Value> {
    for _ in 0..30 {
        let v = audit_lines(path);
        if v.len() >= n {
            return v;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!(
        "the audit file did not reach {n} lines, got {}:\n{}",
        audit_lines(path).len(),
        std::fs::read_to_string(path).unwrap_or_default()
    );
}

#[tokio::test]
async fn emergency_p2p_dc_with_local_fallback() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let room = format!("vehicle-{pid}");
    let psk = psk();

    // host-agent (local gateway): random port + real remote server + unique room
    let gw_port = free_local_port();
    let agent_log = tempfile::NamedTempFile::new().expect("agent log");
    let mut agent = Command::new(env!("CARGO_BIN_EXE_host-agent"))
        .args([
            "--port",
            &gw_port.to_string(),
            "--remote",
            &ws_url(),
            "--psk",
            &psk,
            "--room",
            &room,
        ])
        .stdout(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .stderr(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .spawn()
        .expect("spawn host-agent");
    // Room-type guard: the agent (Host) must join first -> P2P room
    wait_for(&agent_log, "agent joined the vehicle room");

    // the mock cockpit connects to the server directly and joins (remote slot of the P2P room)
    let cockpit =
        MockCockpit::connect(&ws_url(), &psk, &room).await.expect("mock cockpit connects");

    // (1a) negotiation #1: host-controller (3 DC) - completes first to free p2p_owner for #2
    let ctrl_log = tempfile::NamedTempFile::new().expect("ctrl log");
    let mut controller = Command::new(env!("CARGO_BIN_EXE_host-controller"))
        .args(["--gateway", &format!("ws://127.0.0.1:{gw_port}/ws")])
        .stdout(Stdio::from(ctrl_log.reopen().expect("reopen ctrl log")))
        .stderr(Stdio::from(ctrl_log.reopen().expect("reopen ctrl log")))
        .spawn()
        .expect("spawn host-controller");
    wait_for(&ctrl_log, "controller ready");
    for label in ["chassis", "gimbal", "light"] {
        let dc = wait_channel_open(&cockpit, 0, label).await;
        assert_eq!(dc.label(), label);
    }
    assert_eq!(cockpit.answerer_channel_count(0), 3, "negotiation #1 = controller's 3 channels");

    // (1b) negotiation #2: host-emergency (1 DC) - both negotiators stay alive (p2p_owner pressure point)
    let audit_file = tempfile::NamedTempFile::new().expect("audit file");
    let em_log = tempfile::NamedTempFile::new().expect("emergency log");
    let mut emergency = Command::new(env!("CARGO_BIN_EXE_host-emergency"))
        .args([
            "--gateway",
            &format!("ws://127.0.0.1:{gw_port}/ws"),
            "--audit",
            audit_file.path().to_str().expect("audit path"),
        ])
        .stdout(Stdio::from(em_log.reopen().expect("reopen em log")))
        .stderr(Stdio::from(em_log.reopen().expect("reopen em log")))
        .spawn()
        .expect("spawn host-emergency");
    wait_for(&em_log, "emergency ready");
    let em_dc = wait_channel_open(&cockpit, 1, "emergency").await;
    assert_eq!(em_dc.label(), "emergency");
    assert_eq!(cockpit.answerer_channel_count(1), 1, "negotiation #2 = emergency's 1 channel");

    // (2) estop DC: first trigger -> latch armed (latched=true); repeat -> latched=false
    let ack1 = send_and_await_ack(&em_dc, 1, "stop", serde_json::json!({})).await;
    assert_eq!(ack1["ack"], 1);
    assert_eq!(ack1["result"]["ok"], true);
    assert_eq!(ack1["result"]["source"], "dc");
    assert_eq!(ack1["result"]["latched"], true, "the first trigger must arm the latch");
    assert_eq!(ack1["result"]["trigger_count"], 1);
    assert_eq!(ack1["result"]["audit"], "ok");
    eprintln!("[emergency_e2e] (1) first estop OK: {ack1}");

    let ack2 = send_and_await_ack(&em_dc, 2, "stop", serde_json::json!({})).await;
    assert_eq!(ack2["result"]["ok"], true);
    assert_eq!(ack2["result"]["latched"], false, "a repeated trigger must not re-arm");
    assert_eq!(ack2["result"]["trigger_count"], 2);
    eprintln!("[emergency_e2e] (2) repeated estop (latch held): {ack2}");

    // (3) a controller crash (SIGKILL) does not affect emergency (independent PC, D-H3)
    unsafe { libc::kill(controller.id() as i32, libc::SIGKILL) };
    controller.wait().expect("wait controller");
    eprintln!("[emergency_e2e] controller SIGKILLed, emergency keeps serving");

    let ack3 = send_and_await_ack(&em_dc, 3, "stop", serde_json::json!({})).await;
    assert_eq!(
        ack3["result"]["ok"], true,
        "emergency must still be able to stop after the controller crash"
    );
    assert_eq!(ack3["result"]["trigger_count"], 3);
    eprintln!("[emergency_e2e] (3) estop after controller crash OK: {ack3}");

    // (4) local fallback: SIGUSR1 (network-independent path) -> the same actuator
    unsafe { libc::kill(emergency.id() as i32, libc::SIGUSR1) };

    // (5) strict audit: 4 lines = dc(seq1,latched) / dc(seq2,latched-already) / dc(seq3,post-crash) / local(seq=null)
    let lines = wait_audit_lines(audit_file.path(), 4);
    assert_eq!(lines[0]["source"], "dc");
    assert_eq!(lines[0]["seq"], 1);
    assert_eq!(lines[0]["latched"], true);
    assert_eq!(lines[0]["trigger_count"], 1);
    assert_eq!(lines[0]["cmd"], "stop");
    assert!(lines[0]["ts"].as_str().is_some(), "the audit must carry a timestamp");
    assert_eq!(lines[1]["source"], "dc");
    assert_eq!(lines[1]["seq"], 2);
    assert_eq!(lines[1]["latched"], false);
    assert_eq!(lines[1]["trigger_count"], 2);
    assert_eq!(lines[2]["source"], "dc");
    assert_eq!(lines[2]["seq"], 3);
    assert_eq!(lines[2]["latched"], false);
    assert_eq!(lines[2]["trigger_count"], 3);
    assert_eq!(lines[3]["source"], "local");
    assert!(lines[3]["seq"].is_null(), "a local trigger has no envelope seq");
    assert_eq!(
        lines[3]["latched"], false,
        "the latch was already armed by the first 3 DC triggers - the local trigger truthfully records no re-arm"
    );
    assert_eq!(lines[3]["trigger_count"], 4);
    eprintln!("[emergency_e2e] (4) audit 4 lines: {lines:?}");

    // (6) SIGTERM -> emergency + agent exit gracefully with 0
    unsafe { libc::kill(emergency.id() as i32, libc::SIGTERM) };
    let et = emergency.wait().expect("wait emergency");
    assert_eq!(et.code(), Some(0), "emergency should exit gracefully with 0, got {et:?}");
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let at = agent.wait().expect("wait agent");
    assert_eq!(at.code(), Some(0), "agent should exit gracefully with 0, got {at:?}");

    cockpit.close().await;
}
