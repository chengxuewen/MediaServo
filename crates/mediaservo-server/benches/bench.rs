//! MediaServo Server benchmarks — stdlib-only, CI-friendly.
//!
//! Run with: `cargo bench -p mediaservo-server`
//!
//! These benchmarks measure:
//! - SignalingServer construction time
//! - JSON serialization/deserialization throughput for signaling messages
//! - Prometheus metrics encoding
//!
//! ponytail: stdlib-only timing loop (no criterion). Good enough for CI
//! regression detection; add criterion if numbers become noisy.

use std::time::{Duration, Instant};

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_server::monitor::{monitor_router, SharedMetrics};
use mediaservo_server::signaling::SignalingServer;

// ── Benchmark runner (stdlib, no framework) ────────────────────────────────

/// Minimal benchmark loop: runs `closure` enough times to total `target`,
/// then prints iterations/s.  `target` is the wall-clock duration to aim for.
fn run_bench(name: &str, target: Duration, closure: impl Fn()) -> f64 {
    let start = Instant::now();
    let mut count = 0u64;
    while start.elapsed() < target {
        closure();
        count += 1;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let per_sec = count as f64 / elapsed;
    println!("{:<45} {:>12.0} it/s  ({} iterations in {:.3}s)", name, per_sec, count, elapsed);
    per_sec
}

fn main() {
    println!("=== MediaServo Server Benchmarks ===\n");

    // 1. SignalingServer construction
    // sfu-mediasoup 特征下构造函数需活体 SfuManager（mediasoup worker），bench 无法构造 —
    // 该 bench 仅在无 mediasoup 特征下运行（G2 顺手修 --tests --benches 编译挂）。
    #[cfg(not(feature = "sfu-mediasoup"))]
    run_bench("SignalingServer::new(65536)", Duration::from_millis(200), || {
        let _server = SignalingServer::new(65536, None);
    });

    // 2. RoomJoin serialization
    run_bench("SignalingMessage::RoomJoin serialize", Duration::from_millis(200), || {
        let msg = SignalingMessage::RoomJoin {
            room_id: "room-abc-123".into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
        };
        serde_json::to_string(&msg).unwrap();
    });

    // 3. RoomJoin deserialization
    run_bench("SignalingMessage::RoomJoin deserialize", Duration::from_millis(200), || {
        let json = r#"{"type":"room_join","room_id":"room-abc-123","peer_role":"host"}"#;
        serde_json::from_str::<SignalingMessage>(json).unwrap();
    });

    // 4. SDP serialization (larger payload)
    run_bench("SignalingMessage::Sdp serialize (large)", Duration::from_millis(200), || {
        let sdp_payload = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n".repeat(10);
        let msg = SignalingMessage::Sdp {
            room_id: "test-room".into(),
            target: Some("peer-1".into()),
            sdp: sdp_payload,
        };
        serde_json::to_string(&msg).unwrap();
    });

    // 5. Prometheus metrics encoding
    run_bench("Prometheus metrics encode", Duration::from_millis(200), || {
        use mediaservo_common::metrics::CoreMetrics;
        let metrics = SharedMetrics::new(CoreMetrics::new());
        metrics.active_connections.set(42);
        metrics.encode();
    });

    // 6. Health endpoint request round-trip（同 #1: sfu-mediasoup 特征下跳过）
    #[cfg(not(feature = "sfu-mediasoup"))]
    run_bench("/health GET round-trip", Duration::from_millis(200), || {
        let signaling = SignalingServer::new(65536, None);
        let app = monitor_router(signaling);
        use http::Request;
        use axum::body::Body;
        use tower::util::ServiceExt;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _result = rt.block_on(async {
            let req = Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await;
            drop(resp);
        });
    });

    println!("\n=== Benchmarks complete ===");
}
