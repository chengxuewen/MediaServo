//! host-capturer: 采集进程（Task C1）— 相机视频 → FrameBus 发布（I420）。
//!
//! 用法: `host-capturer --camera <id> --config <host.yaml 路径> --token <令牌文件路径> [--reconnect-ms <ms>]`
//!
//! 流程: 读 host.yaml 视频源配置（mode/width/height/fps/input/reconnect_ms）→ mode 分派:
//! generator → mediaservo-media `VideoFrameGenerator` 产帧（C17 单调时钟由 generator 保证）；
//! camera → `V4l2Backend` 采集（DQBUF 阻塞即节拍——不叠加 sleep_until；open/read 失败按
//! reconnect_ms 退避重试）。两路统一 publish topic `camera/<id>`（payload = FrameMeta 36B +
//! 紧凑 I420，与 deck closed_loop 线格式一致）。desktop / subscriber 未实现——明确报错退出
//! （不静默降级，C36）。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
use mediaservo_media::pipeline::capture::v4l2::V4l2Backend;
#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
use mediaservo_media::pipeline::capture::{CaptureBackend, CaptureError};

/// FrameMeta 像素格式: 1 = I420（D243 枚举）。
const FORMAT_I420: u8 = 1;

/// 采集失败重连间隔缺省（ms；host.yaml `reconnect_ms` 未配置时生效）。
const DEFAULT_RECONNECT_MS: u64 = 5000;

const USAGE: &str = "用法: host-capturer --camera <id> --config <host.yaml> --token <令牌文件> [--reconnect-ms <ms>]";

struct Args {
    camera: String,
    config: PathBuf,
    token: PathBuf,
    /// 采集失败重连间隔 ms（open 失败/流错误退避；缺省 5000）。
    reconnect_ms: u64,
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I: Iterator<Item = String>>(mut args: I) -> Result<Args, String> {
    let mut camera: Option<String> = None;
    let mut config: Option<PathBuf> = None;
    let mut token: Option<PathBuf> = None;
    let mut reconnect_ms: Option<u64> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--camera" => camera = Some(args.next().ok_or("--camera 缺值")?),
            "--config" => config = Some(PathBuf::from(args.next().ok_or("--config 缺值")?)),
            "--token" => token = Some(PathBuf::from(args.next().ok_or("--token 缺值")?)),
            "--reconnect-ms" => {
                let v = args.next().ok_or("--reconnect-ms 缺值")?;
                reconnect_ms = Some(
                    v.parse()
                        .map_err(|e| format!("--reconnect-ms 解析失败: {e}"))?,
                );
            }
            _ => return Err(format!("未知参数: {arg}")),
        }
    }
    Ok(Args {
        camera: camera.ok_or("缺少 --camera")?,
        config: config.ok_or("缺少 --config")?,
        token: token.ok_or("缺少 --token")?,
        reconnect_ms: reconnect_ms.unwrap_or(DEFAULT_RECONNECT_MS),
    })
}

/// 采集后端选择（纯函数，可单测）。camera → V4l2（input 为设备路径）；generator → 现状
/// （不进 trait——push 模型与 pull 语义不兼容）；desktop/subscriber → 未支持（C15 报错退出）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum BackendKind {
    V4l2 { path: String, width: u32, height: u32, fps: u32 },
    Generator,
    Unsupported(&'static str),
}

fn select_backend(
    mode: SourceMode,
    input: Option<&str>,
    width: u32,
    height: u32,
    fps: u32,
) -> BackendKind {
    match mode {
        SourceMode::Generator => BackendKind::Generator,
        SourceMode::Camera => match input {
            Some(path) if !path.trim().is_empty() => BackendKind::V4l2 {
                path: path.to_string(),
                width,
                height,
                fps,
            },
            _ => BackendKind::Unsupported(
                "camera 需 input 设备路径（如 /dev/video0 或 /dev/v4l/by-id/ 稳定路径）",
            ),
        },
        SourceMode::Desktop => BackendKind::Unsupported("desktop 采集未实现（屏幕捕获后接）"),
        SourceMode::Subscriber => BackendKind::Unsupported("subscriber 消费未实现（外部源订阅后接）"),
    }
}

