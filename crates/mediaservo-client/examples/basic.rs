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

    // 3. 消费一路视频并取首帧（late-join NewProducer 回放 ≤30s）。
    //    MSRTC_SKIP_VIDEO=1：纯控制回路验证（整车房间——controller  producers/消费皆在此）。
    let skip_video = std::env::var("MSRTC_SKIP_VIDEO").is_ok_and(|v| v == "1");
    let producer = if skip_video { String::new() } else { session.wait_video_producer(Duration::from_secs(30)).await? };
    if !skip_video { println!("video producer discovered: {producer}"); }
    let mut frame_opt = None;
    if !skip_video {
    let mut frames = session.consume_video(&producer).await?;
    let frame = match tokio::time::timeout(Duration::from_secs(30), frames.recv()).await {
        Ok(Some(f)) => f,
        Ok(None) => return Err("frame stream closed".into()),
        Err(_) => {
            // S2c 二分判据：超时即 dump 收侧 stats（packetsReceived>0 而不解码=解码/mid 面；
            // 0 包=到达性/SRTP 面）。
            for s in session.video_receiver_stats() {
                println!("receiver-stats {s:?}");
            }
            return Err("timeout waiting first video frame".into());
        }
    };
    println!("first frame {}x{} ({}B I420)", frame.width, frame.height, frame.data.len());
    frame_opt = Some(frame);
    }

    // 4. 控制出程：open → send → ack
    let mut ctl = session.open_control(&[label.as_str()]).await?;
    println!("control open: labels={:?} producers={:?}", ctl.labels(), ctl.producer_ids());
    // 对端（车端 controller）消费舱端 producer 存在 ~20s 事件链时延——demo 以
    // 5s 间隔重发 + 25s 窗收 ack（真实座舱为人手操作，天然覆盖该窗口）。
    // 同 seq 重发（车端 consumer attach 前的消息 mediasoup 不缓存会丢；D-H3 seq
    // 配对 = 重发幂等安全）。recv_ack_for 跳过错序旧 ack。
    let seq = 1u64;
    let mut ack_opt = None;
    for attempt in 0..12u64 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        ctl.send(&label, seq, "steer", serde_json::json!({ "deg": 0.0 })).await?;
        ack_opt = ctl.recv_ack_for(seq, Duration::from_secs(5)).await.ok();
        if ack_opt.is_some() {
            break;
        }
    }
    let ack = ack_opt.ok_or("timed out waiting for ControlAck (12 retries)")?;
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
