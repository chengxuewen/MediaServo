//! Task F3: streamer 视觉 DC e2e — 外部发布者 → FrameBus vision topic → streamer
//! transport B（独立纯 DC PC，label "vision"）→ 舱端 mock。
//!
//! 拓扑（生产路径，D-H6/D-H8）：
//! ```text
//! host-agent (网关进程)           ──WS──▶ 外部 mediasoup server（SFU_E2E_WS_URL）
//! host-capturer (进程)            ──FrameBus camera/<cam>──▶ host-streamer (进程)
//! vision 发布者 (测试内, Perception 角色, D-H7 外部节点模拟)
//!                                ──FrameBus vision/<cam>──▶ host-streamer
//! host-streamer transport A (视频, SFU produce) ──▶ server（原 C2 链路不变）
//! host-streamer transport B (纯 DC "vision", P2P relay) ──▶ mock 舱端 answerer
//! mock 舱端 (测试内)              ──WS 直连 server（PSK, PeerRole::Remote）
//! ```
//!
//! 断言：
//! ① transport 分离（D-H8）：vision offer SDP 仅 1 个 m-line（m=application，
//!    无视频 m-line → 视觉不在视频 PC 上）；answerer 无 RTP sender、仅 1 通道 label "vision"
//! ② 透明转发：DC 收到的文本 == 发布者 payload 原文（streamer = pipe，不重编码），
//!    帧关联字段 frame.seq / frame.ts_mono_ns 与 objects（class/confidence/bbox/text/color）齐全
//! ③ 视频不受影响（transport A）：streamer stats bytes_sent>0 且 frames_encoded>0
//! ④ D-H7 外部节点验证缺口：Perception 角色令牌发布 vision/*（ACL 放行实证）
//! ⑤ SIGTERM → streamer + capturer + agent 优雅退出 0
//!
//! 前置: `SFU_E2E_WS_URL` 指向外部 mediasoup server（C21 纯外部模式）；
//! C25: 跑前清 `/tmp/iceoryx2` + `/dev/shm/iox2_*`。

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
use mediaservo_webrtc::{RTCPeerConnection, RTCPeerConnectionFactory, RTCConfiguration};
use tokio::sync::mpsc;

// 测试用 Ed25519 密钥对（openssl 生成，仅测试用；与 link/deck/capturer 测试同源）。
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: 清 iceoryx2 0.9.3 运行时残留（/tmp/iceoryx2 + /dev/shm/iox2_*）。
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
            "SFU_E2E_WS_URL 未设置 — vision e2e 需连外部 mediasoup server (C21);
            例: SFU_E2E_WS_URL=ws://127.0.0.1:9800/ws"
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
    file.reopen()
        .expect("reopen log")
        .read_to_string(&mut out)
        .expect("read log");
    out
}

fn wait_for(log: &tempfile::NamedTempFile, needle: &str) {
    for _ in 0..20 {
        if read_log(log).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("未见 {needle:?}, log:\n{}", read_log(log));
}


/// 轮询日志直到 stats 行 bytes_sent>0 且 frames_encoded>0（≤30s, D4 证据模式）。
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
    panic!("30s 内未见流证据, log:\n{}", read_log(log));
}

// ── mock 舱端：直连 server（PSK），每 offer 一个独立 answerer PC ──
// F3 只有一个 P2P offer（streamer transport B "vision"）→ answerer 0。
// 额外捕获收到的 offer SDP 原文（m-line 断言 = transport 分离证据）。

struct AnswererPc {
    pc: RTCPeerConnection,
    channels: Arc<Mutex<HashMap<String, RTCDataChannel>>>,
}

