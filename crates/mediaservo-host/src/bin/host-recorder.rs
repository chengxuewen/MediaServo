//! host-recorder: 录制进程（Task C3）— FrameBus 订阅 camera/<id> → deck Recorder 落盘。
//!
//! 用法: `host-recorder --config <host.yaml 路径> --token <令牌文件路径>`
//!
//! 流程: 读 host.yaml（`[record]` enabled 门控 + out_dir，缺省 disabled +
//! /tmp/mediaservo-recordings）→ 每相机一个录制任务：link `FrameBus` 订阅
//! `camera/<id>`（FrameMeta + 紧凑 I420，C1 capturer 线格式）→ deck `Recorder`
//! （H264 + MP4 mux，复用 deck closed_loop 已验证闭环）→ `{out_dir}/{id}.mp4`。
//! SIGTERM → 全部 Recorder stop（flush + trailer，worker 完成 mux）。
//!
//! 看门狗决策（C5 crash-recovery 对齐）: recorder **不设**无帧退出——录制必须
//! 存活过 capturer 重启（停帧期间帧稀疏，pts 时间间隙由 MP4 时间戳保留）；
//! 超过 10s 无帧仅打 stall 警告（`LinkFrames` 适配器内），SIGTERM 时 0 帧也正常收尾。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mediaservo_codec::codec::PixelFormat;
use mediaservo_codec::frame::{Plane, VideoFrame};
use mediaservo_deck::record::{Container, Frames, RecordOptions, Recorder, VideoCodec};
use mediaservo_link::{FrameBus, FrameMeta, FrameTopic, TokenFile};

/// FrameMeta 像素格式: 1 = I420（D243 枚举，与 C1 capturer 一致）。
const FORMAT_I420: u8 = 1;
/// 无帧 stall 警告间隔（recorder 不退出 — C5: 存活过 capturer 重启）。
const STALL_WARN_INTERVAL: Duration = Duration::from_secs(10);
/// 关键帧间隔 = 帧数（1s，对齐 capturer is_keyframe 节奏）。
const KEYFRAME_INTERVAL_SECS: u32 = 1;
/// SIGTERM 后录制任务收尾（flush + trailer）超时。
const FINISH_TIMEOUT: Duration = Duration::from_secs(30);

const USAGE: &str = "用法: host-recorder --config <host.yaml> --token <令牌文件>";

struct Args {
    config: PathBuf,
    token: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut config: Option<PathBuf> = None;
    let mut token: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = Some(PathBuf::from(args.next().ok_or("--config 缺值")?)),
            "--token" => token = Some(PathBuf::from(args.next().ok_or("--token 缺值")?)),
            _ => return Err(format!("未知参数: {arg}")),
        }
    }
    Ok(Args {
        config: config.ok_or("缺少 --config")?,
        token: token.ok_or("缺少 --token")?,
    })
}

/// 紧凑 I420 payload 校验（线格式假设: tight strides Y + U + V）。
fn valid_i420(meta: &FrameMeta, payload_len: usize) -> bool {
    meta.format == FORMAT_I420
        && meta.width.is_multiple_of(2)
        && meta.height.is_multiple_of(2)
        && payload_len == (meta.width * meta.height * 3 / 2) as usize
}

/// payload → VideoFrame（flat I420；deck closed_loop 同款线格式）。
/// 非法帧返回 None（调用方跳过并打日志，C15 — 绝不静默丢弃）。
fn payload_to_frame(meta: &FrameMeta, payload: &[u8]) -> Option<VideoFrame> {
    if !valid_i420(meta, payload.len()) {
        return None;
    }
    let w = meta.width as usize;
    let h = meta.height as usize;
    let y = w * h;
    let uv = (w / 2) * (h / 2);
    Some(VideoFrame {
        format: mediaservo_codec::codec::VideoFormat {
            width: meta.width,
            height: meta.height,
            pixel_format: PixelFormat::Yuv420p,
        },
        planes: vec![
            Plane { data: payload[..y].to_vec(), stride: w as u32 },
            Plane { data: payload[y..y + uv].to_vec(), stride: (w / 2) as u32 },
            Plane { data: payload[y + uv..y + 2 * uv].to_vec(), stride: (w / 2) as u32 },
        ],
        pts: meta.ts_mono_ns / 1000,
        keyframe: meta.is_keyframe,
    })
}

