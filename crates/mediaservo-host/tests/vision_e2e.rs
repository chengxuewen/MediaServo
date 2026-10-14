//! Task F3: streamer vision DC e2e - external publisher -> FrameBus vision topic -> streamer
//! transport B (independent pure-DC PC, label "vision") -> mock cockpit.
//!
//! Topology (production path, D-H6/D-H8):
//! ```text
//! host-agent (gateway process)           ──WS──▶ external mediasoup server (SFU_E2E_WS_URL)
//! host-capturer (process)            ──FrameBus camera/<cam>──▶ host-streamer (process)
//! vision publisher (in-test, Perception role, D-H7 external-node simulation)
//!                                ──FrameBus vision/<cam>──▶ host-streamer
//! host-streamer transport A (video, SFU produce) ──▶ server (the original C2 chain unchanged)
//! host-streamer transport B (pure DC "vision", P2P relay) ──▶ mock cockpit answerer
//! mock cockpit (in-test)              ──WS direct to server (PSK, PeerRole::Remote)
//! ```
//!
//! Assertions:
//! (1) transport separation (D-H8): the vision offer SDP has exactly 1 m-line (m=application,
//!    no video m-line -> vision is not on the video PC); the answerer has no RTP sender and exactly 1 channel labeled "vision"
//! (2) transparent forwarding: the text received on the DC == the publisher payload verbatim (streamer = pipe, no re-encoding),
//!    with frame correlation fields frame.seq / frame.ts_mono_ns and objects (class/confidence/bbox/text/color) all present
//! (3) video unaffected (transport A): streamer stats bytes_sent>0 and frames_encoded>0
//! (4) D-H7 external-node verification gap: a Perception-role token publishes vision/* (ACL passthrough proven)
//! (5) SIGTERM -> streamer + capturer + agent exit gracefully with 0
//!
//! Prerequisite: `SFU_E2E_WS_URL` pointing at an external mediasoup server (C21 pure external mode);
//! C25: clean `/tmp/iceoryx2` + `/dev/shm/iox2_*` before running.

#![cfg(target_os = "linux")]

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role, SignalClient, SignalEvent, TokenFile,
};
use mediaservo_webrtc::data_channel::{RTCDataChannel, RTCDataChannelEvent};
use mediaservo_webrtc::peer_connection::RTCIceCandidate;
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{RTCConfiguration, RTCPeerConnection, RTCPeerConnectionFactory};
use tokio::sync::mpsc;

// Test Ed25519 keypair (generated with openssl, test-only; same source as link/deck/capturer tests).
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: clean iceoryx2 0.9.3 runtime residue (/tmp/iceoryx2 + /dev/shm/iox2_*).
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

fn ws_url() -> String {
    std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| {
        panic!(
            "SFU_E2E_WS_URL not set - vision e2e needs an external mediasoup server (C21);
            e.g.: SFU_E2E_WS_URL=ws://127.0.0.1:9800/ws"
        )
    })
}

fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "mediaservo-dev".to_string())
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
    for _ in 0..20 {
        if read_log(log).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("needle {needle:?} not seen, log:\n{}", read_log(log));
}