/// FrameBus 发布 sink：generator 线程 → publish（payload = FrameMeta + 紧凑 I420）。
/// generator 忽略 sink 返回的 Err，发布失败在此打日志（C15，不静默丢帧）。
struct FrameBusSink {
    bus: Arc<FrameBus>,
    topic: FrameTopic,
    seq: AtomicU64,
    fps: u32,
}

/// 发布一帧紧凑 I420（payload = FrameMeta + I420）——generator 与 v4l2 共用线格式（DRY）。
fn publish_i420(
    bus: &FrameBus,
    topic: &FrameTopic,
    seq: &AtomicU64,
    fps: u32,
    width: u32,
    height: u32,
    ts_mono_ns: u64,
    data: &[u8],
) {
    let seq = seq.fetch_add(1, Ordering::Relaxed);
    let meta = FrameMeta {
        seq,
        width,
        height,
        format: FORMAT_I420,
        version: 1,
        is_keyframe: seq.is_multiple_of(u64::from(fps)),
        ts_mono_ns,
        ts_epoch_ns: 0,
    };
    if let Err(e) = bus.publish(topic, data, &meta) {
        tracing::error!(topic = %topic.as_str(), seq, "publish failed: {e}");
    }
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
        let mut payload = Vec::with_capacity(buf.data_y.len() + buf.data_u.len() + buf.data_v.len());
        payload.extend_from_slice(&buf.data_y);
        payload.extend_from_slice(&buf.data_u);
        payload.extend_from_slice(&buf.data_v);
        publish_i420(
            &self.bus,
            &self.topic,
            &self.seq,
            self.fps,
            buf.width(),
            buf.height(),
            (frame.timestamp_us.max(0) as u64) * 1000,
            &payload,
        );
        Ok(VideoSinkWants::default())
    }
}

/// open() 幂等重连循环：失败 → C15 日志（含 /dev/v4l/by-id/ 与一相机一实例提示）→ 退避重试。
#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
fn reconnect_with_backoff(backend: &mut V4l2Backend, reconnect_ms: u64) {
    loop {
        match backend.open() {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!(
                    path = %backend.path(),
                    error = %e,
                    "capture open 失败——{reconnect_ms}ms 后重试；请核对 /dev/v4l/by-id/ 稳定路径；一相机一实例（多实例抢占会 EBUSY，C32）"
                );
                std::thread::sleep(Duration::from_millis(reconnect_ms));
            }
        }
    }
}

/// V4L2 帧循环（DQBUF 阻塞即节拍——不叠加 sleep_until；generator 维持 C17 绝对时间轴）。
/// read 失败: 转换错误丢帧不退出（瞬时错误不杀进程）；其余（Stream/Open/Format/Unsupported）
/// → 重连退避（open() 幂等重调，时间戳锚定不重采——跨重连 ts 连续，Momus H1）。
#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
fn run_v4l2_capture(
    mut backend: V4l2Backend,
    bus: Arc<FrameBus>,
    topic: FrameTopic,
    reconnect_ms: u64,
) {
    reconnect_with_backoff(&mut backend, reconnect_ms);
    let seq = AtomicU64::new(0);
    loop {
        match backend.read_frame() {
            Ok(frame) => publish_i420(
                &bus,
                &topic,
                &seq,
                frame.fps,
                frame.width,
                frame.height,
                frame.ts_mono_ns,
                &frame.data,
            ),
            Err(e) => match &e {
                CaptureError::Convert(_) => {
                    tracing::warn!(
                        path = %backend.path(),
                        error = %e,
                        "帧转换失败——本帧丢弃（瞬时错误不杀进程）"
                    );
                }
                _ => {
                    tracing::warn!(path = %backend.path(), error = %e, "采集错误——重连中");
                    reconnect_with_backoff(&mut backend, reconnect_ms);
                }
            },
        }
    }
}

