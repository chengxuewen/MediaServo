//! E2E integration tests for field::PushSession → mediasoup SFU pipeline.
//!
//! Runs on Linux only — connects to an external mediasoup server (C21).
//! Pure external mode: interacts only through the WS signaling protocol, imports no server internal types.
//!
//! Tests:
//! - D1-field: PushSession connect → publish_video full chain (transport→produce)
//! - D2-field: signaling event bridge (SignalEvent → SessionEvent::Message)

#![cfg(target_os = "linux")]

use std::time::Duration;

// Diagnostics: init tracing inside tests (otherwise logs are invisible)
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
}

use mediaservo_common::protocol::PeerRole;
use mediaservo_field::{
    FieldError, PublishOptions, PullConfig, PullSession, PushConfig, PushSession, SessionEvent,
};
use mediaservo_webrtc::traits::PeerConnectionApi;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

fn ws_url() -> String {
    // Default dev server (docker compose 9800) — still connects to an external server (C21); production/CI override via env
    std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9800/ws".to_string())
}

fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "mediaservo-dev".to_string())
}

fn test_config() -> PushConfig {
    // Independent room per call (test parallelism + unique per run; guards against leftover producers on the server)
    let room = format!(
        "field-push-test-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    );
    let mut cfg = PushConfig::new(ws_url(), psk(), room);
    cfg.role = PeerRole::Host;
    cfg
}

/// D1-field: PushSession full chain — connect → publish_video succeeds.
///
/// Flow (mirrors host e2e_sfu D1/D2):
/// 1. PushSession::connect(cfg) — signaling connection + join the room
/// 2. publish_video — transport created → answer negotiation → Connect → Produce
/// 3. Assert the returned track id + TrackPublished event
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_push_session_publish_video() {
    let cfg = test_config();
    let (mut session, mut events) =
        PushSession::connect(cfg.clone()).await.expect("PushSession connect failed");

    let opts = PublishOptions::default(); // VP8 / auto backend
    let track = tokio::time::timeout(CONNECT_TIMEOUT, session.publish_video(&cfg, &opts))
        .await
        .expect("publish_video timeout")
        .expect("publish_video failed");
    assert!(!track.is_empty(), "track id empty");

    // TrackPublished should appear in the event stream
    tokio::time::timeout(CONNECT_TIMEOUT, async {
        while let Some(ev) = events.recv().await {
            match ev {
                SessionEvent::TrackPublished { track: published } => {
                    assert_eq!(published, track, "published track id mismatch");
                    return;
                }
                SessionEvent::Error(e) => panic!("session error during publish: {e:?}"),
                _ => continue, // Message/Connected/Disconnected ignored
            }
        }
        panic!("event stream closed before TrackPublished");
    })
    .await
    .expect("TrackPublished event timeout");

    session.close().await.expect("close failed");
}

/// D2-field: connection-failure path — connect should return LinkError rather than hang when no server.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_push_session_connect_failure() {
    let mut cfg = test_config();
    cfg.url = "ws://127.0.0.1:1/ws".to_string(); // certainly unreachable port
    let err = PushSession::connect(cfg).await;
    match err {
        Err(FieldError::Link(_)) => {}
        other => panic!("expected LinkError, got {other:?}"),
    }
}

/// D3-field: peer_connection escape hatch — the underlying PC should be exposed after publish.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_push_session_peer_connection_available() {
    let cfg = test_config();
    let (mut session, _events) = PushSession::connect(cfg.clone()).await.expect("connect failed");

    // No PC before publish
    assert!(session.peer_connection().is_none(), "PC should be None before publish");

    let opts = PublishOptions::default();
    tokio::time::timeout(CONNECT_TIMEOUT, session.publish_video(&cfg, &opts))
        .await
        .expect("publish timeout")
        .expect("publish failed");

    // PC available after publish (escape hatch reaches the underlying state)
    let pc = session.peer_connection().expect("PC available after publish");
    assert_eq!(pc.track_count(), 1, "one video track");

    session.close().await.expect("close failed");
}
/// D4-field: frame publishing — after start_video_frames the sender should produce encoded frames (bytes_sent/frames_encoded growing).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_push_session_video_frames_flow() {
    let cfg = test_config();
    let (mut session, _events) = PushSession::connect(cfg.clone()).await.expect("connect failed");

    let opts = PublishOptions::default();
    tokio::time::timeout(CONNECT_TIMEOUT, session.publish_video(&cfg, &opts))
        .await
        .expect("publish timeout")
        .expect("publish failed");

    // Before frame publishing: no generator
    assert!(session.peer_connection().is_some(), "PC after publish");

    // Start frame generation
    session.start_video_frames(&cfg).expect("start_video_frames failed");
    // Repeated start should return InvalidState
    let dup = session.start_video_frames(&cfg).unwrap_err();
    assert!(matches!(dup, FieldError::InvalidState(_)), "got {dup:?}");

    // Poll sender stats: frame encoding should start (wait ≤10s for frames)
    let pc = session.peer_connection().expect("pc");
    let mut observed_frames = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let stats = pc.sender_get_stats("video");
        if let Some(o) = stats.iter().find_map(|s| match s {
            mediaservo_webrtc::stats::RTCStats::OutboundRtp(o) => Some(o),
            _ => None,
        }) && o.bytes_sent > 0
            && o.frames_encoded > 0
        {
            tracing::info!(
                "frames flowing: bytes_sent={} frames_encoded={}",
                o.bytes_sent,
                o.frames_encoded
            );
            observed_frames = true;
            break;
        }
    }
    assert!(observed_frames, "no outbound frames observed within 10s");

    // Stop frame generation (idempotent)
    session.stop_video_frames();
    session.stop_video_frames(); // Second call has no effect

    session.close().await.expect("close failed");
}