/// Poll the log until the stats line has bytes_sent>0 and frames_encoded>0 (<=30s, D4 evidence mode).
fn wait_for_flow(log: &tempfile::NamedTempFile) -> String {
    for _ in 0..60 {
        for line in read_log(log).lines() {
            let Some(rest) = line.split("streamer stats:").nth(1) else {
                continue;
            };
            let bytes: u64 = rest
                .split_whitespace()
                .find_map(|t| t.strip_prefix("bytes_sent="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let frames: u64 = rest
                .split_whitespace()
                .find_map(|t| t.strip_prefix("frames_encoded="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if bytes > 0 && frames > 0 {
                return format!("bytes_sent={bytes} frames_encoded={frames}");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("no stream evidence within 30s, log:\n{}", read_log(log));
}

// -- mock cockpit: direct server connect (PSK), one independent answerer PC per offer --
// F3 has only one P2P offer (streamer transport B "vision") -> answerer 0.
// Also capture the received offer SDP verbatim (the m-line assertion = transport-separation evidence).

struct AnswererPc {
    pc: RTCPeerConnection,
    channels: Arc<Mutex<HashMap<String, RTCDataChannel>>>,
}

struct MockCockpit {
    answerers: Arc<Mutex<Vec<AnswererPc>>>,
    /// The received offer SDP verbatim (F3 assertion: exactly 1 application m-line).
    offers: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockCockpit {
    async fn connect(url: &str, psk: &str, room: &str) -> Result<Self, String> {
        let signal = SignalClient::new(url, psk, room, PeerRole::Remote)
            .connect()
            .await
            .map_err(|e| format!("mock signaling connect failed: {e}"))?;

        let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidate>();
        let sent: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let answerers: Arc<Mutex<Vec<AnswererPc>>> = Arc::new(Mutex::new(Vec::new()));
        let offers: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let answerers_task = answerers.clone();
        let offers_task = offers.clone();

        let mut events = signal.events();
        let room_owned = room.to_string();
        let task = tokio::spawn(async move {
            let s = signal; // the session moves into the event task (SignalSession has no Clone)
            let answerers = answerers_task;
            let offers = offers_task;
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
                                    offers.lock().unwrap().push(desc.sdp.clone());
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

        Ok(Self { answerers, offers, task })
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

    fn answerer_sender_count(&self, answerer: usize) -> usize {
        self.answerers.lock().unwrap().get(answerer).map(|a| a.pc.get_senders().len()).unwrap_or(0)
    }

    fn offer_sdp(&self, idx: usize) -> String {
        self.offers.lock().unwrap().get(idx).cloned().unwrap_or_default()
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
/// The spool's Open event cannot be relied on - the channel may Open before the observer registers (F1 lesson).
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

/// D-H8 message format (published by the ROS vision node, transparently forwarded): frame correlation frame.seq/ts_mono_ns +
/// an objects array (class/confidence/bbox/text/color).
fn vision_payload(seq: u64, ts_mono_ns: u64) -> String {
    format!(
        r##"{{"frame":{{"seq":{seq},"ts_mono_ns":{ts_mono_ns}}},"objects":[{{"class":"person","confidence":0.95,"bbox":[10,20,100,200],"text":"person","color":"#ff0000"}}]}}"##
    )
}

#[tokio::test]
async fn vision_dc_transport_b_forwards_external_publisher() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let room = format!("vehicle-{pid}");
    let cam_id = format!("cam{pid}");
    let stream_id = format!("s{pid}-stream");

    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"{cam_id}\"\n    mode: \"generator\"\n    fps: 30\n\
             streams:\n  - id: \"{stream_id}\"\n    source: \"{cam_id}\"\n    codec: \"vp8\"\n"
        ),
    )
    .expect("write host.toml");

    // tokens: capturer=Capture (camera/*), streamer=Recorder (camera/*+vision/*, F3),
    // vision publisher=Perception (vision/*, D-H7 external node)
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
    let (str_tok, str_vk) = token(Role::Recorder, &format!("streamer-{pid}"));
    let str_path = dir.path().join("streamer.token");
    std::fs::write(&str_path, TokenFile::encode(&str_tok, &str_vk)).expect("write str token");
    let (vis_tok, vis_vk) = token(Role::Perception, &format!("vision-pub-{pid}"));
    let vis_path = dir.path().join("vision.token");
    std::fs::write(&vis_path, TokenFile::encode(&vis_tok, &vis_vk)).expect("write vis token");

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
            &psk(),
            "--room",
            &room,
        ])
        .stdout(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .stderr(Stdio::from(agent_log.reopen().expect("reopen agent log")))
        .spawn()
        .expect("spawn host-agent");
    wait_for(&agent_log, "agent joined the vehicle room");

    // the mock cockpit connects to the server directly and joins (remote slot of the P2P room) - before the streamer is ready
    let cockpit =
        MockCockpit::connect(&ws_url(), &psk(), &room).await.expect("mock cockpit connects");

    // capturer process (started first: the streamer's first-frame gate depends on the publisher)
    let cap_log = tempfile::NamedTempFile::new().expect("cap log");
    let mut capturer = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
        .args([
            "--camera",
            &cam_id,
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            cap_path.to_str().expect("cap token utf8"),
        ])
        .stdout(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .stderr(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .spawn()
        .expect("spawn host-capturer");
    wait_for(&cap_log, "capturer ready");

    // streamer process -> the local gateway (transport A video + transport B vision DC)
    let str_log = tempfile::NamedTempFile::new().expect("str log");
    let mut streamer = Command::new(env!("CARGO_BIN_EXE_host-streamer"))
        .args([
            "--stream",
            &stream_id,
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            str_path.to_str().expect("str token utf8"),
            "--gateway",
            &format!("ws://127.0.0.1:{gw_port}/ws"),
        ])
        .stdout(Stdio::from(str_log.reopen().expect("reopen str log")))
        .stderr(Stdio::from(str_log.reopen().expect("reopen str log")))
        .spawn()
        .expect("spawn host-streamer");
    wait_for(&str_log, "streamer ready");
    // the transport B negotiation-completion wait must be async (the mock cockpit task shares the test's tokio runtime;
    // thread::sleep blocking the runtime starves the mock's answers - F3 lesson, same mechanism as
    // wait_channel_open in the emergency e2e)
    let vision_dc = wait_channel_open(&cockpit, 0, "vision").await;
    assert_eq!(vision_dc.label(), "vision");
    assert!(
        read_log(&str_log).contains("vision answer set — negotiation complete"),
        "the streamer should confirm the answer landed:\n{}",
        read_log(&str_log)
    );

    // (1) transport separation (D-H8): the vision offer has exactly 1 m-line (application), no video m-line
    let offer = cockpit.offer_sdp(0);
    assert!(!offer.is_empty(), "the mock must receive the vision offer");
    let m_lines: Vec<&str> = offer.lines().filter(|l| l.starts_with("m=")).collect();
    assert_eq!(
        m_lines.len(),
        1,
        "the vision offer must have exactly 1 m-line (pure DC transport B, separated from video transport A), offer:\n{offer}"
    );
    assert!(
        m_lines[0].starts_with("m=application"),
        "the vision offer m-line must be application, got {:?}",
        m_lines
    );
    assert_eq!(cockpit.answerer_sender_count(0), 0, "transport B has no RTP track (pure DC PC)");
    assert_eq!(cockpit.answerer_channel_count(0), 1, "transport B has exactly 1 channel");

    // (2) the vision DC is open (wait_channel_open above)
    eprintln!("[vision_e2e] (1) vision DC open (transport B, independent of video transport A)");

    // D-H7 external-node path: a Perception-role token publishes vision/<cam> (ACL passthrough proven).
    // Continuous publishing (the ROS-detection 10-30Hz semantics) - frames within the subscriber's C5 rebuild window (5s without frames)
    // may be dropped; the continuous stream covers that window (the same holds for a real ROS node publishing continuously).
    let vbus = FrameBus::attach("", &vis_tok, &vis_vk).expect("vision publisher attach");
    let vt = FrameTopic::new(format!("vision/{cam_id}"));
    let published: Arc<Mutex<HashMap<u64, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let published_task = published.clone();
    let pub_handle = tokio::spawn(async move {
        let mut seq: u64 = 1;
        loop {
            let payload = vision_payload(seq, 1_700_000_000_000_000 + seq * 33_000_000);
            let meta = FrameMeta {
                seq,
                format: FrameMeta::FORMAT_JSON,
                ts_mono_ns: 1_700_000_000_000_000 + seq * 33_000_000,
                ..Default::default()
            };
            if vbus.publish(&vt, payload.as_bytes(), &meta).is_ok() {
                published_task.lock().unwrap().insert(seq, payload);
            }
            seq += 1;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });
    eprintln!(
        "[vision_e2e] (2) the external publisher keeps publishing vision JSON (Perception role, 5Hz)"
    );

    // The cockpit receives 3 messages (<=15s, including the subscriber's C5 rebuild window) - content = the publisher's verbatim payload (transparent pipe) + frame correlation fields
    let mut rx = vision_dc.spool().await;
    let mut received: Vec<Vec<u8>> = Vec::new();
    let recv_result = tokio::time::timeout(Duration::from_secs(15), async {
        while received.len() < 3 {
            if let Some(RTCDataChannelEvent::Message(m)) = rx.recv().await {
                received.push(m.data);
            }
        }
    })
    .await;
    pub_handle.abort();
    if recv_result.is_err() {
        eprintln!("[vision_e2e] FAIL DC message timeout - streamer log:\n{}", read_log(&str_log));
        eprintln!("[vision_e2e] FAIL agent log:\n{}", read_log(&agent_log));
        panic!("vision DC message timeout");
    }

    for data in &received {
        let v: serde_json::Value = serde_json::from_slice(data).expect("received valid JSON");
        let seq = v["frame"]["seq"].as_u64().expect("the frame correlation seq");
        let src = published
            .lock()
            .unwrap()
            .get(&seq)
            .cloned()
            .expect("the seq should have been published");
        assert_eq!(
            data,
            src.as_bytes(),
            "seq {seq} must match the publisher payload verbatim (transparent forwarding, no re-encoding)"
        );
        assert!(
            v["frame"]["ts_mono_ns"].as_u64().is_some(),
            "the frame correlation ts_mono_ns must be present"
        );
        assert_eq!(v["objects"][0]["class"], "person");
        assert_eq!(v["objects"][0]["confidence"], 0.95);
        assert_eq!(v["objects"][0]["bbox"], serde_json::json!([10, 20, 100, 200]));
        assert_eq!(v["objects"][0]["text"], "person");
        assert_eq!(v["objects"][0]["color"], "#ff0000");
    }
    let seqs: Vec<u64> = received
        .iter()
        .filter_map(|d| serde_json::from_slice::<serde_json::Value>(d).ok())
        .filter_map(|v| v["frame"]["seq"].as_u64())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "ordered DC delivery (SCTP ordered), seqs: {seqs:?}"
    );
    eprintln!(
        "[vision_e2e] (2) vision JSON transparent forwarding proven (frame seq {seqs:?} + complete objects fields)"
    );

    // (3) video unaffected (transport A): outbound stats bytes_sent>0 and frames_encoded>0
    let evidence = wait_for_flow(&str_log);
    eprintln!("[vision_e2e] (3) video stream evidence (transport A unaffected): {evidence}");

    // (5) SIGTERM -> all three processes exit gracefully with 0
    unsafe { libc::kill(streamer.id() as i32, libc::SIGTERM) };
    let st = streamer.wait().expect("wait streamer");
    assert_eq!(st.code(), Some(0), "the streamer should exit gracefully with 0, got {st:?}");
    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "the capturer should exit gracefully with 0, got {ct:?}");
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let at = agent.wait().expect("wait agent");
    assert_eq!(at.code(), Some(0), "agent should exit gracefully with 0, got {at:?}");

    cockpit.close().await;
}