/// link FrameStream → deck `Frames` 适配器（closed_loop 参考模式 + stall 警告）。
///
/// 看门狗决策（C5 crash-recovery 对齐）: 无帧**不退出**——recorder 必须存活过
/// capturer 重启（停帧期间帧稀疏，pts 时间间隙由 MP4 时间戳保留）；capturer
/// 恢复发布后帧自动续录（FrameBus 订阅常驻）。仅流关停（recv None）才结束录制。
///
/// stall 检测位置（审查 #1 修复）: deck `Recorder::record` 的帧泵以 50ms 轮询
/// `next()`（外层 timeout）——**不能在 next 内再用内层 timeout 包 recv**（外层
/// 50ms 取消内层 10s 等待 → 永不触发）。改为在每次泵 tick 处评估
/// `last_frame.elapsed()`：>10s 打 warn（节流 1 次/10s 防刷屏）后继续等待。
struct LinkFrames {
    stream: mediaservo_link::FrameStream,
    topic: String,
    last_frame: Instant,
    last_warn: Instant,
}

impl Frames for LinkFrames {
    async fn next(&mut self) -> Option<VideoFrame> {
        loop {
            // 泵 tick（约 50ms/次）: 评估无帧时长，>10s 节流告警（stall 只打日志）
            let idle = self.last_frame.elapsed();
            if idle >= STALL_WARN_INTERVAL && self.last_warn.elapsed() >= STALL_WARN_INTERVAL {
                self.last_warn = Instant::now();
                tracing::warn!(
                    topic = %self.topic,
                    idle_secs = idle.as_secs(),
                    "无帧 — capturer 未运行? 录制继续（C3 看门狗: 不退出）"
                );
            }
            // 直接 recv（无内层 timeout — 外层 50ms 泵轮询负责唤醒/取消）
            let f = self.stream.recv().await?;
            let meta = f.meta();
            if let Some(frame) = payload_to_frame(meta, f.payload()) {
                self.last_frame = Instant::now();
                return Some(frame);
            }
            tracing::warn!(
                topic = %self.topic,
                seq = meta.seq,
                "invalid frame skipped (format={} {}x{} payload={})",
                meta.format, meta.width, meta.height, f.payload().len()
            );
        }
    }
}

/// 出错路径（审查 #2）: 停止已创建的全部录制任务并等其收尾（flush + trailer），
/// 避免残留无 trailer 的残缺 mp4。
async fn stop_and_finish(
    stops: &[mediaservo_deck::record::StopSignal],
    tasks: Vec<tokio::task::JoinHandle<Result<(), mediaservo_deck::DeckError>>>,
) {
    for s in stops {
        s.stop();
    }
    for t in tasks {
        let _ = tokio::time::timeout(FINISH_TIMEOUT, t).await;
    }
}

