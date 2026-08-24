//! Task E3: StatusReport 上报 e2e — host-agent 网关 + 状态上报任务 → mock server。
//!
//! 覆盖: ① 网关远端入房后，reporter 周期聚合三快照 → StatusReport 上行；
//! ② 内容断言: room_id/ts/config_version + 拓扑进程（期望并集）+ 信令平面
//! （remote_connected/peer_id）+ 空数据流段（无 token → FlowMonitor 缺席）。
//!
//! 前置（C25）: `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*`（FrameBus 发现残留）。

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_host::gateway::{run_gateway, GatewayConfig};
use mediaservo_host::monitor::signal::spawn_status_reporter;
use mediaservo_link::RetryConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

const VEHICLE_ROOM: &str = "vehicle-1";
const VEHICLE_PEER: &str = "veh-peer";
const HOST_TOML: &str = r#"
sources:
  - id: "cam0"
    mode: "generator"
    fps: 30
"#;

/// mock server 完整握手（对齐 link::SignalClient 协议流程）。
async fn mock_handshake(listener: &TcpListener) -> WebSocketStream<TcpStream> {
    let (stream, _) = listener.accept().await.expect("mock accept");
    let mut ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("mock ws handshake");
    let psk = ws.next().await.unwrap().unwrap();
    assert!(matches!(psk, Message::Text(_)), "首条应为 PSK 文本");
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::Error { code: 0, message: String::new() })
            .unwrap()
            .into(),
    ))
    .await
    .unwrap();
    let join = ws.next().await.unwrap().unwrap();
    let room = match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap())
        .expect("parse RoomJoin")
    {
        SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
            assert_eq!(peer_role, PeerRole::Host);
            room_id
        }
        other => panic!("expected RoomJoin, got {other:?}"),
    };
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::RoomJoined {
            room_id: room.clone(),
            peer_id: VEHICLE_PEER.into(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    ws
}

#[tokio::test]
async fn status_report_reaches_mock_server_with_expected_content() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut ws = mock_handshake(&listener).await;
        // 入房后第一条应用层消息应为 StatusReport（reporter 首个周期）
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("10s 内应收到 StatusReport")
            .expect("ws 关闭")
            .expect("ws 错误");
        let msg: SignalingMessage =
            serde_json::from_str(msg.to_text().unwrap()).expect("StatusReport 解析");
        match msg {
            SignalingMessage::StatusReport {
                room_id,
                topics,
                streams,
                processes,
                signal,
                ts,
                config_version,
            } => {
                assert_eq!(room_id, VEHICLE_ROOM, "上报 room_id 应为整车房间");
                assert!(topics.is_empty(), "无 token → 数据流段应为空: {topics:?}");
                assert!(streams.is_empty(), "无 token → 数据流段应为空: {streams:?}");
                assert!(
                    processes.iter().any(|p| p.name == "host-agent" && p.expected),
                    "固定进程 host-agent 应在期望并集中: {processes:?}"
                );
                assert!(
                    processes.iter().any(|p| p.name == "host-capturer-cam0" && p.expected),
                    "host.yaml 视频源实例应在期望并集中（单源无 id 后缀）: {processes:?}"
                );
                assert!(signal.remote_connected, "网关已入房, remote 应 connected");
                assert_eq!(signal.remote_peer_id, VEHICLE_PEER, "peer_id 应来自 RoomJoined");
                assert!(ts > 0, "ts 应为 unix 秒: {ts}");
                assert_eq!(config_version, 0, "E4 前 config_version 恒 0");
            }
            other => panic!("期望 StatusReport, got {other:?}"),
        }
    });

    let cfg = GatewayConfig {
        local_port: 0,
        remote_url: format!("ws://{addr}/ws"),
        psk: "test-psk".into(),
        device: None,
        room: VEHICLE_ROOM.into(),
        retry: RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(500),
        },
    };
    let (_port, handle) = run_gateway(cfg).await.unwrap();
    spawn_status_reporter(
        HOST_TOML.into(),
        Instant::now(),
        None,
        handle,
        Duration::from_millis(300),
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    server.await.unwrap();
}