struct MockCockpit {
    answerers: Arc<Mutex<Vec<AnswererPc>>>,
    /// 收到的 offer SDP 原文（F3 断言: 仅 1 个 application m-line）。
    offers: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockCockpit {
    async fn connect(url: &str, psk: &str, room: &str) -> Result<Self, String> {
        let signal = SignalClient::new(url, psk, room, PeerRole::Remote)
            .connect()
            .await
            .map_err(|e| format!("mock 信令连接失败: {e}"))?;

        let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<RTCIceCandidate>();
        let sent: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let answerers: Arc<Mutex<Vec<AnswererPc>>> = Arc::new(Mutex::new(Vec::new()));
        let offers: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let answerers_task = answerers.clone();
        let offers_task = offers.clone();

        let mut events = signal.events();
        let room_owned = room.to_string();
        let task = tokio::spawn(async move {
            let s = signal; // 会话移入事件任务（SignalSession 无 Clone）
            let answerers = answerers_task;
            let offers = offers_task;
            let mut active: Option<usize> = None;
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(SignalEvent::Message(m)) => {
                            let text = serde_json::to_string(&m).unwrap_or_default();
                            if sent.lock().unwrap().contains(&text) {
                                continue; // 自己消息的回显
                            }
                            match m {
                                SignalingMessage::Sdp { sdp, .. } => {
                                    let desc: RTCSessionDescription =
                                        serde_json::from_str(&sdp).expect("mock 解析 Sdp JSON");
                                    if desc.sdp_type != RTCSdpType::Offer {
                                        continue;
                                    }
                                    offers.lock().unwrap().push(desc.sdp.clone());
                                    // 新 offer → 新 answerer PC（每协商独立对端）
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
                                        serde_json::to_string(&answer).expect("序列化 answer");
                                    sent.lock().unwrap().insert(
                                        serde_json::to_string(&SignalingMessage::Sdp {
                                            room_id: room_owned.clone(),
                                            target: None,
                                            sdp: json.clone(),
                                        })
                                        .expect("序列化 Sdp 消息"),
                                    );
                                    s.send(SignalingMessage::Sdp {
                                        room_id: room_owned.clone(),
                                        target: None,
                                        sdp: json,
                                    })
                                    .await
                                    .expect("mock 发送 answer");
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
                                        None => tracing::warn!("ICE 无活动 answerer，丢弃"),
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
                            serde_json::to_string(&msg).expect("序列化 ICE"),
                        );
                        s.send(msg).await.expect("mock 发送 ICE");
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
        self.answerers
            .lock()
            .unwrap()
            .get(answerer)
            .map(|a| a.pc.get_senders().len())
            .unwrap_or(0)
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

/// 等待 answerer 的 label 通道出现并 Open（轮询 state()，≤15s）。
/// 不能依赖 spool 的 Open 事件 — 通道可能在观察者注册前已 Open（F1 教训）。
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
    .expect("answerer {answerer} 通道 {label} 未出现")
    .clone();
    tokio::time::timeout(Duration::from_secs(15), async {
        while dc.state() != mediaservo_webrtc::data_channel::RTCDataChannelState::Open {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("通道 {label} 未 Open");
    dc
}

/// D-H8 消息格式（ROS 视觉节点发布，透明转发）：帧关联 frame.seq/ts_mono_ns +
/// objects 数组（class/confidence/bbox/text/color）。
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

    // 令牌: capturer=Capture（camera/*）, streamer=Recorder（camera/*+vision/*, F3）,
    // vision 发布者=Perception（vision/*, D-H7 外部节点）
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
    let (str_tok, str_vk) = token(Role::Recorder, &format!("streamer-{pid}"));
    let str_path = dir.path().join("streamer.token");
    std::fs::write(&str_path, TokenFile::encode(&str_tok, &str_vk)).expect("write str token");
    let (vis_tok, vis_vk) = token(Role::Perception, &format!("vision-pub-{pid}"));
    let vis_path = dir.path().join("vision.token");
    std::fs::write(&vis_path, TokenFile::encode(&vis_tok, &vis_vk)).expect("write vis token");

    // host-agent（本地网关）: 随机端口 + 远端真 server + 唯一房间
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
    wait_for(&agent_log, "agent 已加入整车房间");

    // mock 舱端直连 server 入房（P2P 房间 remote 槽位）— 先于 streamer 就绪
    let cockpit = MockCockpit::connect(&ws_url(), &psk(), &room)
        .await
        .expect("mock cockpit 连接");

    // capturer 进程（先起：streamer 首帧 gate 依赖发布端）
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

    // streamer 进程 → 本地网关（transport A 视频 + transport B 视觉 DC）
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
    // transport B 协商完成等待必须 async（mock 舱端任务与测试同 tokio runtime，
    // thread::sleep 阻塞 runtime 会饿死 mock 应答——F3 教训，emergency e2e 的
    // wait_channel_open 同机制）
    let vision_dc = wait_channel_open(&cockpit, 0, "vision").await;
    assert_eq!(vision_dc.label(), "vision");
    assert!(
        read_log(&str_log).contains("vision answer 已设置 — 协商完成"),
        "streamer 应确认 answer 落地:\n{}",
        read_log(&str_log)
    );

    // ① transport 分离（D-H8）: vision offer 仅 1 个 m-line（application），无视频 m-line
    let offer = cockpit.offer_sdp(0);
    assert!(!offer.is_empty(), "mock 必须收到 vision offer");
    let m_lines: Vec<&str> = offer.lines().filter(|l| l.starts_with("m=")).collect();
    assert_eq!(
        m_lines.len(),
        1,
        "vision offer 必须仅 1 个 m-line（纯 DC transport B, 与视频 transport A 分离）, offer:\n{offer}"
    );
    assert!(
        m_lines[0].starts_with("m=application"),
        "vision offer m-line 必须为 application, got {:?}",
        m_lines
    );
    assert_eq!(
        cockpit.answerer_sender_count(0),
        0,
        "transport B 无 RTP track（纯 DC PC）"
    );
    assert_eq!(
        cockpit.answerer_channel_count(0),
        1,
        "transport B 仅 1 通道"
    );

    // ② vision DC 已打开（上方 wait_channel_open）
    eprintln!("[vision_e2e] ① vision DC open（transport B，独立于视频 transport A）");

    // D-H7 外部节点路径: Perception 角色令牌发布 vision/<cam>（ACL 放行实证）。
    // 持续发布（ROS 检测 10-30Hz 语义）— 订阅端 C5 重建窗口（5s 无帧）内
    // 的帧可丢，连续流覆盖该窗口（生产上 ROS 节点持续发布同理）。
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
    eprintln!("[vision_e2e] ② 外部发布者持续发布 vision JSON（Perception 角色, 5Hz）");

    // 舱端收取 3 条消息（≤15s，含订阅端 C5 重建窗口）— 内容 = 发布者原文（透明管道）+ 帧关联字段
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
        eprintln!("[vision_e2e] FAIL DC 消息超时 — streamer log:\n{}", read_log(&str_log));
        eprintln!("[vision_e2e] FAIL agent log:\n{}", read_log(&agent_log));
        panic!("vision DC 消息超时");
    }

    for data in &received {
        let v: serde_json::Value = serde_json::from_slice(data).expect("收到合法 JSON");
        let seq = v["frame"]["seq"].as_u64().expect("帧关联 seq");
        let src = published.lock().unwrap().get(&seq).cloned().expect("seq 应已发布");
        assert_eq!(
            data,
            src.as_bytes(),
            "seq {seq} 必须与发布者 payload 原文一致（透明转发，不重编码）"
        );
        assert!(
            v["frame"]["ts_mono_ns"].as_u64().is_some(),
            "帧关联 ts_mono_ns 必须存在"
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
        "DC 有序投递（SCTP ordered）, seqs: {seqs:?}"
    );
    eprintln!("[vision_e2e] ② vision JSON 透明转发实证（frame seq {seqs:?} + objects 字段齐全）");

    // ③ 视频不受影响（transport A）: 出站统计 bytes_sent>0 且 frames_encoded>0
    let evidence = wait_for_flow(&str_log);
    eprintln!("[vision_e2e] ③ 视频流证据（transport A 未受影响）: {evidence}");

    // ⑤ SIGTERM → 三进程优雅退出 0
    unsafe { libc::kill(streamer.id() as i32, libc::SIGTERM) };
    let st = streamer.wait().expect("wait streamer");
    assert_eq!(st.code(), Some(0), "streamer 应优雅退出 0, got {st:?}");
    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "capturer 应优雅退出 0, got {ct:?}");
    unsafe { libc::kill(agent.id() as i32, libc::SIGTERM) };
    let at = agent.wait().expect("wait agent");
    assert_eq!(at.code(), Some(0), "agent 应优雅退出 0, got {at:?}");

    cockpit.close().await;
}
