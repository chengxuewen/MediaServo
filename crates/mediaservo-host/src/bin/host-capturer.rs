//! host-capturer: capture process (Task C1) — camera video → FrameBus publish (I420).
//!
//! Usage: `host-capturer --camera <id> --config <host.yaml path> --token <token file path> [--reconnect-ms <ms>]`
//!
//! Flow: read host.yaml video source config (mode/width/height/fps/input/reconnect_ms) → dispatch by mode:
//! generator → mediaservo-media `VideoFrameGenerator` produces frames (C17 monotonic clock guaranteed by
//! the generator); camera → `V4l2Backend` capture (blocking DQBUF is the pacing — no added sleep_until;
//! open/read failures retry with reconnect_ms backoff). Both paths publish topic `camera/<id>` uniformly
//! (payload = FrameMeta 36B + packed I420, matching the deck closed_loop wire format). desktop / subscriber
//! unimplemented — explicit error exit (no silent downgrade, C36).

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
use mediaservo_media::pipeline::generator::fonts::Anchor;
use mediaservo_media::pipeline::generator::{
    BitmapFont, ColorStrategy, PatternMode, SquaresConfig, TextBurner, TimestampFormat,
    TimestampOverlay, VideoFrameGenerator,
};
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};
use mediaservo_media::pipeline::source::VideoSource;

#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
use mediaservo_media::pipeline::capture::v4l2::V4l2Backend;
#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
use mediaservo_media::pipeline::capture::{CaptureBackend, CaptureError};

/// FrameMeta pixel format: 1 = I420 (D243 enum).
const FORMAT_I420: u8 = 1;

/// Default reconnect interval after capture failure (ms; applies when host.yaml `reconnect_ms` is unset).
const DEFAULT_RECONNECT_MS: u64 = 5000;

const USAGE: &str = "Usage: host-capturer --camera <id> --config <host.yaml> --token <token file> [--reconnect-ms <ms>]";

struct Args {
    camera: String,
    config: PathBuf,
    token: PathBuf,
    /// Reconnect interval ms after capture failure (backoff on open failure / stream error; default 5000).
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
            "--camera" => camera = Some(args.next().ok_or("--camera missing value")?),
            "--config" => {
                config = Some(PathBuf::from(args.next().ok_or("--config missing value")?))
            }
            "--token" => token = Some(PathBuf::from(args.next().ok_or("--token missing value")?)),
            "--reconnect-ms" => {
                let v = args.next().ok_or("--reconnect-ms missing value")?;
                reconnect_ms =
                    Some(v.parse().map_err(|e| format!("--reconnect-ms parse failed: {e}"))?);
            }
            _ => return Err(format!("unknown arg: {arg}")),
        }
    }
    Ok(Args {
        camera: camera.ok_or("missing --camera")?,
        config: config.ok_or("missing --config")?,
        token: token.ok_or("missing --token")?,
        reconnect_ms: reconnect_ms.unwrap_or(DEFAULT_RECONNECT_MS),
    })
}

/// Capture backend selection (pure function, unit-testable). camera → V4l2 (input is the device path);
/// generator → as-is (not behind the trait — push model incompatible with pull semantics);
/// desktop/subscriber → unsupported (C15 error exit).
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
            Some(path) if !path.trim().is_empty() => {
                BackendKind::V4l2 { path: path.to_string(), width, height, fps }
            }
            _ => BackendKind::Unsupported(
                "camera requires an input device path (e.g. /dev/video0 or a stable /dev/v4l/by-id/ path)",
            ),
        },
        SourceMode::Desktop => {
            BackendKind::Unsupported("desktop capture not implemented (screen capture to follow)")
        }
        SourceMode::Subscriber => BackendKind::Unsupported(
            "subscriber consumption not implemented (external source subscription to follow)",
        ),
    }
}

/// FrameBus publish sink: generator thread → publish (payload = FrameMeta + packed I420).
/// The generator ignores Err returned by the sink; publish failures are logged here (C15, no silent frame drop).
struct FrameBusSink {
    bus: Arc<FrameBus>,
    topic: FrameTopic,
    seq: AtomicU64,
    fps: u32,
}

/// Publish one packed I420 frame (payload = FrameMeta + I420) — shared wire format for generator and v4l2 (DRY).
#[allow(clippy::too_many_arguments)] // capture pump parameter group (8 args after D274); bundling = separate refactor
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
        version: FrameMeta::WIRE_VERSION,
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
            buf.stride_y,
            buf.width(),
            "generator buffer must use packed layout (payload wire-format assumption)"
        );
        let mut payload =
            Vec::with_capacity(buf.data_y.len() + buf.data_u.len() + buf.data_v.len());
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