/// 等待 SIGINT/SIGTERM → 优雅停止（generator/v4l2 共用）。
async fn wait_for_shutdown() -> ExitCode {
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
    ExitCode::from(0)
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

    // 视频源配置（mode/width/height/fps/input/reconnect_ms；缺省 generator/1280x720/30）
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
    // mode 分派（纯函数）: camera → V4l2Backend；generator → 现状；desktop/subscriber → 报错退出
    let backend_kind =
        select_backend(cam.mode, cam.input.as_deref(), cam.width, cam.height, cam.fps);
    if let BackendKind::Unsupported(reason) = &backend_kind {
        eprintln!(
            "capturer: 视频源 {} mode={} 未支持: {reason}",
            cam.id,
            cam.mode.as_str()
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

    match backend_kind {
        BackendKind::Generator => {
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
            let exit = wait_for_shutdown().await;
            generator.stop();
            tracing::info!(topic = %topic.as_str(), "capturer stopped");
            exit
        }
        BackendKind::V4l2 { path, width, height, fps } => {
            println!(
                "capturer ready: topic={} {}x{}@{} source=camera path={}",
                topic.as_str(),
                width,
                height,
                fps,
                path
            );
            #[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
            {
                let reconnect_ms = args.reconnect_ms;
                let backend = V4l2Backend::new(&path, width, height, fps);
                // 帧循环跑普通线程（DQBUF 阻塞）；信号到达 → main 返回 → 进程退出回收。
                // 不用 spawn_blocking：tokio runtime drop 会等待 blocking 任务 → 永不退出。
                std::thread::spawn(move || {
                    run_v4l2_capture(backend, Arc::new(bus), topic.clone(), reconnect_ms)
                });
                wait_for_shutdown().await
            }
            #[cfg(not(all(feature = "capture-v4l2", target_os = "linux")))]
            {
                // 非 Linux / feature 关闭：camera 模式不可用（避免未用变量警告）。
                let _ = (path.as_str(), width, height, fps);
                eprintln!(
                    "capturer: 视频源 {} mode=camera 需 Linux + capture-v4l2 feature（当前平台不支持）",
                    cam.id
                );
                ExitCode::from(1)
            }
        }
        BackendKind::Unsupported(_) => unreachable!("已在分派前处理"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args<'a>(items: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        items.iter().map(|s| s.to_string())
    }

    #[test]
    fn select_backend_camera_with_input_is_v4l2() {
        let kind = select_backend(SourceMode::Camera, Some("/dev/v4l/by-id/cam0"), 1920, 1080, 30);
        assert_eq!(
            kind,
            BackendKind::V4l2 {
                path: "/dev/v4l/by-id/cam0".into(),
                width: 1920,
                height: 1080,
                fps: 30,
            }
        );
    }

    #[test]
    fn select_backend_camera_without_input_is_unsupported() {
        assert!(matches!(
            select_backend(SourceMode::Camera, None, 1920, 1080, 30),
            BackendKind::Unsupported(_)
        ));
        assert!(matches!(
            select_backend(SourceMode::Camera, Some("  "), 1920, 1080, 30),
            BackendKind::Unsupported(_)
        ));
    }

    #[test]
    fn select_backend_generator_is_generator() {
        assert_eq!(
            select_backend(SourceMode::Generator, None, 1280, 720, 30),
            BackendKind::Generator
        );
    }

    #[test]
    fn select_backend_desktop_subscriber_unsupported() {
        assert!(matches!(
            select_backend(SourceMode::Desktop, None, 0, 0, 30),
            BackendKind::Unsupported(_)
        ));
        assert!(matches!(
            select_backend(SourceMode::Subscriber, None, 0, 0, 30),
            BackendKind::Unsupported(_)
        ));
    }

    // ── CLI 参数（T7） ───────────────────────────────────────

    #[test]
    fn parse_args_defaults_reconnect_ms_to_5000() {
        let a = parse_args_from(args(&["--camera", "cam0", "--config", "c", "--token", "t"])).unwrap();
        assert_eq!(a.reconnect_ms, DEFAULT_RECONNECT_MS);
        assert_eq!(a.camera, "cam0");
    }

    #[test]
    fn parse_args_accepts_and_validates_reconnect_ms() {
        let a = parse_args_from(args(&[
            "--camera",
            "cam0",
            "--config",
            "c",
            "--token",
            "t",
            "--reconnect-ms",
            "3000",
        ]))
        .unwrap();
        assert_eq!(a.reconnect_ms, 3000);
        assert!(parse_args_from(args(&[
            "--camera", "cam0", "--config", "c", "--token", "t", "--reconnect-ms", "abc",
        ]))
        .is_err());
        assert!(parse_args_from(args(&[
            "--camera", "cam0", "--config", "c", "--token", "t", "--reconnect-ms",
        ]))
        .is_err());
    }
}
