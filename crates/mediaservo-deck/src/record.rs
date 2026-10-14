//! Record domain — Recorder: I420 frame stream → H264 encode → container mux to disk.
//!
//! MVP: FFmpeg backend (ffmpeg-the-third 6.0), H264 encoding + MP4 mux.
//! Entry point per contract §6: `Recorder::new(path, opts)` → `record(stream)` consumes
//! all frames (continuous recording); `stop()` finalizes (flush + trailer + join).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mediaservo_codec::frame::VideoFrame;

use crate::DeckError;

/// Container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mkv,
}

/// Recording options.
#[derive(Debug, Clone)]
pub struct RecordOptions {
    pub codec: VideoCodec,
    pub container: Container,
    #[allow(dead_code)]
    /// Target frame rate (container time_base and encoder input cadence).
    pub fps: u32,
    /// Keyframe interval (in frames).
    pub keyframe_interval: u32,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self { codec: VideoCodec::H264, container: Container::Mp4, fps: 30, keyframe_interval: 60 }
    }
}

/// Video codec (MVP: H264).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
}

/// Recorder: encodes every frame of the input stream and writes it to a container file (async recording task).
pub struct Recorder {
    path: PathBuf,
    opts: RecordOptions,
    running: Arc<AtomicBool>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

impl Recorder {
    /// Creates the recorder (not started; writing begins after `record`).
    pub fn new(path: impl Into<PathBuf>, opts: RecordOptions) -> Result<Self, DeckError> {
        let path = path.into();
        // Parent directory must exist (no implicit creation — fail loudly so the caller learns of the path problem)
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Err(DeckError::NotFound(format!(
                "parent dir {} does not exist",
                parent.display()
            )));
        }
        Ok(Self { path, opts, running: Arc::new(AtomicBool::new(false)), worker: None })
    }

    /// Stop signal (shares the running flag with the recorder).
    pub fn stop_signal(&self) -> StopSignal {
        StopSignal { running: Arc::clone(&self.running) }
    }

    /// Start recording: consume the frame stream until it ends (`recv` returns None) or `stop()`.
    ///
    /// Encoding/mux runs on a `spawn_blocking` thread (FFmpeg is a blocking API);
    /// frame bridging sends frames one by one on the runtime that called this async fn. Stream end or stop
    /// both trigger flush + trailer.
    pub async fn record(&mut self, mut frames: impl Frames) -> Result<(), DeckError> {
        if self.worker.is_some() {
            return Err(DeckError::InvalidState("already recording".into()));
        }
        self.running.store(true, Ordering::SeqCst);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let path = self.path.clone();
        let opts = self.opts.clone();
        let running = Arc::clone(&self.running);

        // Encoding worker (blocking thread)
        let handle = tokio::task::spawn_blocking(move || {
            if let Err(e) = mux_worker(&path, &opts, &mut rx, &running) {
                tracing::error!("recorder worker failed: {e}");
            }
        });

        // Frame bridge loop: async frame source → the worker's sync channel.
        // End conditions (either): (1) frame source ends (recv None); (2) stop() requested
        // (running=false — polling the frame source is async semantics and cannot be cancelled by tokio,
        // so after stop() we exit via an immediate state check)
        loop {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            match tokio::time::timeout(std::time::Duration::from_millis(50), frames.next()).await {
                Ok(Some(frame)) => {
                    if tx.send(frame).is_err() {
                        tracing::warn!("recorder worker exited; stopping frame pump");
                        break;
                    }
                }
                Ok(None) => break,  // Frame source ended
                Err(_elapsed) => {} // Timed out: keep checking running
            }
        }
        drop(tx);

        let _ = handle.await;
        if self.running.swap(false, Ordering::SeqCst) {
            // running is reset when mux_worker ends; this is only for state consistency
        }
        self.running.store(false, Ordering::SeqCst);
        self.worker = None;
        Ok(())
    }

