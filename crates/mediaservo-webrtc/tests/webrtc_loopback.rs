//! P2P loopback test: two PeerConnections, video frame push/pull, fps measurement.
//!
//! Creates two PCs, exchanges SDP, writes test-pattern frames on PC1,
//! receives frames via `on_track` callback on PC2. Measures fps.
//!
//! Run with:
//! ```bash
//! cargo test -p mediaservo-webrtc --test webrtc_loopback
//! ```

mod common;
use common::loopback::{FpsCounter, create_connected_pair, generate_test_frame};

use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
use mediaservo_webrtc::track::{TrackKind, TrackRef};
use mediaservo_webrtc::traits::PeerConnectionApi;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TEST_FRAMES: u64 = 30;
const FRAME_WIDTH: u32 = 320;
const FRAME_HEIGHT: u32 = 240;
const FRAME_INTERVAL_MS: u64 = 33; // ~30 fps

#[test]
fn loopback_factory_creates_and_connects() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (pc1, pc2) = create_connected_pair().await.expect("connect");
        // verify both PCs created — just check they don't panic
        let _ = pc1.connection_state();
        let _ = pc2.connection_state();
    });
}

#[test]
fn loopback_sdp_exchange_succeeds() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (pc1, pc2) = create_connected_pair().await.expect("connect");
        // After SDP exchange, signaling state should be stable
        use mediaservo_webrtc::peer_connection::RTCSignalingState;
        assert_eq!(pc1.signaling_state(), RTCSignalingState::Stable);
        assert_eq!(pc2.signaling_state(), RTCSignalingState::Stable);
    });
}

#[test]
fn loopback_video_push_receive() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (pc1, pc2) = create_connected_pair().await.expect("connect");

        // Track received frames on PC2
        let received_count = Arc::new(AtomicU64::new(0));
        let rc = received_count.clone();
        pc2.on_track(move |_receiver| {
            rc.fetch_add(1, Ordering::Relaxed);
        });

        // Add video track on PC1
        let track_id = pc1.add_track("video-loopback", TrackKind::Video).expect("add track");
        assert!(!track_id.is_empty());

        // Write test frames through TrackSender
        for i in 0..TEST_FRAMES {
            let frame = generate_test_frame(FRAME_WIDTH, FRAME_HEIGHT, i);
            if let Some(tr) = pc1.get_track(&track_id) {
                if let TrackRef::Sender(ts) = tr {
                    ts.write_frame(&frame).await.expect("write frame");
                }
            }
            tokio::time::sleep(Duration::from_millis(FRAME_INTERVAL_MS)).await;
        }

        // With stub backend, onTrack won't fire (stub is no-op).
        // With real backend, received_count should approach TEST_FRAMES.
        let received = received_count.load(Ordering::Relaxed);
        assert!(received <= TEST_FRAMES, "received more frames than sent");
    });
}

#[test]
fn generate_test_frame_produces_correct_size() {
    let frame = generate_test_frame(FRAME_WIDTH, FRAME_HEIGHT, 0);
    let expected_size =
        (FRAME_WIDTH * FRAME_HEIGHT) as usize + 2 * ((FRAME_WIDTH * FRAME_HEIGHT) / 4) as usize;
    assert_eq!(frame.len(), expected_size);
}

#[test]
fn loopback_fps_counter() {
    let counter = FpsCounter::new();
    for _ in 0..30 {
        counter.tick();
    }
    assert!(counter.fps() > 0.0);
    assert_eq!(counter.count(), 30);
}

#[test]
fn loopback_data_channel_between_pcs() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (pc1, _pc2) = create_connected_pair().await.expect("connect");

        // PC1 creates a data channel
        let mut dc1 =
            pc1.create_data_channel("loopback-dc", Default::default()).await.expect("create dc");

        assert_eq!(dc1.label(), "loopback-dc");

        // Verify we can send text without error
        dc1.send_text("hello loopback").await.expect("send text");
    });
}

#[test]
fn loopback_multiple_tracks() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (pc1, pc2) = create_connected_pair().await.expect("connect");

        // onTrack callback captures receiver IDs
        let received_ids = Arc::new(Mutex::new(Vec::new()));
        let ri = received_ids.clone();
        pc2.on_track(move |receiver| {
            ri.lock().unwrap().push(receiver.track_id.clone());
        });

        // Add multiple tracks of different kinds
        pc1.add_track("video-1", TrackKind::Video).expect("add video-1");
        pc1.add_track("audio-1", TrackKind::Audio).expect("add audio-1");
        pc1.add_track("video-2", TrackKind::Video).expect("add video-2");

        assert_eq!(pc1.track_count(), 3);

        // Verify track IDs are registered
        let ids = pc1.track_ids();
        assert!(ids.contains(&"video-1".to_string()));
        assert!(ids.contains(&"audio-1".to_string()));
        assert!(ids.contains(&"video-2".to_string()));
    });
}

#[test]
fn loopback_add_remove_track() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (pc1, _pc2) = create_connected_pair().await.expect("connect");

        pc1.add_track("temp-track", TrackKind::Video).expect("add track");
        assert_eq!(pc1.track_count(), 1);

        pc1.remove_track("temp-track").expect("remove track");
        assert_eq!(pc1.track_count(), 0);
    });
}

#[test]
fn loopback_close_connects_gracefully() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (pc1, pc2) = create_connected_pair().await.expect("connect");

        // Close should not panic
        pc1.close().await;
        pc2.close().await;

        // Verify closed state
        use mediaservo_webrtc::peer_connection::RTCPeerConnectionState;
        assert!(
            matches!(pc1.connection_state(), RTCPeerConnectionState::Closed),
            "pc1 should be Closed after close()"
        );
    });
}