/// 等待 SIGINT/SIGTERM（unix 主路径；其他平台仅 ctrl_c）。
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("recorder");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    // [record] enabled 门控（先于令牌读取 — 禁用时无需任何凭据即退出 0）
    let cfg_text = match std::fs::read_to_string(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("recorder: 读取配置 {} 失败: {e}", args.config.display());
            return ExitCode::from(1);
        }
    };
    let rec = match mediaservo_host::translate::record_config(&cfg_text) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("recorder: {e}");
            return ExitCode::from(1);
        }
    };
    if !rec.enabled {
        println!("recorder: [record] enabled=false — 录制未启用，退出");
        return ExitCode::from(0);
    }
    let cams = match mediaservo_host::translate::camera_configs(&cfg_text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("recorder: {e}");
            return ExitCode::from(1);
        }
    };
    if cams.is_empty() {
        eprintln!("recorder: [record] enabled 但 host.yaml 无 [[cameras]] — 无 topic 可订阅");
        return ExitCode::from(1);
    }

    // 令牌 → FrameBus attach（验签失败/过期/缺失均明确报错退出）
    let token_bytes = match std::fs::read(&args.token) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("recorder: 读取令牌 {} 失败: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let (verifying_key, token) = match TokenFile::decode(&token_bytes) {
        Ok(kv) => kv,
        Err(e) => {
            eprintln!("recorder: 令牌 {} 无效: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let bus = match FrameBus::attach("", &token, &verifying_key) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("recorder: FrameBus attach 失败: {e}");
            return ExitCode::from(1);
        }
    };

    // 输出目录（deck Recorder::new 要求父目录存在 — 显式创建，失败可见）
    if let Err(e) = std::fs::create_dir_all(&rec.out_dir) {
        eprintln!("recorder: 创建输出目录 {} 失败: {e}", rec.out_dir.display());
        return ExitCode::from(1);
    }

    // 每相机: 订阅 camera/<id> + deck Recorder 落盘任务（持续录制至 SIGTERM）
    let mut stops: Vec<mediaservo_deck::record::StopSignal> = Vec::new();
    let mut tasks: Vec<tokio::task::JoinHandle<Result<(), mediaservo_deck::DeckError>>> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    for cam in &cams {
        let topic = FrameTopic::new(format!("camera/{}", cam.id));
        let frames = match bus.subscribe(&topic) {
            Ok(f) => f,
            Err(e) => {
                // 审查 #2: 已创建的录制任务必须收尾（flush + trailer），否则残留无 trailer 的残缺 mp4
                stop_and_finish(&stops, tasks).await;
                eprintln!("recorder: 订阅 {} 失败: {e}", topic.as_str());
                return ExitCode::from(1);
            }
        };
        let path = rec.out_dir.join(format!("{}.mp4", cam.id));
        let mut recorder = match Recorder::new(
            &path,
            RecordOptions {
                codec: VideoCodec::H264,
                container: Container::Mp4,
                fps: cam.fps,
                keyframe_interval: cam.fps * KEYFRAME_INTERVAL_SECS,
            },
        ) {
            Ok(r) => r,
            Err(e) => {
                // 审查 #2: 同上 — 停掉先前相机再退出
                stop_and_finish(&stops, tasks).await;
                eprintln!("recorder: 创建录制器 {} 失败: {e}", path.display());
                return ExitCode::from(1);
            }
        };
        let stop = recorder.stop_signal();
        let link_frames = LinkFrames {
            stream: frames,
            topic: topic.as_str().to_string(),
            last_frame: Instant::now(),
            last_warn: Instant::now(),
        };
        tasks.push(tokio::spawn(async move {
            recorder.record(link_frames).await
        }));
        stops.push(stop);
        ids.push(cam.id.clone());
        tracing::info!(topic = %topic.as_str(), out = %path.display(), "recording camera");
    }

    println!(
        "recorder ready: cameras={ids:?} out={}",
        rec.out_dir.display()
    );

    // 等待 SIGINT/SIGTERM → 全部 stop（flush + trailer）→ 优雅退出 0
    match shutdown_signal().await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("recorder: 信号处理失败: {e}");
            return ExitCode::from(1);
        }
    }
    for s in &stops {
        s.stop();
    }
    for t in tasks {
        if tokio::time::timeout(FINISH_TIMEOUT, t).await.is_err() {
            tracing::error!("录制任务 {FINISH_TIMEOUT:?} 内未收尾（mux 卡死?）");
            return ExitCode::from(1);
        }
    }
    tracing::info!("recorder stopped");
    ExitCode::from(0)
}