/// D5-field: PullSession consume — Push frames → Pull subscribes → decoded frames flow out.
/// Known limitation (closed 2026-08-18): negotiation fully succeeds but libwebrtc hangs receiving frames (RTP all correct,
/// on_frame never fires) — belongs to client-side development. Test kept as a documented limitation.
#[ignore = "PullSession frame reception hangs (libwebrtc receive-pipeline defect, closed 2026-08-18)"]
///
/// Same room: PushSession publish + start_video_frames → PullSession subscribe
/// (producer_id from the NewProducer broadcast) → on_track → FrameSink → PullFrame received.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_pull_session_consumes_video() {
    init_tracing();
    use mediaservo_common::protocol::SignalingMessage;

    let room = format!(
        "field-pull-test-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    );
    let push_cfg = PushConfig::new(ws_url(), psk(), room.clone());
    let pull_cfg = PullConfig {
        url: ws_url(),
        psk: psk(),
        room,
        role: PeerRole::Consumer,
        auto_subscribe: true,
    };

    // 1. Pull joins the room first (Consumer role) — guarantees the NewProducer broadcast of the push publish always arrives
    let (mut pull, mut pull_events) =
        PullSession::connect(pull_cfg.clone()).await.expect("pull connect");

    // 2. Push side: connect + publish + frame generation (broadcast to peers already in the room = Pull)
    let (mut push, push_events) =
        PushSession::connect(push_cfg.clone()).await.expect("push connect");
    let opts = PublishOptions::default();
    let track = tokio::time::timeout(CONNECT_TIMEOUT, push.publish_video(&push_cfg, &opts))
        .await
        .expect("push publish timeout")
        .expect("push publish failed");
    push.start_video_frames(&push_cfg).expect("start frames");

    // 3. Wait for the NewProducer broadcast (Push's producer)
    let producer_id = tokio::time::timeout(CONNECT_TIMEOUT, async {
        loop {
            match pull_events.recv().await {
                Some(SessionEvent::Message(SignalingMessage::NewProducer {
                    producer_id, ..
                })) => {
                    return producer_id;
                }
                Some(SessionEvent::Error(e)) => panic!("pull session error: {e:?}"),
                _ => continue,
            }
        }
    })
    .await
    .expect("NewProducer timeout");

    // 4. Subscribe + receive frames
    let mut frames = tokio::time::timeout(CONNECT_TIMEOUT, pull.subscribe(&pull_cfg, &producer_id))
        .await
        .expect("subscribe timeout")
        .expect("subscribe failed");

    // 5. Wait for decoded frames to flow out (≤15s; WebRtcTrackSink pushes frames → SFU relay → decode → FrameSink)
    let mut got_frame = false;
    for _ in 0..30 {
        tokio::time::timeout(Duration::from_secs(1), frames.recv()).await.map(|opt| {
            if let Some(f) = opt {
                assert!(f.width > 0 && f.height > 0, "frame dims");
                assert!(!f.data.is_empty(), "frame data");
                tracing::info!("pull frame: {}x{} ({} bytes)", f.width, f.height, f.data.len());
                got_frame = true;
            }
        });
        if got_frame {
            break;
        }
    }
    assert!(got_frame, "no decoded frame within 15s");

    // Cleanup
    push.stop_video_frames();
    push.close().await.expect("push close");
    pull.close().await.expect("pull close");
    let _ = track;
    let _ = push_events;
}

/// D6-field: repeated publish should return InvalidState (MVP single-video-track constraint).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_push_session_double_publish_fails() {
    let cfg = test_config();
    let (mut session, _events) = PushSession::connect(cfg.clone()).await.expect("connect failed");

    let opts = PublishOptions::default();
    tokio::time::timeout(CONNECT_TIMEOUT, session.publish_video(&cfg, &opts))
        .await
        .expect("first publish timeout")
        .expect("first publish failed");

    let dup = session.publish_video(&cfg, &opts).await.unwrap_err();
    assert!(matches!(dup, FieldError::InvalidState(_)), "got {dup:?}");

    session.close().await.expect("close failed");
}

/// D7-field: low-resolution low-fps config — frame publishing parameter boundary (640x360@15fps).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn field_push_session_low_res_frames() {
    let mut cfg = test_config();
    cfg.width = 640;
    cfg.height = 360;
    cfg.framerate = 15;
    cfg.bitrate_kbps = 500;

    let (mut session, _events) = PushSession::connect(cfg.clone()).await.expect("connect failed");
    let opts = PublishOptions::default();
    tokio::time::timeout(CONNECT_TIMEOUT, session.publish_video(&cfg, &opts))
        .await
        .expect("publish timeout")
        .expect("publish failed");

    session.start_video_frames(&cfg).expect("start frames");

    // Poll sender stats: frames should still encode under the low-bitrate config
    let pc = session.peer_connection().expect("pc");
    let mut observed = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let stats = pc.sender_get_stats("video");
        if let Some(o) = stats.iter().find_map(|s| match s {
            mediaservo_webrtc::stats::RTCStats::OutboundRtp(o) => Some(o),
            _ => None,
        }) && o.frames_encoded > 0
        {
            // Resolution may be degraded adaptively by libwebrtc BWE (low bitrate → scaling down) —
            // do not force equality with the config; verifying frames encode is enough (dimension semantics guaranteed by the C17 frame loop)
            assert!(o.frame_width > 0 && o.frame_height > 0, "frame dims");
            observed = true;
            break;
        }
    }
    assert!(observed, "no frames at low-res config within 10s");

    session.stop_video_frames();
    session.close().await.expect("close failed");
}
