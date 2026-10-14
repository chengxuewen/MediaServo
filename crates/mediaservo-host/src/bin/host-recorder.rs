//! host-recorder: recording process (Task C3) — FrameBus subscribes camera/<id> → deck Recorder writes files.
//!
//! Usage: `host-recorder --config <host.yaml path> --token <token file path>`
//!
//! Flow: read host.yaml (`[record]` enabled gate + out_dir; defaults disabled +
//! /tmp/mediaservo-recordings) → one recording task per video source: link `FrameBus` subscribes
//! `camera/<id>` (FrameMeta + packed I420, C1 capturer wire format) → deck `Recorder`
//! (H264 + MP4 mux, reusing the deck closed_loop verified pipeline) → `{out_dir}/{id}.mp4`.
//! SIGTERM → stop all Recorders (flush + trailer, worker finishes mux).
//!
//! Watchdog decision (C5 crash-recovery alignment): the recorder does **not** exit on frame
//! starvation — recording must survive capturer restarts (frames are sparse while stopped; pts
//! gaps are preserved by MP4 timestamps). Beyond 10s without frames only a stall warning is
//! logged (inside the `LinkFrames` adapter); at SIGTERM, even 0 frames finalizes cleanly.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mediaservo_codec::codec::PixelFormat;
use mediaservo_codec::frame::{Plane, VideoFrame};
use mediaservo_deck::record::{Container, Frames, RecordOptions, Recorder, VideoCodec};
use mediaservo_link::{FrameBus, FrameMeta, FrameTopic, TokenFile};

/// FrameMeta pixel format: 1 = I420 (D243 enum, matches C1 capturer).
const FORMAT_I420: u8 = 1;
/// No-frame stall warning interval (recorder does not exit — C5: survive capturer restarts).
const STALL_WARN_INTERVAL: Duration = Duration::from_secs(10);
/// Keyframe interval = frame count (1s, aligned with the capturer is_keyframe cadence).
const KEYFRAME_INTERVAL_SECS: u32 = 1;
/// Timeout for recording-task finish (flush + trailer) after SIGTERM.
const FINISH_TIMEOUT: Duration = Duration::from_secs(30);

const USAGE: &str = "Usage: host-recorder --config <host.yaml> --token <token file>";

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
            "--config" => {
                config = Some(PathBuf::from(args.next().ok_or("--config missing value")?))
            }
            "--token" => token = Some(PathBuf::from(args.next().ok_or("--token missing value")?)),
            _ => return Err(format!("unknown arg: {arg}")),
        }
    }
    Ok(Args { config: config.ok_or("missing --config")?, token: token.ok_or("missing --token")? })
}

/// Packed I420 payload validation (wire-format assumption: tight strides Y + U + V).
fn valid_i420(meta: &FrameMeta, payload_len: usize) -> bool {
    meta.format == FORMAT_I420
        && meta.width.is_multiple_of(2)
        && meta.height.is_multiple_of(2)
        && payload_len == (meta.width * meta.height * 3 / 2) as usize
}

/// payload → VideoFrame (flat I420; same wire format as deck closed_loop).
/// Invalid frames return None (caller skips and logs, C15 — never drop silently).
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

/// link FrameStream → deck `Frames` adapter (closed_loop reference pattern + stall warning).
///
/// Watchdog decision (C5 crash-recovery alignment): no frames does **not** exit — the recorder
/// must survive capturer restarts (frames are sparse while stopped; pts gaps are preserved by MP4
/// timestamps); once the capturer resumes publishing, frames continue recording automatically (the
/// FrameBus subscription stays alive). Only stream closure (recv None) ends the recording.
///
/// Stall detection site (review #1 fix): deck `Recorder::record`'s frame pump polls `next()` every
/// 50ms (outer timeout) — **do not wrap recv in an inner timeout inside next** (the outer 50ms
/// would cancel the inner 10s wait → never fires). Instead evaluate `last_frame.elapsed()` at each
/// pump tick: >10s logs a warn (throttled to 1/10s against spam) and keeps waiting.
struct LinkFrames {
    stream: mediaservo_link::FrameStream,
    topic: String,
    last_frame: Instant,
    last_warn: Instant,
}

