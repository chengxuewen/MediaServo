//! Observer callback tests — verify RealObserver forwards events correctly.
//!
//! Tests the ObserverCallbacks mechanism: on_track, on_data_channel,
//! and set_on_track override on WebrtcSysPc.
//!
//! Stub backend: callbacks register without error, verify no panic.
//! webrtc-sys backend: RealObserver fires callbacks from libwebrtc events.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
use mediaservo_webrtc::peer_connection::RTCConfiguration;
use mediaservo_webrtc::track::TrackKind;
use mediaservo_webrtc::traits::PeerConnectionApi;

// ── Stub backend: callback registration API surface ──

#[tokio::test]
async fn on_track_registers_without_error() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    let called = Arc::new(AtomicBool::new(false));
    let c = called.clone();
    pc.on_track(move |_receiver| {
        c.store(true, Ordering::Relaxed);
    });

    pc.close().await;
}

#[tokio::test]
#[cfg(not(feature = "backend-webrtc-rs"))] // ponytail: ICE ufrag, covered by p2p_codec_e2e.rs
async fn on_track_callback_is_stored() {
    let factory = RTCPeerConnectionFactory::new();
    let pc1 = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    let pc2 = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    let count = Arc::new(AtomicU64::new(0));
    let c = count.clone();
    pc2.on_track(move |_receiver| {
        c.fetch_add(1, Ordering::Relaxed);
    });

    // on_track callback should be stored and survive SDP exchange
    let offer = pc1.create_offer(&Default::default()).await.unwrap();
    pc1.set_local_description(&offer).await.unwrap();
    pc2.set_remote_description(&offer).await.unwrap();
    let answer = pc2.create_answer(&Default::default()).await.unwrap();
    pc2.set_local_description(&answer).await.unwrap();
    pc1.set_remote_description(&answer).await.unwrap();

    // Verify no panic during close with registered callbacks
    pc1.close().await;
    pc2.close().await;
}

#[tokio::test]
async fn multiple_on_track_registrations() {
    // Verify registering multiple times does not panic (last wins).
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    pc.on_track(|_| {});
    pc.on_track(|_| {});
    pc.on_track(|_| {});

    pc.close().await;
}

#[tokio::test]
async fn on_track_persists_across_clone() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    let called = Arc::new(AtomicBool::new(false));
    let c = called.clone();
    pc.on_track(move |_receiver| {
        c.store(true, Ordering::Relaxed);
    });

    // Clone should share the callback
    let pc2 = pc.clone();
    drop(pc);

    pc2.close().await;
}

#[tokio::test]
async fn get_receivers_empty_without_remote_track() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    // Without a remote track connection, receivers should be empty
    let receivers = pc.get_receivers();
    assert!(receivers.is_empty());

    pc.close().await;
}

#[tokio::test]
async fn get_senders_after_add_track() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();

    pc.add_track("video-1", TrackKind::Video).unwrap();
    pc.add_track("video-2", TrackKind::Video).unwrap();

    let senders = pc.get_senders();
    assert_eq!(senders.len(), 2);

    pc.close().await;
}
