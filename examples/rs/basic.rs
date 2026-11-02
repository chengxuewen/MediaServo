//! The client v2 minimal-loop example (also the orchestration-side live-chain acceptance vehicle).
//!
//! Flow: login (REST JWT) -> join -> wait for a video producer -> consume 1 frame ->
//! open the control channel, send steer(seq=1) -> wait 5s for the receipt -> exit.
//!
//! All endpoints/credentials come via environment variables (the no-defaults discipline):
//!   MSRTC_WS_URL   ws://<server>:<port>/ws   (required)
//!   MSRTC_ROOM     room ID                    (required)
//!   MSRTC_USER     account                    (required)
//!   MSRTC_PASS     password                   (required)
//!   MSRTC_LABEL    control channel label      (optional, default "chassis")
//!
//! Success exit 0; any step failing logs and exits 1.

use std::time::Duration;

use mediaservo_client::ClientConfig;
use mediaservo_client::{RoomSession, auth::login, config::http_base_from_signaling};
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

    // 1. login for the JWT (http_base derived from the ws URL)
    let http_base = http_base_from_signaling(&signaling_url)?;
    let token = login(&http_base, &user, &pass).await?;
    println!("login ok: user={} role={}", token.username, token.role);

    // 2. join (JWT via the Sec-WebSocket-Protocol header, carried by link)
    let cfg = ClientConfig {
        signaling_url,
        room_id,
        psk: None,
        jwt: Some(token.jwt),
        role: PeerRole::Consumer,
        // S4/T3.5: the e-stop HMAC pre-shared key (same value as the vehicle's MEDIASERVO_CONTROL_HMAC_KEY).
        hmac_key: std::env::var("MEDIASERVO_CONTROL_HMAC_KEY").ok().filter(|v| !v.is_empty()),
        identity_dir: None,
    };
    let mut session = RoomSession::connect(&cfg).await?;
    println!("joined room={} negotiated={}", session.room_id(), session.negotiated());

    // 3. consume one video path and take the first frame (the late-join NewProducer replay window <= 30s).
    //    MSRTC_SKIP_VIDEO=1: a pure control-loop verification (the vehicle room -- controller producers/consumption all live here).
    let skip_video = std::env::var("MSRTC_SKIP_VIDEO").is_ok_and(|v| v == "1");
    let producer = if skip_video {
        String::new()
    } else {
        session.wait_video_producer(Duration::from_secs(30)).await?
    };
    if !skip_video {
        println!("video producer discovered: {producer}");
    }
    if !skip_video {
        let mut frames = session.consume_video(&producer).await?;
        let frame = match tokio::time::timeout(Duration::from_secs(30), frames.recv()).await {
            Ok(Some(f)) => f,
            Ok(None) => return Err("frame stream closed".into()),
            Err(_) => {
                // S2c bisection predicate: on timeout dump the receive-side stats (packetsReceived>0 without decoding = the decode/mid surface;
                // 0 packets = the arrival/SRTP surface).
                for s in session.video_receiver_stats() {
                    println!("receiver-stats {s:?}");
                }
                return Err("timeout waiting first video frame".into());
            }
        };
        println!("first frame {}x{} ({}B I420)", frame.width, frame.height, frame.data.len());
    }

    // 4. control egress: open -> send -> ack
    let mut ctl = session.open_control(&[label.as_str()]).await?;
    println!("control open: labels={:?} producers={:?}", ctl.labels(), ctl.producer_ids());
    // the peer's (vehicle controller) consumption of the cockpit producer has event-chain latency = the same-seq resend is idempotent
    // (mediasoup does not buffer messages sent before attach; the D-H3 seq pairing); recv_ack_for skips stale acks.
    let seq = 1u64;
    let mut ack_opt = None;
    // S4 weaknet repayment metering: each attempt logs the send->ack latency (ms, -1 when unrecorded),
    // the last line `[rtt] a,b,c` is for scripts to grab (remote-burst/baseline distribution comparison).
    let mut rtts: Vec<i64> = Vec::new();
    for attempt in 0..12u64 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        let t0 = std::time::Instant::now();
        ctl.send(&label, seq, "steer", serde_json::json!({ "deg": 0.0 })).await?;
        ack_opt = ctl.recv_ack_for(seq, Duration::from_secs(5)).await.ok();
        rtts.push(match &ack_opt {
            Some(_) => t0.elapsed().as_millis().min(i64::MAX as u128) as i64,
            None => -1,
        });
        if ack_opt.is_some() {
            break;
        }
    }
    println!("[rtt] {}", rtts.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
    let ack = ack_opt.ok_or("timed out waiting for ControlAck (12 retries)")?;
    println!("ack seq={} result={}", ack.ack, ack.result);
    // S4/a4: the e-stop dual-path demo (triggered by MSRTC_ESTOP=1; the DC sig + WS audit copy + the vehicle actuation record).
    if std::env::var("MSRTC_ESTOP").is_ok_and(|v| v == "1") {
        session.emergency_stop(&mut ctl, &label, 900, serde_json::json!({"reason":"demo"})).await?;
        println!(
            "estop sent (sig={})",
            if session.hmac_key_debug_present() { "signed" } else { "unsigned" }
        );
        let estop_ack = ctl.recv_ack_for(900, Duration::from_secs(10)).await?;
        println!("estop ack result={}", estop_ack.result);
    }
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