/// Idempotent reconnect loop for open(): failure → C15 log (with /dev/v4l/by-id/ and one-camera-per-instance hints) → backoff retry.
#[cfg(all(feature = "capture-v4l2", target_os = "linux"))]
fn reconnect_with_backoff(backend: &mut V4l2Backend, reconnect_ms: u64) {
    loop {
        match backend.open() {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!(
                    path = %backend.path(),
                    error = %e,
                    "capture open failed — retrying in {reconnect_ms}ms; check the stable /dev/v4l/by-id/ path; one camera per instance (multi-instance contention gives EBUSY, C32)"
                );
                std::thread::sleep(Duration::from_millis(reconnect_ms));
            }
        }
    }
}

/// V4L2 frame loop (blocking DQBUF is the pacing — no added sleep_until; generator keeps the C17 absolute timeline).
/// read failures: Convert errors drop the frame without exiting (transient errors must not kill the process);
/// others (Stream/Open/Format/Unsupported) → reconnect backoff (open() is idempotent, timestamp anchoring is
/// not re-sampled — ts stays continuous across reconnects, Momus H1).
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
                        "frame convert failed — frame dropped (transient error must not kill the process)"
                    );
                }
                _ => {
                    tracing::warn!(path = %backend.path(), error = %e, "capture error — reconnecting");
                    reconnect_with_backoff(&mut backend, reconnect_ms);
                }
            },
        }
    }
}

/// Wait for SIGINT/SIGTERM → graceful stop (shared by generator/v4l2).
async fn wait_for_shutdown() -> ExitCode {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("capturer: register SIGTERM handler failed: {e}");
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

    // video source config (mode/width/height/fps/input/reconnect_ms; defaults generator/1280x720/30)
    let cfg_text = match std::fs::read_to_string(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("capturer: read config {} failed: {e}", args.config.display());
            return ExitCode::from(1);
        }
    };
    let cam = match mediaservo_host::translate::camera_config(&cfg_text, &args.camera) {
        Ok(Some(c)) => c,
        Ok(None) => {
            eprintln!("capturer: no video source {} in config", args.camera);
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("capturer: {e}");
            return ExitCode::from(1);
        }
    };
    // dispatch by mode (pure function): camera → V4l2Backend; generator → as-is; desktop/subscriber → error exit
    let backend_kind =
        select_backend(cam.mode, cam.input.as_deref(), cam.width, cam.height, cam.fps);
    if let BackendKind::Unsupported(reason) = &backend_kind {
        eprintln!(
            "capturer: video source {} mode={} unsupported: {reason}",
            cam.id,
            cam.mode.as_str()
        );
        return ExitCode::from(1);
    }

    // token → FrameBus attach (verification failure / expiry / missing all exit with an explicit error)
    let token_bytes = match std::fs::read(&args.token) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("capturer: read token {} failed: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let (verifying_key, token) = match TokenFile::decode(&token_bytes) {
        Ok(kv) => kv,
        Err(e) => {
            eprintln!("capturer: token {} invalid: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let bus = match FrameBus::attach("", &token, &verifying_key) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("capturer: FrameBus attach failed: {e}");
            return ExitCode::from(1);
        }
    };
    let topic = FrameTopic::new(format!("camera/{}", cam.id));
    // ponytail: reopening a fixed topic across processes relies on residue cleanup (C25 covers tests only; production mechanism belongs to the C2 plan).

    match backend_kind {
        BackendKind::Generator => {
            // produce frames + publish (generator owned in main scope, PIT-81)
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
                // The frame loop runs on a plain thread (blocking DQBUF); signal arrives → main returns → process exit reaps it.
                // Not spawn_blocking: dropping the tokio runtime waits for blocking tasks → never exits.
                std::thread::spawn(move || {
                    run_v4l2_capture(backend, Arc::new(bus), topic.clone(), reconnect_ms)
                });
                wait_for_shutdown().await
            }
            #[cfg(not(all(feature = "capture-v4l2", target_os = "linux")))]
            {
                // Non-Linux / feature off: camera mode unavailable (avoids unused-variable warnings).
                let _ = (path.as_str(), width, height, fps);
                eprintln!(
                    "capturer: video source {} mode=camera requires Linux + capture-v4l2 feature (current platform unsupported)",
                    cam.id
                );
                ExitCode::from(1)
            }
        }
        BackendKind::Unsupported(_) => unreachable!("handled before dispatch"),
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

    // ── CLI args (T7) ──────────────────────────────────────

    #[test]
    fn parse_args_defaults_reconnect_ms_to_5000() {
        let a =
            parse_args_from(args(&["--camera", "cam0", "--config", "c", "--token", "t"])).unwrap();
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
        assert!(
            parse_args_from(args(&[
                "--camera",
                "cam0",
                "--config",
                "c",
                "--token",
                "t",
                "--reconnect-ms",
                "abc",
            ]))
            .is_err()
        );
        assert!(
            parse_args_from(args(&[
                "--camera",
                "cam0",
                "--config",
                "c",
                "--token",
                "t",
                "--reconnect-ms",
            ]))
            .is_err()
        );
    }
}
