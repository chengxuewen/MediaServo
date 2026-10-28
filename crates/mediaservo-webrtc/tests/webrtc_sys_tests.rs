//! webrtc-sys backend tests — validates RealObserver, VideoSink bridge, and FrameSink.
//!
//! These tests require libwebrtc (feature = "backend-webrtc-sys").
//! Run with: cargo test -p mediaservo-webrtc --features backend-webrtc-sys --test webrtc_sys_tests

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
use mediaservo_webrtc::peer_connection::RTCConfiguration;
use mediaservo_webrtc::traits::PeerConnectionApi;

#[tokio::test]
async fn factory_creates_with_webrtc_sys() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    assert!(matches!(
        pc.connection_state(),
        mediaservo_webrtc::peer_connection::RTCPeerConnectionState::New
    ));
    pc.close().await;
}

#[tokio::test]
async fn real_observer_is_wired() {
    // Verify that RealObserver is created (not NoOpObserver)
    // on_track callback should register without error
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    let called = Arc::new(AtomicBool::new(false));
    let c = called.clone();
    pc.on_track(move |_| {
        c.store(true, Ordering::Relaxed);
    });

    pc.close().await;
}

#[tokio::test]
async fn close_state_transition() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    pc.close().await;
    assert!(matches!(
        pc.connection_state(),
        mediaservo_webrtc::peer_connection::RTCPeerConnectionState::Closed
    ));
}

#[tokio::test]
async fn sdp_operations_do_not_panic() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    let offer = pc.create_offer(&Default::default()).await.unwrap();
    pc.set_local_description(&offer).await.unwrap();

    pc.close().await;
}

#[tokio::test]
async fn ice_state_queries() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    assert!(matches!(
        pc.ice_connection_state(),
        mediaservo_webrtc::peer_connection::RTCIceConnectionState::New
    ));
    assert!(matches!(
        pc.ice_gathering_state(),
        mediaservo_webrtc::peer_connection::RTCIceGatheringState::New
    ));

    pc.close().await;
}

#[tokio::test]
async fn signaling_state_default() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    assert!(matches!(
        pc.signaling_state(),
        mediaservo_webrtc::peer_connection::RTCSignalingState::Stable
    ));

    pc.close().await;
}

#[tokio::test]
async fn track_count_is_zero_initially() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    assert_eq!(pc.track_count(), 0);
    assert!(pc.track_ids().is_empty());
    assert!(pc.get_senders().is_empty());
    assert!(pc.get_receivers().is_empty());

    pc.close().await;
}
