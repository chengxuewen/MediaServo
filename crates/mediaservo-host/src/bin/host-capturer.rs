//! host-capturer: 采集进程（Task C1）— 相机视频 → FrameBus 发布（I420）。
//!
//! 用法: `host-capturer --camera <id> --config <host.yaml 路径> --token <令牌文件路径>`
//!
//! 流程: 读 host.yaml 视频源配置（mode/width/height/fps，缺省 generator/1280x720/30）→ mediaservo-media
//! `VideoFrameGenerator` 产帧（generator 模式；C17 单调时钟由 generator 保证）→
//! link `FrameBus` 发布 topic `camera/<id>`（payload = FrameMeta 36B + 紧凑 I420，
//! 与 deck closed_loop 线格式一致）。mode 分派: generator 真实现；camera（v4l2/mipi
//! 采集后端）/ desktop / subscriber 本期未实现——明确报错退出（不静默降级）。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mediaservo_host::translate::SourceMode;
use mediaservo_link::{FrameBus, FrameMeta, FrameTopic, TokenFile};
use mediaservo_media::base::buffer::VideoBuffer;
use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::error::MediaError;
use mediaservo_media::pipeline::generator::{
    BitmapFont, ColorStrategy, PatternMode, SquaresConfig, TextBurner,
    TimestampFormat, TimestampOverlay, VideoFrameGenerator,
};
use mediaservo_media::pipeline::generator::fonts::Anchor;
use mediaservo_media::pipeline::source::VideoSource;
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};

/// FrameMeta 像素格式: 1 = I420（D243 枚举）。
const FORMAT_I420: u8 = 1;

const USAGE: &str = "用法: host-capturer --camera <id> --config <host.yaml> --token <令牌文件>";

struct Args {
    camera: String,
    config: PathBuf,
    token: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut camera: Option<String> = None;
    let mut config: Option<PathBuf> = None;
    let mut token: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--camera" => camera = Some(args.next().ok_or("--camera 缺值")?),
            "--config" => config = Some(PathBuf::from(args.next().ok_or("--config 缺值")?)),
            "--token" => token = Some(PathBuf::from(args.next().ok_or("--token 缺值")?)),
            _ => return Err(format!("未知参数: {arg}")),
        }
    }
    Ok(Args {
        camera: camera.ok_or("缺少 --camera")?,
        config: config.ok_or("缺少 --config")?,
        token: token.ok_or("缺少 --token")?,
    })
}

/// FrameBus 发布 sink：generator 线程 → publish（payload = FrameMeta + 紧凑 I420）。
/// generator 忽略 sink 返回的 Err，发布失败在此打日志（C15，不静默丢帧）。
struct FrameBusSink {
    bus: Arc<FrameBus>,
    topic: FrameTopic,
    seq: AtomicU64,
    fps: u32,
}

impl VideoSink<BoxVideoFrame> for FrameBusSink {
    fn on_frame(&self, frame: &BoxVideoFrame) -> Result<VideoSinkWants, MediaError> {
        let buf = frame
            .buffer
            .as_i420()
            .ok_or_else(|| MediaError::Internal("frame buffer not I420".into()))?;
        debug_assert_eq!(
            buf.stride_y, buf.width(),
            "generator buffer 应为紧凑布局（payload 线格式假设）"
        );
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let meta = FrameMeta {
            seq,
            width: buf.width(),
            height: buf.height(),
            format: FORMAT_I420,
            version: 1,
            is_keyframe: seq.is_multiple_of(u64::from(self.fps)),
            ts_mono_ns: (frame.timestamp_us.max(0) as u64) * 1000,
            ts_epoch_ns: 0,
        };
        let mut payload = Vec::with_capacity(buf.data_y.len() + buf.data_u.len() + buf.data_v.len());
        payload.extend_from_slice(&buf.data_y);
        payload.extend_from_slice(&buf.data_u);
        payload.extend_from_slice(&buf.data_v);
        if let Err(e) = self.bus.publish(&self.topic, &payload, &meta) {
            tracing::error!(topic = %self.topic.as_str(), seq, "publish failed: {e}");
        }
        Ok(VideoSinkWants::default())
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("capturer");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    // 视频源配置（mode/width/height/fps；缺省 generator/1280x720/30）
    let cfg_text = match std::fs::read_to_string(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("capturer: 读取配置 {} 失败: {e}", args.config.display());
            return ExitCode::from(1);
        }
    };
    let cam = match mediaservo_host::translate::camera_config(&cfg_text, &args.camera) {
        Ok(Some(c)) => c,
        Ok(None) => {
            eprintln!("capturer: 配置中无视频源 {}", args.camera);
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("capturer: {e}");
            return ExitCode::from(1);
        }
    };
    // mode 分派: generator → VideoFrameGenerator 真实产帧；camera/desktop/subscriber
    // 采集后端未实现 → 明确报错退出（C15，不静默降级）。
    if cam.mode != SourceMode::Generator {
        let reason = match cam.mode {
            SourceMode::Camera => "camera 采集未实现（backend=v4l2/mipi 后接）",
            SourceMode::Desktop => "desktop 采集未实现（屏幕捕获后接）",
            SourceMode::Subscriber => "subscriber 消费未实现（外部源订阅后接）",
            SourceMode::Generator => "unreachable",
        };
        eprintln!(
            "capturer: 视频源 {} mode={} 未支持: {}（本期仅 generator 可用）",
            cam.id,
            cam.mode.as_str(),
            reason
        );
        return ExitCode::from(1);
    }

    // 令牌 → FrameBus attach（验签失败/过期/缺失均明确报错退出）
    let token_bytes = match std::fs::read(&args.token) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("capturer: 读取令牌 {} 失败: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let (verifying_key, token) = match TokenFile::decode(&token_bytes) {
        Ok(kv) => kv,
        Err(e) => {
            eprintln!("capturer: 令牌 {} 无效: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let bus = match FrameBus::attach("", &token, &verifying_key) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("capturer: FrameBus attach 失败: {e}");
            return ExitCode::from(1);
        }
    };
    let topic = FrameTopic::new(format!("camera/{}", cam.id));
    // ponytail: 固定 topic 跨进程重开依赖残留清理（C25 只覆盖测试；生产机制归 C2 计划）。

    // 产帧 + 发布（generator owned 于 main 作用域，PIT-81）
    let generator = VideoFrameGenerator::new();
    generator.add_or_update_sink(
        Box::new(FrameBusSink {
            bus: Arc::new(bus),
            topic: topic.clone(),
            seq: AtomicU64::new(0),
            fps: cam.fps,
        }),
        VideoSinkWants::default(),
    );
    generator.start(
        cam.fps,
        PatternMode::Squares(SquaresConfig {
            count: 16,
            min_size: 32,
            max_size: 96,
            motion_speed: 3,
            color_strategy: ColorStrategy::RandomPerSquare,
        }),
        Some(TimestampOverlay::new(
            TextBurner::new(BitmapFont::new(), false, Anchor::TopLeft),
            TimestampFormat::Combined,
        )),
        cam.width,
        cam.height,
    );
    println!(
        "capturer ready: topic={} {}x{}@{} source=generator",
        topic.as_str(),
        cam.width,
        cam.height,
        cam.fps
    );

    // 等待 SIGINT/SIGTERM → 优雅停止
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("capturer: 注册 SIGTERM 处理器失败: {e}");
                return ExitCode::from(1);
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("ctrl_c handler");
    }

    generator.stop();
    tracing::info!(topic = %topic.as_str(), "capturer stopped");
    ExitCode::from(0)
}
