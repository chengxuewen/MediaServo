//! client v2 最小闭环样例（兼编排方活体链路验收载体）。
//!
//! 流程：登录（REST JWT）→ 入房 → 等视频 producer → 消费 1 帧 →
//! 开控制通道发 steer(seq=1) → 等回执 5s → 退出。
//!
//! 全部端点/凭证经环境变量传入（无默认值纪律）：
//!   MSRTC_WS_URL   ws://<server>:<port>/ws   （必需）
//!   MSRTC_ROOM     房间 ID                    （必需）
//!   MSRTC_USER     账号                       （必需）
//!   MSRTC_PASS     口令                       （必需）
//!   MSRTC_LABEL    控制通道 label             （可选，默认 "chassis"）
//!
//! 成功 exit 0；任何一步失败打日志 exit 1。

use std::time::Duration;

use mediaservo_client::{RoomSession, auth::login, config::http_base_from_signaling};
use mediaservo_client::ClientConfig;
use mediaservo_common::protocol::PeerRole;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match run().await {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("basic example FAILED: {e:#}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let signaling_url = required_env("MSRTC_WS_URL")?;
    let room_id = required_env("MSRTC_ROOM")?;
    let user = required_env("MSRTC_USER")?;
    let pass = required_env("MSRTC_PASS")?;
    let label = std::env::var("MSRTC_LABEL").unwrap_or_else(|_| "chassis".to_string());

    // 1. 登录换 JWT（http_base 由 ws URL 派生）
    let http_base = http_base_from_signaling(&signaling_url)?;
    let token = login(&http_base, &user, &pass).await?;
    println!("login ok: user={} role={}", token.username, token.role);

    // 2. 入房（JWT 经 Sec-WebSocket-Protocol 头，link 承载）
    let cfg = ClientConfig {
        signaling_url,
        room_id,
        psk: None,
        jwt: Some(token.jwt),
        role: PeerRole::Consumer,
    };
    let mut session = RoomSession::connect(&cfg).await?;
    println!("joined room={} negotiated={}", session.room_id(), session.negotiated());

    // 3. 消费一路视频并取首帧（late-join NewProducer 回放 ≤30s）
    let producer = session.wait_video_producer(Duration::from_secs(30)).await?;
    println!("video producer discovered: {producer}");
    let mut frames = session.consume_video(&producer).await?;
    let frame = tokio::time::timeout(Duration::from_secs(30), frames.recv())
        .await
        .map_err(|_| "timeout waiting first video frame")?
        .ok_or("frame stream closed")?;
    println!("first frame {}x{} ({}B I420)", frame.width, frame.height, frame.data.len());

    // 4. 控制出程：open → send → ack
    let mut ctl = session.open_control(&[label.as_str()]).await?;
    println!("control open: labels={:?} producers={:?}", ctl.labels(), ctl.producer_ids());
    ctl.send(&label, 1, "steer", serde_json::json!({ "deg": 0.0 })).await?;
    let ack = ctl.recv_ack(Duration::from_secs(5)).await?;
    println!("ack seq={} result={}", ack.ack, ack.result);
    if ack.ack != 1 {
        return Err(format!("ack seq mismatch: got {} want 1", ack.ack).into());
    }

    session.close().await?;
    println!("basic example OK");
    Ok(())
}

fn required_env(key: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(key).map_err(|_| format!("required env {key} not set").into())
}