impl Frames for LinkFrames {
    async fn next(&mut self) -> Option<VideoFrame> {
        loop {
            // pump tick (~50ms each): evaluate idle duration, >10s warns (throttled; stall only logs)
            let idle = self.last_frame.elapsed();
            if idle >= STALL_WARN_INTERVAL && self.last_warn.elapsed() >= STALL_WARN_INTERVAL {
                self.last_warn = Instant::now();
                tracing::warn!(
                    topic = %self.topic,
                    idle_secs = idle.as_secs(),
                    "no frames — capturer not running? recording continues (C3 watchdog: no exit)"
                );
            }
            // direct recv (no inner timeout — the outer 50ms pump polling handles wakeup/cancel)
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

/// Error path (review #2): stop all already-created recording tasks and wait for them to finish
/// (flush + trailer), avoiding leftover truncated mp4s without a trailer.
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

/// Wait for SIGINT/SIGTERM (unix main path; other platforms only ctrl_c).
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
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

    // [record] enabled gate (before token read — when disabled, exit 0 with no credentials at all)
    let cfg_text = match std::fs::read_to_string(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("recorder: read config {} failed: {e}", args.config.display());
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
        println!("recorder: [record] enabled=false — recording not enabled, exiting");
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
        eprintln!("recorder: [record] enabled but host.yaml has no sources — nothing to subscribe");
        return ExitCode::from(1);
    }

    // token → FrameBus attach (verification failure / expiry / missing all exit with an explicit error)
    let token_bytes = match std::fs::read(&args.token) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("recorder: read token {} failed: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let (verifying_key, token) = match TokenFile::decode(&token_bytes) {
        Ok(kv) => kv,
        Err(e) => {
            eprintln!("recorder: token {} invalid: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let bus = match FrameBus::attach("", &token, &verifying_key) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("recorder: FrameBus attach failed: {e}");
            return ExitCode::from(1);
        }
    };

    // output dir (deck Recorder::new requires the parent dir — create explicitly, failures visible)
    if let Err(e) = std::fs::create_dir_all(&rec.out_dir) {
        eprintln!("recorder: create output dir {} failed: {e}", rec.out_dir.display());
        return ExitCode::from(1);
    }

    // per video source: subscribe camera/<id> + deck Recorder file task (records until SIGTERM)
    let mut stops: Vec<mediaservo_deck::record::StopSignal> = Vec::new();
    let mut tasks: Vec<tokio::task::JoinHandle<Result<(), mediaservo_deck::DeckError>>> =
        Vec::new();
    let mut ids: Vec<String> = Vec::new();
    for cam in &cams {
        let topic = FrameTopic::new(format!("camera/{}", cam.id));
        let frames = match bus.subscribe(&topic) {
            Ok(f) => f,
            Err(e) => {
                // Review #2: already-created recording tasks must finish (flush + trailer), else truncated mp4s without trailer remain
                stop_and_finish(&stops, tasks).await;
                eprintln!("recorder: subscribe {} failed: {e}", topic.as_str());
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
                // Review #2: same — stop previously created camera recorders before exiting
                stop_and_finish(&stops, tasks).await;
                eprintln!("recorder: create recorder {} failed: {e}", path.display());
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
        tasks.push(tokio::spawn(async move { recorder.record(link_frames).await }));
        stops.push(stop);
        ids.push(cam.id.clone());
        tracing::info!(topic = %topic.as_str(), out = %path.display(), "recording source");
    }

    println!("recorder ready: sources={ids:?} out={}", rec.out_dir.display());

    // wait for SIGINT/SIGTERM → stop all (flush + trailer) → graceful exit 0
    match shutdown_signal().await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("recorder: signal handling failed: {e}");
            return ExitCode::from(1);
        }
    }
    for s in &stops {
        s.stop();
    }
    for t in tasks {
        if tokio::time::timeout(FINISH_TIMEOUT, t).await.is_err() {
            tracing::error!("recording task did not finish within {FINISH_TIMEOUT:?} (mux stuck?)");
            return ExitCode::from(1);
        }
    }
    tracing::info!("recorder stopped");
    ExitCode::from(0)
}
