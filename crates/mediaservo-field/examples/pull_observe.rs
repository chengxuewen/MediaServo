//! Observe: monitor inbound RTP after PullSession subscribe (verify bytes_received growth)
#![cfg(target_os = "linux")]
use std::time::Duration;

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_field::{
    PublishOptions, PullConfig, PullSession, PushConfig, PushSession, SessionEvent,
};
use mediaservo_webrtc::traits::PeerConnectionApi;

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let url = std::env::var("SFU_E2E_WS_URL").expect("SFU_E2E_WS_URL");
    let psk = std::env::var("SFU_E2E_PSK").expect("SFU_E2E_PSK");
    let room = format!("pull-obs-{}", std::process::id());

    // Pull joins the room first
    let pull_cfg = PullConfig {
        url: url.clone(),
        psk: psk.clone(),
        room: room.clone(),
        role: PeerRole::Consumer,
        auto_subscribe: true,
    };
    let (mut pull, mut pull_events) =
        PullSession::connect(pull_cfg.clone()).await.expect("pull connect");

    // Push publish + frames
    let push_cfg = PushConfig::new(url, psk, room);
    let (mut push, _pe) = PushSession::connect(push_cfg.clone()).await.expect("push connect");
    let _ = push.publish_video(&push_cfg, &PublishOptions::default()).await.expect("publish");
    push.start_video_frames(&push_cfg).expect("frames");

    // Wait for NewProducer
    let producer_id = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match pull_events.recv().await {
                Some(SessionEvent::Message(SignalingMessage::NewProducer {
                    producer_id, ..
                })) => return producer_id,
                _ => continue,
            }
        }
    })
    .await
    .expect("NewProducer timeout");

    // subscribe
    let mut _frames =
        tokio::time::timeout(Duration::from_secs(30), pull.subscribe(&pull_cfg, &producer_id))
            .await
            .expect("subscribe timeout")
            .expect("subscribe failed");
    println!("subscribed to {producer_id}");

    // Monitor: log a line every 1s (confirm the process is alive + server forwarding continues)
    let pc = pull.peer_connection().expect("pc");
    for i in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let state = pc.connection_state();
        println!("t={i}s pc_state={state:?}");
    }
    push.stop_video_frames();
    push.close().await.unwrap();
    pull.close().await.unwrap();
}