    /// Request stop: set running=false; the worker exits on the next tick and flushes.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// Independently ownable stop signal (recording can still be stopped externally after record() moves the recorder).
#[derive(Clone, Debug, Default)]
pub struct StopSignal {
    running: Arc<AtomicBool>,
}

impl StopSignal {
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub(crate) fn is_stopped(&self) -> bool {
        !self.running.load(Ordering::SeqCst)
    }
}

/// Frame stream abstraction (the deck FrameStream user supplies an async frame source).
pub trait Frames {
    /// Returns the next frame; None = stream ended.
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send;
}

impl Frames for crate::source::FrameStream {
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send {
        crate::source::FrameStream::recv(self)
    }
}

impl Frames for &mut crate::source::FrameStream {
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send {
        (*self).recv()
    }
}

/// Frame pump for continuous recording loops: an async fn wrapper around iterator semantics also works.
/// (If the caller has a sync frame source, `tokio::task::block_in_place` can adapt it — YAGNI.)
#[cfg(feature = "backend-ffmpeg")]
fn mux_worker(
    path: &PathBuf,
    opts: &RecordOptions,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<VideoFrame>,
    running: &std::sync::atomic::AtomicBool,
) -> Result<(), DeckError> {
    use ffmpeg_the_third as ffmpeg;

    ffmpeg::init().map_err(|e| DeckError::Codec(format!("ffmpeg init: {e}")))?;

    let mut out = ffmpeg::format::output(path)
        .map_err(|e| DeckError::Io(std::io::Error::other(format!("open output: {e}"))))?;

    let codec = ffmpeg::codec::encoder::find(ffmpeg::codec::Id::H264)
        .ok_or_else(|| DeckError::Codec("h264 encoder not found".into()))?;

    // First frame decides the dimensions (block waiting for the first frame)
    let first = next_frame(rx, running, None)?;
    let w = first.format.width;
    let h = first.format.height;
    let _fps = opts.fps as i32;

    // Configure and open the encoder
    let ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
    let mut enc =
        ctx.encoder().video().map_err(|e| DeckError::Codec(format!("create encoder: {e}")))?;
    enc.set_width(w);
    enc.set_height(h);
    enc.set_format(ffmpeg::format::Pixel::YUV420P);
    // pts is in µs (input frame pts derives from ts_mono_ns/1000)
    enc.set_time_base(ffmpeg::Rational(1, 1_000_000));
    enc.set_gop(opts.keyframe_interval);
    enc.set_max_b_frames(0);
    enc.set_bit_rate(2_000_000);
    let mut enc =
        enc.open_with(h264_dict()).map_err(|e| DeckError::Codec(format!("open encoder: {e}")))?;

    // Container stream: copy the full codecpar from the opened encoder (including SPS/PPS extradata,
    // official muxing.c pattern; ctx cannot be reused because encoder() consumes it — copy from enc.0)
    let mut stream = out
        .add_stream(ffmpeg::codec::Id::H264)
        .map_err(|e| DeckError::Codec(format!("add stream: {e}")))?;
    stream.copy_parameters_from_context(&enc.0);
    stream.set_time_base(ffmpeg::Rational(1, 1_000_000));
    out.write_header().map_err(|e| DeckError::Codec(format!("write header: {e}")))?;

    // pts anchored to the first frame's baseline (generator/source pts are large epoch values →
    // unweighted they would produce a fake file with duration= 117s)
    let base_pts = first.pts;
    encode_and_mux(&mut enc, &mut out, &first, base_pts)?;

    while running.load(Ordering::SeqCst) {
        match next_frame(rx, running, None) {
            Ok(f) => encode_and_mux(&mut enc, &mut out, &f, base_pts)?,
            Err(e) => {
                tracing::info!("recorder stopping: {e}");
                break;
            }
        }
    }

    enc.send_eof().map_err(|e| DeckError::Codec(format!("send_eof: {e}")))?;
    loop {
        let mut pkt = ffmpeg::codec::packet::Packet::empty();
        match enc.receive_packet(&mut pkt) {
            Ok(_) => pkt
                .write_interleaved(&mut out)
                .map_err(|e| DeckError::Codec(format!("flush pkt: {e}")))?,
            Err(_) => break,
        }
    }
    out.write_trailer().map_err(|e| DeckError::Codec(format!("write trailer: {e}")))?;
    tracing::info!("recorder finished: {:?}", path);
    Ok(())
}

/// Blocking next-frame fetch; returns Err when `stop` is requested or the channel closes (triggering flush finalization).
#[cfg(feature = "backend-ffmpeg")]
fn next_frame(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<VideoFrame>,
    running: &AtomicBool,
    _first: Option<&VideoFrame>,
) -> Result<VideoFrame, DeckError> {
    loop {
        match rx.try_recv() {
            Ok(f) => return Ok(f),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if !running.load(Ordering::SeqCst) {
                    return Err(DeckError::InvalidState("stopped by stop()".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // Frame stream ended (tx dropped after the frame bridge finishes sending)
                return Err(DeckError::InvalidState("frame stream ended".into()));
            }
        }
    }
}

#[cfg(feature = "backend-ffmpeg")]
fn h264_dict() -> ffmpeg_the_third::Dictionary {
    let mut d = ffmpeg_the_third::Dictionary::new();
    d.set("preset", "ultrafast");
    d.set("tune", "zerolatency");
    d
}

#[cfg(feature = "backend-ffmpeg")]
fn encode_and_mux(
    enc: &mut ffmpeg_the_third::encoder::Video,
    out: &mut ffmpeg_the_third::format::context::Output,
    frame: &VideoFrame,
    base_pts: u64,
) -> Result<(), DeckError> {
    use ffmpeg_the_third as ffmpeg;
    let w = frame.format.width as usize;
    let h = frame.format.height as usize;
    let mut avframe =
        ffmpeg::util::frame::Video::new(ffmpeg::format::Pixel::YUV420P, w as u32, h as u32);
    for i in 0..3 {
        if let (Some(src), Some(dst)) = (frame.plane_data(i), Some(avframe.data_mut(i))) {
            let n = dst.len().min(src.len());
            dst[..n].copy_from_slice(&src[..n]);
        }
    }
    avframe.set_pts(Some(frame.pts.saturating_sub(base_pts) as i64));
    enc.send_frame(&avframe).map_err(|e| DeckError::Codec(format!("send frame: {e}")))?;
    loop {
        let mut pkt = ffmpeg::codec::packet::Packet::empty();
        match enc.receive_packet(&mut pkt) {
            Ok(_) => pkt
                .write_interleaved(out)
                .map_err(|e| DeckError::Codec(format!("write pkt: {e}")))?,
            Err(_) => break,
        }
    }
    Ok(())
}

#[cfg(not(feature = "backend-ffmpeg"))]
fn mux_worker(
    _path: &PathBuf,
    _opts: &RecordOptions,
    _rx: &mut tokio::sync::mpsc::UnboundedReceiver<VideoFrame>,
    _running: &std::sync::atomic::AtomicBool,
) -> Result<(), DeckError> {
    Err(DeckError::Codec("recorder requires backend-ffmpeg feature".into()))
}
