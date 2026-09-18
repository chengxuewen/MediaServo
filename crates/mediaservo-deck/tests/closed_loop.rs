//! deck 闭环 e2e：CameraSource → FrameBus 传输 → Recorder 落盘。
//!
//! 验证最小闭环"采集→传输→落盘"（用户确认的 Phase 2 范围）。
//! 需要 `backend-ffmpeg` + link FrameBus 能力。

#![cfg(feature = "backend-ffmpeg")]

use std::time::Duration;

use mediaservo_codec::codec::PixelFormat;
use mediaservo_codec::frame::VideoFrame;
use mediaservo_deck::record::{RecordOptions, Recorder, VideoCodec};
use mediaservo_deck::source::{CameraSource, CaptureOptions, DeviceId};
use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role,
};

const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// VideoFrame(I420) → 扁平 payload（YkV+kU+kV 连续）。
fn frame_to_payload(f: &VideoFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(f.plane_data(0).unwrap().len() * 3 / 2);
    for i in 0..3 {
        out.extend_from_slice(f.plane_data(i).unwrap());
    }
    out
}

/// payload → VideoFrame（flat I420，假设紧凑 stride == width）。
fn payload_to_frame(meta: &FrameMeta, payload: &[u8]) -> VideoFrame {
    let w = meta.width as usize;
    let h = meta.height as usize;
    let y = w * h;
    let uv = (w / 2) * (h / 2);
    VideoFrame {
        format: mediaservo_codec::codec::VideoFormat {
            width: meta.width,
            height: meta.height,
            pixel_format: PixelFormat::Yuv420p,
        },
        planes: vec![
            mediaservo_codec::frame::Plane { data: payload[..y].to_vec(), stride: w as u32 },
            mediaservo_codec::frame::Plane {
                data: payload[y..y + uv].to_vec(),
                stride: (w / 2) as u32,
            },
            mediaservo_codec::frame::Plane {
                data: payload[y + uv..y + 2 * uv].to_vec(),
                stride: (w / 2) as u32,
            },
        ],
        pts: meta.ts_mono_ns / 1000,
        keyframe: meta.is_keyframe,
    }
}

#[tokio::test]
async fn camera_framebus_recorder_roundtrip() {
    let suffix = std::process::id();
    let topic = FrameTopic::new(format!("camera/deck-e2e/{suffix}/raw"));
    let dir = std::env::temp_dir().join("deck-closed-loop");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("loop.mp4");
    let _ = std::fs::remove_file(&path);

    // ── 采集侧（Capture）────────────────────────────
    let (tok_cap, vk_cap) = token(Role::Capture, "deck-capture");
    let cap_bus = FrameBus::attach("", &tok_cap, &vk_cap).expect("capture attach");
    let mut cam =
        CameraSource::open(DeviceId("stub:test-camera".into()), CaptureOptions::default())
            .expect("open cam");
    let opts = CaptureOptions { resolution: Some((320, 240)), framerate: Some(30), format: None };
    let mut cam_frames = cam.start(&opts).expect("cam start");

    // 帧发布泵：CameraSource 帧 → FrameBus publish（flat I420）。
    // FrameBus 非 Clone — 发布泵持有 Arc（publish 是 &self）。
    // 结束条件：源帧结束 OR 停止信号（与 recorder 同模式）。
    let pub_bus = std::sync::Arc::new(cap_bus);
    let pub_topic = topic.clone();
    let pub_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let pub_stop_task = std::sync::Arc::clone(&pub_stop);
    let pub_task = tokio::spawn(async move {
        let mut seq: u64 = 0;
        loop {
            if pub_stop_task.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(50), cam_frames.recv()).await {
                Ok(Some(f)) => {
                    let meta = FrameMeta {
                        seq,
                        width: f.format.width,
                        height: f.format.height,
                        format: 1, // I420
                        version: FrameMeta::WIRE_VERSION,
                        is_keyframe: seq.is_multiple_of(30),
                        ts_mono_ns: f.pts * 1000,
                        ts_epoch_ns: 0,
                    };
                    let payload = frame_to_payload(&f);
                    if let Err(e) = pub_bus.publish(&pub_topic, &payload, &meta) {
                        tracing::error!("publish failed: {e}");
                        break;
                    }
                    seq += 1;
                }
                Ok(None) => break,
                Err(_elapsed) => {}
            }
        }
    });

    // ── 录制侧（Recorder 角色，订阅 topic + 落盘）────────
    // 说明：设置中 define Recorder 消费的 Frames 来自 FrameBus 订阅。
    // 用自定义 Frames 实现（link FrameStream 是同步 recv — 需转 async）。
    let (tok_rec, vk_rec) = token(Role::Pusher, "deck-recorder"); // Pusher 订阅 video/* camera/*
    let rec_bus = FrameBus::attach("", &tok_rec, &vk_rec).expect("recorder attach");
    let link_stream = rec_bus.subscribe(&topic).expect("subscribe");

    // async 帧桥：link FrameStream（tokio receiver, async recv）→
    // deck Recorder 的 Frames 接口。delegate 一次取一帧转 VideoFrame。
    // Recorder::record 消费 impl Frames（async next()）——
    // 桥接直接复用 mediaservo-link FrameStream 的 as_async。
    let mut recorder = Recorder::new(
        &path,
        RecordOptions {
            codec: VideoCodec::H264,
            container: mediaservo_deck::record::Container::Mp4,
            fps: 30,
            keyframe_interval: 30,
        },
    )
    .unwrap_or_else(|e| panic!("recorder: {e}"));
    let stop = recorder.stop_signal();
    let rec_task = tokio::spawn(async move {
        recorder.record(LinkFrames::new(link_stream)).await.expect("record ok");
    });

    // ── 驱动 1.5s → 停止 ──────────────────────────────
    tokio::time::sleep(Duration::from_millis(1800)).await;
    cam.stop();
    pub_stop.store(true, std::sync::atomic::Ordering::SeqCst); // 发布泵退出
    stop.stop(); // recorder pump 退出 → flush + trailer

    tokio::time::timeout(Duration::from_secs(10), pub_task)
        .await
        .expect("publisher finishes")
        .expect("publisher panicked");
    tokio::time::timeout(Duration::from_secs(10), rec_task)
        .await
        .expect("recorder finishes")
        .expect("recorder panicked");

    // ── 验证：MP4 完整有效 ─────────────────────────────
    let size = std::fs::metadata(&path).expect("file exists").len();
    assert!(size > 1_000, "mp4 size {size}");

    if std::env::var("DECK_KEEP").is_err() {
        let _ = std::fs::remove_file(&path);
    } else {
        eprintln!("kept: {path:?}");
    }
}

/// link FrameStream 封装成 deck Frames（async next() 适配）。
struct LinkFrames {
    stream: mediaservo_link::FrameStream,
}

impl LinkFrames {
    fn new(stream: mediaservo_link::FrameStream) -> Self {
        Self { stream }
    }
}

impl mediaservo_deck::record::Frames for LinkFrames {
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send {
        async {
            let f = self.stream.recv().await?;
            Some(payload_to_frame(f.meta(), f.payload()))
        }
    }
}
