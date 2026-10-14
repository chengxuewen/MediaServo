//! Observe: PushSession pushes frames for 10s — watch outbound-rtp network-level stats
#![cfg(target_os = "linux")]
use std::time::Duration;

use mediaservo_field::{PublishOptions, PushConfig, PushSession};
use mediaservo_webrtc::stats::RTCStats;
use mediaservo_webrtc::traits::PeerConnectionApi;

#[tokio::main]
async fn main() {
    let url = std::env::var("SFU_E2E_WS_URL").expect("SFU_E2E_WS_URL");
    let psk = std::env::var("SFU_E2E_PSK").expect("SFU_E2E_PSK");
    let room = format!("obs-{}", std::process::id());
    let cfg = PushConfig::new(url, psk, room);
    let (mut s, _ev) = PushSession::connect(cfg.clone()).await.expect("connect");
    let opts = PublishOptions::default();
    let _ = s.publish_video(&cfg, &opts).await.expect("publish");
    s.start_video_frames(&cfg).expect("frames");
    let pc = s.peer_connection().expect("pc");
    for i in 0..10 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let stats = pc.sender_get_stats("video");
        for st in &stats {
            if let RTCStats::OutboundRtp(o) = st {
                println!(
                    "t={i}s bytes_sent={} packets_sent={} frames_encoded={} fps={:.1}",
                    o.bytes_sent, o.packets_sent, o.frames_encoded, o.frames_per_second
                );
            }
        }
    }
    s.stop_video_frames();
    s.close().await.expect("close");
}
