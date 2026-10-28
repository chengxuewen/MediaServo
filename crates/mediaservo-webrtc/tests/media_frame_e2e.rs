use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
use mediaservo_webrtc::peer_connection::RTCConfiguration;
use mediaservo_webrtc::track::{FrameSink, TrackKind, TrackRef};
use mediaservo_webrtc::traits::PeerConnectionApi;
use std::sync::atomic::{AtomicU64, Ordering};
mod common;

struct CountingSink {
    count: AtomicU64,
}
impl CountingSink {
    fn new() -> Self {
        Self { count: AtomicU64::new(0) }
    }
}
impl FrameSink for CountingSink {
    fn on_frame(&self, _: &[u8], _: u32, _: u32) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn write_raw_i420_does_not_panic() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    let sender = factory.create_video_track("video-1");
    pc.add_track("video-1", TrackKind::Video).unwrap();
    let frame = common::loopback::generate_test_frame(320, 240, 0);
    sender.write_raw_i420(&frame, 320, 240).await.unwrap();
    pc.close().await;
}

#[tokio::test]
async fn create_video_track_and_write_frames() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    let track_sender = factory.create_video_track("camera-1");
    pc.add_track("camera-1", TrackKind::Video).unwrap();
    for i in 0..10 {
        let frame = common::loopback::generate_test_frame(320, 240, i);
        track_sender.write_raw_i420(&frame, 320, 240).await.unwrap();
    }
    pc.close().await;
}

#[tokio::test]
async fn on_track_callback_registers() {
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    pc.on_track(|_receiver| {});
    pc.close().await;
}

#[tokio::test]
#[cfg(not(feature = "backend-webrtc-rs"))] // ponytail: ICE ufrag issue on webrtc-rs, covered by p2p_codec_e2e.rs
async fn frame_sink_and_p2p_exchange() {
    let factory = RTCPeerConnectionFactory::new();
    let pc1 = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    let pc2 = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    let sender = factory.create_video_track("test-video");
    pc1.add_track("test-video", TrackKind::Video).unwrap();
    pc2.on_track(move |receiver| {
        if let TrackRef::Receiver(ref track_receiver) = receiver.track {
            track_receiver.set_frame_sink(Box::new(CountingSink::new()));
        }
    });
    common::loopback::exchange_sdp(&pc1, &pc2).await.unwrap();
    for i in 0..5 {
        let frame = common::loopback::generate_test_frame(320, 240, i);
        sender.write_raw_i420(&frame, 320, 240).await.unwrap();
    }
    pc1.close().await;
    pc2.close().await;
}
